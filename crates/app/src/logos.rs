//! Symbol logos, round like TradingView's: a brand-coloured disc with the
//! mark in white.
//!
//! - Coins: the flat colour discs of the `cryptocurrency-icons` set (CC0).
//! - Companies and funds: theSVG's brand marks (`@thesvg/icons`, MIT; the
//!   marks stay their owners' trademarks), found by the symbol's name. The
//!   one-colour "mono" mark is drawn white on the brand colour (or on a dark
//!   disc when the brand colour is very light); marks with no mono version
//!   go on a white disc as they are.
//! - Anything else: a coloured disc with the first letter.
//!
//! A background thread finds and fetches each logo once, keeps the SVG in
//! `<workspace>/logos/` (a miss as an empty file), and renders the disc with
//! resvg. Off in Settings → Symbol logos, or in demo mode: no requests.

use crate::theme::Theme;
use crossbeam_channel::{Receiver, Sender, unbounded};
use egui::{Color32, Pos2, TextureHandle, pos2};
use opentern_core::{AssetKind, Symbol};
use resvg::{tiny_skia, usvg};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The disc's size in pixels (sharp up to ~24 px radius on a 2× screen).
const SIZE: u32 = 96;
const THESVG: &str = "@thesvg/icons@3.3.14";
const COINS: &str = "cryptocurrency-icons@0.18.1";

/// A rendered disc: width, height, premultiplied RGBA.
type Image = (usize, usize, Vec<u8>);

enum Slot {
    Loading,
    Missing,
    Ready(TextureHandle),
}

pub struct Logos {
    pub enabled: bool,
    dir: PathBuf,
    // Interior mutability: the chart paints logos through `&Logos`.
    slots: RefCell<HashMap<String, Slot>>,
    ask: RefCell<Option<Sender<(String, Symbol)>>>,
    got: Receiver<(String, Option<Image>)>,
    got_tx: Sender<(String, Option<Image>)>,
}

/// Quote currencies stripped from a crypto pair to find its coin.
const QUOTES: [&str; 12] = [
    "USDT", "USDC", "FDUSD", "BUSD", "TUSD", "DAI", "USD", "EUR", "GBP", "TRY", "BTC", "ETH",
];

/// The cache key for a symbol's logo (None: letter disc only).
fn key(s: &Symbol) -> Option<String> {
    let ticker = s.ticker().to_uppercase();
    match s.kind {
        AssetKind::Crypto => coin(&ticker).map(|c| format!("coin-{c}")),
        AssetKind::Stock | AssetKind::Fund => {
            let t: String = ticker.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            (!t.is_empty()).then(|| format!("co-{}-{}", s.exchange().to_lowercase(), t.to_lowercase()))
        }
        _ => None,
    }
}

/// A crypto pair's coin: BTCUSDT → btc.
fn coin(ticker: &str) -> Option<String> {
    let pair = ticker.replace(['-', '/', '_'], "");
    let base = QUOTES
        .iter()
        .find_map(|q| pair.strip_suffix(q).filter(|b| b.len() >= 2))
        .unwrap_or(&pair);
    let base = match base {
        "XBT" => "BTC",
        "XDG" => "DOGE",
        b => b,
    };
    (!base.is_empty() && base.chars().all(|c| c.is_ascii_alphanumeric())).then(|| base.to_lowercase())
}

/// Words that say what kind of company, not which.
const FILLER: [&str; 22] = [
    "inc",
    "incorporated",
    "corp",
    "corporation",
    "co",
    "company",
    "ltd",
    "limited",
    "plc",
    "holdings",
    "holding",
    "group",
    "sa",
    "ag",
    "nv",
    "se",
    "the",
    "class",
    "etf",
    "trust",
    "fund",
    "shares",
];

/// Names too general to stand alone as a brand.
const GENERIC: [&str; 14] = [
    "bank",
    "first",
    "american",
    "united",
    "general",
    "national",
    "international",
    "global",
    "new",
    "royal",
    "state",
    "the",
    "great",
    "china",
];

/// Tickers whose company name doesn't lead to their brand.
fn alias(ticker: &str) -> Option<&'static str> {
    Some(match ticker {
        "GOOGL" | "GOOG" => "google",
        "META" => "meta",
        "BRKB" | "BRKA" => "berkshire-hathaway",
        "TCS" => "tata-consultancy-services",
        "HDFCBANK" => "hdfc-bank",
        "ICICIBANK" => "icici-bank",
        "SBIN" => "state-bank-of-india",
        "INFY" => "infosys",
        _ => return None,
    })
}

/// theSVG slugs to try for a company, best first, from its name. With a
/// slug list, only slugs in it.
fn candidates(s: &Symbol, slugs: &HashSet<String>) -> Vec<String> {
    let ticker: String = s.ticker().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let mut out = Vec::new();
    if let Some(a) = alias(&ticker.to_uppercase()) {
        out.push(a.to_string());
    }
    let name = s.description.to_lowercase().replace('&', " and ");
    let words: Vec<&str> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty() && !FILLER.contains(w))
        .collect();
    for n in (1..=words.len().min(4)).rev() {
        let slug = words[..n].join("-");
        if n == 1 && (slug.len() < 3 || GENERIC.contains(&slug.as_str())) {
            continue;
        }
        // "Reliance Industries" is "reliance-industries-limited" there.
        for tail in ["limited", "inc", "corporation", "group"] {
            out.push(format!("{slug}-{tail}"));
        }
        out.insert(out.len() - 4, slug);
    }
    out.retain(|c| slugs.is_empty() || slugs.contains(c));
    let mut seen = HashSet::new();
    out.retain(|c| seen.insert(c.clone()));
    out
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(5)))
        .http_status_as_error(false)
        .build()
        .into()
}

/// The package mirrors, and which have failed this session (a mirror that
/// times out once isn't asked again: it can hang on every request).
struct Mirrors([bool; 2]);

/// GETs a package file from jsDelivr, or unpkg when jsDelivr is down.
fn fetch(agent: &ureq::Agent, m: &mut Mirrors, path: &str) -> Option<String> {
    for (k, base) in ["https://cdn.jsdelivr.net/npm/", "https://unpkg.com/"]
        .into_iter()
        .enumerate()
    {
        if m.0[k] {
            continue;
        }
        match agent.get(&format!("{base}{path}")).call() {
            Ok(mut r) if r.status() == 200 => {
                return r.body_mut().with_config().limit(1 << 20).read_to_string().ok();
            }
            Ok(r) if r.status() == 404 => return None,
            _ => m.0[k] = true,
        }
    }
    None
}

/// theSVG's slugs (fetched once, kept as a text file).
fn slug_list(dir: &Path, agent: &ureq::Agent) -> HashSet<String> {
    let path = dir.join("thesvg-slugs.txt");
    if let Ok(text) = std::fs::read_to_string(&path)
        && !text.is_empty()
    {
        return text.lines().map(str::to_string).collect();
    }
    let url = format!("https://data.jsdelivr.com/v1/package/npm/{THESVG}/flat");
    let mut list: Vec<String> = agent
        .get(&url)
        .call()
        .ok()
        .filter(|r| r.status() == 200)
        .and_then(|mut r| r.body_mut().with_config().limit(8 << 20).read_to_string().ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["files"].as_array().cloned())
        .map(|files| {
            files
                .iter()
                .filter_map(|f| {
                    f["name"]
                        .as_str()?
                        .strip_prefix("/dist/")?
                        .strip_suffix(".js")
                        .map(str::to_string)
                })
                .filter(|s| s != "index")
                .collect()
        })
        .unwrap_or_default();
    list.sort();
    if !list.is_empty() {
        let _ = std::fs::write(&path, list.join("\n"));
    }
    list.into_iter().collect()
}

/// The SVG (and how to draw it) kept for one logo: "coin\n<svg>", or
/// "mark\n<hex>\n<mono|default>\n<svg>". None when there's none.
fn find(
    s: &Symbol,
    agent: &ureq::Agent,
    m: &mut Mirrors,
    slugs: &mut Option<HashSet<String>>,
    dir: &Path,
) -> Option<String> {
    if s.kind == AssetKind::Crypto {
        let c = coin(&s.ticker().to_uppercase())?;
        if let Some(svg) = fetch(agent, m, &format!("{COINS}/svg/color/{c}.svg")) {
            return Some(format!("coin\n{svg}"));
        }
    }
    let slugs = slugs.get_or_insert_with(|| slug_list(dir, agent));
    if slugs.is_empty() {
        // No list (offline): try again next time rather than keep a miss.
        return Some(String::new());
    }
    let mut sym = s.clone();
    if s.kind == AssetKind::Crypto {
        // "Bitcoin / TetherUS" → Bitcoin.
        sym.description = s.description.split('/').next().unwrap_or_default().trim().to_string();
    }
    for slug in candidates(&sym, slugs) {
        let Some(js) = fetch(agent, m, &format!("{THESVG}/dist/{slug}.js")) else {
            continue;
        };
        let hex = js
            .split("export const hex = \"")
            .nth(1)
            .and_then(|r| r.split('"').next())
            .unwrap_or("")
            .to_string();
        let literal = |at: &str| {
            js.split(at)
                .nth(1)
                .and_then(|r| r.split('`').next())
                .map(str::to_string)
        };
        if let Some(svg) = literal("\"mono\": `") {
            return Some(format!("mark\n{hex}\nmono\n{svg}"));
        }
        if let Some(svg) = literal("export const svg = `") {
            return Some(format!("mark\n{hex}\ndefault\n{svg}"));
        }
    }
    None
}

fn parse_hex(h: &str) -> Option<Color32> {
    let h = h.trim_start_matches('#');
    let h: String = if h.len() == 3 {
        h.chars().flat_map(|c| [c, c]).collect()
    } else {
        h.to_string()
    };
    let v = u32::from_str_radix(&h, 16).ok()?;
    (h.len() == 6).then(|| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

fn luma(c: Color32) -> f32 {
    (0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32) / 255.0
}

/// Paints every fill and stroke of a one-colour mark in `col`.
fn recolor(svg: &str, col: &str) -> String {
    let mut s = svg.to_string();
    // Attributes (fill="…") end at a quote; styles (fill:…) at ; or a quote.
    for (key, stops) in [
        ("fill=\"", &['"'][..]),
        ("stroke=\"", &['"'][..]),
        ("fill:", &[';', '"', '\''][..]),
        ("stroke:", &[';', '"', '\''][..]),
    ] {
        let mut out = String::with_capacity(s.len());
        let mut rest = s.as_str();
        while let Some(i) = rest.find(key) {
            let (head, tail) = rest.split_at(i + key.len());
            out.push_str(head);
            let end = tail.find(stops).unwrap_or(tail.len());
            let v = tail[..end].trim();
            out.push_str(if v == "none" || v.starts_with("url(") {
                &tail[..end]
            } else {
                col
            });
            rest = &tail[end..];
        }
        out.push_str(rest);
        s = out;
    }
    let s = s.replace("currentColor", col);
    // Paths without a fill of their own take the root's.
    s.replacen("<svg ", &format!("<svg fill=\"{col}\" "), 1)
}

/// Draws the disc for a kept logo.
fn render(kept: &str) -> Option<Image> {
    let mut pm = tiny_skia::Pixmap::new(SIZE, SIZE)?;
    let opt = usvg::Options::default();
    let size = SIZE as f32;
    let place = |tree: &usvg::Tree, box_size: f32| {
        let ts = tree.size();
        let k = box_size / ts.width().max(ts.height());
        let (w, h) = (ts.width() * k, ts.height() * k);
        tiny_skia::Transform::from_scale(k, k).post_translate((size - w) / 2.0, (size - h) / 2.0)
    };
    let disc = |pm: &mut tiny_skia::Pixmap, c: Color32| {
        let mut paint = tiny_skia::Paint::default();
        paint.set_color_rgba8(c.r(), c.g(), c.b(), 255);
        paint.anti_alias = true;
        if let Some(path) = tiny_skia::PathBuilder::from_circle(size / 2.0, size / 2.0, size / 2.0) {
            pm.fill_path(
                &path,
                &paint,
                tiny_skia::FillRule::Winding,
                tiny_skia::Transform::identity(),
                None,
            );
        }
    };
    let (kind, rest) = kept.split_once('\n')?;
    match kind {
        "coin" => {
            let tree = usvg::Tree::from_str(rest, &opt).ok()?;
            resvg::render(&tree, place(&tree, size), &mut pm.as_mut());
        }
        "mark" => {
            let mut parts = rest.splitn(3, '\n');
            let brand = parse_hex(parts.next()?);
            let mono = parts.next()? == "mono";
            let svg = parts.next()?;
            if mono {
                // The brand colour behind, the mark in white; light brands
                // (Apple's white) go on a dark disc instead.
                let bg = brand
                    .filter(|c| luma(*c) < 0.82)
                    .unwrap_or(Color32::from_rgb(0x1E, 0x22, 0x2D));
                disc(&mut pm, bg);
                let tree = usvg::Tree::from_str(&recolor(svg, "#FFFFFF"), &opt).ok()?;
                resvg::render(&tree, place(&tree, size * 0.54), &mut pm.as_mut());
            } else {
                disc(&mut pm, Color32::WHITE);
                let tree = usvg::Tree::from_str(svg, &opt).ok()?;
                resvg::render(&tree, place(&tree, size * 0.6), &mut pm.as_mut());
            }
        }
        _ => return None,
    }
    Some((SIZE as usize, SIZE as usize, pm.take()))
}

impl Logos {
    pub fn new(dir: PathBuf, enabled: bool) -> Logos {
        let (got_tx, got) = unbounded();
        Logos {
            enabled,
            dir,
            slots: RefCell::new(HashMap::new()),
            ask: RefCell::new(None),
            got,
            got_tx,
        }
    }

    /// Forgets every kept logo (they're fetched again as needed).
    pub fn clear(&self) {
        if let Ok(d) = std::fs::read_dir(&self.dir) {
            for e in d.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        self.slots.borrow_mut().clear();
    }

    /// One thread finds, fetches and renders, in order, so a long
    /// watchlist doesn't fire a burst of requests.
    fn ask(&self, ctx: &egui::Context, key: String, sym: Symbol) {
        let mut ask = self.ask.borrow_mut();
        if ask.is_none() {
            let (tx, rx) = unbounded::<(String, Symbol)>();
            let out = self.got_tx.clone();
            let dir = self.dir.clone();
            let ctx = ctx.clone();
            let _ = std::thread::Builder::new().name("logos".into()).spawn(move || {
                let _ = std::fs::create_dir_all(&dir);
                let agent = agent();
                let mut slugs = None;
                let mut mirrors = Mirrors([false; 2]);
                while let Ok((key, sym)) = rx.recv() {
                    let path = dir.join(format!("{key}.logo"));
                    let kept = match std::fs::read_to_string(&path) {
                        Ok(k) => k,
                        Err(_) => match find(&sym, &agent, &mut mirrors, &mut slugs, &dir) {
                            // Offline: nothing kept, so it's tried again.
                            Some(k) if k.is_empty() => k,
                            found => {
                                let k = found.unwrap_or_default();
                                let _ = std::fs::write(&path, &k);
                                k
                            }
                        },
                    };
                    let img = (!kept.is_empty()).then(|| render(&kept)).flatten();
                    let _ = out.send((key, img));
                    ctx.request_repaint();
                }
            });
            *ask = Some(tx);
        }
        if let Some(tx) = ask.as_ref() {
            let _ = tx.send((key, sym));
        }
    }

    /// Takes in what the logo thread finished.
    pub fn poll(&self, ctx: &egui::Context) {
        while let Ok((key, img)) = self.got.try_recv() {
            let slot = match img {
                Some((w, h, rgba)) => Slot::Ready(ctx.load_texture(
                    format!("logo-{key}"),
                    egui::ColorImage::from_rgba_premultiplied([w, h], &rgba),
                    egui::TextureOptions::LINEAR,
                )),
                None => Slot::Missing,
            };
            self.slots.borrow_mut().insert(key, slot);
        }
    }

    fn texture(&self, ctx: &egui::Context, s: &Symbol) -> Option<TextureHandle> {
        if !self.enabled {
            return None;
        }
        let key = key(s)?;
        if let Some(slot) = self.slots.borrow().get(&key) {
            return match slot {
                Slot::Ready(t) => Some(t.clone()),
                _ => None,
            };
        }
        self.slots.borrow_mut().insert(key.clone(), Slot::Loading);
        self.ask(ctx, key, s.clone());
        None
    }

    /// Paints a symbol's logo as a disc of radius `r` at `c`.
    pub fn paint(&self, p: &egui::Painter, t: &Theme, s: &Symbol, c: Pos2, r: f32) {
        match self.texture(p.ctx(), s) {
            Some(tex) => {
                let rect = egui::Rect::from_center_size(c, egui::vec2(r * 2.0, r * 2.0));
                p.image(
                    tex.id(),
                    rect,
                    egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => letter_disc(p, t, s, c, r),
        }
    }
}

/// A coloured disc with the symbol's first letter (its hue from the name).
pub fn letter_disc(p: &egui::Painter, t: &Theme, s: &Symbol, c: Pos2, r: f32) {
    let name = s.ticker();
    let hash = name
        .bytes()
        .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let hue = (hash % 360) as f32 / 360.0;
    let col: Color32 = egui::ecolor::Hsva::new(hue, 0.45, if t.dark { 0.55 } else { 0.62 }, 1.0).into();
    p.circle_filled(c, r, col);
    let letter = match s.kind {
        AssetKind::Index if opentern_data::expr::is_expr(&s.id) => "ƒ".to_string(),
        _ => name.chars().find(|c| c.is_alphanumeric()).unwrap_or('?').to_string(),
    };
    p.text(
        c,
        egui::Align2::CENTER_CENTER,
        letter,
        crate::theme::semibold(r * 1.05),
        Color32::WHITE,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(t: &str) -> Symbol {
        opentern_core::starter_watchlist()
            .into_iter()
            .find(|s| s.ticker() == t)
            .unwrap()
    }

    #[test]
    fn keys_and_names() {
        assert_eq!(key(&sym("BTCUSDT")).as_deref(), Some("coin-btc"));
        assert_eq!(key(&sym("EURUSD")), None);
        let slugs: HashSet<String> = ["apple", "nvidia", "reliance-industries-limited", "reliance"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(candidates(&sym("AAPL"), &slugs), ["apple"]);
        assert_eq!(candidates(&sym("NVDA"), &slugs), ["nvidia"]);
        assert_eq!(candidates(&sym("RELIANCE"), &slugs)[0], "reliance-industries-limited");
    }

    #[test]
    fn marks_are_recoloured_and_rendered() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path style="fill:#000;opacity:1" d="M2 2h20v20H2z"/><path fill="#123456" d="M0 0h1v1H0z"/></svg>"##;
        let r = recolor(svg, "#FFFFFF");
        assert!(r.contains("fill:#FFFFFF;") && r.contains("fill=\"#FFFFFF\" d="), "{r}");
        let (w, h, px) = render(&format!("mark\n76B900\nmono\n{svg}")).unwrap();
        assert_eq!((w, h), (SIZE as usize, SIZE as usize));
        // The centre is the white mark, the edge the brand green.
        let at = |x: usize, y: usize| px[(y * w + x) * 4..(y * w + x) * 4 + 3].to_vec();
        assert_eq!(at(48, 48), [255, 255, 255]);
        assert_eq!(at(48, 4), [0x76, 0xB9, 0x00]);
    }
}

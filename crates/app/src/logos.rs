//! Symbol logos, round like TradingView's: a brand-coloured disc with the
//! mark in white.
//!
//! - About 200 ship inside the app (`assets/logos/`, made by
//!   `scripts/fetch-logos.py`), so the common ones show at once.
//! - Coins: web3icons' token icons (MIT): the brand colour or gradient with
//!   the mark in white, cropped round; else the `cryptocurrency-icons` discs.
//! - Commodities: our own drawings (`assets/commodities/`, made by
//!   `scripts/make-commodity-icons.py`): gold bars, an oil barrel, a flame.
//! - Indices: TradingView's badges, drawn here ("500" for the S&P 500,
//!   "50" for the Nifty 50); other countries' indices, Treasury yields and
//!   the dollar index show their country's flag (`assets/flags/`,
//!   flag-icons, MIT).
//! - Companies and funds: theSVG's brand marks (`@thesvg/icons`, MIT; the
//!   marks stay their owners' trademarks), found by the symbol's name. The
//!   one-colour "mono" mark is drawn white on the brand colour (or on a dark
//!   disc when the brand colour is very light); marks with no mono version
//!   go on a white disc as they are.
//! - Stocks theSVG lacks: logoapi.sh's ticker logos (free, no key; its
//!   terms allow keeping them in an app), real icons only. Indian
//!   companies: public-domain Commons logos, bundled.
//! - Anything else: a coloured disc with the first letter.
//!
//! A background thread finds and fetches each logo once, keeps the SVG in
//! `<workspace>/logos/` (a miss as an empty file), and renders the disc with
//! resvg. Off in Settings → Symbol logos. Demo mode shows only the bundled
//! ones: no requests.

use crate::icons::ph;
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
const W3: &str = "@web3icons/core@4.0.58";

/// A rendered disc: width, height, premultiplied RGBA.
type Image = (usize, usize, Vec<u8>);

enum Slot {
    Loading,
    Missing,
    Ready(TextureHandle),
}

pub struct Logos {
    pub enabled: bool,
    /// Demo mode: only the logos bundled in the app, never a request.
    offline: bool,
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
    // It names a file, and symbols can come from an imported watchlist.
    file_key(s).filter(|k| k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
}

fn file_key(s: &Symbol) -> Option<String> {
    let ticker = s.ticker().to_uppercase();
    match s.kind {
        AssetKind::Crypto => coin(&ticker).map(|c| format!("coin-{c}")),
        AssetKind::Stock | AssetKind::Fund => {
            let t: String = ticker.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            (!t.is_empty()).then(|| format!("co-{}-{}", s.exchange().to_lowercase(), t.to_lowercase()))
        }
        AssetKind::Commodity | AssetKind::CommoditySpot => opentern_core::commodity_of(s).map(|c| format!("cmdty-{c}")),
        AssetKind::Index => flag_of(s).map(|c| format!("flag-{c}")),
        _ => None,
    }
}

/// Our own commodity drawings, by `commodity_of` name.
const COMMODITY_ICONS: &[(&str, &str)] = &[
    ("gold", include_str!("../assets/commodities/gold.svg")),
    ("silver", include_str!("../assets/commodities/silver.svg")),
    ("platinum", include_str!("../assets/commodities/platinum.svg")),
    ("palladium", include_str!("../assets/commodities/palladium.svg")),
    ("copper", include_str!("../assets/commodities/copper.svg")),
    ("wti", include_str!("../assets/commodities/wti.svg")),
    ("brent", include_str!("../assets/commodities/brent.svg")),
    ("natgas", include_str!("../assets/commodities/natgas.svg")),
    ("gasoline", include_str!("../assets/commodities/gasoline.svg")),
    ("heating-oil", include_str!("../assets/commodities/heating-oil.svg")),
    ("wheat", include_str!("../assets/commodities/wheat.svg")),
    ("corn", include_str!("../assets/commodities/corn.svg")),
    ("soybeans", include_str!("../assets/commodities/soybeans.svg")),
    ("coffee", include_str!("../assets/commodities/coffee.svg")),
    ("sugar", include_str!("../assets/commodities/sugar.svg")),
    ("cocoa", include_str!("../assets/commodities/cocoa.svg")),
    ("cotton", include_str!("../assets/commodities/cotton.svg")),
    ("cattle", include_str!("../assets/commodities/cattle.svg")),
];

/// Country flags (flag-icons, MIT; the square set).
const FLAGS: &[(&str, &str)] = &[
    ("au", include_str!("../assets/flags/au.svg")),
    ("br", include_str!("../assets/flags/br.svg")),
    ("ca", include_str!("../assets/flags/ca.svg")),
    ("ch", include_str!("../assets/flags/ch.svg")),
    ("cn", include_str!("../assets/flags/cn.svg")),
    ("de", include_str!("../assets/flags/de.svg")),
    ("eu", include_str!("../assets/flags/eu.svg")),
    ("fr", include_str!("../assets/flags/fr.svg")),
    ("gb", include_str!("../assets/flags/gb.svg")),
    ("hk", include_str!("../assets/flags/hk.svg")),
    ("jp", include_str!("../assets/flags/jp.svg")),
    ("kr", include_str!("../assets/flags/kr.svg")),
    ("sg", include_str!("../assets/flags/sg.svg")),
    ("tw", include_str!("../assets/flags/tw.svg")),
    ("us", include_str!("../assets/flags/us.svg")),
    ("in", include_str!("../assets/flags/in.svg")),
    ("nz", include_str!("../assets/flags/nz.svg")),
    ("mx", include_str!("../assets/flags/mx.svg")),
    ("za", include_str!("../assets/flags/za.svg")),
    ("tr", include_str!("../assets/flags/tr.svg")),
    ("se", include_str!("../assets/flags/se.svg")),
    ("no", include_str!("../assets/flags/no.svg")),
    ("dk", include_str!("../assets/flags/dk.svg")),
    ("pl", include_str!("../assets/flags/pl.svg")),
    ("hu", include_str!("../assets/flags/hu.svg")),
    ("cz", include_str!("../assets/flags/cz.svg")),
    ("ru", include_str!("../assets/flags/ru.svg")),
    ("th", include_str!("../assets/flags/th.svg")),
    ("id", include_str!("../assets/flags/id.svg")),
    ("my", include_str!("../assets/flags/my.svg")),
    ("ph", include_str!("../assets/flags/ph.svg")),
    ("il", include_str!("../assets/flags/il.svg")),
    ("ae", include_str!("../assets/flags/ae.svg")),
    ("sa", include_str!("../assets/flags/sa.svg")),
    ("vn", include_str!("../assets/flags/vn.svg")),
    ("pk", include_str!("../assets/flags/pk.svg")),
    ("cl", include_str!("../assets/flags/cl.svg")),
    ("co", include_str!("../assets/flags/co.svg")),
    ("ar", include_str!("../assets/flags/ar.svg")),
    ("eg", include_str!("../assets/flags/eg.svg")),
    ("ng", include_str!("../assets/flags/ng.svg")),
];

/// The flag an index shows: indices outside the US and India (which have
/// TradingView's number badges), Treasury yields and the dollar index.
fn flag_of(s: &Symbol) -> Option<&'static str> {
    Some(match s.id.as_str() {
        "TVC:NI225" => "jp",
        "TVC:UKX" => "gb",
        "XETR:DAX" => "de",
        "TVC:CAC40" => "fr",
        "TVC:SX5E" => "eu",
        "SIX:SMI" => "ch",
        "TVC:HSI" => "hk",
        "SSE:000001" => "cn",
        "TVC:KOSPI" => "kr",
        "TWSE:TAIEX" => "tw",
        "TVC:STI" => "sg",
        "ASX:XJO" => "au",
        "TSX:TSX" => "ca",
        "BMFBOVESPA:IBOV" => "br",
        "TVC:US03MY" | "TVC:US05Y" | "TVC:US10Y" | "TVC:US30Y" | "TVC:DXY" => "us",
        _ => return None,
    })
}

/// A kept logo for one of our commodity drawings (a square, cropped round).
fn commodity_icon(key: &str) -> Option<String> {
    let svg = match key.strip_prefix("flag-") {
        Some(cc) => FLAGS.iter().find(|(n, _)| *n == cc)?.1,
        None => {
            let name = key.strip_prefix("cmdty-")?;
            COMMODITY_ICONS.iter().find(|(n, _)| *n == name)?.1
        }
    };
    Some(format!("square\n{svg}"))
}

/// The flag a currency shows (flag-icons' country codes).
fn currency_flag(ccy: &str) -> Option<&'static str> {
    Some(match ccy {
        "USD" => "us",
        "EUR" => "eu",
        "GBP" => "gb",
        "JPY" => "jp",
        "CHF" => "ch",
        "AUD" => "au",
        "NZD" => "nz",
        "CAD" => "ca",
        "CNY" | "CNH" => "cn",
        "HKD" => "hk",
        "SGD" => "sg",
        "INR" => "in",
        "KRW" => "kr",
        "TWD" => "tw",
        "BRL" => "br",
        "MXN" => "mx",
        "ZAR" => "za",
        "TRY" => "tr",
        "SEK" => "se",
        "NOK" => "no",
        "DKK" => "dk",
        "PLN" => "pl",
        "HUF" => "hu",
        "CZK" => "cz",
        "RUB" => "ru",
        "THB" => "th",
        "IDR" => "id",
        "MYR" => "my",
        "PHP" => "ph",
        "ILS" => "il",
        "AED" => "ae",
        "SAR" => "sa",
        "VND" => "vn",
        "PKR" => "pk",
        "CLP" => "cl",
        "COP" => "co",
        "ARS" => "ar",
        "EGP" => "eg",
        "NGN" => "ng",
        _ => return None,
    })
}

/// A forex pair's two flags: `FX:EURUSD`, OANDA's `EUR_USD`, or Yahoo's
/// `FX:INR` (= USD/INR).
fn forex_flags(s: &Symbol) -> Option<(&'static str, &'static str)> {
    if s.kind != AssetKind::Forex {
        return None;
    }
    let t: String = s
        .ticker()
        .to_uppercase()
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect();
    let (b, q) = match t.len() {
        6 => (t[..3].to_string(), t[3..].to_string()),
        3 => ("USD".to_string(), t),
        _ => return None,
    };
    Some((currency_flag(&b)?, currency_flag(&q)?))
}

/// A crypto pair's coin: BTCUSDT → btc.
fn coin(ticker: &str) -> Option<String> {
    // A perpetual (BTCUSDT.P) shows its coin's logo.
    let pair = ticker.trim_end_matches(".P").replace(['-', '/', '_'], "");
    let base = QUOTES
        .iter()
        .find_map(|q| pair.strip_suffix(q).filter(|b| b.len() >= 2))
        .unwrap_or(&pair);
    let base = match base {
        "XBT" => "BTC",
        "XDG" => "DOGE",
        b => b,
    };
    // Binance quotes some coins per thousand or million (1000SATS,
    // 1MBABYDOGE): the coin's own logo.
    let base = ["1000000", "1000", "1M"]
        .iter()
        .find_map(|m| {
            base.strip_prefix(m)
                .filter(|b| b.len() >= 2 && b.starts_with(|c: char| c.is_ascii_alphabetic()))
        })
        .unwrap_or(base);
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
struct Mirrors([std::sync::atomic::AtomicBool; 2]);

/// GETs a package file from jsDelivr, or unpkg when jsDelivr is down.
fn fetch(agent: &ureq::Agent, m: &Mirrors, path: &str) -> Option<String> {
    for (k, base) in ["https://cdn.jsdelivr.net/npm/", "https://unpkg.com/"]
        .into_iter()
        .enumerate()
    {
        if m.0[k].load(std::sync::atomic::Ordering::Relaxed) {
            continue;
        }
        match agent.get(&format!("{base}{path}")).call() {
            Ok(mut r) if r.status() == 200 => {
                return r.body_mut().with_config().limit(1 << 20).read_to_string().ok();
            }
            Ok(r) if r.status() == 404 => return None,
            _ => m.0[k].store(true, std::sync::atomic::Ordering::Relaxed),
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
    m: &Mirrors,
    slugs: &std::sync::OnceLock<HashSet<String>>,
    dir: &Path,
) -> Option<String> {
    if s.kind == AssetKind::Crypto {
        let c = coin(&s.ticker().to_uppercase())?;
        let w3 = format!("{W3}/dist/svgs/tokens/background/{}.svg.js", c.to_uppercase());
        if let Some(svg) = fetch(agent, m, &w3).and_then(|js| js_string(&js)) {
            return Some(format!("square\n{svg}"));
        }
        if let Some(svg) = fetch(agent, m, &format!("{COINS}/svg/color/{c}.svg")) {
            return Some(format!("coin\n{svg}"));
        }
    }
    let slugs = slugs.get_or_init(|| slug_list(dir, agent));
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
    match logoapi(agent, s) {
        Got::Logo(k) => Some(k),
        Got::Later => Some(String::new()),
        Got::None => None,
    }
}

/// What a source answered: a logo, none, or ask again another time (it
/// failed or asked us to slow down).
enum Got {
    Logo(String),
    None,
    Later,
}

/// logoapi.sh's ticker logos (free, no key; its terms allow using and
/// keeping them in an app). US listings, and NSE/BSE's largest companies.
/// Only real icons: its stand-ins (a favicon, a name tile, a wordmark too
/// wide for a disc) come with `X-Logo-Fallback` and count as none.
fn logoapi(agent: &ureq::Agent, s: &Symbol) -> Got {
    if !matches!(s.kind, AssetKind::Stock | AssetKind::Fund) {
        return Got::None;
    }
    let t = s.ticker().to_uppercase();
    if t.is_empty() || !t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') {
        return Got::None;
    }
    let ticker = match s.exchange() {
        e @ ("NASDAQ" | "NYSE" | "AMEX" | "NSE") => format!("{e}:{t}"),
        "BSE" => format!("{t}.BO"),
        _ => return Got::None,
    };
    let url = format!("https://logoapi.sh/ticker/{ticker}.svg?fallback=none");
    let mut r = match agent.get(&url).call() {
        Ok(r) => r,
        Err(_) => return Got::Later,
    };
    match r.status().as_u16() {
        200 => {}
        404 => return Got::None,
        _ => return Got::Later,
    }
    if r.headers().contains_key("x-logo-fallback") {
        return Got::None;
    }
    let Ok(svg) = r.body_mut().with_config().limit(1 << 20).read_to_string() else {
        return Got::Later;
    };
    from_logoapi(&svg).map_or(Got::None, Got::Logo)
}

/// How to draw a logoapi.sh answer: a one-colour Simple Icons mark (24 px,
/// a root fill) in white on its colour, a picture wrapped in an SVG as an
/// image on white, anything else on white as it is.
fn from_logoapi(svg: &str) -> Option<String> {
    let start = svg.find("<svg")?;
    let svg = &svg[start..];
    let head = &svg[..svg.find('>')?];
    if let Some(b64) = svg
        .split("data:image/png;base64,")
        .nth(1)
        .and_then(|r| r.split(['"', '\'']).next())
    {
        return Some(format!("image\nmark\n{b64}"));
    }
    let fill = head
        .split("fill=\"#")
        .nth(1)
        .and_then(|r| r.split('"').next())
        .filter(|h| h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()));
    match fill {
        Some(hex) if head.contains("viewBox=\"0 0 24 24\"") => {
            // The mark is recoloured white, which sets the root's fill.
            let svg = svg.replacen(&format!(" fill=\"#{hex}\""), "", 1);
            Some(format!("mark\n{hex}\nmono\n{svg}"))
        }
        _ => Some(format!("mark\n\ndefault\n{svg}")),
    }
}

/// The SVG in a web3icons module: `var X = '<svg …>'`, unescaped.
fn js_string(js: &str) -> Option<String> {
    let start = js.find('=')? + 1;
    let rest = js[start..].trim_start();
    let q = rest.chars().next().filter(|c| matches!(c, '\'' | '"' | '`'))?;
    let mut out = String::new();
    let mut chars = rest[1..].chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                o => out.push(o),
            },
            c if c == q => return out.contains("<svg").then_some(out),
            c => out.push(c),
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

/// Cuts a square image to a circle (anti-aliased edge).
fn crop_round(pm: &mut tiny_skia::Pixmap) {
    let c = SIZE as f32 / 2.0;
    for (i, px) in pm.data_mut().chunks_mut(4).enumerate() {
        let (x, y) = ((i as u32 % SIZE) as f32 + 0.5, (i as u32 / SIZE) as f32 + 0.5);
        let d = ((x - c).powi(2) + (y - c).powi(2)).sqrt();
        let k = (c - d + 0.5).clamp(0.0, 1.0);
        if k < 1.0 {
            for v in px.iter_mut() {
                *v = (*v as f32 * k) as u8;
            }
        }
    }
}

/// A PNG as a premultiplied pixmap (any colour type; at most 2048 px a side).
fn raster_pixmap(bytes: &[u8]) -> Option<tiny_skia::Pixmap> {
    // A downloaded file's header can claim any size: refuse a big one before
    // anything is allocated for it.
    let mut dec = png::Decoder::new_with_limits(std::io::Cursor::new(bytes), png::Limits { bytes: 32 << 20 });
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().ok()?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w == 0 || h == 0 || w > 2048 || h > 2048 {
        return None;
    }
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    if w == 0 || h == 0 || w > 2048 || h > 2048 {
        return None;
    }
    let px = &buf[..info.buffer_size()];
    let n = (w * h) as usize;
    let mut rgba = Vec::with_capacity(n * 4);
    let mut push = |r: u8, g: u8, b: u8, a: u8| {
        let m = |v: u8| ((v as u16 * a as u16 + 127) / 255) as u8;
        rgba.extend_from_slice(&[m(r), m(g), m(b), a]);
    };
    match info.color_type {
        png::ColorType::Rgba => px.as_chunks::<4>().0.iter().for_each(|p| push(p[0], p[1], p[2], p[3])),
        png::ColorType::Rgb => px.as_chunks::<3>().0.iter().for_each(|p| push(p[0], p[1], p[2], 255)),
        png::ColorType::GrayscaleAlpha => px.as_chunks::<2>().0.iter().for_each(|p| push(p[0], p[0], p[0], p[1])),
        png::ColorType::Grayscale => px.iter().for_each(|&v| push(v, v, v, 255)),
        _ => return None,
    }
    (rgba.len() == n * 4).then_some(())?;
    tiny_skia::Pixmap::from_vec(rgba, tiny_skia::IntSize::from_wh(w, h)?)
}

/// SVG options for downloaded logos: an `<image>` may only use data that's
/// in the file itself (by default usvg reads any path an `href` names
/// from the disk).
fn svg_options() -> usvg::Options<'static> {
    usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    }
}

/// Draws the disc for a kept logo.
fn render(kept: &str) -> Option<Image> {
    let mut pm = tiny_skia::Pixmap::new(SIZE, SIZE)?;
    let opt = svg_options();
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
    // A checkout with Windows line endings must still parse.
    let kept = kept.replace("\r\n", "\n");
    let (kind, rest) = kept.split_once('\n')?;
    match kind {
        "coin" => {
            let tree = usvg::Tree::from_str(rest, &opt).ok()?;
            resvg::render(&tree, place(&tree, size), &mut pm.as_mut());
        }
        "square" => {
            // A square icon (colour to the edges), cropped to a circle.
            let tree = usvg::Tree::from_str(rest, &opt).ok()?;
            resvg::render(&tree, place(&tree, size), &mut pm.as_mut());
            crop_round(&mut pm);
        }
        "image" => {
            // A PNG: "fill" covers the disc, "mark" sits on a white disc
            // (a company's logo).
            let (how, b64) = rest.split_once('\n')?;
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()?;
            let src = raster_pixmap(&bytes)?;
            let fill = how == "fill";
            if !fill {
                disc(&mut pm, Color32::WHITE);
            }
            let room = if fill { size } else { size * 0.72 };
            let (w, h) = (src.width() as f32, src.height() as f32);
            let k = room / w.max(h);
            let ts = tiny_skia::Transform::from_scale(k, k).post_translate((size - w * k) / 2.0, (size - h * k) / 2.0);
            let paint = tiny_skia::PixmapPaint {
                quality: tiny_skia::FilterQuality::Bicubic,
                ..Default::default()
            };
            pm.draw_pixmap(0, 0, src.as_ref(), &paint, ts, None);
            crop_round(&mut pm);
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

/// Logo downloads at once (each is a few small requests to a CDN).
const NET_WORKERS: usize = 4;

/// Where a fetched logo (or a miss, as an empty file) is kept.
/// v3: logoapi.sh added, so misses are tried again (older files are ignored).
fn kept_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{key}.v3.logo"))
}

/// A logo from the machine: bundled, our drawings, or kept from an earlier
/// fetch (Some("") = known to have none). None: ask the network.
fn local(key: &str, dir: &Path) -> Option<String> {
    // A BSE listing shares its NSE twin's logo.
    let twin = key.strip_prefix("co-bse-").map(|t| format!("co-nse-{t}"));
    let ours = |k: &str| k == key || Some(k) == twin.as_deref();
    if let Some((_, v)) = crate::logos_bundled::BUNDLED.iter().find(|(k, _)| ours(k)) {
        return Some(v.to_string());
    }
    if let Some((_, png)) = crate::logos_bundled::BUNDLED_IMAGES.iter().find(|(k, _)| ours(k)) {
        use base64::Engine;
        return Some(format!(
            "image\nmark\n{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        ));
    }
    commodity_icon(key).or_else(|| std::fs::read_to_string(kept_path(dir, key)).ok())
}

impl Logos {
    pub fn new(dir: PathBuf, enabled: bool, offline: bool) -> Logos {
        let (got_tx, got) = unbounded();
        Logos {
            enabled,
            offline,
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

    /// One thread answers from what's on the machine (bundled logos, our
    /// drawings, kept files) at once; `NET_WORKERS` find and fetch the
    /// rest, a few at a time, so a long watchlist doesn't fire a burst of
    /// requests. Split so a bundled logo never waits behind slow lookups
    /// (150 coins on the market overview queued minutes of them).
    fn ask(&self, ctx: &egui::Context, key: String, sym: Symbol) {
        let mut ask = self.ask.borrow_mut();
        if ask.is_none() {
            let (tx, rx) = unbounded::<(String, Symbol)>();
            let (net_tx, net_rx) = unbounded::<(String, Symbol)>();
            let dir = self.dir.clone();
            let offline = self.offline;
            {
                let out = self.got_tx.clone();
                let ctx = ctx.clone();
                let dir = dir.clone();
                let _ = std::thread::Builder::new().name("logos".into()).spawn(move || {
                    while let Ok((key, sym)) = rx.recv() {
                        let kept = local(&key, &dir);
                        if kept.is_none() && !offline {
                            let _ = net_tx.send((key, sym));
                            continue;
                        }
                        let img = kept.filter(|k| !k.is_empty()).and_then(|k| render(&k));
                        let _ = out.send((key, img));
                        ctx.request_repaint();
                    }
                });
            }
            let _ = std::fs::create_dir_all(&dir);
            let agent = agent();
            let slugs = std::sync::Arc::new(std::sync::OnceLock::new());
            let mirrors = std::sync::Arc::new(Mirrors(Default::default()));
            for _ in 0..NET_WORKERS {
                let (out, ctx, dir, net_rx) = (self.got_tx.clone(), ctx.clone(), dir.clone(), net_rx.clone());
                let (agent, slugs, mirrors) = (agent.clone(), slugs.clone(), mirrors.clone());
                let _ = std::thread::Builder::new().name("logos-net".into()).spawn(move || {
                    while let Ok((key, sym)) = net_rx.recv() {
                        let kept = match find(&sym, &agent, &mirrors, &slugs, &dir) {
                            // Offline: nothing kept, so it's tried again.
                            Some(k) if k.is_empty() => k,
                            found => {
                                let k = found.unwrap_or_default();
                                let _ = std::fs::write(kept_path(&dir, &key), &k);
                                k
                            }
                        };
                        let img = (!kept.is_empty()).then(|| render(&kept)).flatten();
                        let _ = out.send((key, img));
                        ctx.request_repaint();
                    }
                });
            }
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
        self.texture_key(ctx, key(s)?, s)
    }

    fn texture_key(&self, ctx: &egui::Context, key: String, s: &Symbol) -> Option<TextureHandle> {
        if !self.enabled {
            return None;
        }
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
        if self.enabled
            && let Some((bg, mark)) = badge(s)
        {
            p.circle_filled(c, r, bg);
            let size = if mark.chars().count() >= 3 { r * 0.78 } else { r * 1.0 };
            p.text(
                c,
                egui::Align2::CENTER_CENTER,
                mark,
                crate::theme::semibold(size),
                Color32::WHITE,
            );
            return;
        }
        // A currency pair: its two flags, the base in front (as TradingView).
        if let Some((base, quote)) = forex_flags(s) {
            let tb = self.texture_key(p.ctx(), format!("flag-{base}"), s);
            let tq = self.texture_key(p.ctx(), format!("flag-{quote}"), s);
            if let (Some(tb), Some(tq)) = (tb, tq) {
                let d = r * 0.3;
                let rr = r * 0.72;
                let uv = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                let back = c + egui::vec2(d, d);
                let front = c - egui::vec2(d, d);
                p.image(
                    tq.id(),
                    egui::Rect::from_center_size(back, egui::vec2(rr * 2.0, rr * 2.0)),
                    uv,
                    Color32::WHITE,
                );
                p.image(
                    tb.id(),
                    egui::Rect::from_center_size(front, egui::vec2(rr * 2.0, rr * 2.0)),
                    uv,
                    Color32::WHITE,
                );
                p.circle_stroke(front, rr, egui::Stroke::new(1.0, Color32::from_black_alpha(90)));
                return;
            }
        }
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

/// Badges for symbols with no logo of their own, as TradingView draws
/// them: indices by their count ("50"), commodities by their element or a
/// picture. (background, mark)
fn badge(s: &Symbol) -> Option<(Color32, &'static str)> {
    let rgb = |v: u32| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
    let t = s.ticker().to_uppercase();
    match s.kind {
        AssetKind::Index if flag_of(s).is_some() => None,
        AssetKind::Index => Some(match t.trim_start_matches('^') {
            "NIFTY" | "NSEI" | "NIFTY50" => (rgb(0x2B3990), "50"),
            "BANKNIFTY" | "NSEBANK" => (rgb(0x2B3990), ph::BANK),
            "SENSEX" | "BSESN" => (rgb(0x8B1E3F), "30"),
            "SPX" | "GSPC" | "SP500" => (rgb(0xC8102E), "500"),
            "NDX" | "NDX100" | "NASDAQ100" => (rgb(0x0091C8), "100"),
            "IXIC" => (rgb(0x0091C8), "C"),
            "DJI" | "DJIA" => (rgb(0x0B3D91), "30"),
            "RUT" => (rgb(0x6D2077), "2K"),
            "VIX" => (rgb(0x4B2E83), "VIX"),
            "VXN" => (rgb(0x0091C8), "VXN"),
            "MOVE" => (rgb(0x1F3B5C), "MV"),
            "INDIAVIX" => (rgb(0x2B3990), "VIX"),
            "DAX" | "GDAXI" => (rgb(0x1E1E1E), "40"),
            "FTSE" => (rgb(0x00205B), "100"),
            "N225" | "NI225" => (rgb(0xBC002D), "225"),
            _ => return None,
        }),
        // Ones we have no drawing for.
        AssetKind::Commodity | AssetKind::CommoditySpot if opentern_core::commodity_of(s).is_none() => {
            Some((rgb(0x5B6472), ph::BARN))
        }
        _ => None,
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
        // Coins quoted per thousand or million share the coin's logo.
        assert_eq!(coin("1000SATSUSDT").as_deref(), Some("sats"));
        assert_eq!(coin("1MBABYDOGEUSDT").as_deref(), Some("babydoge"));
        assert_eq!(coin("1INCHUSDT").as_deref(), Some("1inch"));
        assert_eq!(coin("BTCUSDT.P").as_deref(), Some("btc"));
        let slugs: HashSet<String> = ["apple", "nvidia", "reliance-industries-limited", "reliance"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(candidates(&sym("AAPL"), &slugs), ["apple"]);
        assert_eq!(candidates(&sym("NVDA"), &slugs), ["nvidia"]);
        assert_eq!(candidates(&sym("RELIANCE"), &slugs)[0], "reliance-industries-limited");
    }

    #[test]
    fn bundled_coins_render_round() {
        let (_, sol) = crate::logos_bundled::BUNDLED
            .iter()
            .find(|(k, _)| *k == "coin-sol")
            .unwrap();
        let (w, _, px) = render(sol).unwrap();
        // A corner is cut away; the middle has colour.
        assert_eq!(px[3], 0);
        let mid = (48 * w + 48) * 4;
        assert!(px[mid + 3] > 200);
        // The same file checked out with CRLF line endings (Windows).
        assert_eq!(render(&sol.replace('\n', "\r\n")).map(|i| i.2), Some(px.clone()));
        let (_, aapl) = crate::logos_bundled::BUNDLED
            .iter()
            .find(|(k, _)| *k == "co-nasdaq-aapl")
            .unwrap();
        assert!(render(&aapl.replace('\n', "\r\n")).is_some());
        assert_eq!(
            js_string("var X =\n  '<svg a=\\'1\\'>\\n</svg>';").as_deref(),
            Some("<svg a='1'>\n</svg>")
        );
        // Every commodity, futures and spot, has a drawing that renders.
        for c in opentern_core::commodities()
            .iter()
            .chain(&opentern_core::spot_commodities())
        {
            assert!(badge(c).is_none(), "{}", c.id);
            let kept = commodity_icon(&key(c).unwrap()).unwrap_or_else(|| panic!("{}", c.id));
            let (w, _, px) = render(&kept).unwrap();
            assert_eq!(px[3], 0, "{}: corner cut away", c.id);
            assert!(px[(48 * w + 4) * 4 + 3] > 200, "{}: filled to the edge", c.id);
        }
    }

    /// A w×h PNG of one colour.
    fn png_of(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut enc = png::Encoder::new(&mut buf, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(&rgba.repeat((w * h) as usize)).unwrap();
        wr.finish().unwrap();
        buf
    }

    #[test]
    fn png_logos_fill_the_disc_or_sit_on_white() {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(png_of(40, 20, [200, 30, 30, 255]));
        let at = |px: &[u8], x: usize, y: usize| px[(y * SIZE as usize + x) * 4..][..4].to_vec();
        // A coin's image covers the disc, corners cut away.
        let (_, _, px) = render(&format!("image\nfill\n{b64}")).unwrap();
        assert_eq!(at(&px, 48, 48), [200, 30, 30, 255]);
        assert_eq!(at(&px, 0, 0)[3], 0);
        // A company's logo on white, with room around it.
        let (_, _, px) = render(&format!("image\nmark\n{b64}")).unwrap();
        assert_eq!(at(&px, 48, 48), [200, 30, 30, 255]);
        assert_eq!(at(&px, 48, 4), [255, 255, 255, 255]);
        // Not a PNG: no logo, not a crash.
        assert!(render("image\nfill\nbm90IGEgcG5n").is_none());
    }

    #[test]
    fn index_flags_and_badges() {
        for s in opentern_core::indices() {
            match flag_of(&s) {
                // Flags render, and the badge stands aside for them.
                Some(_) => {
                    assert!(badge(&s).is_none(), "{}", s.id);
                    let k = commodity_icon(&key(&s).unwrap()).unwrap();
                    assert!(render(&k).is_some(), "{}", s.id);
                }
                // Everything else has a badge.
                None => assert!(badge(&s).is_some(), "{} has no icon", s.id),
            }
        }
    }

    /// Downloaded logos, mangled: every kind still gives a picture or
    /// nothing, never a crash.
    #[test]
    fn mangled_logos_never_crash() {
        use opentern_core::fuzzing::{bytes_target, text_target};
        let svg = include_str!("../assets/commodities/gold.svg");
        for kind in ["square", "coin", "mark\nD1AB66\nmono", "mark\n\ndefault"] {
            text_target(&format!("logo {kind}"), svg, |s| {
                let _ = render(&format!("{kind}\n{s}"));
            });
        }
        text_target(
            "logoapi",
            r##"<svg role="img" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" fill="#D1AB66"><path d="M0 0h24v24H0z"/></svg>"##,
            |s| {
                if let Some(k) = from_logoapi(s) {
                    let _ = render(&k);
                }
            },
        );
        let png = png_of(16, 8, [10, 200, 30, 255]);
        bytes_target("png", &png, |b| {
            let _ = raster_pixmap(b);
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(b);
            let _ = render(&format!("image\nfill\n{b64}"));
        });
    }

    #[test]
    fn logos_never_read_files_from_the_disk() {
        let dir = std::env::temp_dir().join(format!("opentern-svg-href-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let inner = dir.join("inner.svg");
        std::fs::write(&inner, r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#ff0000"/></svg>"##).unwrap();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 10 10"><image width="10" height="10" xlink:href="{}"/></svg>"#,
            inner.display()
        );
        // The default options would draw the file's red square.
        let tree = usvg::Tree::from_str(&svg, &svg_options()).unwrap();
        assert!(!tree.root().has_children(), "an outside file was loaded");
        let _ = std::fs::remove_dir_all(&dir);
        // A PNG whose header claims a huge size is refused, not allocated.
        let mut big = png_of(1, 1, [0, 0, 0, 255]);
        big[16..20].copy_from_slice(&60_000u32.to_be_bytes());
        big[20..24].copy_from_slice(&60_000u32.to_be_bytes());
        assert!(raster_pixmap(&big).is_none());
    }

    /// Bundled coins and our drawings are answered by the local thread,
    /// never queued behind network lookups.
    #[test]
    fn bundled_logos_are_local() {
        let dir = std::env::temp_dir().join("opentern-no-logos-here");
        for key in [
            "coin-xrp",
            "coin-zec",
            "coin-near",
            "cmdty-silver",
            "cmdty-gold",
            "flag-jp",
        ] {
            let kept = local(key, &dir).unwrap_or_else(|| panic!("{key} isn't local"));
            assert!(render(&kept).is_some(), "{key} doesn't render");
        }
        for s in opentern_core::commodities() {
            let k = key(&s).unwrap_or_else(|| panic!("{} has no key", s.id));
            assert!(local(&k, &dir).is_some(), "{} isn't local", s.id);
        }
        assert!(local("coin-notacoinxyz", &dir).is_none());
    }

    #[test]
    fn forex_pairs_get_two_flags() {
        let fx = |id: &str| Symbol {
            id: id.into(),
            kind: AssetKind::Forex,
            ..opentern_core::commodities()[0].clone()
        };
        assert_eq!(forex_flags(&fx("FX:EURUSD")), Some(("eu", "us")));
        assert_eq!(forex_flags(&fx("FX:INR")), Some(("us", "in")));
        assert_eq!(forex_flags(&fx("OANDA:GBP_JPY")), Some(("gb", "jp")));
        let dir = std::env::temp_dir().join("opentern-no-logos-here");
        for (_, f) in FLAGS {
            assert!(render(&format!("square\n{f}")).is_some());
        }
        assert!(local("flag-in", &dir).is_some());
    }

    #[test]
    fn bundled_pictures_render() {
        assert!(!crate::logos_bundled::BUNDLED_IMAGES.is_empty());
        for (k, png) in crate::logos_bundled::BUNDLED_IMAGES {
            assert!(raster_pixmap(png).is_some(), "{k}");
        }
    }

    #[test]
    fn logoapi_answers_become_discs() {
        let simple = r##"<svg role="img" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" fill="#D1AB66"><path d="M0 0h24v24H0z"/></svg>"##;
        let kept = from_logoapi(simple).unwrap();
        assert!(kept.starts_with("mark\nD1AB66\nmono\n<svg"), "{kept}");
        let full = r##"<?xml version="1.0"?><svg viewBox="0 0 256 315" xmlns="http://www.w3.org/2000/svg"><path fill="#000" d="M0 0h9v9H0z"/></svg>"##;
        assert!(from_logoapi(full).unwrap().starts_with("mark\n\ndefault\n<svg"));
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(png_of(4, 4, [0, 0, 0, 255]));
        let wrapped =
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg"><image href="data:image/png;base64,{b64}"/></svg>"#);
        assert_eq!(from_logoapi(&wrapped).unwrap(), format!("image\nmark\n{b64}"));
        for k in [kept, from_logoapi(full).unwrap(), from_logoapi(&wrapped).unwrap()] {
            assert!(render(&k).is_some(), "{k}");
        }
    }

    /// Real logoapi.sh lookups (network):
    /// `cargo test -p opentern --bin opentern live_logo_sources -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_logo_sources() {
        let agent = agent();
        let s = |id: &str, kind| Symbol {
            id: id.into(),
            description: String::new(),
            kind,
            currency: "USD".into(),
            tick_size: 0.01,
            min_qty: 0.0,
            timezone: "UTC".into(),
        };
        for id in [
            "NASDAQ:MNDY",
            "NYSE:KO",
            "NSE:RELIANCE",
            "BSE:TCS",
            "NSE:HDFCLIFE",
            "NASDAQ:ZZZZQ",
        ] {
            let got = match logoapi(&agent, &s(id, AssetKind::Stock)) {
                Got::Logo(k) => format!(
                    "logo ({}), renders: {}",
                    k.lines().next().unwrap(),
                    render(&k).is_some()
                ),
                Got::None => "none".into(),
                Got::Later => "later".into(),
            };
            println!("logoapi {id}: {got}");
        }
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

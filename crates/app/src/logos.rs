//! Symbol logos, round like TradingView's: crypto from the
//! `cryptocurrency-icons` package (CC0) on jsDelivr, stocks and funds from
//! Financial Modeling Prep's public logo images. Each is fetched once on a
//! background thread and kept in `<workspace>/logos/` (a miss is kept as an
//! empty file, so it isn't asked for again). Anything without a logo gets a
//! coloured disc with its first letter. Off in Settings, or in demo mode,
//! means no requests at all.

use crate::theme::Theme;
use crossbeam_channel::{Receiver, Sender, unbounded};
use egui::{Color32, Pos2, TextureHandle, pos2};
use opentern_core::{AssetKind, Symbol};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

/// A decoded image: width, height, RGBA bytes.
type Image = (usize, usize, Vec<u8>);

enum Slot {
    Loading,
    Missing,
    /// The texture, and whether the artwork is light (it goes on a dark
    /// disc, not a white one).
    Ready(TextureHandle, bool),
}

/// Light artwork: its visible pixels are mostly near white.
fn is_light(rgba: &[u8]) -> bool {
    let (mut sum, mut n) = (0.0f32, 0.0f32);
    for px in rgba.chunks(4) {
        let a = px[3] as f32 / 255.0;
        if a > 0.3 {
            sum += a * (0.2126 * px[0] as f32 + 0.7152 * px[1] as f32 + 0.0722 * px[2] as f32) / 255.0;
            n += a;
        }
    }
    n > 0.0 && sum / n > 0.85
}

fn load(ctx: &egui::Context, key: &str, img: Option<Image>) -> Slot {
    match img {
        Some((w, h, rgba)) => Slot::Ready(
            ctx.load_texture(
                format!("logo-{key}"),
                egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba),
                egui::TextureOptions::LINEAR,
            ),
            is_light(&rgba),
        ),
        None => Slot::Missing,
    }
}

pub struct Logos {
    pub enabled: bool,
    dir: PathBuf,
    // Interior mutability: the chart paints logos through `&Logos`.
    slots: RefCell<HashMap<String, Slot>>,
    ask: RefCell<Option<Sender<(String, String)>>>,
    got: Receiver<(String, Option<Image>)>,
    got_tx: Sender<(String, Option<Image>)>,
}

/// Quote currencies stripped from a crypto pair to find its coin.
const QUOTES: [&str; 12] = [
    "USDT", "USDC", "FDUSD", "BUSD", "TUSD", "DAI", "USD", "EUR", "GBP", "TRY", "BTC", "ETH",
];

/// Where a symbol's logo comes from: a cache key and a URL.
fn source(s: &Symbol) -> Option<(String, String)> {
    let ticker = s.ticker().to_uppercase();
    match s.kind {
        AssetKind::Crypto => {
            let pair = ticker.replace(['-', '/', '_'], "");
            let base = QUOTES
                .iter()
                .find_map(|q| pair.strip_suffix(q).filter(|b| b.len() >= 2))
                .unwrap_or(&pair)
                .to_string();
            let base = match base.as_str() {
                "XBT" => "BTC".to_string(),
                "XDG" => "DOGE".to_string(),
                _ => base,
            };
            if !base.chars().all(|c| c.is_ascii_alphanumeric()) {
                return None;
            }
            let coin = base.to_lowercase();
            Some((
                format!("crypto-{coin}"),
                format!("https://cdn.jsdelivr.net/npm/cryptocurrency-icons@0.18.1/32/color/{coin}.png"),
            ))
        }
        AssetKind::Stock | AssetKind::Fund => {
            let suffix = match s.exchange() {
                "NSE" => ".NS",
                "BSE" => ".BO",
                _ => "",
            };
            if !ticker
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
            {
                return None;
            }
            let t = format!("{ticker}{suffix}");
            Some((
                format!("stock-{t}"),
                format!("https://financialmodelingprep.com/image-stock/{t}.png"),
            ))
        }
        _ => None,
    }
}

fn decode(bytes: &[u8]) -> Option<Image> {
    let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
    d.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut r = d.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let px = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::GrayscaleAlpha => px.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        _ => return None,
    };
    (rgba.len() == w * h * 4).then_some((w, h, rgba))
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

    /// One thread fetches, in order, so a long watchlist doesn't fire a
    /// burst of requests.
    fn ask(&self, ctx: &egui::Context, key: String, url: String) {
        let mut ask = self.ask.borrow_mut();
        if ask.is_none() {
            let (tx, rx) = unbounded::<(String, String)>();
            let out = self.got_tx.clone();
            let dir = self.dir.clone();
            let ctx = ctx.clone();
            let _ = std::thread::Builder::new().name("logos".into()).spawn(move || {
                let _ = std::fs::create_dir_all(&dir);
                let agent: ureq::Agent = ureq::Agent::config_builder()
                    .timeout_global(Some(std::time::Duration::from_secs(10)))
                    .http_status_as_error(false)
                    .build()
                    .into();
                while let Ok((key, url)) = rx.recv() {
                    let bytes = agent.get(&url).call().ok().and_then(|mut r| {
                        (r.status() == 200)
                            .then(|| r.body_mut().with_config().limit(512 * 1024).read_to_vec().ok())
                            .flatten()
                    });
                    let img = bytes.as_deref().and_then(decode);
                    // Kept either way: a miss is an empty file.
                    let path = dir.join(format!("{key}.png"));
                    let _ = std::fs::write(
                        &path,
                        if img.is_some() {
                            bytes.unwrap_or_default()
                        } else {
                            Vec::new()
                        },
                    );
                    let _ = out.send((key, img));
                    ctx.request_repaint();
                }
            });
            *ask = Some(tx);
        }
        if let Some(tx) = ask.as_ref() {
            let _ = tx.send((key, url));
        }
    }

    /// Takes in what the fetch thread finished.
    pub fn poll(&self, ctx: &egui::Context) {
        while let Ok((key, img)) = self.got.try_recv() {
            let slot = load(ctx, &key, img);
            self.slots.borrow_mut().insert(key, slot);
        }
    }

    fn texture(&self, ctx: &egui::Context, s: &Symbol) -> Option<(TextureHandle, bool)> {
        if !self.enabled {
            return None;
        }
        let (key, url) = source(s)?;
        if let Some(slot) = self.slots.borrow().get(&key) {
            return match slot {
                Slot::Ready(t, light) => Some((t.clone(), *light)),
                _ => None,
            };
        }
        // On disk from an earlier run?
        let path = self.dir.join(format!("{key}.png"));
        if let Ok(bytes) = std::fs::read(&path) {
            let slot = load(ctx, &key, decode(&bytes));
            let t = match &slot {
                Slot::Ready(t, light) => Some((t.clone(), *light)),
                _ => None,
            };
            self.slots.borrow_mut().insert(key, slot);
            return t;
        }
        self.slots.borrow_mut().insert(key.clone(), Slot::Loading);
        self.ask(ctx, key, url);
        None
    }

    /// Paints a symbol's logo as a disc of radius `r` at `c`.
    pub fn paint(&self, p: &egui::Painter, t: &Theme, s: &Symbol, c: Pos2, r: f32) {
        match self.texture(p.ctx(), s) {
            Some((tex, light)) => {
                // Square logos sit on a disc (dark for light artwork); round
                // crypto icons fill it.
                if s.kind != AssetKind::Crypto {
                    p.circle_filled(c, r, if light { Color32::from_gray(38) } else { Color32::WHITE });
                }
                p.add(textured_disc(
                    tex.id(),
                    c,
                    r,
                    if s.kind == AssetKind::Crypto { 1.0 } else { 0.86 },
                ));
            }
            None => letter_disc(p, t, s, c, r),
        }
    }
}

/// A circle textured with the whole image (scaled by `inset`).
fn textured_disc(tex: egui::TextureId, c: Pos2, r: f32, inset: f32) -> egui::Shape {
    let mut mesh = egui::Mesh::with_texture(tex);
    let n = 40;
    mesh.vertices.push(egui::epaint::Vertex {
        pos: c,
        uv: pos2(0.5, 0.5),
        color: Color32::WHITE,
    });
    for i in 0..=n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let (dx, dy) = (a.cos(), a.sin());
        mesh.vertices.push(egui::epaint::Vertex {
            pos: c + egui::vec2(dx, dy) * r,
            uv: pos2(0.5 + dx * 0.5 / inset, 0.5 + dy * 0.5 / inset),
            color: Color32::WHITE,
        });
    }
    for i in 1..=n as u32 {
        mesh.indices.extend([0, i, i + 1]);
    }
    egui::Shape::mesh(mesh)
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

    #[test]
    fn logo_sources() {
        let w = opentern_core::starter_watchlist();
        let src = |t: &str| source(w.iter().find(|s| s.ticker() == t).unwrap()).map(|x| x.1);
        assert!(src("BTCUSDT").unwrap().ends_with("/btc.png"));
        assert!(src("AAPL").unwrap().ends_with("/AAPL.png"));
        assert!(src("RELIANCE").unwrap().ends_with("/RELIANCE.NS.png"));
        assert_eq!(src("EURUSD"), None);
    }
}

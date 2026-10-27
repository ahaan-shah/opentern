//! First run: the logo and a Begin button, then the markets you trade
//! (each becomes a watchlist of its top seven), then a first chart picked
//! from them, which grows into the app. Shown once, for a new workspace
//! only. Motion and layout follow Magpie's onboarding; colours are the
//! logo's greens whatever the theme.

use crate::app::App;
use crate::icons::ph;
use crate::motion;
use crate::theme::{self, Theme};
use crate::watchlists::{Item, Watchlist};
use egui::{Align2, Color32, CornerRadius, Id, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use opentern_core::{AssetKind, Symbol};
use opentern_data::Request;
use std::collections::HashMap;

/// The logo's greens (packaging/linux/opentern.svg).
const GREEN: Color32 = Color32::from_rgb(0x22, 0xB5, 0x6C);
const GREEN_DEEP: Color32 = Color32::from_rgb(0x07, 0x83, 0x4A);

/// The settings key set once onboarding is done (or skipped for an
/// existing workspace).
pub const DONE_KEY: &str = "onboarded";

/// Fetch sequence numbers for the cards' sparklines.
pub const SEQ_BASE: u64 = 1 << 44;

/// Leaving, in beats: the rest of the screen folds away around the picked
/// card (`FOLD`; onboarding still covers the app). Once the chart has bars
/// (or after `WAIT`) the card turns into a window onto the chart and opens
/// out to fill the chart's place, its face dissolving and its logo and
/// ticker gliding to the legend (`GROW`). Then the rest of the app fades in
/// around it (`SETTLE`).
const FOLD: f32 = 0.45;
const WAIT: f32 = 1.5;
const GROW: f32 = 0.75;
const SETTLE: f32 = 0.45;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Market {
    Crypto,
    Us,
    India,
    Forex,
    Commodities,
}

impl Market {
    pub const ALL: [Market; 5] = [
        Market::Crypto,
        Market::Us,
        Market::India,
        Market::Forex,
        Market::Commodities,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Market::Crypto => "Crypto",
            Market::Us => "US stocks",
            Market::India => "Indian stocks",
            Market::Forex => "Forex",
            Market::Commodities => "Commodities",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Market::Crypto => ph::CURRENCY_BTC,
            Market::Us => ph::CURRENCY_DOLLAR,
            Market::India => ph::CURRENCY_INR,
            Market::Forex => ph::ARROWS_LEFT_RIGHT,
            Market::Commodities => ph::DROP,
        }
    }

    /// The market's seven most traded (liquid, well known) symbols.
    pub fn symbols(self) -> Vec<Symbol> {
        let s = |id: &str, description: &str, kind, currency: &str, tick_size: f64| Symbol {
            id: id.into(),
            description: description.into(),
            kind,
            currency: currency.into(),
            tick_size,
            min_qty: 0.0,
            timezone: match id.split(':').next() {
                Some("NSE" | "BSE") => "Asia/Kolkata",
                Some("NASDAQ" | "NYSE" | "AMEX" | "COMEX" | "NYMEX" | "ICEEUR" | "CBOT") => "America/New_York",
                _ => "UTC",
            }
            .into(),
        };
        use AssetKind::*;
        match self {
            Market::Crypto => [
                ("BTC", "Bitcoin"),
                ("ETH", "Ethereum"),
                ("SOL", "Solana"),
                ("BNB", "BNB"),
                ("XRP", "XRP"),
                ("DOGE", "Dogecoin"),
                ("ADA", "Cardano"),
            ]
            .iter()
            .map(|(t, n)| {
                s(
                    &format!("BINANCE:{t}USDT"),
                    &format!("{n} / TetherUS"),
                    Crypto,
                    "USDT",
                    if *t == "DOGE" || *t == "ADA" || *t == "XRP" {
                        0.0001
                    } else {
                        0.01
                    },
                )
            })
            .collect(),
            Market::Us => [
                ("AAPL", "Apple Inc."),
                ("MSFT", "Microsoft Corporation"),
                ("NVDA", "NVIDIA Corporation"),
                ("AMZN", "Amazon.com, Inc."),
                ("GOOGL", "Alphabet Inc."),
                ("META", "Meta Platforms, Inc."),
                ("TSLA", "Tesla, Inc."),
            ]
            .iter()
            .map(|(t, n)| s(&format!("NASDAQ:{t}"), n, Stock, "USD", 0.01))
            .collect(),
            Market::India => [
                ("RELIANCE", "Reliance Industries"),
                ("HDFCBANK", "HDFC Bank"),
                ("TCS", "Tata Consultancy Services"),
                ("BHARTIARTL", "Bharti Airtel"),
                ("ICICIBANK", "ICICI Bank"),
                ("INFY", "Infosys"),
                ("SBIN", "State Bank of India"),
            ]
            .iter()
            .map(|(t, n)| s(&format!("NSE:{t}"), n, Stock, "INR", 0.05))
            .collect(),
            Market::Forex => [
                ("EURUSD", "Euro / U.S. Dollar", 0.00001),
                ("GBPUSD", "British Pound / U.S. Dollar", 0.00001),
                ("USDJPY", "U.S. Dollar / Japanese Yen", 0.001),
                ("USDCHF", "U.S. Dollar / Swiss Franc", 0.00001),
                ("AUDUSD", "Australian Dollar / U.S. Dollar", 0.00001),
                ("USDCAD", "U.S. Dollar / Canadian Dollar", 0.00001),
                ("NZDUSD", "New Zealand Dollar / U.S. Dollar", 0.00001),
            ]
            .iter()
            .map(|(t, n, tick)| s(&format!("FX:{t}"), n, Forex, &t[3..], *tick))
            .collect(),
            Market::Commodities => {
                let all = opentern_core::commodities();
                [
                    "COMEX:GC1!",
                    "COMEX:SI1!",
                    "NYMEX:CL1!",
                    "ICEEUR:BRN1!",
                    "NYMEX:NG1!",
                    "COMEX:HG1!",
                    "CBOT:ZC1!",
                ]
                .iter()
                .filter_map(|id| all.iter().find(|s| s.id == *id).cloned())
                .collect()
            }
        }
    }
}

#[derive(Default)]
pub struct Onboarding {
    shown_at: Option<f64>,
    /// 0 welcome, 1 markets, 2 first chart.
    step: usize,
    markets: Vec<Market>,
    pick: Option<Symbol>,
    /// Daily closes for the cards, by symbol id.
    sparks: HashMap<String, Vec<f64>>,
    asked: bool,
    /// Set by Enter Opentern: the picked card grows into the chart.
    leaving_at: Option<f64>,
    /// Where the picked card was last frame.
    pick_rect: Option<Rect>,
    /// When the card started opening onto the chart.
    grow_at: Option<f64>,
}

impl Onboarding {
    /// Whether onboarding still covers the app (it shows through once the
    /// card starts growing).
    pub fn covers(&self, now: f64) -> bool {
        self.leaving_at.is_none_or(|at| ((now - at) as f32) < FOLD)
    }

    /// 0 → 1 over the fold, while leaving.
    fn fold(&self, now: f64) -> f32 {
        self.leaving_at.map_or(0.0, |at| {
            motion::ease_in_out(((now - at) as f32 / FOLD).clamp(0.0, 1.0))
        })
    }
}

/// Whether to onboard: a new workspace that hasn't been, never for tests or
/// screenshots (unless `OPENTERN_STILL=onboarding[:1|2]`).
pub fn wanted(store: &mut opentern_core::Store, fresh: bool) -> Option<Onboarding> {
    let still = std::env::var("OPENTERN_STILL").ok();
    let asked = still
        .as_deref()
        .and_then(|v| v.split(',').find_map(|p| p.strip_prefix("onboarding")));
    if let Some(step) = asked {
        let step = step.trim_start_matches(':').parse().unwrap_or(0usize).min(2);
        let markets = if step > 0 {
            vec![Market::Crypto, Market::Us, Market::India]
        } else {
            Vec::new()
        };
        let pick = (step > 1).then(|| Market::Crypto.symbols().remove(0));
        return Some(Onboarding {
            step,
            markets,
            pick,
            ..Default::default()
        });
    }
    if still.is_some() || crate::app::headless() || store.value(DONE_KEY).is_some() {
        return None;
    }
    if !fresh || !store.watchlists().is_empty() {
        // A workspace from before onboarding existed.
        let _ = store.set_value(DONE_KEY, "1");
        return None;
    }
    Some(Onboarding::default())
}

impl App {
    /// Runs while onboarding covers the window.
    pub(crate) fn onboarding(&mut self, ui: &mut Ui) {
        let t = self.t();
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let Some(st) = self.onboarding.as_mut() else { return };
        let shown = *st.shown_at.get_or_insert(now);
        if st.step == 2 && !st.asked {
            st.asked = true;
            self.ask_sparks();
        }
        let Some(st) = self.onboarding.as_mut() else { return };
        let fold = st.fold(now);
        if st.leaving_at.is_some() {
            ctx.request_repaint();
        }
        let mut act = Act::None;
        egui::CentralPanel::no_frame()
            .frame(egui::Frame::new().fill(t.bg))
            .show(ui, |ui| {
                let full = ui.max_rect();
                let glow = motion::appear(&ctx, shown, 0.0, 1.4) * (1.0 - fold);
                radial_glow(
                    ui.painter(),
                    full.center() - vec2(0.0, full.height() * 0.18),
                    full.width().max(full.height()) * 0.55,
                    motion::with_alpha(GREEN, if t.dark { 0.13 } else { 0.10 } * glow),
                );
                for i in 0..3 {
                    let k = motion::toggle(&ctx, Id::new(("onb-step", i)), st.step == i, motion::EMPHASIS);
                    if k < 0.001 {
                        continue;
                    }
                    // Steps rise in from below and leave upwards.
                    let dy = if st.step >= i {
                        (1.0 - k) * 36.0
                    } else {
                        -(1.0 - k) * 36.0
                    };
                    // Leaving: everything but the picked card sinks away.
                    let dy = dy + fold * 14.0;
                    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(full.translate(vec2(0.0, dy))));
                    c.set_opacity(k * (1.0 - fold));
                    if st.step != i || st.leaving_at.is_some() {
                        c.disable();
                    }
                    let a = match i {
                        0 => welcome(&mut c, &t, shown),
                        1 => markets(&mut c, &t, st, &self.logos),
                        _ => first_chart(&mut c, &t, st, &self.logos, &self.quotes),
                    };
                    if st.step == i && !matches!(a, Act::None) {
                        act = a;
                    }
                }
            });
        if let Some(st) = &self.onboarding
            && st.leaving_at.is_some()
            && let (Some(s), Some(r)) = (&st.pick, st.pick_rect)
        {
            // The picked card lifts towards the viewer as the rest goes.
            let r = Rect::from_center_size(r.center() - vec2(0.0, 6.0 * fold), r.size() * (1.0 + 0.06 * fold));
            let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("onb-fly")));
            let card = Fly {
                rect: r,
                radius: 12.0,
                lift: fold,
                shadow: fold,
                fill: 1.0,
                detail: 1.0,
                logo: pos2(r.left() + 22.0, r.top() + 22.0),
                logo_r: 10.0,
                label: 1.0,
                ticker: 1.0,
            };
            card.paint(&p, &t, &self.logos, s, st.sparks.get(&s.id), self.quotes.get(&s.id));
        }
        match act {
            Act::None => {}
            Act::Step(s) => {
                if let Some(st) = self.onboarding.as_mut() {
                    st.step = s;
                    if s == 2 && st.pick.as_ref().is_none_or(|p| !in_markets(&st.markets, p)) {
                        st.pick = st.markets.first().map(|m| m.symbols().remove(0));
                    }
                }
            }
            Act::Enter => self.finish_onboarding(now),
        }
    }

    /// Daily bars for every card's sparkline, and live quotes for them.
    fn ask_sparks(&mut self) {
        let Some(st) = &self.onboarding else { return };
        let symbols: Vec<Symbol> = st.markets.iter().flat_map(|m| m.symbols()).collect();
        let day = opentern_core::Timeframe::parse("1D").expect("valid");
        for (k, s) in symbols.iter().enumerate() {
            self.hub.send(Request::Fetch {
                seq: SEQ_BASE + k as u64,
                symbol: s.clone(),
                tf: day,
                count: 60,
            });
        }
        self.hub.send(Request::Quotes { symbols });
    }

    pub(crate) fn onboarding_fetched(&mut self, symbol: &str, result: Result<Vec<opentern_core::Bar>, String>) {
        if let (Some(st), Ok(bars)) = (self.onboarding.as_mut(), result) {
            let closes: Vec<f64> = bars.iter().rev().take(45).rev().map(|b| b.close).collect();
            st.sparks.insert(symbol.to_string(), closes);
        }
    }

    /// Enter Opentern: a watchlist per market, the picked chart opened
    /// behind, and the card starts growing into it.
    fn finish_onboarding(&mut self, now: f64) {
        let Some(st) = self.onboarding.as_mut() else { return };
        let Some(pick) = st.pick.clone() else { return };
        st.leaving_at = Some(now);
        let markets = st.markets.clone();
        self.watchlists = markets
            .iter()
            .enumerate()
            .map(|(k, m)| Watchlist {
                id: k as i64 + 1,
                name: m.name().into(),
                items: m.symbols().into_iter().map(Item::Symbol).collect(),
            })
            .collect();
        self.active_list = markets
            .iter()
            .position(|m| m.symbols().iter().any(|s| s.id == pick.id))
            .unwrap_or(0);
        self.save_watchlists();
        let _ = self.store.set_value(crate::onboarding::DONE_KEY, "1");
        if pick.id != self.chart.symbol.id {
            self.open_symbol(pick);
        }
    }

    /// While leaving, over the app: the picked card becomes a window onto
    /// the chart and opens out to the chart's place, then the rest of the
    /// app fades in around it.
    pub(crate) fn onboarding_overlay(&mut self, ctx: &egui::Context) {
        let t = self.t();
        let now = ctx.input(|i| i.time);
        let has_bars = !self.chart.bars.is_empty();
        let Some(st) = self.onboarding.as_mut() else { return };
        let Some(at) = st.leaving_at else { return };
        let u = (now - at) as f32 - FOLD;
        if u < 0.0 {
            return;
        }
        ctx.request_repaint();
        if st.grow_at.is_none() && (has_bars || u >= WAIT) {
            st.grow_at = Some(now);
        }
        let since = st.grow_at.map_or(0.0, |g| (now - g) as f32);
        let settle = ((since - GROW) / SETTLE).clamp(0.0, 1.0);
        if settle >= 1.0 {
            self.onboarding = None;
            return;
        }
        let Some(st) = &self.onboarding else { return };
        let Some(s) = &st.pick else { return };
        let screen = ctx.content_rect();
        let to = if self.grid_rect.is_positive() {
            self.grid_rect
        } else {
            screen
        };
        let pane = if self.chart_ui.main_rect.is_positive() {
            self.chart_ui.main_rect
        } else {
            to
        };
        let from = st
            .pick_rect
            .map(|r| Rect::from_center_size(r.center() - vec2(0.0, 6.0), r.size() * 1.06))
            .unwrap_or_else(|| Rect::from_center_size(screen.center(), vec2(160.0, 104.0)));
        let g = motion::ease_in_out((since / GROW).clamp(0.0, 1.0));
        let hole = Rect::from_min_max(from.min.lerp(to.min, g), from.max.lerp(to.max, g));
        let radius = motion::lerp(12.0, 9.0, g);
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("onb-fly")));
        // The backdrop everywhere but the window, until the app settles in.
        let back = motion::with_alpha(t.bg, 1.0 - motion::ease_in_out(settle));
        for r in [
            Rect::from_min_max(screen.min, pos2(screen.right(), hole.top())),
            Rect::from_min_max(pos2(screen.left(), hole.bottom()), screen.max),
            Rect::from_min_max(pos2(screen.left(), hole.top()), pos2(hole.left(), hole.bottom())),
            Rect::from_min_max(pos2(hole.right(), hole.top()), pos2(screen.right(), hole.bottom())),
        ] {
            if r.is_positive() {
                p.rect_filled(r, 0.0, back);
            }
        }
        crate::shell::round_corners(&p, hole, radius, back);
        // While it waits for bars, the lifted card breathes.
        let breathe = if st.grow_at.is_none() {
            0.75 + 0.25 * (u * 4.0).sin()
        } else {
            1.0
        };
        // The card's face dissolves early, showing the chart through it.
        let face = 1.0 - motion::ease_out((since / (GROW * 0.45)).clamp(0.0, 1.0));
        // The ticker goes before it would cross the real legend; the logo
        // flies on and lands on the legend's own.
        let ticker = 1.0 - motion::ease_out(((since - GROW * 0.2) / (GROW * 0.4)).clamp(0.0, 1.0));
        let card = Fly {
            rect: hole,
            radius,
            lift: (1.0 - g) * breathe,
            // Fades with the face (it would darken the window otherwise).
            shadow: face * face,
            fill: face,
            detail: face,
            logo: pos2(from.left() + 22.0 * 1.06, from.top() + 22.0 * 1.06).lerp(pane.left_top() + vec2(19.0, 16.5), g),
            logo_r: motion::lerp(10.0, 9.0, g),
            label: 1.0,
            ticker,
        };
        card.paint(&p, &t, &self.logos, s, st.sparks.get(&s.id), self.quotes.get(&s.id));
        // A green rim that settles into the chart card's own edge.
        p.rect_stroke(
            hole,
            CornerRadius::same(radius.round() as u8),
            Stroke::new(1.5, motion::with_alpha(GREEN, (1.0 - g) * breathe)),
            egui::StrokeKind::Inside,
        );
    }
}

/// The picked card in flight: its frame, the logo and ticker (which travel
/// to the legend) and the rest of its face (which fades first).
struct Fly {
    rect: Rect,
    radius: f32,
    /// 1 = lifted off the page (green edge and tint), 0 = landed.
    lift: f32,
    /// How much shadow and glow it casts (none once it's a window).
    shadow: f32,
    fill: f32,
    detail: f32,
    logo: Pos2,
    logo_r: f32,
    /// The logo shows (it needn't fade: it lands on the legend's own).
    label: f32,
    ticker: f32,
}

impl Fly {
    fn paint(
        &self,
        p: &egui::Painter,
        t: &Theme,
        logos: &crate::logos::Logos,
        s: &Symbol,
        spark: Option<&Vec<f64>>,
        quote: Option<&(f64, f64)>,
    ) {
        let r = self.rect;
        let radius = CornerRadius::same(self.radius.round() as u8);
        if self.shadow > 0.01 {
            let shadow = egui::Shadow {
                offset: [0, 14],
                blur: 40,
                spread: 0,
                color: motion::with_alpha(Color32::BLACK, if t.dark { 0.45 } else { 0.14 } * self.shadow),
            };
            p.add(shadow.as_shape(r, radius));
            let glow = egui::Shadow {
                offset: [0, 0],
                blur: 28,
                spread: 0,
                color: motion::with_alpha(GREEN, 0.28 * self.shadow),
            };
            p.add(glow.as_shape(r, radius));
        }
        // Landing, it takes the chart's own colours so the swap is seamless.
        let fill = motion::lerp_color(t.card, motion::lerp_color(t.card, GREEN, 0.08), self.lift);
        let edge = motion::lerp_color(t.border, GREEN, self.lift);
        p.rect(
            r,
            radius,
            motion::with_alpha(fill, self.fill),
            Stroke::new(
                1.0 + 0.5 * self.lift,
                motion::with_alpha(edge, self.fill * (0.4 + 0.6 * self.lift)),
            ),
            egui::StrokeKind::Inside,
        );
        if self.label > 0.01 {
            // Landed, it sits exactly on the legend's own logo, so it needn't
            // fade (logos paint at full alpha).
            logos.paint(p, t, s, self.logo, self.logo_r);
            p.text(
                self.logo + vec2(self.logo_r + 6.0, 0.0),
                Align2::LEFT_CENTER,
                s.ticker(),
                theme::semibold(13.5),
                motion::with_alpha(t.text, self.ticker),
            );
        }
        let a = self.detail * self.fill;
        if a < 0.01 {
            return;
        }
        p.text(
            pos2(r.left() + 12.0, r.top() + 42.0),
            Align2::LEFT_CENTER,
            elide(p, &s.description, theme::regular(11.5), r.width() - 24.0),
            theme::regular(11.5),
            motion::with_alpha(t.text3, a),
        );
        if let Some(c) = spark {
            let spark = Rect::from_min_max(
                pos2(r.left() + 12.0, r.top() + 58.0),
                pos2(r.right() - 12.0, r.top() + 78.0),
            );
            sparkline(p, t, spark, c, a);
        }
        if let Some((last, prev)) = quote
            && *prev != 0.0
        {
            let pct = (last / prev - 1.0) * 100.0;
            let y = r.top() + 90.0;
            p.text(
                pos2(r.left() + 12.0, y),
                Align2::LEFT_CENTER,
                crate::format::price(*last, s.precision()),
                theme::regular(11.5),
                motion::with_alpha(t.text2, a),
            );
            p.text(
                pos2(r.right() - 12.0, y),
                Align2::RIGHT_CENTER,
                crate::format::signed_pct(pct),
                theme::regular(11.5),
                motion::with_alpha(if pct >= 0.0 { t.pos } else { t.neg }, a),
            );
        }
    }
}

enum Act {
    None,
    Step(usize),
    Enter,
}

fn in_markets(markets: &[Market], s: &Symbol) -> bool {
    markets.iter().any(|m| m.symbols().iter().any(|x| x.id == s.id))
}

/// Step 1: the logo, a line, Begin.
fn welcome(ui: &mut Ui, t: &Theme, shown: f64) -> Act {
    let ctx = ui.ctx().clone();
    let full = ui.max_rect();
    let logo = motion::appear(&ctx, shown, 0.1, 0.9);
    let words = motion::appear(&ctx, shown, 0.45, 0.7);
    let button = motion::appear(&ctx, shown, 0.75, 0.6);
    let c = full.center() - vec2(0.0, 70.0);
    let size = 112.0 * (0.86 + 0.14 * motion::ease_out_back(logo));
    let tex = crate::settings::app_logo(&ctx);
    let p = ui.painter();
    p.image(
        tex.id(),
        Rect::from_center_size(c, vec2(size, size)),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        motion::with_alpha(Color32::WHITE, logo),
    );
    let y = c.y + 84.0 + (1.0 - words) * 10.0;
    p.text(
        pos2(c.x, y),
        Align2::CENTER_TOP,
        "Opentern",
        theme::display(40.0),
        motion::with_alpha(t.text, words),
    );
    p.text(
        pos2(c.x, y + 56.0),
        Align2::CENTER_TOP,
        "Charts, scripts and backtests. All on your own machine.",
        theme::regular(16.0),
        motion::with_alpha(t.text2, words),
    );
    let r = Rect::from_center_size(pos2(c.x, y + 130.0 + (1.0 - button) * 8.0), vec2(168.0, 44.0));
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(r));
    // Multiplied, so the button fades with its step when Begin is pressed.
    ui.multiply_opacity(button);
    let go = cta(&mut ui, "onb-begin", r, "Begin", true);
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    if go || (enter && button > 0.5) {
        Act::Step(1)
    } else {
        Act::None
    }
}

/// Step 2: a tile per market, pick any.
fn markets(ui: &mut Ui, t: &Theme, st: &mut Onboarding, logos: &crate::logos::Logos) -> Act {
    let full = ui.max_rect();
    let n = Market::ALL.len() as f32;
    let gap = 14.0;
    let tile_w = ((full.width() - 96.0 - gap * (n - 1.0)) / n).min(184.0);
    let tile = vec2(tile_w, 196.0);
    let row_w = tile.x * n + gap * (n - 1.0);
    let top = full.center().y - 190.0;
    let p = ui.painter().clone();
    p.text(
        pos2(full.center().x, top),
        Align2::CENTER_TOP,
        "What do you trade?",
        theme::display(30.0),
        t.text,
    );
    let mut x = full.center().x - row_w / 2.0;
    let y = top + 82.0;
    for m in Market::ALL {
        let r = Rect::from_min_size(pos2(x, y), tile);
        x += tile.x + gap;
        let on = st.markets.contains(&m);
        let resp = ui.interact(r, Id::new(("onb-market", m)), Sense::click());
        let h = motion::toggle(ui.ctx(), resp.id.with("h"), resp.hovered(), motion::MICRO);
        let s = motion::toggle(ui.ctx(), resp.id.with("s"), on, motion::STANDARD);
        let r = r.translate(vec2(0.0, -3.0 * h));
        tile_frame(&p, t, r, h, s);
        // The market's sign in a soft disc.
        let c = pos2(r.center().x, r.top() + 54.0);
        p.circle_filled(c, 28.0, motion::lerp_color(t.hover, motion::with_alpha(GREEN, 0.18), s));
        p.text(
            c,
            Align2::CENTER_CENTER,
            m.glyph(),
            egui::FontId::proportional(26.0),
            motion::lerp_color(t.text2, if t.dark { GREEN } else { GREEN_DEEP }, s),
        );
        p.text(
            pos2(r.center().x, r.top() + 100.0),
            Align2::CENTER_TOP,
            m.name(),
            theme::semibold(15.0),
            t.text,
        );
        // Its first few symbols' logos, overlapping.
        let syms = m.symbols();
        let k = 5;
        let lr = 13.0;
        let step = 21.0;
        let mut lx = r.center().x - step * (k - 1) as f32 / 2.0;
        for s in syms.iter().take(k) {
            let lc = pos2(lx, r.bottom() - 34.0);
            p.circle_filled(lc, lr + 2.0, motion::lerp_color(t.card, t.hovered(t.card), h));
            logos.paint(&p, t, s, lc, lr);
            lx += step;
        }
        check(&p, pos2(r.right() - 18.0, r.top() + 18.0), s);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            if on {
                st.markets.retain(|x| *x != m);
            } else {
                st.markets.push(m);
                st.markets.sort_by_key(|x| Market::ALL.iter().position(|a| a == x));
            }
        }
    }
    let by = y + tile.y + 48.0;
    let ready = !st.markets.is_empty();
    let mut act = Act::None;
    if back(
        ui,
        t,
        Rect::from_center_size(pos2(full.center().x - 130.0, by + 22.0), vec2(96.0, 44.0)),
    ) {
        act = Act::Step(0);
    }
    let r = Rect::from_center_size(pos2(full.center().x + 50.0, by + 22.0), vec2(168.0, 44.0));
    if cta(ui, "onb-continue", r, "Continue", ready) || (ready && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
        act = Act::Step(2);
    }
    act
}

/// Step 3: the chosen markets' symbols as cards, a row per market.
fn first_chart(
    ui: &mut Ui,
    t: &Theme,
    st: &mut Onboarding,
    logos: &crate::logos::Logos,
    quotes: &HashMap<String, (f64, f64)>,
) -> Act {
    let full = ui.max_rect();
    let width = (full.width() - 96.0).min(1120.0);
    let left = full.center().x - width / 2.0;
    let p = ui.painter().clone();
    p.text(
        pos2(full.center().x, full.top() + 48.0),
        Align2::CENTER_TOP,
        "Pick your first chart",
        theme::display(30.0),
        t.text,
    );
    let foot = 100.0;
    let body = Rect::from_min_max(
        pos2(left - 8.0, full.top() + 112.0),
        pos2(left + width + 8.0, full.bottom() - foot),
    );
    let mut act = Act::None;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
    egui::ScrollArea::vertical()
        .id_salt("onb-cards")
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let gap = 10.0;
            let card_w = (width - gap * 6.0) / 7.0;
            let card = vec2(card_w, 104.0);
            // Rows centred when they don't fill the space.
            let rows = st.markets.len() as f32 * (card.y + 44.0);
            ui.add_space(((body.height() - rows) / 2.0 - 8.0).max(4.0));
            for m in st.markets.clone() {
                let (head, _) = ui.allocate_exact_size(vec2(width + 16.0, 30.0), Sense::hover());
                ui.painter().text(
                    pos2(head.left() + 8.0, head.center().y),
                    Align2::LEFT_CENTER,
                    format!("{}  {}", m.glyph(), m.name()),
                    theme::medium(13.0),
                    t.text2,
                );
                let (row, _) = ui.allocate_exact_size(vec2(width + 16.0, card.y + 14.0), Sense::hover());
                let mut x = row.left() + 8.0;
                for s in m.symbols() {
                    let r = Rect::from_min_size(pos2(x, row.top() + 4.0), card);
                    x += card.x + gap;
                    let on = st.pick.as_ref().is_some_and(|p| p.id == s.id);
                    let resp = ui.interact(r, Id::new(("onb-card", &s.id)), Sense::click());
                    let h = motion::toggle(ui.ctx(), resp.id.with("h"), resp.hovered(), motion::MICRO);
                    let k = motion::toggle(ui.ctx(), resp.id.with("s"), on, motion::STANDARD);
                    let r = r.translate(vec2(0.0, -2.0 * h));
                    if on {
                        st.pick_rect = Some(r);
                    }
                    if on && st.leaving_at.is_some() {
                        continue;
                    }
                    let p = ui.painter();
                    tile_frame(p, t, r, h, k);
                    logos.paint(p, t, &s, pos2(r.left() + 22.0, r.top() + 22.0), 10.0);
                    p.text(
                        pos2(r.left() + 38.0, r.top() + 22.0),
                        Align2::LEFT_CENTER,
                        s.ticker(),
                        theme::semibold(13.5),
                        t.text,
                    );
                    let name = elide(p, &s.description, theme::regular(11.5), r.width() - 24.0);
                    p.text(
                        pos2(r.left() + 12.0, r.top() + 42.0),
                        Align2::LEFT_CENTER,
                        name,
                        theme::regular(11.5),
                        t.text3,
                    );
                    let spark = Rect::from_min_max(
                        pos2(r.left() + 12.0, r.top() + 58.0),
                        pos2(r.right() - 12.0, r.bottom() - 26.0),
                    );
                    if let Some(c) = st.sparks.get(&s.id) {
                        sparkline(p, t, spark, c, 1.0);
                    }
                    if let Some((last, prev)) = quotes.get(&s.id)
                        && *prev != 0.0
                    {
                        let pct = (last / prev - 1.0) * 100.0;
                        let y = r.bottom() - 14.0;
                        p.text(
                            pos2(r.left() + 12.0, y),
                            Align2::LEFT_CENTER,
                            crate::format::price(*last, s.precision()),
                            theme::regular(11.5),
                            t.text2,
                        );
                        p.text(
                            pos2(r.right() - 12.0, y),
                            Align2::RIGHT_CENTER,
                            crate::format::signed_pct(pct),
                            theme::regular(11.5),
                            if pct >= 0.0 { t.pos } else { t.neg },
                        );
                    }
                    if resp.clicked() {
                        st.pick = Some(s.clone());
                    }
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).double_clicked() {
                        act = Act::Enter;
                    }
                }
                ui.add_space(14.0);
            }
        });
    let by = full.bottom() - foot / 2.0;
    if back(
        ui,
        t,
        Rect::from_center_size(pos2(full.center().x - 130.0, by), vec2(96.0, 44.0)),
    ) {
        act = Act::Step(1);
    }
    let ready = st.pick.is_some();
    let r = Rect::from_center_size(pos2(full.center().x + 50.0, by), vec2(168.0, 44.0));
    if cta(ui, "onb-enter", r, "Enter Opentern", ready) || (ready && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
        act = Act::Enter;
    }
    act
}

/// A tile's card: lifts on hover, turns green when picked.
fn tile_frame(p: &egui::Painter, t: &Theme, r: Rect, hover: f32, on: f32) {
    let shadow = egui::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: motion::with_alpha(Color32::BLACK, if t.dark { 0.22 } else { 0.06 } * (0.4 + 0.6 * hover)),
    };
    p.add(shadow.as_shape(r, CornerRadius::same(12)));
    let fill = motion::lerp_color(
        motion::lerp_color(t.card, t.hovered(t.card), hover),
        motion::lerp_color(t.card, GREEN, if t.dark { 0.10 } else { 0.06 }),
        on,
    );
    let edge = motion::lerp_color(t.border, GREEN, on);
    p.rect(
        r,
        CornerRadius::same(12),
        fill,
        Stroke::new(1.0 + on * 0.5, edge),
        egui::StrokeKind::Inside,
    );
}

/// A ticked circle in a picked tile's corner.
fn check(p: &egui::Painter, c: Pos2, on: f32) {
    if on < 0.01 {
        return;
    }
    p.circle_filled(c, 10.0 * motion::ease_out_back(on), GREEN);
    p.text(
        c,
        Align2::CENTER_CENTER,
        ph::CHECK,
        egui::FontId::proportional(12.0),
        motion::with_alpha(Color32::WHITE, on),
    );
}

/// The main button, in the logo's green. Hovered, it brightens, lifts a
/// little with a green glow, and an arrow slides in after the label.
fn cta(ui: &mut Ui, id: &str, r: Rect, label: &str, enabled: bool) -> bool {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let resp = ui.interact(r, Id::new(id), sense);
    let h = motion::toggle(ui.ctx(), resp.id.with("h"), enabled && resp.hovered(), motion::STANDARD);
    let down = enabled && resp.is_pointer_button_down_on();
    let p = ui.painter();
    let fade = if enabled { 1.0 } else { 0.4 };
    let scale = 1.0 + 0.04 * h - if down { 0.03 } else { 0.0 };
    let r = Rect::from_center_size(r.center() - vec2(0.0, 2.0 * h), r.size() * scale);
    if h > 0.01 {
        let glow = egui::Shadow {
            offset: [0, 6],
            blur: 24,
            spread: 0,
            color: motion::with_alpha(GREEN, 0.45 * h),
        };
        p.add(glow.as_shape(r, CornerRadius::same(22)));
    }
    let fill = motion::lerp_color(GREEN_DEEP, GREEN, 0.2 + 0.8 * h);
    p.rect_filled(r, CornerRadius::same(22), motion::with_alpha(fill, fade));
    // A soft top highlight, stronger on hover.
    p.rect_stroke(
        r,
        CornerRadius::same(22),
        Stroke::new(1.0, motion::with_alpha(Color32::WHITE, 0.10 + 0.25 * h)),
        egui::StrokeKind::Inside,
    );
    let font = theme::semibold(15.0);
    let w = p
        .layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE)
        .size()
        .x;
    let arrow = 18.0 * h;
    let x = r.center().x - (w + arrow) / 2.0;
    p.text(
        pos2(x, r.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        motion::with_alpha(Color32::WHITE, fade),
    );
    if h > 0.01 {
        p.text(
            pos2(x + w + 6.0 + 4.0 * h, r.center().y),
            Align2::LEFT_CENTER,
            ph::ARROW_RIGHT,
            egui::FontId::proportional(15.0),
            motion::with_alpha(Color32::WHITE, h * fade),
        );
    }
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
    } else {
        false
    }
}

/// A quiet Back button.
fn back(ui: &mut Ui, t: &Theme, r: Rect) -> bool {
    let resp = ui.interact(r, Id::new(("onb-back", r.min.x as i32)), Sense::click());
    let h = motion::toggle(ui.ctx(), resp.id.with("h"), resp.hovered(), motion::MICRO);
    let p = ui.painter();
    p.rect_filled(r, CornerRadius::same(22), motion::with_alpha(t.hover, h));
    p.text(
        r.center(),
        Align2::CENTER_CENTER,
        format!("{}  Back", ph::ARROW_LEFT),
        theme::medium(14.0),
        motion::lerp_color(t.text2, t.text, h),
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn sparkline(p: &egui::Painter, t: &Theme, r: Rect, closes: &[f64], alpha: f32) {
    if closes.len() < 2 {
        return;
    }
    let (lo, hi) = closes
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &c| (a.min(c), b.max(c)));
    let span = (hi - lo).max(hi.abs() * 1e-9).max(f64::MIN_POSITIVE);
    let n = closes.len() - 1;
    let pts: Vec<Pos2> = closes
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            pos2(
                r.left() + r.width() * i as f32 / n as f32,
                r.bottom() - r.height() * ((c - lo) / span) as f32,
            )
        })
        .collect();
    let up = closes[n] >= closes[0];
    let color = if up { t.pos } else { t.neg };
    p.add(egui::Shape::line(
        pts,
        Stroke::new(1.5, motion::with_alpha(color, alpha)),
    ));
}

/// Text cut to fit `width`, with an ellipsis.
fn elide(p: &egui::Painter, text: &str, font: egui::FontId, width: f32) -> String {
    let fits = |s: &str| p.layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE).size().x <= width;
    if fits(text) {
        return text.to_string();
    }
    let mut s: String = text.to_string();
    while !s.is_empty() {
        s.pop();
        let cut = format!("{}…", s.trim_end());
        if fits(&cut) {
            return cut;
        }
    }
    String::new()
}

/// A soft round glow (Magpie's `widgets::radial_glow`).
fn radial_glow(p: &egui::Painter, center: Pos2, radius: f32, color: Color32) {
    use egui::epaint::Mesh;
    let rings = 24;
    let segs = 64;
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, color);
    for r in 1..=rings {
        let k = r as f32 / rings as f32;
        // Smooth falloff so there's no visible edge.
        let a = (1.0 - k).powf(2.2);
        for s in 0..segs {
            let ang = s as f32 / segs as f32 * std::f32::consts::TAU;
            mesh.colored_vertex(
                center + vec2(ang.cos(), ang.sin()) * radius * k,
                color.gamma_multiply(a),
            );
        }
    }
    for s in 0..segs as u32 {
        let n = (s + 1) % segs as u32;
        mesh.add_triangle(0, 1 + s, 1 + n);
    }
    for r in 0..(rings - 1) as u32 {
        let a0 = 1 + r * segs as u32;
        let b0 = a0 + segs as u32;
        for s in 0..segs as u32 {
            let n = (s + 1) % segs as u32;
            mesh.add_triangle(a0 + s, b0 + s, b0 + n);
            mesh.add_triangle(a0 + s, b0 + n, a0 + n);
        }
    }
    p.add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seven_symbols_per_market() {
        for m in Market::ALL {
            let s = m.symbols();
            assert_eq!(s.len(), 7, "{}", m.name());
            assert!(s.iter().all(|s| !s.description.is_empty()));
        }
    }
}

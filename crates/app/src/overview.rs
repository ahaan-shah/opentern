//! The market overview: a heatmap of a whole market (the S&P 500, the
//! NIFTY 500 or the largest coins) that takes the place of the charts and
//! the bottom panel. Tiles are sized by market cap, grouped by sector and
//! coloured by the change over the picked period; a click opens the chart,
//! a sector's name zooms into it, Esc goes back. Data from
//! `opentern_data::overview` (`Request::Overview`).

use crate::app::App;
use crate::icons::ph;
use crate::motion::{self, lerp_color, with_alpha};
use crate::theme::{self, Theme};
use crate::widgets as w;
use egui::{Align, Align2, Color32, CornerRadius, Id, Layout, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use opentern_data::overview::{Board, Market, Member, Period, Sizing};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How often an open map asks for fresh numbers.
const REFRESH: Duration = Duration::from_secs(180);
/// After a failure.
const RETRY: Duration = Duration::from_secs(30);
const HEADER_H: f32 = 44.0;
const SECTOR_H: f32 = 18.0;

#[derive(Default)]
pub struct Overview {
    pub market: Market,
    pub period: Period,
    /// The sector zoomed into.
    pub sector: Option<String>,
    pub boards: HashMap<Market, Board>,
    pub errors: HashMap<Market, String>,
    /// When each market last failed.
    failed: HashMap<Market, Instant>,
    /// The request in flight per market, and when it was sent.
    asked: HashMap<Market, (u64, Instant)>,
    seq: u64,
    /// Screenshots: the tile to show as hovered (`overview-hover:<ticker>`).
    pub still_hover: Option<String>,
    /// Where last frame drew each tile (symbol id) and sector title, and
    /// the top-bar button (tests click them).
    pub(crate) drawn: std::cell::RefCell<Drawn>,
}

#[derive(Default)]
pub(crate) struct Drawn {
    pub tiles: Vec<(String, Rect)>,
    pub sectors: Vec<(String, Rect)>,
    pub button: Option<Rect>,
}

impl Overview {
    pub fn load(store: &opentern_core::Store) -> Overview {
        let v: serde_json::Value = store
            .value("overview")
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Overview {
            market: v["market"].as_str().and_then(Market::from_key).unwrap_or_default(),
            period: v["period"].as_str().and_then(Period::from_label).unwrap_or_default(),
            ..Default::default()
        }
    }

    fn json(&self) -> String {
        serde_json::json!({"market": self.market.key(), "period": self.period.label()}).to_string()
    }

    pub fn loading(&self, m: Market) -> bool {
        self.asked.contains_key(&m)
    }
}

/// How far the colour scale reaches per period (± this % is full colour).
pub fn scale(p: Period) -> f64 {
    match p {
        Period::Day => 3.0,
        Period::Week => 6.0,
        Period::Month => 10.0,
        Period::Quarter => 20.0,
        Period::Ytd | Period::Year => 30.0,
    }
}

/// A tile's colour: the neutral tile for no change, deepening towards the
/// theme's up or down colour.
pub fn fill(t: &Theme, change: f64, period: Period) -> Color32 {
    // Light themes get a mid-grey neutral, so white text reads on every
    // tile (as on TradingView's light heatmap).
    let neutral = if t.dark {
        lerp_color(t.card, t.border, 0.85)
    } else {
        lerp_color(t.border, t.text3, 0.6)
    };
    if !change.is_finite() || change.abs() < 0.005 {
        return neutral;
    }
    let k = (change.abs() / scale(period)).min(1.0).powf(0.8) as f32;
    let end = if change > 0.0 { t.pos } else { t.neg };
    let end = if t.dark {
        lerp_color(end, Color32::BLACK, 0.12)
    } else {
        end
    };
    let floor = if t.dark { 0.25 } else { 0.4 };
    lerp_color(neutral, end, floor + (1.0 - floor) * k)
}

/// White on dark fills, the theme's text on light ones.
fn ink(t: &Theme, fill: Color32) -> Color32 {
    let l = 0.2126 * fill.r() as f32 + 0.7152 * fill.g() as f32 + 0.0722 * fill.b() as f32;
    if l > 200.0 { t.text } else { Color32::WHITE }
}

/// What a tile shows: the coin, not the pair.
pub fn short(m: &Member) -> String {
    let t = m.symbol.ticker();
    if m.symbol.kind == opentern_core::AssetKind::Crypto {
        for q in ["USDT", "USDC", "USD"] {
            if let Some(b) = t.strip_suffix(q).filter(|b| !b.is_empty()) {
                return b.to_string();
            }
        }
    }
    t.to_string()
}

pub fn money(v: f64, currency: &str) -> String {
    if !v.is_finite() {
        return "—".into();
    }
    let sign = match currency {
        "INR" => "₹",
        "USD" => "$",
        _ => "",
    };
    let a = v.abs();
    let (d, s) = if a >= 1e12 {
        (1e12, "T")
    } else if a >= 1e9 {
        (1e9, "B")
    } else if a >= 1e6 {
        (1e6, "M")
    } else {
        (1.0, "")
    };
    format!("{sign}{:.2}{s}", v / d)
}

fn price(v: f64) -> String {
    let d = if v >= 1.0 {
        2
    } else if v >= 0.01 {
        4
    } else {
        8
    };
    crate::format::grouped(v, d)
}

/// A sector: its name, total size, and members with their sizes.
pub type Group<'a> = (String, f64, Vec<(&'a Member, f64)>);

/// Members grouped by sector, biggest sector first; each sector's
/// members biggest first. Weights are what tiles are sized by.
pub fn sectors<'a>(b: &'a Board, only: Option<&str>) -> Vec<Group<'a>> {
    let weight = |m: &Member| match b.sizing {
        Sizing::Equal => 1.0,
        _ if m.cap.is_finite() && m.cap > 0.0 => m.cap,
        _ => 0.0,
    };
    let mut groups: Vec<Group> = Vec::new();
    for m in &b.members {
        if only.is_some_and(|s| s != m.sector) {
            continue;
        }
        let wt = weight(m);
        if wt <= 0.0 {
            continue;
        }
        match groups.iter_mut().find(|g| g.0 == m.sector) {
            Some(g) => {
                g.1 += wt;
                g.2.push((m, wt));
            }
            None => groups.push((m.sector.clone(), wt, vec![(m, wt)])),
        }
    }
    for g in &mut groups {
        g.2.sort_by(|a, b| b.1.total_cmp(&a.1));
    }
    groups.sort_by(|a, b| b.1.total_cmp(&a.1));
    groups
}

/// A sector's change: its members' changes weighted by size.
pub fn weighted(members: &[(&Member, f64)], p: Period) -> f64 {
    let (mut s, mut w) = (0.0, 0.0);
    for (m, wt) in members {
        let c = m.change(p);
        if c.is_finite() {
            s += c * wt;
            w += wt;
        }
    }
    if w > 0.0 { s / w } else { f64::NAN }
}

/// The overview's top-bar button: a little treemap.
pub fn button(ui: &mut Ui, t: &Theme, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(7), with_alpha(t.hover_wash(), hover));
        let c = if active {
            t.accent
        } else {
            lerp_color(t.text2, t.text, hover.max(0.35))
        };
        let r = Rect::from_center_size(rect.center(), vec2(15.0, 15.0));
        let s = Stroke::new(1.3, c);
        p.rect_stroke(r, CornerRadius::same(2), s, egui::StrokeKind::Middle);
        let x = r.left() + r.width() * 0.55;
        p.vline(x, r.y_range(), s);
        p.hline(x..=r.right(), r.top() + r.height() * 0.48, s);
        p.hline(r.left()..=x, r.top() + r.height() * 0.62, s);
        let x2 = x + (r.right() - x) * 0.5;
        p.vline(x2, (r.top() + r.height() * 0.48)..=r.bottom(), s);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Market overview (Alt M)")
}

impl App {
    pub(crate) fn overview_open(&self) -> bool {
        self.layout.overview && self.settings.is_none()
    }

    pub(crate) fn set_overview(&mut self, open: bool) {
        self.layout.overview = open;
        if open {
            self.settings = None;
            self.overview_fetch(self.overview.market);
        }
    }

    pub(crate) fn save_overview(&mut self) {
        let _ = self.store.set_value("overview", &self.overview.json());
    }

    /// Asks for a market's board unless one is on its way.
    pub(crate) fn overview_fetch(&mut self, m: Market) {
        if self
            .overview
            .asked
            .get(&m)
            .is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(90))
        {
            return;
        }
        self.overview.seq += 1;
        let seq = self.overview.seq;
        self.overview.asked.insert(m, (seq, Instant::now()));
        self.hub.send(opentern_data::Request::Overview { seq, market: m });
    }

    pub(crate) fn overview_event(&mut self, seq: u64, market: Market, result: Result<Board, String>) {
        if self.overview.asked.get(&market).is_none_or(|(s, _)| *s != seq) {
            return;
        }
        self.overview.asked.remove(&market);
        match result {
            Ok(b) => {
                self.overview.errors.remove(&market);
                self.overview.boards.insert(market, b);
            }
            Err(e) => {
                self.overview.errors.insert(market, e);
                self.overview.failed.insert(market, Instant::now());
            }
        }
    }

    /// Fresh numbers while the map shows. Every frame.
    pub(crate) fn poll_overview(&mut self, ctx: &egui::Context) {
        if !self.overview_open() {
            return;
        }
        let m = self.overview.market;
        let now = jiff::Timestamp::now().as_millisecond();
        let fresh_for = self.overview.boards.get(&m).map_or(Duration::ZERO, |b| {
            REFRESH.saturating_sub(Duration::from_millis((now - b.at).max(0) as u64))
        });
        let retry_in = self
            .overview
            .failed
            .get(&m)
            .map_or(Duration::ZERO, |at| RETRY.saturating_sub(at.elapsed()));
        let left = fresh_for.max(retry_in);
        if left.is_zero() && !self.overview.loading(m) {
            self.overview_fetch(m);
        }
        ctx.request_repaint_after(left.max(Duration::from_secs(1)));
    }

    /// The page, in the chart area.
    pub(crate) fn overview_page(&mut self, ui: &mut Ui, t: &Theme) {
        let area = ui.max_rect();
        ui.painter().rect_filled(area, 0, t.bg);
        let head = Rect::from_min_size(area.min, vec2(area.width(), HEADER_H));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(head.shrink2(vec2(10.0, 0.0)))
                .layout(Layout::left_to_right(Align::Center)),
        );
        child.set_clip_rect(head.intersect(ui.clip_rect()));
        self.overview_header(&mut child, t);
        let map = Rect::from_min_max(pos2(area.left() + 8.0, head.bottom()), area.max - vec2(8.0, 8.0));
        if map.width() < 40.0 || map.height() < 40.0 {
            return;
        }
        let m = self.overview.market;
        let Some(board) = self.overview.boards.get(&m).cloned() else {
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(map));
            c.add_space((map.height() * 0.35).max(0.0));
            match self.overview.errors.get(&m).cloned() {
                Some(e) if !self.overview.loading(m) => {
                    w::empty_state(&mut c, t, ph::CLOUD_SLASH, &e);
                    c.vertical_centered(|ui| {
                        if w::secondary(ui, "Try again").clicked() {
                            self.overview.errors.remove(&m);
                            self.overview.failed.remove(&m);
                            self.overview_fetch(m);
                        }
                    });
                }
                _ => {
                    c.vertical_centered(|ui| ui.add(egui::Spinner::new().size(22.0).color(t.text3)));
                }
            }
            return;
        };
        if let Some(click) = self.overview_map(ui, t, &board, map) {
            match click {
                Click::Tile(sym) => self.open_symbol_synced(sym),
                Click::Sector(s) => self.overview.sector = Some(s),
            }
        }
        // Esc: out of a sector, then out of the overview.
        if self.modal.is_none()
            && ui.input(|i| i.key_pressed(egui::Key::Escape))
            && self.overview.sector.take().is_none()
        {
            self.set_overview(false);
        }
    }

    fn overview_header(&mut self, ui: &mut Ui, t: &Theme) {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut market = self.overview.market;
        let opts: Vec<(Market, &str)> = Market::ALL.iter().map(|m| (*m, m.label())).collect();
        if w::segmented(ui, t, &mut market, &opts) {
            self.overview.market = market;
            self.overview.sector = None;
            self.save_overview();
            if !self.overview.boards.contains_key(&market) {
                self.overview_fetch(market);
            }
        }
        ui.add_space(6.0);
        let narrow = ui.max_rect().width() < 640.0;
        if !narrow {
            ui.label(
                egui::RichText::new(self.overview.market.universe())
                    .font(theme::medium(12.5))
                    .color(t.text3),
            );
            ui.add_space(2.0);
        }
        w::vdivider(ui, t);
        for p in Period::ALL {
            if w::bar_button(ui, t, None, p.label(), "", self.overview.period == p).clicked() {
                self.overview.period = p;
                self.save_overview();
            }
        }
        if let Some(s) = self.overview.sector.clone() {
            w::vdivider(ui, t);
            let label = format!("{s}  {}", ph::X);
            if w::bar_button(ui, t, None, &label, "All sectors (Esc)", false).clicked() {
                self.overview.sector = None;
            }
        }
        // The legend on the right, while there's room.
        let room = ui.available_width();
        let m = self.overview.market;
        let busy = self.overview.loading(m) && self.overview.boards.contains_key(&m);
        let error = self.overview.errors.get(&m).cloned();
        let board = self.overview.boards.get(&m);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if room > 150.0 {
                legend(ui, t, self.overview.period, board);
            }
            if busy {
                ui.add(egui::Spinner::new().size(13.0).color(t.text3));
            } else if let Some(e) = error.filter(|_| board.is_some()) {
                ui.label(
                    egui::RichText::new(ph::WARNING_CIRCLE)
                        .font(theme::regular(14.0))
                        .color(t.warn),
                )
                .on_hover_text(e);
            }
        });
    }

    /// The map. Returns what was clicked.
    fn overview_map(&self, ui: &mut Ui, t: &Theme, board: &Board, area: Rect) -> Option<Click> {
        let resp = ui.interact(area, Id::new("overview-map"), Sense::click());
        let hover = resp.hover_pos();
        let p = ui.painter_at(area);
        let period = self.overview.period;
        let only = self.overview.sector.as_deref();
        let groups = sectors(board, only);
        let mut click = None;
        let mut hot: Option<(&Member, Rect)> = None;
        let mut drawn = self.overview.drawn.borrow_mut();
        drawn.tiles.clear();
        drawn.sectors.clear();
        if groups.is_empty() {
            return None;
        }
        let weights: Vec<f64> = groups.iter().map(|g| g.1).collect();
        let gap = 3.0;
        for ((name, _, members), r) in groups.iter().zip(crate::indices::squarify(&weights, area)) {
            let r = r.shrink(gap / 2.0);
            if r.width() < 2.0 || r.height() < 2.0 {
                continue;
            }
            // The sector's name across its top: a click zooms in.
            let titled = r.height() > SECTOR_H + 14.0 && r.width() > 36.0 && only.is_none();
            let body = if titled {
                let bar = Rect::from_min_size(r.min, vec2(r.width(), SECTOR_H));
                drawn.sectors.push((name.clone(), bar));
                let hovered = hover.is_some_and(|h| bar.contains(h));
                let font = theme::medium(11.5);
                let ch = weighted(members, period);
                let title = format!("{name} ›");
                let text = crate::format::elide_to(&p, &title, &font, r.width() - 8.0);
                let color = if hovered { t.text } else { t.text2 };
                p.text(
                    bar.left_center() + vec2(3.0, 0.0),
                    Align2::LEFT_CENTER,
                    &text,
                    font.clone(),
                    color,
                );
                let used = p.layout_no_wrap(text, font.clone(), color).size().x;
                if r.width() - used > 70.0 && ch.is_finite() {
                    let c = if ch >= 0.0 { t.pos } else { t.neg };
                    p.text(
                        bar.right_center() - vec2(3.0, 0.0),
                        Align2::RIGHT_CENTER,
                        crate::format::signed_pct(ch),
                        theme::medium(11.0),
                        with_alpha(c, 0.9),
                    );
                }
                if hovered {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    if resp.clicked() {
                        click = Some(Click::Sector(name.clone()));
                    }
                }
                Rect::from_min_max(pos2(r.left(), r.top() + SECTOR_H), r.max)
            } else {
                r
            };
            let ws: Vec<f64> = members.iter().map(|(_, wt)| *wt).collect();
            for ((m, _), tr) in members.iter().zip(crate::indices::squarify(&ws, body)) {
                let tr = tr.shrink(0.5);
                if tr.width() < 0.5 || tr.height() < 0.5 {
                    continue;
                }
                drawn.tiles.push((m.symbol.id.clone(), tr));
                let c = m.change(period);
                let f = fill(t, c, period);
                p.rect_filled(
                    tr,
                    CornerRadius::same(if tr.width().min(tr.height()) > 24.0 { 2 } else { 1 }),
                    f,
                );
                self.tile_label(&p, t, m, c, f, tr);
                if hover.is_some_and(|h| tr.contains(h)) || self.overview.still_hover.as_deref() == Some(&short(m)) {
                    hot = Some((m, tr));
                }
            }
        }
        if let Some((m, r)) = hot {
            p.rect_stroke(
                r,
                CornerRadius::same(2),
                Stroke::new(1.5, with_alpha(Color32::WHITE, 0.85)),
                egui::StrokeKind::Inside,
            );
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if resp.clicked() && click.is_none() {
                click = Some(Click::Tile(m.symbol.clone()));
            }
            let still = self.overview.still_hover.as_ref().map(|_| r.center());
            if let Some(h) = hover.or(still) {
                self.hover_card(ui, t, board, m, h, area);
            }
        }
        click
    }

    fn tile_label(&self, p: &egui::Painter, t: &Theme, m: &Member, change: f64, fill: Color32, r: Rect) {
        let side = r.width().min(r.height());
        let ink = ink(t, fill);
        let soft = with_alpha(ink, 0.88);
        let name = short(m);
        let pct = crate::format::signed_pct(change).replace('-', "−");
        if side >= 58.0 && r.width() >= 64.0 {
            let lr = (side * 0.17).clamp(8.0, 22.0);
            let big = (side * 0.14).clamp(10.0, 21.0);
            let small = (big * 0.78).max(9.5);
            let block = lr * 2.0 + 6.0 + big + 3.0 + small;
            let top = r.center().y - block / 2.0;
            self.logos.paint(p, t, &m.symbol, pos2(r.center().x, top + lr), lr);
            let font = theme::semibold(big);
            let label = crate::format::elide_to(p, &name, &font, r.width() - 10.0);
            p.text(
                pos2(r.center().x, top + lr * 2.0 + 6.0),
                Align2::CENTER_TOP,
                label,
                font,
                ink,
            );
            p.text(
                pos2(r.center().x, top + lr * 2.0 + 9.0 + big),
                Align2::CENTER_TOP,
                pct,
                theme::medium(small),
                soft,
            );
        } else if side >= 30.0 && r.width() >= 40.0 {
            let font = theme::semibold(10.0);
            let label = crate::format::elide_to(p, &name, &font, r.width() - 6.0);
            p.text(r.center() - vec2(0.0, 6.0), Align2::CENTER_CENTER, label, font, ink);
            p.text(
                r.center() + vec2(0.0, 6.5),
                Align2::CENTER_CENTER,
                pct,
                theme::medium(9.0),
                soft,
            );
        } else if r.height() >= 14.0 && r.width() >= 74.0 && r.width() > r.height() * 2.5 {
            // Wide and short: one line.
            let font = theme::semibold(9.5);
            let line = format!("{name}  {pct}");
            let label = crate::format::elide_to(p, &line, &font, r.width() - 8.0);
            p.text(r.center(), Align2::CENTER_CENTER, label, font, ink);
        } else if side >= 18.0 {
            self.logos.paint(p, t, &m.symbol, r.center(), (side * 0.32).min(10.0));
        }
    }

    /// The card beside the pointer: name, line, price, change, size.
    fn hover_card(&self, ui: &Ui, t: &Theme, board: &Board, m: &Member, at: Pos2, area: Rect) {
        let p = ui
            .ctx()
            .layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("overview-card")));
        let w = 236.0;
        let period = self.overview.period;
        let change = m.change(period);
        let size_label = match board.sizing {
            Sizing::Volume => "24h volume",
            _ => "Market cap",
        };
        let mut rows: Vec<(String, String, Color32)> = vec![
            ("Price".into(), price(m.price), t.text),
            (
                format!("Change {}", period.label()),
                crate::format::signed_pct(change),
                if change >= 0.0 { t.pos } else { t.neg },
            ),
        ];
        if board.sizing != Sizing::Equal {
            rows.push((size_label.into(), money(m.cap, &board.currency), t.text));
        }
        let line_h = if m.closes.len() > 2 { 40.0 } else { 0.0 };
        let h = 16.0 + 34.0 + 8.0 + line_h + rows.len() as f32 * 21.0 + 10.0;
        let mut pos = at + vec2(16.0, 18.0);
        let screen = ui.ctx().content_rect();
        if pos.x + w > area.right().min(screen.right()) - 4.0 {
            pos.x = at.x - 16.0 - w;
        }
        if pos.y + h > screen.bottom() - 4.0 {
            pos.y = (at.y - 18.0 - h).max(screen.top() + 4.0);
        }
        let card = Rect::from_min_size(pos, vec2(w, h));
        p.rect_filled(
            card.translate(vec2(0.0, 6.0)).expand(2.0),
            CornerRadius::same(12),
            Color32::from_black_alpha(if t.dark { 70 } else { 22 }),
        );
        p.rect(
            card,
            CornerRadius::same(10),
            t.elevated,
            Stroke::new(1.0, t.border),
            egui::StrokeKind::Inside,
        );
        let x = card.left() + 14.0;
        let mut y = card.top() + 16.0;
        self.logos.paint(&p, t, &m.symbol, pos2(x + 13.0, y + 13.0), 13.0);
        let tx = x + 34.0;
        let tw = card.right() - 14.0 - tx;
        let f = theme::semibold(13.0);
        p.text(
            pos2(tx, y - 1.0),
            Align2::LEFT_TOP,
            crate::format::elide_to(&p, &short(m), &f, tw),
            f,
            t.text,
        );
        let sub = if m.industry.is_empty() || m.industry == m.name {
            format!("{} · {}", m.name, m.sector)
        } else {
            format!("{} · {}", m.name, m.industry)
        };
        let f = theme::regular(11.0);
        p.text(
            pos2(tx, y + 16.0),
            Align2::LEFT_TOP,
            crate::format::elide_to(&p, &sub, &f, tw),
            f,
            t.text3,
        );
        y += 34.0 + 8.0;
        if line_h > 0.0 {
            let r = Rect::from_min_size(pos2(x, y), vec2(w - 28.0, line_h - 10.0));
            let (lo, hi) = m
                .closes
                .iter()
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(l, h), c| (l.min(*c), h.max(*c)));
            let span = (hi - lo).max(hi.abs() * 1e-6).max(1e-9);
            let n = m.closes.len() as f32 - 1.0;
            let pts: Vec<Pos2> = m
                .closes
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    pos2(
                        r.left() + r.width() * i as f32 / n,
                        r.bottom() - (c - lo) / span * r.height(),
                    )
                })
                .collect();
            let up = m.closes.last() >= m.closes.first();
            p.add(egui::Shape::line(pts, Stroke::new(1.4, if up { t.pos } else { t.neg })));
            y += line_h;
        }
        for (k, v, c) in rows {
            p.text(pos2(x, y), Align2::LEFT_TOP, k, theme::regular(12.0), t.text2);
            p.text(
                pos2(card.right() - 14.0, y),
                Align2::RIGHT_TOP,
                v,
                theme::medium(12.0),
                c,
            );
            y += 21.0;
        }
    }
}

pub enum Click {
    Tile(opentern_core::Symbol),
    Sector(String),
}

/// −3% ▮▮▮▮▮▮▮ +3%, for the period's scale.
fn legend(ui: &mut Ui, t: &Theme, period: Period, board: Option<&Board>) {
    let s = scale(period);
    let label = |v: f64| {
        format!(
            "{}{}%",
            if v > 0.0 {
                "+"
            } else if v < 0.0 {
                "−"
            } else {
                ""
            },
            v.abs()
        )
    };
    let f = theme::regular(11.0);
    ui.label(egui::RichText::new(label(s)).font(f.clone()).color(t.text3));
    let (r, resp) = ui.allocate_exact_size(vec2(7.0 * 12.0, 10.0), Sense::hover());
    let p = ui.painter();
    for (i, k) in [1.0, 0.6, 0.25, 0.0, -0.25, -0.6, -1.0].iter().enumerate() {
        // Laid out right to left.
        let x = r.right() - (i as f32 + 1.0) * 12.0;
        let cell = Rect::from_min_size(pos2(x + 1.0, r.top()), vec2(10.0, r.height()));
        p.rect_filled(cell, CornerRadius::same(2), fill(t, k * s, period));
    }
    ui.label(egui::RichText::new(label(-s)).font(f).color(t.text3));
    if let Some(b) = board {
        let sized = match b.sizing {
            Sizing::MarketCap => format!("Sized by market cap, from {}", b.source),
            Sizing::Volume => format!("Sized by 24h volume, from {}", b.source),
            Sizing::Equal => "Market caps didn't load, so every tile is the same size".into(),
        };
        resp.on_hover_text(sized);
    }
}

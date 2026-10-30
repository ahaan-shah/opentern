//! The smaller side widgets: Hotlists (top movers), News for the chart's
//! symbol, and the Calendar (market holidays and early closes, and the
//! symbol's dividends and splits).

use crate::app::App;
use crate::theme::{self, Theme};
use egui::{Sense, Ui, vec2};
use opentern_data::Request;
use opentern_data::feeds::{HotKind, HotMarket, HotRow, NewsItem};
use std::time::{Duration, Instant};

pub struct Side {
    pub market: HotMarket,
    pub kind: HotKind,
    pub hot: Option<Result<Vec<HotRow>, String>>,
    hot_seq: u64,
    hot_asked: Option<(HotMarket, HotKind, Instant)>,
    pub news: Option<Result<Vec<NewsItem>, String>>,
    news_seq: u64,
    news_for: Option<(String, Instant)>,
}

impl Default for Side {
    fn default() -> Self {
        Side {
            market: HotMarket::Crypto,
            kind: HotKind::Gainers,
            hot: None,
            hot_seq: 1 << 50,
            hot_asked: None,
            news: None,
            news_seq: 1 << 51,
            news_for: None,
        }
    }
}

/// Seconds before a failed list asks again.
const RETRY: u64 = 15;

fn ago(ms: i64) -> String {
    let s = (jiff::Timestamp::now().as_millisecond() - ms).max(0) / 1000;
    match s {
        s if s < 3600 => format!("{}m ago", (s / 60).max(1)),
        s if s < 86_400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

impl App {
    pub(crate) fn side_event(&mut self, e: opentern_data::Event) {
        match e {
            opentern_data::Event::Hotlist { seq, result } if seq == self.side.hot_seq => {
                self.side.hot = Some(result);
            }
            opentern_data::Event::News { seq, result } if seq == self.side.news_seq => {
                self.side.news = Some(result);
            }
            _ => {}
        }
    }

    pub(crate) fn hotlists_widget(&mut self, ui: &mut Ui, t: &Theme) {
        let sd = &mut self.side;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (m, label) in [
                (HotMarket::Crypto, "Crypto"),
                (HotMarket::UsStocks, "US stocks"),
                (HotMarket::India, "India"),
            ] {
                if crate::shell::small_button(ui, t, label, sd.market == m).clicked() {
                    sd.market = m;
                    // India has no volume list.
                    if m == HotMarket::India && sd.kind == HotKind::Volume {
                        sd.kind = HotKind::Gainers;
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (k, label) in [
                (HotKind::Gainers, "Gainers"),
                (HotKind::Losers, "Losers"),
                (HotKind::Volume, "Volume leaders"),
            ] {
                if k == HotKind::Volume && sd.market == HotMarket::India {
                    continue;
                }
                if crate::shell::small_button(ui, t, label, sd.kind == k).clicked() {
                    sd.kind = k;
                }
            }
        });
        // Ask again when the choice changes, every minute while open, or
        // after 15 s when it failed (the connection may be back).
        let every = if matches!(sd.hot, Some(Err(_))) { RETRY } else { 60 };
        let stale = match sd.hot_asked {
            Some((m, k, at)) => m != sd.market || k != sd.kind || at.elapsed() > Duration::from_secs(every),
            None => true,
        };
        if stale {
            if sd.hot_asked.is_some_and(|(m, k, _)| m != sd.market || k != sd.kind) {
                sd.hot = None;
            }
            sd.hot_seq += 1;
            sd.hot_asked = Some((sd.market, sd.kind, Instant::now()));
            self.hub.send(Request::Hotlist {
                seq: self.side.hot_seq,
                market: self.side.market,
                kind: self.side.kind,
            });
        }
        ui.ctx().request_repaint_after(Duration::from_secs(every));
        ui.add_space(6.0);
        let mut open = None;
        match &self.side.hot {
            None => {
                ui.label(egui::RichText::new("Loading…").color(t.text3));
            }
            Some(Err(e)) => {
                ui.label(egui::RichText::new(e).color(t.text3));
            }
            Some(Ok(rows)) => {
                // Binance's change is over a rolling 24 hours; Yahoo's is
                // since the previous session's close.
                let (chg, tip) = match self.side.market {
                    HotMarket::Crypto => ("Chg% 24h", "Change over the last 24 hours"),
                    _ => ("Chg% today", "Change since the previous close"),
                };
                let (cols, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
                let p = ui.painter();
                let head = |p: &egui::Painter, x: f32, align: egui::Align2, text: &str| {
                    p.text(
                        egui::pos2(x, cols.center().y),
                        align,
                        text,
                        theme::medium(11.5),
                        t.text3,
                    )
                };
                head(p, cols.left() + 6.0, egui::Align2::LEFT_CENTER, "Symbol");
                head(p, cols.right() - 82.0, egui::Align2::RIGHT_CENTER, "Last");
                let r = head(p, cols.right() - 6.0, egui::Align2::RIGHT_CENTER, chg);
                ui.interact(r, ui.id().with("hot-chg"), Sense::hover())
                    .on_hover_text(tip);
                egui::ScrollArea::vertical()
                    .id_salt("hot")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for r in rows {
                            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_filled(rect, 6, t.hover_wash());
                            }
                            let p = ui.painter();
                            p.text(
                                rect.left_center() + vec2(6.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                r.symbol.ticker(),
                                theme::medium(12.5),
                                t.text,
                            );
                            let col = if r.change_pct >= 0.0 { t.pos } else { t.neg };
                            p.text(
                                rect.right_center() - vec2(6.0, 0.0),
                                egui::Align2::RIGHT_CENTER,
                                crate::format::signed_pct(r.change_pct),
                                theme::regular(12.5),
                                col,
                            );
                            p.text(
                                rect.right_center() - vec2(82.0, 0.0),
                                egui::Align2::RIGHT_CENTER,
                                crate::format::price(r.last, if r.last < 1.0 { 5 } else { 2 }),
                                theme::regular(12.5),
                                t.text2,
                            );
                            let resp = if r.symbol.description.is_empty() {
                                resp
                            } else {
                                resp.on_hover_text(&r.symbol.description)
                            };
                            if resp.clicked() {
                                open = Some(r.symbol.clone());
                            }
                        }
                    });
            }
        }
        if let Some(s) = open {
            self.open_symbol_synced(s);
        }
    }

    pub(crate) fn news_widget(&mut self, ui: &mut Ui, t: &Theme) {
        let id = self.chart.symbol.id.clone();
        // Every 5 minutes, or 15 s after a failure (the connection may be back).
        let failed = matches!(self.side.news, Some(Err(_)));
        let every = if failed { RETRY } else { 300 };
        let stale = match &self.side.news_for {
            Some((s, at)) => *s != id || at.elapsed() > Duration::from_secs(every),
            None => true,
        };
        ui.ctx().request_repaint_after(Duration::from_secs(every));
        if stale {
            if self.side.news_for.as_ref().is_some_and(|(s, _)| *s != id) {
                self.side.news = None;
            }
            self.side.news_seq += 1;
            self.side.news_for = Some((id, Instant::now()));
            self.hub.send(Request::News {
                seq: self.side.news_seq,
                symbol: self.chart.symbol.clone(),
            });
        }
        let loading = self.side.news.is_none();
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("About {}", self.chart.symbol.ticker())).color(t.text3));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                refresh = crate::widgets::tool_button(ui, t, crate::icons::ph::ARROW_CLOCKWISE, "Refresh", false, 24.0)
                    .clicked()
                    && !loading;
            });
        });
        if refresh {
            // A failure shows "Loading…" again; headlines stay until new ones come.
            if failed {
                self.side.news = None;
            }
            self.side.news_for = None;
        }
        ui.add_space(4.0);
        let mut open = None;
        match &self.side.news {
            None => {
                ui.label(egui::RichText::new("Loading…").color(t.text3));
            }
            Some(Err(e)) => {
                ui.label(egui::RichText::new(e).color(t.text3));
            }
            Some(Ok(items)) if items.is_empty() => {
                ui.label(egui::RichText::new("No headlines right now.").color(t.text3));
            }
            Some(Ok(items)) => {
                egui::ScrollArea::vertical()
                    .id_salt("news")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for n in items {
                            let r = egui::Frame::new()
                                .inner_margin(egui::Margin::symmetric(4, 6))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(egui::RichText::new(&n.title).font(theme::medium(12.5)).color(t.text));
                                    ui.label(
                                        egui::RichText::new(format!("{} · {}", n.publisher, ago(n.time)))
                                            .color(t.text3)
                                            .size(11.5),
                                    );
                                });
                            let resp = r
                                .response
                                .interact(Sense::click())
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if resp.hovered() {
                                ui.painter().rect_filled(r.response.rect, 6, t.hover_wash());
                            }
                            if resp.clicked() {
                                open = Some(n.link.clone());
                            }
                        }
                    });
            }
        }
        if let Some(link) = open {
            crate::app::open_url(&link);
        }
    }

    pub(crate) fn calendar_widget(&mut self, ui: &mut Ui, t: &Theme) {
        use opentern_core::calendar::{Market, Special, upcoming};
        let today = jiff::Zoned::now().date();
        egui::ScrollArea::vertical()
            .id_salt("calendar")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                for (m, name, exchanges) in [
                    (Market::Us, "US stocks", "NYSE · NASDAQ"),
                    (Market::India, "India", "NSE · BSE"),
                ] {
                    section_head(ui, t, name, exchanges);
                    let list = upcoming(m, today, 120);
                    if list.is_empty() {
                        ui.label(egui::RichText::new("No holidays in the next four months.").color(t.text3));
                    }
                    for (d, what) in list.iter().take(8) {
                        let (title, pill, tone) = match what {
                            Special::Holiday(n) => (n.clone(), "Closed".to_string(), t.text2),
                            Special::EarlyClose(at) => (
                                "Early close".to_string(),
                                format!("Closes {}", at.strftime("%H:%M")),
                                t.warn,
                            ),
                        };
                        let sub = format!("{} · {}", d.strftime("%A"), relative_days((*d - today).get_days()));
                        holiday_row(ui, t, *d, *d == today, &title, &sub, &pill, tone);
                    }
                    ui.add_space(14.0);
                }
                if let Some(c) = &self.chart.corporate {
                    let mut ev: Vec<(i64, bool, f64)> = c
                        .dividends
                        .iter()
                        .map(|(ts, amt)| (*ts, true, *amt))
                        .chain(c.splits.iter().map(|(ts, r)| (*ts, false, *r)))
                        .collect();
                    if ev.is_empty() {
                        return;
                    }
                    ev.sort_by_key(|e| std::cmp::Reverse(e.0));
                    section_head(ui, t, "Dividends and splits", self.chart.symbol.ticker());
                    let cur = currency_prefix(&self.chart.symbol);
                    let mut year = None;
                    for (ts, dividend, v) in ev.iter().take(12) {
                        let Ok(z) =
                            jiff::Timestamp::from_millisecond(*ts).map(|x| x.to_zoned(self.chart.sym_tz.clone()))
                        else {
                            continue;
                        };
                        if year != Some(z.year()) {
                            year = Some(z.year());
                            ui.add_space(if ev.first().is_some_and(|f| f.0 == *ts) {
                                0.0
                            } else {
                                6.0
                            });
                            ui.label(
                                egui::RichText::new(z.year().to_string())
                                    .font(theme::medium(11.5))
                                    .color(t.text3),
                            );
                        }
                        let (what, amount) = if *dividend {
                            ("Dividend", format!("{cur}{}", amount(*v)))
                        } else {
                            ("Split", format!("{}:1", amount(*v)))
                        };
                        let upcoming = z.date() >= today;
                        corporate_row(ui, t, &z.strftime("%d %b").to_string(), what, &amount, upcoming);
                    }
                }
            });
    }
}

/// A section title with a quieter note on the right.
fn section_head(ui: &mut Ui, t: &Theme, title: &str, note: &str) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
    let p = ui.painter();
    p.text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        title,
        theme::semibold(13.0),
        t.text,
    );
    p.text(
        rect.right_center(),
        egui::Align2::RIGHT_CENTER,
        note,
        theme::regular(11.5),
        t.text3,
    );
    ui.add_space(4.0);
}

/// "Today", "Tomorrow", "in 5 days", "in 3 weeks".
fn relative_days(n: i32) -> String {
    match n {
        ..=0 => "Today".into(),
        1 => "Tomorrow".into(),
        2..=13 => format!("in {n} days"),
        _ => format!("in {} weeks", (n + 3) / 7),
    }
}

/// A holiday or early close: a date tile, the name and when, and a pill.
#[allow(clippy::too_many_arguments)]
fn holiday_row(
    ui: &mut Ui,
    t: &Theme,
    d: jiff::civil::Date,
    today: bool,
    title: &str,
    sub: &str,
    pill: &str,
    tone: egui::Color32,
) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 46.0), Sense::hover());
    let p = ui.painter();
    // Date tile.
    let tile = egui::Rect::from_min_size(rect.left_top() + vec2(0.0, 5.0), vec2(36.0, 36.0));
    p.rect_filled(tile, 8, if today { t.accent_soft() } else { t.hover_wash() });
    p.text(
        tile.center_top() + vec2(0.0, 6.0),
        egui::Align2::CENTER_TOP,
        d.strftime("%b").to_string().to_uppercase(),
        theme::semibold(8.5),
        if today { t.accent } else { t.text3 },
    );
    p.text(
        tile.center_bottom() - vec2(0.0, 5.0),
        egui::Align2::CENTER_BOTTOM,
        d.day().to_string(),
        theme::semibold(14.0),
        if today { t.accent } else { t.text },
    );
    // Pill on the right.
    let pill_font = theme::medium(11.0);
    let pw = p.layout_no_wrap(pill.to_string(), pill_font.clone(), tone).size().x + 14.0;
    let pill_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - pw / 2.0, rect.center().y), vec2(pw, 20.0));
    p.rect_filled(pill_rect, 10, crate::motion::with_alpha(tone, 0.14));
    p.text(pill_rect.center(), egui::Align2::CENTER_CENTER, pill, pill_font, tone);
    // Name and when, cut to fit.
    let x = tile.right() + 10.0;
    let max_w = (pill_rect.left() - 8.0 - x).max(20.0);
    let title_font = theme::medium(13.0);
    let shown = crate::format::elide_to(p, title, &title_font, max_w);
    let cut = shown != title;
    p.text(
        egui::pos2(x, rect.top() + 6.0),
        egui::Align2::LEFT_TOP,
        shown,
        title_font,
        t.text,
    );
    let sub_font = theme::regular(11.5);
    p.text(
        egui::pos2(x, rect.bottom() - 7.0),
        egui::Align2::LEFT_BOTTOM,
        crate::format::elide_to(p, sub, &sub_font, max_w),
        sub_font,
        t.text3,
    );
    if cut {
        ui.interact(rect, ui.id().with(("holiday", d)), Sense::hover())
            .on_hover_text(title);
    }
}

/// One dividend or split: date, kind and amount on a line.
fn corporate_row(ui: &mut Ui, t: &Theme, date: &str, what: &str, amount: &str, upcoming: bool) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
    let p = ui.painter();
    p.text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        date,
        theme::regular(12.5),
        if upcoming { t.accent } else { t.text2 },
    );
    p.text(
        rect.left_center() + vec2(64.0, 0.0),
        egui::Align2::LEFT_CENTER,
        what,
        theme::regular(12.5),
        t.text,
    );
    p.text(
        rect.right_center(),
        egui::Align2::RIGHT_CENTER,
        amount,
        theme::medium(12.5),
        t.text,
    );
}

/// What goes before a dividend amount.
fn currency_prefix(sym: &opentern_core::Symbol) -> &'static str {
    match (sym.currency.as_str(), sym.exchange()) {
        ("INR", _) | (_, "NSE" | "BSE") => "₹",
        ("USD", _) | (_, "NASDAQ" | "NYSE" | "AMEX" | "ARCA") => "$",
        ("EUR", _) => "€",
        ("GBP", _) => "£",
        _ => "",
    }
}

/// 25 → "25.00", 0.2375 → "0.2375".
fn amount(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0');
    match s.split_once('.') {
        Some((_, f)) if f.len() >= 2 => s.to_string(),
        _ => format!("{v:.2}"),
    }
}

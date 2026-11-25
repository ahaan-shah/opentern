//! The smaller side widgets: Hotlists (top movers), News for the chart's
//! symbol, and the Calendar (market holidays and early closes, and the
//! symbol's dividends and splits).

use crate::app::App;
use crate::i18n::{date, nt, tr, trf, trm};
use crate::icons::ph;
use crate::theme::{self, Theme};
use egui::{Sense, Ui, pos2, vec2};
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
    /// A refresh is out (since when), so the button turns.
    news_busy: Option<Instant>,
    /// The Calendar's folded sections, by title (settings key
    /// `calendar_folded`; read on first use).
    folded: Option<std::collections::HashSet<String>>,
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
            news_busy: None,
            folded: None,
        }
    }
}

/// Seconds before a failed list asks again.
const RETRY: u64 = 15;

fn ago(ms: i64) -> String {
    let s = (jiff::Timestamp::now().as_millisecond() - ms).max(0) / 1000;
    match s {
        s if s < 3600 => trf!("{}m ago", (s / 60).max(1)),
        s if s < 86_400 => trf!("{}h ago", s / 3600),
        s => trf!("{}d ago", s / 86_400),
    }
}

impl App {
    pub(crate) fn side_event(&mut self, e: opentern_data::Event) {
        match e {
            opentern_data::Event::Hotlist { seq, result } if seq == self.side.hot_seq => {
                self.side.hot = Some(result);
                self.share_hotlist();
            }
            opentern_data::Event::News { seq, result } if seq == self.side.news_seq => {
                self.side.news = Some(result);
                self.side.news_busy = None;
            }
            _ => {}
        }
    }

    pub(crate) fn hotlists_widget(&mut self, ui: &mut Ui, t: &Theme) {
        let sd = &mut self.side;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (m, label) in [
                (HotMarket::Crypto, nt!("Crypto")),
                (HotMarket::UsStocks, nt!("US Markets")),
                (HotMarket::India, nt!("Indian Markets")),
            ] {
                if crate::shell::small_button(ui, t, tr(label), sd.market == m).clicked() {
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
                (HotKind::Gainers, nt!("Gainers")),
                (HotKind::Losers, nt!("Losers")),
                (HotKind::Volume, nt!("Volume leaders")),
            ] {
                if k == HotKind::Volume && sd.market == HotMarket::India {
                    continue;
                }
                if crate::shell::small_button(ui, t, tr(label), sd.kind == k).clicked() {
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
                ui.label(egui::RichText::new(tr("Loading…")).color(t.text3));
            }
            Some(Err(e)) => {
                ui.label(egui::RichText::new(trm(e)).color(t.text3));
            }
            Some(Ok(rows)) => {
                // Binance's change is over a rolling 24 hours; Yahoo's is
                // since the previous session's close.
                let (chg, tip) = match self.side.market {
                    HotMarket::Crypto => (tr("Chg% 24h"), tr("Change over the last 24 hours")),
                    _ => (tr("Chg% today"), tr("Change since the previous close")),
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
                head(p, cols.left() + 6.0, egui::Align2::LEFT_CENTER, tr("Symbol"));
                head(p, cols.right() - 82.0, egui::Align2::RIGHT_CENTER, tr("Last"));
                let r = head(p, cols.right() - 6.0, egui::Align2::RIGHT_CENTER, chg);
                ui.interact(r, ui.id().with("hot-chg"), Sense::hover())
                    .on_hover_text(tip);
                egui::ScrollArea::vertical()
                    .id_salt("hot")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for r in rows {
                            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                            ui.painter().rect_filled(
                                rect,
                                6,
                                crate::motion::with_alpha(t.hover_wash(), crate::motion::hover(&resp)),
                            );
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
            self.side.news_busy = Some(Instant::now());
            self.hub.send(Request::News {
                seq: self.side.news_seq,
                symbol: self.chart.symbol.clone(),
            });
        }
        // Turns while a refresh is out, and long enough to be seen.
        let spinning = self.side.news_busy.is_some()
            || self
                .side
                .news_for
                .as_ref()
                .is_some_and(|(_, at)| at.elapsed() < Duration::from_millis(700));
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(trf!("About {}", self.chart.symbol.ticker())).color(t.text3));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                refresh =
                    crate::widgets::spin_button(ui, t, crate::icons::ph::ARROWS_CLOCKWISE, tr("Refresh"), spinning)
                        .clicked()
                        && !spinning;
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
                ui.label(egui::RichText::new(tr("Loading…")).color(t.text3));
            }
            Some(Err(e)) => {
                ui.label(egui::RichText::new(trm(e)).color(t.text3));
            }
            Some(Ok(items)) if items.is_empty() => {
                ui.label(egui::RichText::new(tr("No headlines right now.")).color(t.text3));
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
                            ui.painter().rect_filled(
                                r.response.rect,
                                6,
                                crate::motion::with_alpha(t.hover_wash(), crate::motion::hover(&resp)),
                            );
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
        if self.side.folded.is_none() {
            let saved = self.store.value(FOLDED_KEY).unwrap_or_default();
            self.side.folded = Some(serde_json::from_str(&saved).unwrap_or_default());
        }
        let folded = self.side.folded.clone().unwrap_or_default();
        let mut toggled: Vec<String> = Vec::new();
        egui::ScrollArea::vertical()
            .id_salt("calendar")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                // The chart's own exchange first, when it's another country's;
                // then the US and India (open), then every other exchange the
                // app knows, by region, folded until opened.
                use opentern_core::calendar::Venue;
                let here = match Market::of(&self.chart.symbol) {
                    Market::Exchange(v) => Some(v),
                    _ => None,
                };
                // (market, title, note, open by default, region heading)
                let mut entries: Vec<(Market, &str, &str, bool, Option<&str>)> = Vec::new();
                if let Some(v) = here {
                    entries.push((Market::Exchange(v), tr(v.name().1), v.name().0, true, None));
                }
                entries.push((Market::Us, tr("US Markets"), tr("NYSE · NASDAQ"), true, None));
                entries.push((Market::India, tr("Indian Markets"), tr("NSE · BSE"), true, None));
                let regions: [(&str, &[Venue]); 3] = [
                    (
                        tr("Europe"),
                        &[Venue::London, Venue::Frankfurt, Venue::Paris, Venue::Zurich],
                    ),
                    (
                        tr("Asia-Pacific"),
                        &[
                            Venue::Tokyo,
                            Venue::HongKong,
                            Venue::Shanghai,
                            Venue::Seoul,
                            Venue::Taipei,
                            Venue::Singapore,
                            Venue::Sydney,
                        ],
                    ),
                    (tr("Americas"), &[Venue::Toronto, Venue::SaoPaulo]),
                ];
                for (region, venues) in regions {
                    let mut first = true;
                    for v in venues.iter().filter(|v| Some(**v) != here) {
                        let head = first.then_some(region);
                        first = false;
                        entries.push((Market::Exchange(*v), tr(v.name().1), v.name().0, false, head));
                    }
                }
                for (m, name, exchanges, open_default, region) in entries {
                    if let Some(r) = region {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(r).font(crate::theme::medium(12.0)).color(t.text3));
                        ui.add_space(2.0);
                    }
                    // Sections open by default are remembered when folded;
                    // the others when opened ("+name").
                    let key = if open_default {
                        name.to_string()
                    } else {
                        format!("+{name}")
                    };
                    let shut = folded.contains(&key) == open_default;
                    if section_head(ui, t, name, exchanges, Some(shut)) {
                        toggled.push(key);
                    }
                    if shut {
                        ui.add_space(if open_default { 6.0 } else { 0.0 });
                        continue;
                    }
                    let list = upcoming(m, today, 120);
                    if list.is_empty() {
                        ui.label(egui::RichText::new(tr("No holidays in the next four months.")).color(t.text3));
                    }
                    for (d, what) in list.iter().take(8) {
                        let (title, pill, tone) = match what {
                            Special::Holiday(n) => (tr(n).to_string(), tr("Closed").to_string(), t.text2),
                            Special::EarlyClose(at) => (
                                tr("Early close").to_string(),
                                trf!("Closes {}", at.strftime("%H:%M")),
                                t.warn,
                            ),
                            Special::Session(n, open, close) => (
                                tr(n).to_string(),
                                format!("{}–{}", open.strftime("%H:%M"), close.strftime("%H:%M")),
                                t.accent,
                            ),
                        };
                        let sub = format!(
                            "{} · {}",
                            tr(&d.strftime("%A").to_string()),
                            relative_days((*d - today).get_days())
                        );
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
                    let name = tr("Dividends and splits");
                    let shut = folded.contains(name);
                    ui.add_space(10.0);
                    if section_head(ui, t, name, self.chart.symbol.ticker(), Some(shut)) {
                        toggled.push(name.to_string());
                    }
                    if shut {
                        return;
                    }
                    let cur = currency_prefix(&self.chart.symbol);
                    // Yahoo lists Indian bonus issues as splits too.
                    let split = if matches!(self.chart.symbol.exchange(), "NSE" | "BSE") {
                        tr("Split or bonus")
                    } else {
                        tr("Split")
                    };
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
                            (tr("Dividend"), format!("{cur}{}", amount(*v)))
                        } else {
                            (split, format!("{}:1", amount(*v)))
                        };
                        let upcoming = z.date() >= today;
                        corporate_row(ui, t, &date(&z, "%d %b"), what, &amount, upcoming);
                    }
                }
            });
        if !toggled.is_empty() {
            let set = self.side.folded.get_or_insert_with(Default::default);
            for name in toggled {
                if !set.remove(&name) {
                    set.insert(name);
                }
            }
            let json = serde_json::to_string(&set.iter().collect::<Vec<_>>()).unwrap_or_default();
            let _ = self.store.set_value(FOLDED_KEY, &json);
        }
    }
}

/// A section title with a quieter note on the right.
const FOLDED_KEY: &str = "calendar_folded";

/// A section's title row. With `folded` (Some), it's a toggle: a caret
/// before the title, and a click (returned) folds or opens it.
fn section_head(ui: &mut Ui, t: &Theme, title: &str, note: &str, folded: Option<bool>) -> bool {
    let w = ui.available_width();
    let sense = if folded.is_some() {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), sense);
    let p = ui.painter();
    let mut x = rect.left();
    if let Some(shut) = folded {
        let hover = crate::motion::hover_if(&resp, ui.rect_contains_pointer(rect));
        p.text(
            pos2(x + 6.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            if shut { ph::CARET_RIGHT } else { ph::CARET_DOWN },
            theme::regular(11.0),
            crate::motion::lerp_color(t.text3, t.text2, hover),
        );
        x += 16.0;
    }
    p.text(
        pos2(x, rect.center().y),
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
    folded.is_some() && resp.clicked()
}

/// "Today", "Tomorrow", "in 5 days", "in 3 weeks".
fn relative_days(n: i32) -> String {
    match n {
        ..=0 => tr("Today").into(),
        1 => tr("Tomorrow").into(),
        2..=13 => trf!("in {n} days"),
        _ => trf!("in {} weeks", (n + 3) / 7),
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
        tr(&d.strftime("%b").to_string()).to_uppercase(),
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

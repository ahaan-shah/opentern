//! The smaller side widgets: Hotlists (top movers), News for the chart's
//! symbol, and the Calendar (market holidays and early closes, and the
//! symbol's dividends and splits).

use crate::app::App;
use crate::icons::ph;
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
        // Ask again when the choice changes, or every minute while open.
        let stale = match sd.hot_asked {
            Some((m, k, at)) => m != sd.market || k != sd.kind || at.elapsed() > Duration::from_secs(60),
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
        ui.ctx().request_repaint_after(Duration::from_secs(60));
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
        let stale = match &self.side.news_for {
            Some((s, at)) => *s != id || at.elapsed() > Duration::from_secs(300),
            None => true,
        };
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
        ui.label(egui::RichText::new(format!("About {}", self.chart.symbol.ticker())).color(t.text3));
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
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new("Headlines from Yahoo Finance.")
                                .color(t.text3)
                                .size(11.0),
                        );
                    });
            }
        }
        if let Some(link) = open {
            crate::app::open_url(&link);
        }
    }

    pub(crate) fn calendar_widget(&mut self, ui: &mut Ui, t: &Theme) {
        use opentern_core::calendar::{Market, upcoming};
        let today = jiff::Zoned::now().date();
        egui::ScrollArea::vertical()
            .id_salt("calendar")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width());
                for (m, name) in [
                    (Market::Us, "US stocks (NYSE, NASDAQ)"),
                    (Market::India, "India (NSE, BSE)"),
                ] {
                    ui.label(egui::RichText::new(name).font(theme::semibold(12.5)));
                    let list = upcoming(m, today, 120);
                    if list.is_empty() {
                        ui.label(egui::RichText::new("No holidays in the next four months.").color(t.text3));
                    }
                    for (d, what) in list.iter().take(8) {
                        ui.horizontal_top(|ui| {
                            ui.add_sized(
                                [82.0, 16.0],
                                egui::Label::new(
                                    egui::RichText::new(d.strftime("%a %d %b").to_string()).color(t.text2),
                                ),
                            );
                            ui.add(egui::Label::new(egui::RichText::new(what).color(t.text)).wrap());
                        });
                    }
                    ui.add_space(8.0);
                }
                if let Some(c) = &self.chart.corporate {
                    ui.label(
                        egui::RichText::new(format!("{} dividends and splits", self.chart.symbol.ticker()))
                            .font(theme::semibold(12.5)),
                    );
                    let mut ev: Vec<(i64, String)> = c
                        .dividends
                        .iter()
                        .map(|(ts, amt)| (*ts, format!("Dividend {amt:.4}")))
                        .chain(c.splits.iter().map(|(ts, r)| (*ts, format!("Split {r}:1"))))
                        .collect();
                    ev.sort_by_key(|e| std::cmp::Reverse(e.0));
                    for (ts, what) in ev.iter().take(10) {
                        let d = jiff::Timestamp::from_millisecond(*ts)
                            .map(|x| x.to_zoned(self.chart.sym_tz.clone()).strftime("%d %b %Y").to_string())
                            .unwrap_or_default();
                        ui.horizontal_top(|ui| {
                            ui.add_sized([82.0, 16.0], egui::Label::new(egui::RichText::new(d).color(t.text2)));
                            ui.add(egui::Label::new(what).wrap());
                        });
                    }
                } else {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!(
                                "{}  Dividends and splits show here for stocks and funds.",
                                ph::INFO
                            ))
                            .color(t.text3),
                        )
                        .wrap(),
                    );
                }
            });
    }
}

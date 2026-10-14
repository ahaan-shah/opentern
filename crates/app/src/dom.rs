//! The DOM (depth of market) widget: the chart symbol's order book as a
//! ladder, asks above bids, with size bars, polled about once a second
//! while it's open. Click the Buy or Sell column at a price to place a
//! paper order there: a limit on the near side of the market, a stop on the
//! far side, as TradingView's DOM does. Working paper orders show on their
//! rows.

use crate::app::App;
use crate::theme::{self, Theme};
use egui::{Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use opentern_data::Request;
use opentern_data::depth::Book;
use opentern_engine::paper::{Kind, Ticket};
use std::time::{Duration, Instant};

pub struct Dom {
    pub book: Option<Result<Book, String>>,
    seq: u64,
    asked: Option<(String, Instant)>,
    pub qty: f64,
}

impl Default for Dom {
    fn default() -> Self {
        Dom {
            book: None,
            seq: 1 << 52,
            asked: None,
            qty: 1.0,
        }
    }
}

const ROW_H: f32 = 22.0;
const POLL: Duration = Duration::from_millis(1000);

impl App {
    pub(crate) fn dom_event(&mut self, seq: u64, result: Result<Book, String>) {
        if seq == self.dom.seq {
            self.dom.book = Some(result);
        }
    }

    /// A paper order from a click on the ladder.
    fn dom_order(&mut self, buy: bool, price: f64) {
        let sym = self.chart.symbol.id.clone();
        let last = self.last_price(&sym);
        let Some(last) = last else {
            self.paper.status = "No price yet for this symbol.".into();
            return;
        };
        // Below the market a buy is a limit, above it a stop (and the other
        // way for sells).
        let kind = if (buy && price <= last) || (!buy && price >= last) {
            Kind::Limit(price)
        } else {
            Kind::Stop(price)
        };
        let ticket = Ticket {
            symbol: sym,
            buy,
            qty: self.dom.qty,
            kind,
            take_profit: None,
            stop_loss: None,
        };
        let what = format!(
            "{} {} {} at {}",
            if buy { "Buy" } else { "Sell" },
            kind.name().to_lowercase(),
            self.dom.qty,
            crate::format::price(price, self.chart.symbol.precision())
        );
        match self
            .paper
            .account
            .place(ticket, Some(last), jiff::Timestamp::now().as_millisecond())
        {
            Ok(_) => {
                self.paper.status = format!("{what} placed.");
                self.save_paper_now();
            }
            Err(e) => self.paper.status = e,
        }
    }

    pub(crate) fn dom_widget(&mut self, ui: &mut Ui, t: &Theme) {
        let sym = self.chart.symbol.clone();
        let last = self.last_price(&sym.id).unwrap_or(f64::NAN);
        // Ask while open: on a new symbol at once, then once a second.
        let due = match &self.dom.asked {
            Some((id, at)) => *id != sym.id || at.elapsed() >= POLL,
            None => true,
        };
        if due {
            if self.dom.asked.as_ref().is_some_and(|(id, _)| *id != sym.id) {
                self.dom.book = None;
            }
            self.dom.seq += 1;
            self.dom.asked = Some((sym.id.clone(), Instant::now()));
            self.hub.send(Request::Depth {
                seq: self.dom.seq,
                symbol: sym.clone(),
                last: if last.is_finite() { last } else { 100.0 },
            });
        }
        ui.ctx().request_repaint_after(POLL);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(sym.ticker()).font(theme::semibold(13.0)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::DragValue::new(&mut self.dom.qty)
                        .range(0.000001..=1e9)
                        .speed(0.1)
                        .max_decimals(6),
                );
                ui.label(egui::RichText::new("Qty").color(t.text3));
            });
        });
        ui.add_space(4.0);
        let book = match &self.dom.book {
            None => {
                ui.label(egui::RichText::new("Loading…").color(t.text3));
                return;
            }
            Some(Err(e)) => {
                ui.label(egui::RichText::new(e).color(t.text3));
                return;
            }
            Some(Ok(b)) => b.clone(),
        };
        let prec = sym.precision();
        let w = ui.available_width();
        let room = ((ui.available_height() - ROW_H * 2.0) / ROW_H).floor().max(4.0) as usize;
        let side = (room / 2).max(2);
        let asks: Vec<(f64, f64)> = book.asks.iter().take(side).rev().copied().collect();
        let bids: Vec<(f64, f64)> = book.bids.iter().take(side).copied().collect();
        let max = asks.iter().chain(&bids).map(|x| x.1).fold(0.0f64, f64::max).max(1e-12);
        // Columns: Buy | Bid size | Price | Ask size | Sell.
        let cols = [0.16, 0.22, 0.24, 0.22, 0.16].map(|f| f * w);
        let x0 = ui.cursor().left();
        let edges: Vec<f32> = cols
            .iter()
            .scan(x0, |x, c| {
                let a = *x;
                *x += c;
                Some(a)
            })
            .collect();
        let head = ["Buy", "Bid", "Price", "Ask", "Sell"];
        let (hr, _) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::hover());
        for (k, h) in head.iter().enumerate() {
            ui.painter().text(
                pos2(edges[k] + cols[k] / 2.0, hr.center().y),
                Align2::CENTER_CENTER,
                *h,
                theme::medium(11.5),
                t.text3,
            );
        }
        ui.painter()
            .hline(hr.x_range(), hr.bottom(), Stroke::new(1.0, t.border));
        let working: Vec<(f64, bool, f64)> = self
            .paper
            .account
            .orders
            .iter()
            .filter(|o| o.symbol == sym.id)
            .filter_map(|o| o.kind.price(o.buy).map(|p| (p, o.buy, o.qty)))
            .collect();
        let mut click: Option<(bool, f64)> = None;
        let mut row = |ui: &mut Ui, price: f64, bid: Option<f64>, ask: Option<f64>, mid: bool| {
            let (r, _) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::hover());
            let p = ui.painter();
            let cell = |k: usize| Rect::from_min_size(pos2(edges[k], r.top()), vec2(cols[k], ROW_H));
            if mid {
                p.rect_filled(r, CornerRadius::same(4), t.hover_wash());
            }
            if let Some(q) = bid {
                let c = cell(1);
                let bw = c.width() * (q / max) as f32;
                p.rect_filled(
                    Rect::from_min_max(pos2(c.right() - bw, c.top() + 3.0), pos2(c.right(), c.bottom() - 3.0)),
                    CornerRadius::same(2),
                    t.pos.gamma_multiply(0.22),
                );
                p.text(
                    c.right_center() - vec2(4.0, 0.0),
                    Align2::RIGHT_CENTER,
                    crate::format::compact(q),
                    theme::regular(11.5),
                    t.text,
                );
            }
            if let Some(q) = ask {
                let c = cell(3);
                let bw = c.width() * (q / max) as f32;
                p.rect_filled(
                    Rect::from_min_max(pos2(c.left(), c.top() + 3.0), pos2(c.left() + bw, c.bottom() - 3.0)),
                    CornerRadius::same(2),
                    t.neg.gamma_multiply(0.22),
                );
                p.text(
                    c.left_center() + vec2(4.0, 0.0),
                    Align2::LEFT_CENTER,
                    crate::format::compact(q),
                    theme::regular(11.5),
                    t.text,
                );
            }
            let pc = if mid {
                t.text
            } else if ask.is_some() {
                t.neg
            } else {
                t.pos
            };
            p.text(
                cell(2).center(),
                Align2::CENTER_CENTER,
                crate::format::price(price, prec),
                if mid {
                    theme::semibold(12.5)
                } else {
                    theme::regular(12.0)
                },
                pc,
            );
            // Working orders at this price.
            let tick = if sym.tick_size > 0.0 { sym.tick_size } else { 0.01 };
            for (op, buy, q) in &working {
                if (op - price).abs() < tick / 2.0 {
                    let c = cell(if *buy { 0 } else { 4 });
                    let tag = Rect::from_center_size(c.center(), vec2(c.width() - 6.0, ROW_H - 6.0));
                    p.rect_filled(tag, CornerRadius::same(3), if *buy { t.accent } else { t.neg });
                    p.text(
                        tag.center(),
                        Align2::CENTER_CENTER,
                        crate::format::compact(*q),
                        theme::medium(11.0),
                        egui::Color32::WHITE,
                    );
                }
            }
            if mid {
                return;
            }
            for (k, buy) in [(0usize, true), (4usize, false)] {
                let c = cell(k);
                let resp = ui.interact(c, ui.id().with(("dom", k, price.to_bits())), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_stroke(
                        c.shrink(2.0),
                        CornerRadius::same(3),
                        Stroke::new(1.0, if buy { t.accent } else { t.neg }),
                        egui::StrokeKind::Inside,
                    );
                }
                let tip = if buy { "Buy" } else { "Sell" };
                if resp
                    .on_hover_text(format!(
                        "{tip} {} at {}",
                        self.dom.qty,
                        crate::format::price(price, prec)
                    ))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    click = Some((buy, price));
                }
            }
        };
        for (p, q) in &asks {
            row(ui, *p, None, Some(*q), false);
        }
        let mid = if last.is_finite() {
            last
        } else {
            match (book.bids.first(), book.asks.first()) {
                (Some(b), Some(a)) => (a.0 + b.0) / 2.0,
                _ => f64::NAN,
            }
        };
        row(ui, mid, None, None, true);
        for (p, q) in &bids {
            row(ui, *p, Some(*q), None, false);
        }
        if let Some((buy, price)) = click {
            self.dom_order(buy, price);
        }
        if !self.paper.status.is_empty() {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(&self.paper.status).color(t.text3).size(11.5));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::drive::Driver;
    use opentern_engine::paper::Kind;

    #[test]
    fn the_ladder_loads_and_clicks_place_paper_orders() {
        let mut d = Driver::new("dom");
        d.app.layout.widget = Some(crate::shell::Widget::Dom);
        d.wait(|a| matches!(a.dom.book, Some(Ok(_))));
        let sym = d.app.chart.symbol.id.clone();
        let last = d.app.last_price(&sym).unwrap();
        // Below the market a buy is a limit; above it, a stop.
        d.app.dom_order(true, last * 0.99);
        d.app.dom_order(true, last * 1.01);
        d.app.dom_order(false, last * 1.01);
        let kinds: Vec<Kind> = d.app.paper.account.orders.iter().map(|o| o.kind).collect();
        assert!(matches!(kinds[0], Kind::Limit(_)));
        assert!(matches!(kinds[1], Kind::Stop(_)));
        assert!(matches!(kinds[2], Kind::Limit(_)));
        for _ in 0..3 {
            d.frame(vec![]);
        }
        assert_eq!(d.app.panics, 0);
    }
}

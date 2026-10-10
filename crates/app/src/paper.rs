//! The Paper Trading tab: a simulated account (`opentern_engine::paper`)
//! fed with live prices, an order ticket for the chart's symbol, positions,
//! working orders and history, and the position and order lines on the
//! chart. The account is saved in the workspace.

use crate::app::App;
use crate::chart::PaperLine;
use crate::shell::small_button;
use crate::theme::{self, Theme};
use egui::{Align, Align2, Color32, CornerRadius, Layout, Sense, Stroke, Ui, pos2, vec2};
use opentern_core::Symbol;
use opentern_engine::paper::{Account, Kind, Ticket};
use std::collections::HashMap;

const KEY: &str = "paper_account";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Positions,
    Orders,
    History,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OrderType {
    #[default]
    Market,
    Limit,
    Stop,
    StopLimit,
    Trailing,
}

impl OrderType {
    const ALL: [OrderType; 5] = [
        OrderType::Market,
        OrderType::Limit,
        OrderType::Stop,
        OrderType::StopLimit,
        OrderType::Trailing,
    ];

    fn name(self) -> &'static str {
        match self {
            OrderType::Market => "Market",
            OrderType::Limit => "Limit",
            OrderType::Stop => "Stop",
            OrderType::StopLimit => "Stop limit",
            OrderType::Trailing => "Trailing stop",
        }
    }
}

pub struct Paper {
    pub account: Account,
    pub view: View,
    pub buy: bool,
    pub kind: OrderType,
    pub qty: f64,
    pub price: f64,
    pub limit: f64,
    pub offset: f64,
    pub tp: Option<f64>,
    pub sl: Option<f64>,
    pub status: String,
    /// The last price fed per symbol (only new prices are fed).
    fed: HashMap<String, f64>,
    subscribed: Vec<String>,
    confirm_reset: bool,
}

impl Paper {
    pub fn load(store: &opentern_core::Store) -> Paper {
        let account = store
            .value(KEY)
            .and_then(|s| serde_json::from_str::<Account>(&s).ok())
            .unwrap_or_default();
        Paper {
            account,
            view: View::Positions,
            buy: true,
            kind: OrderType::Market,
            qty: 1.0,
            price: 0.0,
            limit: 0.0,
            offset: 0.0,
            tp: None,
            sl: None,
            status: String::new(),
            fed: HashMap::new(),
            subscribed: Vec::new(),
            confirm_reset: false,
        }
    }
}

fn now() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

impl App {
    fn save_paper(&mut self) {
        match serde_json::to_string(&self.paper.account) {
            Ok(s) => {
                if let Err(e) = self.store.set_value(KEY, &s) {
                    crate::diag::crumb(format!("couldn't save the paper account: {e}"));
                }
            }
            Err(e) => crate::diag::crumb(format!("couldn't save the paper account: {e}")),
        }
    }

    /// The latest price of a symbol: the chart's last bar, else its quote.
    pub(crate) fn last_price(&self, id: &str) -> Option<f64> {
        if id == self.chart.symbol.id {
            return self.chart.bars.last().map(|b| b.close);
        }
        self.quotes.get(id).map(|q| q.0)
    }

    /// Every frame: feeds new prices to the account, keeps quotes coming
    /// for its symbols, and puts its lines on the chart.
    pub(crate) fn poll_paper(&mut self) {
        let symbols = self.paper.account.symbols();
        let mut changed = false;
        for s in &symbols {
            let Some(p) = self.last_price(s) else { continue };
            if self.paper.fed.get(s) == Some(&p) {
                continue;
            }
            self.paper.fed.insert(s.clone(), p);
            let fills = self.paper.account.on_price(s, p, now());
            for f in &fills {
                let side = if f.buy { "Bought" } else { "Sold" };
                self.paper.status = format!("{side} {} {} at {}", qty_text(f.qty), short(&f.symbol), num(f.price));
                crate::diag::crumb(format!("paper fill: order {}", f.order));
            }
            changed |= !fills.is_empty();
        }
        if changed {
            self.save_paper();
        }
        let want: Vec<String> = self.paper.account.symbols();
        if want != self.paper.subscribed {
            self.paper.subscribed = want;
            self.subscribe_quotes();
        }
        self.paper_lines();
    }

    /// The paper position and orders on the chart's symbol, as lines.
    pub(crate) fn paper_lines(&mut self) {
        let id = self.chart.symbol.id.clone();
        let mut lines = Vec::new();
        if let Some(p) = self.paper.account.position(&id) {
            let pnl = self.last_price(&id).map(|l| (l - p.avg) * p.qty).unwrap_or(0.0);
            lines.push(PaperLine {
                price: p.avg,
                label: format!(
                    "{} {}  {}",
                    if p.qty > 0.0 { "Long" } else { "Short" },
                    qty_text(p.qty),
                    signed(pnl)
                ),
                buy: p.qty > 0.0,
                position: true,
            });
        }
        for o in self.paper.account.orders.iter().filter(|o| o.symbol == id) {
            if let Some(price) = o.kind.price(o.buy) {
                let what = if o.reduce_only {
                    if matches!(o.kind, Kind::Limit(_)) { "TP" } else { "SL" }
                } else {
                    o.kind.name()
                };
                lines.push(PaperLine {
                    price,
                    label: format!("{what} {} {}", if o.buy { "buy" } else { "sell" }, qty_text(o.qty)),
                    buy: o.buy,
                    position: false,
                });
            }
        }
        self.chart.paper = lines;
    }

    /// Symbols the account holds or has orders in, as quote subscriptions.
    pub(crate) fn paper_symbols(&self) -> Vec<Symbol> {
        self.paper.account.symbols().iter().map(|s| Symbol::bare(s)).collect()
    }

    pub(crate) fn paper_tab(&mut self, ui: &mut Ui, t: &Theme) {
        ui.horizontal_top(|ui| {
            let h = ui.available_height();
            ui.allocate_ui_with_layout(vec2(250.0, h), Layout::top_down(Align::Min), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("paper-ticket")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.ticket(ui, t));
            });
            let (r, _) = ui.allocate_exact_size(vec2(17.0, h), Sense::hover());
            ui.painter()
                .vline(r.center().x, r.y_range(), Stroke::new(1.0, t.border));
            ui.vertical(|ui| self.account_view(ui, t));
        });
    }

    fn ticket(&mut self, ui: &mut Ui, t: &Theme) {
        let sym = self.chart.symbol.id.clone();
        let last = self.last_price(&sym);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(short(&sym)).font(theme::semibold(14.0)));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(egui::RichText::new(last.map(num).unwrap_or("—".into())).color(t.text2));
            });
        });
        ui.add_space(6.0);
        // Buy / Sell.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (buy, label, c) in [(true, "Buy", t.pos), (false, "Sell", t.neg)] {
                let on = self.paper.buy == buy;
                let (r, resp) = ui.allocate_exact_size(vec2(119.0, 30.0), Sense::click());
                let fill = if on { c } else { t.card };
                ui.painter().rect(
                    r,
                    CornerRadius::same(7),
                    fill,
                    Stroke::new(1.0, if on { c } else { t.border }),
                    egui::StrokeKind::Inside,
                );
                ui.painter().text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    label,
                    theme::semibold(13.0),
                    if on { Color32::WHITE } else { t.text2 },
                );
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.paper.buy = buy;
                }
            }
        });
        ui.add_space(8.0);
        let row = |ui: &mut Ui, label: &str, f: &mut dyn FnMut(&mut Ui)| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(84.0, 24.0), Sense::hover());
                ui.painter().text(
                    r.left_center(),
                    Align2::LEFT_CENTER,
                    label,
                    theme::regular(12.5),
                    t.text2,
                );
                f(ui);
            });
        };
        let mut kind = self.paper.kind;
        row(ui, "Type", &mut |ui| {
            egui::ComboBox::from_id_salt("paper-type")
                .width(150.0)
                .selected_text(kind.name())
                .show_ui(ui, |ui| {
                    for k in OrderType::ALL {
                        ui.selectable_value(&mut kind, k, k.name());
                    }
                });
        });
        if kind != self.paper.kind {
            self.paper.kind = kind;
            if let Some(l) = last {
                self.paper.price = round_to(l, self.chart.symbol.tick_size);
                self.paper.limit = self.paper.price;
                self.paper.offset = round_to(l * 0.01, self.chart.symbol.tick_size);
            }
        }
        let step = self.chart.symbol.tick_size.max(1e-8);
        let mut qty = self.paper.qty;
        row(ui, "Quantity", &mut |ui| {
            ui.add(
                egui::DragValue::new(&mut qty)
                    .range(0.0..=1e12)
                    .speed(0.1)
                    .max_decimals(6),
            );
        });
        self.paper.qty = qty;
        let (mut price, mut limit, mut offset) = (self.paper.price, self.paper.limit, self.paper.offset);
        match self.paper.kind {
            OrderType::Limit => row(ui, "Limit price", &mut |ui| {
                ui.add(egui::DragValue::new(&mut price).speed(step).max_decimals(8));
            }),
            OrderType::Stop => row(ui, "Stop price", &mut |ui| {
                ui.add(egui::DragValue::new(&mut price).speed(step).max_decimals(8));
            }),
            OrderType::StopLimit => {
                row(ui, "Stop price", &mut |ui| {
                    ui.add(egui::DragValue::new(&mut price).speed(step).max_decimals(8));
                });
                row(ui, "Limit price", &mut |ui| {
                    ui.add(egui::DragValue::new(&mut limit).speed(step).max_decimals(8));
                });
            }
            OrderType::Trailing => row(ui, "Trail by", &mut |ui| {
                ui.add(
                    egui::DragValue::new(&mut offset)
                        .speed(step)
                        .max_decimals(8)
                        .range(0.0..=1e12),
                );
            }),
            OrderType::Market => {}
        }
        (self.paper.price, self.paper.limit, self.paper.offset) = (price, limit, offset);
        // Take-profit and stop-loss.
        let reference = match self.paper.kind {
            OrderType::Market | OrderType::Trailing => last,
            OrderType::StopLimit => Some(self.paper.limit),
            _ => Some(self.paper.price),
        };
        let dir = if self.paper.buy { 1.0 } else { -1.0 };
        for (tp, label) in [(true, "Take profit"), (false, "Stop loss")] {
            let slot = if tp { &mut self.paper.tp } else { &mut self.paper.sl };
            let mut on = slot.is_some();
            ui.horizontal(|ui| {
                if ui.checkbox(&mut on, label).changed() {
                    *slot = if on {
                        reference.map(|r| round_to(r * (1.0 + dir * if tp { 0.02 } else { -0.01 }), step))
                    } else {
                        None
                    };
                }
                if let Some(v) = slot.as_mut() {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add(egui::DragValue::new(v).speed(step).max_decimals(8));
                    });
                }
            });
        }
        ui.add_space(8.0);
        let side = if self.paper.buy { "Buy" } else { "Sell" };
        let label = format!("{side} {} {}", qty_text(self.paper.qty), short(&sym));
        let c = if self.paper.buy { t.pos } else { t.neg };
        let (r, resp) = ui.allocate_exact_size(vec2(244.0, 34.0), Sense::click());
        let hover = resp.hovered();
        ui.painter()
            .rect_filled(r, CornerRadius::same(8), if hover { c.gamma_multiply(0.85) } else { c });
        ui.painter().text(
            r.center(),
            Align2::CENTER_CENTER,
            label,
            theme::semibold(13.5),
            Color32::WHITE,
        );
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            let kind = match self.paper.kind {
                OrderType::Market => Kind::Market,
                OrderType::Limit => Kind::Limit(self.paper.price),
                OrderType::Stop => Kind::Stop(self.paper.price),
                OrderType::StopLimit => Kind::StopLimit {
                    stop: self.paper.price,
                    limit: self.paper.limit,
                    armed: false,
                },
                OrderType::Trailing => Kind::Trailing {
                    offset: self.paper.offset,
                    best: 0.0,
                },
            };
            let ticket = Ticket {
                symbol: sym.clone(),
                buy: self.paper.buy,
                qty: self.paper.qty,
                kind,
                take_profit: self.paper.tp,
                stop_loss: self.paper.sl,
            };
            match self.paper.account.place(ticket, last, now()) {
                Ok(_) => {
                    self.paper.status = match self.paper.kind {
                        OrderType::Market => format!("{side} order filled."),
                        _ => "Order placed.".into(),
                    };
                    self.save_paper();
                }
                Err(e) => self.paper.status = e,
            }
        }
        if !self.paper.status.is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&self.paper.status).color(t.text3).size(12.0));
        }
    }

    fn account_view(&mut self, ui: &mut Ui, t: &Theme) {
        let acc = &self.paper.account;
        let prices: HashMap<String, f64> = acc
            .symbols()
            .into_iter()
            .filter_map(|s| self.last_price(&s).map(|p| (s, p)))
            .collect();
        let price = |s: &str| prices.get(s).copied();
        let unreal = acc.unrealized(price);
        let equity = acc.equity(price);
        let realized = acc.balance - acc.initial;
        let balance = acc.balance;
        // Four even columns that never wrap.
        let items = [
            ("Balance", num(balance), t.text),
            ("Equity", num(equity), t.text),
            ("Unrealized", signed(unreal), tone(t, unreal)),
            ("Realized", signed(realized), tone(t, realized)),
        ];
        let w = ui.available_width();
        let (r, _) = ui.allocate_exact_size(vec2(w, 38.0), Sense::hover());
        let cw = w / items.len() as f32;
        for (k, (label, value, c)) in items.iter().enumerate() {
            let cell = egui::Rect::from_min_size(pos2(r.left() + cw * k as f32, r.top()), vec2(cw - 8.0, 38.0));
            let p = ui.painter_at(cell);
            p.text(cell.left_top(), Align2::LEFT_TOP, *label, theme::regular(11.5), t.text3);
            p.text(
                cell.left_top() + vec2(0.0, 17.0),
                Align2::LEFT_TOP,
                value,
                theme::semibold(14.0),
                *c,
            );
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let counts = (
                self.paper.account.positions.len(),
                self.paper.account.orders.len(),
                self.paper.account.fills.len(),
            );
            for (v, label) in [
                (View::Positions, format!("Positions {}", counts.0)),
                (View::Orders, format!("Orders {}", counts.1)),
                (View::History, format!("History {}", counts.2)),
            ] {
                if small_button(ui, t, &label, self.paper.view == v).clicked() {
                    self.paper.view = v;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.paper.confirm_reset {
                    if small_button(ui, t, "Cancel", false).clicked() {
                        self.paper.confirm_reset = false;
                    }
                    if small_button(ui, t, "Yes, reset", true).clicked() {
                        let commission = self.paper.account.commission_pct;
                        self.paper.account = Account::new("Paper account", self.paper.account.initial);
                        self.paper.account.commission_pct = commission;
                        self.paper.confirm_reset = false;
                        self.paper.status = "The account was reset.".into();
                        self.save_paper();
                    }
                    ui.label(egui::RichText::new("Clear positions, orders and history?").color(t.text2));
                } else if small_button(ui, t, "Reset account", false).clicked() {
                    self.paper.confirm_reset = true;
                }
            });
        });
        ui.add_space(4.0);
        match self.paper.view {
            View::Positions => self.positions_table(ui, t, &prices),
            View::Orders => self.orders_table(ui, t),
            View::History => self.history_table(ui, t),
        }
    }

    fn positions_table(&mut self, ui: &mut Ui, t: &Theme, prices: &HashMap<String, f64>) {
        let rows: Vec<_> = self.paper.account.positions.clone();
        if rows.is_empty() {
            empty(ui, t, "No open positions.");
            return;
        }
        let cols = [
            ("Symbol", 150.0),
            ("Side", 70.0),
            ("Qty", 90.0),
            ("Avg price", 110.0),
            ("Last", 110.0),
            ("P&L", 150.0),
            ("", 80.0),
        ];
        header(ui, t, &cols);
        let mut close = None;
        egui::ScrollArea::vertical().id_salt("paper-pos").show(ui, |ui| {
            for p in &rows {
                let last = prices.get(&p.symbol).copied();
                let pnl = last.map(|l| (l - p.avg) * p.qty);
                let cells = [
                    (short(&p.symbol), t.text),
                    (
                        if p.qty > 0.0 { "Long" } else { "Short" }.to_string(),
                        if p.qty > 0.0 { t.pos } else { t.neg },
                    ),
                    (qty_text(p.qty), t.text),
                    (num(p.avg), t.text),
                    (last.map(num).unwrap_or("—".into()), t.text2),
                    (pnl.map(signed).unwrap_or("—".into()), tone(t, pnl.unwrap_or(0.0))),
                ];
                if row_with_action(ui, t, &cols, &cells, "Close") {
                    close = Some((p.symbol.clone(), last));
                }
            }
        });
        if let Some((s, last)) = close {
            match last {
                Some(l) => {
                    self.paper.account.close_position(&s, l, now());
                    self.paper.status = format!("Closed {}.", short(&s));
                    self.save_paper();
                }
                None => self.paper.status = "No price yet for this symbol.".into(),
            }
        }
    }

    fn orders_table(&mut self, ui: &mut Ui, t: &Theme) {
        let rows = self.paper.account.orders.clone();
        if rows.is_empty() {
            empty(ui, t, "No working orders.");
            return;
        }
        let cols = [
            ("Symbol", 150.0),
            ("Side", 70.0),
            ("Type", 110.0),
            ("Qty", 90.0),
            ("Price", 110.0),
            ("Placed", 140.0),
            ("", 80.0),
        ];
        header(ui, t, &cols);
        let mut cancel = None;
        egui::ScrollArea::vertical().id_salt("paper-ord").show(ui, |ui| {
            for o in &rows {
                let kind = if o.reduce_only {
                    if matches!(o.kind, Kind::Limit(_)) {
                        "Take profit"
                    } else {
                        "Stop loss"
                    }
                } else {
                    o.kind.name()
                };
                let cells = [
                    (short(&o.symbol), t.text),
                    (
                        if o.buy { "Buy" } else { "Sell" }.to_string(),
                        if o.buy { t.pos } else { t.neg },
                    ),
                    (kind.to_string(), t.text2),
                    (qty_text(o.qty), t.text),
                    (o.kind.price(o.buy).map(num).unwrap_or("—".into()), t.text),
                    (time_text(o.placed, &self.chart.tz), t.text2),
                ];
                if row_with_action(ui, t, &cols, &cells, "Cancel") {
                    cancel = Some(o.id);
                }
            }
        });
        if let Some(id) = cancel {
            self.paper.account.cancel(id);
            self.paper.status = "Order cancelled.".into();
            self.save_paper();
        }
    }

    fn history_table(&mut self, ui: &mut Ui, t: &Theme) {
        let fills = &self.paper.account.fills;
        if fills.is_empty() {
            empty(ui, t, "Nothing traded yet.");
            return;
        }
        let cols = [
            ("Time", 140.0),
            ("Symbol", 150.0),
            ("Side", 70.0),
            ("Qty", 90.0),
            ("Price", 110.0),
            ("Realized", 140.0),
            ("Commission", 110.0),
        ];
        header(ui, t, &cols);
        let n = fills.len();
        egui::ScrollArea::vertical()
            .id_salt("paper-hist")
            .auto_shrink([false; 2])
            .show_rows(ui, 24.0, n, |ui, range| {
                for k in range {
                    let f = &fills[n - 1 - k];
                    let cells = [
                        (time_text(f.time, &self.chart.tz), t.text2),
                        (short(&f.symbol), t.text),
                        (
                            if f.buy { "Buy" } else { "Sell" }.to_string(),
                            if f.buy { t.pos } else { t.neg },
                        ),
                        (qty_text(f.qty), t.text),
                        (num(f.price), t.text),
                        (signed(f.realized), tone(t, f.realized)),
                        (num(f.commission), t.text2),
                    ];
                    row_with_action(ui, t, &cols, &cells, "");
                }
            });
    }
}

fn empty(ui: &mut Ui, t: &Theme, what: &str) {
    ui.add_space(8.0);
    ui.label(egui::RichText::new(what).color(t.text3));
}

fn header(ui: &mut Ui, t: &Theme, cols: &[(&str, f32)]) {
    let w: f32 = cols.iter().map(|c| c.1).sum();
    let (r, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
    let mut x = r.left();
    for (name, cw) in cols {
        ui.painter().text(
            pos2(x + 8.0, r.center().y),
            Align2::LEFT_CENTER,
            *name,
            theme::medium(12.0),
            t.text3,
        );
        x += cw;
    }
    ui.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0, t.border));
}

/// A row of cells and, in the last column, an action button. Returns true
/// when the action was clicked.
fn row_with_action(ui: &mut Ui, t: &Theme, cols: &[(&str, f32)], cells: &[(String, Color32)], action: &str) -> bool {
    let w: f32 = cols.iter().map(|c| c.1).sum();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(4), t.hover_wash());
    }
    let mut x = r.left();
    for ((_, cw), (text, c)) in cols.iter().zip(cells) {
        ui.painter().text(
            pos2(x + 8.0, r.center().y),
            Align2::LEFT_CENTER,
            text,
            theme::regular(12.5),
            *c,
        );
        x += cw;
    }
    let mut clicked = false;
    if !action.is_empty()
        && let Some((_, aw)) = cols.last()
    {
        let br = egui::Rect::from_min_size(pos2(r.right() - aw + 6.0, r.top() + 1.0), vec2(aw - 12.0, 22.0));
        let b = ui.interact(br, ui.id().with((action, r.top() as i64)), Sense::click());
        let col = if b.hovered() { t.text } else { t.text2 };
        ui.painter().rect_stroke(
            br,
            CornerRadius::same(5),
            Stroke::new(1.0, t.border),
            egui::StrokeKind::Inside,
        );
        ui.painter()
            .text(br.center(), Align2::CENTER_CENTER, action, theme::medium(12.0), col);
        clicked = b.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
    }
    ui.painter()
        .hline(r.x_range(), r.bottom(), Stroke::new(1.0, t.border.gamma_multiply(0.5)));
    clicked
}

fn short(id: &str) -> String {
    id.rsplit(':').next().unwrap_or(id).to_string()
}

fn qty_text(q: f64) -> String {
    let s = format!("{:.6}", q.abs());
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn num(v: f64) -> String {
    let plain = num_plain(v);
    // Thousands separators on the whole part.
    let (sign, rest) = match plain.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", plain.as_str()),
    };
    let (int, frac) = rest.split_once('.').unwrap_or((rest, ""));
    let mut g = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i).is_multiple_of(3) {
            g.push(',');
        }
        g.push(c);
    }
    if frac.is_empty() {
        format!("{sign}{g}")
    } else {
        format!("{sign}{g}.{frac}")
    }
}

fn num_plain(v: f64) -> String {
    let dec = if v.abs() >= 1000.0 {
        2
    } else if v.abs() >= 1.0 {
        4
    } else {
        6
    };
    let s = format!("{v:.dec$}");
    if s.contains('.') {
        let t = s.trim_end_matches('0');
        let t = t.strip_suffix('.').unwrap_or(t);
        // Keep at least two decimals for prices.
        match t.split_once('.') {
            Some((_, f)) if f.len() >= 2 => t.to_string(),
            _ => format!("{v:.2}"),
        }
    } else {
        s
    }
}

fn signed(v: f64) -> String {
    if v > 0.0 {
        format!("+{}", num(v))
    } else if v < 0.0 {
        format!("−{}", num(-v))
    } else {
        num(0.0)
    }
}

fn tone(t: &Theme, v: f64) -> Color32 {
    if v > 0.0 {
        t.pos
    } else if v < 0.0 {
        t.neg
    } else {
        t.text
    }
}

fn round_to(v: f64, step: f64) -> f64 {
    if step > 0.0 { (v / step).round() * step } else { v }
}

fn time_text(ms: i64, tz: &jiff::tz::TimeZone) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|ts| ts.to_zoned(tz.clone()).strftime("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_text() {
        assert_eq!(num(64210.5), "64,210.50");
        assert_eq!(num(-1234567.0), "-1,234,567.00");
        assert_eq!(num(1.23456), "1.2346");
        assert_eq!(num(1.5), "1.50");
        assert_eq!(signed(-2.0), "−2.0000".replace("2.0000", &num(2.0)));
        assert_eq!(short("BINANCE:BTCUSDT"), "BTCUSDT");
    }
}

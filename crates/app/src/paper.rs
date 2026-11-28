//! The Paper Trading tab: a simulated account (`opentern_engine::paper`)
//! fed with live prices, an order ticket for the chart's symbol, positions,
//! working orders and history, and the position and order lines on the
//! chart. The account is saved in the workspace.

use crate::app::App;
use crate::chart::PaperLine;
use crate::i18n::{msg, nt, tr, trc, trf, trm};
use crate::shell::small_button;
use crate::theme::{self, Theme};
use egui::{Align, Align2, Color32, CornerRadius, Layout, Sense, Stroke, Ui, pos2, vec2};
use opentern_core::Symbol;
use opentern_engine::paper::{Account, Kind, Ticket, same_money};
use std::collections::HashMap;

/// Before several accounts: one account under this key (migrated).
const OLD_KEY: &str = "paper_account";
const KEY: &str = "paper_accounts";

/// What's saved: every account, and which one is in use.
#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    accounts: Vec<Account>,
    active: usize,
}

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
            OrderType::Market => nt!("Market"),
            OrderType::Limit => nt!("Limit"),
            OrderType::Stop => nt!("Stop"),
            OrderType::StopLimit => nt!("Stop limit"),
            OrderType::Trailing => nt!("Trailing stop"),
        }
    }
}

pub struct Paper {
    /// The account in use (the ticket, tables and chart lines show it).
    pub account: Account,
    /// The other accounts; their orders keep working.
    pub others: Vec<Account>,
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
        let (account, others) = match store.value(KEY).and_then(|s| serde_json::from_str::<Saved>(&s).ok()) {
            Some(mut s) if !s.accounts.is_empty() => {
                let k = s.active.min(s.accounts.len() - 1);
                let a = s.accounts.remove(k);
                (a, s.accounts)
            }
            _ => (
                store
                    .value(OLD_KEY)
                    .and_then(|s| serde_json::from_str::<Account>(&s).ok())
                    .unwrap_or_else(|| {
                        let mut a = Account::default();
                        a.name = tr("Paper account").into();
                        a
                    }),
                Vec::new(),
            ),
        };
        Paper {
            account,
            others,
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

/// Below this width a panel's two halves stack.
pub(crate) const STACKED: f32 = 560.0;

/// An order type as it's drawn ("Stop" the order isn't "Stop" the verb).
pub(crate) fn order_name(name: &str) -> &str {
    match name {
        "Stop" => trc("order", "Stop"),
        n => tr(n),
    }
}

impl App {
    pub(crate) fn save_paper_now(&mut self) {
        self.save_paper();
    }

    fn save_paper(&mut self) {
        let mut accounts = vec![self.paper.account.clone()];
        accounts.extend(self.paper.others.iter().cloned());
        match serde_json::to_string(&Saved { accounts, active: 0 }) {
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

    /// A symbol's currency: from its details when they're loaded (the
    /// chart's, a watchlist's), else from its id; "" when unknown.
    pub(crate) fn currency_of(&self, id: &str) -> String {
        if self.chart.symbol.id == id && !self.chart.symbol.currency.is_empty() {
            return self.chart.symbol.currency.clone();
        }
        self.watchlists
            .iter()
            .flat_map(|w| w.symbols())
            .find(|s| s.id == id && !s.currency.is_empty())
            .map(|s| s.currency)
            .unwrap_or_else(|| opentern_data::expr::guess_currency(id))
    }

    /// Feeds each account the exchange rates it needs (its positions' and
    /// orders' currencies, and the chart symbol's for the one in use) from
    /// forex quotes. Returns the forex symbols to keep quoted.
    fn feed_rates(&mut self) -> Vec<String> {
        // Stablecoins trade as dollars.
        let money = |c: &str| {
            if same_money(c, "USD") {
                "USD".to_string()
            } else {
                c.to_uppercase()
            }
        };
        let chart_ccy = self.currency_of(&self.chart.symbol.id);
        let mut wanted: Vec<(usize, String, String)> = Vec::new();
        let paper = &self.paper;
        for (k, acc) in std::iter::once(&paper.account).chain(&paper.others).enumerate() {
            if acc.currency.is_empty() {
                continue;
            }
            let mut ccys = acc.currencies();
            if k == 0 && !chart_ccy.is_empty() && !same_money(&chart_ccy, &acc.currency) {
                ccys.push(chart_ccy.to_uppercase());
            }
            for c in ccys {
                let pair = format!("FX:{}{}", money(&c), money(&acc.currency));
                wanted.push((k, c, pair));
            }
        }
        let mut pairs = Vec::new();
        for (k, c, pair) in wanted {
            if let Some(r) = self.last_price(&pair) {
                let acc = if k == 0 {
                    &mut self.paper.account
                } else {
                    &mut self.paper.others[k - 1]
                };
                acc.fx.insert(c, r);
            }
            pairs.push(pair);
        }
        pairs
    }

    /// Every frame: feeds new prices to the account, keeps quotes coming
    /// for its symbols, and puts its lines on the chart.
    pub(crate) fn poll_paper(&mut self) {
        let mut symbols = self.paper.account.symbols();
        for a in &self.paper.others {
            symbols.extend(a.symbols());
        }
        symbols.sort();
        symbols.dedup();
        let pairs = self.feed_rates();
        let mut changed = false;
        for s in &symbols {
            let Some(p) = self.last_price(s) else { continue };
            if self.paper.fed.get(s) == Some(&p) {
                continue;
            }
            self.paper.fed.insert(s.clone(), p);
            let paper = &mut self.paper;
            let many = !paper.others.is_empty();
            for acc in std::iter::once(&mut paper.account).chain(paper.others.iter_mut()) {
                let fills = acc.on_price(s, p, now());
                for f in &fills {
                    let (account, qty, sym, price) = (&acc.name, qty_text(f.qty), short(&f.symbol), num(f.price));
                    paper.status = match (many, f.buy) {
                        (false, true) => msg!("Bought {qty} {sym} at {price}"),
                        (false, false) => msg!("Sold {qty} {sym} at {price}"),
                        (true, true) => msg!("{account}: Bought {qty} {sym} at {price}"),
                        (true, false) => msg!("{account}: Sold {qty} {sym} at {price}"),
                    };
                    crate::diag::crumb(format!("paper fill: order {}", f.order));
                }
                changed |= !fills.is_empty();
            }
        }
        // Orders refused when they triggered, margin calls.
        let many = !self.paper.others.is_empty();
        let mut notes = Vec::new();
        for acc in std::iter::once(&mut self.paper.account).chain(self.paper.others.iter_mut()) {
            for n in acc.notices.drain(..) {
                notes.push(if many { format!("{}: {n}", acc.name) } else { n });
            }
        }
        for n in notes {
            self.paper.status = n.clone();
            self.notify(crate::icons::ph::WALLET, nt!("Paper trading"), &n, None);
            changed = true;
        }
        if changed {
            self.save_paper();
        }
        let mut want: Vec<String> = symbols;
        want.extend(pairs);
        want.sort();
        want.dedup();
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
                    if p.qty > 0.0 { tr("Long") } else { tr("Short") },
                    qty_text(p.qty),
                    signed(pnl)
                ),
                buy: p.qty > 0.0,
                position: true,
                order: None,
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
                    label: format!(
                        "{} {} {}",
                        order_name(what),
                        if o.buy { tr("buy") } else { tr("sell") },
                        qty_text(o.qty)
                    ),
                    buy: o.buy,
                    position: false,
                    order: Some(o.id),
                });
            }
        }
        self.chart.paper = lines;
    }

    /// Symbols the accounts hold or have orders in, as quote subscriptions.
    pub(crate) fn paper_symbols(&self) -> Vec<Symbol> {
        self.paper.subscribed.iter().map(|s| Symbol::bare(s)).collect()
    }

    /// Chart trading: an order's line was dragged.
    pub(crate) fn paper_move(&mut self, id: u64, price: f64) {
        match self.paper.account.modify(id, price) {
            Ok(()) => {
                self.paper.status = msg!("Order moved to {}.", num(price));
                self.save_paper();
            }
            Err(e) => self.paper.status = e,
        }
    }

    pub(crate) fn paper_cancel(&mut self, id: u64) {
        if self.paper.account.cancel(id) {
            self.paper.status = nt!("Order cancelled.").into();
            self.save_paper();
        }
    }

    pub(crate) fn paper_close(&mut self, symbol: &str) {
        match self.last_price(symbol) {
            Some(l) => {
                self.paper.account.close_position(symbol, l, now());
                self.paper.status = msg!("Closed {}.", short(symbol));
                self.save_paper();
            }
            None => self.paper.status = nt!("No price yet for this symbol.").into(),
        }
    }

    /// Makes account `k` of the others the one in use.
    pub(crate) fn switch_account(&mut self, k: usize) {
        if k < self.paper.others.len() {
            std::mem::swap(&mut self.paper.account, &mut self.paper.others[k]);
            // Keep the others in their order: the previous one goes first.
            let prev = self.paper.others.remove(k);
            self.paper.others.insert(0, prev);
            self.paper.status = msg!("Using {}.", self.paper.account.name);
            self.save_paper();
        }
    }

    /// A free "Paper account N" name.
    fn free_account_name(&self) -> String {
        (2..)
            .map(|k| trf!("Paper account {k}"))
            .find(|n| !self.paper_name_taken(n, true))
            .expect("a free name")
    }

    /// Whether another account has this name (`new`: any account; else any
    /// but the one in use).
    pub(crate) fn paper_name_taken(&self, name: &str, new: bool) -> bool {
        let used = (new && self.paper.account.name == name) || self.paper.others.iter().any(|a| a.name == name);
        used && !name.is_empty()
    }

    /// Opens the account dialog: a new account (starting as the one in use
    /// is set up, under a free name), or the one in use.
    pub(crate) fn open_paper_account(&mut self, new: bool) {
        let a = &self.paper.account;
        let form = crate::dialogs::paper_account::PaperAccountForm {
            new,
            name: if new { self.free_account_name() } else { a.name.clone() },
            capital: a.initial,
            commission: a.commission_pct,
            slippage: a.slippage_pct,
            leverage: a.leverage.max(1.0),
            currency: a.currency.clone(),
            // Money already in one currency can't switch to another.
            currency_locked: !new && !(a.fills.is_empty() && a.positions.is_empty() && a.orders.is_empty()),
        };
        self.modal = Some(crate::dialogs::Modal::PaperAccount(Box::new(form)));
    }

    /// Makes the account the dialog describes, or saves its settings to the
    /// one in use (commission applies to fills from now on).
    pub(crate) fn apply_paper_account(&mut self, f: &crate::dialogs::paper_account::PaperAccountForm) {
        let name = f.name.trim().to_string();
        let set = |a: &mut Account| {
            a.commission_pct = f.commission;
            a.slippage_pct = f.slippage;
            a.leverage = f.leverage.max(1.0);
            if !f.currency_locked {
                a.currency = f.currency.clone();
                a.fx.clear();
            }
        };
        if f.new {
            let mut fresh = Account::new(&name, f.capital);
            set(&mut fresh);
            let prev = std::mem::replace(&mut self.paper.account, fresh);
            self.paper.others.insert(0, prev);
            self.paper.status = msg!("Made {name}.");
        } else {
            let a = &mut self.paper.account;
            a.name = name;
            a.set_initial(f.capital);
            set(a);
        }
        self.save_paper();
    }

    #[cfg(test)]
    pub(crate) fn new_account_for_test(&mut self) {
        let form = crate::dialogs::paper_account::PaperAccountForm {
            new: true,
            name: self.free_account_name(),
            capital: 100_000.0,
            commission: 0.0,
            slippage: 0.0,
            leverage: 1.0,
            currency: String::new(),
            currency_locked: false,
        };
        self.apply_paper_account(&form);
    }

    #[cfg(test)]
    pub(crate) fn switch_account_for_test(&mut self, k: usize) {
        self.switch_account(k);
    }

    pub(crate) fn delete_account(&mut self) {
        if self.paper.others.is_empty() {
            return;
        }
        let gone = std::mem::replace(&mut self.paper.account, self.paper.others.remove(0));
        self.paper.status = msg!("Deleted {}.", gone.name);
        self.save_paper();
    }

    pub(crate) fn paper_tab(&mut self, ui: &mut Ui, t: &Theme) {
        // No room side by side: the order form, then the account below it.
        if ui.available_width() < STACKED {
            egui::ScrollArea::vertical()
                .id_salt("paper-stacked")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.ticket(ui, t);
                    ui.add_space(10.0);
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter()
                        .hline(r.x_range(), r.center().y, Stroke::new(1.0, t.border));
                    ui.add_space(10.0);
                    // A height of its own: a long history scrolls inside it.
                    let size = vec2(ui.available_width(), 420.0);
                    ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| self.account_view(ui, t));
                });
            return;
        }
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
            for (buy, label, c) in [(true, tr("Buy"), t.pos), (false, tr("Sell"), t.neg)] {
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
        row(ui, tr("Type"), &mut |ui| {
            crate::widgets::combo("paper-type")
                .width(150.0)
                .selected_text(order_name(kind.name()))
                .show_ui(ui, |ui| {
                    for k in OrderType::ALL {
                        crate::widgets::option_value(ui, &mut kind, k, order_name(k.name()));
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
        row(ui, tr("Quantity"), &mut |ui| {
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
            OrderType::Limit => row(ui, tr("Limit price"), &mut |ui| {
                ui.add(egui::DragValue::new(&mut price).speed(step).max_decimals(8));
            }),
            OrderType::Stop => row(ui, tr("Stop price"), &mut |ui| {
                ui.add(egui::DragValue::new(&mut price).speed(step).max_decimals(8));
            }),
            OrderType::StopLimit => {
                row(ui, tr("Stop price"), &mut |ui| {
                    ui.add(egui::DragValue::new(&mut price).speed(step).max_decimals(8));
                });
                row(ui, tr("Limit price"), &mut |ui| {
                    ui.add(egui::DragValue::new(&mut limit).speed(step).max_decimals(8));
                });
            }
            OrderType::Trailing => row(ui, tr("Trail by"), &mut |ui| {
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
        for (tp, label) in [(true, tr("Take profit")), (false, tr("Stop loss"))] {
            let slot = if tp { &mut self.paper.tp } else { &mut self.paper.sl };
            let mut on = slot.is_some();
            ui.horizontal(|ui| {
                if crate::widgets::checkbox(ui, &mut on, label).changed() {
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
        let side = if self.paper.buy { tr("Buy") } else { tr("Sell") };
        let label = format!("{side} {} {}", qty_text(self.paper.qty), short(&sym));
        let c = if self.paper.buy { t.pos } else { t.neg };
        let (r, resp) = ui.allocate_exact_size(vec2(244.0, 34.0), Sense::click());
        let hover = crate::motion::hover(&resp);
        ui.painter()
            .rect_filled(r, CornerRadius::same(8), c.gamma_multiply(1.0 - 0.15 * hover));
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
                currency: self.currency_of(&sym),
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
                        OrderType::Market if self.paper.buy => nt!("Buy order filled.").into(),
                        OrderType::Market => nt!("Sell order filled.").into(),
                        _ => nt!("Order placed.").into(),
                    };
                    self.save_paper();
                }
                Err(e) => self.paper.status = e,
            }
        }
        if !self.paper.status.is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(trm(&self.paper.status)).color(t.text3).size(12.0));
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
        let unit = if acc.currency.is_empty() {
            String::new()
        } else {
            format!(" {}", acc.currency)
        };
        let margin = trf!(
            "Margin in use {}{unit} · Buying power {}{unit}",
            num(acc.margin_used()),
            num(acc.buying_power().max(0.0))
        );
        // The account, and resetting it, on top.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            self.account_menu(ui, t);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.paper.confirm_reset {
                    if small_button(ui, t, tr("Cancel"), false).clicked() {
                        self.paper.confirm_reset = false;
                    }
                    if small_button(ui, t, tr("Yes, reset"), true).clicked() {
                        self.paper.account.reset();
                        self.paper.confirm_reset = false;
                        self.paper.status = nt!("The account was reset.").into();
                        self.save_paper();
                    }
                    ui.label(egui::RichText::new(tr("Clear everything?")).color(t.text2));
                } else if small_button(ui, t, tr("Reset account"), false).clicked() {
                    self.paper.confirm_reset = true;
                }
            });
        });
        ui.add_space(8.0);
        // Four even columns, or two by two when the panel is narrow; values
        // never get cut off.
        let items = [
            (tr("Balance"), format!("{}{unit}", num(balance)), t.text),
            (tr("Equity"), format!("{}{unit}", num(equity)), t.text),
            (tr("Unrealized"), signed(unreal), tone(t, unreal)),
            (tr("Realized"), signed(realized), tone(t, realized)),
        ];
        let w = ui.available_width();
        let per_row = if w / 4.0 >= 120.0 { 4 } else { 2 };
        let rows = items.len() / per_row;
        let (r, resp) = ui.allocate_exact_size(vec2(w, 40.0 * rows as f32), Sense::hover());
        resp.on_hover_text(margin);
        let cw = w / per_row as f32;
        for (k, (label, value, c)) in items.iter().enumerate() {
            let (col, row) = (k % per_row, k / per_row);
            let cell = egui::Rect::from_min_size(
                pos2(r.left() + cw * col as f32, r.top() + 40.0 * row as f32),
                vec2(cw - 8.0, 38.0),
            );
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
        let counts = (
            self.paper.account.positions.len(),
            self.paper.account.orders.len(),
            self.paper.account.fills.len(),
        );
        let labels = [
            trf!("Positions {}", counts.0),
            trf!("Orders {}", counts.1),
            trf!("History {}", counts.2),
        ];
        let mut view = self.paper.view;
        crate::widgets::segmented(
            ui,
            t,
            &mut view,
            &[
                (View::Positions, labels[0].as_str()),
                (View::Orders, labels[1].as_str()),
                (View::History, labels[2].as_str()),
            ],
        );
        self.paper.view = view;
        ui.add_space(4.0);
        // The tables are as wide as their columns; they scroll sideways.
        egui::ScrollArea::horizontal()
            .id_salt(("paper-table", self.paper.view as u8))
            .auto_shrink([false, true])
            .show(ui, |ui| match self.paper.view {
                View::Positions => self.positions_table(ui, t, &prices),
                View::Orders => self.orders_table(ui, t),
                View::History => self.history_table(ui, t),
            });
    }

    /// The account picker: switch, make, rename and delete accounts.
    fn account_menu(&mut self, ui: &mut Ui, t: &Theme) {
        let current = self.paper.account.name.clone();
        let mut pick = None;
        let mut make = false;
        let mut settings = false;
        let mut delete = false;
        let resp = crate::widgets::button(
            ui,
            crate::widgets::Kind::Secondary,
            true,
            Some(crate::icons::ph::WALLET),
            &format!("{current}  {}", crate::icons::ph::CARET_DOWN),
        );
        egui::Popup::menu(&resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                use crate::widgets as w;
                ui.set_width(260.0);
                ui.spacing_mut().item_spacing.y = 1.0;
                w::menu_heading(ui, t, tr("ACCOUNTS"));
                w::menu_item(ui, t, Some(crate::icons::ph::WALLET), &current, tr("in use"), true);
                for (k, a) in self.paper.others.iter().enumerate() {
                    if w::menu_item(ui, t, Some(crate::icons::ph::WALLET), &a.name, "", false).clicked() {
                        pick = Some(k);
                        ui.close();
                    }
                }
                w::menu_rule(ui, t);
                if w::menu_item(
                    ui,
                    t,
                    Some(crate::icons::ph::GEAR_SIX),
                    tr("Account settings"),
                    "",
                    false,
                )
                .clicked()
                {
                    settings = true;
                    ui.close();
                }
                if w::menu_item(ui, t, Some(crate::icons::ph::PLUS), tr("New account"), "", false).clicked() {
                    make = true;
                    ui.close();
                }
                let can = !self.paper.others.is_empty();
                let r = if can {
                    w::menu_danger(ui, t, Some(crate::icons::ph::TRASH), tr("Delete this account"))
                } else {
                    w::menu_row(
                        ui,
                        t,
                        Some(crate::icons::ph::TRASH),
                        tr("Delete this account"),
                        "",
                        false,
                        false,
                    )
                    .on_hover_text(tr("Make another account first"))
                };
                if can && r.clicked() {
                    delete = true;
                    ui.close();
                }
            });
        if let Some(k) = pick {
            self.switch_account(k);
        }
        if make {
            self.open_paper_account(true);
        }
        if settings {
            self.open_paper_account(false);
        }
        if delete {
            self.delete_account();
        }
    }

    fn positions_table(&mut self, ui: &mut Ui, t: &Theme, prices: &HashMap<String, f64>) {
        let rows: Vec<_> = self.paper.account.positions.clone();
        if rows.is_empty() {
            empty(ui, t, nt!("No open positions."));
            return;
        }
        let cols = [
            (tr("Symbol"), 150.0),
            (tr("Side"), 70.0),
            (tr("Qty"), 90.0),
            (tr("Avg price"), 110.0),
            (tr("Last"), 110.0),
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
                        if p.qty > 0.0 { tr("Long") } else { tr("Short") }.to_string(),
                        if p.qty > 0.0 { t.pos } else { t.neg },
                    ),
                    (qty_text(p.qty), t.text),
                    (num(p.avg), t.text),
                    (last.map(num).unwrap_or("—".into()), t.text2),
                    (pnl.map(signed).unwrap_or("—".into()), tone(t, pnl.unwrap_or(0.0))),
                ];
                if row_with_action(ui, t, &cols, &cells, tr("Close")) {
                    close = Some((p.symbol.clone(), last));
                }
            }
        });
        if let Some((s, last)) = close {
            match last {
                Some(l) => {
                    self.paper.account.close_position(&s, l, now());
                    self.paper.status = msg!("Closed {}.", short(&s));
                    self.save_paper();
                }
                None => self.paper.status = nt!("No price yet for this symbol.").into(),
            }
        }
    }

    fn orders_table(&mut self, ui: &mut Ui, t: &Theme) {
        let rows = self.paper.account.orders.clone();
        if rows.is_empty() {
            empty(ui, t, nt!("No working orders."));
            return;
        }
        let cols = [
            (tr("Symbol"), 150.0),
            (tr("Side"), 70.0),
            (tr("Type"), 110.0),
            (tr("Qty"), 90.0),
            (tr("Price"), 110.0),
            (tr("Placed"), 140.0),
            ("", 80.0),
        ];
        header(ui, t, &cols);
        let mut cancel = None;
        egui::ScrollArea::vertical().id_salt("paper-ord").show(ui, |ui| {
            for o in &rows {
                let kind = if o.reduce_only {
                    if matches!(o.kind, Kind::Limit(_)) {
                        tr("Take profit")
                    } else {
                        tr("Stop loss")
                    }
                } else {
                    o.kind.name()
                };
                let cells = [
                    (short(&o.symbol), t.text),
                    (
                        if o.buy { tr("Buy") } else { tr("Sell") }.to_string(),
                        if o.buy { t.pos } else { t.neg },
                    ),
                    (kind.to_string(), t.text2),
                    (qty_text(o.qty), t.text),
                    (o.kind.price(o.buy).map(num).unwrap_or("—".into()), t.text),
                    (time_text(o.placed, &self.chart.tz), t.text2),
                ];
                if row_with_action(ui, t, &cols, &cells, tr("Cancel")) {
                    cancel = Some(o.id);
                }
            }
        });
        if let Some(id) = cancel {
            self.paper.account.cancel(id);
            self.paper.status = nt!("Order cancelled.").into();
            self.save_paper();
        }
    }

    fn history_table(&mut self, ui: &mut Ui, t: &Theme) {
        let fills = &self.paper.account.fills;
        if fills.is_empty() {
            empty(ui, t, nt!("Nothing traded yet."));
            return;
        }
        let cols = [
            (tr("Time"), 140.0),
            (tr("Symbol"), 150.0),
            (tr("Side"), 70.0),
            (tr("Qty"), 90.0),
            (tr("Price"), 110.0),
            (tr("Realized"), 140.0),
            (tr("Commission"), 110.0),
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
                            if f.buy { tr("Buy") } else { tr("Sell") }.to_string(),
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
    let icon = match what {
        "No open positions." => crate::icons::ph::STACK,
        "No working orders." => crate::icons::ph::LIST_CHECKS,
        _ => crate::icons::ph::RECEIPT,
    };
    crate::widgets::empty_state(ui, t, icon, tr(what).trim_end_matches(['.', '。', '।']));
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
    let (r, row) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
    let hover = crate::motion::hover_if(&row, ui.rect_contains_pointer(r));
    ui.painter().rect_filled(
        r,
        CornerRadius::same(4),
        crate::motion::with_alpha(t.hover_wash(), hover),
    );
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
        let col = crate::motion::lerp_color(t.text2, t.text, crate::motion::hover(&b));
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

//! The broker emulator. Driven bar by bar:
//!
//! 1. [`Broker::open_bar`] fills what was ordered on the previous bar:
//!    market orders at this bar's open, then stops and limits along the
//!    intrabar path TradingView assumes (open → the nearer of high/low →
//!    the other → close).
//! 2. The strategy runs on the bar's close and places orders with
//!    [`Broker::submit`] (it can read the position meanwhile).
//! 3. [`Broker::close_bar`] fills market orders at the close when
//!    `process_orders_on_close` is set, and records equity.

use opentern_core::Bar;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QtyType {
    Fixed,
    Cash,
    PercentOfEquity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Commission {
    Percent,
    CashPerContract,
    CashPerOrder,
}

/// `strategy(...)`'s properties (TradingView's defaults).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Props {
    pub initial_capital: f64,
    pub qty_type: QtyType,
    pub qty_value: f64,
    /// Extra entries allowed in the same direction (0 = one position).
    pub pyramiding: u32,
    pub commission: Commission,
    pub commission_value: f64,
    /// Ticks added against every market and stop fill.
    pub slippage: u32,
    pub process_orders_on_close: bool,
    /// The symbol's tick size (for slippage and tick-based exits).
    pub mintick: f64,
    /// Margin for longs and shorts, percent (100 = no leverage).
    pub margin_long: f64,
    pub margin_short: f64,
}

impl Default for Props {
    fn default() -> Self {
        Props {
            initial_capital: 1_000_000.0,
            qty_type: QtyType::Fixed,
            qty_value: 1.0,
            pyramiding: 0,
            commission: Commission::Percent,
            commission_value: 0.0,
            slippage: 0,
            process_orders_on_close: false,
            mintick: 0.01,
            margin_long: 100.0,
            margin_short: 100.0,
        }
    }
}

impl Props {
    /// From Pine/Python `strategy(...)` arguments as JSON.
    pub fn from_json(v: &serde_json::Value, mintick: f64) -> Props {
        let d = Props::default();
        let f = |k: &str, def: f64| v.get(k).and_then(|x| x.as_f64()).unwrap_or(def);
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("");
        Props {
            initial_capital: f("initial_capital", d.initial_capital),
            qty_type: match s("default_qty_type") {
                "cash" | "strategy.cash" => QtyType::Cash,
                "percent_of_equity" | "strategy.percent_of_equity" => QtyType::PercentOfEquity,
                _ => QtyType::Fixed,
            },
            qty_value: f("default_qty_value", 1.0),
            pyramiding: f("pyramiding", 0.0) as u32,
            commission: match s("commission_type") {
                "cash_per_contract" | "strategy.commission.cash_per_contract" => Commission::CashPerContract,
                "cash_per_order" | "strategy.commission.cash_per_order" => Commission::CashPerOrder,
                _ => Commission::Percent,
            },
            commission_value: f("commission_value", 0.0),
            slippage: f("slippage", 0.0) as u32,
            process_orders_on_close: v
                .get("process_orders_on_close")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
            mintick: if mintick > 0.0 { mintick } else { d.mintick },
            margin_long: f("margin_long", 100.0),
            margin_short: f("margin_short", 100.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderKind {
    /// `strategy.entry`: reverses an opposite position; honours pyramiding.
    Entry,
    /// `strategy.order`: adds to or reduces the position as is.
    Order,
    /// `strategy.exit`: a bracket (stop and/or limit) on open entries.
    Exit,
    /// `strategy.close` / `strategy.close_all` (empty id): at market.
    Close,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Order {
    pub id: String,
    pub kind: OrderKind,
    pub long: bool,
    /// Contracts; `None` uses the strategy's default size.
    pub qty: Option<f64>,
    pub limit: Option<f64>,
    pub stop: Option<f64>,
    /// Exits: which entry they close ("" = all).
    pub from_entry: String,
    /// Exits and closes: how much of the position, in percent.
    pub qty_percent: f64,
    /// Exits: profit target / stop loss in ticks from the entry price.
    pub profit_ticks: Option<f64>,
    pub loss_ticks: Option<f64>,
    pub comment: String,
    /// `strategy.close(..., immediately = true)`: fill at this close.
    pub immediately: bool,
}

impl Order {
    pub fn market(id: &str, kind: OrderKind, long: bool) -> Order {
        Order {
            id: id.to_string(),
            kind,
            long,
            qty: None,
            limit: None,
            stop: None,
            from_entry: String::new(),
            qty_percent: 100.0,
            profit_ticks: None,
            loss_ticks: None,
            comment: String::new(),
            immediately: false,
        }
    }
}

/// An open entry (positions are a FIFO list of entries).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lot {
    pub id: String,
    pub long: bool,
    pub qty: f64,
    pub price: f64,
    pub bar: usize,
    pub time: i64,
    /// Entry commission still to be attributed to the closing trade(s).
    pub commission: f64,
    pub comment: String,
    /// Best and worst price seen while open (for run-up and drawdown).
    pub hi: f64,
    pub lo: f64,
}

/// A closed trade, as in the Strategy Report's List of trades.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trade {
    pub entry_id: String,
    pub exit_id: String,
    pub long: bool,
    pub qty: f64,
    pub entry_price: f64,
    pub entry_bar: usize,
    pub entry_time: i64,
    pub exit_price: f64,
    pub exit_bar: usize,
    pub exit_time: i64,
    /// Net of commission.
    pub pnl: f64,
    pub pnl_pct: f64,
    pub commission: f64,
    /// Largest unrealised profit / loss while open (money).
    pub runup: f64,
    pub drawdown: f64,
    pub entry_comment: String,
    pub exit_comment: String,
}

/// A fill, for the chart's trade markers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fill {
    pub bar: usize,
    pub time: i64,
    pub price: f64,
    /// Signed: + bought, − sold.
    pub qty: f64,
    pub id: String,
    pub comment: String,
    /// True for exits (closing fills).
    pub exit: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Broker {
    pub props: Props,
    pub lots: Vec<Lot>,
    pub trades: Vec<Trade>,
    pub fills: Vec<Fill>,
    /// Orders waiting to fill.
    pub pending: Vec<Order>,
    pub net_profit: f64,
    pub commission_paid: f64,
    /// Equity at each bar's close, and its intrabar best and worst.
    pub equity: Vec<f64>,
    pub equity_hi: Vec<f64>,
    pub equity_lo: Vec<f64>,
    /// Signed position size at each bar's close.
    pub position: Vec<f64>,
    bar: usize,
    time: i64,
    last_close: f64,
}

impl Broker {
    pub fn new(props: Props) -> Broker {
        Broker {
            props,
            ..Default::default()
        }
    }

    /// Signed position size: + long, − short.
    pub fn position_size(&self) -> f64 {
        self.lots.iter().map(|l| if l.long { l.qty } else { -l.qty }).sum()
    }

    pub fn position_avg_price(&self) -> f64 {
        let q: f64 = self.lots.iter().map(|l| l.qty).sum();
        if q == 0.0 {
            return f64::NAN;
        }
        self.lots.iter().map(|l| l.qty * l.price).sum::<f64>() / q
    }

    pub fn open_profit(&self, price: f64) -> f64 {
        self.lots
            .iter()
            .map(|l| (price - l.price) * l.qty * if l.long { 1.0 } else { -1.0 })
            .sum()
    }

    pub fn equity_at(&self, price: f64) -> f64 {
        self.props.initial_capital + self.net_profit + self.open_profit(price)
    }

    pub fn wins(&self) -> usize {
        self.trades.iter().filter(|t| t.pnl > 0.0).count()
    }

    pub fn losses(&self) -> usize {
        self.trades.iter().filter(|t| t.pnl < 0.0).count()
    }

    /// Places or replaces (same id and kind) an order.
    pub fn submit(&mut self, o: Order) {
        self.pending.retain(|p| !(p.id == o.id && p.kind == o.kind));
        self.pending.push(o);
    }

    pub fn cancel(&mut self, id: &str) {
        self.pending.retain(|p| p.id != id || p.kind == OrderKind::Close);
    }

    pub fn cancel_all(&mut self) {
        self.pending.retain(|p| p.kind == OrderKind::Close);
    }

    fn commission(&self, qty: f64, price: f64) -> f64 {
        let v = self.props.commission_value;
        match self.props.commission {
            Commission::Percent => qty * price * v / 100.0,
            Commission::CashPerContract => qty * v,
            Commission::CashPerOrder => v,
        }
    }

    fn default_qty(&self, price: f64) -> f64 {
        let v = self.props.qty_value;
        match self.props.qty_type {
            QtyType::Fixed => v,
            QtyType::Cash => {
                if price > 0.0 {
                    v / price
                } else {
                    0.0
                }
            }
            QtyType::PercentOfEquity => {
                if price > 0.0 {
                    self.equity_at(price) * v / 100.0 / price
                } else {
                    0.0
                }
            }
        }
    }

    fn slip(&self, price: f64, buy: bool) -> f64 {
        let s = self.props.slippage as f64 * self.props.mintick;
        if buy { price + s } else { price - s }
    }

    /// Opens or adds to a position.
    fn open(&mut self, o: &Order, qty: f64, price: f64) {
        if qty <= 0.0 {
            return;
        }
        let commission = self.commission(qty, price);
        self.commission_paid += commission;
        self.lots.push(Lot {
            id: o.id.clone(),
            long: o.long,
            qty,
            price,
            bar: self.bar,
            time: self.time,
            commission,
            comment: o.comment.clone(),
            hi: price,
            lo: price,
        });
        self.fills.push(Fill {
            bar: self.bar,
            time: self.time,
            price,
            qty: if o.long { qty } else { -qty },
            id: o.id.clone(),
            comment: o.comment.clone(),
            exit: false,
        });
    }

    /// Closes up to `qty` of the lots matching `from` (FIFO), at `price`.
    fn close(&mut self, from: &str, qty: f64, price: f64, exit_id: &str, comment: &str) {
        let mut left = qty;
        let mut i = 0;
        let mut sold = 0.0;
        let mut long = true;
        while i < self.lots.len() && left > 1e-12 {
            if !from.is_empty() && self.lots[i].id != from {
                i += 1;
                continue;
            }
            let lot = self.lots[i].clone();
            let q = lot.qty.min(left);
            let share = q / lot.qty;
            let entry_comm = lot.commission * share;
            let exit_comm = self.commission(q, price);
            self.commission_paid += exit_comm;
            let dir = if lot.long { 1.0 } else { -1.0 };
            let gross = (price - lot.price) * q * dir;
            let pnl = gross - entry_comm - exit_comm;
            // The exit price counts too (an exit at the open of a gap bar).
            let (hi, lo) = (lot.hi.max(price), lot.lo.min(price));
            let (best, worst) = if lot.long { (hi, lo) } else { (lo, hi) };
            self.trades.push(Trade {
                entry_id: lot.id.clone(),
                exit_id: exit_id.to_string(),
                long: lot.long,
                qty: q,
                entry_price: lot.price,
                entry_bar: lot.bar,
                entry_time: lot.time,
                exit_price: price,
                exit_bar: self.bar,
                exit_time: self.time,
                pnl,
                pnl_pct: if lot.price != 0.0 {
                    (price / lot.price - 1.0) * 100.0 * dir
                } else {
                    0.0
                },
                commission: entry_comm + exit_comm,
                runup: ((best - lot.price) * q * dir).max(0.0),
                drawdown: ((worst - lot.price) * q * dir).min(0.0),
                entry_comment: lot.comment.clone(),
                exit_comment: comment.to_string(),
            });
            self.net_profit += pnl;
            sold += q;
            long = lot.long;
            left -= q;
            if q >= lot.qty - 1e-12 {
                self.lots.remove(i);
            } else {
                self.lots[i].qty -= q;
                self.lots[i].commission -= entry_comm;
                i += 1;
            }
        }
        if sold > 0.0 {
            self.fills.push(Fill {
                bar: self.bar,
                time: self.time,
                price,
                qty: if long { -sold } else { sold },
                id: exit_id.to_string(),
                comment: comment.to_string(),
                exit: true,
            });
        }
    }

    fn open_qty(&self, from: &str) -> f64 {
        self.lots
            .iter()
            .filter(|l| from.is_empty() || l.id == from)
            .map(|l| l.qty)
            .sum()
    }

    /// Executes an entry/order/close that has triggered at `price`.
    fn execute(&mut self, o: &Order, price: f64) {
        match o.kind {
            OrderKind::Close => {
                let q = self.open_qty(&o.id) * o.qty_percent / 100.0;
                let long = self.position_size() > 0.0;
                let p = self.slip(price, !long);
                self.close(
                    &o.id,
                    q,
                    p,
                    &format!("Close entry(s) order {}", if o.id.is_empty() { "all" } else { &o.id }),
                    &o.comment,
                );
            }
            OrderKind::Entry => {
                let pos = self.position_size();
                let opposite = (pos > 0.0 && !o.long) || (pos < 0.0 && o.long);
                let p = self.slip(price, o.long);
                if opposite {
                    // Reverse: close everything first.
                    let q = self.open_qty("");
                    self.close("", q, p, &o.id, &o.comment);
                } else if pos != 0.0 {
                    let entries = self.lots.len() as u32;
                    if entries > self.props.pyramiding {
                        return;
                    }
                }
                let qty = o.qty.unwrap_or_else(|| self.default_qty(p));
                self.open(o, qty, p);
            }
            OrderKind::Order => {
                let pos = self.position_size();
                let p = self.slip(price, o.long);
                let mut qty = o.qty.unwrap_or_else(|| self.default_qty(p));
                // Against the position, an order reduces it first.
                if (pos > 0.0 && !o.long) || (pos < 0.0 && o.long) {
                    let q = qty.min(pos.abs());
                    self.close("", q, p, &o.id, &o.comment);
                    qty -= q;
                }
                if qty > 1e-12 {
                    self.open(o, qty, p);
                }
            }
            OrderKind::Exit => {}
        }
    }

    /// Stop/limit trigger for an order going `long` (buying) over a path
    /// segment from `a` to `b`: the fill price, if it triggers.
    fn trigger(limit: Option<f64>, stop: Option<f64>, buy: bool, a: f64, b: f64) -> Option<f64> {
        let (lo, hi) = (a.min(b), a.max(b));
        // A price already through the level at the segment start fills at
        // the start (a gap).
        let at = |level: f64, better_when_below: bool| -> Option<f64> {
            if better_when_below {
                // Fills when price falls to the level.
                if a <= level {
                    Some(a)
                } else if lo <= level {
                    Some(level)
                } else {
                    None
                }
            } else if a >= level {
                Some(a)
            } else if hi >= level {
                Some(level)
            } else {
                None
            }
        };
        match (limit, stop) {
            (Some(l), None) => at(l, buy),
            (None, Some(s)) => at(s, !buy),
            (Some(l), Some(s)) => {
                // Stop-limit: the stop arms the limit; treated as the stop
                // price clamped by the limit.
                at(s, !buy).map(|p| if buy { p.min(l) } else { p.max(l) })
            }
            (None, None) => Some(a),
        }
    }

    /// The prices of an exit order for the lots it covers.
    fn exit_levels(&self, o: &Order) -> Option<(bool, f64, Option<f64>, Option<f64>)> {
        let lots: Vec<&Lot> = self
            .lots
            .iter()
            .filter(|l| o.from_entry.is_empty() || l.id == o.from_entry)
            .collect();
        let first = lots.first()?;
        let long = first.long;
        let qty: f64 = lots.iter().map(|l| l.qty).sum::<f64>() * o.qty_percent / 100.0;
        let avg = lots.iter().map(|l| l.qty * l.price).sum::<f64>() / lots.iter().map(|l| l.qty).sum::<f64>();
        let t = self.props.mintick;
        let dir = if long { 1.0 } else { -1.0 };
        let limit = o.limit.or(o.profit_ticks.map(|p| avg + dir * p * t));
        let stop = o.stop.or(o.loss_ticks.map(|p| avg - dir * p * t));
        Some((long, qty, limit, stop))
    }

    /// Fills what was ordered on the previous bar against this bar.
    pub fn open_bar(&mut self, index: usize, bar: &Bar) {
        self.bar = index;
        self.time = bar.time;
        // Market orders at the open.
        let pending = std::mem::take(&mut self.pending);
        let (market, mut waiting): (Vec<Order>, Vec<Order>) = pending.into_iter().partition(|o| {
            matches!(o.kind, OrderKind::Close) || (o.kind != OrderKind::Exit && o.limit.is_none() && o.stop.is_none())
        });
        for o in &market {
            self.execute(o, bar.open);
        }
        // Then stops and limits along the intrabar path.
        let path = if (bar.high - bar.open).abs() <= (bar.open - bar.low).abs() {
            [bar.open, bar.high, bar.low, bar.close]
        } else {
            [bar.open, bar.low, bar.high, bar.close]
        };
        for seg in 0..3 {
            let (a, b) = (path[seg], path[seg + 1]);
            let mut k = 0;
            while k < waiting.len() {
                let o = waiting[k].clone();
                let filled = match o.kind {
                    OrderKind::Exit => match self.exit_levels(&o) {
                        None => false,
                        Some((long, qty, limit, stop)) => {
                            // Selling a long (or buying back a short).
                            let buy = !long;
                            let hit_l = limit.and_then(|l| Self::trigger(Some(l), None, buy, a, b));
                            let hit_s = stop.and_then(|s| Self::trigger(None, Some(s), buy, a, b));
                            // If both trigger in one segment, the one nearer
                            // the segment start comes first.
                            let pick = match (hit_l, hit_s) {
                                (Some(l), Some(s)) => {
                                    if (l - a).abs() <= (s - a).abs() {
                                        Some((l, false))
                                    } else {
                                        Some((s, true))
                                    }
                                }
                                (Some(l), None) => Some((l, false)),
                                (None, Some(s)) => Some((s, true)),
                                (None, None) => None,
                            };
                            match pick {
                                Some((p, is_stop)) => {
                                    let p = if is_stop { self.slip(p, buy) } else { p };
                                    let from = o.from_entry.clone();
                                    self.close(&from, qty, p, &o.id, &o.comment);
                                    true
                                }
                                None => false,
                            }
                        }
                    },
                    _ => match Self::trigger(o.limit, o.stop, o.long, a, b) {
                        Some(p) => {
                            self.execute(&o, p);
                            true
                        }
                        None => false,
                    },
                };
                if filled {
                    waiting.remove(k);
                } else {
                    k += 1;
                }
            }
        }
        // Unfilled orders (exits included) wait for later bars.
        self.pending = waiting;
        // Track run-up and drawdown of open lots.
        for l in &mut self.lots {
            l.hi = l.hi.max(bar.high);
            l.lo = l.lo.min(bar.low);
        }
    }

    /// End of the bar: market orders fill at the close when the strategy
    /// processes orders on close (or `immediately`); equity is recorded.
    pub fn close_bar(&mut self, bar: &Bar) {
        let now: Vec<Order> = self
            .pending
            .iter()
            .filter(|o| {
                let market =
                    o.kind == OrderKind::Close || (o.kind != OrderKind::Exit && o.limit.is_none() && o.stop.is_none());
                market && (self.props.process_orders_on_close || o.immediately)
            })
            .cloned()
            .collect();
        if !now.is_empty() {
            self.pending.retain(|o| !now.contains(o));
            for o in &now {
                self.execute(o, bar.close);
            }
        }
        self.last_close = bar.close;
        let eq = self.equity_at(bar.close);
        // Intrabar extremes of equity: open positions at the bar's high/low.
        let (a, b) = (self.equity_at(bar.high), self.equity_at(bar.low));
        self.equity.push(eq);
        self.equity_hi.push(a.max(b).max(eq));
        self.equity_lo.push(a.min(b).min(eq));
        self.position.push(self.position_size());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(t: i64, o: f64, h: f64, l: f64, c: f64) -> Bar {
        Bar {
            time: t,
            open: o,
            high: h,
            low: l,
            close: c,
            volume: 1.0,
        }
    }

    /// Runs bars through the broker; `orders(i, broker)` places orders on
    /// bar i's close.
    fn run(bars: &[Bar], props: Props, mut orders: impl FnMut(usize, &mut Broker)) -> Broker {
        let mut b = Broker::new(props);
        for (i, x) in bars.iter().enumerate() {
            b.open_bar(i, x);
            orders(i, &mut b);
            b.close_bar(x);
        }
        b
    }

    #[test]
    fn market_entry_fills_next_open_and_close_fills_the_open_after() {
        let bars = [
            bar(0, 10.0, 11.0, 9.0, 10.0),
            bar(1, 12.0, 13.0, 11.0, 12.5),
            bar(2, 14.0, 15.0, 13.0, 14.0),
            bar(3, 15.0, 16.0, 14.0, 15.0),
        ];
        let b = run(&bars, Props::default(), |i, b| {
            if i == 0 {
                b.submit(Order::market("L", OrderKind::Entry, true));
            }
            if i == 1 {
                b.submit(Order::market("L", OrderKind::Close, true));
            }
        });
        assert_eq!(b.trades.len(), 1);
        let t = &b.trades[0];
        assert_eq!(
            (t.entry_price, t.entry_bar, t.exit_price, t.exit_bar),
            (12.0, 1, 14.0, 2)
        );
        assert_eq!(t.pnl, 2.0);
        // Run-up: bar 1's high (13) and the exit at 14 → 2; worst low 11 → -1.
        // Bar 2's high came after the exit, so it doesn't count.
        assert_eq!((t.runup, t.drawdown), (2.0, -1.0));
        assert_eq!(b.position, vec![0.0, 1.0, 0.0, 0.0]);
        assert_eq!(b.equity[1], 1_000_000.0 + 0.5);
    }

    #[test]
    fn entry_in_the_other_direction_reverses() {
        let bars = [
            bar(0, 10.0, 10.0, 10.0, 10.0),
            bar(1, 10.0, 10.0, 10.0, 10.0),
            bar(2, 8.0, 8.0, 8.0, 8.0),
            bar(3, 8.0, 8.0, 8.0, 8.0),
        ];
        let b = run(&bars, Props::default(), |i, b| match i {
            0 => b.submit(Order::market("L", OrderKind::Entry, true)),
            1 => b.submit(Order::market("S", OrderKind::Entry, false)),
            _ => {}
        });
        assert_eq!(b.trades.len(), 1);
        assert_eq!(b.trades[0].pnl, -2.0);
        assert_eq!(b.position_size(), -1.0);
        assert_eq!(b.lots[0].price, 8.0);
    }

    #[test]
    fn pyramiding_limits_entries() {
        let bars: Vec<Bar> = (0..6).map(|i| bar(i, 10.0, 10.0, 10.0, 10.0)).collect();
        let b = run(&bars, Props::default(), |_, b| {
            b.submit(Order::market("L", OrderKind::Entry, true))
        });
        assert_eq!(b.position_size(), 1.0, "pyramiding 0 allows one entry");
        let props = Props {
            pyramiding: 2,
            ..Props::default()
        };
        let b = run(&bars, props, |_, b| {
            b.submit(Order::market("L", OrderKind::Entry, true))
        });
        assert_eq!(b.position_size(), 3.0);
    }

    #[test]
    fn exit_bracket_fills_stop_or_limit_along_the_intrabar_path() {
        // Entry at 100; target 110, stop 95. Bar 2's open is nearer its
        // high, so the path is O→H→L: the high (111) hits the target first.
        let bars = [
            bar(0, 100.0, 100.0, 100.0, 100.0),
            bar(1, 100.0, 101.0, 99.0, 100.0),
            bar(2, 108.0, 111.0, 90.0, 92.0),
        ];
        let b = run(&bars, Props::default(), |i, b| {
            if i == 0 {
                b.submit(Order::market("L", OrderKind::Entry, true));
                let mut x = Order::market("X", OrderKind::Exit, true);
                x.from_entry = "L".into();
                x.limit = Some(110.0);
                x.stop = Some(95.0);
                b.submit(x);
            }
        });
        assert_eq!(b.trades.len(), 1);
        assert_eq!(b.trades[0].exit_price, 110.0);
        // Same bar but opening near the low: O→L→H, so the stop goes first.
        let bars = [
            bar(0, 100.0, 100.0, 100.0, 100.0),
            bar(1, 100.0, 101.0, 99.0, 100.0),
            bar(2, 92.0, 111.0, 90.0, 92.0),
        ];
        let b = run(&bars, Props::default(), |i, b| {
            if i == 0 {
                b.submit(Order::market("L", OrderKind::Entry, true));
                let mut x = Order::market("X", OrderKind::Exit, true);
                x.from_entry = "L".into();
                x.limit = Some(110.0);
                x.stop = Some(95.0);
                b.submit(x);
            }
        });
        // It gapped below the stop: filled at the open.
        assert_eq!(b.trades[0].exit_price, 92.0);
    }

    #[test]
    fn tick_based_exits_commission_and_slippage() {
        let props = Props {
            mintick: 0.5,
            slippage: 2,
            commission: Commission::Percent,
            commission_value: 0.1,
            ..Props::default()
        };
        let bars = [
            bar(0, 100.0, 100.0, 100.0, 100.0),
            bar(1, 100.0, 100.0, 100.0, 100.0),
            bar(2, 100.0, 106.0, 100.0, 105.0),
        ];
        let b = run(&bars, props, |i, b| {
            if i == 0 {
                b.submit(Order::market("L", OrderKind::Entry, true));
                let mut x = Order::market("TP", OrderKind::Exit, true);
                x.profit_ticks = Some(8.0);
                b.submit(x);
            }
        });
        let t = &b.trades[0];
        // Market entry with 2 ticks of slippage: 101; target 8 ticks above.
        assert_eq!(t.entry_price, 101.0);
        assert_eq!(t.exit_price, 105.0);
        let comm = 101.0 * 0.001 + 105.0 * 0.001;
        assert!((t.commission - comm).abs() < 1e-12);
        assert!((t.pnl - (4.0 - comm)).abs() < 1e-12);
    }

    #[test]
    fn percent_of_equity_and_process_on_close() {
        let props = Props {
            initial_capital: 1000.0,
            qty_type: QtyType::PercentOfEquity,
            qty_value: 50.0,
            process_orders_on_close: true,
            ..Props::default()
        };
        let bars = [bar(0, 10.0, 10.0, 10.0, 10.0), bar(1, 20.0, 20.0, 20.0, 20.0)];
        let b = run(&bars, props, |i, b| {
            if i == 0 {
                b.submit(Order::market("L", OrderKind::Entry, true));
            }
        });
        // Filled at bar 0's close: 50% of 1000 at 10 = 50 contracts.
        assert_eq!(b.lots[0].qty, 50.0);
        assert_eq!(b.lots[0].bar, 0);
        assert_eq!(b.equity[1], 1000.0 + 50.0 * 10.0);
    }
}

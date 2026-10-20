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
    /// `strategy.risk.allow_entry_in`: only longs (`Some(true)`) or only
    /// shorts (`Some(false)`); an entry the other way closes the position.
    pub allow_entry_in: Option<bool>,
    /// The smallest tradable quantity (`syminfo.mincontract`): sizes
    /// worked out from cash or equity are rounded down to it, as are
    /// margin-call liquidations. 0 = fractional.
    #[serde(default)]
    pub min_qty: f64,
    /// Pine v6's `strategy.exit`: given both `limit` and `profit` (or
    /// `stop` and `loss`), the level price reaches first wins. v5 always
    /// took the absolute price.
    #[serde(default = "yes")]
    pub nearest_exit: bool,
}

fn yes() -> bool {
    true
}

/// `syminfo.mincontract` by `syminfo.type`: whole shares and contracts;
/// crypto and forex trade in fractions.
pub fn min_qty_for(kind: &str) -> f64 {
    match kind {
        "crypto" | "forex" => 0.0,
        _ => 1.0,
    }
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
            allow_entry_in: None,
            min_qty: 0.0,
            nearest_exit: true,
        }
    }
}

/// `base` with `over`'s keys replaced (both JSON objects).
pub fn merge_json(base: &serde_json::Value, over: &serde_json::Value) -> serde_json::Value {
    let mut out = base.as_object().cloned().unwrap_or_default();
    if let Some(o) = over.as_object() {
        for (k, v) in o {
            out.insert(k.clone(), v.clone());
        }
    }
    serde_json::Value::Object(out)
}

impl Props {
    /// From Pine/Python `strategy(...)` arguments as JSON, for a symbol
    /// with this tick size and minimum quantity ([`min_qty_for`]).
    /// `pine_version` (set by the Pine compiler) picks v5's or v6's rules.
    pub fn from_json(v: &serde_json::Value, mintick: f64, min_qty: f64) -> Props {
        let d = Props::default();
        let f = |k: &str, def: f64| v.get(k).and_then(|x| x.as_f64()).unwrap_or(def);
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("");
        let v6 = f("pine_version", 6.0) >= 6.0;
        // Margin defaults to 100% in Pine v6 and to 0 (never checked) in v5.
        let margin = if v6 { 100.0 } else { 0.0 };
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
            margin_long: f("margin_long", margin),
            margin_short: f("margin_short", margin),
            allow_entry_in: None,
            min_qty,
            nearest_exit: v6,
        }
    }

    /// The `strategy(...)` arguments these properties come from (the
    /// inverse of [`Props::from_json`], minus `mintick`).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "initial_capital": self.initial_capital,
            "default_qty_type": match self.qty_type {
                QtyType::Fixed => "fixed",
                QtyType::Cash => "cash",
                QtyType::PercentOfEquity => "percent_of_equity",
            },
            "default_qty_value": self.qty_value,
            "pyramiding": self.pyramiding,
            "commission_type": match self.commission {
                Commission::Percent => "percent",
                Commission::CashPerContract => "cash_per_contract",
                Commission::CashPerOrder => "cash_per_order",
            },
            "commission_value": self.commission_value,
            "slippage": self.slippage,
            "process_orders_on_close": self.process_orders_on_close,
            "margin_long": self.margin_long,
            "margin_short": self.margin_short,
        })
    }

    /// The keys of `self.to_json()` that differ from `base`'s.
    pub fn diff_json(&self, base: &Props) -> serde_json::Value {
        let (a, b) = (self.to_json(), base.to_json());
        let mut out = serde_json::Map::new();
        if let (Some(a), Some(b)) = (a.as_object(), b.as_object()) {
            for (k, v) in a {
                if b.get(k) != Some(v) {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
        serde_json::Value::Object(out)
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

/// An exit's side (long), quantity, average entry price, target and stop.
type ExitLevels = (bool, f64, f64, Option<f64>, Option<f64>);

/// What an order in a one-cancels-all group does to the others when it fills.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OcaType {
    #[default]
    None,
    Cancel,
    Reduce,
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
    /// Exits: a trailing stop. It activates `trail_points` ticks into
    /// profit (or at `trail_price`) and then trails the best price by
    /// `trail_offset` ticks.
    #[serde(default)]
    pub trail_points: Option<f64>,
    #[serde(default)]
    pub trail_price: Option<f64>,
    #[serde(default)]
    pub trail_offset: Option<f64>,
    /// The best price since the trailing stop activated (state).
    #[serde(default)]
    pub trail_best: Option<f64>,
    /// Exits: comments for the target, the stop and the trailing stop.
    #[serde(default)]
    pub comment_profit: String,
    #[serde(default)]
    pub comment_loss: String,
    #[serde(default)]
    pub comment_trailing: String,
    #[serde(default)]
    pub oca_name: String,
    #[serde(default)]
    pub oca_type: OcaType,
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
            trail_points: None,
            trail_price: None,
            trail_offset: None,
            trail_best: None,
            comment_profit: String::new(),
            comment_loss: String::new(),
            comment_trailing: String::new(),
            oca_name: String::new(),
            oca_type: OcaType::None,
        }
    }

    fn has_trail(&self) -> bool {
        self.trail_offset.is_some() && (self.trail_points.is_some() || self.trail_price.is_some())
    }
}

/// An open entry (positions are a FIFO list of entries).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lot {
    pub id: String,
    pub long: bool,
    pub qty: f64,
    /// The size when it was entered (exits' `qty_percent` refers to it).
    #[serde(default)]
    pub orig_qty: f64,
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
    /// The entry's share of `commission`.
    #[serde(default)]
    pub entry_commission: f64,
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
    /// Largest close-to-close drawdown and run-up of equity so far
    /// (`strategy.max_drawdown` / `max_runup`).
    #[serde(default)]
    pub max_drawdown: f64,
    #[serde(default)]
    pub max_runup: f64,
    /// Margin calls so far, and the quantity each one liquidated.
    #[serde(default)]
    pub margin_calls: Vec<f64>,
    bar: usize,
    /// The close of the bar the strategy is running on: orders sized from
    /// cash or equity are sized at it when placed, as TradingView does.
    #[serde(default)]
    mark: f64,
    time: i64,
    last_close: f64,
    #[serde(default)]
    peak: Option<f64>,
    #[serde(default)]
    trough: Option<f64>,
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

    /// Places or replaces (same id and kind) an order. An entry without a
    /// quantity gets the default size now, at this bar's close (TradingView
    /// sizes it when placed, not at the fill: 100% of equity at the close
    /// can be more than the account has at the next open).
    pub fn submit(&mut self, mut o: Order) {
        if o.qty.is_none() && matches!(o.kind, OrderKind::Entry | OrderKind::Order) && self.mark > 0.0 {
            o.qty = Some(self.default_qty(self.mark));
        }
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

    /// The strategy's default order size if it filled at `price`
    /// (`strategy.default_entry_qty`), rounded down to the minimum quantity
    /// when worked out from cash or equity.
    pub fn default_qty(&self, price: f64) -> f64 {
        let v = self.props.qty_value;
        let q = match self.props.qty_type {
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
                    // The position and its entry commission together make
                    // up the share of equity (checked against TradingView
                    // for percent commission).
                    let cash = self.equity_at(price) * v / 100.0;
                    let c = self.props.commission_value;
                    match self.props.commission {
                        Commission::Percent => cash / (price * (1.0 + c / 100.0)),
                        Commission::CashPerContract => cash / (price + c),
                        Commission::CashPerOrder => (cash - c).max(0.0) / price,
                    }
                } else {
                    0.0
                }
            }
        };
        if self.props.qty_type == QtyType::Fixed {
            q
        } else {
            self.round_qty(q)
        }
    }

    /// `q` rounded down to the minimum quantity (a hair of float error
    /// doesn't cost a whole contract).
    fn round_qty(&self, q: f64) -> f64 {
        let m = self.props.min_qty;
        if m > 0.0 { (q / m + 1e-9).floor() * m } else { q }
    }

    fn margin_ratio(&self, long: bool) -> f64 {
        let m = if long {
            self.props.margin_long
        } else {
            self.props.margin_short
        };
        m.max(0.0) / 100.0
    }

    /// Equity minus the margin the open position ties up, at `price`.
    fn available_funds(&self, price: f64) -> f64 {
        let used: f64 = self
            .lots
            .iter()
            .map(|l| l.qty * price * self.margin_ratio(l.long))
            .sum();
        self.equity_at(price) - used
    }

    /// Whether the account can pay the margin for `qty` more at `price`
    /// (always, with no margin set: Pine v5's default).
    fn affordable(&self, long: bool, qty: f64, price: f64) -> bool {
        let r = self.margin_ratio(long);
        if r == 0.0 {
            return true;
        }
        let need = qty * price * r;
        let have = self.available_funds(price);
        need <= have + 1e-9 * have.abs().max(1.0)
    }

    /// TradingView's margin call: when available funds go negative at
    /// `price`, it liquidates four times the quantity that covers the
    /// shortfall (so the next bars don't call again), at that price.
    fn margin_call(&mut self, price: f64) {
        let pos = self.position_size();
        if pos == 0.0 || !price.is_finite() || price <= 0.0 {
            return;
        }
        let r = self.margin_ratio(pos > 0.0);
        if r == 0.0 {
            return;
        }
        let avail = self.available_funds(price);
        if avail >= 0.0 {
            return;
        }
        let loss = avail / r;
        let m = self.props.min_qty;
        let cover = (loss / price).abs();
        let cover = if m > 0.0 { (cover / m).trunc() * m } else { cover };
        let size = (cover * 4.0).min(pos.abs());
        if size <= 0.0 {
            return;
        }
        let closed = self.close("", size, price, "Margin call", "Margin call");
        if closed > 0.0 {
            self.margin_calls.push(closed);
        }
    }

    fn slip(&self, price: f64, buy: bool) -> f64 {
        let s = self.props.slippage as f64 * self.props.mintick;
        if buy { price + s } else { price - s }
    }

    /// Opens or adds to a position; returns the quantity filled.
    fn open(&mut self, o: &Order, qty: f64, price: f64) -> f64 {
        if qty <= 0.0 || !qty.is_finite() {
            return 0.0;
        }
        // An entry the account can't pay the margin for isn't opened.
        if !self.affordable(o.long, qty, price) {
            return 0.0;
        }
        let commission = self.commission(qty, price);
        self.commission_paid += commission;
        self.lots.push(Lot {
            id: o.id.clone(),
            long: o.long,
            qty,
            orig_qty: qty,
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
        qty
    }

    /// Closes up to `qty` of the lots matching `from` (FIFO), at `price`;
    /// returns the quantity closed.
    fn close(&mut self, from: &str, qty: f64, price: f64, exit_id: &str, comment: &str) -> f64 {
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
                entry_commission: entry_comm,
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
        sold
    }

    fn open_qty(&self, from: &str) -> f64 {
        self.lots
            .iter()
            .filter(|l| from.is_empty() || l.id == from)
            .map(|l| l.qty)
            .sum()
    }

    /// Executes an entry/order/close that has triggered at `price`;
    /// returns the quantity filled.
    fn execute(&mut self, o: &Order, price: f64) -> f64 {
        match o.kind {
            OrderKind::Close => {
                let open = self.open_qty(&o.id);
                let q = match o.qty {
                    Some(q) => q.min(open),
                    None => open * o.qty_percent / 100.0,
                };
                let long = self.position_size() > 0.0;
                let p = self.slip(price, !long);
                self.close(
                    &o.id,
                    q,
                    p,
                    &format!("Close entry(s) order {}", if o.id.is_empty() { "all" } else { &o.id }),
                    &o.comment,
                )
            }
            OrderKind::Entry => {
                let pos = self.position_size();
                let opposite = (pos > 0.0 && !o.long) || (pos < 0.0 && o.long);
                let p = self.slip(price, o.long);
                // strategy.risk.allow_entry_in: an entry the other way only
                // closes the position.
                if self.props.allow_entry_in.is_some_and(|l| l != o.long) {
                    if opposite {
                        let q = self.open_qty("");
                        return self.close("", q, p, &o.id, &o.comment);
                    }
                    return 0.0;
                }
                let mut filled = 0.0;
                if opposite {
                    // Reverse: close everything first.
                    let q = self.open_qty("");
                    filled += self.close("", q, p, &o.id, &o.comment);
                } else if pos != 0.0 {
                    // `pyramiding` is the most entries in one direction
                    // (0 and 1 both mean one).
                    if self.lots.len() as u32 >= self.props.pyramiding.max(1) {
                        return 0.0;
                    }
                }
                let qty = o.qty.unwrap_or_else(|| self.default_qty(p));
                filled + self.open(o, qty, p)
            }
            OrderKind::Order => {
                let pos = self.position_size();
                let p = self.slip(price, o.long);
                let mut qty = o.qty.unwrap_or_else(|| self.default_qty(p));
                let mut filled = 0.0;
                // Against the position, an order reduces it first.
                if (pos > 0.0 && !o.long) || (pos < 0.0 && o.long) {
                    let q = qty.min(pos.abs());
                    filled += self.close("", q, p, &o.id, &o.comment);
                    qty -= q;
                }
                if qty > 1e-12 {
                    filled += self.open(o, qty, p);
                }
                filled
            }
            OrderKind::Exit => 0.0,
        }
    }

    /// One-cancels-all: what a fill of `o` does to the other orders.
    fn oca(o: &Order, filled: f64, others: &mut Vec<Order>) {
        if o.oca_name.is_empty() || filled <= 0.0 {
            return;
        }
        match o.oca_type {
            OcaType::None => {}
            OcaType::Cancel => others.retain(|x| x.oca_name != o.oca_name),
            OcaType::Reduce => {
                for x in others.iter_mut().filter(|x| x.oca_name == o.oca_name) {
                    if let Some(q) = x.qty.as_mut() {
                        *q -= filled;
                    }
                }
                others.retain(|x| x.oca_name != o.oca_name || x.qty.is_none_or(|q| q > 1e-12));
            }
        }
    }

    /// Tries an exit order on one segment of the intrabar path; returns
    /// the quantity closed. Updates its trailing stop when nothing fills.
    fn try_exit(&mut self, o: &mut Order, a: f64, b: f64) -> f64 {
        let Some((long, qty, avg, limit, stop)) = self.exit_levels(o) else {
            return 0.0;
        };
        let t = self.props.mintick;
        let buy = !long;
        let trail_stop = o.trail_best.map(|best| {
            let off = o.trail_offset.unwrap_or(0.0) * t;
            if long { best - off } else { best + off }
        });
        let hit_l = limit.and_then(|l| Self::trigger(Some(l), None, buy, a, b));
        let hit_s = stop.and_then(|s| Self::trigger(None, Some(s), buy, a, b));
        let hit_t = trail_stop.and_then(|s| Self::trigger(None, Some(s), buy, a, b));
        // The fill nearest the segment start comes first: 0 target, 1 stop, 2 trailing.
        let pick = [(hit_l, 0), (hit_s, 1), (hit_t, 2)]
            .into_iter()
            .filter_map(|(h, k)| h.map(|p| (p, k)))
            .min_by(|x, y| (x.0 - a).abs().total_cmp(&(y.0 - a).abs()));
        if let Some((p, k)) = pick {
            let p = if k == 0 { p } else { self.slip(p, buy) };
            let comment = match k {
                0 if !o.comment_profit.is_empty() => &o.comment_profit,
                1 if !o.comment_loss.is_empty() => &o.comment_loss,
                2 if !o.comment_trailing.is_empty() => &o.comment_trailing,
                _ => &o.comment,
            }
            .clone();
            let from = o.from_entry.clone();
            return self.close(&from, qty, p, &o.id, &comment);
        }
        if o.has_trail() {
            let dir = if long { 1.0 } else { -1.0 };
            let act = o
                .trail_price
                .unwrap_or_else(|| avg + dir * o.trail_points.unwrap_or(0.0) * t);
            let best_here = if long { a.max(b) } else { a.min(b) };
            o.trail_best = match o.trail_best {
                Some(best) => Some(if long { best.max(best_here) } else { best.min(best_here) }),
                None if (long && best_here >= act) || (!long && best_here <= act) => Some(best_here),
                None => None,
            };
        }
        0.0
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

    /// An exit order's side, quantity, the covered lots' average price, and
    /// its target and stop prices.
    fn exit_levels(&self, o: &Order) -> Option<ExitLevels> {
        let lots: Vec<&Lot> = self
            .lots
            .iter()
            .filter(|l| o.from_entry.is_empty() || l.id == o.from_entry)
            .collect();
        let first = lots.first()?;
        let long = first.long;
        // qty_percent is of the size entered (two 50% exits close it all);
        // an explicit qty wins.
        let open: f64 = lots.iter().map(|l| l.qty).sum();
        let entered: f64 = lots
            .iter()
            .map(|l| if l.orig_qty > 0.0 { l.orig_qty } else { l.qty })
            .sum();
        let qty = match o.qty {
            Some(q) => q.min(open),
            None => (entered * o.qty_percent / 100.0).min(open),
        };
        let avg = lots.iter().map(|l| l.qty * l.price).sum::<f64>() / lots.iter().map(|l| l.qty).sum::<f64>();
        let t = self.props.mintick;
        let dir = if long { 1.0 } else { -1.0 };
        let profit = o.profit_ticks.map(|p| avg + dir * p * t);
        let loss = o.loss_ticks.map(|p| avg - dir * p * t);
        // Both an absolute and a relative level: v6 uses the one price
        // reaches first (the nearer one); v5 the absolute one.
        let first = |abs: Option<f64>, rel: Option<f64>, lower: bool| match (abs, rel) {
            (Some(a), Some(r)) if self.props.nearest_exit => Some(if lower { a.min(r) } else { a.max(r) }),
            (Some(a), _) => Some(a),
            (None, r) => r,
        };
        let limit = first(o.limit, profit, long);
        let stop = first(o.stop, loss, !long);
        Some((long, qty, avg, limit, stop))
    }

    /// Fills what was ordered on the previous bar against this bar.
    pub fn open_bar(&mut self, index: usize, bar: &Bar) {
        self.bar = index;
        self.time = bar.time;
        self.mark = bar.close;
        // Market orders at the open.
        let pending = std::mem::take(&mut self.pending);
        let (market, mut waiting): (Vec<Order>, Vec<Order>) = pending.into_iter().partition(|o| {
            matches!(o.kind, OrderKind::Close) || (o.kind != OrderKind::Exit && o.limit.is_none() && o.stop.is_none())
        });
        for o in &market {
            let filled = self.execute(o, bar.open);
            Self::oca(o, filled, &mut waiting);
        }
        self.margin_call(bar.open);
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
                // An exit fills (or not); an entry/order that triggers is
                // used up even if pyramiding turns it away.
                let (done, filled) = match waiting[k].kind {
                    OrderKind::Exit => {
                        let mut o = waiting[k].clone();
                        let q = self.try_exit(&mut o, a, b);
                        waiting[k] = o;
                        (q > 0.0, q)
                    }
                    _ => {
                        let o = waiting[k].clone();
                        match Self::trigger(o.limit, o.stop, o.long, a, b) {
                            Some(p) => (true, self.execute(&o, p)),
                            None => (false, 0.0),
                        }
                    }
                };
                if done {
                    let o = waiting.remove(k);
                    Self::oca(&o, filled, &mut waiting);
                } else {
                    k += 1;
                }
            }
            // Margin is checked at each point of the path, after the
            // orders that price passed on the way there.
            self.margin_call(b);
        }
        // Unfilled orders (exits included) wait for later bars.
        self.pending = waiting;
        // Track run-up and drawdown of open lots.
        for l in &mut self.lots {
            l.hi = l.hi.max(bar.high);
            l.lo = l.lo.min(bar.low);
        }
    }

    fn track_extremes(&mut self, eq: f64) {
        let (peak, trough) = match (self.peak, self.trough) {
            (Some(p), Some(t)) => (p.max(eq), t.min(eq)),
            _ => (eq.max(self.props.initial_capital), eq.min(self.props.initial_capital)),
        };
        self.max_drawdown = self.max_drawdown.max(peak - eq);
        self.max_runup = self.max_runup.max(eq - trough);
        self.peak = Some(peak);
        self.trough = Some(trough);
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
                let filled = self.execute(o, bar.close);
                let mut rest = std::mem::take(&mut self.pending);
                Self::oca(o, filled, &mut rest);
                self.pending = rest;
            }
        }
        self.last_close = bar.close;
        let eq = self.equity_at(bar.close);
        // Intrabar extremes of equity: open positions at the bar's high/low.
        let (a, b) = (self.equity_at(bar.high), self.equity_at(bar.low));
        self.track_extremes(eq);
        self.equity.push(eq);
        self.equity_hi.push(a.max(b).max(eq));
        self.equity_lo.push(a.min(b).min(eq));
        self.position.push(self.position_size());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn props_round_trip_through_json() {
        let p = Props {
            initial_capital: 5000.0,
            qty_type: QtyType::PercentOfEquity,
            qty_value: 10.0,
            pyramiding: 2,
            commission: Commission::CashPerOrder,
            commission_value: 1.5,
            slippage: 3,
            process_orders_on_close: true,
            mintick: 0.5,
            margin_long: 50.0,
            margin_short: 25.0,
            allow_entry_in: None,
            min_qty: 0.0,
            nearest_exit: true,
        };
        assert_eq!(Props::from_json(&p.to_json(), 0.5, 0.0), p);
        let base = Props::default();
        let d = p.diff_json(&base);
        assert_eq!(d.as_object().map(|o| o.len()), Some(10));
        let mut q = base.clone();
        q.slippage = 2;
        assert_eq!(q.diff_json(&base), serde_json::json!({"slippage": 2}));
        let merged = merge_json(
            &serde_json::json!({"initial_capital": 100.0, "slippage": 1}),
            &q.diff_json(&base),
        );
        assert_eq!(merged, serde_json::json!({"initial_capital": 100.0, "slippage": 2}));
    }

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
        assert_eq!(b.position_size(), 2.0, "pyramiding 2 allows two entries");
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

    #[test]
    fn trailing_stop_activates_then_trails() {
        let props = Props {
            mintick: 1.0,
            ..Props::default()
        };
        // Entry at 100; activates 5 ticks up (105), trails by 3.
        let bars = [
            bar(0, 100.0, 100.0, 100.0, 100.0),
            bar(1, 100.0, 101.0, 99.0, 100.0),
            bar(2, 101.0, 110.0, 101.0, 109.0),
            bar(3, 109.0, 112.0, 108.0, 111.0),
            bar(4, 111.0, 111.0, 104.0, 105.0),
        ];
        let b = run(&bars, props, |i, b| {
            if i == 0 {
                b.submit(Order::market("L", OrderKind::Entry, true));
                let mut x = Order::market("T", OrderKind::Exit, true);
                x.trail_points = Some(5.0);
                x.trail_offset = Some(3.0);
                x.comment_trailing = "trail".into();
                b.submit(x);
            }
        });
        assert_eq!(b.trades.len(), 1);
        // Best price 112 on bar 3 → stop 109; bar 4 falls through it.
        let t = &b.trades[0];
        assert_eq!((t.exit_price, t.exit_bar), (109.0, 4));
        assert_eq!(t.exit_comment, "trail");
    }

    #[test]
    fn two_half_exits_close_the_whole_position() {
        let bars = [
            bar(0, 100.0, 100.0, 100.0, 100.0),
            bar(1, 100.0, 100.0, 100.0, 100.0),
            bar(2, 100.0, 106.0, 100.0, 105.0),
            bar(3, 105.0, 112.0, 105.0, 111.0),
        ];
        let props = Props {
            qty_value: 10.0,
            ..Props::default()
        };
        let b = run(&bars, props, |i, b| {
            if i == 0 {
                b.submit(Order::market("L", OrderKind::Entry, true));
                for (id, lim) in [("TP1", 105.0), ("TP2", 110.0)] {
                    let mut x = Order::market(id, OrderKind::Exit, true);
                    x.from_entry = "L".into();
                    x.qty_percent = 50.0;
                    x.limit = Some(lim);
                    b.submit(x);
                }
            }
        });
        assert_eq!(b.trades.len(), 2);
        assert_eq!((b.trades[0].qty, b.trades[0].exit_price), (5.0, 105.0));
        assert_eq!((b.trades[1].qty, b.trades[1].exit_price), (5.0, 110.0));
        assert_eq!(b.position_size(), 0.0);
    }

    #[test]
    fn oca_cancel_and_allow_entry_in() {
        // Two stop entries in one OCA group: the first to trigger cancels
        // the other.
        let bars = [
            bar(0, 100.0, 100.0, 100.0, 100.0),
            bar(1, 100.0, 106.0, 99.0, 105.0),
            bar(2, 105.0, 105.0, 80.0, 81.0),
        ];
        let b = run(&bars, Props::default(), |i, b| {
            if i == 0 {
                for (id, long, stop) in [("Up", true, 105.0), ("Down", false, 90.0)] {
                    let mut o = Order::market(id, OrderKind::Entry, long);
                    o.stop = Some(stop);
                    o.oca_name = "break".into();
                    o.oca_type = OcaType::Cancel;
                    b.submit(o);
                }
            }
        });
        assert_eq!(b.position_size(), 1.0);
        assert!(b.pending.is_empty());
        // Longs only: a short entry just closes the long.
        let props = Props {
            allow_entry_in: Some(true),
            ..Props::default()
        };
        let bars: Vec<Bar> = (0..4).map(|i| bar(i, 10.0, 10.0, 10.0, 10.0)).collect();
        let b = run(&bars, props, |i, b| match i {
            0 => b.submit(Order::market("L", OrderKind::Entry, true)),
            1 => b.submit(Order::market("S", OrderKind::Entry, false)),
            _ => {}
        });
        assert_eq!(b.position_size(), 0.0);
        assert_eq!(b.trades.len(), 1);
    }

    #[test]
    fn tradingviews_margin_call_example() {
        // The Pine manual's example: TSLA daily, 300% of equity, 25% margin.
        // The order is sized at the signal bar's close (4.396), rounded down
        // to whole shares, and fills at the next open (4.43).
        let props = Props {
            qty_type: QtyType::PercentOfEquity,
            qty_value: 300.0,
            margin_long: 25.0,
            min_qty: 1.0,
            mintick: 0.001,
            ..Props::default()
        };
        let bars = [
            bar(0, 4.3, 4.4, 4.3, 4.396),
            bar(1, 4.43, 4.5, 4.40, 4.45),
            bar(2, 4.2, 4.25, 3.9, 4.0),
        ];
        let b = run(&bars, props, |i, b| {
            if i == 0 {
                b.submit(Order::market("Long", OrderKind::Entry, true));
            }
        });
        assert_eq!(b.fills[0].qty, 682_438.0);
        assert_eq!(b.fills[0].price, 4.43);
        // At 3.9: available funds −27,069.19 → cover 27,763 shares, × 4.
        assert_eq!(b.margin_calls, vec![111_052.0]);
        let t = &b.trades[0];
        assert_eq!((t.qty, t.exit_price, t.exit_bar), (111_052.0, 3.9, 2));
        assert_eq!(t.exit_id, "Margin call");
        assert_eq!(b.position_size(), 682_438.0 - 111_052.0);
    }

    #[test]
    fn an_entry_the_account_cant_pay_for_isnt_opened() {
        // 100% of equity sized at a close of 10 is 100 shares; the next bar
        // opens at 10.5, so they'd cost 1,050 of a 1,000 account.
        let bars = [
            bar(0, 10.0, 10.0, 10.0, 10.0),
            bar(1, 10.5, 10.5, 10.5, 10.5),
            bar(2, 10.5, 10.5, 10.5, 10.5),
        ];
        let props = Props {
            initial_capital: 1000.0,
            qty_type: QtyType::PercentOfEquity,
            qty_value: 100.0,
            min_qty: 1.0,
            ..Props::default()
        };
        let entry = |i: usize, b: &mut Broker| {
            if i == 0 {
                b.submit(Order::market("L", OrderKind::Entry, true));
            }
        };
        let b = run(&bars, props.clone(), entry);
        assert_eq!(b.position_size(), 0.0);
        assert!(b.pending.is_empty(), "the order is used up");
        // Pine v5's default margin (0) never checks.
        let v5 = Props {
            margin_long: 0.0,
            margin_short: 0.0,
            ..props
        };
        let b = run(&bars, v5, entry);
        assert_eq!(b.position_size(), 100.0);
    }

    #[test]
    fn a_losing_short_at_full_size_is_margin_called() {
        // Short 100 at 10 with 1,000 and 100% margin: at 11 equity is 900
        // and the margin 1,100, so funds are −200 → cover 18 → 72 bought back.
        let bars = [
            bar(0, 10.0, 10.0, 10.0, 10.0),
            bar(1, 10.0, 10.0, 10.0, 10.0),
            bar(2, 10.0, 11.0, 10.0, 11.0),
        ];
        let props = Props {
            initial_capital: 1000.0,
            qty_type: QtyType::PercentOfEquity,
            qty_value: 100.0,
            min_qty: 1.0,
            ..Props::default()
        };
        let b = run(&bars, props, |i, b| {
            if i == 0 {
                b.submit(Order::market("S", OrderKind::Entry, false));
            }
        });
        assert_eq!(b.margin_calls, vec![72.0]);
        assert_eq!(b.position_size(), -28.0);
    }

    #[test]
    fn v6_exit_uses_the_level_reached_first() {
        let bars = [
            bar(0, 100.0, 100.0, 100.0, 100.0),
            bar(1, 100.0, 100.0, 100.0, 100.0),
            bar(2, 100.0, 112.0, 100.0, 111.0),
        ];
        let exit = |i: usize, b: &mut Broker| {
            if i == 0 {
                b.submit(Order::market("L", OrderKind::Entry, true));
                let mut x = Order::market("X", OrderKind::Exit, true);
                x.limit = Some(110.0);
                x.profit_ticks = Some(5.0);
                b.submit(x);
            }
        };
        let v6 = Props {
            mintick: 1.0,
            ..Props::default()
        };
        assert_eq!(run(&bars, v6.clone(), exit).trades[0].exit_price, 105.0);
        let v5 = Props {
            nearest_exit: false,
            ..v6
        };
        assert_eq!(run(&bars, v5, exit).trades[0].exit_price, 110.0);
    }

    #[test]
    fn version_picks_the_default_margin() {
        let v5 = Props::from_json(&serde_json::json!({"pine_version": 5}), 0.01, 1.0);
        assert_eq!((v5.margin_long, v5.margin_short, v5.nearest_exit), (0.0, 0.0, false));
        let v6 = Props::from_json(&serde_json::json!({"pine_version": 6}), 0.01, 1.0);
        assert_eq!((v6.margin_long, v6.margin_short, v6.nearest_exit), (100.0, 100.0, true));
        let set = Props::from_json(&serde_json::json!({"pine_version": 5, "margin_long": 50}), 0.01, 1.0);
        assert_eq!((set.margin_long, set.margin_short), (50.0, 0.0));
    }

    #[test]
    fn percent_of_equity_leaves_room_for_the_commission() {
        // 100% of 10,000 at a close of 100 with 0.1% commission: the
        // position plus its commission is the equity, rounded down to the
        // lot step (BTCUSDT's 0.00001, as TradingView does).
        let props = Props {
            initial_capital: 10_000.0,
            qty_type: QtyType::PercentOfEquity,
            qty_value: 100.0,
            commission_value: 0.1,
            min_qty: 0.00001,
            process_orders_on_close: true,
            ..Props::default()
        };
        let bars = [bar(0, 100.0, 100.0, 100.0, 100.0)];
        let b = run(&bars, props, |_, b| {
            b.submit(Order::market("L", OrderKind::Entry, true))
        });
        assert_eq!(b.lots[0].qty, 99.90009);
    }
}

//! Paper trading: a simulated account driven by live prices instead of
//! bars. Market orders fill at the last price; limit, stop, stop-limit and
//! trailing orders fill when the price reaches them; an order can carry a
//! take-profit and a stop-loss, which become a one-cancels-the-other pair
//! once it fills. Positions are per symbol and net (buying against a short
//! covers it first).
//!
//! Costs and limits: commission (% of each fill's value), slippage (market,
//! stop and trailing fills move against the order by a % of the price;
//! limits fill at their price), and leverage: an order that would need
//! more margin (exposure ÷ leverage) than the account's equity is refused,
//! and when equity falls below half the margin in use every position is
//! closed at market (a margin call). An account can keep its money in one
//! currency: each symbol's profit and loss is converted at the rate the
//! app feeds in (`Account::fx`) when it's realised.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Equity below this share of the margin in use calls the margin.
pub const MAINTENANCE: f64 = 0.5;

/// Whether two currency codes are the same money (stablecoins count as
/// the dollar).
pub fn same_money(a: &str, b: &str) -> bool {
    let norm = |c: &str| match c {
        "USDT" | "USDC" | "BUSD" | "FDUSD" | "TUSD" | "DAI" => "USD".to_string(),
        c => c.to_string(),
    };
    norm(&a.to_uppercase()) == norm(&b.to_uppercase())
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    Market,
    Limit(f64),
    Stop(f64),
    /// Stop price arms a limit order at the limit price.
    StopLimit {
        stop: f64,
        limit: f64,
        armed: bool,
    },
    /// A stop that follows the best price by `offset` (in price units).
    Trailing {
        offset: f64,
        best: f64,
    },
}

impl Kind {
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Market => "Market",
            Kind::Limit(_) => "Limit",
            Kind::Stop(_) => "Stop",
            Kind::StopLimit { .. } => "Stop limit",
            Kind::Trailing { .. } => "Trailing stop",
        }
    }

    /// The level shown for the order.
    pub fn price(&self, buy: bool) -> Option<f64> {
        match *self {
            Kind::Market => None,
            Kind::Limit(p) | Kind::Stop(p) => Some(p),
            Kind::StopLimit { stop, limit, armed } => Some(if armed { limit } else { stop }),
            Kind::Trailing { offset, best } => Some(if buy { best + offset } else { best - offset }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaperOrder {
    pub id: u64,
    pub symbol: String,
    pub buy: bool,
    pub qty: f64,
    pub kind: Kind,
    /// Brackets placed when this order fills.
    pub take_profit: Option<f64>,
    pub stop_loss: Option<f64>,
    /// Orders sharing a group cancel each other when one fills.
    pub oco: Option<u64>,
    /// Only reduces the position (bracket legs).
    pub reduce_only: bool,
    pub placed: i64,
    /// The symbol's currency ("" = unknown: the account's own).
    #[serde(default)]
    pub currency: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    /// + long, − short.
    pub qty: f64,
    pub avg: f64,
    pub opened: i64,
    /// The symbol's currency ("" = the account's own).
    #[serde(default)]
    pub currency: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaperFill {
    pub order: u64,
    pub symbol: String,
    pub buy: bool,
    pub qty: f64,
    pub price: f64,
    pub time: i64,
    /// Profit realised by this fill (after commission), in the account's
    /// currency.
    pub realized: f64,
    pub commission: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    pub name: String,
    pub initial: f64,
    /// Cash: initial capital plus realised P&L.
    pub balance: f64,
    /// Commission per fill, percent of the fill's value.
    pub commission_pct: f64,
    /// How far market, stop and trailing fills move against the order, as
    /// a percent of the price.
    pub slippage_pct: f64,
    /// Exposure allowed per unit of equity (1 = cash only).
    pub leverage: f64,
    /// The money the account is kept in ("" = each symbol's own, no
    /// conversion).
    pub currency: String,
    pub positions: Vec<Position>,
    pub orders: Vec<PaperOrder>,
    pub fills: Vec<PaperFill>,
    next_id: u64,
    /// The last price seen per symbol (margin is valued at it).
    #[serde(skip)]
    marks: HashMap<String, f64>,
    /// How much of the account's currency one unit of a currency is worth,
    /// fed by the app from live exchange rates.
    #[serde(skip)]
    pub fx: HashMap<String, f64>,
    /// Why orders were refused or positions closed since the app last
    /// looked (it shows and clears them).
    #[serde(skip)]
    pub notices: Vec<String>,
}

impl Default for Account {
    fn default() -> Self {
        Account::new("Paper account", 100_000.0)
    }
}

/// What a new order asks for.
#[derive(Clone, Debug, PartialEq)]
pub struct Ticket {
    pub symbol: String,
    /// The symbol's currency ("" = unknown: the account's own).
    pub currency: String,
    pub buy: bool,
    pub qty: f64,
    pub kind: Kind,
    pub take_profit: Option<f64>,
    pub stop_loss: Option<f64>,
}

impl Account {
    pub fn new(name: &str, initial: f64) -> Account {
        Account {
            name: name.to_string(),
            initial,
            balance: initial,
            commission_pct: 0.0,
            slippage_pct: 0.0,
            leverage: 1.0,
            currency: String::new(),
            positions: Vec::new(),
            orders: Vec::new(),
            fills: Vec::new(),
            next_id: 1,
            marks: HashMap::new(),
            fx: HashMap::new(),
            notices: Vec::new(),
        }
    }

    /// The account-currency value of one unit of `ccy`; `None` while its
    /// rate is unknown.
    pub fn rate(&self, ccy: &str) -> Option<f64> {
        if self.currency.is_empty() || ccy.is_empty() || same_money(ccy, &self.currency) {
            return Some(1.0);
        }
        self.fx
            .get(&ccy.to_uppercase())
            .copied()
            .filter(|r| *r > 0.0 && r.is_finite())
    }

    /// The currencies the account needs rates for.
    pub fn currencies(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .positions
            .iter()
            .map(|p| p.currency.clone())
            .chain(self.orders.iter().map(|o| o.currency.clone()))
            .filter(|c| !c.is_empty() && !self.currency.is_empty() && !same_money(c, &self.currency))
            .map(|c| c.to_uppercase())
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// The margin the open positions take, in the account's currency,
    /// valued at their last prices (their average where none is known).
    pub fn margin_used(&self) -> f64 {
        let lev = self.leverage.max(1.0);
        self.positions
            .iter()
            .map(|p| {
                let mark = self.marks.get(&p.symbol).copied().unwrap_or(p.avg);
                p.qty.abs() * mark * self.rate(&p.currency).unwrap_or(1.0) / lev
            })
            .sum()
    }

    /// Equity at the last prices seen.
    pub fn equity_at_marks(&self) -> f64 {
        let marks = &self.marks;
        self.equity(|s| marks.get(s).copied())
    }

    /// What's left for new positions: equity less the margin in use.
    pub fn buying_power(&self) -> f64 {
        (self.equity_at_marks() - self.margin_used()) * self.leverage.max(1.0)
    }

    /// Clears positions, orders and history and puts the cash back to the
    /// starting capital; the account's settings stay.
    pub fn reset(&mut self) {
        let mut fresh = Account::new(&self.name, self.initial);
        fresh.commission_pct = self.commission_pct;
        fresh.slippage_pct = self.slippage_pct;
        fresh.leverage = self.leverage;
        fresh.currency = self.currency.clone();
        fresh.fx = std::mem::take(&mut self.fx);
        *self = fresh;
    }

    /// Changes the starting capital, keeping the profit and loss made so
    /// far: the cash moves by the same amount.
    pub fn set_initial(&mut self, initial: f64) {
        self.balance += initial - self.initial;
        self.initial = initial;
    }

    pub fn position(&self, symbol: &str) -> Option<&Position> {
        self.positions.iter().find(|p| p.symbol == symbol)
    }

    pub fn unrealized(&self, price: impl Fn(&str) -> Option<f64>) -> f64 {
        self.positions
            .iter()
            .filter_map(|p| {
                let r = self.rate(&p.currency)?;
                price(&p.symbol).map(|x| (x - p.avg) * p.qty * r)
            })
            .sum()
    }

    pub fn equity(&self, price: impl Fn(&str) -> Option<f64>) -> f64 {
        self.balance + self.unrealized(price)
    }

    /// Symbols that need live prices.
    pub fn symbols(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .positions
            .iter()
            .map(|p| p.symbol.clone())
            .chain(self.orders.iter().map(|o| o.symbol.clone()))
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// Places an order; a market order fills now at `last`. Returns its id.
    pub fn place(&mut self, t: Ticket, last: Option<f64>, now: i64) -> Result<u64, String> {
        if !(t.qty > 0.0 && t.qty.is_finite()) {
            return Err("Enter a quantity above zero.".into());
        }
        let check = |p: f64| p > 0.0 && p.is_finite();
        match t.kind {
            Kind::Limit(p) | Kind::Stop(p) if !check(p) => return Err("Enter a price.".into()),
            Kind::StopLimit { stop, limit, .. } if !check(stop) || !check(limit) => {
                return Err("Enter both the stop and the limit price.".into());
            }
            Kind::Trailing { offset, .. } if !check(offset) => {
                return Err("Enter how far the stop trails the price.".into());
            }
            _ => {}
        }
        // Brackets must sit on the right side of the entry.
        let reference = t.kind.price(t.buy).or(last);
        if let Some(r) = reference {
            let dir = if t.buy { 1.0 } else { -1.0 };
            if t.take_profit.is_some_and(|tp| (tp - r) * dir <= 0.0) {
                return Err(format!(
                    "The take-profit has to be {} the entry price.",
                    if t.buy { "above" } else { "below" }
                ));
            }
            if t.stop_loss.is_some_and(|sl| (sl - r) * dir >= 0.0) {
                return Err(format!(
                    "The stop-loss has to be {} the entry price.",
                    if t.buy { "below" } else { "above" }
                ));
            }
        }
        if self.rate(&t.currency).is_none() {
            return Err(format!(
                "No {}/{} exchange rate yet; try again in a moment.",
                t.currency, self.currency
            ));
        }
        if let Some(l) = last.filter(|l| *l > 0.0 && l.is_finite()) {
            self.marks.insert(t.symbol.clone(), l);
        }
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        let mut kind = t.kind;
        if let Kind::Trailing { offset, .. } = kind {
            let Some(l) = last else {
                return Err("No price yet for this symbol.".into());
            };
            kind = Kind::Trailing { offset, best: l };
        }
        let o = PaperOrder {
            id,
            symbol: t.symbol,
            buy: t.buy,
            qty: t.qty,
            kind,
            take_profit: t.take_profit,
            stop_loss: t.stop_loss,
            oco: None,
            reduce_only: false,
            placed: now,
            currency: t.currency,
        };
        if o.kind == Kind::Market {
            let Some(l) = last else {
                return Err("No price yet for this symbol.".into());
            };
            if let Err(e) = self.affordable(&o, l) {
                self.next_id = id;
                return Err(e);
            }
            self.fill(o, l, now);
        } else {
            self.orders.push(o);
        }
        Ok(id)
    }

    pub fn cancel(&mut self, id: u64) -> bool {
        let n = self.orders.len();
        self.orders.retain(|o| o.id != id);
        self.orders.len() != n
    }

    /// Moves a working order to a new price (dragging its line on the
    /// chart). A stop-limit moves its stop until armed, then its limit.
    pub fn modify(&mut self, id: u64, price: f64) -> Result<(), String> {
        if !(price > 0.0 && price.is_finite()) {
            return Err("That price isn't valid.".into());
        }
        let o = self
            .orders
            .iter_mut()
            .find(|o| o.id == id)
            .ok_or("That order isn't working any more.")?;
        match &mut o.kind {
            Kind::Limit(p) | Kind::Stop(p) => *p = price,
            Kind::StopLimit { stop, limit, armed } => {
                if *armed {
                    *limit = price;
                } else {
                    *stop = price;
                }
            }
            Kind::Trailing { .. } => return Err("A trailing stop follows the price; it can't be moved.".into()),
            Kind::Market => return Err("A market order has no price to move.".into()),
        }
        Ok(())
    }

    /// Closes a position at market (and cancels its bracket legs).
    pub fn close_position(&mut self, symbol: &str, last: f64, now: i64) {
        let Some(p) = self.position(symbol).cloned() else {
            return;
        };
        self.orders.retain(|o| !(o.symbol == symbol && o.reduce_only));
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        let o = PaperOrder {
            id,
            symbol: symbol.to_string(),
            buy: p.qty < 0.0,
            qty: p.qty.abs(),
            kind: Kind::Market,
            take_profit: None,
            stop_loss: None,
            oco: None,
            reduce_only: true,
            placed: now,
            currency: p.currency.clone(),
        };
        self.marks.insert(symbol.to_string(), last);
        self.fill(o, last, now);
    }

    /// Whether filling `o` at `price` stays within the margin the equity
    /// allows (orders that only shrink a position always do).
    fn affordable(&self, o: &PaperOrder, price: f64) -> Result<(), String> {
        let pos = self.position(&o.symbol).map(|p| p.qty).unwrap_or(0.0);
        let signed = if o.buy { o.qty } else { -o.qty };
        let after = pos + signed;
        if o.reduce_only || after.abs() <= pos.abs() + 1e-12 {
            return Ok(());
        }
        let Some(rate) = self.rate(&o.currency) else {
            return Err(format!("No {}/{} exchange rate yet.", o.currency, self.currency));
        };
        let lev = self.leverage.max(1.0);
        // The margin this symbol would take, in place of what it takes now.
        let mark = self.marks.get(&o.symbol).copied().unwrap_or(price);
        let now_margin = pos.abs() * mark * rate / lev;
        let new_margin = after.abs() * price * rate / lev;
        let cost = o.qty * price * rate * (self.commission_pct + self.slippage_pct) / 100.0;
        let need = self.margin_used() - now_margin + new_margin;
        if need + cost > self.equity_at_marks() + 1e-9 {
            return Err("Not enough buying power for this order.".into());
        }
        Ok(())
    }

    /// Closes every position at its last price when equity is below
    /// `MAINTENANCE` of the margin in use.
    fn margin_call(&mut self, now: i64) -> Vec<PaperFill> {
        let used = self.margin_used();
        if self.positions.is_empty() || used <= 0.0 || self.equity_at_marks() >= used * MAINTENANCE {
            return Vec::new();
        }
        let mut out = Vec::new();
        let held: Vec<(String, f64)> = self
            .positions
            .iter()
            .map(|p| (p.symbol.clone(), self.marks.get(&p.symbol).copied().unwrap_or(p.avg)))
            .collect();
        for (sym, px) in held {
            let before = self.fills.len();
            self.close_position(&sym, px, now);
            out.extend(self.fills[before..].iter().cloned());
        }
        self.notices
            .push("Margin call: equity fell below half the margin in use, so every position was closed.".into());
        out
    }

    /// A new price for `symbol`: triggers what it reaches. Returns the
    /// fills it caused.
    pub fn on_price(&mut self, symbol: &str, price: f64, now: i64) -> Vec<PaperFill> {
        if !(price > 0.0 && price.is_finite()) {
            return Vec::new();
        }
        self.marks.insert(symbol.to_string(), price);
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.orders.len() {
            if self.orders[i].symbol != symbol {
                i += 1;
                continue;
            }
            let o = &mut self.orders[i];
            let buy = o.buy;
            let fill_at = match &mut o.kind {
                Kind::Market => Some(price),
                Kind::Limit(l) => ((buy && price <= *l) || (!buy && price >= *l)).then_some(price),
                Kind::Stop(s) => ((buy && price >= *s) || (!buy && price <= *s)).then_some(price),
                Kind::StopLimit { stop, limit, armed } => {
                    if !*armed && ((buy && price >= *stop) || (!buy && price <= *stop)) {
                        *armed = true;
                    }
                    (*armed && ((buy && price <= *limit) || (!buy && price >= *limit))).then_some(price)
                }
                Kind::Trailing { offset, best } => {
                    // A sell stop trails below the high, a buy stop above the low.
                    if buy {
                        *best = best.min(price);
                        (price >= *best + *offset).then_some(price)
                    } else {
                        *best = best.max(price);
                        (price <= *best - *offset).then_some(price)
                    }
                }
            };
            match fill_at {
                Some(p) => {
                    let o = self.orders.remove(i);
                    match self.affordable(&o, p) {
                        Ok(()) => {
                            if let Some(f) = self.fill(o, p, now) {
                                out.push(f);
                            }
                        }
                        Err(e) => self.notices.push(format!(
                            "{} order {} on {} cancelled: {e}",
                            o.kind.name(),
                            o.id,
                            o.symbol
                        )),
                    }
                    i = 0;
                }
                None => i += 1,
            }
        }
        out.extend(self.margin_call(now));
        out
    }

    /// Fills an order: nets it against the position, books realised P&L
    /// and commission, places brackets and cancels its OCO partner.
    fn fill(&mut self, o: PaperOrder, price: f64, now: i64) -> Option<PaperFill> {
        if let Some(g) = o.oco {
            self.orders.retain(|x| x.oco != Some(g));
        }
        let pos_qty = self.position(&o.symbol).map(|p| p.qty).unwrap_or(0.0);
        let signed = if o.buy { o.qty } else { -o.qty };
        let mut qty = o.qty;
        if o.reduce_only {
            // Never flips or grows the position.
            if pos_qty == 0.0 || pos_qty.signum() == signed.signum() {
                return None;
            }
            qty = qty.min(pos_qty.abs());
        }
        let signed = if o.buy { qty } else { -qty };
        // Market, stop and trailing fills slip against the order.
        let slips = matches!(o.kind, Kind::Market | Kind::Stop(_) | Kind::Trailing { .. });
        let price = if slips && self.slippage_pct > 0.0 {
            let s = self.slippage_pct / 100.0;
            if o.buy { price * (1.0 + s) } else { price * (1.0 - s) }
        } else {
            price
        };
        let rate = self.rate(&o.currency).unwrap_or(1.0);
        let commission = qty * price * rate * self.commission_pct / 100.0;
        let mut realized = -commission;
        match self.positions.iter().position(|p| p.symbol == o.symbol) {
            None => self.positions.push(Position {
                symbol: o.symbol.clone(),
                qty: signed,
                avg: price,
                opened: now,
                currency: o.currency.clone(),
            }),
            Some(k) => {
                let p = &mut self.positions[k];
                if p.qty.signum() == signed.signum() {
                    p.avg = (p.avg * p.qty.abs() + price * qty) / (p.qty.abs() + qty);
                    p.qty += signed;
                } else {
                    let closed = qty.min(p.qty.abs());
                    realized += (price - p.avg) * closed * p.qty.signum() * rate;
                    let rest = p.qty + signed;
                    if rest.abs() < 1e-12 {
                        self.positions.remove(k);
                    } else if rest.signum() == p.qty.signum() {
                        p.qty = rest;
                    } else {
                        // Flipped: the remainder opens at this price.
                        p.qty = rest;
                        p.avg = price;
                        p.opened = now;
                    }
                }
            }
        }
        self.balance += realized;
        // Bracket legs of a closed position go with it.
        if self.position(&o.symbol).is_none() {
            self.orders.retain(|x| !(x.symbol == o.symbol && x.reduce_only));
        }
        if o.take_profit.is_some() || o.stop_loss.is_some() {
            let group = self.next_id.max(1);
            self.next_id = group + 1;
            let legs = [o.take_profit.map(Kind::Limit), o.stop_loss.map(Kind::Stop)];
            for kind in legs.into_iter().flatten() {
                let id = self.next_id;
                self.next_id += 1;
                self.orders.push(PaperOrder {
                    id,
                    symbol: o.symbol.clone(),
                    buy: !o.buy,
                    qty,
                    kind,
                    take_profit: None,
                    stop_loss: None,
                    oco: Some(group),
                    reduce_only: true,
                    placed: now,
                    currency: o.currency.clone(),
                });
            }
        }
        let f = PaperFill {
            order: o.id,
            symbol: o.symbol,
            buy: o.buy,
            qty,
            price,
            time: now,
            realized,
            commission,
        };
        self.fills.push(f.clone());
        Some(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(buy: bool, qty: f64, kind: Kind) -> Ticket {
        Ticket {
            symbol: "X".into(),
            currency: String::new(),
            buy,
            qty,
            kind,
            take_profit: None,
            stop_loss: None,
        }
    }

    #[test]
    fn orders_move_to_a_new_price() {
        let mut a = Account::new("t", 10_000.0);
        let id = a.place(ticket(true, 1.0, Kind::Limit(90.0)), Some(100.0), 0).unwrap();
        a.modify(id, 95.0).unwrap();
        assert!(a.on_price("X", 96.0, 1).is_empty());
        assert_eq!(a.on_price("X", 95.0, 2).len(), 1);
        let t = a
            .place(
                ticket(
                    false,
                    1.0,
                    Kind::Trailing {
                        offset: 5.0,
                        best: 95.0,
                    },
                ),
                Some(95.0),
                3,
            )
            .unwrap();
        assert!(a.modify(t, 80.0).is_err());
        assert!(a.modify(999, 80.0).is_err());
    }

    #[test]
    fn market_orders_net_and_realise() {
        let mut a = Account::new("t", 1000.0);
        a.place(ticket(true, 2.0, Kind::Market), Some(10.0), 0).unwrap();
        a.place(ticket(true, 2.0, Kind::Market), Some(12.0), 1).unwrap();
        assert_eq!(a.position("X").unwrap().avg, 11.0);
        // Sell 6: closes 4 at 15 (+16), opens a 2 short.
        a.place(ticket(false, 6.0, Kind::Market), Some(15.0), 2).unwrap();
        assert_eq!(a.balance, 1016.0);
        let p = a.position("X").unwrap();
        assert_eq!((p.qty, p.avg), (-2.0, 15.0));
        assert_eq!(a.equity(|_| Some(14.0)), 1018.0);
    }

    #[test]
    fn limits_stops_and_stop_limits() {
        let mut a = Account::new("t", 1000.0);
        a.place(ticket(true, 1.0, Kind::Limit(9.0)), Some(10.0), 0).unwrap();
        a.place(ticket(true, 1.0, Kind::Stop(11.0)), Some(10.0), 0).unwrap();
        a.place(
            ticket(
                true,
                1.0,
                Kind::StopLimit {
                    stop: 12.0,
                    limit: 11.5,
                    armed: false,
                },
            ),
            Some(10.0),
            0,
        )
        .unwrap();
        assert!(a.on_price("X", 10.0, 1).is_empty());
        assert_eq!(a.on_price("X", 8.9, 2).len(), 1);
        assert_eq!(a.on_price("X", 11.2, 3).len(), 1);
        // Stop-limit arms at 12, then fills back at 11.5 or below.
        assert!(a.on_price("X", 12.1, 4).is_empty());
        assert_eq!(a.on_price("X", 11.4, 5).len(), 1);
        assert_eq!(a.position("X").unwrap().qty, 3.0);
    }

    #[test]
    fn brackets_are_one_cancels_the_other() {
        let mut a = Account::new("t", 1000.0);
        let mut t = ticket(true, 1.0, Kind::Market);
        t.take_profit = Some(12.0);
        t.stop_loss = Some(9.0);
        a.place(t, Some(10.0), 0).unwrap();
        assert_eq!(a.orders.len(), 2);
        let f = a.on_price("X", 12.5, 1);
        assert_eq!(f.len(), 1);
        assert!(a.orders.is_empty(), "the stop went with the target");
        assert!(a.position("X").is_none());
        assert_eq!(a.balance, 1002.5);
        // Wrong-side brackets are refused with a plain message.
        let mut bad = ticket(true, 1.0, Kind::Market);
        bad.stop_loss = Some(11.0);
        assert!(a.place(bad, Some(10.0), 2).unwrap_err().contains("below"));
    }

    #[test]
    fn trailing_stop_follows_the_high() {
        let mut a = Account::new("t", 1000.0);
        a.place(ticket(true, 1.0, Kind::Market), Some(100.0), 0).unwrap();
        a.place(
            ticket(false, 1.0, Kind::Trailing { offset: 5.0, best: 0.0 }),
            Some(100.0),
            0,
        )
        .unwrap();
        for p in [104.0, 110.0, 107.0] {
            assert!(a.on_price("X", p, 1).is_empty());
        }
        assert_eq!(a.on_price("X", 104.9, 2).len(), 1);
        assert!(a.position("X").is_none());
        assert!((a.balance - 1004.9).abs() < 1e-9);
    }

    #[test]
    fn close_position_and_commission() {
        let mut a = Account::new("t", 1000.0);
        a.commission_pct = 0.1;
        let mut t = ticket(false, 10.0, Kind::Market);
        t.stop_loss = Some(12.0);
        a.place(t, Some(10.0), 0).unwrap();
        a.close_position("X", 9.0, 1);
        assert!(a.orders.is_empty());
        // +10 on the short, minus 0.1 + 0.09 commission.
        assert!((a.balance - (1000.0 + 10.0 - 0.1 - 0.09)).abs() < 1e-9);
        let json = serde_json::to_string(&a).unwrap();
        let back: Account = serde_json::from_str(&json).unwrap();
        // Prices seen and rates are runtime only; the rest round-trips.
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    #[test]
    fn slippage_moves_market_and_stop_fills_but_not_limits() {
        let mut a = Account::new("t", 10_000.0);
        a.slippage_pct = 1.0;
        a.place(ticket(true, 1.0, Kind::Market), Some(100.0), 0).unwrap();
        assert!((a.position("X").unwrap().avg - 101.0).abs() < 1e-9);
        a.place(ticket(false, 1.0, Kind::Stop(95.0)), Some(100.0), 1).unwrap();
        a.on_price("X", 95.0, 2);
        assert!((a.fills[1].price - 94.05).abs() < 1e-9);
        a.place(ticket(true, 1.0, Kind::Limit(90.0)), Some(95.0), 3).unwrap();
        a.on_price("X", 90.0, 4);
        assert_eq!(a.fills[2].price, 90.0);
    }

    #[test]
    fn orders_beyond_buying_power_are_refused() {
        let mut a = Account::new("t", 1000.0);
        assert!(a.place(ticket(true, 11.0, Kind::Market), Some(100.0), 0).is_err());
        assert!(a.positions.is_empty() && a.fills.is_empty());
        a.place(ticket(true, 9.0, Kind::Market), Some(100.0), 0).unwrap();
        // Selling down is always allowed; a short beyond it isn't.
        assert!(a.place(ticket(false, 9.0, Kind::Market), Some(100.0), 1).is_ok());
        assert!(a.place(ticket(false, 11.0, Kind::Market), Some(100.0), 2).is_err());
        // A working order too big when it triggers is cancelled with a note.
        a.place(ticket(true, 20.0, Kind::Limit(90.0)), Some(100.0), 3).unwrap();
        assert!(a.on_price("X", 90.0, 4).is_empty());
        assert!(a.orders.is_empty());
        assert_eq!(a.notices.len(), 1, "{:?}", a.notices);
        // Leverage lets the same equity hold more.
        a.leverage = 4.0;
        assert!(a.place(ticket(true, 30.0, Kind::Market), Some(100.0), 5).is_ok());
        assert!((a.margin_used() - 750.0).abs() < 1e-9);
    }

    #[test]
    fn a_margin_call_closes_everything() {
        let mut a = Account::new("t", 1000.0);
        a.leverage = 10.0;
        a.place(ticket(true, 90.0, Kind::Market), Some(100.0), 0).unwrap();
        // Down 5%: equity 550, margin 855 → still above half of it.
        a.on_price("X", 95.0, 1);
        assert!(a.position("X").is_some());
        // Down 8%: equity 280 < half of 828.
        let fills = a.on_price("X", 92.0, 2);
        assert_eq!(fills.len(), 1);
        assert!(a.positions.is_empty());
        assert!((a.balance - 280.0).abs() < 1e-9);
        assert!(a.notices.iter().any(|n| n.starts_with("Margin call")));
    }

    #[test]
    fn profit_in_another_currency_is_converted() {
        let mut a = Account::new("t", 1000.0);
        a.currency = "USD".into();
        a.leverage = 100.0;
        let mut t = ticket(true, 10.0, Kind::Market);
        t.currency = "INR".into();
        // No rate yet: refused.
        assert!(a.place(t.clone(), Some(2000.0), 0).is_err());
        a.fx.insert("INR".into(), 0.012);
        a.place(t, Some(2000.0), 0).unwrap();
        // ₹100 a share on 10 shares = ₹1000 = $12.
        assert!((a.unrealized(|_| Some(2100.0)) - 12.0).abs() < 1e-9);
        a.close_position("X", 2100.0, 1);
        assert!((a.balance - 1012.0).abs() < 1e-9);
        // Stablecoins are dollars.
        assert_eq!(a.rate("USDT"), Some(1.0));
    }

    #[test]
    fn older_accounts_load_as_cash_accounts_in_their_symbols_money() {
        let old = r#"{"name":"p","initial":5000.0,"balance":5000.0,"commission_pct":0.0,"positions":[],"orders":[],"fills":[],"next_id":1}"#;
        let a: Account = serde_json::from_str(old).unwrap();
        assert_eq!((a.leverage, a.slippage_pct, a.currency.as_str()), (1.0, 0.0, ""));
    }

    #[test]
    fn new_starting_capital_keeps_the_profit_made() {
        let mut a = Account::new("t", 1000.0);
        a.place(ticket(true, 10.0, Kind::Market), Some(10.0), 0).unwrap();
        a.close_position("X", 12.0, 1);
        assert!((a.balance - 1020.0).abs() < 1e-9);
        a.set_initial(5000.0);
        assert!((a.balance - 5020.0).abs() < 1e-9);
        assert!((a.balance - a.initial - 20.0).abs() < 1e-9);
    }
}

//! Paper trading: a simulated account driven by live prices instead of
//! bars. Market orders fill at the last price; limit, stop, stop-limit and
//! trailing orders fill when the price reaches them; an order can carry a
//! take-profit and a stop-loss, which become a one-cancels-the-other pair
//! once it fills. Positions are per symbol and net (buying against a short
//! covers it first).

use serde::{Deserialize, Serialize};

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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    /// + long, − short.
    pub qty: f64,
    pub avg: f64,
    pub opened: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaperFill {
    pub order: u64,
    pub symbol: String,
    pub buy: bool,
    pub qty: f64,
    pub price: f64,
    pub time: i64,
    /// Profit realised by this fill (after commission).
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
    pub positions: Vec<Position>,
    pub orders: Vec<PaperOrder>,
    pub fills: Vec<PaperFill>,
    next_id: u64,
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
            positions: Vec::new(),
            orders: Vec::new(),
            fills: Vec::new(),
            next_id: 1,
        }
    }

    pub fn position(&self, symbol: &str) -> Option<&Position> {
        self.positions.iter().find(|p| p.symbol == symbol)
    }

    pub fn unrealized(&self, price: impl Fn(&str) -> Option<f64>) -> f64 {
        self.positions
            .iter()
            .filter_map(|p| price(&p.symbol).map(|x| (x - p.avg) * p.qty))
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
        };
        if o.kind == Kind::Market {
            let Some(l) = last else {
                return Err("No price yet for this symbol.".into());
            };
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
        };
        self.fill(o, last, now);
    }

    /// A new price for `symbol`: triggers what it reaches. Returns the
    /// fills it caused.
    pub fn on_price(&mut self, symbol: &str, price: f64, now: i64) -> Vec<PaperFill> {
        if !(price > 0.0 && price.is_finite()) {
            return Vec::new();
        }
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
                    if let Some(f) = self.fill(o, p, now) {
                        out.push(f);
                    }
                    i = 0;
                }
                None => i += 1,
            }
        }
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
        let commission = qty * price * self.commission_pct / 100.0;
        let mut realized = -commission;
        match self.positions.iter().position(|p| p.symbol == o.symbol) {
            None => self.positions.push(Position {
                symbol: o.symbol.clone(),
                qty: signed,
                avg: price,
                opened: now,
            }),
            Some(k) => {
                let p = &mut self.positions[k];
                if p.qty.signum() == signed.signum() {
                    p.avg = (p.avg * p.qty.abs() + price * qty) / (p.qty.abs() + qty);
                    p.qty += signed;
                } else {
                    let closed = qty.min(p.qty.abs());
                    realized += (price - p.avg) * closed * p.qty.signum();
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
            buy,
            qty,
            kind,
            take_profit: None,
            stop_loss: None,
        }
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
        assert_eq!(back, a);
    }
}

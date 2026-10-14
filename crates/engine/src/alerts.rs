//! Alerts: what to watch, and when it fires. The app feeds each alert the
//! watched value and the level it's compared with (a price, a line's price
//! now, another plot) as prices arrive; [`Watch::feed`] decides, following
//! TradingView's trigger options. Script alerts (`alert()`,
//! `alertcondition`) are events from the script and don't go through here.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    Crossing,
    CrossingUp,
    CrossingDown,
    GreaterThan,
    LessThan,
}

impl Op {
    pub const ALL: [Op; 5] = [
        Op::Crossing,
        Op::CrossingUp,
        Op::CrossingDown,
        Op::GreaterThan,
        Op::LessThan,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Op::Crossing => "Crossing",
            Op::CrossingUp => "Crossing up",
            Op::CrossingDown => "Crossing down",
            Op::GreaterThan => "Greater than",
            Op::LessThan => "Less than",
        }
    }

    /// Whether the condition holds, given the previous and the current
    /// (value, level) pair. Crossings need a previous pair.
    pub fn test(self, prev: Option<(f64, f64)>, now: (f64, f64)) -> bool {
        let (v, l) = now;
        if v.is_nan() || l.is_nan() {
            return false;
        }
        match self {
            Op::GreaterThan => v > l,
            Op::LessThan => v < l,
            _ => {
                let Some((pv, pl)) = prev else { return false };
                if pv.is_nan() || pl.is_nan() {
                    return false;
                }
                let up = pv <= pl && v > l;
                let down = pv >= pl && v < l;
                match self {
                    Op::CrossingUp => up,
                    Op::CrossingDown => down,
                    _ => up || down,
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Freq {
    /// Fires once, then turns itself off.
    #[default]
    Once,
    OncePerBar,
    /// Checked only when a bar closes, on its closing values.
    OncePerBarClose,
    /// Every time it holds (at most once a second).
    EveryTime,
}

impl Freq {
    pub const ALL: [Freq; 4] = [Freq::Once, Freq::OncePerBar, Freq::OncePerBarClose, Freq::EveryTime];

    pub fn name(self) -> &'static str {
        match self {
            Freq::Once => "Only once",
            Freq::OncePerBar => "Once per bar",
            Freq::OncePerBarClose => "Once per bar close",
            Freq::EveryTime => "Every time",
        }
    }
}

/// What an alert compares.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    /// The symbol's price against a fixed level.
    Price { op: Op, value: f64 },
    /// The price against a drawn line (its price at each moment, by time).
    Line {
        op: Op,
        t1: i64,
        p1: f64,
        t2: i64,
        p2: f64,
        /// Rays and extended lines go on past their ends.
        extend: bool,
        /// The drawing, to follow it if it moves.
        drawing: Option<u64>,
    },
    /// A study's plot against a level or the price.
    Plot {
        uid: u64,
        study: String,
        plot: usize,
        title: String,
        op: Op,
        /// `None` compares with the price.
        value: Option<f64>,
    },
    /// A script's `alertcondition` (by title) or any `alert()` call.
    Script {
        uid: u64,
        study: String,
        title: Option<String>,
    },
}

impl Condition {
    pub fn summary(&self) -> String {
        match self {
            Condition::Price { op, value } => format!("Price {} {}", op.name().to_lowercase(), trim(*value)),
            Condition::Line { op, .. } => format!("Price {} the line", op.name().to_lowercase()),
            Condition::Plot { title, op, value, .. } => match value {
                Some(v) => format!("{title} {} {}", op.name().to_lowercase(), trim(*v)),
                None => format!("{title} {} the price", op.name().to_lowercase()),
            },
            Condition::Script { study, title, .. } => match title {
                Some(t) => format!("{study}: {t}"),
                None => format!("{study}: any alert() call"),
            },
        }
    }

    /// A line's price at time `t` (`None` outside a segment's ends).
    pub fn line_price(&self, t: i64) -> Option<f64> {
        let Condition::Line {
            t1, p1, t2, p2, extend, ..
        } = self
        else {
            return None;
        };
        if t1 == t2 {
            return Some(*p2);
        }
        let (a, b) = if t1 < t2 {
            ((t1, p1), (t2, p2))
        } else {
            ((t2, p2), (t1, p1))
        };
        if !extend && (t < *a.0 || t > *b.0) {
            return None;
        }
        let k = (t - a.0) as f64 / (b.0 - a.0) as f64;
        Some(a.1 + (b.1 - a.1) * k)
    }
}

fn trim(v: f64) -> String {
    let s = format!("{v:.8}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Alert {
    pub id: i64,
    pub name: String,
    /// Symbol id (`BINANCE:BTCUSDT`).
    pub symbol: String,
    /// The interval bars are counted on (once per bar, bar close).
    pub tf: String,
    pub condition: Condition,
    pub freq: Freq,
    /// Off after this time (ms), if set.
    pub expires: Option<i64>,
    /// The message; `{{close}}`, `{{ticker}}`, … are filled in.
    pub message: String,
    pub active: bool,
    pub notify: bool,
    pub sound: bool,
    pub webhook: String,
    pub telegram: bool,
    pub email: bool,
    pub created: i64,
    pub fired: u32,
    pub last_fired: Option<i64>,
}

impl Default for Alert {
    fn default() -> Self {
        Alert {
            id: 0,
            name: String::new(),
            symbol: String::new(),
            tf: "1m".into(),
            condition: Condition::Price {
                op: Op::Crossing,
                value: 0.0,
            },
            freq: Freq::Once,
            expires: None,
            message: String::new(),
            active: true,
            notify: true,
            sound: true,
            webhook: String::new(),
            telegram: false,
            email: false,
            created: 0,
            fired: 0,
            last_fired: None,
        }
    }
}

impl Alert {
    pub fn expired(&self, now: i64) -> bool {
        self.expires.is_some_and(|e| now >= e)
    }
}

/// An alert's memory between prices (not saved: it starts fresh).
#[derive(Clone, Debug, Default)]
pub struct Watch {
    prev: Option<(f64, f64)>,
    bucket: i64,
    last_in_bucket: Option<(f64, f64)>,
    prev_close: Option<(f64, f64)>,
    fired_bucket: Option<i64>,
    last_fire: Option<i64>,
}

impl Watch {
    /// Feeds the current (value, level) at time `now`, in bar `bucket`
    /// (the open time of the alert's interval bar). Returns whether the
    /// alert fires. Turning a fired `Once` alert off is the caller's job.
    pub fn feed(&mut self, a: &Alert, value: f64, level: f64, now: i64, bucket: i64) -> bool {
        if !a.active || a.expired(now) {
            return false;
        }
        let op = match &a.condition {
            Condition::Price { op, .. } | Condition::Line { op, .. } | Condition::Plot { op, .. } => *op,
            Condition::Script { .. } => return false,
        };
        let cur = (value, level);
        let mut fire = false;
        if a.freq == Freq::OncePerBarClose {
            // A new bar: the previous one closed on its last values.
            if bucket != self.bucket
                && let Some(closed) = self.last_in_bucket
            {
                fire = op.test(self.prev_close, closed);
                self.prev_close = Some(closed);
            }
        } else {
            let hit = op.test(self.prev, cur);
            fire = match a.freq {
                Freq::Once => hit,
                Freq::OncePerBar => hit && self.fired_bucket != Some(bucket),
                Freq::EveryTime => hit && self.last_fire.is_none_or(|t| now - t >= 1000),
                Freq::OncePerBarClose => false,
            };
        }
        self.prev = Some(cur);
        self.bucket = bucket;
        self.last_in_bucket = Some(cur);
        if fire {
            self.fired_bucket = Some(bucket);
            self.last_fire = Some(now);
        }
        fire
    }
}

/// Fills `{{…}}` placeholders in an alert message.
pub fn fill_message(msg: &str, ticker: &str, exchange: &str, interval: &str, close: f64, time: &str) -> String {
    msg.replace("{{ticker}}", ticker)
        .replace("{{exchange}}", exchange)
        .replace("{{interval}}", interval)
        .replace("{{close}}", &trim(close))
        .replace("{{time}}", time)
        .replace("{{timenow}}", time)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price_alert(op: Op, freq: Freq, value: f64) -> Alert {
        Alert {
            condition: Condition::Price { op, value },
            freq,
            ..Alert::default()
        }
    }

    #[test]
    fn crossings_need_a_previous_value() {
        assert!(!Op::Crossing.test(None, (11.0, 10.0)));
        assert!(Op::CrossingUp.test(Some((9.0, 10.0)), (11.0, 10.0)));
        assert!(!Op::CrossingDown.test(Some((9.0, 10.0)), (11.0, 10.0)));
        assert!(Op::Crossing.test(Some((11.0, 10.0)), (9.0, 10.0)));
        // Touching the level and leaving on the same side isn't a cross.
        assert!(!Op::Crossing.test(Some((10.0, 10.0)), (10.0, 10.0)));
        assert!(Op::GreaterThan.test(None, (11.0, 10.0)));
        assert!(!Op::LessThan.test(None, (f64::NAN, 10.0)));
    }

    #[test]
    fn once_per_bar_fires_once_in_each_bar() {
        let a = price_alert(Op::GreaterThan, Freq::OncePerBar, 10.0);
        let mut w = Watch::default();
        let fires: Vec<bool> = [(11.0, 0, 0), (12.0, 1, 0), (13.0, 2, 60), (9.0, 3, 120), (14.0, 4, 120)]
            .iter()
            .map(|&(v, t, b)| w.feed(&a, v, 10.0, t, b))
            .collect();
        assert_eq!(fires, vec![true, false, true, false, true]);
    }

    #[test]
    fn bar_close_uses_the_closing_values() {
        let a = price_alert(Op::CrossingUp, Freq::OncePerBarClose, 10.0);
        let mut w = Watch::default();
        // Bar 0 closes at 9, bar 1 pokes above 10 but closes at 9.5, bar 2
        // closes at 11: only bar 2's close crosses.
        let ticks = [(9.0, 0), (10.5, 60), (9.5, 60), (10.2, 120), (11.0, 120), (11.0, 180)];
        let fires: Vec<bool> = ticks
            .iter()
            .enumerate()
            .map(|(i, &(v, b))| w.feed(&a, v, 10.0, i as i64, b))
            .collect();
        assert_eq!(fires, vec![false, false, false, false, false, true]);
    }

    #[test]
    fn every_time_is_throttled_and_expiry_and_pause_stop_it() {
        let mut a = price_alert(Op::GreaterThan, Freq::EveryTime, 10.0);
        let mut w = Watch::default();
        assert!(w.feed(&a, 11.0, 10.0, 0, 0));
        assert!(!w.feed(&a, 11.0, 10.0, 500, 0));
        assert!(w.feed(&a, 11.0, 10.0, 1500, 0));
        a.expires = Some(2000);
        assert!(!w.feed(&a, 11.0, 10.0, 3000, 0));
        a.expires = None;
        a.active = false;
        assert!(!w.feed(&a, 11.0, 10.0, 5000, 0));
    }

    #[test]
    fn line_prices_and_messages() {
        let c = Condition::Line {
            op: Op::Crossing,
            t1: 0,
            p1: 10.0,
            t2: 100,
            p2: 20.0,
            extend: false,
            drawing: None,
        };
        assert_eq!(c.line_price(50), Some(15.0));
        assert_eq!(c.line_price(150), None);
        let ext = Condition::Line {
            op: Op::Crossing,
            t1: 0,
            p1: 10.0,
            t2: 100,
            p2: 20.0,
            extend: true,
            drawing: None,
        };
        assert_eq!(ext.line_price(200), Some(30.0));
        assert_eq!(
            fill_message("{{ticker}} at {{close}}", "BTCUSDT", "BINANCE", "1h", 64210.5, "t"),
            "BTCUSDT at 64210.5"
        );
        assert_eq!(
            Condition::Price {
                op: Op::CrossingUp,
                value: 100.0
            }
            .summary(),
            "Price crossing up 100"
        );
    }
}

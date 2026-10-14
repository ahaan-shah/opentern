//! Quant tools on top of the broker: a parameter optimizer (grid, random,
//! Bayesian), walk-forward analysis, Monte Carlo trade resampling and
//! portfolio (multi-symbol) backtests.
//!
//! Nothing here knows how a strategy is written: callers hand in a
//! `run(params, bars) -> Broker` closure (a Pine, Luau or JS study run
//! headlessly), so every language gets the same tools. Runs are spread
//! over rayon's pool, and every function is deterministic for a given seed.

pub mod export;
mod gp;
pub mod montecarlo;
pub mod optimize;
pub mod portfolio;
pub mod walkforward;

use crate::{Broker, Report};
use opentern_core::Bar;
use serde::{Deserialize, Serialize};

/// One parameter's value in a trial.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
}

impl std::fmt::Display for PValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PValue::Int(v) => write!(f, "{v}"),
            PValue::Float(v) => {
                let s = format!("{v:.6}");
                f.write_str(s.trim_end_matches('0').trim_end_matches('.'))
            }
            PValue::Bool(v) => f.write_str(if *v { "on" } else { "off" }),
            PValue::Text(s) => f.write_str(s),
        }
    }
}

/// The values a parameter is tried at.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Range {
    Int { from: i64, to: i64, step: i64 },
    Float { from: f64, to: f64, step: f64 },
    Bool,
    Choice(Vec<String>),
}

impl Range {
    /// Every value, in order (ranges run from `from` to `to` inclusive).
    pub fn values(&self) -> Vec<PValue> {
        match self {
            Range::Int { from, to, step } => {
                let (lo, hi) = (*from.min(to), *from.max(to));
                let step = (*step).max(1);
                (0..)
                    .map(|k| lo + k * step)
                    .take_while(|v| *v <= hi)
                    .map(PValue::Int)
                    .collect()
            }
            Range::Float { from, to, step } => {
                let (lo, hi) = (from.min(*to), from.max(*to));
                let step = if *step > 0.0 { *step } else { (hi - lo).max(1e-12) };
                let n = ((hi - lo) / step + 1e-9).floor() as i64;
                (0..=n.clamp(0, 1_000_000))
                    .map(|k| PValue::Float(round_to(lo + k as f64 * step, step)))
                    .collect()
            }
            Range::Bool => vec![PValue::Bool(false), PValue::Bool(true)],
            Range::Choice(c) => c.iter().cloned().map(PValue::Text).collect(),
        }
    }

    pub fn count(&self) -> usize {
        match self {
            Range::Int { from, to, step } => ((from - to).unsigned_abs() / (*step).max(1) as u64 + 1) as usize,
            _ => self.values().len(),
        }
    }
}

/// `v` rounded to the step's decimals (so 0.1 + 0.2 prints as 0.3).
fn round_to(v: f64, step: f64) -> f64 {
    let dec = (-step.abs().log10().floor()).clamp(0.0, 12.0) as i32 + 2;
    let k = 10f64.powi(dec);
    (v * k).round() / k
}

/// A parameter to vary: the input's id, its title and its range.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Param {
    pub id: String,
    pub title: String,
    pub range: Range,
}

/// What the optimizer maximises.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Objective {
    #[default]
    NetProfit,
    ProfitFactor,
    Sharpe,
    Sortino,
    PercentProfitable,
    /// The smallest maximum drawdown.
    MaxDrawdown,
    /// Net profit ÷ max drawdown.
    ReturnOverDrawdown,
    Cagr,
}

impl Objective {
    pub const ALL: [Objective; 8] = [
        Objective::NetProfit,
        Objective::ProfitFactor,
        Objective::Sharpe,
        Objective::Sortino,
        Objective::PercentProfitable,
        Objective::MaxDrawdown,
        Objective::ReturnOverDrawdown,
        Objective::Cagr,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Objective::NetProfit => "Net profit",
            Objective::ProfitFactor => "Profit factor",
            Objective::Sharpe => "Sharpe ratio",
            Objective::Sortino => "Sortino ratio",
            Objective::PercentProfitable => "Percent profitable",
            Objective::MaxDrawdown => "Smallest max drawdown",
            Objective::ReturnOverDrawdown => "Net profit ÷ max drawdown",
            Objective::Cagr => "CAGR",
        }
    }

    /// Higher is better. `NaN` for runs that can't be scored.
    pub fn score(self, s: &Summary) -> f64 {
        let v = match self {
            Objective::NetProfit => s.net_profit,
            Objective::ProfitFactor => {
                // No losing trade: infinitely good, ranked by profit.
                if s.profit_factor.is_nan() && s.net_profit > 0.0 {
                    1e9 + s.net_profit
                } else {
                    s.profit_factor
                }
            }
            Objective::Sharpe => s.sharpe,
            Objective::Sortino => s.sortino,
            Objective::PercentProfitable => s.percent_profitable,
            Objective::MaxDrawdown => -s.max_drawdown,
            Objective::ReturnOverDrawdown => {
                if s.max_drawdown > 0.0 {
                    s.net_profit / s.max_drawdown
                } else if s.net_profit > 0.0 {
                    1e9 + s.net_profit
                } else {
                    f64::NAN
                }
            }
            Objective::Cagr => s.cagr,
        };
        if v.is_finite() { v } else { f64::NAN }
    }
}

/// The headline numbers of one run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub net_profit: f64,
    pub net_profit_pct: f64,
    pub total_trades: usize,
    pub percent_profitable: f64,
    pub profit_factor: f64,
    pub max_drawdown: f64,
    pub max_drawdown_pct: f64,
    pub sharpe: f64,
    pub sortino: f64,
    pub cagr: f64,
    pub avg_trade: f64,
}

impl Summary {
    pub fn of(r: &Report) -> Summary {
        Summary {
            net_profit: r.all.net_profit,
            net_profit_pct: r.return_on_capital,
            total_trades: r.all.total_trades,
            percent_profitable: r.all.percent_profitable,
            profit_factor: r.all.profit_factor,
            max_drawdown: r.max_drawdown,
            max_drawdown_pct: r.max_drawdown_pct,
            sharpe: r.sharpe,
            sortino: r.sortino,
            cagr: r.cagr,
            avg_trade: r.all.avg_trade,
        }
    }

    pub fn of_run(b: &Broker, bars: &[Bar]) -> Summary {
        Summary::of(&Report::new(b, bars))
    }
}

/// Runs a strategy with given parameters on given bars. `None` when the
/// script failed (its error is reported elsewhere).
pub trait Runner: Fn(&[PValue], &[Bar]) -> Option<Broker> + Sync {}
impl<T: Fn(&[PValue], &[Bar]) -> Option<Broker> + Sync> Runner for T {}

/// How far a long job has got, and a way to stop it, shared with the UI.
#[derive(Debug, Default)]
pub struct Progress {
    pub done: std::sync::atomic::AtomicUsize,
    pub total: std::sync::atomic::AtomicUsize,
    pub cancel: std::sync::atomic::AtomicBool,
}

impl Progress {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn tick(&self) {
        self.done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn add_total(&self, n: usize) {
        self.total.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn fraction(&self) -> f32 {
        let t = self.total.load(std::sync::atomic::Ordering::Relaxed);
        if t == 0 {
            0.0
        } else {
            (self.done.load(std::sync::atomic::Ordering::Relaxed) as f32 / t as f32).min(1.0)
        }
    }
}

/// A small, fast, seedable generator (SplitMix64), so runs repeat exactly.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in 0..n (n > 0).
    pub fn below(&mut self, n: usize) -> usize {
        ((self.next_u64() as u128 * n as u128) >> 64) as usize
    }

    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            v.swap(i, self.below(i + 1));
        }
    }
}

/// The `q`-quantile (0..=1) of sorted values, interpolated.
pub fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let pos = q.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let (i, frac) = (pos.floor() as usize, pos.fract());
    match sorted.get(i + 1) {
        Some(next) => sorted[i] + (next - sorted[i]) * frac,
        None => sorted[i],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_list_their_values() {
        let r = Range::Int {
            from: 5,
            to: 20,
            step: 5,
        };
        assert_eq!(
            r.values(),
            vec![PValue::Int(5), PValue::Int(10), PValue::Int(15), PValue::Int(20)]
        );
        assert_eq!(r.count(), 4);
        let f = Range::Float {
            from: 0.1,
            to: 0.5,
            step: 0.1,
        };
        let v = f.values();
        assert_eq!(v.len(), 5);
        assert_eq!(v[2], PValue::Float(0.3));
        assert_eq!(v[4].to_string(), "0.5");
        assert_eq!(
            Range::Int {
                from: 3,
                to: 3,
                step: 1
            }
            .count(),
            1
        );
    }

    #[test]
    fn rng_repeats_and_spreads() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        let xs: Vec<u64> = (0..5).map(|_| a.next_u64()).collect();
        let ys: Vec<u64> = (0..5).map(|_| b.next_u64()).collect();
        assert_eq!(xs, ys);
        let mut r = Rng::new(1);
        let mut hits = [0usize; 4];
        for _ in 0..4000 {
            hits[r.below(4)] += 1;
        }
        assert!(hits.iter().all(|h| (800..1200).contains(h)), "{hits:?}");
    }

    #[test]
    fn quantiles_interpolate() {
        let v = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(quantile(&v, 0.5), 3.0);
        assert_eq!(quantile(&v, 0.0), 1.0);
        assert_eq!(quantile(&v, 1.0), 5.0);
        assert_eq!(quantile(&v, 0.25), 2.0);
    }
}

//! Monte Carlo trade resampling: replays the closed trades' profits in
//! random orders (or draws them with replacement) many times, to show how
//! much of a backtest's drawdown and result was luck of the sequence.
//!
//! - **Shuffle** keeps every trade once, in a random order: the final
//!   profit never changes, but drawdowns do.
//! - **Bootstrap** draws as many trades as there were, with replacement:
//!   both the result and the drawdowns vary.

use super::{Rng, quantile};
use crate::Trade;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resample {
    #[default]
    Shuffle,
    Bootstrap,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct McSetup {
    pub runs: usize,
    pub method: Resample,
    /// A drawdown this deep (percent of the peak) counts as ruin.
    pub ruin_pct: f64,
    pub seed: u64,
}

impl Default for McSetup {
    fn default() -> Self {
        McSetup {
            runs: 1000,
            method: Resample::Shuffle,
            ruin_pct: 50.0,
            seed: 1,
        }
    }
}

/// Five-number summary of a distribution: 5th, 25th, 50th, 75th, 95th
/// percentiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Spread {
    pub p5: f64,
    pub p25: f64,
    pub p50: f64,
    pub p75: f64,
    pub p95: f64,
}

impl Spread {
    fn of(mut v: Vec<f64>) -> Spread {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Spread {
            p5: quantile(&v, 0.05),
            p25: quantile(&v, 0.25),
            p50: quantile(&v, 0.5),
            p75: quantile(&v, 0.75),
            p95: quantile(&v, 0.95),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct McResult {
    pub runs: usize,
    pub trades: usize,
    pub initial_capital: f64,
    /// The backtest as it happened.
    pub actual_net: f64,
    pub actual_max_drawdown: f64,
    pub net_profit: Spread,
    /// Peak-to-trough on the trade-by-trade equity, money and percent.
    pub max_drawdown: Spread,
    pub max_drawdown_pct: Spread,
    pub longest_losing_streak: Spread,
    /// Percent of runs ending below the initial capital.
    pub prob_loss: f64,
    /// Percent of runs whose drawdown reached `ruin_pct`.
    pub prob_ruin: f64,
    /// Percentile bands of equity after each trade (index 0 = start):
    /// p5, p50, p95.
    pub bands: Vec<(f64, f64, f64)>,
    /// Final net profit of every run (for the histogram).
    pub finals: Vec<f64>,
}

struct Path {
    net: f64,
    dd: f64,
    dd_pct: f64,
    streak: usize,
    curve: Vec<f64>,
}

fn walk(cap: f64, pnls: impl Iterator<Item = f64>, keep: bool) -> Path {
    let mut eq = cap;
    let mut peak = cap;
    let (mut dd, mut dd_pct) = (0.0f64, 0.0f64);
    let (mut streak, mut run) = (0usize, 0usize);
    let mut curve = if keep { vec![cap] } else { Vec::new() };
    for p in pnls {
        eq += p;
        if keep {
            curve.push(eq);
        }
        peak = peak.max(eq);
        dd = dd.max(peak - eq);
        if peak > 0.0 {
            dd_pct = dd_pct.max((peak - eq) / peak * 100.0);
        }
        if p < 0.0 {
            run += 1;
            streak = streak.max(run);
        } else {
            run = 0;
        }
    }
    Path {
        net: eq - cap,
        dd,
        dd_pct,
        streak,
        curve,
    }
}

pub fn monte_carlo(trades: &[Trade], initial_capital: f64, s: &McSetup) -> Result<McResult, String> {
    if trades.len() < 2 {
        return Err("Monte Carlo needs at least two closed trades.".into());
    }
    let pnls: Vec<f64> = trades.iter().map(|t| t.pnl).collect();
    let runs = s.runs.clamp(10, 100_000);
    let n = pnls.len();
    // Curves for the bands are kept for as many runs as fit in ~40 MB.
    let keep = (5_000_000 / (n + 1)).clamp(100, runs);
    let paths: Vec<Path> = (0..runs)
        .into_par_iter()
        .map(|k| {
            // One generator per run, so the result doesn't depend on threads.
            let mut rng = Rng::new(s.seed.wrapping_mul(1_000_003).wrapping_add(k as u64));
            let order: Vec<f64> = match s.method {
                Resample::Shuffle => {
                    let mut v = pnls.clone();
                    rng.shuffle(&mut v);
                    v
                }
                Resample::Bootstrap => (0..n).map(|_| pnls[rng.below(n)]).collect(),
            };
            walk(initial_capital, order.into_iter(), k < keep)
        })
        .collect();
    let actual = walk(initial_capital, pnls.iter().cloned(), false);
    let mut bands = Vec::with_capacity(n + 1);
    let mut col = vec![0.0; keep];
    for i in 0..=n {
        for (c, p) in col.iter_mut().zip(&paths[..keep]) {
            *c = p.curve[i];
        }
        col.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        bands.push((quantile(&col, 0.05), quantile(&col, 0.5), quantile(&col, 0.95)));
    }
    let finals: Vec<f64> = paths.iter().map(|p| p.net).collect();
    Ok(McResult {
        runs,
        trades: n,
        initial_capital,
        actual_net: actual.net,
        actual_max_drawdown: actual.dd,
        net_profit: Spread::of(finals.clone()),
        max_drawdown: Spread::of(paths.iter().map(|p| p.dd).collect()),
        max_drawdown_pct: Spread::of(paths.iter().map(|p| p.dd_pct).collect()),
        longest_losing_streak: Spread::of(paths.iter().map(|p| p.streak as f64).collect()),
        prob_loss: paths.iter().filter(|p| p.net < 0.0).count() as f64 / runs as f64 * 100.0,
        prob_ruin: paths.iter().filter(|p| p.dd_pct >= s.ruin_pct).count() as f64 / runs as f64 * 100.0,
        bands,
        finals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trade(pnl: f64) -> Trade {
        Trade {
            entry_id: "L".into(),
            exit_id: "L".into(),
            long: true,
            qty: 1.0,
            entry_price: 100.0,
            entry_bar: 0,
            entry_time: 0,
            exit_price: 100.0 + pnl,
            exit_bar: 1,
            exit_time: 0,
            pnl,
            pnl_pct: pnl,
            commission: 0.0,
            entry_commission: 0.0,
            runup: 0.0,
            drawdown: 0.0,
            entry_comment: String::new(),
            exit_comment: String::new(),
        }
    }

    #[test]
    fn shuffle_keeps_the_total_and_varies_drawdown() {
        let trades: Vec<Trade> = [50.0, -20.0, 30.0, -40.0, 10.0, -5.0, 60.0, -30.0]
            .iter()
            .map(|p| trade(*p))
            .collect();
        let r = monte_carlo(&trades, 1000.0, &McSetup::default()).expect("runs");
        assert_eq!(r.runs, 1000);
        assert!((r.net_profit.p5 - 55.0).abs() < 1e-9 && (r.net_profit.p95 - 55.0).abs() < 1e-9);
        assert!(r.max_drawdown.p5 < r.max_drawdown.p95);
        // Worst possible: all four losses in a row after the peak = 95.
        assert!(r.max_drawdown.p95 <= 95.0 + 1e-9);
        assert_eq!(r.prob_loss, 0.0);
        assert_eq!(r.bands.len(), 9);
        assert!((r.bands[8].1 - 1055.0).abs() < 1e-9);
        assert_eq!(r.actual_net, 55.0);
        // Same seed, same answer.
        assert_eq!(r, monte_carlo(&trades, 1000.0, &McSetup::default()).expect("runs"));
    }

    #[test]
    fn bootstrap_varies_the_result() {
        let trades: Vec<Trade> = (0..40).map(|i| trade(if i % 3 == 0 { -30.0 } else { 20.0 })).collect();
        let s = McSetup {
            method: Resample::Bootstrap,
            runs: 2000,
            ..McSetup::default()
        };
        let r = monte_carlo(&trades, 1000.0, &s).expect("runs");
        assert!(r.net_profit.p5 < r.net_profit.p50 && r.net_profit.p50 < r.net_profit.p95);
        // Expected net ≈ 40 × (2/3·20 − 1/3·30) ≈ 133.
        assert!((r.net_profit.p50 - 133.0).abs() < 40.0, "{}", r.net_profit.p50);
        assert!(r.prob_loss > 0.0 && r.prob_loss < 50.0);
        assert!(monte_carlo(&trades[..1], 1000.0, &s).is_err());
    }
}

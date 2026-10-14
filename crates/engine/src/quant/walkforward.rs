//! Walk-forward analysis: optimize on an in-sample window, trade the best
//! inputs on the out-of-sample window that follows, step forward by the
//! out-of-sample length and repeat. The out-of-sample pieces, stitched
//! together, are what the strategy would have earned had it been
//! re-optimized on that schedule.
//!
//! Each out-of-sample run starts at the in-sample window's first bar, so
//! indicators are warmed up; only what happens on out-of-sample bars
//! counts. A position still open from the in-sample part carries over (it
//! would have been open in real life too): its P&L on out-of-sample bars
//! counts, and the trade counts there when it closes.

use super::optimize::{self, Setup};
use super::{PValue, Progress, Runner, Summary};
use crate::Trade;
use opentern_core::Bar;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WalkSetup {
    pub optimize: Setup,
    /// Bars in each in-sample window.
    pub in_sample: usize,
    /// Bars in each out-of-sample window (also the step).
    pub out_sample: usize,
    /// In-sample windows all start at the first bar (growing) instead of
    /// rolling forward.
    pub anchored: bool,
}

/// One in-sample / out-of-sample pair.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Window {
    /// Bar index ranges, end exclusive.
    pub in_from: usize,
    pub in_to: usize,
    pub out_to: usize,
    pub in_time: (i64, i64),
    pub out_time: (i64, i64),
    /// The best inputs in-sample (`None`: nothing scored).
    pub values: Option<Vec<PValue>>,
    pub in_sample: Summary,
    pub in_score: f64,
    pub out_net: f64,
    pub out_trades: usize,
    pub out_winners: usize,
    pub out_max_drawdown: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WalkResult {
    pub windows: Vec<Window>,
    /// Equity on each out-of-sample bar, stitched: (time, equity).
    pub equity: Vec<(i64, f64)>,
    pub initial_capital: f64,
    pub out_net: f64,
    pub out_trades: usize,
    pub out_percent_profitable: f64,
    pub out_max_drawdown: f64,
    /// Out-of-sample profit per bar ÷ in-sample profit per bar, percent.
    pub efficiency: f64,
    /// Windows with a profitable out-of-sample run, percent.
    pub profitable_windows: f64,
    pub cancelled: bool,
}

/// The windows' bar ranges: (in_from, in_to, out_to).
pub fn windows(n: usize, w: &WalkSetup) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    if w.in_sample == 0 || w.out_sample == 0 {
        return out;
    }
    let mut start = 0;
    while start + w.in_sample < n {
        let in_from = if w.anchored { 0 } else { start };
        let in_to = start + w.in_sample;
        let out_to = (in_to + w.out_sample).min(n);
        out.push((in_from, in_to, out_to));
        start += w.out_sample;
    }
    out
}

fn max_dd(curve: &[f64]) -> f64 {
    let mut peak = f64::NEG_INFINITY;
    let mut dd = 0.0f64;
    for &v in curve {
        peak = peak.max(v);
        dd = dd.max(peak - v);
    }
    dd
}

pub fn walk_forward(w: &WalkSetup, bars: &[Bar], run: &impl Runner, progress: &Progress) -> Result<WalkResult, String> {
    let wins = windows(bars.len(), w);
    if wins.is_empty() {
        return Err(format!(
            "Not enough bars: each step needs {} in-sample bars and some after them, and there are {}.",
            w.in_sample,
            bars.len()
        ));
    }
    if w.optimize.params.is_empty() {
        return Err("Pick at least one input to optimize.".into());
    }
    let per = optimize::planned(&w.optimize).ok_or_else(|| {
        format!(
            "That's {} combinations per window, more than the grid's {}. Narrow the ranges or use Random or Bayesian.",
            optimize::combinations(&w.optimize.params),
            optimize::MAX_GRID
        )
    })?;
    progress.add_total(per * wins.len());
    let mut res = WalkResult::default();
    let mut eq = f64::NAN;
    let (mut in_profit, mut in_bars, mut out_bars) = (0.0, 0usize, 0usize);
    let mut out_trades: Vec<Trade> = Vec::new();
    for (in_from, in_to, out_to) in wins {
        if progress.cancelled() {
            res.cancelled = true;
            break;
        }
        let outcome = optimize::search(&w.optimize, &bars[in_from..in_to], run, progress)?;
        let mut win = Window {
            in_from,
            in_to,
            out_to,
            in_time: (bars[in_from].time, bars[in_to - 1].time),
            out_time: (bars[in_to].time, bars[out_to - 1].time),
            values: None,
            in_sample: Summary::default(),
            in_score: f64::NAN,
            out_net: 0.0,
            out_trades: 0,
            out_winners: 0,
            out_max_drawdown: 0.0,
        };
        let best = outcome.best().cloned();
        let ran = best
            .as_ref()
            .and_then(|t| run(&t.values, &bars[in_from..out_to]).map(|b| (t.clone(), b)));
        if let Some((t, b)) = ran {
            if res.initial_capital == 0.0 {
                res.initial_capital = b.props.initial_capital;
                eq = res.initial_capital;
            }
            let off = in_to - in_from;
            let curve = &b.equity;
            let mut seg = Vec::with_capacity(out_to - in_to);
            for k in off..curve.len().min(out_to - in_from) {
                eq += curve[k] - curve[k - 1];
                seg.push(eq);
                res.equity.push((bars[in_from + k].time, eq));
            }
            let trades: Vec<&Trade> = b.trades.iter().filter(|x| x.exit_bar >= off).collect();
            win.out_net = curve.get(curve.len().min(out_to - in_from) - 1).copied().unwrap_or(0.0) - curve[off - 1];
            win.out_trades = trades.len();
            win.out_winners = trades.iter().filter(|x| x.pnl > 0.0).count();
            win.out_max_drawdown = max_dd(&seg);
            out_trades.extend(trades.into_iter().cloned());
            in_profit += t.summary.net_profit;
            in_bars += in_to - in_from;
            out_bars += out_to - in_to;
            win.in_sample = t.summary;
            win.in_score = t.score;
            win.values = Some(t.values);
        } else {
            // Nothing scored in-sample: stay flat out-of-sample.
            for b in &bars[in_to..out_to] {
                if !eq.is_nan() {
                    res.equity.push((b.time, eq));
                }
            }
        }
        res.windows.push(win);
    }
    res.out_net = res.windows.iter().map(|w| w.out_net).sum();
    res.out_trades = out_trades.len();
    res.out_percent_profitable = if out_trades.is_empty() {
        0.0
    } else {
        out_trades.iter().filter(|t| t.pnl > 0.0).count() as f64 / out_trades.len() as f64 * 100.0
    };
    let curve: Vec<f64> = std::iter::once(res.initial_capital)
        .chain(res.equity.iter().map(|e| e.1))
        .collect();
    res.out_max_drawdown = max_dd(&curve);
    res.efficiency = if in_profit > 0.0 && in_bars > 0 && out_bars > 0 {
        (res.out_net / out_bars as f64) / (in_profit / in_bars as f64) * 100.0
    } else {
        f64::NAN
    };
    let traded: Vec<&Window> = res.windows.iter().filter(|w| w.values.is_some()).collect();
    res.profitable_windows = if traded.is_empty() {
        0.0
    } else {
        traded.iter().filter(|w| w.out_net > 0.0).count() as f64 / traded.len() as f64 * 100.0
    };
    res.cancelled |= progress.cancelled();
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quant::optimize::Method;
    use crate::quant::optimize::tests::{bars, ma_cross};
    use crate::quant::{Objective, Param, Range};

    fn setup(anchored: bool) -> WalkSetup {
        WalkSetup {
            optimize: Setup {
                params: vec![
                    Param {
                        id: "fast".into(),
                        title: "Fast".into(),
                        range: Range::Int {
                            from: 3,
                            to: 12,
                            step: 3,
                        },
                    },
                    Param {
                        id: "slow".into(),
                        title: "Slow".into(),
                        range: Range::Int {
                            from: 20,
                            to: 40,
                            step: 10,
                        },
                    },
                ],
                method: Method::Grid,
                objective: Objective::NetProfit,
                min_trades: 1,
                seed: 1,
            },
            in_sample: 300,
            out_sample: 100,
            anchored,
        }
    }

    #[test]
    fn windows_roll_or_anchor() {
        let w = setup(false);
        assert_eq!(
            windows(650, &w),
            vec![(0, 300, 400), (100, 400, 500), (200, 500, 600), (300, 600, 650)]
        );
        let a = setup(true);
        assert_eq!(windows(650, &a)[2], (0, 500, 600));
        assert!(windows(300, &w).is_empty());
    }

    #[test]
    fn stitched_equity_matches_window_profits() {
        let bars = bars(800);
        let p = Progress::default();
        let r = walk_forward(&setup(false), &bars, &ma_cross, &p).expect("runs");
        assert_eq!(r.windows.len(), 5);
        assert_eq!(p.done.load(std::sync::atomic::Ordering::Relaxed), 5 * 12);
        assert_eq!(r.equity.len(), 500);
        let last = r.equity.last().expect("equity").1;
        assert!((last - r.initial_capital - r.out_net).abs() < 1e-6);
        // Each window's own out-of-sample run reproduces its piece.
        let w = &r.windows[1];
        let b = ma_cross(w.values.as_ref().expect("values"), &bars[w.in_from..w.out_to]).expect("run");
        let off = w.in_to - w.in_from;
        assert!((b.equity[b.equity.len() - 1] - b.equity[off - 1] - w.out_net).abs() < 1e-9);
        assert!(r.equity.windows(2).all(|x| x[0].0 < x[1].0));
    }

    #[test]
    fn too_few_bars_is_a_plain_error() {
        let e = walk_forward(&setup(false), &bars(200), &ma_cross, &Progress::default()).unwrap_err();
        assert!(e.starts_with("Not enough bars"), "{e}");
    }
}

//! The parameter optimizer. Every trial is a full backtest of the strategy
//! with one set of inputs; trials run in parallel on rayon's pool.
//!
//! - **Grid** tries every combination (up to [`MAX_GRID`]).
//! - **Random** tries a fixed number of distinct random combinations.
//! - **Bayesian** starts with random ones, then fits a Gaussian process to
//!   the scores so far and tries where expected improvement is highest, a
//!   batch at a time (one per core, spread out with the "believer" trick).

use super::gp::Gp;
use super::{Objective, PValue, Param, Progress, Rng, Runner, Summary};
use opentern_core::Bar;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Most combinations a grid search will try.
pub const MAX_GRID: usize = 50_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Method {
    Grid,
    Random { trials: usize },
    Bayesian { trials: usize },
}

impl Method {
    pub fn name(self) -> &'static str {
        match self {
            Method::Grid => "Grid (every combination)",
            Method::Random { .. } => "Random",
            Method::Bayesian { .. } => "Bayesian",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Setup {
    pub params: Vec<Param>,
    pub method: Method,
    pub objective: Objective,
    /// Runs with fewer closed trades aren't scored.
    pub min_trades: usize,
    pub seed: u64,
}

/// One backtest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trial {
    pub values: Vec<PValue>,
    pub summary: Summary,
    /// Higher is better; `NaN` when the run failed or had too few trades.
    pub score: f64,
    /// The script failed with these inputs.
    pub failed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    /// Best first; unscored trials last.
    pub trials: Vec<Trial>,
    pub cancelled: bool,
}

impl Outcome {
    pub fn best(&self) -> Option<&Trial> {
        self.trials.first().filter(|t| !t.score.is_nan())
    }
}

/// How many combinations the parameters make (saturating).
pub fn combinations(params: &[Param]) -> usize {
    params
        .iter()
        .fold(1usize, |n, p| n.saturating_mul(p.range.count().max(1)))
}

/// Every parameter's values, listed once.
fn axes(params: &[Param]) -> Vec<Vec<PValue>> {
    params.iter().map(|p| p.range.values()).collect()
}

fn pick(axes: &[Vec<PValue>], pos: &[usize]) -> Vec<PValue> {
    axes.iter().zip(pos).map(|(a, &i)| a[i].clone()).collect()
}

/// Positions in the unit cube (for the GP).
fn unit(axes: &[Vec<PValue>], pos: &[usize]) -> Vec<f64> {
    axes.iter()
        .zip(pos)
        .map(|(a, &i)| {
            if a.len() > 1 {
                i as f64 / (a.len() - 1) as f64
            } else {
                0.5
            }
        })
        .collect()
}

fn random_pos(axes: &[Vec<PValue>], rng: &mut Rng) -> Vec<usize> {
    axes.iter().map(|a| rng.below(a.len().max(1))).collect()
}

fn evaluate(setup: &Setup, bars: &[Bar], run: &impl Runner, values: Vec<PValue>) -> Trial {
    match run(&values, bars) {
        Some(b) => {
            let summary = Summary::of_run(&b, bars);
            let score = if summary.total_trades < setup.min_trades {
                f64::NAN
            } else {
                setup.objective.score(&summary)
            };
            Trial {
                values,
                summary,
                score,
                failed: false,
            }
        }
        None => Trial {
            values,
            summary: Summary::default(),
            score: f64::NAN,
            failed: true,
        },
    }
}

fn run_batch(
    setup: &Setup,
    bars: &[Bar],
    run: &impl Runner,
    batch: Vec<Vec<PValue>>,
    progress: &Progress,
) -> Vec<Trial> {
    batch
        .into_par_iter()
        .filter_map(|v| {
            if progress.cancelled() {
                return None;
            }
            let t = evaluate(setup, bars, run, v);
            progress.tick();
            Some(t)
        })
        .collect()
}

/// Runs the search. Errors are plain sentences for the UI.
pub fn optimize(setup: &Setup, bars: &[Bar], run: &impl Runner, progress: &Progress) -> Result<Outcome, String> {
    if let Some(n) = planned(setup) {
        progress.add_total(n);
    }
    search(setup, bars, run, progress)
}

/// How many runs a search will make, if it can run at all.
pub fn planned(setup: &Setup) -> Option<usize> {
    let combos = combinations(&setup.params);
    match setup.method {
        Method::Random { trials } | Method::Bayesian { trials } if trials < combos => Some(trials),
        _ if combos <= MAX_GRID => Some(combos),
        _ => None,
    }
}

/// [`optimize`] without adding to the progress total (the caller has).
pub(crate) fn search(setup: &Setup, bars: &[Bar], run: &impl Runner, progress: &Progress) -> Result<Outcome, String> {
    if setup.params.is_empty() {
        return Err("Pick at least one input to optimize.".into());
    }
    if bars.is_empty() {
        return Err("There are no bars to test on yet.".into());
    }
    let axes = axes(&setup.params);
    if let Some(p) = setup.params.iter().zip(&axes).find(|(_, a)| a.is_empty()) {
        return Err(format!("{} has no values to try.", p.0.title));
    }
    let combos = combinations(&setup.params);
    let method = match setup.method {
        Method::Random { trials } | Method::Bayesian { trials } if trials >= combos => Method::Grid,
        m => m,
    };
    let mut trials = match method {
        Method::Grid => {
            if combos > MAX_GRID {
                return Err(format!(
                    "That's {combos} combinations, more than the grid's {MAX_GRID}. Narrow the ranges or use Random or Bayesian."
                ));
            }
            let all: Vec<Vec<PValue>> = (0..combos)
                .map(|mut k| {
                    let pos: Vec<usize> = axes
                        .iter()
                        .map(|a| {
                            let i = k % a.len();
                            k /= a.len();
                            i
                        })
                        .collect();
                    pick(&axes, &pos)
                })
                .collect();
            run_batch(setup, bars, run, all, progress)
        }
        Method::Random { trials } => {
            let mut rng = Rng::new(setup.seed);
            let mut seen = HashSet::new();
            let mut batch = Vec::new();
            let mut attempts = 0;
            while batch.len() < trials && attempts < trials * 50 {
                attempts += 1;
                let pos = random_pos(&axes, &mut rng);
                if seen.insert(pos.clone()) {
                    batch.push(pick(&axes, &pos));
                }
            }
            run_batch(setup, bars, run, batch, progress)
        }
        Method::Bayesian { trials } => bayesian(setup, bars, run, &axes, trials, progress),
    };
    sort(&mut trials);
    Ok(Outcome {
        trials,
        cancelled: progress.cancelled(),
    })
}

/// Best first, unscored last; ties keep the order they ran in.
pub fn sort(trials: &mut [Trial]) {
    trials.sort_by(|a, b| match (a.score.is_nan(), b.score.is_nan()) {
        (false, false) => b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal),
        (false, true) => std::cmp::Ordering::Less,
        (true, false) => std::cmp::Ordering::Greater,
        _ => std::cmp::Ordering::Equal,
    });
}

fn bayesian(
    setup: &Setup,
    bars: &[Bar],
    run: &impl Runner,
    axes: &[Vec<PValue>],
    budget: usize,
    progress: &Progress,
) -> Vec<Trial> {
    let mut rng = Rng::new(setup.seed);
    let dims = axes.len();
    let mut seen: HashSet<Vec<usize>> = HashSet::new();
    let mut tried: Vec<(Vec<usize>, Trial)> = Vec::new();
    let fresh =
        |rng: &mut Rng, seen: &HashSet<Vec<usize>>| (0..200).map(|_| random_pos(axes, rng)).find(|p| !seen.contains(p));
    // Start with random points.
    let start = (2 * dims + 6).min(budget);
    let mut first = Vec::new();
    while first.len() < start {
        let Some(p) = fresh(&mut rng, &seen) else { break };
        seen.insert(p.clone());
        first.push(p);
    }
    let batch: Vec<Vec<PValue>> = first.iter().map(|p| pick(axes, p)).collect();
    tried.extend(first.into_iter().zip(run_batch(setup, bars, run, batch, progress)));
    let width = rayon::current_num_threads().clamp(1, 8);
    while tried.len() < budget && !progress.cancelled() {
        // Unscored runs count as a bit worse than the worst scored one.
        let scored: Vec<f64> = tried.iter().map(|(_, t)| t.score).filter(|s| !s.is_nan()).collect();
        let (lo, hi) = scored
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), s| (a.min(*s), b.max(*s)));
        let floor = if scored.is_empty() {
            0.0
        } else {
            lo - (hi - lo).max(1.0) * 0.25
        };
        let mut xs: Vec<Vec<f64>> = tried.iter().map(|(p, _)| unit(axes, p)).collect();
        let mut ys: Vec<f64> = tried
            .iter()
            .map(|(_, t)| if t.score.is_nan() { floor } else { t.score })
            .collect();
        let mut best = ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        // Candidates: random points, plus every neighbour of the best few.
        let mut cands: Vec<Vec<usize>> = (0..1500).map(|_| random_pos(axes, &mut rng)).collect();
        let mut order: Vec<usize> = (0..tried.len()).collect();
        order.sort_by(|a, b| ys[*b].partial_cmp(&ys[*a]).unwrap_or(std::cmp::Ordering::Equal));
        for &k in order.iter().take(5) {
            let p = &tried[k].0;
            for d in 0..dims {
                for step in [-2i64, -1, 1, 2] {
                    let i = p[d] as i64 + step;
                    if i >= 0 && (i as usize) < axes[d].len() {
                        let mut q = p.clone();
                        q[d] = i as usize;
                        cands.push(q);
                    }
                }
            }
        }
        cands.retain(|c| !seen.contains(c));
        cands.sort();
        cands.dedup();
        if cands.is_empty() {
            // Search space exhausted near what's been found; try fresh points.
            match fresh(&mut rng, &seen) {
                Some(p) => cands.push(p),
                None => break,
            }
        }
        let take = width.min(budget - tried.len());
        let mut picks: Vec<Vec<usize>> = Vec::new();
        for _ in 0..take {
            let Some(gp) = Gp::fit(&xs, &ys) else { break };
            let Some((k, _)) = cands
                .iter()
                .enumerate()
                .filter(|(_, c)| !picks.contains(c))
                .map(|(k, c)| (k, gp.expected_improvement(&unit(axes, c), best)))
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            else {
                break;
            };
            let c = cands[k].clone();
            // Believe the prediction for now, so the next pick goes elsewhere.
            let u = unit(axes, &c);
            let (mu, _) = gp.predict(&u);
            xs.push(u);
            ys.push(mu);
            best = best.max(mu);
            picks.push(c);
        }
        if picks.is_empty() {
            break;
        }
        for p in &picks {
            seen.insert(p.clone());
        }
        let batch: Vec<Vec<PValue>> = picks.iter().map(|p| pick(axes, p)).collect();
        tried.extend(picks.into_iter().zip(run_batch(setup, bars, run, batch, progress)));
    }
    tried.into_iter().map(|(_, t)| t).collect()
}

/// The best score for each pair of values of two parameters (the others
/// free): the optimizer's heatmap.
#[derive(Clone, Debug, PartialEq)]
pub struct Heat {
    pub xs: Vec<PValue>,
    pub ys: Vec<PValue>,
    /// `cells[y][x]`; `NaN` where nothing was tried.
    pub cells: Vec<Vec<f64>>,
}

pub fn heatmap(setup: &Setup, trials: &[Trial], px: usize, py: usize) -> Option<Heat> {
    let xs = setup.params.get(px)?.range.values();
    let ys = setup.params.get(py)?.range.values();
    let mut cells = vec![vec![f64::NAN; xs.len()]; ys.len()];
    for t in trials.iter().filter(|t| !t.score.is_nan()) {
        let (Some(x), Some(y)) = (
            xs.iter().position(|v| Some(v) == t.values.get(px)),
            ys.iter().position(|v| Some(v) == t.values.get(py)),
        ) else {
            continue;
        };
        let c = &mut cells[y][x];
        if c.is_nan() || t.score > *c {
            *c = t.score;
        }
    }
    Some(Heat { xs, ys, cells })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::quant::Range;
    use crate::{Broker, Order, OrderKind, Props};

    pub fn bars(n: usize) -> Vec<Bar> {
        // A wavy uptrend: MA crosses make and lose money.
        (0..n)
            .map(|i| {
                let x = i as f64;
                let c = 100.0 + x * 0.05 + (x / 7.0).sin() * 4.0 + (x / 23.0).cos() * 6.0;
                Bar {
                    time: i as i64 * 86_400_000,
                    open: c - 0.3,
                    high: c + 1.0,
                    low: c - 1.0,
                    close: c,
                    volume: 1000.0,
                }
            })
            .collect()
    }

    fn sma(bars: &[Bar], i: usize, n: usize) -> Option<f64> {
        (i + 1 >= n).then(|| bars[i + 1 - n..=i].iter().map(|b| b.close).sum::<f64>() / n as f64)
    }

    /// A long-only moving-average cross, driven straight at the broker.
    pub fn ma_cross(values: &[PValue], bars: &[Bar]) -> Option<Broker> {
        let n = |k: usize| match values.get(k) {
            Some(PValue::Int(v)) if *v > 0 => Some(*v as usize),
            _ => None,
        };
        let (fast, slow) = (n(0)?, n(1)?);
        let mut b = Broker::new(Props {
            initial_capital: 10_000.0,
            ..Props::default()
        });
        for (i, bar) in bars.iter().enumerate() {
            b.open_bar(i, bar);
            if i > 0
                && let (Some(f0), Some(s0), Some(f1), Some(s1)) = (
                    sma(bars, i - 1, fast),
                    sma(bars, i - 1, slow),
                    sma(bars, i, fast),
                    sma(bars, i, slow),
                )
            {
                if f0 <= s0 && f1 > s1 {
                    b.submit(Order::market("L", OrderKind::Entry, true));
                } else if f0 >= s0 && f1 < s1 {
                    b.submit(Order::market("L", OrderKind::Close, true));
                }
            }
            b.close_bar(bar);
        }
        Some(b)
    }

    fn setup(method: Method) -> Setup {
        Setup {
            params: vec![
                Param {
                    id: "fast".into(),
                    title: "Fast".into(),
                    range: Range::Int {
                        from: 2,
                        to: 20,
                        step: 1,
                    },
                },
                Param {
                    id: "slow".into(),
                    title: "Slow".into(),
                    range: Range::Int {
                        from: 10,
                        to: 60,
                        step: 2,
                    },
                },
            ],
            method,
            objective: Objective::NetProfit,
            min_trades: 1,
            seed: 42,
        }
    }

    #[test]
    fn grid_tries_everything_and_sorts() {
        let bars = bars(600);
        let s = setup(Method::Grid);
        let p = Progress::default();
        let out = optimize(&s, &bars, &ma_cross, &p).expect("runs");
        assert_eq!(out.trials.len(), 19 * 26);
        assert_eq!(p.done.load(std::sync::atomic::Ordering::Relaxed), 19 * 26);
        let scores: Vec<f64> = out.trials.iter().map(|t| t.score).filter(|s| !s.is_nan()).collect();
        assert!(scores.windows(2).all(|w| w[0] >= w[1]));
        // Deterministic: the same best every time.
        let again = optimize(&s, &bars, &ma_cross, &Progress::default()).expect("runs");
        assert_eq!(out.best().map(|t| &t.values), again.best().map(|t| &t.values));
        let heat = heatmap(&s, &out.trials, 0, 1).expect("heat");
        assert_eq!(heat.xs.len(), 19);
        assert_eq!(heat.ys.len(), 26);
        let top = heat
            .cells
            .iter()
            .flatten()
            .cloned()
            .filter(|v| !v.is_nan())
            .fold(f64::NEG_INFINITY, f64::max);
        assert_eq!(top, out.best().expect("best").score);
    }

    #[test]
    fn bayesian_gets_close_to_the_grid_best_in_fewer_runs() {
        let bars = bars(600);
        let grid = optimize(&setup(Method::Grid), &bars, &ma_cross, &Progress::default()).expect("grid");
        let best = grid.best().expect("best").score;
        let worst = grid
            .trials
            .iter()
            .map(|t| t.score)
            .filter(|s| !s.is_nan())
            .fold(f64::INFINITY, f64::min);
        let bo = optimize(
            &setup(Method::Bayesian { trials: 80 }),
            &bars,
            &ma_cross,
            &Progress::default(),
        )
        .expect("bo");
        assert_eq!(bo.trials.len(), 80);
        let got = bo.best().expect("best").score;
        // Within the top tenth of the range of outcomes, with 1/6 of the runs.
        assert!(
            got >= best - (best - worst) * 0.1,
            "bayes {got}, grid best {best}, worst {worst}"
        );
        let distinct: HashSet<String> = bo.trials.iter().map(|t| format!("{:?}", t.values)).collect();
        assert_eq!(distinct.len(), 80);
        let rnd = optimize(
            &setup(Method::Random { trials: 30 }),
            &bars,
            &ma_cross,
            &Progress::default(),
        )
        .expect("random");
        assert_eq!(rnd.trials.len(), 30);
    }

    #[test]
    fn plain_errors() {
        let mut s = setup(Method::Grid);
        s.params[0].range = Range::Int {
            from: 1,
            to: 100_000,
            step: 1,
        };
        let e = optimize(&s, &bars(10), &ma_cross, &Progress::default()).unwrap_err();
        assert!(e.contains("more than the grid"), "{e}");
        s.params.clear();
        assert!(optimize(&s, &bars(10), &ma_cross, &Progress::default()).is_err());
    }

    #[test]
    fn too_few_trades_are_unscored() {
        let mut s = setup(Method::Grid);
        s.min_trades = 1_000;
        let out = optimize(&s, &bars(300), &ma_cross, &Progress::default()).expect("runs");
        assert!(out.best().is_none());
    }
}

//! Portfolio backtests: one strategy on several symbols, each with its own
//! slice of the capital, combined into one equity curve on the union of
//! their bar times (a symbol's equity holds between its bars, and before
//! its first bar it's just its capital).

use super::Summary;
use crate::Broker;
use crate::report::{excursions, monthly_returns_at, sharpe_sortino};
use opentern_core::Bar;
use serde::{Deserialize, Serialize};

/// One symbol's run.
pub struct Leg<'a> {
    pub symbol: String,
    pub bars: &'a [Bar],
    pub broker: &'a Broker,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegRow {
    pub symbol: String,
    pub capital: f64,
    pub summary: Summary,
    /// Buy and hold on this symbol over its bars, percent.
    pub buy_hold_pct: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PortfolioResult {
    pub legs: Vec<LegRow>,
    pub equity: Vec<(i64, f64)>,
    pub initial_capital: f64,
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
    /// Equal-weight buy and hold of every symbol, percent.
    pub buy_hold_pct: f64,
    /// Correlation of the legs' equity changes, `corr[i][j]`.
    pub correlation: Vec<Vec<f64>>,
}

fn pearson(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    if n < 3 {
        return f64::NAN;
    }
    let (ma, mb) = (
        a[..n].iter().sum::<f64>() / n as f64,
        b[..n].iter().sum::<f64>() / n as f64,
    );
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (x, y) = (a[i] - ma, b[i] - mb);
        sab += x * y;
        saa += x * x;
        sbb += y * y;
    }
    if saa > 0.0 && sbb > 0.0 {
        sab / (saa * sbb).sqrt()
    } else {
        f64::NAN
    }
}

pub fn combine(legs: &[Leg]) -> PortfolioResult {
    let mut times: Vec<i64> = legs.iter().flat_map(|l| l.bars.iter().map(|b| b.time)).collect();
    times.sort_unstable();
    times.dedup();
    let cap: f64 = legs.iter().map(|l| l.broker.props.initial_capital).sum();
    // Each leg's equity on the union grid.
    let grids: Vec<Vec<f64>> = legs
        .iter()
        .map(|l| {
            let n = l.bars.len().min(l.broker.equity.len());
            let mut k = 0;
            let mut cur = l.broker.props.initial_capital;
            times
                .iter()
                .map(|&t| {
                    while k < n && l.bars[k].time <= t {
                        cur = l.broker.equity[k];
                        k += 1;
                    }
                    cur
                })
                .collect()
        })
        .collect();
    let total: Vec<f64> = (0..times.len()).map(|i| grids.iter().map(|g| g[i]).sum()).collect();
    let rows: Vec<LegRow> = legs
        .iter()
        .map(|l| {
            let n = l.bars.len().min(l.broker.equity.len());
            let bh = match (l.bars.first(), l.bars.get(n.wrapping_sub(1))) {
                (Some(a), Some(b)) if a.open > 0.0 => (b.close / a.open - 1.0) * 100.0,
                _ => 0.0,
            };
            LegRow {
                symbol: l.symbol.clone(),
                capital: l.broker.props.initial_capital,
                summary: Summary::of_run(l.broker, &l.bars[..n]),
                buy_hold_pct: bh,
            }
        })
        .collect();
    let pnls: Vec<f64> = legs
        .iter()
        .flat_map(|l| l.broker.trades.iter().map(|t| t.pnl))
        .collect();
    let gp: f64 = pnls.iter().filter(|p| **p > 0.0).sum();
    let gl: f64 = -pnls.iter().filter(|p| **p < 0.0).sum::<f64>();
    let final_eq = total.last().copied().unwrap_or(cap);
    let (dd, _, _) = excursions(&total, true);
    let mut peak = f64::NEG_INFINITY;
    let mut dd_pct = 0.0f64;
    for &v in &total {
        peak = peak.max(v);
        if peak > 0.0 {
            dd_pct = dd_pct.max((peak - v) / peak * 100.0);
        }
    }
    let rets = monthly_returns_at(times.iter().copied(), &total);
    let (sharpe, sortino) = sharpe_sortino(&rets);
    let days = match (times.first(), times.last()) {
        (Some(a), Some(b)) => (b - a) as f64 / 86_400_000.0,
        _ => 0.0,
    };
    let changes: Vec<Vec<f64>> = grids
        .iter()
        .map(|g| g.windows(2).map(|w| w[1] - w[0]).collect())
        .collect();
    let correlation = (0..legs.len())
        .map(|i| {
            (0..legs.len())
                .map(|j| if i == j { 1.0 } else { pearson(&changes[i], &changes[j]) })
                .collect()
        })
        .collect();
    PortfolioResult {
        buy_hold_pct: if rows.is_empty() {
            0.0
        } else {
            rows.iter().map(|r| r.buy_hold_pct).sum::<f64>() / rows.len() as f64
        },
        legs: rows,
        equity: times.iter().copied().zip(total.iter().copied()).collect(),
        initial_capital: cap,
        net_profit: final_eq - cap,
        net_profit_pct: if cap > 0.0 { (final_eq - cap) / cap * 100.0 } else { 0.0 },
        total_trades: pnls.len(),
        percent_profitable: if pnls.is_empty() {
            0.0
        } else {
            pnls.iter().filter(|p| **p > 0.0).count() as f64 / pnls.len() as f64 * 100.0
        },
        profit_factor: if gl > 0.0 { gp / gl } else { f64::NAN },
        max_drawdown: dd,
        max_drawdown_pct: dd_pct,
        sharpe,
        sortino,
        cagr: if days > 0.0 && final_eq > 0.0 && cap > 0.0 {
            ((final_eq / cap).powf(365.0 / days) - 1.0) * 100.0
        } else {
            0.0
        },
        correlation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quant::PValue;
    use crate::quant::optimize::tests::{bars, ma_cross};

    #[test]
    fn legs_add_up() {
        let a = bars(400);
        // A second symbol on half the dates, shifted half a day.
        let b: Vec<Bar> = bars(300)
            .into_iter()
            .map(|mut x| {
                x.time += 43_200_000 + 100 * 86_400_000;
                x
            })
            .collect();
        let p = [PValue::Int(5), PValue::Int(20)];
        let ra = ma_cross(&p, &a).expect("a");
        let rb = ma_cross(&p, &b).expect("b");
        let r = combine(&[
            Leg {
                symbol: "A".into(),
                bars: &a,
                broker: &ra,
            },
            Leg {
                symbol: "B".into(),
                bars: &b,
                broker: &rb,
            },
        ]);
        assert_eq!(r.initial_capital, 20_000.0);
        assert_eq!(r.equity.len(), 700);
        let sum = (ra.equity.last().expect("a") - 10_000.0) + (rb.equity.last().expect("b") - 10_000.0);
        assert!((r.net_profit - sum).abs() < 1e-6);
        assert_eq!(r.total_trades, ra.trades.len() + rb.trades.len());
        assert_eq!(r.legs.len(), 2);
        assert_eq!(r.correlation[0][0], 1.0);
        assert!(r.correlation[0][1].abs() <= 1.0 || r.correlation[0][1].is_nan());
        assert!(r.equity.windows(2).all(|w| w[0].0 < w[1].0));
    }
}

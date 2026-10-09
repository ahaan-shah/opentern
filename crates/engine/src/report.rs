//! The Strategy Report: TradingView's metrics, computed from a broker run.
//! Formulas, and which ones still need checking against TradingView, are
//! in `docs/metrics.md`.

use crate::broker::{Broker, Trade};
use opentern_core::Bar;
use serde::Serialize;

/// Metrics for one side (all, long, short) of the trades.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Side {
    pub net_profit: f64,
    pub gross_profit: f64,
    pub gross_loss: f64,
    pub profit_factor: f64,
    pub commission: f64,
    pub total_trades: usize,
    pub winners: usize,
    pub losers: usize,
    pub even: usize,
    pub percent_profitable: f64,
    pub avg_trade: f64,
    pub avg_trade_pct: f64,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub win_loss_ratio: f64,
    pub largest_win: f64,
    pub largest_win_pct_of_gross: f64,
    pub largest_loss: f64,
    pub largest_loss_pct_of_gross: f64,
    pub avg_bars: f64,
    pub avg_bars_win: f64,
    pub avg_bars_loss: f64,
    pub max_contracts: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Report {
    pub initial_capital: f64,
    pub all: Side,
    pub long: Side,
    pub short: Side,
    pub open_pnl: f64,
    pub open_trades: usize,
    pub buy_hold_pnl: f64,
    pub buy_hold_pct: f64,
    pub outperformance: f64,
    pub sharpe: f64,
    pub sortino: f64,
    pub cagr: f64,
    pub return_on_capital: f64,
    pub account_size_required: f64,
    pub return_on_account_size: f64,
    pub net_profit_pct_of_largest_loss: f64,
    pub avg_margin_used: f64,
    pub max_margin_used: f64,
    pub margin_efficiency: f64,
    pub margin_calls: usize,
    pub max_drawdown: f64,
    pub max_drawdown_pct: f64,
    pub max_drawdown_intrabar: f64,
    pub max_drawdown_intrabar_pct: f64,
    pub avg_drawdown: f64,
    pub avg_drawdown_bars: f64,
    pub max_runup: f64,
    pub max_runup_pct: f64,
    pub max_runup_intrabar: f64,
    pub max_runup_intrabar_pct: f64,
    pub avg_runup: f64,
    pub avg_runup_bars: f64,
    pub return_of_max_drawdown: f64,
    pub liquidated_volume: f64,
    pub largest_liquidated_volume: f64,
}

fn side(trades: &[&Trade], position: &[f64], filter: impl Fn(f64) -> bool) -> Side {
    let n = trades.len();
    let pnl: Vec<f64> = trades.iter().map(|t| t.pnl).collect();
    let gp: f64 = pnl.iter().filter(|p| **p > 0.0).sum();
    let gl: f64 = -pnl.iter().filter(|p| **p < 0.0).sum::<f64>();
    let wins: Vec<&&Trade> = trades.iter().filter(|t| t.pnl > 0.0).collect();
    let losses: Vec<&&Trade> = trades.iter().filter(|t| t.pnl < 0.0).collect();
    let avg = |v: &[f64]| {
        if v.is_empty() {
            0.0
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    };
    let bars = |ts: &[&&Trade]| avg(&ts.iter().map(|t| (t.exit_bar - t.entry_bar) as f64).collect::<Vec<_>>());
    let avg_win = avg(&wins.iter().map(|t| t.pnl).collect::<Vec<_>>());
    let avg_loss = avg(&losses.iter().map(|t| t.pnl).collect::<Vec<_>>());
    let largest_win = pnl.iter().cloned().fold(0.0, f64::max);
    let largest_loss = pnl.iter().cloned().fold(0.0, f64::min);
    let all: Vec<&&Trade> = trades.iter().collect();
    Side {
        net_profit: pnl.iter().sum(),
        gross_profit: gp,
        gross_loss: gl,
        profit_factor: if gl > 0.0 { gp / gl } else { f64::NAN },
        commission: trades.iter().map(|t| t.commission).sum(),
        total_trades: n,
        winners: wins.len(),
        losers: losses.len(),
        even: n - wins.len() - losses.len(),
        percent_profitable: if n > 0 {
            wins.len() as f64 / n as f64 * 100.0
        } else {
            0.0
        },
        avg_trade: avg(&pnl),
        avg_trade_pct: avg(&trades.iter().map(|t| t.pnl_pct).collect::<Vec<_>>()),
        avg_win,
        avg_loss,
        win_loss_ratio: if avg_loss != 0.0 {
            avg_win / avg_loss.abs()
        } else {
            f64::NAN
        },
        largest_win,
        largest_win_pct_of_gross: if gp > 0.0 { largest_win / gp * 100.0 } else { 0.0 },
        largest_loss,
        largest_loss_pct_of_gross: if gl > 0.0 { largest_loss.abs() / gl * 100.0 } else { 0.0 },
        avg_bars: bars(&all),
        avg_bars_win: bars(&wins),
        avg_bars_loss: bars(&losses),
        max_contracts: position
            .iter()
            .filter(|p| filter(**p))
            .map(|p| p.abs())
            .fold(0.0, f64::max),
    }
}

/// Drawdowns (peak to trough) or run-ups (trough to peak) of a curve:
/// (largest, average, average length in bars).
fn excursions(curve: &[f64], down: bool) -> (f64, f64, f64) {
    let mut best = 0.0f64;
    let mut sizes = Vec::new();
    let mut lens = Vec::new();
    let mut anchor = f64::NAN;
    let mut start = 0usize;
    let mut cur = 0.0f64;
    for (i, &v) in curve.iter().enumerate() {
        let new_anchor = anchor.is_nan() || if down { v >= anchor } else { v <= anchor };
        if new_anchor {
            if cur > 0.0 {
                sizes.push(cur);
                lens.push((i - start) as f64);
            }
            anchor = v;
            start = i;
            cur = 0.0;
        } else {
            let d = if down { anchor - v } else { v - anchor };
            cur = cur.max(d);
            best = best.max(d);
        }
    }
    if cur > 0.0 {
        sizes.push(cur);
        lens.push((curve.len() - start) as f64);
    }
    let avg = |v: &[f64]| {
        if v.is_empty() {
            0.0
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    };
    (best, avg(&sizes), avg(&lens))
}

/// Monthly returns of the equity curve (month ends in UTC).
fn monthly_returns(bars: &[Bar], equity: &[f64]) -> Vec<f64> {
    let month = |t: i64| {
        jiff::Timestamp::from_millisecond(t)
            .map(|x| {
                let d = x.to_zoned(jiff::tz::TimeZone::UTC).date();
                d.year() as i32 * 12 + d.month() as i32
            })
            .unwrap_or(0)
    };
    let mut ends: Vec<f64> = Vec::new();
    let mut last_month = None;
    for (b, e) in bars.iter().zip(equity) {
        let m = month(b.time);
        if Some(m) != last_month {
            ends.push(*e);
            last_month = Some(m);
        } else if let Some(x) = ends.last_mut() {
            *x = *e;
        }
    }
    ends.windows(2)
        .map(|w| if w[0] != 0.0 { w[1] / w[0] - 1.0 } else { 0.0 })
        .collect()
}

impl Report {
    pub fn new(b: &Broker, bars: &[Bar]) -> Report {
        let cap = b.props.initial_capital;
        let all: Vec<&Trade> = b.trades.iter().collect();
        let longs: Vec<&Trade> = b.trades.iter().filter(|t| t.long).collect();
        let shorts: Vec<&Trade> = b.trades.iter().filter(|t| !t.long).collect();
        let n = bars.len().min(b.equity.len());
        let last_close = bars.get(n.wrapping_sub(1)).map(|x| x.close).unwrap_or(0.0);
        let mut r = Report {
            initial_capital: cap,
            all: side(&all, &b.position, |_| true),
            long: side(&longs, &b.position, |p| p > 0.0),
            short: side(&shorts, &b.position, |p| p < 0.0),
            open_pnl: b.open_profit(last_close),
            open_trades: b.lots.len(),
            ..Default::default()
        };
        if n == 0 {
            return r;
        }
        // Buy and hold: the whole capital at the first bar's open, to the
        // last close.
        let first = bars[0].open;
        if first > 0.0 {
            r.buy_hold_pct = (last_close / first - 1.0) * 100.0;
            r.buy_hold_pnl = cap * (last_close / first - 1.0);
        }
        r.outperformance = r.all.net_profit - r.buy_hold_pnl;
        // Sharpe and Sortino on monthly returns, risk-free rate 2% a year.
        let rets = monthly_returns(&bars[..n], &b.equity[..n]);
        let rf = 0.02 / 12.0;
        if rets.len() >= 2 {
            let m = rets.iter().sum::<f64>() / rets.len() as f64;
            let sd = (rets.iter().map(|x| (x - m).powi(2)).sum::<f64>() / rets.len() as f64).sqrt();
            let down = (rets.iter().map(|x| (x - rf).min(0.0).powi(2)).sum::<f64>() / rets.len() as f64).sqrt();
            r.sharpe = if sd > 0.0 { (m - rf) / sd } else { f64::NAN };
            r.sortino = if down > 0.0 { (m - rf) / down } else { f64::NAN };
        } else {
            r.sharpe = f64::NAN;
            r.sortino = f64::NAN;
        }
        let final_eq = b.equity[n - 1];
        let days = (bars[n - 1].time - bars[0].time) as f64 / 86_400_000.0;
        r.cagr = if days > 0.0 && final_eq > 0.0 && cap > 0.0 {
            ((final_eq / cap).powf(365.0 / days) - 1.0) * 100.0
        } else {
            0.0
        };
        r.return_on_capital = r.all.net_profit / cap * 100.0;
        let (dd, avg_dd, dd_len) = excursions(&b.equity[..n], true);
        let (ru, avg_ru, ru_len) = excursions(&b.equity[..n], false);
        // Intrabar: the worst point of each bar against the best before it.
        let mut peak = f64::NAN;
        let mut dd_ib = 0.0f64;
        let mut trough = f64::NAN;
        let mut ru_ib = 0.0f64;
        for i in 0..n {
            let (hi, lo) = (b.equity_hi[i], b.equity_lo[i]);
            if !peak.is_nan() {
                dd_ib = dd_ib.max(peak - lo);
            }
            if !trough.is_nan() {
                ru_ib = ru_ib.max(hi - trough);
            }
            peak = if peak.is_nan() {
                hi.max(b.equity[i])
            } else {
                peak.max(hi)
            };
            trough = if trough.is_nan() {
                lo.min(b.equity[i])
            } else {
                trough.min(lo)
            };
        }
        r.max_drawdown = dd;
        r.max_drawdown_pct = dd / cap * 100.0;
        r.max_drawdown_intrabar = dd_ib.max(dd);
        r.max_drawdown_intrabar_pct = r.max_drawdown_intrabar / cap * 100.0;
        r.avg_drawdown = avg_dd;
        r.avg_drawdown_bars = dd_len;
        r.max_runup = ru;
        r.max_runup_pct = ru / cap * 100.0;
        r.max_runup_intrabar = ru_ib.max(ru);
        r.max_runup_intrabar_pct = r.max_runup_intrabar / cap * 100.0;
        r.avg_runup = avg_ru;
        r.avg_runup_bars = ru_len;
        r.return_of_max_drawdown = if dd > 0.0 { r.all.net_profit / dd } else { f64::NAN };
        // Margin: position value × margin %, on bars in the market.
        let margins: Vec<f64> = (0..n)
            .map(|i| {
                let p = b.position[i];
                let pct = if p > 0.0 {
                    b.props.margin_long
                } else {
                    b.props.margin_short
                };
                p.abs() * bars[i].close * pct / 100.0
            })
            .collect();
        let used: Vec<f64> = margins.iter().cloned().filter(|m| *m > 0.0).collect();
        r.avg_margin_used = if used.is_empty() {
            0.0
        } else {
            used.iter().sum::<f64>() / used.len() as f64
        };
        r.max_margin_used = used.iter().cloned().fold(0.0, f64::max);
        r.margin_efficiency = if r.max_margin_used > 0.0 {
            r.all.net_profit / r.max_margin_used * 100.0
        } else {
            0.0
        };
        r.account_size_required = r.max_drawdown_intrabar + r.max_margin_used.min(cap);
        r.return_on_account_size = if r.account_size_required > 0.0 {
            r.all.net_profit / r.account_size_required * 100.0
        } else {
            0.0
        };
        r.net_profit_pct_of_largest_loss = if r.all.largest_loss < 0.0 {
            r.all.net_profit / r.all.largest_loss.abs() * 100.0
        } else {
            f64::NAN
        };
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::{Order, OrderKind, Props};

    fn bar(day: i64, o: f64, h: f64, l: f64, c: f64) -> Bar {
        Bar {
            time: day * 86_400_000,
            open: o,
            high: h,
            low: l,
            close: c,
            volume: 1.0,
        }
    }

    #[test]
    fn report_on_a_known_run() {
        // Two long trades: +10 and −4, one contract each.
        let bars = vec![
            bar(0, 100.0, 100.0, 100.0, 100.0),
            bar(1, 100.0, 105.0, 98.0, 104.0),
            bar(2, 110.0, 110.0, 110.0, 110.0),
            bar(3, 110.0, 110.0, 110.0, 110.0),
            bar(4, 106.0, 106.0, 106.0, 106.0),
            bar(5, 106.0, 106.0, 106.0, 106.0),
        ];
        let mut b = Broker::new(Props {
            initial_capital: 1000.0,
            ..Props::default()
        });
        for (i, x) in bars.iter().enumerate() {
            b.open_bar(i, x);
            match i {
                0 | 2 => b.submit(Order::market("L", OrderKind::Entry, true)),
                1 | 3 => b.submit(Order::market("L", OrderKind::Close, true)),
                _ => {}
            }
            b.close_bar(x);
        }
        let r = Report::new(&b, &bars);
        assert_eq!(r.all.total_trades, 2);
        assert_eq!(r.all.net_profit, 6.0);
        assert_eq!(r.all.gross_profit, 10.0);
        assert_eq!(r.all.gross_loss, 4.0);
        assert_eq!(r.all.profit_factor, 2.5);
        assert_eq!(r.all.percent_profitable, 50.0);
        assert_eq!(r.all.avg_trade, 3.0);
        assert_eq!(r.all.largest_win, 10.0);
        assert_eq!(r.all.largest_loss, -4.0);
        assert_eq!(r.all.win_loss_ratio, 2.5);
        assert_eq!(r.long.total_trades, 2);
        assert_eq!(r.short.total_trades, 0);
        assert_eq!(r.all.max_contracts, 1.0);
        // Equity: 1000, 1004, 1010, 1010, 1006, 1006 → close-to-close DD 4.
        assert_eq!(b.equity, vec![1000.0, 1004.0, 1010.0, 1010.0, 1006.0, 1006.0]);
        assert_eq!(r.max_drawdown, 4.0);
        assert_eq!(r.max_runup, 10.0);
        // Intrabar, bar 1 dipped to 98 with the position open: 1000 → 998.
        assert_eq!(r.max_drawdown_intrabar, 4.0);
        assert!(r.max_runup_intrabar >= 10.0);
        assert_eq!(r.return_on_capital, 0.6);
        assert!((r.buy_hold_pnl - 60.0).abs() < 1e-9);
        assert!((r.outperformance + 54.0).abs() < 1e-9);
        assert_eq!(r.net_profit_pct_of_largest_loss, 150.0);
    }

    #[test]
    fn excursions_find_peak_to_trough() {
        let (dd, avg, len) = excursions(&[10.0, 8.0, 9.0, 12.0, 7.0, 13.0], true);
        assert_eq!(dd, 5.0);
        assert_eq!(avg, (2.0 + 5.0) / 2.0);
        // Peak to recovery: 3 bars, then 2.
        assert_eq!(len, 2.5);
        let (ru, _, _) = excursions(&[10.0, 8.0, 9.0, 12.0, 7.0, 13.0], false);
        assert_eq!(ru, 6.0);
    }
}

//! CSV exports of the quant tools' results (JSON is plain `serde_json` on
//! the result types). Numbers are written in full precision, times as
//! ISO 8601 UTC.

use super::montecarlo::McResult;
use super::optimize::{Outcome, Setup};
use super::portfolio::PortfolioResult;
use super::walkforward::WalkResult;

fn cell(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn num(v: f64) -> String {
    if v.is_finite() { format!("{v}") } else { String::new() }
}

fn time(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|t| t.to_string())
        .unwrap_or_default()
}

const SUMMARY_HEAD: &str = "Net profit,Net profit %,Total trades,Percent profitable,Profit factor,Max drawdown,Max drawdown %,Sharpe,Sortino,CAGR %,Avg trade";

fn summary(s: &super::Summary) -> String {
    [
        num(s.net_profit),
        num(s.net_profit_pct),
        s.total_trades.to_string(),
        num(s.percent_profitable),
        num(s.profit_factor),
        num(s.max_drawdown),
        num(s.max_drawdown_pct),
        num(s.sharpe),
        num(s.sortino),
        num(s.cagr),
        num(s.avg_trade),
    ]
    .join(",")
}

pub fn optimizer_csv(setup: &Setup, out: &Outcome) -> String {
    let mut s = String::from("Rank,");
    for p in &setup.params {
        s.push_str(&cell(&p.title));
        s.push(',');
    }
    s.push_str(&format!("Score ({}),{SUMMARY_HEAD}\n", setup.objective.name()));
    for (k, t) in out.trials.iter().enumerate() {
        s.push_str(&format!("{},", k + 1));
        for v in &t.values {
            s.push_str(&cell(&v.to_string()));
            s.push(',');
        }
        s.push_str(&format!("{},{}\n", num(t.score), summary(&t.summary)));
    }
    s
}

pub fn walk_forward_csv(setup: &Setup, r: &WalkResult) -> String {
    let mut s = String::from("Window,In-sample from,In-sample to,Out-of-sample from,Out-of-sample to,");
    for p in &setup.params {
        s.push_str(&cell(&p.title));
        s.push(',');
    }
    s.push_str("In-sample net profit,In-sample trades,Out-of-sample net profit,Out-of-sample trades,Out-of-sample winners,Out-of-sample max drawdown\n");
    for (k, w) in r.windows.iter().enumerate() {
        s.push_str(&format!(
            "{},{},{},{},{},",
            k + 1,
            time(w.in_time.0),
            time(w.in_time.1),
            time(w.out_time.0),
            time(w.out_time.1)
        ));
        for i in 0..setup.params.len() {
            let v = w.values.as_ref().and_then(|v| v.get(i)).map(|v| v.to_string());
            s.push_str(&cell(&v.unwrap_or_default()));
            s.push(',');
        }
        s.push_str(&format!(
            "{},{},{},{},{},{}\n",
            num(w.in_sample.net_profit),
            w.in_sample.total_trades,
            num(w.out_net),
            w.out_trades,
            w.out_winners,
            num(w.out_max_drawdown)
        ));
    }
    s
}

pub fn monte_carlo_csv(r: &McResult) -> String {
    let mut s = String::from("Measure,5th percentile,25th percentile,Median,75th percentile,95th percentile\n");
    for (name, sp) in [
        ("Net profit", &r.net_profit),
        ("Max drawdown", &r.max_drawdown),
        ("Max drawdown %", &r.max_drawdown_pct),
        ("Longest losing streak", &r.longest_losing_streak),
    ] {
        s.push_str(&format!(
            "{name},{},{},{},{},{}\n",
            num(sp.p5),
            num(sp.p25),
            num(sp.p50),
            num(sp.p75),
            num(sp.p95)
        ));
    }
    s.push_str(&format!(
        "\nRuns,{}\nTrades,{}\nChance of a loss %,{}\nChance of ruin %,{}\n\nTrade,Equity 5th,Equity median,Equity 95th\n",
        r.runs,
        r.trades,
        num(r.prob_loss),
        num(r.prob_ruin)
    ));
    for (k, (a, b, c)) in r.bands.iter().enumerate() {
        s.push_str(&format!("{k},{},{},{}\n", num(*a), num(*b), num(*c)));
    }
    s
}

pub fn portfolio_csv(r: &PortfolioResult) -> String {
    let mut s = format!("Symbol,Capital,{SUMMARY_HEAD},Buy and hold %\n");
    for l in &r.legs {
        s.push_str(&format!(
            "{},{},{},{}\n",
            cell(&l.symbol),
            num(l.capital),
            summary(&l.summary),
            num(l.buy_hold_pct)
        ));
    }
    s.push_str(&format!(
        "Portfolio,{},{},{},{},{},{},{},{},{},{},{},,{}\n\nTime,Equity\n",
        num(r.initial_capital),
        num(r.net_profit),
        num(r.net_profit_pct),
        r.total_trades,
        num(r.percent_profitable),
        num(r.profit_factor),
        num(r.max_drawdown),
        num(r.max_drawdown_pct),
        num(r.sharpe),
        num(r.sortino),
        num(r.cagr),
        num(r.buy_hold_pct)
    ));
    for (t, e) in &r.equity {
        s.push_str(&format!("{},{}\n", time(*t), num(*e)));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quant::optimize::tests::{bars, ma_cross};
    use crate::quant::optimize::{Method, optimize};
    use crate::quant::{Objective, Param, Progress, Range};

    #[test]
    fn optimizer_csv_has_a_row_per_trial() {
        let setup = Setup {
            params: vec![Param {
                id: "fast".into(),
                title: "Fast, length".into(),
                range: Range::Int {
                    from: 3,
                    to: 5,
                    step: 1,
                },
            }],
            method: Method::Grid,
            objective: Objective::NetProfit,
            min_trades: 0,
            seed: 0,
        };
        // Slow fixed at 20 through the runner.
        let run = |v: &[crate::quant::PValue], b: &[opentern_core::Bar]| {
            ma_cross(&[v[0].clone(), crate::quant::PValue::Int(20)], b)
        };
        let out = optimize(&setup, &bars(200), &run, &Progress::default()).expect("runs");
        let csv = optimizer_csv(&setup, &out);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].starts_with("Rank,\"Fast, length\",Score (Net profit),Net profit"));
        assert!(lines[1].starts_with("1,"));
    }
}

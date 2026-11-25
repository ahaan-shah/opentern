//! The bugs found in the Pine engine (`crates/pine/src/hunt.rs`) that these
//! languages had too. Each script runs in Luau and in JavaScript.

use opentern_core::{Bar, Timeframe, Unit};
use opentern_engine::Broker;
use opentern_script::inproc::{Lang, compile};
use opentern_script::{ScriptStudy, SymInfo};
use opentern_series::study::{Inputs, Instance, Study, Value};
use std::sync::Arc;

const H1: Timeframe = Timeframe::new(1, Unit::Hour);
const D1: Timeframe = Timeframe::new(1, Unit::Day);

fn ms(s: &str) -> i64 {
    s.parse::<jiff::Timestamp>().unwrap().as_millisecond()
}

fn bars(n: usize) -> Vec<Bar> {
    let t0 = ms("2024-01-01T00:00:00Z");
    (0..n)
        .map(|i| {
            let x = i as f64;
            let c = 100.0 + (x * 0.7).sin() * 3.0 + x * 0.1;
            Bar {
                time: t0 + i as i64 * 3_600_000,
                open: c - 0.5,
                high: c + 1.0,
                low: c - 1.0,
                close: c,
                volume: 100.0 + (x * 1.3).cos() * 50.0,
            }
        })
        .collect()
}

/// The same body in Luau and JavaScript: `on_bar`'s statements are written
/// so both read them.
fn both(head: &str, body: &str) -> [(Arc<dyn Lang>, String); 2] {
    [
        (
            Arc::new(opentern_script::luau::Luau),
            format!("{head}\nfunction on_bar(bar)\n{body}\nend\n"),
        ),
        (
            Arc::new(opentern_script::js::Js { ts: false }),
            format!("{}\nfunction onBar(bar) {{\n{body}\n}}\n", head.replace(" = {", ": {")).replace("local ", "let "),
        ),
    ]
}

fn run(
    (lang, src): &(Arc<dyn Lang>, String),
    inputs: &Inputs,
    sym: SymInfo,
    b: &[Bar],
    tf: Timeframe,
) -> (Vec<Vec<f64>>, Instance) {
    let c = compile(lang.clone(), src, "t", sym.clone()).unwrap_or_else(|e| panic!("{e}\n{src}"));
    let tz = jiff::tz::TimeZone::get(&sym.timezone).unwrap();
    let mut inst = Instance::with_factory(c.meta(), inputs, c.factory());
    // One bar more, so every bar asked for is closed.
    let mut all = b.to_vec();
    all.push(Bar {
        time: b[b.len() - 1].time + 1,
        ..b[b.len() - 1]
    });
    inst.recompute(&all, &opentern_series::study::sessions(&all, tf, &tz), tf);
    let st = inst.state().and_then(|s| s.downcast_ref::<ScriptStudy>()).unwrap();
    assert!(st.error().is_none(), "{:?}\n{src}", st.error());
    let plots = inst.out.plots.iter().map(|p| p[..b.len()].to_vec()).collect();
    (plots, inst)
}

fn broker(inst: &Instance) -> Broker {
    let st = inst.state().and_then(|s| s.downcast_ref::<ScriptStudy>()).unwrap();
    st.broker().unwrap().clone()
}

#[test]
fn lengths_may_change() {
    const CALLS: [&str; 15] = [
        "ta.sma(bar.close, N)",
        "ta.wma(bar.close, N)",
        "ta.vwma(bar.close, N)",
        "ta.hma(bar.close, N)",
        "ta.linreg(bar.close, N, 0)",
        "ta.stdev(bar.close, N)",
        "ta.dev(bar.close, N)",
        "ta.highest(bar.high, N)",
        "ta.lowest(bar.low, N)",
        "ta.change(bar.close, N)",
        "ta.roc(bar.close, N)",
        "ta.stoch(bar.close, bar.high, bar.low, N)",
        "ta.cci(bar.close, N)",
        "ta.mfi(bar.close, N)",
        "ta.prev(bar.close, N)",
    ];
    let b = bars(60);
    let plots = |n: &str| CALLS.map(|c| format!("plot({})", c.replace('N', n))).join("\n");
    for k in 0..2 {
        let make = |n: &str| both("indicator(\"T\")", &plots(n))[k].clone();
        let (got, _) = run(
            &make("(4 + bar.index % 3)"),
            &Inputs::default(),
            SymInfo::default(),
            &b,
            H1,
        );
        let fixed: Vec<_> = (4..7)
            .map(|n| run(&make(&n.to_string()), &Inputs::default(), SymInfo::default(), &b, H1).0)
            .collect();
        for (c, call) in CALLS.iter().enumerate() {
            for i in 14..60 {
                let want = fixed[i % 3][c][i];
                assert!(
                    (got[c][i] - want).abs() < 1e-6,
                    "{call} bar {i}: {} vs {want}",
                    got[c][i]
                );
            }
        }
    }
}

#[test]
fn bands_may_change_width() {
    let b = bars(60);
    let luau = "indicator(\"T\")\nfunction on_bar(bar)\nlocal m, u, l = ta.bb(bar.close, 10, 1 + bar.index % 2)\nplot(u - m)\nend".to_string();
    let (p, _) = run(
        &(Arc::new(opentern_script::luau::Luau) as Arc<dyn Lang>, luau),
        &Inputs::default(),
        SymInfo::default(),
        &b,
        H1,
    );
    assert!(p[0][20..].iter().all(|x| x.is_finite()), "{:?}", p[0]);
}

#[test]
fn a_quantity_of_zero_places_nothing() {
    let b = bars(12);
    for s in both("strategy(\"T\")", "if2\nplot(bar.close)") {
        // `if` is spelt differently in each.
        let luau = s.1.contains("on_bar(");
        let stmt = |q: &str, what: &str, at: u32| {
            if luau {
                format!("if bar.index == {at} then strategy.{what}, {{ qty = {q} }}) end")
            } else {
                format!("if (bar.index === {at}) strategy.{what}, {{ qty: {q} }})")
            }
        };
        let src = |body: String| (s.0.clone(), s.1.replace("if2", &body));
        let (_, i) = run(
            &src(stmt("0", "entry(\"L\", \"long\"", 2)),
            &Inputs::default(),
            SymInfo::default(),
            &b,
            H1,
        );
        assert!(broker(&i).lots.is_empty(), "entry of 0: {:?}", broker(&i).lots);
        let (_, i) = run(
            &src(stmt("0", "order(\"L\", \"long\"", 2)),
            &Inputs::default(),
            SymInfo::default(),
            &b,
            H1,
        );
        assert!(broker(&i).lots.is_empty());
        let two = format!(
            "{}\n{}",
            stmt("2", "entry(\"L\", \"long\"", 2),
            stmt("0", "close(\"L\"", 5)
        );
        let (_, i) = run(&src(two), &Inputs::default(), SymInfo::default(), &b, H1);
        assert_eq!(broker(&i).lots.len(), 1, "a close of 0 closes nothing");
        let (_, i) = run(
            &src(stmt("0/0", "entry(\"L\", \"long\"", 2)),
            &Inputs::default(),
            SymInfo::default(),
            &b,
            H1,
        );
        assert_eq!(broker(&i).lots[0].qty, 1.0, "NaN is the default size");
    }
}

#[test]
fn daily_bars_open_with_their_session() {
    let us = SymInfo {
        tickerid: "NASDAQ:AAPL".into(),
        kind: "stock".into(),
        timezone: "America/New_York".into(),
        ..SymInfo::default()
    };
    // Tuesday 2 January 2024 and Wednesday, as the app stamps them.
    let b: Vec<Bar> = ["2024-01-02T00:00:00Z", "2024-01-03T00:00:00Z"]
        .iter()
        .zip(bars(2))
        .map(|(t, b)| Bar { time: ms(t), ..b })
        .collect();
    for s in both(
        "strategy(\"T\")",
        "plot(bar.time)\nplot(time(1))\nstrategy.entry(\"L\", \"long\")",
    ) {
        let (p, i) = run(&s, &Inputs::default(), us.clone(), &b, D1);
        assert_eq!(p[0][0] as i64, ms("2024-01-02T14:30:00Z"));
        assert_eq!(p[1][1] as i64, ms("2024-01-02T14:30:00Z"));
        // Trades keep the chart's own stamps, for its marks and the report.
        assert_eq!(broker(&i).lots[0].time, b[1].time);
    }
    // Crypto days start at 00:00 UTC, and intraday bars are as they are.
    for (sym, tf) in [(SymInfo::default(), D1), (us, H1)] {
        for s in both("indicator(\"T\")", "plot(bar.time)") {
            assert_eq!(
                run(&s, &Inputs::default(), sym.clone(), &b, tf).0[0][1] as i64,
                b[1].time
            );
        }
    }
}

#[test]
fn levels_follow_their_inputs() {
    for (lang, src) in both(
        "indicator(\"T\")\nlocal n = input.int(\"OB\", 70)\nhline(\"Top\", n + 5)",
        "plot(bar.close)",
    ) {
        let c = compile(lang, &src, "t", SymInfo::default()).unwrap_or_else(|e| panic!("{e}\n{src}"));
        assert_eq!(c.study(&Inputs::default()).levels()[0].0, 75.0);
        let mut i = Inputs::default();
        i.values.insert("ob".into(), Value::Int(80));
        assert_eq!(c.study(&i).levels()[0].0, 85.0);
    }
}

#[test]
fn tiny_negatives_print_as_zero() {
    assert_eq!(
        opentern_script::host::show(&opentern_script::host::Val::Num(-1e-12)),
        "0"
    );
}

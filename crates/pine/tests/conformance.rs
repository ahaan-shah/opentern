//! The "triple check" of the built-ins: TradingView's built-in indicators,
//! written in Pine (`tests/corpus/`), must match Opentern's Rust ports bar
//! for bar (same `na` bars, values within 1e-9 relative). With
//! `OPENTERN_PRIVATE_CORPUS` set, TradingView exports are checked too (see
//! `golden` in opentern-series).

use opentern_core::{Bar, Timeframe, Unit};
use opentern_pine::{Env, compile};
use opentern_series::study::{Inputs, Instance, sessions};

/// A deterministic random walk with the awkward cases indicators trip on:
/// gaps, flat bars (high == low), zero volume and a flat stretch.
fn bars(n: usize, tf: Timeframe, seed: u64) -> Vec<Bar> {
    let mut s = seed;
    let mut rnd = move || {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((s >> 33) as f64) / (1u64 << 31) as f64
    };
    let step = tf.millis().unwrap_or(86_400_000);
    let mut t = 1_600_000_000_000 / step * step;
    let mut close: f64 = 100.0;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let open = if i % 97 == 50 {
            close * (1.0 + (rnd() - 0.5) * 0.06)
        } else {
            close
        };
        let flat = (300..310).contains(&i) || i % 211 == 7;
        let c = if flat {
            open
        } else {
            open * (1.0 + (rnd() - 0.5) * 0.03 + 0.0003)
        };
        let (hi, lo) = if flat {
            (open, open)
        } else {
            (open.max(c) * (1.0 + rnd() * 0.01), open.min(c) * (1.0 - rnd() * 0.01))
        };
        let volume = if i % 173 == 3 {
            0.0
        } else {
            (1000.0 + rnd() * 9000.0).round()
        };
        out.push(Bar {
            time: t,
            open,
            high: hi,
            low: lo,
            close: c,
            volume,
        });
        close = c;
        t += step;
    }
    out
}

fn corpus(name: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn close(a: f64, b: f64) -> bool {
    if a.is_nan() || b.is_nan() {
        return a.is_nan() && b.is_nan();
    }
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

/// (corpus file, built-in id, (pine plot, built-in plot) pairs, compare colour indices)
type Case = (&'static str, &'static str, &'static [(usize, usize)], bool);

const CASES: &[Case] = &[
    ("sma.pine", "sma", &[(0, 0)], false),
    ("median.pine", "median", &[(0, 0), (1, 1), (2, 2), (3, 3)], true),
    ("majority_rule.pine", "majority_rule", &[(0, 0)], false),
    ("ema.pine", "ema", &[(0, 0)], false),
    ("wma.pine", "wma", &[(0, 0)], false),
    ("vwma.pine", "vwma", &[(0, 0)], false),
    ("hma.pine", "hma", &[(0, 0)], false),
    ("smma.pine", "smma", &[(0, 0)], false),
    ("dema.pine", "dema", &[(0, 0)], false),
    ("tema.pine", "tema", &[(0, 0)], false),
    ("alma.pine", "alma", &[(0, 0)], false),
    ("lsma.pine", "lsma", &[(0, 0)], false),
    ("mcginley.pine", "mcginley", &[(0, 0)], false),
    ("rsi.pine", "rsi", &[(0, 0), (1, 1)], false),
    ("rsi_v6.pine", "rsi", &[(0, 0)], false),
    ("macd.pine", "macd", &[(0, 0), (1, 1), (2, 2)], true),
    ("bb.pine", "bb", &[(0, 0), (1, 1), (2, 2)], false),
    ("bbw.pine", "bbw", &[(0, 0)], false),
    ("bbpb.pine", "bbpb", &[(0, 0)], false),
    ("atr.pine", "atr", &[(0, 0)], false),
    ("supertrend.pine", "supertrend", &[(0, 0), (1, 1), (2, 2)], false),
    ("stoch.pine", "stoch", &[(0, 0), (1, 1)], false),
    ("stochrsi.pine", "stochrsi", &[(0, 0), (1, 1)], false),
    ("cci.pine", "cci", &[(0, 0)], false),
    ("mom.pine", "mom", &[(0, 0)], false),
    ("roc.pine", "roc", &[(0, 0)], false),
    ("willr.pine", "willr", &[(0, 0)], false),
    ("obv.pine", "obv", &[(0, 0)], false),
    ("obv_var.pine", "obv", &[(0, 0)], false),
    ("mfi.pine", "mfi", &[(0, 0)], false),
    ("dmi.pine", "dmi", &[(0, 0), (1, 1), (2, 2)], false),
    ("dmi_ta.pine", "dmi", &[(0, 0), (1, 1), (2, 2)], false),
    ("psar.pine", "psar", &[(0, 0)], false),
    ("dc.pine", "dc", &[(0, 0), (1, 1), (2, 2)], false),
    ("kc.pine", "kc", &[(0, 0), (1, 1), (2, 2)], false),
    ("stdev.pine", "stdev", &[(0, 0)], false),
    (
        "ichimoku.pine",
        "ichimoku",
        &[(0, 0), (1, 1), (2, 2), (3, 3), (4, 4)],
        false,
    ),
    ("aroon.pine", "aroon", &[(0, 0), (1, 1)], false),
    ("ao.pine", "ao", &[(0, 0)], false),
    ("tsi.pine", "tsi", &[(0, 0), (1, 1)], false),
    ("cmo.pine", "cmo", &[(0, 0)], false),
    ("hv.pine", "hv", &[(0, 0)], false),
    ("bop.pine", "bop", &[(0, 0)], false),
    ("efi.pine", "efi", &[(0, 0)], false),
    ("pvt.pine", "pvt", &[(0, 0)], false),
    ("ad.pine", "ad", &[(0, 0)], false),
    ("cmf.pine", "cmf", &[(0, 0)], false),
    ("uo.pine", "uo", &[(0, 0)], false),
    ("trix.pine", "trix", &[(0, 0)], false),
    ("vortex.pine", "vortex", &[(0, 0), (1, 1)], false),
    ("chop.pine", "chop", &[(0, 0)], false),
    ("mass.pine", "mass", &[(0, 0)], false),
    ("fisher.pine", "fisher", &[(0, 0), (1, 1)], false),
    ("coppock.pine", "coppock", &[(0, 0)], false),
    ("vwap.pine", "vwap", &[(0, 0)], false),
    ("ma_cross.pine", "ma_cross", &[(0, 0), (1, 1)], false),
    ("ema_cross.pine", "ema_cross", &[(0, 0), (1, 1)], false),
    ("env.pine", "env", &[(0, 0), (1, 1), (2, 2)], false),
    ("kst.pine", "kst", &[(0, 0), (1, 1)], false),
    ("ppo.pine", "ppo", &[(0, 0)], false),
    ("eom.pine", "eom", &[(0, 0)], false),
    ("vosc.pine", "vosc", &[(0, 0)], false),
    ("bbp.pine", "bbp", &[(0, 0)], false),
    ("alligator.pine", "alligator", &[(0, 0), (1, 1), (2, 2)], false),
    ("dpo.pine", "dpo", &[(0, 0)], false),
    // Wave 2.
    ("chaikin_osc.pine", "chaikin_osc", &[(0, 0)], false),
    ("ckstop.pine", "ckstop", &[(0, 0), (1, 1)], false),
    ("chop_zone.pine", "chop_zone", &[(0, 0)], true),
    ("crsi.pine", "crsi", &[(0, 0)], false),
    ("kvo.pine", "kvo", &[(0, 0), (1, 1)], false),
    ("ma_ribbon.pine", "ma_ribbon", &[(0, 0), (1, 1), (2, 2), (3, 3)], false),
    ("net_volume.pine", "net_volume", &[(0, 0)], false),
    ("pivots_hl.pine", "pivots_hl", &[(0, 0), (1, 1)], false),
    ("rci.pine", "rci", &[(0, 0)], false),
    ("rci_ribbon.pine", "rci_ribbon", &[(0, 0), (1, 1), (2, 2)], false),
    ("rvgi.pine", "rvgi", &[(0, 0), (1, 1)], false),
    ("rvi.pine", "rvi", &[(0, 0)], false),
    ("smi_ergodic.pine", "smi_ergodic", &[(0, 0), (1, 1)], false),
    ("smi_ergodic_osc.pine", "smi_ergodic_osc", &[(0, 0)], false),
    ("stderr.pine", "stderr", &[(0, 0)], false),
    ("smi.pine", "smi", &[(0, 0), (1, 1)], false),
    ("twap.pine", "twap", &[(0, 0)], false),
    ("trend_strength.pine", "trend_strength", &[(0, 0)], false),
    ("volatility_stop.pine", "volatility_stop", &[(0, 0)], true),
    ("fractals.pine", "fractals", &[(0, 0), (1, 1)], false),
    ("woodies_cci.pine", "woodies_cci", &[(0, 0), (1, 1), (2, 2)], true),
    ("volume_24h.pine", "volume_24h", &[(0, 0)], false),
];

#[test]
fn builtins_written_in_pine_match_the_rust_ports() {
    let sets = [
        (
            Timeframe::new(1, Unit::Hour),
            bars(3000, Timeframe::new(1, Unit::Hour), 7),
        ),
        (
            Timeframe::new(1, Unit::Day),
            bars(1500, Timeframe::new(1, Unit::Day), 11),
        ),
    ];
    let mut failures = Vec::new();
    for (file, id, pairs, colors) in CASES {
        let src = corpus(file);
        let script = match compile(&src, file, Env::default()) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{file}: doesn't compile: {e}"));
                continue;
            }
        };
        let meta = opentern_series::builtins::by_id(id).unwrap_or_else(|| panic!("no built-in {id}"));
        for (tf, bars) in &sets {
            let mut pine = script.study(&Inputs::default());
            let got = pine.run(bars, *tf);
            if let Some(e) = pine.error() {
                failures.push(format!("{file} ({}): runtime error: {e}", tf.label()));
                continue;
            }
            let mut inputs = meta.default_inputs();
            // TradingView's VWAP hides itself on daily charts; compare anyway.
            if *id == "vwap" {
                inputs.set("hide_daily", opentern_series::study::Value::Bool(false));
            }
            let mut inst = Instance::new(meta, &inputs);
            let sess = sessions(bars, *tf, &jiff::tz::TimeZone::UTC);
            inst.recompute(bars, &sess, *tf);
            for &(p, b) in *pairs {
                let (a, w) = (&got.plots[p], &inst.out.plots[b]);
                if let Some(i) = (0..bars.len()).find(|&i| !close(a[i], w[i])) {
                    failures.push(format!(
                        "{file} ({}) plot {p} vs {id} plot {b}: bar {i}: pine {} vs rust {}",
                        tf.label(),
                        a[i],
                        w[i]
                    ));
                    continue;
                }
                if *colors && let Some(i) = (0..bars.len()).find(|&i| got.colors[p][i] != inst.out.colors[b][i]) {
                    failures.push(format!(
                        "{file} ({}) plot {p} colour at bar {i}: pine {} vs rust {}",
                        tf.label(),
                        got.colors[p][i],
                        inst.out.colors[b][i]
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_corpus_script_compiles_and_runs() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    let tf = Timeframe::new(1, Unit::Hour);
    let bars = bars(500, tf, 3);
    let mut failures = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    files.sort();
    for p in files.iter().filter(|p| p.extension().is_some_and(|e| e == "pine")) {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(p).unwrap();
        match compile(&src, &name, Env::default()) {
            Ok(s) => {
                let mut st = s.study(&Inputs::default());
                st.run(&bars, tf);
                if let Some(e) = st.error() {
                    failures.push(format!("{name}: {e}"));
                }
            }
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `cargo test --release -p opentern-pine --test conformance -- --ignored --nocapture`
#[test]
#[ignore]
fn timing_100k_bars() {
    let tf = Timeframe::new(1, Unit::Hour);
    let b = bars(100_000, tf, 5);
    for file in [
        "rsi.pine",
        "macd.pine",
        "bb.pine",
        "supertrend.pine",
        "dmi.pine",
        "ichimoku.pine",
        "kc.pine",
    ] {
        let s = compile(&corpus(file), file, Env::default()).unwrap();
        let sess = opentern_series::study::sessions(&b, tf, &jiff::tz::TimeZone::UTC);
        let t = std::time::Instant::now();
        let mut st = s.study(&Inputs::default());
        st.run_with(&b, &sess, tf);
        println!("{file:>16}: {:>6.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    }
}

#[test]
#[ignore]
fn timing_micro() {
    let tf = Timeframe::new(1, Unit::Hour);
    let b = bars(100_000, tf, 5);
    for body in [
        "plot(close)",
        "plot(close + 1)",
        "x = close\nplot(x)",
        "plot(ta.sma(close, 14))",
        "plot(math.max(close, 0))",
        "f(x) => x\nplot(f(close))",
        "plot(close[1])",
        "x = close\nplot(x[1])",
        "s = \"SMA\"\nplot(s == \"SMA\" ? 1 : 0)",
        "plot(ta.change(close))",
        "plot(ta.rma(close, 14))",
        "f(x, t) =>\n    switch t\n        \"SMA\" => x\n        => 0\nplot(f(close, \"SMA\"))",
        "up = ta.rma(math.max(ta.change(close), 0), 14)\nplot(up)",
    ] {
        let src = format!("//@version=5\nindicator(\"t\")\n{body}");
        let s = compile(&src, "t", Env::default()).unwrap();
        let sess = opentern_series::study::sessions(&b, tf, &jiff::tz::TimeZone::UTC);
        let mut best = f64::MAX;
        for _ in 0..3 {
            let t = std::time::Instant::now();
            let mut st = s.study(&Inputs::default());
            st.run_with(&b, &sess, tf);
            best = best.min(t.elapsed().as_secs_f64());
        }
        let t = std::time::Instant::now() - std::time::Duration::from_secs_f64(best);
        println!(
            "{:>40}: {:>6.1} ms",
            body.replace('\n', "; "),
            t.elapsed().as_secs_f64() * 1000.0
        );
    }
}

#[test]
#[ignore]
fn timing_step_overhead() {
    use opentern_series::study::{Ctx, Out, Study};
    let tf = Timeframe::new(1, Unit::Hour);
    let b = bars(100_000, tf, 5);
    for body in [
        "x = 1",
        "plot(close)",
        "a = close\nb = a + 1\nc = b * 2\nd = c - a\nplot(d)",
    ] {
        let src = format!("//@version=5\nindicator(\"t\")\n{body}");
        let s = compile(&src, "t", Env::default()).unwrap();
        let n = s.meta().plots.len();
        let mut best = f64::MAX;
        for _ in 0..5 {
            let mut st = s.study(&Inputs::default());
            let mut o = Out {
                values: vec![f64::NAN; n],
                colors: vec![0; n],
                fills: vec![],
                raw: vec![0; n],
                fill_raw: vec![],
                bg: 0,
                bar: 0,
            };
            let t = std::time::Instant::now();
            for (i, bar) in b.iter().enumerate() {
                let c = Ctx {
                    bar,
                    index: i,
                    new_session: false,
                    tf,
                    forming: false,
                    count: b.len(),
                };
                st.step(&c, &mut o);
            }
            best = best.min(t.elapsed().as_secs_f64());
        }
        println!(
            "{:>50}: {:>6.1} ns/bar",
            body.replace('\n', "; "),
            best * 1e9 / b.len() as f64
        );
    }
}

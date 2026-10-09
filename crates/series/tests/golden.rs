//! Checks built-ins against values exported from TradingView.
//!
//! Each case is a pair in `tests/golden/` (or in the folder named by
//! `OPENTERN_PRIVATE_CORPUS`, for exports that mustn't be committed):
//!
//! - `<name>.csv`: TradingView's "Export chart data" for a chart with the
//!   indicator on it, with times as UNIX timestamps (the export's default).
//! - `<name>.json`: which study, its inputs, and which CSV column holds each
//!   plot, for example:
//!   `{"study": "rsi", "interval": "1D", "inputs": {"length": 14},
//!     "columns": {"RSI": "RSI", "RSI-based MA": "RSI-based MA"}}`
//!
//! Values must match within a relative 1e-6 once both sides have values
//! (TradingView's export rounds), and `na` bars must be `na` on both sides.
//! `"skip": N` ignores the first N rows: an export starts mid-history, so
//! recursive indicators (EMA, RSI, ATR) need a warm-up before they agree.
//! See docs/indicators.md for how to make an export.

use opentern_core::{Bar, Timeframe};
use opentern_series::builtins;
use opentern_series::study::{Inputs, Instance, Value, sessions};
use std::path::{Path, PathBuf};

struct Csv {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
}

fn read_csv(p: &Path) -> Csv {
    let text = std::fs::read_to_string(p).unwrap();
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines.next().unwrap().split(',').map(|s| s.trim().to_string()).collect();
    let rows = lines
        .map(|l| l.split(',').map(|s| s.trim().to_string()).collect())
        .collect();
    Csv { header, rows }
}

fn col(csv: &Csv, name: &str) -> Option<usize> {
    csv.header.iter().position(|h| h.eq_ignore_ascii_case(name))
}

fn num(s: &str) -> f64 {
    s.parse().unwrap_or(f64::NAN)
}

fn dirs() -> Vec<PathBuf> {
    let mut d = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")];
    if let Some(p) = std::env::var_os("OPENTERN_PRIVATE_CORPUS") {
        d.push(PathBuf::from(p));
    }
    d
}

fn json_value(v: &str) -> Value {
    // A tiny reader for the input values: numbers, true/false, strings.
    let v = v.trim();
    if v == "true" || v == "false" {
        Value::Bool(v == "true")
    } else if let Ok(i) = v.parse::<i64>() {
        Value::Int(i)
    } else if let Ok(f) = v.parse::<f64>() {
        Value::Float(f)
    } else {
        Value::Text(v.trim_matches('"').to_string())
    }
}

/// Pulls `"key": value` pairs out of a flat JSON object (the manifests are
/// simple enough not to need a JSON library in this crate).
fn object(text: &str, key: &str) -> Vec<(String, String)> {
    let Some(start) = text.find(&format!("\"{key}\"")) else {
        return vec![];
    };
    let rest = &text[start..];
    let Some(open) = rest.find('{') else { return vec![] };
    let Some(close) = rest[open..].find('}') else {
        return vec![];
    };
    rest[open + 1..open + close]
        .split(',')
        .filter_map(|kv| {
            let (k, v) = kv.split_once(':')?;
            Some((k.trim().trim_matches('"').to_string(), v.trim().to_string()))
        })
        .collect()
}

fn number(text: &str, key: &str) -> Option<usize> {
    let start = text.find(&format!("\"{key}\""))?;
    let rest = &text[start + key.len() + 2..];
    let rest = rest[rest.find(':')? + 1..].trim_start();
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn field(text: &str, key: &str) -> Option<String> {
    let start = text.find(&format!("\"{key}\""))?;
    let rest = &text[start + key.len() + 2..];
    let rest = &rest[rest.find(':')? + 1..];
    let rest = rest.trim_start().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

#[test]
fn matches_tradingview_exports() {
    let mut cases = 0;
    for dir in dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "json") {
                continue;
            }
            cases += 1;
            let text = std::fs::read_to_string(&path).unwrap();
            let study = field(&text, "study").expect("manifest needs \"study\"");
            let meta = builtins::by_id(&study).unwrap_or_else(|| panic!("unknown study {study}"));
            let tf = Timeframe::parse(&field(&text, "interval").unwrap_or("1D".into())).unwrap();
            let mut inputs = meta.default_inputs();
            for (k, v) in object(&text, "inputs") {
                inputs.set(&k, json_value(&v));
            }
            let inputs: Inputs = inputs.normalized(meta);
            let skip = number(&text, "skip").unwrap_or(0);
            let csv = read_csv(&path.with_extension("csv"));
            let c = |n: &str| col(&csv, n).unwrap_or_else(|| panic!("{}: no column {n}", path.display()));
            let (t, o, h, l, cl) = (c("time"), c("open"), c("high"), c("low"), c("close"));
            let v = col(&csv, "Volume");
            let bars: Vec<Bar> = csv
                .rows
                .iter()
                .map(|r| Bar {
                    time: num(&r[t]) as i64 * 1000,
                    open: num(&r[o]),
                    high: num(&r[h]),
                    low: num(&r[l]),
                    close: num(&r[cl]),
                    volume: v.map(|v| num(&r[v])).unwrap_or(0.0),
                })
                .collect();
            let sess = sessions(&bars, tf, &jiff::tz::TimeZone::UTC);
            let mut inst = Instance::new(meta, &inputs);
            inst.recompute(&bars, &sess, tf);
            for (plot, column) in object(&text, "columns") {
                let column = column.trim_matches('"');
                let pi = meta
                    .plots
                    .iter()
                    .position(|p| p.title == plot)
                    .unwrap_or_else(|| panic!("{study}: no plot {plot}"));
                let ci = c(column);
                let mut compared = 0;
                for (i, row) in csv.rows.iter().enumerate().skip(skip) {
                    let want = num(&row[ci]);
                    let got = inst.out.plots[pi][i];
                    match (want.is_nan(), got.is_nan()) {
                        (true, true) => {}
                        (false, false) => {
                            let tol = 1e-6 * want.abs().max(1e-9);
                            assert!(
                                (want - got).abs() <= tol.max(1e-8),
                                "{}: {plot} bar {i}: TradingView {want}, Opentern {got}",
                                path.display()
                            );
                            compared += 1;
                        }
                        _ => panic!(
                            "{}: {plot} bar {i}: TradingView {want}, Opentern {got} (na mismatch)",
                            path.display()
                        ),
                    }
                }
                println!("{}: {plot}: {compared} values match", path.display());
            }
        }
    }
    if cases == 0 {
        println!("No TradingView exports yet; see docs/indicators.md for how to add one.");
    }
}

/// How long the built-ins take over a million bars (release build):
///   cargo test --release -p opentern-series --test golden -- --ignored --nocapture
#[test]
#[ignore]
fn million_bars_timing() {
    let n = 1_000_000;
    let bars: Vec<Bar> = (0..n)
        .map(|i| {
            let x = i as f64;
            let c = 100.0 + (x * 0.01).sin() * 10.0 + (x * 0.37).cos();
            Bar {
                time: i as i64 * 60_000,
                open: c - 0.2,
                high: c + 0.5,
                low: c - 0.5,
                close: c,
                volume: 100.0 + (x * 0.3).sin().abs() * 50.0,
            }
        })
        .collect();
    let tf = Timeframe::parse("1m").unwrap();
    let sess = sessions(&bars, tf, &jiff::tz::TimeZone::UTC);
    let mut total = std::time::Duration::ZERO;
    for meta in builtins::all() {
        let start = std::time::Instant::now();
        let mut inst = Instance::new(meta, &meta.default_inputs());
        inst.recompute(&bars, &sess, tf);
        let took = start.elapsed();
        total += took;
        if took.as_millis() > 150 {
            println!("{:<12} {took:?}", meta.id);
        }
    }
    println!("all {} built-ins on {n} bars: {total:?}", builtins::all().len());
}

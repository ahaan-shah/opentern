//! Fuzzing Pine scripts: scripts get shared and imported, so a mangled one
//! must give an error, never crash the app. Every corpus script, mutated
//! (`opentern_core::fuzzing`), compiled, and run on a few hundred bars
//! when it still compiles. A long run:
//!
//!     OPENTERN_FUZZ_ROUNDS=200000 cargo test -p opentern-pine --test fuzz --release

use opentern_core::fuzzing::text_target;
use opentern_core::{Bar, Timeframe, Unit};
use opentern_pine::{Env, compile};
use opentern_series::study::Inputs;

fn bars(n: usize) -> Vec<Bar> {
    let mut close = 100.0_f64;
    (0..n)
        .map(|i| {
            let open = close;
            close = open * (1.0 + ((i * 7919 % 101) as f64 - 50.0) / 5000.0);
            Bar {
                time: 1_600_000_000_000 + i as i64 * 3_600_000,
                open,
                high: open.max(close) * 1.002,
                low: open.min(close) * 0.998,
                close,
                volume: (i % 13) as f64 * 100.0,
            }
        })
        .collect()
}

#[test]
fn mangled_scripts_never_crash() {
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut files: Vec<_> = ["corpus", "scripts"]
        .iter()
        .flat_map(|d| std::fs::read_dir(tests.join(d)).unwrap().flatten())
        .map(|e| e.path())
        .collect();
    files.sort();
    let bars = bars(300);
    let tf = Timeframe::new(1, Unit::Hour);
    // The rounds are shared out over the corpus (each script gets at least 20).
    let per = (opentern_core::fuzzing::rounds() / files.len()).max(20);
    // SAFETY: only this test binary reads it.
    unsafe { std::env::set_var("OPENTERN_FUZZ_ROUNDS", per.to_string()) };
    for path in files {
        let src = std::fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        text_target(&name, &src, |s| {
            // An abort (out of memory) can't be caught: leave the input behind.
            if let Ok(trace) = std::env::var("OPENTERN_FUZZ_TRACE") {
                let _ = std::fs::write(&trace, format!("{name}\n{s}"));
            }
            if let Ok(script) = compile(s, "fuzz.pine", Env::default()) {
                let mut study = script.study(&Inputs::default());
                let _ = study.run(&bars, tf);
            }
        });
    }
}

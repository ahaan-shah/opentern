//! How long Luau and JavaScript take over 100,000 bars (release build):
//! cargo test --release -p opentern-script --test timing -- --ignored --nocapture

use opentern_core::Timeframe;
use opentern_script::SymInfo;
use opentern_script::inproc::compile;
use opentern_series::study::{Instance, sessions};
use std::sync::Arc;
use std::time::Instant;

#[test]
#[ignore]
fn hundred_thousand_bars() {
    let bars = opentern_script::host::dry_bars(100_000);
    let tf = Timeframe::parse("1h").expect("tf");
    let sess = sessions(&bars, tf, &jiff::tz::TimeZone::UTC);
    let luau = "indicator(\"T\")\nfunction on_bar(bar)\n  local r = ta.rsi(bar.close, 14)\n  local m, s, h = ta.macd(bar.close, 12, 26, 9)\n  plot(r, \"RSI\")\n  plot(h, \"Hist\")\nend\n";
    let js = "indicator(\"T\");\nfunction onBar(bar) {\n  const r = ta.rsi(bar.close, 14);\n  const [m, s, h] = ta.macd(bar.close, 12, 26, 9);\n  plot(r, \"RSI\");\n  plot(h, \"Hist\");\n}\n";
    let empty_luau = "indicator(\"T\")\nfunction on_bar(bar)\nend\n";
    let empty_js = "indicator(\"T\");\nfunction onBar(bar) {}\n";
    let langs: [(Arc<dyn opentern_script::inproc::Lang>, &str, &str); 4] = [
        (Arc::new(opentern_script::luau::Luau), empty_luau, "empty.luau"),
        (Arc::new(opentern_script::js::Js { ts: false }), empty_js, "empty.js"),
        (Arc::new(opentern_script::luau::Luau), luau, "t.luau"),
        (Arc::new(opentern_script::js::Js { ts: false }), js, "t.js"),
    ];
    for (lang, src, file) in langs {
        let c = compile(lang, src, file, SymInfo::default()).expect("compiles");
        let m = c.meta();
        let mut inst = Instance::with_factory(m, &m.default_inputs(), c.factory());
        let t = Instant::now();
        inst.recompute(&bars, &sess, tf);
        println!("{file}: {:.0} ms for 100,000 bars", t.elapsed().as_secs_f64() * 1000.0);
    }
}

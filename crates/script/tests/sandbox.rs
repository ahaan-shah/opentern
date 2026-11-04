//! Scripts can arrive in a file from someone else, so these try to break out
//! of Luau and JavaScript the way a hostile script would. (Python isn't
//! sandboxed: it never starts without being agreed to. See `transfer.rs`.)

use opentern_script::SymInfo;
use opentern_script::inproc::{Lang, compile};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn luau() -> Arc<dyn Lang> {
    Arc::new(opentern_script::luau::Luau)
}

fn js() -> Arc<dyn Lang> {
    Arc::new(opentern_script::js::Js { ts: false })
}

fn loads(lang: Arc<dyn Lang>, src: &str, file: &str) -> Result<(), String> {
    compile(lang, src, file, SymInfo::default())
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
}

/// Finishes (either way) within the limit, or the test fails rather than hangs.
fn ends_in_time(lang: Arc<dyn Lang>, src: &'static str, file: &'static str) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(loads(lang, src, file));
    });
    let start = Instant::now();
    let r = rx
        .recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|_| panic!("{file} hung"));
    assert!(start.elapsed() < Duration::from_secs(30));
    r
}

#[test]
fn luau_has_no_way_to_the_computer() {
    for missing in [
        "io",
        "require",
        "package",
        "dofile",
        "loadfile",
        "load",
        "loadstring",
        "ffi",
        "newproxy_unsafe",
        "os.execute",
        "os.remove",
        "os.rename",
        "os.getenv",
        "os.exit",
        "os.tmpname",
        "string.dump",
        "debug.sethook",
        "debug.getregistry",
    ] {
        let src = format!(
            "indicator(\"T\")\nassert({missing} == nil, \"{missing} is reachable\")\nfunction on_bar(bar)\nend\n"
        );
        loads(luau(), &src, "t.luau").unwrap_or_else(|e| panic!("{missing}: {e}"));
    }
}

#[test]
fn javascript_has_no_way_to_the_computer() {
    for name in [
        "require",
        "process",
        "std",
        "os",
        "Deno",
        "Bun",
        "fetch",
        "XMLHttpRequest",
        "WebSocket",
        "module",
        "__loadScript",
        "scriptArgs",
        "Worker",
        "print_to_file",
    ] {
        let src = format!(
            "indicator(\"T\");\nif (typeof {name} !== \"undefined\") throw new Error(\"{name} is reachable\");\nfunction onBar(bar) {{}}\n"
        );
        loads(js(), &src, "t.js").unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    // A dynamic import has nothing to load from.
    let src = "indicator(\"T\");\nimport(\"os\").then(m => { m.exec(\"touch pwned\"); throw new Error(\"loaded\"); });\nfunction onBar(bar) {}\n";
    let _ = loads(js(), src, "t.js");
    assert!(!std::path::Path::new("pwned").exists());
}

#[test]
fn runaway_scripts_are_stopped() {
    let luau_spin = "indicator(\"T\")\nwhile true do end\n";
    let js_spin = "indicator(\"T\");\nwhile (true) {}\n";
    let bar_spin = "indicator(\"T\")\nfunction on_bar(bar)\n  while true do end\nend\n";
    let hog = "indicator(\"T\")\nlocal t = {}\nfor i = 1, 1e12 do t[i] = string.rep(\"x\", 1024 * 1024) end\n";
    let js_hog = "indicator(\"T\");\nconst t = [];\nfor (;;) t.push(\"x\".repeat(1048576) + t.length);\n";
    assert!(ends_in_time(luau(), luau_spin, "spin.luau").is_err());
    assert!(ends_in_time(js(), js_spin, "spin.js").is_err());
    assert!(ends_in_time(luau(), bar_spin, "bar.luau").is_err());
    assert!(ends_in_time(luau(), hog, "hog.luau").is_err());
    assert!(ends_in_time(js(), js_hog, "hog.js").is_err());
}

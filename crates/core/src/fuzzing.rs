//! A small fuzzer for the tests, on stable Rust: mutates a realistic input
//! (JSON structure: values swapped for other types, huge or negative
//! numbers, missing keys, deep nesting; text and bytes: flipped, cut,
//! repeated, odd tokens inserted) and reports the first input that makes
//! the code under test panic, with where. Deterministic, so a failure
//! repeats. The tests that use it (data replies, logos, Pine scripts,
//! import files) run a short round in `cargo test`; a long one:
//!
//! ```text
//! OPENTERN_FUZZ_ROUNDS=200000 cargo test --all fuzz
//! ```

use serde_json::{Value, json};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// A small deterministic generator (xorshift), so a failure repeats.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
    pub fn chance(&mut self, one_in: u64) -> bool {
        self.next_u64().is_multiple_of(one_in)
    }
}

/// How many mutated inputs each target gets.
pub fn rounds() -> usize {
    std::env::var("OPENTERN_FUZZ_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500)
}

/// Values that break careless code.
fn nasty(r: &mut Rng) -> Value {
    match r.below(22) {
        0 => Value::Null,
        1 => json!(true),
        2 => json!(0),
        3 => json!(-1),
        4 => json!(i64::MAX),
        5 => json!(i64::MIN),
        6 => json!(u64::MAX),
        7 => json!(1e308),
        8 => json!(-1e308),
        9 => json!(f64::MIN_POSITIVE),
        10 => json!(""),
        11 => json!("NaN"),
        12 => json!("-inf"),
        13 => json!("1e400"),
        14 => json!("-0"),
        15 => json!("\u{0}\u{202e}ä漢🙂"),
        16 => json!("9".repeat(400)),
        17 => json!([]),
        18 => json!({}),
        19 => json!([null, [], {}]),
        20 => json!("2024-13-45T99:99:99Z"),
        _ => {
            // Deep nesting.
            let mut v = json!(1);
            for _ in 0..(r.below(200) + 1) {
                v = json!([v]);
            }
            v
        }
    }
}

/// Every place in a tree a mutation could go.
fn paths(v: &Value, here: &mut Vec<PathStep>, out: &mut Vec<Vec<PathStep>>) {
    out.push(here.clone());
    match v {
        Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                here.push(PathStep::Index(i));
                paths(x, here, out);
                here.pop();
            }
        }
        Value::Object(o) => {
            for (k, x) in o {
                here.push(PathStep::Key(k.clone()));
                paths(x, here, out);
                here.pop();
            }
        }
        _ => {}
    }
}

#[derive(Clone)]
enum PathStep {
    Index(usize),
    Key(String),
}

fn at<'a>(v: &'a mut Value, path: &[PathStep]) -> Option<&'a mut Value> {
    let mut v = v;
    for p in path {
        v = match p {
            PathStep::Index(i) => v.get_mut(*i)?,
            PathStep::Key(k) => v.get_mut(k.as_str())?,
        };
    }
    Some(v)
}

/// One structural mutation of a JSON tree.
pub fn mutate(v: &mut Value, r: &mut Rng) {
    let mut all = Vec::new();
    paths(v, &mut Vec::new(), &mut all);
    let path = all[r.below(all.len())].clone();
    let op = r.below(8);
    // Removing an entry: from the parent.
    if op == 0
        && let Some((last, parent)) = path.split_last()
        && let Some(p) = at(v, parent)
    {
        match (p, last) {
            (Value::Array(a), PathStep::Index(i)) if *i < a.len() => {
                a.remove(*i);
            }
            (Value::Object(o), PathStep::Key(k)) => {
                o.remove(k);
            }
            _ => {}
        }
        return;
    }
    let Some(node) = at(v, &path) else { return };
    match op {
        1..=3 => *node = nasty(r),
        4 => {
            // Numbers ↔ strings, the confusion replies really have.
            *node = match node.take() {
                Value::Number(n) => Value::String(n.to_string()),
                Value::String(s) => s
                    .parse::<f64>()
                    .ok()
                    .and_then(|f| serde_json::Number::from_f64(f).map(Value::Number))
                    .unwrap_or(Value::String(s)),
                other => other,
            }
        }
        5 => {
            // Grow an array (a huge list of entries), or empty it.
            if let Value::Array(a) = node {
                if r.chance(2) {
                    a.clear();
                } else if let Some(first) = a.first().cloned() {
                    for _ in 0..r.below(300) {
                        a.push(first.clone());
                    }
                }
            }
        }
        6 => {
            // A stray key.
            if let Value::Object(o) = node {
                o.insert(format!("k{}", r.below(9)), nasty(r));
            }
        }
        _ => {
            if let Value::Array(a) = node
                && a.len() > 1
            {
                let (i, j) = (r.below(a.len()), r.below(a.len()));
                a.swap(i, j);
            }
        }
    }
}

/// One text mutation: flip, cut, repeat or insert.
pub fn mutate_text(s: &str, r: &mut Rng) -> String {
    let mut b = s.as_bytes().to_vec();
    if b.is_empty() {
        return "<".into();
    }
    match r.below(6) {
        0 => {
            let i = r.below(b.len());
            b[i] ^= 1 << r.below(8);
        }
        1 => b.truncate(r.below(b.len())),
        2 => {
            let i = r.below(b.len());
            let j = (i + r.below(40)).min(b.len());
            let piece = b[i..j].to_vec();
            for _ in 0..r.below(50) {
                b.splice(i..i, piece.iter().copied());
            }
        }
        3 => {
            let tokens: [&[u8]; 12] = [
                b"<",
                b">",
                b"</",
                b"&amp;",
                b"&#xFFFFFF;",
                b"<![CDATA[",
                b"]]>",
                b"\"",
                b",",
                b"\n",
                "ä漢🙂".as_bytes(),
                b"\x00",
            ];
            let t = tokens[r.below(tokens.len())];
            let i = r.below(b.len() + 1);
            b.splice(i..i, t.iter().copied());
        }
        4 => {
            let i = r.below(b.len());
            b.remove(i);
        }
        _ => b.reverse(),
    }
    String::from_utf8_lossy(&b).into_owned()
}

thread_local! {
    /// Where the last panic on this thread happened, and what it said.
    static WHERE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    /// Whether this thread is running a fuzz input (its panics are expected).
    static CATCHING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `inputs` through `f`, catching panics quietly; returns the first
/// that panicked, with where.
fn first_panic<T>(inputs: impl Iterator<Item = T>, f: impl Fn(&T)) -> Option<(T, String)> {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if CATCHING.with(|c| c.get()) {
                let at = info
                    .location()
                    .map(|l| format!("{}:{}", l.file(), l.line()))
                    .unwrap_or_default();
                let msg = info
                    .payload()
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| info.payload().downcast_ref::<String>().cloned())
                    .unwrap_or_default();
                WHERE.with(|w| *w.borrow_mut() = format!("{at}: {msg}"));
            } else {
                default(info);
            }
        }));
    });
    CATCHING.with(|c| c.set(true));
    let mut found = None;
    for x in inputs {
        if catch_unwind(AssertUnwindSafe(|| f(&x))).is_err() {
            found = Some((x, WHERE.with(|w| w.borrow().clone())));
            break;
        }
    }
    CATCHING.with(|c| c.set(false));
    found
}

/// Runs `f` on `rounds()` mutations of `seed`; panics with the input that
/// made it panic and where.
pub fn json_target(name: &str, seed: Value, f: impl Fn(&Value)) {
    let mut r = Rng::new(name.bytes().map(u64::from).sum::<u64>() + 1);
    let inputs = (0..rounds()).map(move |k| {
        let mut v = seed.clone();
        if k > 0 {
            for _ in 0..(1 + r.below(4)) {
                mutate(&mut v, &mut r);
            }
        }
        v
    });
    if let Some((v, at)) = first_panic(inputs, |v| f(v)) {
        panic!("{name} panicked at {at}\non: {}", truncate(&v.to_string()));
    }
}

/// The same for text inputs (RSS, CSV, formulas).
pub fn text_target(name: &str, seed: &str, f: impl Fn(&str)) {
    let mut r = Rng::new(name.bytes().map(u64::from).sum::<u64>() + 7);
    let seed = seed.to_string();
    let inputs = (0..rounds()).map(move |k| {
        let mut s = seed.clone();
        if k > 0 {
            for _ in 0..(1 + r.below(4)) {
                s = mutate_text(&s, &mut r);
            }
        }
        s
    });
    if let Some((s, at)) = first_panic(inputs, |s| f(s)) {
        panic!("{name} panicked at {at}\non: {:?}", truncate(&s));
    }
}

fn truncate(s: &str) -> String {
    s.chars().take(2000).collect()
}

/// One byte-level mutation (binary files: images, archives).
pub fn mutate_bytes(b: &[u8], r: &mut Rng) -> Vec<u8> {
    let mut b = b.to_vec();
    if b.is_empty() {
        return vec![0];
    }
    match r.below(5) {
        0 => {
            for _ in 0..(1 + r.below(8)) {
                let i = r.below(b.len());
                b[i] ^= 1 << r.below(8);
            }
        }
        1 => b.truncate(r.below(b.len())),
        2 => {
            let i = r.below(b.len());
            b[i] = [0x00, 0xFF, 0x7F, 0x80][r.below(4)];
        }
        3 => {
            // A 4-byte field set to an extreme (lengths, sizes, counts).
            let i = r.below(b.len().saturating_sub(4).max(1));
            let v: u32 = [0, 1, 0x7FFF_FFFF, 0xFFFF_FFFF, 0x0001_0000][r.below(5)];
            for (k, x) in v.to_be_bytes().iter().enumerate() {
                if i + k < b.len() {
                    b[i + k] = *x;
                }
            }
        }
        _ => {
            let i = r.below(b.len());
            let j = (i + r.below(64)).min(b.len());
            let piece = b[i..j].to_vec();
            b.splice(i..i, piece);
        }
    }
    b
}

/// The same for binary inputs.
pub fn bytes_target(name: &str, seed: &[u8], f: impl Fn(&[u8])) {
    let mut r = Rng::new(name.bytes().map(u64::from).sum::<u64>() + 13);
    let seed = seed.to_vec();
    let inputs = (0..rounds()).map(move |k| {
        let mut b = seed.clone();
        if k > 0 {
            for _ in 0..(1 + r.below(3)) {
                b = mutate_bytes(&b, &mut r);
            }
        }
        b
    });
    if let Some((b, at)) = first_panic(inputs, |b| f(b)) {
        panic!(
            "{name} panicked at {at}\non {} bytes: {:02x?}",
            b.len(),
            &b[..b.len().min(200)]
        );
    }
}

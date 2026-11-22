//! Symbol maths, as on TradingView: chart `BTCUSDT/ETHUSDT`, `AAPL/SPY`,
//! `(BTCUSDT + ETHUSDT) / 2` or `NSE:RELIANCE * 2`. Each leg is fetched and
//! streamed like any symbol; bars are joined on time and the formula is
//! applied to open, high, low and close.
//!
//! Subtraction needs spaces (`AAPL - MSFT`), because `-` can be part of a
//! ticker (`BRK-B`).

use crate::provider::{Error, Live, Provider, Result};
use crossbeam_channel::{RecvTimeoutError, unbounded};
use opentern_core::{AssetKind, Bar, Symbol, Timeframe};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Num(f64),
    /// Index into the legs.
    Leg(usize),
    Neg(Box<Expr>),
    Bin(char, Box<Expr>, Box<Expr>),
}

impl Expr {
    pub fn eval(&self, legs: &[f64]) -> f64 {
        // A long index is a long left-leaning chain (`((A + B) + C) + …`):
        // walk down it in a loop, so the stack only goes as deep as the
        // brackets (`MAX_NESTING`), not as long as the index.
        let mut spine = Vec::new();
        let mut cur = self;
        while let Expr::Bin(op, a, b) = cur {
            spine.push((*op, b));
            cur = a;
        }
        let mut v = match cur {
            Expr::Num(v) => *v,
            Expr::Leg(i) => legs.get(*i).copied().unwrap_or(f64::NAN),
            Expr::Neg(e) => -e.eval(legs),
            Expr::Bin(..) => unreachable!("walked down above"),
        };
        for (op, b) in spine.into_iter().rev() {
            let b = b.eval(legs);
            v = match op {
                '+' => v + b,
                '-' => v - b,
                '*' => v * b,
                _ => {
                    if b == 0.0 {
                        f64::NAN
                    } else {
                        v / b
                    }
                }
            };
        }
        v
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(char),
    Open,
    Close,
}

fn lex(s: &str) -> Option<Vec<Tok>> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    // `&` is part of some NSE tickers (M&M, M&MFIN).
    let word = |ch: char| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | ':' | '^' | '=' | '!' | '&');
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '(' {
            out.push(Tok::Open);
            i += 1;
        } else if ch == ')' {
            out.push(Tok::Close);
            i += 1;
        } else if matches!(ch, '+' | '*' | '/')
            || (ch == '-' && !(i > 0 && word(c[i - 1]) && i + 1 < c.len() && word(c[i + 1])))
        {
            out.push(Tok::Op(ch));
            i += 1;
        } else if word(ch) {
            let start = i;
            // A hyphen joins two word characters (BRK-B).
            while i < c.len() && (word(c[i]) || (c[i] == '-' && i + 1 < c.len() && word(c[i + 1]) && word(c[i - 1]))) {
                i += 1;
            }
            let w: String = c[start..i].iter().collect();
            match w.parse::<f64>() {
                Ok(v) => out.push(Tok::Num(v)),
                Err(_) => out.push(Tok::Ident(w.to_uppercase())),
            }
        } else {
            return None;
        }
    }
    Some(out)
}

/// True when `s` is a formula rather than a single symbol.
pub fn is_expr(s: &str) -> bool {
    lex(s).is_some_and(|t| t.iter().any(|x| matches!(x, Tok::Op(_) | Tok::Open)))
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    legs: Vec<String>,
    /// Brackets and minus signs open now (`MAX_NESTING`).
    depth: usize,
}

/// Formulas come from people and from imported files: brackets nest at
/// most this deep, and a formula has at most `MAX_TOKENS` pieces (a custom
/// index of 1,000 members fits, even on Windows' 1 MB main-thread stack). Past either the parser (and the code that
/// walks the tree) would recurse deep enough to overflow the stack.
const MAX_NESTING: usize = 64;
const MAX_TOKENS: usize = 4_000;

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn sum(&mut self) -> Option<Expr> {
        let mut e = self.product()?;
        while let Some(Tok::Op(op @ ('+' | '-'))) = self.peek().cloned() {
            self.pos += 1;
            e = Expr::Bin(op, Box::new(e), Box::new(self.product()?));
        }
        Some(e)
    }

    fn product(&mut self) -> Option<Expr> {
        let mut e = self.unary()?;
        while let Some(Tok::Op(op @ ('*' | '/'))) = self.peek().cloned() {
            self.pos += 1;
            e = Expr::Bin(op, Box::new(e), Box::new(self.unary()?));
        }
        Some(e)
    }

    fn unary(&mut self) -> Option<Expr> {
        if self.depth >= MAX_NESTING {
            return None;
        }
        self.depth += 1;
        let e = if let Some(Tok::Op('-')) = self.peek() {
            self.pos += 1;
            self.unary().map(|x| Expr::Neg(Box::new(x)))
        } else {
            self.atom()
        };
        self.depth -= 1;
        e
    }

    fn atom(&mut self) -> Option<Expr> {
        let t = self.peek()?.clone();
        self.pos += 1;
        match t {
            Tok::Num(v) => Some(Expr::Num(v)),
            Tok::Ident(id) => {
                let i = match self.legs.iter().position(|l| *l == id) {
                    Some(i) => i,
                    None => {
                        self.legs.push(id);
                        self.legs.len() - 1
                    }
                };
                Some(Expr::Leg(i))
            }
            Tok::Open => {
                let e = self.sum()?;
                (self.peek() == Some(&Tok::Close)).then(|| self.pos += 1)?;
                Some(e)
            }
            _ => None,
        }
    }
}

/// Parses a formula into an expression and its legs (as written).
pub fn parse(s: &str) -> Option<(Expr, Vec<String>)> {
    let toks = lex(s)?;
    if toks.len() > MAX_TOKENS {
        return None;
    }
    let mut p = Parser {
        toks,
        pos: 0,
        legs: Vec::new(),
        depth: 0,
    };
    let e = p.sum()?;
    (p.pos == p.toks.len() && !p.legs.is_empty()).then_some((e, p.legs))
}

/// Writes the formula back with each leg replaced by `name(i)`.
pub fn render(e: &Expr, name: &dyn Fn(usize) -> String) -> String {
    fn prec(e: &Expr) -> u8 {
        match e {
            Expr::Bin('+' | '-', ..) => 1,
            Expr::Bin(..) => 2,
            _ => 3,
        }
    }
    match e {
        Expr::Num(v) => {
            let s = format!("{v}");
            s
        }
        Expr::Leg(i) => name(*i),
        Expr::Neg(x) => format!("-{}", wrap(x, 3, name)),
        Expr::Bin(..) => {
            // Down the left-leaning chain in a loop (as `eval`): the stack
            // stays as deep as the brackets, however long the index.
            let p = prec(e);
            let mut spine = Vec::new();
            let mut cur = e;
            while let Expr::Bin(op, a, b) = cur
                && prec(cur) == p
            {
                spine.push((*op, b));
                cur = a;
            }
            let mut out = wrap(cur, p, name);
            for (op, b) in spine.into_iter().rev() {
                // The right side of - and / needs brackets at equal precedence.
                let right = if matches!(op, '-' | '/') { p + 1 } else { p };
                if op == '-' || op == '+' {
                    out.push_str(&format!(" {op} "));
                } else {
                    out.push(op);
                }
                out.push_str(&wrap(b, right, name));
            }
            out
        }
    }
}

fn wrap(e: &Expr, min: u8, name: &dyn Fn(usize) -> String) -> String {
    let p = match e {
        Expr::Bin('+' | '-', ..) => 1,
        Expr::Bin(..) => 2,
        _ => 3,
    };
    let s = render(e, name);
    if p < min { format!("({s})") } else { s }
}

/// Guesses the full id of a bare ticker in a formula: crypto quote assets
/// mean Binance; a known symbol's id wins; otherwise Yahoo.
pub fn resolve(ticker: &str, known: &[Symbol]) -> String {
    if ticker.contains(':') {
        return ticker.to_string();
    }
    if let Some(s) = known.iter().find(|s| s.ticker() == ticker) {
        return s.id.clone();
    }
    let crypto = ["USDT", "USDC", "FDUSD", "BTC", "ETH", "BNB"];
    if crypto.iter().any(|q| ticker.ends_with(q) && ticker.len() > q.len()) {
        format!("BINANCE:{ticker}")
    } else {
        format!("YAHOO:{ticker}")
    }
}

/// The symbol for a formula, with every leg resolved to a full id.
pub fn symbol(formula: &str, known: &[Symbol]) -> Option<Symbol> {
    let (e, legs) = parse(formula)?;
    let ids: Vec<String> = legs.iter().map(|l| resolve(l, known)).collect();
    let id = render(&e, &|i| ids[i].clone());
    let short = render(&e, &|i| {
        ids[i]
            .split_once(':')
            .map(|(_, t)| t.to_string())
            .unwrap_or(ids[i].clone())
    });
    let first = known.iter().find(|s| s.id == ids[0]);
    Some(Symbol {
        id,
        description: short,
        kind: AssetKind::Index,
        currency: String::new(),
        tick_size: 0.000001,
        min_qty: 0.0,
        timezone: first.map(|s| s.timezone.clone()).unwrap_or_else(|| guess_tz(&ids[0])),
    })
}

/// A symbol's currency from its id, for when its details aren't loaded:
/// "" when the id doesn't say.
pub fn guess_currency(id: &str) -> String {
    let (ex, t) = id.split_once(':').unwrap_or(("", id));
    match ex {
        "NSE" | "BSE" => "INR".into(),
        "AMEX" | "NASDAQ" | "NYSE" | "ARCA" | "BATS" | "CBOE" | "COMEX" | "NYMEX" | "CBOT" | "CME" => "USD".into(),
        "FX" | "OANDA" if t.replace('_', "").len() == 6 => t.replace('_', "")[3..].to_string(),
        "BINANCE" | "BYBIT" | "OKX" | "COINBASE" | "KRAKEN" => {
            let t = t.replace(['-', '/'], "");
            crate::provider::split_pair(&t)
                .map(|(_, q)| q.to_string())
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}

pub fn guess_tz(id: &str) -> String {
    match id.split(':').next() {
        Some("NSE" | "BSE") => "Asia/Kolkata",
        Some("AMEX" | "NASDAQ" | "NYSE") => "America/New_York",
        _ => "UTC",
    }
    .to_string()
}

/// Joins legs' bars on time and applies the formula to each price.
pub fn combine(e: &Expr, legs: &[Vec<Bar>]) -> Vec<Bar> {
    let mut by_time: BTreeMap<i64, Vec<Option<Bar>>> = BTreeMap::new();
    for (k, bars) in legs.iter().enumerate() {
        for b in bars {
            by_time.entry(b.time).or_insert_with(|| vec![None; legs.len()])[k] = Some(*b);
        }
    }
    by_time
        .into_values()
        .filter_map(|row| {
            let row: Vec<Bar> = row.into_iter().collect::<Option<Vec<_>>>()?;
            combine_one(e, &row)
        })
        .collect()
}

/// One bar from one bar per leg (all at the same time).
pub fn combine_one(e: &Expr, row: &[Bar]) -> Option<Bar> {
    let f = |g: fn(&Bar) -> f64| e.eval(&row.iter().map(g).collect::<Vec<_>>());
    let (o, h, l, c) = (f(|b| b.open), f(|b| b.high), f(|b| b.low), f(|b| b.close));
    if ![o, h, l, c].iter().all(|v| v.is_finite()) {
        return None;
    }
    // A ratio's high can come from the denominator's low, so take the
    // extremes of all four.
    Some(Bar {
        time: row[0].time,
        open: o,
        high: o.max(h).max(l).max(c),
        low: o.min(h).min(l).min(c),
        close: c,
        volume: 0.0,
    })
}

/// Custom indices (`CUSTOM:NAME`): the app registers each one's formulas
/// here, and the hub charts and quotes it through them. An index is a list
/// of (from, formula) segments, oldest first: a rebalance starts a new
/// segment, so each bar uses the weights that held on its day.
type Segments = Vec<(i64, String)>;

fn customs() -> &'static std::sync::RwLock<std::collections::HashMap<String, Segments>> {
    static C: std::sync::OnceLock<std::sync::RwLock<std::collections::HashMap<String, Segments>>> =
        std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

pub const CUSTOM: &str = "CUSTOM:";

pub fn is_custom(id: &str) -> bool {
    id.starts_with(CUSTOM)
}

/// Replaces every custom index (id → segments).
pub fn set_customs(all: impl IntoIterator<Item = (String, Segments)>) {
    if let Ok(mut m) = customs().write() {
        *m = all.into_iter().collect();
    }
}

/// A custom index's segments.
pub fn custom_segments(id: &str) -> Option<Segments> {
    if !is_custom(id) {
        return None;
    }
    customs().read().ok()?.get(id).cloned().filter(|s| !s.is_empty())
}

/// A custom index's current formula (its latest segment's).
pub fn custom_formula(id: &str) -> Option<String> {
    custom_segments(id)?.pop().map(|(_, f)| f)
}

/// `e` with its legs renumbered through `map`.
fn remap(e: &Expr, map: &[usize]) -> Expr {
    match e {
        Expr::Num(v) => Expr::Num(*v),
        Expr::Leg(i) => Expr::Leg(map[*i]),
        Expr::Neg(x) => Expr::Neg(Box::new(remap(x, map))),
        Expr::Bin(op, a, b) => Expr::Bin(*op, Box::new(remap(a, map)), Box::new(remap(b, map))),
    }
}

/// One formula in force from a time on, and the legs it uses.
#[derive(Clone, Debug)]
struct Seg {
    from: i64,
    expr: Expr,
    used: Vec<usize>,
}

/// The segment in force at `t` (the first one covers everything before).
fn seg_at(segs: &[Seg], t: i64) -> &Seg {
    let k = segs.partition_point(|s| s.from <= t);
    &segs[k.saturating_sub(1)]
}

/// One bar at `time` from the legs' bars (unused legs may be missing).
fn combine_at(s: &Seg, row: &[Option<Bar>], time: i64) -> Option<Bar> {
    if s.used.iter().any(|&i| row[i].is_none()) {
        return None;
    }
    let blank = Bar {
        time,
        open: 0.0,
        high: 0.0,
        low: 0.0,
        close: 0.0,
        volume: 0.0,
    };
    let full: Vec<Bar> = row.iter().map(|b| b.unwrap_or(blank)).collect();
    combine_one(&s.expr, &full).map(|b| Bar { time, ..b })
}

/// Joins legs' bars on time and applies each time's segment (a custom
/// index). A member with no bar at a time some other member traded (a
/// suspension, a gap in the source's data) counts at its last close, as
/// an exchange's index does; the time is skipped only until every member
/// has a first price.
fn combine_segs(segs: &[Seg], legs: &[Vec<Bar>]) -> Vec<Bar> {
    let mut by_time: BTreeMap<i64, Vec<Option<Bar>>> = BTreeMap::new();
    for (k, bars) in legs.iter().enumerate() {
        for b in bars {
            by_time.entry(b.time).or_insert_with(|| vec![None; legs.len()])[k] = Some(*b);
        }
    }
    let mut last: Vec<Option<f64>> = vec![None; legs.len()];
    let mut out = Vec::new();
    for (t, mut row) in by_time {
        let s = seg_at(segs, t);
        let traded = s.used.iter().any(|&i| row[i].is_some());
        for (i, b) in row.iter_mut().enumerate() {
            match b {
                Some(b) => last[i] = Some(b.close),
                None => {
                    *b = last[i].map(|c| Bar {
                        time: t,
                        open: c,
                        high: c,
                        low: c,
                        close: c,
                        volume: 0.0,
                    })
                }
            }
        }
        if traded && let Some(b) = combine_at(s, &row, t) {
            out.push(b);
        }
    }
    out
}

/// A provider for a formula (or a custom index's segments): runs the
/// legs' providers and combines.
pub struct ExprProvider {
    segs: Vec<Seg>,
    legs: Vec<(Symbol, Arc<dyn Provider>)>,
    demo: bool,
}

/// A leg's error with the leg's name in front ("NSE:TATAMOTORS: …"); a
/// lost connection stays as it is.
fn named(s: &Symbol, e: Error) -> Error {
    match e {
        Error::Bad(m) => Error::Bad(format!("{}: {m}", s.id)),
        Error::Down(m) => Error::Down(format!("{}: {m}", s.id)),
        Error::Offline => Error::Offline,
    }
}

impl ExprProvider {
    pub fn new(sym: &Symbol, demo: bool) -> Option<ExprProvider> {
        ExprProvider::from_formula(&sym.id, demo)
    }

    pub fn from_formula(formula: &str, demo: bool) -> Option<ExprProvider> {
        ExprProvider::from_segments(&[(i64::MIN, formula.to_string())], demo)
    }

    pub fn from_segments(segments: &[(i64, String)], demo: bool) -> Option<ExprProvider> {
        let mut ids: Vec<String> = Vec::new();
        let mut segs = Vec::new();
        for (from, f) in segments {
            let (expr, names) = parse(f)?;
            let map: Vec<usize> = names
                .iter()
                .map(|n| match ids.iter().position(|x| x == n) {
                    Some(i) => i,
                    None => {
                        ids.push(n.clone());
                        ids.len() - 1
                    }
                })
                .collect();
            let mut used = map.clone();
            used.sort_unstable();
            used.dedup();
            segs.push(Seg {
                from: *from,
                expr: remap(&expr, &map),
                used,
            });
        }
        if segs.is_empty() {
            return None;
        }
        segs.sort_by_key(|s| s.from);
        let legs = ids
            .iter()
            .map(|id| {
                let s = Symbol {
                    timezone: guess_tz(id),
                    ..Symbol::bare(id)
                };
                let p = crate::hub::provider_for(&s, demo);
                (s, p)
            })
            .collect();
        Some(ExprProvider { segs, legs, demo })
    }

    /// Runs `f` for every leg at once (a custom index can have dozens) and
    /// combines. A leg's error names it.
    fn each(&self, f: impl Fn(&Symbol, &dyn Provider) -> Result<Vec<Bar>> + Sync) -> Result<Vec<Bar>> {
        let results: Vec<Result<Vec<Bar>>> = std::thread::scope(|sc| {
            let hs: Vec<_> = self
                .legs
                .iter()
                .map(|(s, p)| {
                    let f = &f;
                    sc.spawn(move || f(s, p.as_ref()))
                })
                .collect();
            hs.into_iter()
                .map(|h| h.join().unwrap_or(Err(Error::Offline)))
                .collect()
        });
        let mut all = Vec::new();
        for ((s, _), r) in self.legs.iter().zip(results) {
            all.push(r.map_err(|e| named(s, e))?);
        }
        Ok(combine_segs(&self.segs, &all))
    }
}

impl Provider for ExprProvider {
    fn natives(&self) -> Vec<Timeframe> {
        let mut it = self.legs.iter().map(|(_, p)| p.natives());
        let first = it.next().unwrap_or_default();
        let rest: Vec<Vec<Timeframe>> = it.collect();
        // Weights that change over time (a custom index's segments) apply
        // from a day on: combining the members' weekly or monthly bars would
        // value a week or month in which they changed with the weights at
        // its start. Such weeks and months are built from the index's days.
        let multi = self.segs.len() > 1;
        first
            .into_iter()
            .filter(|t| rest.iter().all(|r| r.contains(t)))
            .filter(|t| !multi || t.is_intraday() || t.unit == opentern_core::Unit::Day)
            .collect()
    }

    fn latest(&self, _: &Symbol, tf: Timeframe, n: usize) -> Result<Vec<Bar>> {
        // Through the bar cache: switching between indices doesn't fetch
        // every member's whole history again.
        let demo = self.demo;
        self.each(|s, _| crate::hub::fetch_cached(s, tf, n, demo))
    }

    fn since(&self, _: &Symbol, tf: Timeframe, start: i64) -> Result<Vec<Bar>> {
        self.each(|s, p| p.since(s, tf, start))
    }

    fn before(&self, _: &Symbol, tf: Timeframe, end: i64, n: usize) -> Result<Vec<Bar>> {
        self.each(|s, p| p.before(s, tf, end, n))
    }

    /// Each leg streams on its own thread and retries on its own after an
    /// error (one member's failed poll doesn't stop the index). A leg with
    /// no new price counts at its last one. The stream ends, with the real
    /// error, only when every leg is failing at once.
    fn stream(&self, _: &Symbol, tf: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()> {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = unbounded::<(usize, std::result::Result<Bar, Error>)>();
        let mut threads = Vec::new();
        // Only the current segment's legs stream; the live bar uses it.
        let live = self.segs.last().expect("a segment").clone();
        for (k, (s, p)) in self.legs.iter().enumerate() {
            if !live.used.contains(&k) {
                continue;
            }
            let (s, p, tx, stop) = (s.clone(), p.clone(), tx.clone(), stop.clone());
            threads.push(std::thread::spawn(move || {
                let mut backoff = Duration::from_secs(2);
                while !stop.load(Ordering::Relaxed) {
                    let r = p.stream(&s, tf, &mut |item| {
                        if let Live::Bar(b) = item {
                            let _ = tx.send((k, Ok(b)));
                        }
                        !stop.load(Ordering::Relaxed)
                    });
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let e = r.err().unwrap_or(Error::Offline);
                    let _ = tx.send((k, Err(named(&s, e))));
                    let until = std::time::Instant::now() + backoff;
                    while std::time::Instant::now() < until && !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            }));
        }
        drop(tx);
        let mut last: Vec<Option<Bar>> = vec![None; self.legs.len()];
        let mut failing: Vec<bool> = vec![false; self.legs.len()];
        let result = loop {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok((k, Ok(b))) => {
                    last[k] = Some(b);
                    failing[k] = false;
                    // Once every leg has a price: legs behind the newest
                    // bar count at their last close.
                    let Some(t) = live.used.iter().filter_map(|&i| last[i]).map(|b| b.time).max() else {
                        continue;
                    };
                    let row: Vec<Option<Bar>> = last
                        .iter()
                        .map(|b| {
                            b.map(|b| {
                                if b.time == t {
                                    b
                                } else {
                                    Bar {
                                        time: t,
                                        open: b.close,
                                        high: b.close,
                                        low: b.close,
                                        close: b.close,
                                        volume: 0.0,
                                    }
                                }
                            })
                        })
                        .collect();
                    if let Some(bar) = combine_at(&live, &row, t)
                        && !sink(Live::Bar(bar))
                    {
                        break Ok(());
                    }
                }
                Ok((k, Err(e))) => {
                    failing[k] = true;
                    if live.used.iter().all(|&i| failing[i]) {
                        break Err(e);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if !sink(Live::Idle) {
                        break Ok(());
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break Err(Error::Offline),
            }
        };
        stop.store(true, Ordering::Relaxed);
        for t in threads {
            let _ = t.join();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A member that sends `bars` (one every 20 ms), then fails with
    /// `fail` (or keeps idling when there's none).
    struct Fake {
        bars: Vec<f64>,
        fail: Option<&'static str>,
    }

    impl Provider for Fake {
        fn natives(&self) -> Vec<Timeframe> {
            vec![Timeframe::parse("1D").unwrap()]
        }
        fn latest(&self, _: &Symbol, _: Timeframe, _: usize) -> Result<Vec<Bar>> {
            Ok(Vec::new())
        }
        fn since(&self, _: &Symbol, _: Timeframe, _: i64) -> Result<Vec<Bar>> {
            Ok(Vec::new())
        }
        fn before(&self, _: &Symbol, _: Timeframe, _: i64, _: usize) -> Result<Vec<Bar>> {
            Ok(Vec::new())
        }
        fn stream(&self, _: &Symbol, _: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()> {
            for &c in &self.bars {
                std::thread::sleep(Duration::from_millis(20));
                let b = Bar {
                    time: 0,
                    open: c,
                    high: c,
                    low: c,
                    close: c,
                    volume: 1.0,
                };
                if !sink(Live::Bar(b)) {
                    return Ok(());
                }
            }
            match self.fail {
                Some(m) => Err(Error::Down(m.into())),
                None => loop {
                    std::thread::sleep(Duration::from_millis(20));
                    if !sink(Live::Idle) {
                        return Ok(());
                    }
                },
            }
        }
    }

    fn index_of(legs: Vec<(&str, Fake)>) -> ExprProvider {
        let mut p = ExprProvider::from_segments(&[(i64::MIN, "1*A + 1*B".into())], true).unwrap();
        p.legs = legs
            .into_iter()
            .map(|(id, f)| (Symbol::bare(id), Arc::new(f) as Arc<dyn Provider>))
            .collect();
        p
    }

    #[test]
    fn one_failing_member_doesnt_stop_the_index() {
        let tf = Timeframe::parse("1D").unwrap();
        // B fails after its first price; A keeps going. The index keeps
        // updating with B at its last price.
        let p = index_of(vec![
            (
                "A",
                Fake {
                    bars: vec![10.0, 11.0, 12.0, 13.0, 14.0],
                    fail: None,
                },
            ),
            (
                "B",
                Fake {
                    bars: vec![5.0],
                    fail: Some("Yahoo isn't answering right now (error 502)."),
                },
            ),
        ]);
        let mut closes = Vec::new();
        let r = p.stream(&Symbol::bare("X"), tf, &mut |item| {
            if let Live::Bar(b) = item {
                closes.push(b.close);
            }
            closes.last() != Some(&19.0)
        });
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(closes.last(), Some(&19.0), "{closes:?}");
        // Every member failing: the stream ends with the real error, named.
        let p = index_of(vec![
            (
                "A",
                Fake {
                    bars: vec![10.0],
                    fail: Some("Yahoo is too slow to answer right now."),
                },
            ),
            (
                "B",
                Fake {
                    bars: vec![5.0],
                    fail: Some("Yahoo is too slow to answer right now."),
                },
            ),
        ]);
        let e = p.stream(&Symbol::bare("X"), tf, &mut |_| true).unwrap_err().to_string();
        assert!(e.ends_with(": Yahoo is too slow to answer right now."), "{e}");
        assert!(e.starts_with('A') || e.starts_with('B'), "{e}");
    }

    #[test]
    fn currencies_from_ids() {
        assert_eq!(guess_currency("NSE:INFY"), "INR");
        assert_eq!(guess_currency("NASDAQ:AAPL"), "USD");
        assert_eq!(guess_currency("FX:EURUSD"), "USD");
        assert_eq!(guess_currency("OANDA:EUR_JPY"), "JPY");
        assert_eq!(guess_currency("BINANCE:BTCUSDT"), "USDT");
        assert_eq!(guess_currency("COINBASE:BTC-EUR"), "EUR");
        assert_eq!(guess_currency("YAHOO:VOD.L"), "");
    }

    #[test]
    fn an_index_member_missing_a_day_counts_at_its_last_close() {
        let p = ExprProvider::from_segments(&[(i64::MIN, "1*A + 2*B".into())], false).unwrap();
        let flat = |t: i64, c: f64| Bar {
            time: t,
            open: c,
            high: c,
            low: c,
            close: c,
            volume: 1.0,
        };
        let a = vec![flat(1, 10.0), flat(2, 11.0), flat(3, 12.0)];
        let b = vec![flat(1, 5.0), flat(3, 6.0)];
        let out = combine_segs(&p.segs, &[a, b]);
        assert_eq!(out.iter().map(|b| b.close).collect::<Vec<_>>(), vec![20.0, 21.0, 24.0]);
        // Before B's first price there's no index.
        let late = vec![flat(2, 5.0)];
        let out = combine_segs(&p.segs, &[vec![flat(1, 10.0), flat(2, 11.0)], late]);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn an_index_whose_weights_change_mid_month_builds_its_months_from_days() {
        const DAY: i64 = 86_400_000;
        // 2026-03-01 at 00:00 UTC; A rises 100 → 200 over March and April,
        // B stays at 50. On 16 March the index moves from A to B, keeping
        // its level (units = A's close on the 15th ÷ 50).
        let t0 = 20_513 * DAY;
        let a: Vec<Bar> = (0..61)
            .map(|d| {
                let c = 100.0 + d as f64 * 100.0 / 60.0;
                Bar {
                    time: t0 + d * DAY,
                    open: c,
                    high: c,
                    low: c,
                    close: c,
                    volume: 1.0,
                }
            })
            .collect();
        let b: Vec<Bar> = (0..61)
            .map(|d| Bar {
                time: t0 + d * DAY,
                open: 50.0,
                high: 50.0,
                low: 50.0,
                close: 50.0,
                volume: 1.0,
            })
            .collect();
        let level = a[14].close;
        let segs = [
            (i64::MIN, "1*A".to_string()),
            (t0 + 15 * DAY, format!("{}*B", level / 50.0)),
        ];
        let p = ExprProvider::from_segments(&segs, true).unwrap();
        // Weeks and months aren't taken from the members' own bars.
        let natives = p.natives();
        assert!(natives.contains(&Timeframe::parse("1D").unwrap()));
        assert!(!natives.contains(&Timeframe::parse("1M").unwrap()), "{natives:?}");
        assert!(!natives.contains(&Timeframe::parse("1W").unwrap()));
        // A plain formula keeps them (one set of weights throughout).
        let plain = ExprProvider::from_formula("1*A + 1*B", true).unwrap();
        assert!(plain.natives().contains(&Timeframe::parse("1M").unwrap()));

        let m = Timeframe::parse("1M").unwrap();
        let utc = jiff::tz::TimeZone::UTC;
        // From the index's days: March ends at the level kept since the
        // 16th, April stays there.
        let days = combine_segs(&p.segs, &[a.clone(), b.clone()]);
        let months = crate::resample::resample(&days, m, &utc);
        assert_eq!(months.len(), 2);
        assert!((months[0].close - level).abs() < 1e-9, "{months:?}");
        assert!((months[1].close - level).abs() < 1e-9);
        // The members' monthly bars, combined, value March with A's weights
        // at its end: the old way, wrong by the whole rise after the 15th.
        let by_month = combine_segs(
            &p.segs,
            &[
                crate::resample::resample(&a, m, &utc),
                crate::resample::resample(&b, m, &utc),
            ],
        );
        assert!((by_month[0].close - a[30].close).abs() < 1e-9);
        assert!((by_month[0].close - level).abs() > 20.0);
    }

    fn bar(t: i64, o: f64, h: f64, l: f64, c: f64) -> Bar {
        Bar {
            time: t,
            open: o,
            high: h,
            low: l,
            close: c,
            volume: 5.0,
        }
    }

    #[test]
    fn parses_formulas_and_tickers_with_hyphens() {
        assert!(is_expr("BTCUSDT/ETHUSDT"));
        assert!(is_expr("AAPL - MSFT"));
        assert!(!is_expr("BRK-B"));
        assert!(!is_expr("NASDAQ:AAPL"));
        let (_, legs) = parse("0.0000012*NSE:M&M + 3*NSE:BAJAJ-AUTO").unwrap();
        assert_eq!(legs, ["NSE:M&M", "NSE:BAJAJ-AUTO"]);
        let (e, legs) = parse("(BTCUSDT + ETHUSDT) / 2").unwrap();
        assert_eq!(legs, ["BTCUSDT", "ETHUSDT"]);
        assert_eq!(e.eval(&[10.0, 20.0]), 15.0);
        let (e, legs) = parse("NYSE:BRK-B / SPY").unwrap();
        assert_eq!(legs, ["NYSE:BRK-B", "SPY"]);
        assert_eq!(e.eval(&[4.0, 2.0]), 2.0);
        let (e, _) = parse("-AAPL * 2 + 1").unwrap();
        assert_eq!(e.eval(&[3.0]), -5.0);
        assert!(parse("AAPL +").is_none());
        assert!(parse("(AAPL").is_none());
        assert!(parse("2 * 3").is_none(), "needs a symbol");
    }

    #[test]
    fn renders_back_with_brackets_only_where_needed() {
        for s in ["A/B", "(A + B)/2", "A - (B - C)", "A/(B*C)", "A*B + C"] {
            let (e, legs) = parse(s).unwrap();
            assert_eq!(render(&e, &|i| legs[i].clone()), s);
        }
    }

    #[test]
    fn resolves_bare_tickers() {
        let known = opentern_core::starter_watchlist();
        assert_eq!(resolve("ETHUSDT", &known), "BINANCE:ETHUSDT");
        assert_eq!(resolve("AAPL", &known), "NASDAQ:AAPL");
        assert_eq!(resolve("DOGEUSDT", &known), "BINANCE:DOGEUSDT");
        assert_eq!(resolve("MSFT", &known), "YAHOO:MSFT");
        let s = symbol("btcusdt/ethusdt", &known).unwrap();
        assert_eq!(s.id, "BINANCE:BTCUSDT/BINANCE:ETHUSDT");
        assert_eq!(s.description, "BTCUSDT/ETHUSDT");
        assert!(is_expr(&s.id));
    }

    #[test]
    fn combines_on_shared_times_with_true_extremes() {
        let (e, _) = parse("A/B").unwrap();
        let a = vec![bar(0, 10.0, 12.0, 9.0, 11.0), bar(1, 11.0, 11.0, 11.0, 11.0)];
        let b = vec![bar(0, 2.0, 4.0, 1.0, 2.0), bar(2, 1.0, 1.0, 1.0, 1.0)];
        let out = combine(&e, &[a, b]);
        assert_eq!(out.len(), 1, "only times both have");
        let x = out[0];
        assert_eq!((x.open, x.close), (5.0, 5.5));
        // H/H = 3, L/L = 9: the high and low cover both.
        assert_eq!((x.high, x.low), (9.0, 3.0));
    }

    #[test]
    fn segments_switch_weights_on_their_day() {
        let segs = [(i64::MIN, "2*A".to_string()), (10, "B + A".to_string())];
        let p = ExprProvider::from_segments(&segs, true).unwrap();
        let a = vec![bar(5, 1.0, 1.0, 1.0, 1.0), bar(10, 1.0, 1.0, 1.0, 1.0)];
        // B starts later: before its segment it isn't needed.
        let b = vec![bar(10, 3.0, 3.0, 3.0, 3.0)];
        let out = combine_segs(&p.segs, &[a, b]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].close, 2.0);
        assert_eq!(out[1].close, 4.0);
    }

    #[test]
    fn demo_formula_streams() {
        let known = opentern_core::starter_watchlist();
        let s = symbol("BTCUSDT/ETHUSDT", &known).unwrap();
        let p = ExprProvider::new(&s, true).unwrap();
        let tf = Timeframe::parse("1m").unwrap();
        let hist = p.latest(&s, tf, 100).unwrap();
        assert_eq!(hist.len(), 100);
        assert!(hist[0].close > 5.0 && hist[0].close < 50.0, "BTC/ETH around 20");
        let mut got = 0;
        p.stream(&s, tf, &mut |item| {
            if let Live::Bar(_) = item {
                got += 1;
            }
            got < 2
        })
        .unwrap();
        assert_eq!(got, 2);
    }
}

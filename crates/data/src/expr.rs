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
        match self {
            Expr::Num(v) => *v,
            Expr::Leg(i) => legs[*i],
            Expr::Neg(e) => -e.eval(legs),
            Expr::Bin(op, a, b) => {
                let (a, b) = (a.eval(legs), b.eval(legs));
                match op {
                    '+' => a + b,
                    '-' => a - b,
                    '*' => a * b,
                    _ => {
                        if b == 0.0 {
                            f64::NAN
                        } else {
                            a / b
                        }
                    }
                }
            }
        }
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
}

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
        if let Some(Tok::Op('-')) = self.peek() {
            self.pos += 1;
            return Some(Expr::Neg(Box::new(self.unary()?)));
        }
        self.atom()
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
    let mut p = Parser {
        toks: lex(s)?,
        pos: 0,
        legs: Vec::new(),
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
        Expr::Bin(op, a, b) => {
            let p = prec(e);
            // The right side of - and / needs brackets at equal precedence.
            let right = if matches!(op, '-' | '/') { p + 1 } else { p };
            let sep = if *op == '-' || *op == '+' {
                format!(" {op} ")
            } else {
                op.to_string()
            };
            format!("{}{sep}{}", wrap(a, p, name), wrap(b, right, name))
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

/// Custom indices (`CUSTOM:NAME`): the app registers each one's formula
/// here, and the hub charts and quotes it like that formula.
fn customs() -> &'static std::sync::RwLock<std::collections::HashMap<String, String>> {
    static C: std::sync::OnceLock<std::sync::RwLock<std::collections::HashMap<String, String>>> =
        std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

pub const CUSTOM: &str = "CUSTOM:";

pub fn is_custom(id: &str) -> bool {
    id.starts_with(CUSTOM)
}

/// Replaces every custom index (id → formula).
pub fn set_customs(all: impl IntoIterator<Item = (String, String)>) {
    if let Ok(mut m) = customs().write() {
        *m = all.into_iter().collect();
    }
}

/// A custom index's formula.
pub fn custom_formula(id: &str) -> Option<String> {
    if !is_custom(id) {
        return None;
    }
    customs().read().ok()?.get(id).cloned()
}

/// A provider for a formula: runs the legs' providers and combines.
pub struct ExprProvider {
    expr: Expr,
    legs: Vec<(Symbol, Arc<dyn Provider>)>,
}

impl ExprProvider {
    pub fn new(sym: &Symbol, demo: bool) -> Option<ExprProvider> {
        ExprProvider::from_formula(&sym.id, demo)
    }

    pub fn from_formula(formula: &str, demo: bool) -> Option<ExprProvider> {
        let (expr, ids) = parse(formula)?;
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
        Some(ExprProvider { expr, legs })
    }

    fn each(&self, f: impl Fn(&Symbol, &dyn Provider) -> Result<Vec<Bar>>) -> Result<Vec<Bar>> {
        let mut all = Vec::new();
        for (s, p) in &self.legs {
            all.push(f(s, p.as_ref())?);
        }
        Ok(combine(&self.expr, &all))
    }
}

impl Provider for ExprProvider {
    fn natives(&self) -> Vec<Timeframe> {
        let mut it = self.legs.iter().map(|(_, p)| p.natives());
        let first = it.next().unwrap_or_default();
        let rest: Vec<Vec<Timeframe>> = it.collect();
        first
            .into_iter()
            .filter(|t| rest.iter().all(|r| r.contains(t)))
            .collect()
    }

    fn latest(&self, _: &Symbol, tf: Timeframe, n: usize) -> Result<Vec<Bar>> {
        self.each(|s, p| p.latest(s, tf, n))
    }

    fn since(&self, _: &Symbol, tf: Timeframe, start: i64) -> Result<Vec<Bar>> {
        self.each(|s, p| p.since(s, tf, start))
    }

    fn before(&self, _: &Symbol, tf: Timeframe, end: i64, n: usize) -> Result<Vec<Bar>> {
        self.each(|s, p| p.before(s, tf, end, n))
    }

    fn stream(&self, _: &Symbol, tf: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()> {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = unbounded::<(usize, Option<Bar>)>();
        let mut threads = Vec::new();
        for (k, (s, p)) in self.legs.iter().enumerate() {
            let (s, p, tx, stop) = (s.clone(), p.clone(), tx.clone(), stop.clone());
            threads.push(std::thread::spawn(move || {
                let r = p.stream(&s, tf, &mut |item| {
                    if let Live::Bar(b) = item {
                        let _ = tx.send((k, Some(b)));
                    }
                    !stop.load(Ordering::Relaxed)
                });
                // Tell the combiner this leg's stream ended.
                let _ = tx.send((k, None));
                r
            }));
        }
        drop(tx);
        let mut last: Vec<Option<Bar>> = vec![None; self.legs.len()];
        let result = loop {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok((k, Some(b))) => {
                    last[k] = Some(b);
                    // Emit once every leg has a bar at the newest time.
                    let newest = last.iter().flatten().map(|b| b.time).max();
                    if let Some(t) = newest
                        && last.iter().all(|b| b.is_some_and(|b| b.time == t))
                    {
                        let row: Vec<Bar> = last.iter().flatten().copied().collect();
                        if let Some(bar) = combine_one(&self.expr, &row)
                            && !sink(Live::Bar(bar))
                        {
                            break Ok(());
                        }
                    }
                }
                Ok((_, None)) => break Err(Error::Offline),
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

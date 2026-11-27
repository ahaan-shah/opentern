//! `request.security` and `request.security_lower_tf`.
//!
//! A request runs its expression (and the script's variables it needs, see
//! `Sec::prelude`) on other bars, in a frame of its own. Each symbol and
//! timeframe a call site asks for is a *context* with its own bars and
//! state, so a request in a loop over symbols or timeframes keeps them
//! apart. A context catches up with every bar it hasn't seen, so a request
//! in a branch or a function that isn't reached on each bar still sees all
//! of them.
//!
//! The chart's own symbol at the same or a longer interval is built from
//! the chart's bars; anything else comes from `Env::feeds`.

use crate::compile::{Sec, SecBody};
use crate::feed_key;
use crate::run::{Frame, G, Ring, Stop, X, close_in, day_of, market_of, open_in};
use crate::value::V;
use crate::{
    Env,
    natives::{HEIKIN_ASHI, parse_tf},
};
use opentern_core::calendar::Market;
use opentern_core::{Bar, Timeframe, Unit};
use std::sync::Arc;

/// The most contexts one call site may ask for.
const MAX_CONTEXTS: usize = 256;

/// One symbol and timeframe of a call site.
#[derive(Clone, Debug)]
pub(crate) struct SecState {
    frame: Frame,
    bars: Ring<Bar>,
    /// Its finished bars, for requests inside this one.
    all: Arc<Vec<Bar>>,
    /// Heikin Ashi bars: the last finished one's open and close.
    ha: Option<Option<(f64, f64)>>,
    /// The newest bar as it came.
    raw: Option<Bar>,
    /// The period being built from chart bars: its key and real end.
    open: Option<(i64, i64)>,
    /// The last period that's over.
    done: i64,
    count: usize,
    bar_no: u32,
    /// The last finished bar's value.
    last: V,
    tf: Timeframe,
    /// How many source bars have been run.
    k: usize,
    /// What it returned on the chart bar it last ran on.
    out: Option<(usize, V)>,
    /// The last bar's day, and the day's first bar.
    day: (i64, i64),
    new_session: bool,
    /// Another symbol's surroundings.
    env: Option<Arc<Env>>,
    tz: jiff::tz::TimeZone,
    market: Market,
}

impl SecState {
    pub fn refs(&self, out: &mut Vec<u32>) {
        self.frame.refs(out);
        match &self.last {
            V::Ref(r) => out.push(*r),
            V::Tuple(t) => out.extend(t.iter().filter_map(|v| match v {
                V::Ref(r) => Some(*r),
                _ => None,
            })),
            _ => {}
        }
    }

    /// Its period is over.
    fn close(&mut self) {
        if let Some((k, _)) = self.open.take() {
            self.done = k;
        }
    }

    /// A bar as the expression sees it.
    fn shown(&self, b: &Bar) -> Bar {
        let Some(prev) = self.ha else { return *b };
        let close = (b.open + b.high + b.low + b.close) / 4.0;
        let open = match prev {
            Some((o, c)) => (o + c) / 2.0,
            None => (b.open + b.close) / 2.0,
        };
        Bar {
            open,
            high: b.high.max(open).max(close),
            low: b.low.min(open).min(close),
            close,
            ..*b
        }
    }

    /// A new bar of this context.
    fn push(&mut self, b: Bar) {
        self.raw = Some(b);
        self.bars.push(self.shown(&b));
        self.count += 1;
        let day = day_of(self.tf, b.time, &self.tz);
        self.new_session = !self.tf.is_intraday() || day != self.day.0;
        if day != self.day.0 {
            self.day = (day, b.time);
        }
    }
}

/// A call site's contexts.
#[derive(Clone, Debug, Default)]
pub(crate) struct Contexts(Vec<Context>);

#[derive(Clone, Debug)]
struct Context {
    symbol: Option<Arc<str>>,
    ha: bool,
    tf: Timeframe,
    state: Option<Box<SecState>>,
}

impl Contexts {
    pub fn refs(&self, out: &mut Vec<u32>) {
        self.0.iter().filter_map(|c| c.state.as_ref()).for_each(|s| s.refs(out));
    }
}

/// Rough length, to tell higher from lower timeframes.
pub(crate) fn approx_ms(tf: Timeframe) -> i64 {
    tf.millis().unwrap_or(match tf.unit {
        Unit::Month => tf.count as i64 * 30 * 86_400_000,
        _ => 86_400_000,
    })
}

fn truthy(v: &V) -> bool {
    matches!(v, V::Bool(true)) || v.as_str().is_some_and(|s| s.ends_with("_on"))
}

pub(crate) fn security(x: &mut X, sec: &Sec, f: &mut Frame, root: Option<&Frame>) -> Result<V, Stop> {
    let tf_v = x.eval(&sec.tf, f, root)?;
    let tf_s = tf_v.as_str().unwrap_or("").trim();
    let here = x.g.tf;
    let tf = if tf_s.is_empty() {
        here
    } else {
        parse_tf(tf_s).ok_or_else(|| Stop::Err(format!("\"{tf_s}\" isn't a timeframe.")))?
    };
    let mut ha = false;
    let symbol: Option<Arc<str>> = match &sec.symbol {
        Some(e) => {
            let v = x.eval(e, f, root)?;
            let s = v.as_str().unwrap_or("").trim();
            let s = match s.strip_prefix(HEIKIN_ASHI) {
                Some(rest) => {
                    ha = true;
                    rest
                }
                None => s,
            };
            let env = &x.g.env;
            (!(s.is_empty() || s == env.tickerid || s == env.ticker())).then(|| Arc::from(s))
        }
        None => None,
    };
    let body = sec.body(x.p);
    let gaps = !sec.lower && truthy(&x.eval(&sec.gaps, f, root)?);
    let lookahead = !sec.lower && truthy(&x.eval(&sec.lookahead, f, root)?);
    let fed = sec.lower || symbol.is_some() || approx_ms(tf) < approx_ms(here);

    let contexts = &mut f.secs[sec.site as usize].0;
    let at = match contexts
        .iter()
        .position(|c| c.tf == tf && c.ha == ha && c.symbol == symbol)
    {
        Some(k) => k,
        None => {
            if contexts.len() >= MAX_CONTEXTS {
                return Err(Stop::Err(format!(
                    "This request.security() asks for more than {MAX_CONTEXTS} symbols and timeframes."
                )));
            }
            contexts.push(Context {
                symbol: symbol.clone(),
                ha,
                tf,
                state: None,
            });
            contexts.len() - 1
        }
    };
    let mut st = match contexts[at].state.take() {
        Some(s) => s,
        None => Box::new(new_state(x, body, symbol.as_deref(), tf, ha)),
    };
    // Asked again on this bar (in a loop): the same answer.
    let now = x.g.all.len();
    let r = match &st.out {
        Some((k, v)) if *k == now && !sec.lower => Ok(v.clone()),
        _ => {
            let r = if fed {
                from_feed(x, body, &mut st, symbol.as_deref(), (gaps, lookahead, sec.lower))
            } else {
                from_chart(x, body, &mut st, gaps, lookahead)
            };
            if let Ok(v) = &r {
                st.out = Some((now, v.clone()));
            }
            r
        }
    };
    f.secs[sec.site as usize].0[at].state = Some(st);
    r
}

fn new_state(x: &X, sec: &SecBody, symbol: Option<&str>, tf: Timeframe, ha: bool) -> SecState {
    let g = &*x.g;
    let (env, tz, market) = match symbol {
        None => (None, g.tz.clone(), g.market),
        Some(s) => {
            let market = market_of(s.split_once(':').map_or("", |(p, _)| p), "");
            let (tz, kind) = match market {
                Market::Other => (g.tz.clone(), g.env.kind.as_str()),
                Market::Crypto => (market.tz(), "crypto"),
                Market::Forex => (market.tz(), "forex"),
                _ => (market.tz(), "stock"),
            };
            let env = Env {
                tickerid: s.to_string(),
                kind: kind.into(),
                libraries: Vec::new(),
                ..(*g.env).clone()
            };
            (Some(Arc::new(env)), tz, market)
        }
    };
    SecState {
        frame: Frame::new(&sec.layout),
        // Functions shared with the rest of the script look back as far as
        // the script does.
        bars: Ring::new(sec.bar_hist.max(x.p.bar_hist) + 1),
        all: Arc::default(),
        ha: ha.then_some(None),
        raw: None,
        open: None,
        done: i64::MIN,
        count: 0,
        bar_no: 0,
        last: V::Na,
        tf,
        k: 0,
        out: None,
        day: (i64::MIN, 0),
        new_session: false,
        env,
        tz,
        market,
    }
}

/// Evaluates the request with the context's bars standing in for the chart's.
fn run_on(x: &mut X, sec: &SecBody, st: &mut SecState, fr: &mut Frame) -> Result<V, Stop> {
    fn swap(g: &mut G, st: &mut SecState, nested: bool) {
        std::mem::swap(&mut g.bars, &mut st.bars);
        std::mem::swap(&mut g.day, &mut st.day);
        std::mem::swap(&mut g.new_session, &mut st.new_session);
        std::mem::swap(&mut g.tf, &mut st.tf);
        std::mem::swap(&mut g.tz, &mut st.tz);
        std::mem::swap(&mut g.market, &mut st.market);
        if let Some(e) = &mut st.env {
            std::mem::swap(&mut g.env, e);
        }
        if nested {
            std::mem::swap(&mut g.all, &mut st.all);
        }
    }
    let saved = (x.g.index, x.g.bar_no, x.g.line);
    swap(x.g, st, sec.nested);
    let quiet = std::mem::replace(&mut x.g.quiet, true);
    (x.g.index, x.g.bar_no) = (st.count.saturating_sub(1), st.bar_no + 1);
    let r = x
        .exec(&sec.prelude, fr, None)
        .and_then(|_| match &sec.expr {
            Some(e) => x.eval(e, fr, None),
            None => Ok(V::Na),
        })
        .and_then(|v| x.exec(&sec.post, fr, None).map(|_| v));
    swap(x.g, st, sec.nested);
    (x.g.index, x.g.bar_no) = (saved.0, saved.1);
    x.g.quiet = quiet;
    if r.is_ok() {
        x.g.line = saved.2;
    }
    r
}

/// Runs the context's newest bar for good.
fn finish(x: &mut X, sec: &SecBody, st: &mut SecState) -> Result<(), Stop> {
    let mut fr = std::mem::replace(&mut st.frame, Frame::new(&sec.layout));
    let v = run_on(x, sec, st, &mut fr);
    st.bar_no += 1;
    fr.commit(&sec.layout, &x.p.funcs, st.bar_no);
    st.frame = fr;
    st.last = v?;
    if st.ha.is_some()
        && let Some(b) = st.bars.get(0)
    {
        st.ha = Some(Some((b.open, b.close)));
    }
    if sec.nested
        && let Some(b) = st.bars.get(0).copied()
    {
        Arc::make_mut(&mut st.all).push(b);
    }
    Ok(())
}

/// Runs the context's newest bar as it stands, on a copy.
fn peek(x: &mut X, sec: &SecBody, st: &SecState) -> Result<V, Stop> {
    let mut tmp = st.clone();
    let mut fr = std::mem::replace(&mut tmp.frame, Frame::new(&sec.layout));
    run_on(x, sec, &mut tmp, &mut fr)
}

/// This symbol at the same or a longer interval, built from the bars here.
fn from_chart(x: &mut X, sec: &SecBody, st: &mut SecState, gaps: bool, lookahead: bool) -> Result<V, Stop> {
    let all = x.g.all.clone();
    let mut fresh = false;
    while st.k < all.len() {
        let b = all[st.k];
        st.k += 1;
        fold(x, sec, st, &b)?;
    }
    let bar = x.g.bar();
    let started = fold_open(x, sec, st, &bar, &mut fresh)?;
    let ended = ends(x, st, &bar);
    let value = if st.open.is_none() {
        st.last.clone()
    } else if ended && !x.g.forming {
        finish(x, sec, st)?;
        st.close();
        fresh = true;
        st.last.clone()
    } else if lookahead || x.g.forming {
        let v = peek(x, sec, st)?;
        if lookahead {
            fresh = started;
        }
        v
    } else {
        st.last.clone()
    };
    // This bar is done; the next call starts after it.
    st.k = all.len() + 1;
    Ok(if gaps && !fresh { V::Na } else { value })
}

/// Takes in a bar that's over.
fn fold(x: &mut X, sec: &SecBody, st: &mut SecState, b: &Bar) -> Result<(), Stop> {
    fold_open(x, sec, st, b, &mut false)?;
    if ends(x, st, b) {
        finish(x, sec, st)?;
        st.close();
    }
    Ok(())
}

/// Whether the period being built ends with bar `b`.
fn ends(x: &X, st: &SecState, b: &Bar) -> bool {
    st.open.is_some() && (st.tf == x.g.tf || st.open.is_some_and(|(_, end)| x.g.close_time(b.time) >= end))
}

/// Adds `b` to the period it belongs to, after finishing the one before.
/// Returns whether it started one.
fn fold_open(x: &mut X, sec: &SecBody, st: &mut SecState, b: &Bar, fresh: &mut bool) -> Result<bool, Stop> {
    let key = if st.tf == x.g.tf {
        b.time
    } else {
        x.g.bucket(st.tf, b.time)
    };
    if let Some((k, _)) = st.open
        && k != key
    {
        finish(x, sec, st)?;
        (st.open, st.done) = (None, k);
        *fresh = true;
    }
    // A bar after the period's close (a stock's after hours) isn't part of
    // it, nor a period of its own; nor, for days and longer, is one before
    // the session.
    if st.open.is_none() && key == st.done && st.tf != x.g.tf {
        return Ok(false);
    }
    if !st.tf.is_intraday() && x.g.tf.is_intraday() && !x.g.in_hours(b.time) {
        return Ok(false);
    }
    if st.open.is_some() {
        if let Some(h) = &mut st.raw {
            h.high = h.high.max(b.high);
            h.low = h.low.min(b.low);
            h.close = b.close;
            h.volume += b.volume;
            let h = *h;
            let shown = st.shown(&h);
            if let Some(top) = st.bars.get_mut(0) {
                *top = shown;
            }
        }
        return Ok(false);
    }
    st.push(Bar { time: key, ..*b });
    st.open = Some((key, close_in(st.tf, key, &st.tz, st.market)));
    Ok(true)
}

/// What `security_lower_tf`'s expression returns, to shape an empty result:
/// it runs once on a throwaway copy of the state, on this chart bar.
fn probe(x: &mut X, sec: &SecBody, st: &SecState) -> Result<V, Stop> {
    let mut tmp = st.clone();
    tmp.push(x.g.bar());
    peek(x, sec, &tmp)
}

/// `security_lower_tf`'s result: an array of the values, or for a tuple
/// expression a tuple of arrays, one per element (`shape` is any value
/// the expression returned).
fn lower_arrays(x: &mut X, vals: Vec<V>, shape: &V) -> V {
    let arr = |x: &mut X, v: Vec<V>| V::Ref(x.g.heap.alloc(crate::heap::Obj::Array(v)));
    match shape {
        V::Tuple(t) => {
            let cols: Vec<V> = (0..t.len())
                .map(|i| {
                    let col = vals
                        .iter()
                        .map(|v| match v {
                            V::Tuple(t) => t.get(i).cloned().unwrap_or(V::Na),
                            _ => V::Na,
                        })
                        .collect();
                    arr(x, col)
                })
                .collect();
            V::Tuple(cols.into())
        }
        _ => arr(x, vals),
    }
}

/// Another symbol, or a shorter interval, from a fetched feed.
fn from_feed(
    x: &mut X,
    sec: &SecBody,
    st: &mut SecState,
    symbol: Option<&str>,
    (gaps, lookahead, lower): (bool, bool, bool),
) -> Result<V, Stop> {
    let symbol = symbol.unwrap_or(&x.g.env.tickerid).to_string();
    let feed = x.g.env.feed(&feed_key(&symbol, st.tf));
    let Some(feed) = feed else {
        if !x.g.wanted.iter().any(|(s, t)| *s == symbol && *t == st.tf) {
            x.g.wanted.push((symbol, st.tf));
        }
        return Ok(if lower {
            let shape = probe(x, sec, st)?;
            lower_arrays(x, Vec::new(), &shape)
        } else {
            V::Na
        });
    };
    let bar = x.g.bar();
    let (from, to) = (x.g.open_time(bar.time), x.g.close_time(bar.time));
    let opens = |st: &SecState, b: &Bar| open_in(st.tf, b.time, &st.tz, st.market);
    let mut fresh = false;
    let mut inside = Vec::new();
    // Every feed bar that has finished by this chart bar's close.
    while let Some(b) = feed.get(st.k).copied() {
        if close_in(st.tf, b.time, &st.tz, st.market) > to {
            break;
        }
        st.push(b);
        finish(x, sec, st)?;
        if opens(st, &b) >= from {
            inside.push(st.last.clone());
        }
        st.k += 1;
        fresh = true;
    }
    // The next one, if it has started: what the forming bar (and
    // lookahead) sees, on a copy.
    let next = feed.get(st.k).copied().filter(|b| opens(st, b) < to);
    let ahead = |x: &mut X, st: &SecState, b: Bar| {
        let mut tmp = st.clone();
        tmp.push(b);
        peek(x, sec, &tmp)
    };
    if lower {
        if let Some(b) = next.filter(|_| x.g.forming) {
            inside.push(ahead(x, st, b)?);
        }
        let shape = match inside.first() {
            Some(v) => v.clone(),
            None => probe(x, sec, st)?,
        };
        return Ok(lower_arrays(x, inside, &shape));
    }
    let out = match next {
        Some(b) if x.g.forming || (lookahead && opens(st, &b) <= from) => {
            if lookahead {
                fresh = true;
            }
            ahead(x, st, b)?
        }
        _ => st.last.clone(),
    };
    Ok(if gaps && !fresh { V::Na } else { out })
}

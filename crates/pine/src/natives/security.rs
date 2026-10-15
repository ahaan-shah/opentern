//! `request.security`: the expression runs on another series' bars.
//!
//! **This chart's symbol on a higher timeframe:** the higher bars are built
//! from the chart's own bars (below). **Another symbol, or a lower
//! timeframe** (and `request.security_lower_tf`): the bars come from a
//! *feed* the app fetched into `Env::feeds`. A feed that isn't there yet
//! gives `na` and goes on `G::wanted`; the app fetches it and reruns the
//! script. A feed bar counts as finished once its period has ended by the
//! chart bar's close, the same rule as for built higher bars.
//!
//! Each finished higher-timeframe bar runs the expression once on the
//! call site's own frame, so `ta.*` state and `x[1]` inside it step once per
//! HTF bar, as on TradingView. What a chart bar sees:
//! - `lookahead_off` (the default): the last *finished* HTF bar's value. A
//!   chart bar that closes its HTF bar already shows that bar's value.
//!   The forming (realtime) bar shows the HTF bar so far, which repaints,
//!   as TradingView's realtime bars do.
//! - `lookahead_on`: the HTF bar so far. With `expr[1]` (the usual
//!   non-repainting idiom) that is exactly TradingView's value; a bare
//!   `expr` differs from TradingView's historical bars, which see the
//!   finished HTF bar (the future) there.
//! - `gaps_on`: na except on the chart bar where a new value arrives.
//!
//! Periods of daily and longer bars end at their market's last session
//! close (`run::period_end`), so a stock's "D" bar finishes on the chart's
//! last bar of the day, not on the next day's first.

use crate::compile::Sec;
use crate::feed_key;
use crate::natives::{bucket, parse_tf};
use crate::run::{Frame, Ring, Stop, X, market_of, period_end};
use crate::value::V;
use opentern_core::{Bar, Timeframe, Unit};

#[derive(Clone, Debug)]
pub(crate) struct SecState {
    frame: Frame,
    /// Finished HTF bars, then the forming one (newest first via `get`).
    bars: Ring<Bar>,
    /// The forming HTF bar's period start and end, if one is open.
    open: Option<(i64, i64)>,
    /// HTF bars started so far (the forming one's `bar_index` + 1).
    count: usize,
    bar_no: u32,
    /// The last finished HTF bar's value.
    last: V,
    tf: String,
    htf: Timeframe,
    /// Feeds: the feed's key, and how many of its bars have been run.
    key: String,
    k: usize,
}

impl SecState {
    pub fn refs(&self, out: &mut Vec<u32>) {
        self.frame.refs(out);
        if let V::Ref(r) = &self.last {
            out.push(*r);
        }
    }
}

/// Rough length, to tell higher from lower timeframes.
fn approx_ms(tf: Timeframe) -> i64 {
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
    let tf_s = tf_v.as_str().unwrap_or("").trim().to_string();
    let chart_tf = x.g.tf;
    let htf = if tf_s.is_empty() {
        chart_tf
    } else {
        parse_tf(&tf_s).ok_or_else(|| Stop::Err(format!("\"{tf_s}\" isn't a timeframe.")))?
    };
    let symbol = match &sec.symbol {
        Some(e) => {
            let v = x.eval(e, f, root)?;
            let s = v.as_str().unwrap_or("").trim().to_string();
            let env = &x.g.env;
            (!(s.is_empty() || s == env.tickerid || s == env.ticker())).then_some(s)
        }
        None => None,
    };
    let lower_tf = approx_ms(htf) < approx_ms(chart_tf);
    if sec.lower || symbol.is_some() || lower_tf {
        let sym = symbol.unwrap_or_else(|| x.g.env.tickerid.clone());
        return from_feed(x, sec, f, root, &sym, &tf_s, htf);
    }
    // The chart's own timeframe: just the expression, on the chart's bars.
    if htf == chart_tf {
        let st = state(f, sec, &tf_s, htf);
        let mut fr = std::mem::replace(&mut st.frame, Frame::new(&sec.layout));
        let v = x.eval(&sec.expr, &mut fr, root);
        fr.commit(&sec.layout, &x.p.funcs, x.g.bar_no);
        state(f, sec, &tf_s, htf).frame = fr;
        return v;
    }
    let lookahead = truthy(&x.eval(&sec.lookahead, f, root)?);
    let gaps = truthy(&x.eval(&sec.gaps, f, root)?);
    let bar = x.g.bar();
    let tz = x.g.tz.clone();
    let start = bucket(htf, bar.time, &tz);
    let chart_close = x.g.close_time(bar.time);

    let mut st = std::mem::replace(state(f, sec, &tf_s, htf), Box::new(empty(sec, &tf_s, htf)));
    let mut fresh = false;
    // A new HTF period: finish the previous bar if a chart bar didn't.
    if let Some((s, _)) = st.open
        && s != start
    {
        finish(x, sec, &mut st, root)?;
        fresh = true;
    }
    // Fold this chart bar into the forming HTF bar.
    let started = st.open.is_none();
    match st.open {
        Some(_) => {
            let b = st.bars.get_mut(0).expect("a forming bar");
            b.high = b.high.max(bar.high);
            b.low = b.low.min(bar.low);
            b.close = bar.close;
            b.volume += bar.volume;
        }
        None => {
            st.bars.push(Bar { time: start, ..bar });
            st.open = Some((start, period_end(htf, start, &tz, x.g.market)));
            st.count += 1;
        }
    }
    let ends_here = st.open.is_some_and(|(_, end)| chart_close >= end);
    let value = if ends_here && !x.g.forming {
        finish(x, sec, &mut st, root)?;
        fresh = true;
        st.last.clone()
    } else if lookahead || x.g.forming {
        // The HTF bar so far, on a copy of the state.
        let mut fr = st.frame.clone();
        let no = st.bar_no + 1;
        let v = run_on(x, sec, &mut st, &mut fr, no, htf, root)?;
        if lookahead {
            fresh = started;
        }
        v
    } else {
        st.last.clone()
    };
    *state(f, sec, &tf_s, htf) = st;
    Ok(if gaps && !fresh { V::Na } else { value })
}

fn empty(sec: &Sec, tf: &str, htf: Timeframe) -> SecState {
    SecState {
        frame: Frame::new(&sec.layout),
        bars: Ring::new(sec.bar_hist + 1),
        open: None,
        count: 0,
        bar_no: 0,
        last: V::Na,
        tf: tf.to_string(),
        htf,
        key: String::new(),
        k: 0,
    }
}

/// This call site's state (reset if the timeframe argument changed).
fn state<'a>(f: &'a mut Frame, sec: &Sec, tf: &str, htf: Timeframe) -> &'a mut Box<SecState> {
    let slot = &mut f.secs[sec.site as usize];
    if slot.as_ref().is_none_or(|s| s.tf != tf) {
        *slot = Some(Box::new(empty(sec, tf, htf)));
    }
    slot.as_mut().expect("set above")
}

/// Runs the expression on the finished forming bar and commits it.
fn finish(x: &mut X, sec: &Sec, st: &mut SecState, root: Option<&Frame>) -> Result<(), Stop> {
    st.bar_no += 1;
    let mut fr = std::mem::replace(&mut st.frame, Frame::new(&sec.layout));
    let (no, tf) = (st.bar_no, st.htf);
    let v = run_on(x, sec, st, &mut fr, no, tf, root);
    fr.commit(&sec.layout, &x.p.funcs, st.bar_no);
    st.frame = fr;
    st.last = v?;
    st.open = None;
    Ok(())
}

/// Evaluates the expression with the HTF bars standing in for the chart's.
#[allow(clippy::too_many_arguments)]
fn run_on(
    x: &mut X,
    sec: &Sec,
    st: &mut SecState,
    fr: &mut Frame,
    bar_no: u32,
    tf: Timeframe,
    root: Option<&Frame>,
) -> Result<V, Stop> {
    let g = &mut *x.g;
    std::mem::swap(&mut g.bars, &mut st.bars);
    let saved = (g.index, g.bar_no, g.tf);
    (g.index, g.bar_no, g.tf) = (st.count.saturating_sub(1), bar_no, tf);
    let v = x.eval(&sec.expr, fr, root);
    let g = &mut *x.g;
    std::mem::swap(&mut g.bars, &mut st.bars);
    (g.index, g.bar_no, g.tf) = saved;
    v
}

/// What `security_lower_tf`'s expression returns, to shape an empty result:
/// it runs once on a throwaway copy of the state, on this chart bar.
fn probe(x: &mut X, sec: &Sec, st: &SecState, ftf: Timeframe, root: Option<&Frame>) -> Result<V, Stop> {
    let mut tmp = st.clone();
    tmp.bars.push(x.g.bar());
    tmp.count += 1;
    let mut fr = tmp.frame.clone();
    let no = tmp.bar_no + 1;
    run_on(x, sec, &mut tmp, &mut fr, no, ftf, root)
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

/// `request.security` / `security_lower_tf` on a fetched feed.
fn from_feed(
    x: &mut X,
    sec: &Sec,
    f: &mut Frame,
    root: Option<&Frame>,
    symbol: &str,
    tf_s: &str,
    ftf: Timeframe,
) -> Result<V, Stop> {
    let key = feed_key(symbol, ftf);
    let feed = x.g.env.feeds.read().ok().and_then(|m| m.get(&key).cloned());
    let Some(feed) = feed else {
        if !x.g.wanted.iter().any(|(s, t)| s == symbol && *t == ftf) {
            x.g.wanted.push((symbol.to_string(), ftf));
        }
        return Ok(if sec.lower {
            let shape = probe(x, sec, &empty(sec, tf_s, ftf), ftf, root)?;
            lower_arrays(x, Vec::new(), &shape)
        } else {
            V::Na
        });
    };
    let lookahead = !sec.lower && truthy(&x.eval(&sec.lookahead, f, root)?);
    let gaps = !sec.lower && truthy(&x.eval(&sec.gaps, f, root)?);
    let bar = x.g.bar();
    let tz = x.g.tz.clone();
    let chart_close = x.g.close_time(bar.time);
    let market = {
        let prefix = symbol.split_once(':').map(|(p, _)| p).unwrap_or("");
        if symbol == x.g.env.tickerid {
            x.g.market
        } else {
            market_of(prefix, "")
        }
    };
    let end_of = |b: &Bar| period_end(ftf, b.time, &tz, market);
    let st = state(f, sec, tf_s, ftf);
    if st.key != key {
        **st = empty(sec, tf_s, ftf);
        st.key = key;
    }
    let mut st = std::mem::replace(st, Box::new(empty(sec, tf_s, ftf)));
    let mut fresh = false;
    let mut inside = Vec::new();
    // Run every feed bar that has finished by this chart bar's close.
    while let Some(b) = feed.get(st.k).copied() {
        if end_of(&b) > chart_close {
            break;
        }
        st.bars.push(b);
        st.count += 1;
        st.bar_no += 1;
        let mut fr = std::mem::replace(&mut st.frame, Frame::new(&sec.layout));
        let (no, tf) = (st.bar_no, st.htf);
        let v = run_on(x, sec, &mut st, &mut fr, no, tf, root);
        fr.commit(&sec.layout, &x.p.funcs, st.bar_no);
        st.frame = fr;
        let v = v?;
        if b.time >= bar.time {
            inside.push(v.clone());
        }
        st.last = v;
        st.k += 1;
        fresh = true;
    }
    // The next feed bar, if it has started: it's what the forming bar (and
    // lookahead) sees, on a copy.
    let next = feed.get(st.k).copied().filter(|b| b.time < chart_close);
    let peek = |x: &mut X, st: &SecState, b: Bar| -> Result<V, Stop> {
        let mut tmp = st.clone();
        tmp.bars.push(b);
        tmp.count += 1;
        let mut fr = tmp.frame.clone();
        let no = tmp.bar_no + 1;
        run_on(x, sec, &mut tmp, &mut fr, no, ftf, root)
    };
    let out = if sec.lower {
        if let Some(b) = next.filter(|_| x.g.forming) {
            inside.push(peek(x, &st, b)?);
        }
        let shape = match inside.first() {
            Some(v) => v.clone(),
            None => probe(x, sec, &st, ftf, root)?,
        };
        lower_arrays(x, inside, &shape)
    } else {
        match next {
            Some(b) if x.g.forming || (lookahead && b.time <= bar.time) => {
                if lookahead {
                    fresh = true;
                }
                peek(x, &st, b)?
            }
            _ => st.last.clone(),
        }
    };
    *state(f, sec, tf_s, ftf) = st;
    Ok(if gaps && !fresh { V::Na } else { out })
}

//! `ta.*`: thin wrappers over `opentern_series::ta`, the same Pine-exact
//! primitives the Rust built-in indicators use, so a Pine script and the
//! matching built-in agree bar for bar. Each call site keeps its own state;
//! when a length argument changes the state is rebuilt.

use super::{NatDef, arg, bool_or, d, int_or, len_arg, num};
use crate::run::G;
use crate::value::V;
use opentern_series::ta::{self as t, Hist};
use std::collections::VecDeque;

/// A call site's state.
#[derive(Clone, Debug)]
pub(crate) enum Ta {
    Last(V),
    Rng(u64),
    Int(i64),
    /// A primitive with the arguments it was built for.
    K(u64, Box<Prim>),
}

impl Ta {
    pub fn refs(&self, out: &mut Vec<u32>) {
        match self {
            Ta::Last(V::Ref(r)) => out.push(*r),
            Ta::K(_, p) => {
                if let Prim::Values(q, _) = &**p {
                    for v in q {
                        if let V::Ref(r) = v {
                            out.push(*r);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Prim {
    Sma(t::Sma),
    Smooth(t::Smooth),
    Wma(t::Wma),
    Vwma(t::Vwma),
    Hma(t::Hma),
    Alma(t::Alma),
    Linreg(t::Linreg),
    Stdev(t::Stdev),
    Dev(t::Dev),
    Ext(t::Extreme),
    Rsi(t::Rsi),
    Tr(t::Tr),
    Atr(t::Atr),
    Stoch(t::Stoch),
    Cci(t::Cci),
    Mfi(t::Mfi),
    Dmi(t::Dmi),
    Sar(t::Sar),
    St(t::Supertrend),
    Vwap(t::Vwap),
    Cross(t::Cross),
    Sum(t::Sum),
    /// The last n+1 values of a series (newest at the back).
    Ring(VecDeque<V>),
    /// A window of floats.
    Win(Hist),
    Win2(Hist, Hist),
    Macd(t::Smooth, t::Smooth, t::Smooth),
    Bb(t::Sma, t::Stdev),
    Kc(t::Smooth, t::Smooth, t::Tr),
    Tsi(t::Prev, t::Smooth, t::Smooth, t::Smooth, t::Smooth),
    Cmo(t::Prev, t::Sum, t::Sum),
    /// Running values (cum, obv, …): accumulator, previous close/volume.
    Acc(f64, f64, f64),
    /// `ta.valuewhen`: values when the condition held, newest at the back.
    Values(VecDeque<V>, usize),
    Count(i64),
}

type R = Result<V, String>;

fn key(xs: &[f64]) -> u64 {
    xs.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, x| {
        (h ^ x.to_bits()).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The call site's primitive, built (or rebuilt for new arguments) by `make`.
fn prim(st: &mut Option<Ta>, k: u64, make: impl FnOnce() -> Prim) -> &mut Prim {
    if !matches!(st, Some(Ta::K(x, _)) if *x == k) {
        *st = Some(Ta::K(k, Box::new(make())));
    }
    match st {
        Some(Ta::K(_, p)) => p,
        _ => unreachable!(),
    }
}

macro_rules! with {
    ($st:expr, $k:expr, $variant:ident, $make:expr) => {{
        let Prim::$variant(s) = prim($st, $k, || Prim::$variant($make)) else {
            unreachable!()
        };
        s
    }};
}

fn fv(x: f64) -> V {
    V::Float(x)
}

fn tuple(xs: &[f64]) -> V {
    V::Tuple(xs.iter().map(|x| V::Float(*x)).collect::<Vec<_>>().into())
}

fn len(a: &[V], k: usize) -> Result<usize, String> {
    len_arg(a, k, "The length")
}

pub(crate) fn sum(a: &[V], st: &mut Option<Ta>) -> R {
    let n = len(a, 1)?;
    Ok(fv(with!(st, n as u64, Sum, t::Sum::new(n)).update(num(a, 0))))
}

/// A float window of the last `n` values (including the current one).
fn window(st: &mut Option<Ta>, n: usize, x: f64) -> &Hist {
    let w = with!(st, n as u64, Win, Hist::new(n));
    w.push(x);
    w
}

fn full(h: &Hist, n: usize) -> Option<Vec<f64>> {
    let w: Vec<f64> = h.window(n)?.collect();
    Some(w)
}

pub(crate) fn defs() -> Vec<NatDef> {
    vec![
        d("ta.sma", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            Ok(fv(with!(st, n as u64, Sma, t::Sma::new(n)).update(num(a, 0))))
        })
        .state(),
        d("ta.ema", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            Ok(fv(with!(st, n as u64, Smooth, t::ema(n)).update(num(a, 0))))
        })
        .state(),
        d("ta.rma", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            Ok(fv(with!(st, n as u64, Smooth, t::rma(n)).update(num(a, 0))))
        })
        .state(),
        d("ta.wma", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            Ok(fv(with!(st, n as u64, Wma, t::Wma::new(n)).update(num(a, 0))))
        })
        .state(),
        d("ta.vwma", &["source", "length"], |g, a, st| {
            let n = len(a, 1)?;
            let v = g.bar().volume;
            Ok(fv(with!(st, n as u64, Vwma, t::Vwma::new(n)).update(num(a, 0), v)))
        })
        .state(),
        d("ta.hma", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            Ok(fv(with!(st, n as u64, Hma, t::Hma::new(n)).update(num(a, 0))))
        })
        .state(),
        d(
            "ta.alma",
            &["series", "length", "offset", "sigma", "floor"],
            |_, a, st| {
                let n = len(a, 1)?;
                let (off, sig) = (arg(a, 2).f_or(0.85), arg(a, 3).f_or(6.0));
                let fl = bool_or(a, 4, false);
                let k = key(&[n as f64, off, sig, fl as u8 as f64]);
                Ok(fv(with!(st, k, Alma, t::Alma::new(n, off, sig, fl)).update(num(a, 0))))
            },
        )
        .state(),
        d("ta.swma", &["source"], |_, a, st| {
            let w = window(st, 4, num(a, 0));
            Ok(fv(match full(w, 4) {
                Some(x) => x[3] / 6.0 + x[2] * 2.0 / 6.0 + x[1] * 2.0 / 6.0 + x[0] / 6.0,
                None => f64::NAN,
            }))
        })
        .state(),
        d("ta.linreg", &["source", "length", "offset"], |_, a, st| {
            let n = len(a, 1)?;
            let off = int_or(a, 2, 0);
            let k = key(&[n as f64, off as f64]);
            Ok(fv(with!(st, k, Linreg, t::Linreg::new(n, off)).update(num(a, 0))))
        })
        .state(),
        d("ta.stdev", &["source", "length", "biased"], |_, a, st| {
            let n = len(a, 1)?;
            let biased = bool_or(a, 2, true);
            let k = key(&[n as f64, biased as u8 as f64]);
            Ok(fv(with!(st, k, Stdev, t::Stdev::with(n, biased)).update(num(a, 0))))
        })
        .state(),
        d("ta.variance", &["source", "length", "biased"], |_, a, st| {
            let n = len(a, 1)?;
            let biased = bool_or(a, 2, true);
            let k = key(&[n as f64, biased as u8 as f64]);
            Ok(fv(with!(st, k, Stdev, t::Stdev::with(n, biased)).variance(num(a, 0))))
        })
        .state(),
        d("ta.dev", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            Ok(fv(with!(st, n as u64, Dev, t::Dev::new(n)).update(num(a, 0))))
        })
        .state(),
        d("ta.highest", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            Ok(fv(with!(st, n as u64, Ext, t::Extreme::highest(n)).update(num(a, 0))))
        })
        .state(),
        d("ta.lowest", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            Ok(fv(with!(st, n as u64, Ext, t::Extreme::lowest(n)).update(num(a, 0))))
        })
        .state(),
        d("ta.highestbars", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            let (_, back) = with!(st, n as u64, Ext, t::Extreme::highest(n)).update_with_offset(num(a, 0));
            Ok(if back.is_nan() { V::Na } else { V::Int(-(back as i64)) })
        })
        .state(),
        d("ta.lowestbars", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            let (_, back) = with!(st, n as u64, Ext, t::Extreme::lowest(n)).update_with_offset(num(a, 0));
            Ok(if back.is_nan() { V::Na } else { V::Int(-(back as i64)) })
        })
        .state(),
        d("ta.change", &["source", "length"], |_, a, st| {
            let n = int_or(a, 1, 1).max(0) as usize;
            let x = arg(a, 0).clone();
            let ring = with!(st, n as u64, Ring, VecDeque::new());
            ring.push_back(x.clone());
            if ring.len() > n + 1 {
                ring.pop_front();
            }
            if ring.len() < n + 1 {
                return Ok(match x {
                    V::Bool(_) => V::Bool(false),
                    _ => V::Na,
                });
            }
            let p = &ring[0];
            Ok(match (&x, p) {
                (V::Bool(a), V::Bool(b)) => V::Bool(a != b),
                (V::Int(a), V::Int(b)) => V::Int(a - b),
                _ => fv(x.f() - p.f()),
            })
        })
        .state(),
        d("ta.mom", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            let w = window(st, n + 1, num(a, 0));
            Ok(fv(if w.seen() > n { w.get(0) - w.get(n) } else { f64::NAN }))
        })
        .state(),
        d("ta.roc", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            let w = window(st, n + 1, num(a, 0));
            Ok(fv(if w.seen() > n {
                100.0 * t::div(w.get(0) - w.get(n), w.get(n))
            } else {
                f64::NAN
            }))
        })
        .state(),
        d("ta.rsi", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            Ok(fv(with!(st, n as u64, Rsi, t::Rsi::new(n)).update(num(a, 0))))
        })
        .state(),
        d("ta.tr", &["handle_na"], |g, a, st| {
            let h = bool_or(a, 0, false);
            let b = g.bar();
            Ok(fv(with!(st, h as u64, Tr, t::Tr::new(h)).update(b.high, b.low, b.close)))
        })
        .state()
        .var(),
        d("ta.atr", &["length"], |g, a, st| {
            let n = len(a, 0)?;
            let b = g.bar();
            Ok(fv(
                with!(st, n as u64, Atr, t::Atr::new(n)).update(b.high, b.low, b.close)
            ))
        })
        .state(),
        d("ta.stoch", &["source", "high", "low", "length"], |_, a, st| {
            let n = len(a, 3)?;
            Ok(fv(with!(st, n as u64, Stoch, t::Stoch::new(n)).update(
                num(a, 0),
                num(a, 1),
                num(a, 2),
            )))
        })
        .state(),
        d("ta.cci", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            Ok(fv(with!(st, n as u64, Cci, t::Cci::new(n)).update(num(a, 0))))
        })
        .state(),
        d("ta.cmo", &["series", "length"], |_, a, st| {
            let n = len(a, 1)?;
            let Prim::Cmo(prev, up, down) = prim(st, n as u64, || {
                Prim::Cmo(t::Prev::new(1), t::Sum::new(n), t::Sum::new(n))
            }) else {
                unreachable!()
            };
            let x = num(a, 0);
            let m = x - prev.update(x);
            // `mom >= 0` is false for na, so na lands in the second sum.
            let (u, dn) = if m >= 0.0 { (m, 0.0) } else { (0.0, -m) };
            let (s1, s2) = (up.update(u), down.update(dn));
            Ok(fv(100.0 * t::div(s1 - s2, s1 + s2)))
        })
        .state(),
        d("ta.mfi", &["series", "length"], |g, a, st| {
            let n = len(a, 1)?;
            let v = g.bar().volume;
            Ok(fv(with!(st, n as u64, Mfi, t::Mfi::new(n)).update(num(a, 0), v)))
        })
        .state(),
        d("ta.dmi", &["diLength", "adxSmoothing"], |g, a, st| {
            let (di, adx) = (len(a, 0)?, len(a, 1)?);
            let b = g.bar();
            let (p, m, x) =
                with!(st, key(&[di as f64, adx as f64]), Dmi, t::Dmi::new(di, adx)).update(b.high, b.low, b.close);
            Ok(tuple(&[p, m, x]))
        })
        .state(),
        d("ta.sar", &["start", "inc", "max"], |g, a, st| {
            let (s, i, m) = (num(a, 0), num(a, 1), num(a, 2));
            let b = g.bar();
            Ok(fv(
                with!(st, key(&[s, i, m]), Sar, t::Sar::new(s, i, m)).update(b.high, b.low, b.close)
            ))
        })
        .state(),
        d("ta.supertrend", &["factor", "atrPeriod"], |g, a, st| {
            let f = num(a, 0);
            let n = len(a, 1)?;
            let b = g.bar();
            let (v, dir) = with!(st, key(&[f, n as f64]), St, t::Supertrend::new(f, n)).update(b.high, b.low, b.close);
            Ok(V::Tuple(
                vec![fv(v), if dir.is_nan() { V::Na } else { V::Int(dir as i64) }].into(),
            ))
        })
        .state(),
        d("ta.vwap", &["source", "anchor", "stdev_mult"], |g, a, st| {
            let reset = match arg(a, 1) {
                V::Na => g.new_session,
                v => v.truthy(),
            };
            let reset = reset || g.index == 0;
            let vol = g.bar().volume;
            let (v, sd) = with!(st, 0, Vwap, t::Vwap::default()).update(num(a, 0), vol, reset);
            let m = num(a, 2);
            Ok(if m.is_nan() {
                fv(v)
            } else {
                tuple(&[v, v + sd * m, v - sd * m])
            })
        })
        .state()
        .var(),
        d("ta.crossover", &["source1", "source2"], |_, a, st| {
            let s = with!(st, 0, Cross, t::Cross::default());
            Ok(V::Bool(s.update(num(a, 0), num(a, 1)).0))
        })
        .state(),
        d("ta.crossunder", &["source1", "source2"], |_, a, st| {
            let s = with!(st, 0, Cross, t::Cross::default());
            Ok(V::Bool(s.update(num(a, 0), num(a, 1)).1))
        })
        .state(),
        d("ta.cross", &["source1", "source2"], |_, a, st| {
            let s = with!(st, 0, Cross, t::Cross::default());
            let (o, u) = s.update(num(a, 0), num(a, 1));
            Ok(V::Bool(o || u))
        })
        .state(),
        d("ta.cum", &["source"], |_, a, st| {
            let Prim::Acc(acc, _, _) = prim(st, 0, || Prim::Acc(0.0, f64::NAN, f64::NAN)) else {
                unreachable!()
            };
            let x = num(a, 0);
            if !x.is_nan() {
                *acc += x;
            }
            Ok(fv(*acc))
        })
        .state(),
        d("ta.macd", &["source", "fastlen", "slowlen", "siglen"], |_, a, st| {
            let (f, s, g) = (len(a, 1)?, len(a, 2)?, len(a, 3)?);
            let Prim::Macd(ef, es, sig) = prim(st, key(&[f as f64, s as f64, g as f64]), || {
                Prim::Macd(t::ema(f), t::ema(s), t::ema(g))
            }) else {
                unreachable!()
            };
            let x = num(a, 0);
            let m = ef.update(x) - es.update(x);
            let sg = sig.update(m);
            Ok(tuple(&[m, sg, m - sg]))
        })
        .state(),
        d("ta.bb", &["series", "length", "mult"], |_, a, st| {
            let (n, m) = (len(a, 1)?, num(a, 2));
            let (b, u, l) = bb(st, n, m, num(a, 0));
            Ok(tuple(&[b, u, l]))
        })
        .state(),
        d("ta.bbw", &["series", "length", "mult"], |_, a, st| {
            let (n, m) = (len(a, 1)?, num(a, 2));
            let (b, u, l) = bb(st, n, m, num(a, 0));
            Ok(fv(t::div(u - l, b)))
        })
        .state(),
        d("ta.kc", &["series", "length", "mult", "useTrueRange"], |g, a, st| {
            let (b, u, l) = kc(g, a, st)?;
            Ok(tuple(&[b, u, l]))
        })
        .state(),
        d("ta.kcw", &["series", "length", "mult", "useTrueRange"], |g, a, st| {
            let (b, u, l) = kc(g, a, st)?;
            Ok(fv(t::div(u - l, b)))
        })
        .state(),
        d("ta.wpr", &["length"], |g, a, st| {
            let n = len(a, 0)?;
            let b = g.bar();
            let Prim::Win2(h, l) = prim(st, n as u64, || Prim::Win2(Hist::new(n), Hist::new(n))) else {
                unreachable!()
            };
            h.push(b.high);
            l.push(b.low);
            let (Some(hs), Some(ls)) = (h.window(n), l.window(n)) else {
                return Ok(fv(f64::NAN));
            };
            let hi = hs.fold(f64::NAN, |m, x| if m.is_nan() || x > m { x } else { m });
            let lo = ls.fold(f64::NAN, |m, x| if m.is_nan() || x < m { x } else { m });
            Ok(fv(100.0 * t::div(b.close - hi, hi - lo)))
        })
        .state(),
        d("ta.obv", &[], |g, _, st| {
            Ok(fv(acc(g, st, |b, pc, _| {
                let ch = b.close - pc;
                let s = if ch > 0.0 {
                    1.0
                } else if ch < 0.0 {
                    -1.0
                } else if ch == 0.0 {
                    0.0
                } else {
                    f64::NAN
                };
                s * b.volume
            })))
        })
        .state()
        .var(),
        d("ta.accdist", &[], |g, _, st| {
            Ok(fv(acc(g, st, |b, _, _| {
                if (b.close == b.high && b.close == b.low) || b.high == b.low {
                    0.0
                } else {
                    (2.0 * b.close - b.low - b.high) / (b.high - b.low) * b.volume
                }
            })))
        })
        .state()
        .var(),
        d("ta.pvt", &[], |g, _, st| {
            Ok(fv(acc(g, st, |b, pc, _| (b.close - pc) / pc * b.volume)))
        })
        .state()
        .var(),
        d("ta.wad", &[], |g, _, st| {
            Ok(fv(acc(g, st, |b, pc, _| {
                let mom = b.close - pc;
                if mom > 0.0 {
                    b.close - t::min(b.low, pc)
                } else if mom < 0.0 {
                    b.close - t::max(b.high, pc)
                } else {
                    0.0
                }
            })))
        })
        .state()
        .var(),
        d("ta.iii", &[], |g, _, _| {
            let b = g.bar();
            Ok(fv(t::div(2.0 * b.close - b.high - b.low, (b.high - b.low) * b.volume)))
        })
        .var(),
        d("ta.wvad", &[], |g, _, _| {
            let b = g.bar();
            Ok(fv(t::div(b.close - b.open, b.high - b.low) * b.volume))
        })
        .var(),
        d("ta.nvi", &[], |g, _, st| Ok(fv(vi(g, st, true)))).state().var(),
        d("ta.pvi", &[], |g, _, st| Ok(fv(vi(g, st, false)))).state().var(),
        d("ta.tsi", &["source", "short_length", "long_length"], |_, a, st| {
            let (s, l) = (len(a, 1)?, len(a, 2)?);
            let Prim::Tsi(prev, a1, a2, b1, b2) = prim(st, key(&[s as f64, l as f64]), || {
                Prim::Tsi(t::Prev::new(1), t::ema(l), t::ema(s), t::ema(l), t::ema(s))
            }) else {
                unreachable!()
            };
            let x = num(a, 0);
            let pc = x - prev.update(x);
            let num_ = a2.update(a1.update(pc));
            let den = b2.update(b1.update(pc.abs()));
            Ok(fv(t::div(num_, den)))
        })
        .state(),
        d("ta.rising", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            let w = window(st, n + 1, num(a, 0));
            Ok(V::Bool(w.seen() > n && (1..=n).all(|i| w.get(0) > w.get(i))))
        })
        .state(),
        d("ta.falling", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            let w = window(st, n + 1, num(a, 0));
            Ok(V::Bool(w.seen() > n && (1..=n).all(|i| w.get(0) < w.get(i))))
        })
        .state(),
        d("ta.barssince", &["condition"], |_, a, st| {
            let Prim::Count(c) = prim(st, 0, || Prim::Count(-1)) else {
                unreachable!()
            };
            if arg(a, 0).truthy() {
                *c = 0;
            } else if *c >= 0 {
                *c += 1;
            }
            Ok(if *c < 0 { V::Na } else { V::Int(*c) })
        })
        .state(),
        d("ta.valuewhen", &["condition", "source", "occurrence"], |_, a, st| {
            let occ = int_or(a, 2, 0).max(0) as usize;
            let Prim::Values(q, cap) = prim(st, 0, || Prim::Values(VecDeque::new(), 0)) else {
                unreachable!()
            };
            *cap = (*cap).max(occ + 1);
            if arg(a, 0).truthy() {
                q.push_back(arg(a, 1).clone());
                while q.len() > (*cap).max(1).max(500) {
                    q.pop_front();
                }
            }
            Ok(if occ < q.len() {
                q[q.len() - 1 - occ].clone()
            } else {
                V::Na
            })
        })
        .state(),
        d("ta.pivothigh", &["source", "leftbars", "rightbars"], |_, a, st| {
            pivot(a, st, true)
        })
        .state(),
        d("ta.pivotlow", &["source", "leftbars", "rightbars"], |_, a, st| {
            pivot(a, st, false)
        })
        .state(),
        d("ta.percentrank", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            let w = window(st, n + 1, num(a, 0));
            if w.seen() <= n {
                return Ok(fv(f64::NAN));
            }
            let x = w.get(0);
            if x.is_nan() {
                return Ok(fv(f64::NAN));
            }
            let le = (1..=n).filter(|&i| w.get(i) <= x).count();
            Ok(fv(100.0 * le as f64 / n as f64))
        })
        .state(),
        d("ta.median", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            let w = window(st, n, num(a, 0));
            Ok(fv(match full(w, n) {
                Some(mut x) => median(&mut x),
                None => f64::NAN,
            }))
        })
        .state(),
        d("ta.mode", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            let w = window(st, n, num(a, 0));
            Ok(fv(match full(w, n) {
                Some(x) => mode(&x),
                None => f64::NAN,
            }))
        })
        .state(),
        d("ta.range", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            let w = window(st, n, num(a, 0));
            Ok(fv(match full(w, n) {
                Some(x) => {
                    let v: Vec<f64> = x.into_iter().filter(|v| !v.is_nan()).collect();
                    if v.is_empty() {
                        f64::NAN
                    } else {
                        v.iter().cloned().fold(f64::MIN, f64::max) - v.iter().cloned().fold(f64::MAX, f64::min)
                    }
                }
                None => f64::NAN,
            }))
        })
        .state(),
        d("ta.correlation", &["source1", "source2", "length"], |_, a, st| {
            let n = len(a, 2)?;
            let Prim::Win2(h1, h2) = prim(st, n as u64, || Prim::Win2(Hist::new(n), Hist::new(n))) else {
                unreachable!()
            };
            h1.push(num(a, 0));
            h2.push(num(a, 1));
            let (Some(x), Some(y)) = (h1.window(n), h2.window(n)) else {
                return Ok(fv(f64::NAN));
            };
            let x: Vec<f64> = x.collect();
            let y: Vec<f64> = y.collect();
            if x.iter().chain(&y).any(|v| v.is_nan()) {
                return Ok(fv(f64::NAN));
            }
            let nf = n as f64;
            let (mx, my) = (x.iter().sum::<f64>() / nf, y.iter().sum::<f64>() / nf);
            let cov: f64 = x.iter().zip(&y).map(|(a, b)| (a - mx) * (b - my)).sum();
            let vx: f64 = x.iter().map(|a| (a - mx).powi(2)).sum();
            let vy: f64 = y.iter().map(|b| (b - my).powi(2)).sum();
            Ok(fv(t::div(cov, (vx * vy).sqrt())))
        })
        .state(),
        d("ta.cog", &["source", "length"], |_, a, st| {
            let n = len(a, 1)?;
            let w = window(st, n, num(a, 0));
            Ok(fv(match full(w, n) {
                Some(x) => {
                    let s: f64 = x.iter().sum();
                    let ws: f64 = x.iter().enumerate().map(|(i, v)| v * (i as f64 + 1.0)).sum();
                    -t::div(ws, s)
                }
                None => f64::NAN,
            }))
        })
        .state(),
        d(
            "ta.percentile_linear_interpolation",
            &["source", "length", "percentage"],
            |_, a, st| {
                let n = len(a, 1)?;
                let p = num(a, 2);
                let w = window(st, n, num(a, 0));
                Ok(fv(match full(w, n) {
                    Some(x) => percentile_linear(&x, p),
                    None => f64::NAN,
                }))
            },
        )
        .state(),
        d(
            "ta.percentile_nearest_rank",
            &["source", "length", "percentage"],
            |_, a, st| {
                let n = len(a, 1)?;
                let p = num(a, 2);
                let w = window(st, n, num(a, 0));
                Ok(fv(match full(w, n) {
                    Some(x) => percentile_nearest(&x, p),
                    None => f64::NAN,
                }))
            },
        )
        .state(),
        d("ta.max", &["source"], |_, a, st| {
            let Prim::Acc(m, _, _) = prim(st, 0, || Prim::Acc(f64::NAN, 0.0, 0.0)) else {
                unreachable!()
            };
            let x = num(a, 0);
            if !x.is_nan() && (m.is_nan() || x > *m) {
                *m = x;
            }
            Ok(fv(*m))
        })
        .state(),
        d("ta.min", &["source"], |_, a, st| {
            let Prim::Acc(m, _, _) = prim(st, 0, || Prim::Acc(f64::NAN, 0.0, 0.0)) else {
                unreachable!()
            };
            let x = num(a, 0);
            if !x.is_nan() && (m.is_nan() || x < *m) {
                *m = x;
            }
            Ok(fv(*m))
        })
        .state(),
    ]
}

fn bb(st: &mut Option<Ta>, n: usize, m: f64, x: f64) -> (f64, f64, f64) {
    let Prim::Bb(s, d) = prim(st, n as u64, || Prim::Bb(t::Sma::new(n), t::Stdev::new(n))) else {
        unreachable!()
    };
    let b = s.update(x);
    let dev = m * d.update(x);
    (b, b + dev, b - dev)
}

fn kc(g: &mut G, a: &[V], st: &mut Option<Ta>) -> Result<(f64, f64, f64), String> {
    let n = len(a, 1)?;
    let m = num(a, 2);
    let use_tr = bool_or(a, 3, true);
    let Prim::Kc(basis, range, tr) = prim(st, n as u64, || Prim::Kc(t::ema(n), t::ema(n), t::Tr::new(false))) else {
        unreachable!()
    };
    let b = g.bar();
    let x = num(a, 0);
    let trv = tr.update(b.high, b.low, b.close);
    let r = if use_tr { trv } else { b.high - b.low };
    let bs = basis.update(x);
    let re = range.update(r);
    Ok((bs, bs + re * m, bs - re * m))
}

/// A running total of `f(bar, previous close, previous volume)`; `na`
/// terms count as 0 (as `ta.cum` does).
fn acc(g: &mut G, st: &mut Option<Ta>, f: impl Fn(&opentern_core::Bar, f64, f64) -> f64) -> f64 {
    let Prim::Acc(total, pc, pv) = prim(st, 0, || Prim::Acc(0.0, f64::NAN, f64::NAN)) else {
        unreachable!()
    };
    let b = g.bar();
    let x = f(&b, *pc, *pv);
    if !x.is_nan() {
        *total += x;
    }
    *pc = b.close;
    *pv = b.volume;
    *total
}

/// `ta.nvi` / `ta.pvi`.
fn vi(g: &mut G, st: &mut Option<Ta>, negative: bool) -> f64 {
    let Prim::Acc(prev, pc, pv) = prim(st, 0, || Prim::Acc(f64::NAN, f64::NAN, f64::NAN)) else {
        unreachable!()
    };
    let b = g.bar();
    let base = if t::nz(*prev, 0.0) == 0.0 { 1.0 } else { *prev };
    let nzc = t::nz(b.close, 0.0);
    let nzpc = t::nz(*pc, 0.0);
    let moved = if negative {
        b.volume < t::nz(*pv, 0.0)
    } else {
        b.volume > t::nz(*pv, 0.0)
    };
    let v = if nzc == 0.0 || nzpc == 0.0 {
        base
    } else if moved {
        base + (b.close - *pc) / *pc * base
    } else {
        base
    };
    *prev = v;
    *pc = b.close;
    *pv = b.volume;
    v
}

fn pivot(a: &[V], st: &mut Option<Ta>, high: bool) -> Result<V, String> {
    let l = int_or(a, 1, 0).max(0) as usize;
    let r = int_or(a, 2, 0).max(0) as usize;
    let n = l + r + 1;
    let w = window(st, n, num(a, 0));
    if w.seen() < n {
        return Ok(V::Na);
    }
    let c = w.get(r);
    if c.is_nan() {
        return Ok(V::Na);
    }
    let beats = |x: f64, strict: bool| {
        if x.is_nan() {
            return true;
        }
        match (high, strict) {
            (true, true) => c > x,
            (true, false) => c >= x,
            (false, true) => c < x,
            (false, false) => c <= x,
        }
    };
    // Left of the pivot must be strictly beaten; on the right, ties are
    // allowed (see docs/ta-semantics.md).
    let left = (r + 1..n).all(|i| beats(w.get(i), true));
    let right = (0..r).all(|i| beats(w.get(i), false));
    Ok(if left && right { V::Float(c) } else { V::Na })
}

pub(crate) fn median(x: &mut [f64]) -> f64 {
    let mut v: Vec<f64> = x.iter().cloned().filter(|v| !v.is_nan()).collect();
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

pub(crate) fn mode(x: &[f64]) -> f64 {
    let mut v: Vec<f64> = x.iter().cloned().filter(|v| !v.is_nan()).collect();
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let (mut best, mut best_n) = (v[0], 0);
    let mut i = 0;
    while i < v.len() {
        let mut j = i;
        while j < v.len() && v[j] == v[i] {
            j += 1;
        }
        if j - i > best_n {
            best_n = j - i;
            best = v[i];
        }
        i = j;
    }
    best
}

pub(crate) fn percentile_linear(x: &[f64], p: f64) -> f64 {
    let mut v: Vec<f64> = x.iter().cloned().filter(|v| !v.is_nan()).collect();
    if v.is_empty() || p.is_nan() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let rank = (p.clamp(0.0, 100.0) / 100.0) * (v.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    v[lo] + (v[hi] - v[lo]) * (rank - lo as f64)
}

pub(crate) fn percentile_nearest(x: &[f64], p: f64) -> f64 {
    let mut v: Vec<f64> = x.iter().cloned().filter(|v| !v.is_nan()).collect();
    if v.is_empty() || p.is_nan() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let rank = ((p.clamp(0.0, 100.0) / 100.0) * v.len() as f64).ceil() as usize;
    v[rank.saturating_sub(1).min(v.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_helpers() {
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&mut [4.0, 1.0, 2.0, 3.0]), 2.5);
        assert_eq!(mode(&[1.0, 2.0, 2.0, 3.0, 3.0]), 2.0);
        assert_eq!(percentile_linear(&[1.0, 2.0, 3.0, 4.0, 5.0], 50.0), 3.0);
        assert_eq!(percentile_linear(&[1.0, 2.0, 3.0, 4.0], 50.0), 2.5);
        assert_eq!(percentile_nearest(&[1.0, 2.0, 3.0, 4.0, 5.0], 40.0), 2.0);
    }
}

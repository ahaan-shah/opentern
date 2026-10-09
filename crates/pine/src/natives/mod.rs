//! Built-in functions and variables. Each is a plain Rust function over
//! evaluated arguments (in parameter order; missing ones are `na`) with an
//! optional state slot, which the compiler gives to every call site of a
//! stateful built-in (`ta.*`, `math.sum`, …), as Pine does.

mod collections;
mod draw;
mod security;
mod strategy;
mod ta;

pub(crate) use security::{SecState, security};
pub(crate) use ta::Ta;

use crate::run::G;
use crate::value::{self, V};
use jiff::Timestamp;
use opentern_core::{Timeframe, Unit};
use std::collections::HashMap;
use std::sync::OnceLock;

pub(crate) type NatFn = fn(&mut G, &[V], &mut Option<Ta>) -> Result<V, String>;

pub(crate) struct NatDef {
    pub name: &'static str,
    /// Parameter names in order; `["*"]` takes any number of arguments.
    pub params: &'static [&'static str],
    pub f: NatFn,
    /// Needs a state slot per call site.
    pub state: bool,
    /// Same arguments, same result, no side effects: folded when constant.
    pub pure: bool,
    /// Also usable without brackets (`ta.tr`, `barstate.islast`).
    pub var: bool,
}

impl std::fmt::Debug for NatDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

pub(crate) fn d(name: &'static str, params: &'static [&'static str], f: NatFn) -> NatDef {
    NatDef {
        name,
        params,
        f,
        state: false,
        pure: false,
        var: false,
    }
}

impl NatDef {
    pub fn pure(mut self) -> NatDef {
        self.pure = true;
        self
    }

    pub fn state(mut self) -> NatDef {
        self.state = true;
        self
    }

    pub fn var(mut self) -> NatDef {
        self.var = true;
        self
    }
}

pub(crate) fn lookup(name: &str) -> Option<&'static NatDef> {
    static TABLE: OnceLock<HashMap<&'static str, NatDef>> = OnceLock::new();
    TABLE
        .get_or_init(|| {
            let mut m = HashMap::new();
            for d in core_defs()
                .into_iter()
                .chain(ta::defs())
                .chain(strategy::defs())
                .chain(collections::defs())
                .chain(draw::defs())
            {
                m.insert(d.name, d);
            }
            m
        })
        .get(name)
}

/// Every built-in name (for the editor's completions).
#[allow(dead_code)]
pub fn names() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = core_defs()
        .into_iter()
        .chain(ta::defs())
        .chain(strategy::defs())
        .chain(collections::defs())
        .chain(draw::defs())
        .map(|d| d.name)
        .collect();
    v.sort_unstable();
    v
}

type R = Result<V, String>;

pub(crate) fn arg(a: &[V], k: usize) -> &V {
    a.get(k).unwrap_or(&V::Na)
}

pub(crate) fn num(a: &[V], k: usize) -> f64 {
    arg(a, k).f()
}

pub(crate) fn int_or(a: &[V], k: usize, d: i64) -> i64 {
    arg(a, k).i_or(d)
}

pub(crate) fn str_or<'a>(a: &'a [V], k: usize, d: &'a str) -> &'a str {
    arg(a, k).as_str().unwrap_or(d)
}

pub(crate) fn bool_or(a: &[V], k: usize, d: bool) -> bool {
    match arg(a, k) {
        V::Na => d,
        v => v.truthy(),
    }
}

/// A positive length argument.
pub(crate) fn len_arg(a: &[V], k: usize, what: &str) -> Result<usize, String> {
    match arg(a, k).i() {
        Some(n) if n > 0 => Ok(n as usize),
        Some(n) => Err(format!("{what} must be greater than 0 (got {n}).")),
        None => Err(format!("{what} is na.")),
    }
}

fn f1(a: &[V], f: impl Fn(f64) -> f64) -> V {
    let x = num(a, 0);
    if x.is_nan() { V::Na } else { V::Float(f(x)) }
}

fn core_defs() -> Vec<NatDef> {
    vec![
        d("na", &["x"], |_, a, _| Ok(V::Bool(arg(a, 0).is_na()))).pure(),
        d("nz", &["source", "replacement"], |_, a, _| {
            let x = arg(a, 0);
            Ok(if x.is_na() {
                match arg(a, 1) {
                    V::Na => match x {
                        V::Float(_) => V::Float(0.0),
                        _ => V::Int(0),
                    },
                    r => r.clone(),
                }
            } else {
                x.clone()
            })
        })
        .pure(),
        d("fixnan", &["source"], |_, a, st| {
            let x = arg(a, 0);
            let last = match st {
                Some(Ta::Last(v)) => v,
                _ => {
                    *st = Some(Ta::Last(V::Na));
                    match st {
                        Some(Ta::Last(v)) => v,
                        _ => unreachable!(),
                    }
                }
            };
            if !x.is_na() {
                *last = x.clone();
            }
            Ok(last.clone())
        })
        .state(),
        d("int", &["x"], |_, a, _| {
            Ok(match arg(a, 0) {
                V::Int(i) => V::Int(*i),
                v => v.i().map(V::Int).unwrap_or(V::Na),
            })
        })
        .pure(),
        d("float", &["x"], |_, a, _| {
            Ok(match arg(a, 0) {
                V::Na => V::Na,
                v => V::Float(v.f()),
            })
        })
        .pure(),
        d("bool", &["x"], |_, a, _| Ok(V::Bool(arg(a, 0).truthy()))).pure(),
        d("string", &["x"], |_, a, _| Ok(V::str(tostring(arg(a, 0), None, 0.01)))).pure(),
        d("color", &["x"], |_, a, _| Ok(arg(a, 0).clone())).pure(),
        d("runtime.error", &["message"], |_, a, _| {
            Err(str_or(a, 0, "Error.").to_string())
        }),
        d("log.info", &["*"], |g, a, _| log(g, "info", a)),
        d("log.warning", &["*"], |g, a, _| log(g, "warning", a)),
        d("log.error", &["*"], |g, a, _| log(g, "error", a)),
        d("alert", &["message", "freq"], |g, a, _| {
            let message = str_or(a, 0, "").to_string();
            let freq = str_or(a, 1, "alert.freq_once_per_bar")
                .trim_start_matches("alert.freq_")
                .to_string();
            let time = g.bar().time;
            g.alerts.push(crate::run::Alert {
                bar: g.index,
                time,
                message,
                title: String::new(),
                freq,
            });
            Ok(V::Na)
        }),
        d("max_bars_back", &["var", "num"], |_, _, _| Ok(V::Na)),
        d("timestamp", &["*"], timestamp),
        // ---------------------------------------------------------------- math
        d("math.abs", &["number"], |_, a, _| {
            Ok(match arg(a, 0) {
                V::Int(i) => V::Int(i.wrapping_abs()),
                _ => f1(a, f64::abs),
            })
        })
        .pure(),
        d("math.acos", &["angle"], |_, a, _| Ok(f1(a, f64::acos))).pure(),
        d("math.asin", &["angle"], |_, a, _| Ok(f1(a, f64::asin))).pure(),
        d("math.atan", &["angle"], |_, a, _| Ok(f1(a, f64::atan))).pure(),
        d("math.cos", &["angle"], |_, a, _| Ok(f1(a, f64::cos))).pure(),
        d("math.sin", &["angle"], |_, a, _| Ok(f1(a, f64::sin))).pure(),
        d("math.tan", &["angle"], |_, a, _| Ok(f1(a, f64::tan))).pure(),
        d("math.exp", &["number"], |_, a, _| Ok(f1(a, f64::exp))).pure(),
        d("math.log", &["number"], |_, a, _| Ok(f1(a, f64::ln))).pure(),
        d("math.log10", &["number"], |_, a, _| Ok(f1(a, f64::log10))).pure(),
        d("math.sqrt", &["number"], |_, a, _| Ok(f1(a, f64::sqrt))).pure(),
        d("math.todegrees", &["radians"], |_, a, _| Ok(f1(a, f64::to_degrees))).pure(),
        d("math.toradians", &["degrees"], |_, a, _| Ok(f1(a, f64::to_radians))).pure(),
        d("math.sign", &["number"], |_, a, _| {
            Ok(f1(a, |x| {
                if x > 0.0 {
                    1.0
                } else if x < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            }))
        })
        .pure(),
        d("math.ceil", &["number"], |_, a, _| Ok(to_int(f1(a, f64::ceil)))).pure(),
        d("math.floor", &["number"], |_, a, _| Ok(to_int(f1(a, f64::floor)))).pure(),
        d("math.round", &["number", "precision"], |_, a, _| {
            let x = num(a, 0);
            if x.is_nan() {
                return Ok(V::Na);
            }
            Ok(match arg(a, 1).i() {
                Some(p) => {
                    let m = 10f64.powi(p as i32);
                    V::Float(round_half_up(x * m) / m)
                }
                None => V::Int(round_half_up(x) as i64),
            })
        })
        .pure(),
        d("math.round_to_mintick", &["number"], |g, a, _| {
            let x = num(a, 0);
            let t = g.env.mintick;
            Ok(if x.is_nan() {
                V::Na
            } else {
                V::Float(round_half_up(x / t) * t)
            })
        }),
        d("math.pow", &["base", "exponent"], |_, a, _| {
            let (b, e) = (num(a, 0), num(a, 1));
            Ok(if b.is_nan() || e.is_nan() {
                V::Na
            } else {
                V::Float(b.powf(e))
            })
        })
        .pure(),
        d("math.max", &["*"], |_, a, _| {
            Ok(fold(a, |x, y| if y > x { y } else { x }))
        })
        .pure(),
        d("math.min", &["*"], |_, a, _| {
            Ok(fold(a, |x, y| if y < x { y } else { x }))
        })
        .pure(),
        d("math.avg", &["*"], |_, a, _| {
            if a.iter().any(|v| v.is_na()) || a.is_empty() {
                return Ok(V::Na);
            }
            Ok(V::Float(a.iter().map(|v| v.f()).sum::<f64>() / a.len() as f64))
        })
        .pure(),
        d("math.sum", &["source", "length"], |_, a, st| ta::sum(a, st)).state(),
        d("math.random", &["min", "max", "seed"], |g, a, st| {
            let seed = arg(a, 2).i();
            let s = match st {
                Some(Ta::Rng(s)) => s,
                _ => {
                    let init = seed
                        .map(|s| s as u64)
                        .unwrap_or(0x9E37_79B9_7F4A_7C15 ^ g.bar_no as u64);
                    *st = Some(Ta::Rng(init | 1));
                    match st {
                        Some(Ta::Rng(s)) => s,
                        _ => unreachable!(),
                    }
                }
            };
            // xorshift64*
            *s ^= *s >> 12;
            *s ^= *s << 25;
            *s ^= *s >> 27;
            let r = (s.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64;
            let lo = arg(a, 0).f_or(0.0);
            let hi = arg(a, 1).f_or(1.0);
            Ok(V::Float(lo + r * (hi - lo)))
        })
        .state(),
        // ---------------------------------------------------------------- str
        d("str.tostring", &["value", "format"], |g, a, _| {
            Ok(V::str(tostring(arg(a, 0), arg(a, 1).as_str(), g.env.mintick)))
        }),
        d("str.tonumber", &["string"], |_, a, _| {
            Ok(match arg(a, 0).as_str().map(|s| s.trim().parse::<f64>()) {
                Some(Ok(f)) => V::Float(f),
                _ => V::Na,
            })
        })
        .pure(),
        d("str.format", &["*"], |g, a, _| {
            let fmt = str_or(a, 0, "");
            Ok(V::str(format_msg(fmt, &a[1.min(a.len())..], g.env.mintick)))
        }),
        d("str.length", &["string"], |_, a, _| {
            Ok(arg(a, 0)
                .as_str()
                .map(|s| V::Int(s.chars().count() as i64))
                .unwrap_or(V::Na))
        })
        .pure(),
        d("str.contains", &["source", "str"], |_, a, _| {
            Ok(V::Bool(str_or(a, 0, "").contains(str_or(a, 1, "\u{0}"))))
        })
        .pure(),
        d("str.startswith", &["source", "str"], |_, a, _| {
            Ok(V::Bool(str_or(a, 0, "").starts_with(str_or(a, 1, "\u{0}"))))
        })
        .pure(),
        d("str.endswith", &["source", "str"], |_, a, _| {
            Ok(V::Bool(str_or(a, 0, "").ends_with(str_or(a, 1, "\u{0}"))))
        })
        .pure(),
        d("str.pos", &["source", "str"], |_, a, _| {
            let s = str_or(a, 0, "");
            Ok(match s.find(str_or(a, 1, "\u{0}")) {
                Some(b) => V::Int(s[..b].chars().count() as i64),
                None => V::Na,
            })
        })
        .pure(),
        d(
            "str.replace",
            &["source", "target", "replacement", "occurrence"],
            |_, a, _| {
                let (s, t, r) = (str_or(a, 0, ""), str_or(a, 1, ""), str_or(a, 2, ""));
                let occ = int_or(a, 3, 0).max(0) as usize;
                if t.is_empty() {
                    return Ok(V::str(s));
                }
                Ok(V::str(match s.match_indices(t).nth(occ) {
                    Some((k, _)) => format!("{}{r}{}", &s[..k], &s[k + t.len()..]),
                    None => s.to_string(),
                }))
            },
        )
        .pure(),
        d("str.replace_all", &["source", "target", "replacement"], |_, a, _| {
            let t = str_or(a, 1, "");
            let s = str_or(a, 0, "");
            Ok(V::str(if t.is_empty() {
                s.to_string()
            } else {
                s.replace(t, str_or(a, 2, ""))
            }))
        })
        .pure(),
        d("str.lower", &["source"], |_, a, _| {
            Ok(V::str(str_or(a, 0, "").to_lowercase()))
        })
        .pure(),
        d("str.upper", &["source"], |_, a, _| {
            Ok(V::str(str_or(a, 0, "").to_uppercase()))
        })
        .pure(),
        d("str.trim", &["source"], |_, a, _| Ok(V::str(str_or(a, 0, "").trim()))).pure(),
        d("str.substring", &["source", "begin_pos", "end_pos"], |_, a, _| {
            let chars: Vec<char> = str_or(a, 0, "").chars().collect();
            let b = int_or(a, 1, 0).clamp(0, chars.len() as i64) as usize;
            let e = int_or(a, 2, chars.len() as i64).clamp(b as i64, chars.len() as i64) as usize;
            Ok(V::str(chars[b..e].iter().collect::<String>()))
        })
        .pure(),
        d("str.repeat", &["source", "repeat", "separator"], |_, a, _| {
            let n = int_or(a, 1, 0).clamp(0, 100_000) as usize;
            Ok(V::str(vec![str_or(a, 0, ""); n].join(str_or(a, 2, ""))))
        })
        .pure(),
        d("str.format_time", &["time", "format", "timezone"], |g, a, _| {
            let t = arg(a, 0).i();
            let Some(t) = t else { return Ok(V::Na) };
            let tz = tz_arg(g, arg(a, 2));
            Ok(V::str(format_time(t, str_or(a, 1, "yyyy-MM-dd'T'HH:mm:ssZ"), &tz)))
        }),
        // ---------------------------------------------------------------- color
        d("color.new", &["color", "transp"], |_, a, _| {
            let Some(c) = arg(a, 0).color() else { return Ok(V::Na) };
            let t = num(a, 1);
            Ok(V::Color(if t.is_nan() { c } else { value::with_transp(c, t) }))
        })
        .pure(),
        d("color.rgb", &["red", "green", "blue", "transp"], |_, a, _| {
            let c = |k: usize| num(a, k).clamp(0.0, 255.0).round() as u32;
            if (0..3).any(|k| num(a, k).is_nan()) {
                return Ok(V::Na);
            }
            let rgb = (c(0) << 16) | (c(1) << 8) | c(2);
            Ok(V::Color(value::with_transp(value::opaque(rgb), arg(a, 3).f_or(0.0))))
        })
        .pure(),
        d("color.r", &["color"], |_, a, _| Ok(chan(a, 24))).pure(),
        d("color.g", &["color"], |_, a, _| Ok(chan(a, 16))).pure(),
        d("color.b", &["color"], |_, a, _| Ok(chan(a, 8))).pure(),
        d("color.t", &["color"], |_, a, _| {
            Ok(arg(a, 0)
                .color()
                .map(|c| V::Float(value::transp_of(c)))
                .unwrap_or(V::Na))
        })
        .pure(),
        d(
            "color.from_gradient",
            &["value", "bottom_value", "top_value", "bottom_color", "top_color"],
            |_, a, _| {
                let (v, lo, hi) = (num(a, 0), num(a, 1), num(a, 2));
                let (Some(c0), Some(c1)) = (arg(a, 3).color(), arg(a, 4).color()) else {
                    return Ok(V::Na);
                };
                if v.is_nan() || lo.is_nan() || hi.is_nan() {
                    return Ok(V::Na);
                }
                let t = if hi == lo {
                    0.0
                } else {
                    ((v - lo) / (hi - lo)).clamp(0.0, 1.0)
                };
                let mix = |s: u32| {
                    let a = ((c0 >> s) & 0xFF) as f64;
                    let b = ((c1 >> s) & 0xFF) as f64;
                    ((a + (b - a) * t).round() as u32) << s
                };
                Ok(V::Color(mix(24) | mix(16) | mix(8) | mix(0)))
            },
        )
        .pure(),
        // ---------------------------------------------------------------- time
        d("year", &["time", "timezone"], |g, a, _| {
            Ok(time_part(g, a, |z| z.year() as i64))
        })
        .var(),
        d("month", &["time", "timezone"], |g, a, _| {
            Ok(time_part(g, a, |z| z.month() as i64))
        })
        .var(),
        d("dayofmonth", &["time", "timezone"], |g, a, _| {
            Ok(time_part(g, a, |z| z.day() as i64))
        })
        .var(),
        d("dayofweek", &["time", "timezone"], |g, a, _| {
            Ok(time_part(g, a, |z| z.weekday().to_sunday_one_offset() as i64))
        })
        .var(),
        d("weekofyear", &["time", "timezone"], |g, a, _| {
            Ok(time_part(g, a, |z| z.date().iso_week_date().week() as i64))
        })
        .var(),
        d("hour", &["time", "timezone"], |g, a, _| {
            Ok(time_part(g, a, |z| z.hour() as i64))
        })
        .var(),
        d("minute", &["time", "timezone"], |g, a, _| {
            Ok(time_part(g, a, |z| z.minute() as i64))
        })
        .var(),
        d("second", &["time", "timezone"], |g, a, _| {
            Ok(time_part(g, a, |z| z.second() as i64))
        })
        .var(),
        d("time", &["timeframe", "session", "timezone", "bars_back"], |g, a, _| {
            bar_time(g, a, false)
        }),
        d(
            "time_close",
            &["timeframe", "session", "timezone", "bars_back"],
            |g, a, _| bar_time(g, a, true),
        ),
        d("timenow", &[], |_, _, _| Ok(V::Int(Timestamp::now().as_millisecond()))).var(),
        d("time_tradingday", &[], |g, _, _| {
            let t = g.bar().time;
            Ok(day_start(t, &g.tz).map(V::Int).unwrap_or(V::Na))
        })
        .var(),
        d("last_bar_index", &[], |g, _, _| Ok(V::Int(g.count as i64 - 1))).var(),
        d("last_bar_time", &[], |g, _, _| {
            Ok(if g.forming || g.index + 1 == g.count {
                V::Int(g.bar().time)
            } else {
                V::Na
            })
        })
        .var(),
        // ---------------------------------------------------------------- barstate
        d("barstate.isfirst", &[], |g, _, _| Ok(V::Bool(g.index == 0))).var(),
        d("barstate.islast", &[], |g, _, _| {
            Ok(V::Bool(g.forming || g.index + 1 == g.count))
        })
        .var(),
        d("barstate.ishistory", &[], |g, _, _| Ok(V::Bool(!g.forming))).var(),
        d("barstate.isrealtime", &[], |g, _, _| Ok(V::Bool(g.forming))).var(),
        d("barstate.isnew", &[], |_, _, _| Ok(V::Bool(true))).var(),
        d("barstate.isconfirmed", &[], |g, _, _| Ok(V::Bool(!g.forming))).var(),
        d("barstate.islastconfirmedhistory", &[], |g, _, _| {
            Ok(V::Bool(!g.forming && g.index + 2 == g.count))
        })
        .var(),
        d("session.isfirstbar", &[], |g, _, _| Ok(V::Bool(g.new_session))).var(),
        d("session.isfirstbar_regular", &[], |g, _, _| Ok(V::Bool(g.new_session))).var(),
        d("session.islastbar", &[], |_, _, _| Ok(V::Bool(false))).var(),
        d("session.islastbar_regular", &[], |_, _, _| Ok(V::Bool(false))).var(),
        d("session.ismarket", &[], |_, _, _| Ok(V::Bool(true))).var(),
        d("session.ispremarket", &[], |_, _, _| Ok(V::Bool(false))).var(),
        d("session.ispostmarket", &[], |_, _, _| Ok(V::Bool(false))).var(),
        // ---------------------------------------------------------------- syminfo
        d("syminfo.ticker", &[], |g, _, _| Ok(V::str(g.env.ticker()))).var(),
        d("syminfo.root", &[], |g, _, _| Ok(V::str(g.env.ticker()))).var(),
        d("syminfo.tickerid", &[], |g, _, _| Ok(V::str(g.env.tickerid.as_str()))).var(),
        d("syminfo.prefix", &[], |g, _, _| Ok(V::str(g.env.prefix()))).var(),
        d("syminfo.description", &[], |g, _, _| {
            Ok(V::str(g.env.description.as_str()))
        })
        .var(),
        d("syminfo.currency", &[], |g, _, _| Ok(V::str(g.env.currency.as_str()))).var(),
        d("syminfo.basecurrency", &[], |g, _, _| {
            let t = g.env.ticker();
            let base = t.strip_suffix(g.env.currency.as_str()).unwrap_or("");
            Ok(V::str(base))
        })
        .var(),
        d("syminfo.mintick", &[], |g, _, _| Ok(V::Float(g.env.mintick))).var(),
        d("syminfo.pointvalue", &[], |_, _, _| Ok(V::Float(1.0))).var(),
        d("syminfo.timezone", &[], |g, _, _| Ok(V::str(g.env.timezone.as_str()))).var(),
        d("syminfo.type", &[], |g, _, _| Ok(V::str(g.env.kind.as_str()))).var(),
        d("syminfo.session", &[], |_, _, _| Ok(V::str("regular"))).var(),
        d("syminfo.volumetype", &[], |_, _, _| Ok(V::str("base"))).var(),
        d("syminfo.country", &[], |_, _, _| Ok(V::str(""))).var(),
        d("syminfo.sector", &[], |_, _, _| Ok(V::str(""))).var(),
        d("syminfo.industry", &[], |_, _, _| Ok(V::str(""))).var(),
        d("syminfo.minmove", &[], |g, _, _| {
            let _ = g;
            Ok(V::Int(1))
        })
        .var(),
        d("syminfo.pricescale", &[], |g, _, _| {
            Ok(V::Int((1.0 / g.env.mintick).round().max(1.0) as i64))
        })
        .var(),
        d(
            "ticker.new",
            &["prefix", "ticker", "session", "adjustment"],
            |_, a, _| Ok(V::str(format!("{}:{}", str_or(a, 0, ""), str_or(a, 1, "")))),
        )
        .pure(),
        d("ticker.standard", &["symbol"], |g, a, _| {
            Ok(V::str(str_or(a, 0, g.env.tickerid.as_str()).to_string()))
        }),
        d("ticker.modify", &["tickerid", "session", "adjustment"], |_, a, _| {
            Ok(arg(a, 0).clone())
        })
        .pure(),
        d("ticker.heikinashi", &["symbol"], |_, _, _| {
            Err("Heikin Ashi tickers in request.security aren't supported yet.".into())
        }),
        // ---------------------------------------------------------------- timeframe
        d("timeframe.period", &[], |g, _, _| {
            Ok(V::str(tf_string(g.tf, g.version)))
        })
        .var(),
        d("timeframe.main_period", &[], |g, _, _| {
            Ok(V::str(tf_string(g.tf, g.version)))
        })
        .var(),
        d("timeframe.multiplier", &[], |g, _, _| {
            Ok(V::Int(match g.tf.unit {
                Unit::Hour => g.tf.count as i64 * 60,
                _ => g.tf.count as i64,
            }))
        })
        .var(),
        d("timeframe.isintraday", &[], |g, _, _| Ok(V::Bool(g.tf.is_intraday()))).var(),
        d("timeframe.isdaily", &[], |g, _, _| Ok(V::Bool(g.tf.unit == Unit::Day))).var(),
        d("timeframe.isweekly", &[], |g, _, _| {
            Ok(V::Bool(g.tf.unit == Unit::Week))
        })
        .var(),
        d("timeframe.ismonthly", &[], |g, _, _| {
            Ok(V::Bool(g.tf.unit == Unit::Month))
        })
        .var(),
        d("timeframe.isdwm", &[], |g, _, _| Ok(V::Bool(!g.tf.is_intraday()))).var(),
        d("timeframe.isseconds", &[], |g, _, _| {
            Ok(V::Bool(g.tf.unit == Unit::Second))
        })
        .var(),
        d("timeframe.isminutes", &[], |g, _, _| {
            Ok(V::Bool(matches!(g.tf.unit, Unit::Minute | Unit::Hour)))
        })
        .var(),
        d("timeframe.isticks", &[], |_, _, _| Ok(V::Bool(false))).var(),
        d("timeframe.in_seconds", &["timeframe"], |g, a, _| {
            let tf = match arg(a, 0) {
                V::Str(s) if !s.is_empty() => parse_tf(s).ok_or_else(|| format!("\"{s}\" isn't a timeframe."))?,
                _ => g.tf,
            };
            Ok(V::Float(tf_seconds(tf)))
        }),
        d("timeframe.from_seconds", &["seconds"], |g, a, _| {
            let s = num(a, 0);
            if s.is_nan() {
                return Ok(V::Na);
            }
            let s = s.max(1.0) as u32;
            let tf = if s < 60 {
                Timeframe::new(s, Unit::Second)
            } else if s < 86_400 {
                Timeframe::new(s / 60, Unit::Minute)
            } else if s < 7 * 86_400 {
                Timeframe::new(s / 86_400, Unit::Day)
            } else if s < 30 * 86_400 {
                Timeframe::new(s / (7 * 86_400), Unit::Week)
            } else {
                Timeframe::new((s / (30 * 86_400)).max(1), Unit::Month)
            };
            Ok(V::str(tf_string(tf, g.version)))
        }),
        d("timeframe.change", &["timeframe"], |g, a, st| {
            let tf = match arg(a, 0) {
                V::Str(s) if !s.is_empty() => parse_tf(s).ok_or_else(|| format!("\"{s}\" isn't a timeframe."))?,
                _ => g.tf,
            };
            let t = g.bar().time;
            let b = bucket(tf, t, &g.tz);
            let prev = match st {
                Some(Ta::Int(p)) => *p,
                _ => i64::MIN,
            };
            *st = Some(Ta::Int(b));
            Ok(V::Bool(prev != b))
        })
        .state(),
    ]
}

fn to_int(v: V) -> V {
    match v {
        V::Float(f) if f.is_finite() => V::Int(f as i64),
        _ => V::Na,
    }
}

/// Pine rounds halves up (toward +∞), like JavaScript.
pub(crate) fn round_half_up(x: f64) -> f64 {
    (x + 0.5).floor()
}

fn fold(a: &[V], pick: impl Fn(f64, f64) -> f64) -> V {
    if a.is_empty() || a.iter().any(|v| v.is_na()) {
        return V::Na;
    }
    let ints = a.iter().all(|v| matches!(v, V::Int(_)));
    let r = a.iter().map(|v| v.f()).reduce(&pick).unwrap_or(f64::NAN);
    if ints { V::Int(r as i64) } else { V::Float(r) }
}

fn chan(a: &[V], shift: u32) -> V {
    arg(a, 0)
        .color()
        .map(|c| V::Float(((c >> shift) & 0xFF) as f64))
        .unwrap_or(V::Na)
}

/// `str.tostring`.
pub(crate) fn tostring(v: &V, fmt: Option<&str>, mintick: f64) -> String {
    match v {
        V::Na => "NaN".into(),
        V::Bool(b) => b.to_string(),
        V::Str(s) => s.to_string(),
        V::Color(c) => format!("#{c:08X}"),
        V::Tuple(t) => format!(
            "[{}]",
            t.iter()
                .map(|x| tostring(x, fmt, mintick))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        V::Ref(_) => "object".into(),
        V::Int(i) if fmt.is_none() => i.to_string(),
        _ => {
            let f = v.f();
            match fmt {
                Some("format.mintick") => {
                    let dec = decimals_of(mintick);
                    format!("{:.*}", dec, round_half_up(f / mintick) * mintick)
                }
                Some(p) => value::fmt_pattern(f, p),
                None => value::fmt_num(f),
            }
        }
    }
}

pub(crate) fn decimals_of(tick: f64) -> usize {
    let mut d = 0;
    let mut t = tick;
    while d < 12 && (t - t.round()).abs() > 1e-9 {
        t *= 10.0;
        d += 1;
    }
    d
}

/// Java-style `{0}` / `{0,number,#.##}` placeholders (`str.format`).
pub(crate) fn format_msg(fmt: &str, args: &[V], mintick: f64) -> String {
    let mut out = String::new();
    let mut rest = fmt;
    while let Some(k) = rest.find('{') {
        out.push_str(&rest[..k]);
        let Some(e) = rest[k..].find('}') else {
            out.push_str(&rest[k..]);
            return out;
        };
        let inner = &rest[k + 1..k + e];
        let mut parts = inner.splitn(3, ',');
        let idx = parts.next().and_then(|p| p.trim().parse::<usize>().ok());
        match idx {
            Some(i) => {
                let v = args.get(i).cloned().unwrap_or_default();
                let kind = parts.next().map(|s| s.trim());
                let pat = parts.next().map(|s| s.trim());
                let s = match (kind, pat) {
                    (Some("number"), Some("integer")) => value::fmt_pattern(v.f(), "#,###"),
                    (Some("number"), Some("percent")) => value::fmt_pattern(v.f(), "#,###%"),
                    (Some("number"), Some("currency")) => format!("${}", value::fmt_pattern(v.f(), "#,##0.00")),
                    (Some("number"), Some(p)) => value::fmt_pattern(v.f(), p),
                    (Some("number"), None) => value::fmt_pattern(v.f(), "#,###.###"),
                    (Some("date" | "time"), p) => match v.i() {
                        Some(t) => format_time(t, p.unwrap_or("yyyy-MM-dd HH:mm"), &jiff::tz::TimeZone::UTC),
                        None => "NaN".into(),
                    },
                    _ => tostring(&v, None, mintick),
                };
                out.push_str(&s);
            }
            None => {
                out.push('{');
                out.push_str(inner);
                out.push('}');
            }
        }
        rest = &rest[k + e + 1..];
    }
    out.push_str(rest);
    out
}

fn log(g: &mut G, level: &str, a: &[V]) -> R {
    let msg = match a.first() {
        Some(V::Str(f)) if a.len() > 1 => format_msg(f, &a[1..], g.env.mintick),
        Some(v) => tostring(v, None, g.env.mintick),
        None => String::new(),
    };
    let t = format_time(g.bar().time, "yyyy-MM-dd HH:mm", &g.tz);
    g.log(format!("[{t}] {level}: {msg}"));
    Ok(V::Na)
}

/// `{{close}}`-style placeholders in alert messages.
pub(crate) fn alert_placeholders(g: &G, msg: &str) -> String {
    if !msg.contains("{{") {
        return msg.to_string();
    }
    let b = g.bar();
    let t = g.env.mintick;
    let px = |x: f64| tostring(&V::Float(x), Some("format.mintick"), t);
    msg.replace("{{close}}", &px(b.close))
        .replace("{{open}}", &px(b.open))
        .replace("{{high}}", &px(b.high))
        .replace("{{low}}", &px(b.low))
        .replace("{{volume}}", &value::fmt_num(b.volume))
        .replace("{{ticker}}", g.env.ticker())
        .replace("{{exchange}}", g.env.prefix())
        .replace("{{interval}}", &tf_string(g.tf, 5))
        .replace(
            "{{time}}",
            &format_time(b.time, "yyyy-MM-dd'T'HH:mm:ss'Z'", &jiff::tz::TimeZone::UTC),
        )
        .replace("{{timenow}}", &Timestamp::now().to_string())
}

pub(crate) fn tz_arg(g: &G, v: &V) -> jiff::tz::TimeZone {
    match v.as_str() {
        Some(s) if !s.is_empty() => parse_tz(s).unwrap_or_else(|| g.tz.clone()),
        _ => g.tz.clone(),
    }
}

/// "America/New_York", "UTC", "GMT+5:30", "UTC-4".
pub(crate) fn parse_tz(s: &str) -> Option<jiff::tz::TimeZone> {
    let s = s.trim();
    if let Ok(tz) = jiff::tz::TimeZone::get(s) {
        return Some(tz);
    }
    let rest = s.strip_prefix("GMT").or_else(|| s.strip_prefix("UTC"))?;
    if rest.is_empty() {
        return Some(jiff::tz::TimeZone::UTC);
    }
    let sign = if rest.starts_with('-') { -1 } else { 1 };
    let body = rest.trim_start_matches(['+', '-']);
    let (h, m) = match body.split_once(':') {
        Some((h, m)) => (h.parse::<i32>().ok()?, m.parse::<i32>().ok()?),
        None => (body.parse::<i32>().ok()?, 0),
    };
    let secs = sign * (h * 3600 + m * 60);
    jiff::tz::Offset::from_seconds(secs).ok().map(jiff::tz::TimeZone::fixed)
}

fn time_part(g: &G, a: &[V], f: impl Fn(&jiff::Zoned) -> i64) -> V {
    let t = match arg(a, 0) {
        V::Na => g.bar().time,
        v => match v.i() {
            Some(t) => t,
            None => return V::Na,
        },
    };
    let tz = tz_arg(g, arg(a, 1));
    match Timestamp::from_millisecond(t) {
        Ok(ts) => V::Int(f(&ts.to_zoned(tz))),
        Err(_) => V::Na,
    }
}

fn day_start(t: i64, tz: &jiff::tz::TimeZone) -> Option<i64> {
    let z = Timestamp::from_millisecond(t).ok()?.to_zoned(tz.clone());
    Some(z.start_of_day().ok()?.timestamp().as_millisecond())
}

/// Bar time for `time(tf, session)` / `time_close(…)`.
fn bar_time(g: &G, a: &[V], close: bool) -> R {
    let back = int_or(a, 3, 0).max(0) as usize;
    let Some(b) = g.bars.get(back).copied() else {
        return Ok(V::Na);
    };
    let tz = tz_arg(g, arg(a, 2));
    if let Some(sess) = arg(a, 1).as_str().filter(|s| !s.is_empty())
        && !in_session(b.time, sess, &tz)
    {
        return Ok(V::Na);
    }
    let tf = match arg(a, 0) {
        V::Str(s) if !s.is_empty() => parse_tf(s).ok_or_else(|| format!("\"{s}\" isn't a timeframe."))?,
        _ => g.tf,
    };
    let start = if tf == g.tf { b.time } else { bucket(tf, b.time, &tz) };
    Ok(V::Int(if close {
        crate::run::tf_end(tf, start, &tz)
    } else {
        start
    }))
}

/// Whether `t` falls in a session like "0930-1600" or "0930-1600:23456".
pub(crate) fn in_session(t: i64, sess: &str, tz: &jiff::tz::TimeZone) -> bool {
    let Ok(ts) = Timestamp::from_millisecond(t) else {
        return false;
    };
    let z = ts.to_zoned(tz.clone());
    let (range, days) = match sess.split_once(':') {
        Some((r, d)) => (r, Some(d)),
        None => (sess, None),
    };
    if let Some(days) = days {
        let dow = z.weekday().to_sunday_one_offset();
        if !days.chars().any(|c| c.to_digit(10) == Some(dow as u32)) {
            return false;
        }
    }
    let Some((a, b)) = range.split_once('-') else {
        return true;
    };
    let hm = |s: &str| -> Option<i32> {
        let s = s.trim();
        if s.len() != 4 {
            return None;
        }
        Some(s[..2].parse::<i32>().ok()? * 60 + s[2..].parse::<i32>().ok()?)
    };
    let (Some(start), Some(end)) = (hm(a), hm(b)) else {
        return true;
    };
    let now = z.hour() as i32 * 60 + z.minute() as i32;
    if start <= end {
        now >= start && now < end
    } else {
        now >= start || now < end
    }
}

/// The open time of the `tf` bar containing `t`.
pub(crate) fn bucket(tf: Timeframe, t: i64, tz: &jiff::tz::TimeZone) -> i64 {
    let Ok(ts) = Timestamp::from_millisecond(t) else {
        return t;
    };
    let z = ts.to_zoned(tz.clone());
    let off = z.offset().seconds() as i64 * 1000;
    match tf.unit {
        Unit::Second | Unit::Minute | Unit::Hour => {
            let ms = tf.millis().unwrap_or(60_000);
            // Aligned to local midnight (so 4h bars start at 00, 04, …).
            (t + off).div_euclid(ms) * ms - off
        }
        Unit::Day => {
            let start = z.start_of_day().map(|s| s.timestamp().as_millisecond()).unwrap_or(t);
            if tf.count <= 1 {
                start
            } else {
                let days = (start + off).div_euclid(86_400_000);
                let k = days.div_euclid(tf.count as i64) * tf.count as i64;
                k * 86_400_000 - off
            }
        }
        Unit::Week => {
            let d = z.date();
            let back = d.weekday().to_monday_zero_offset() as i64;
            let monday = d.checked_sub(jiff::Span::new().days(back)).unwrap_or(d);
            monday
                .to_zoned(tz.clone())
                .map(|m| m.timestamp().as_millisecond())
                .unwrap_or(t)
        }
        Unit::Month => {
            let d = z.date();
            let months = d.year() as i64 * 12 + d.month() as i64 - 1;
            let k = months.div_euclid(tf.count as i64) * tf.count as i64;
            let (y, m) = (k.div_euclid(12), k.rem_euclid(12) + 1);
            jiff::civil::date(y as i16, m as i8, 1)
                .to_zoned(tz.clone())
                .map(|m| m.timestamp().as_millisecond())
                .unwrap_or(t)
        }
    }
}

/// Pine's timeframe strings: "1", "60", "240", "D", "2D", "W", "M", "15S".
pub(crate) fn parse_tf(s: &str) -> Option<Timeframe> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (num, suffix) = s.split_at(split);
    let n: u32 = if num.is_empty() { 1 } else { num.parse().ok()? };
    if n == 0 {
        return None;
    }
    Some(match suffix {
        "" => {
            if n.is_multiple_of(60) && n < 1440 {
                Timeframe::new(n / 60, Unit::Hour)
            } else if n.is_multiple_of(1440) {
                Timeframe::new(n / 1440, Unit::Day)
            } else {
                Timeframe::new(n, Unit::Minute)
            }
        }
        "S" | "s" => Timeframe::new(n, Unit::Second),
        "D" | "d" => Timeframe::new(n, Unit::Day),
        "W" | "w" => Timeframe::new(n, Unit::Week),
        "M" => Timeframe::new(n, Unit::Month),
        "H" | "h" => Timeframe::new(n, Unit::Hour),
        _ => return None,
    })
}

pub(crate) fn tf_string(tf: Timeframe, version: u8) -> String {
    let n = tf.count;
    let one = |u: &str| {
        if n == 1 && version < 6 {
            u.to_string()
        } else {
            format!("{n}{u}")
        }
    };
    match tf.unit {
        Unit::Second => format!("{n}S"),
        Unit::Minute => n.to_string(),
        Unit::Hour => (n * 60).to_string(),
        Unit::Day => one("D"),
        Unit::Week => one("W"),
        Unit::Month => one("M"),
    }
}

pub(crate) fn tf_seconds(tf: Timeframe) -> f64 {
    match tf.millis() {
        Some(ms) => ms as f64 / 1000.0,
        None => tf.count as f64 * 2_628_003.0,
    }
}

/// `timestamp(year, month, day, hour, minute, second)`,
/// `timestamp(timezone, year, …)` or `timestamp("2020-02-20 15:30 +0300")`.
fn timestamp(g: &mut G, a: &[V], _: &mut Option<Ta>) -> R {
    if a.len() == 1
        && let Some(s) = a[0].as_str()
    {
        return Ok(parse_date(s, &g.tz).map(V::Int).unwrap_or(V::Na));
    }
    let (tz, rest) = match a.first() {
        Some(V::Str(s)) => (parse_tz(s).unwrap_or_else(|| g.tz.clone()), &a[1..]),
        _ => (g.tz.clone(), a),
    };
    let n = |k: usize, d: i64| rest.get(k).and_then(|v| v.i()).unwrap_or(d);
    if rest.iter().take(3).any(|v| v.is_na()) {
        return Ok(V::Na);
    }
    let (y, mo, d) = (n(0, 1970), n(1, 1), n(2, 1));
    let (h, mi, s) = (n(3, 0), n(4, 0), n(5, 0));
    // Out-of-range parts roll over, as in Pine (day 0 = last of previous month).
    let base = jiff::civil::date(y as i16, 1, 1).at(0, 0, 0, 0);
    let span = jiff::Span::new()
        .months(mo - 1)
        .days(d - 1)
        .hours(h)
        .minutes(mi)
        .seconds(s);
    let dt = base.checked_add(span).map_err(|e| e.to_string())?;
    Ok(dt
        .to_zoned(tz)
        .map(|z| V::Int(z.timestamp().as_millisecond()))
        .unwrap_or(V::Na))
}

fn parse_date(s: &str, tz: &jiff::tz::TimeZone) -> Option<i64> {
    let s = s.trim();
    if let Ok(ts) = s.parse::<Timestamp>() {
        return Some(ts.as_millisecond());
    }
    // "2020-02-20", "2020-02-20 15:30", "20 Feb 2020 15:30 +0000".
    if let Ok(dt) = s.parse::<jiff::civil::DateTime>() {
        return dt.to_zoned(tz.clone()).ok().map(|z| z.timestamp().as_millisecond());
    }
    if let Ok(d) = s.parse::<jiff::civil::Date>() {
        return d.to_zoned(tz.clone()).ok().map(|z| z.timestamp().as_millisecond());
    }
    for f in ["%d %b %Y %H:%M %z", "%d %b %Y %H:%M:%S %z", "%Y-%m-%d %H:%M %z"] {
        if let Ok(z) = jiff::Zoned::strptime(f, s) {
            return Some(z.timestamp().as_millisecond());
        }
    }
    for f in ["%d %b %Y %H:%M", "%d %b %Y"] {
        let full = s.to_string();
        if let Ok(dt) = jiff::civil::DateTime::strptime(f, &full) {
            return dt.to_zoned(tz.clone()).ok().map(|z| z.timestamp().as_millisecond());
        }
        if let Ok(d) = jiff::civil::Date::strptime(f, &full) {
            return d.to_zoned(tz.clone()).ok().map(|z| z.timestamp().as_millisecond());
        }
    }
    None
}

/// Java-style date patterns (`yyyy-MM-dd HH:mm:ss`).
pub(crate) fn format_time(t: i64, pat: &str, tz: &jiff::tz::TimeZone) -> String {
    let Ok(ts) = Timestamp::from_millisecond(t) else {
        return "NaN".into();
    };
    let z = ts.to_zoned(tz.clone());
    let mut out = String::new();
    let c: Vec<char> = pat.chars().collect();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch == '\'' {
            let mut j = i + 1;
            while j < c.len() && c[j] != '\'' {
                out.push(c[j]);
                j += 1;
            }
            i = j + 1;
            continue;
        }
        let mut n = 1;
        while i + n < c.len() && c[i + n] == ch {
            n += 1;
        }
        let pad = |v: i64, w: usize| format!("{v:0w$}");
        let s = match ch {
            'y' => {
                if n == 2 {
                    pad(z.year() as i64 % 100, 2)
                } else {
                    pad(z.year() as i64, n.max(4))
                }
            }
            'M' => match n {
                1 | 2 => pad(z.month() as i64, n),
                3 => z.strftime("%b").to_string(),
                _ => z.strftime("%B").to_string(),
            },
            'd' => pad(z.day() as i64, n),
            'H' => pad(z.hour() as i64, n),
            'h' => pad(((z.hour() as i64 + 11) % 12) + 1, n),
            'm' => pad(z.minute() as i64, n),
            's' => pad(z.second() as i64, n),
            'S' => pad(z.millisecond() as i64, 3)[..n.min(3)].to_string(),
            'a' => (if z.hour() < 12 { "AM" } else { "PM" }).to_string(),
            'E' => {
                if n >= 4 {
                    z.strftime("%A").to_string()
                } else {
                    z.strftime("%a").to_string()
                }
            }
            'Z' | 'X' => z.strftime("%z").to_string(),
            'z' => z.strftime("%Z").to_string(),
            other => other.to_string().repeat(n),
        };
        out.push_str(&s);
        i += n;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(
            format_msg("{0} is {1,number,#.##}", &[V::str("x"), V::Float(2.34567)], 0.01),
            "x is 2.35"
        );
        assert_eq!(tostring(&V::Float(1234.5), Some("format.mintick"), 0.01), "1234.50");
        assert_eq!(tostring(&V::Int(14), None, 0.01), "14");
        assert_eq!(
            format_time(1_600_000_000_000, "yyyy-MM-dd HH:mm", &jiff::tz::TimeZone::UTC),
            "2020-09-13 12:26"
        );
    }

    #[test]
    fn timeframes() {
        assert_eq!(parse_tf("60"), Some(Timeframe::new(1, Unit::Hour)));
        assert_eq!(parse_tf("D"), Some(Timeframe::new(1, Unit::Day)));
        assert_eq!(parse_tf("15S"), Some(Timeframe::new(15, Unit::Second)));
        assert_eq!(tf_string(Timeframe::new(4, Unit::Hour), 5), "240");
        assert_eq!(tf_string(Timeframe::new(1, Unit::Day), 5), "D");
        assert_eq!(tf_string(Timeframe::new(1, Unit::Day), 6), "1D");
    }

    #[test]
    fn buckets() {
        let utc = jiff::tz::TimeZone::UTC;
        // 2020-09-13 12:26 UTC
        let t = 1_600_000_000_000;
        assert_eq!(bucket(Timeframe::new(1, Unit::Day), t, &utc), 1_599_955_200_000);
        assert_eq!(
            bucket(Timeframe::new(4, Unit::Hour), t, &utc),
            1_599_955_200_000 + 12 * 3_600_000
        );
        // Monday 2020-09-07
        assert_eq!(bucket(Timeframe::new(1, Unit::Week), t, &utc), 1_599_436_800_000);
        // 2020-09-01
        assert_eq!(bucket(Timeframe::new(1, Unit::Month), t, &utc), 1_598_918_400_000);
    }

    #[test]
    fn sessions() {
        let ny = parse_tz("America/New_York").unwrap();
        // 2020-09-14 14:00 UTC = 10:00 New York, a Monday.
        let t = 1_600_092_000_000;
        assert!(in_session(t, "0930-1600", &ny));
        assert!(!in_session(t, "1000-1600:17", &ny));
        assert!(in_session(t, "0930-1600:23456", &ny));
        assert!(parse_tz("GMT+5:30").is_some());
    }

    #[test]
    fn rounding() {
        assert_eq!(round_half_up(2.5), 3.0);
        assert_eq!(round_half_up(-2.5), -2.0);
    }
}

//! Built-in functions and variables. Each is a plain Rust function over
//! evaluated arguments (in parameter order; missing ones are `na`) with an
//! optional state slot, which the compiler gives to every call site of a
//! stateful built-in (`ta.*`, `math.sum`, …), as Pine does.

mod collections;
mod draw;
mod security;
mod strategy;
mod ta;

pub(crate) use security::{Contexts, approx_ms, security};
pub(crate) use ta::{Ta, replay};

use crate::run::G;
use crate::value::{self, V};
use jiff::Timestamp;
use opentern_core::{Timeframe, Unit};
use std::collections::HashMap;
use std::sync::OnceLock;

/// Marks a symbol whose Heikin Ashi bars a request wants.
pub(crate) const HEIKIN_ASHI: &str = "=HA:";

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
    /// A finite window of its source, whose length may be a series.
    pub window: bool,
    /// Does something besides giving a value (draws, orders, logs).
    pub effect: bool,
    /// Adds to an array or takes from it.
    pub resizes: bool,
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
        window: false,
        effect: is_effect(name),
        resizes: matches!(
            name,
            "array.push"
                | "array.unshift"
                | "array.pop"
                | "array.shift"
                | "array.insert"
                | "array.remove"
                | "array.clear"
                | "array.concat"
        ),
    }
}

/// Calls a built-in. One that resizes a slice resizes its array there.
pub(crate) fn call(def: &NatDef, g: &mut G, a: &[V], st: &mut Option<Ta>) -> R {
    if !def.resizes {
        return (def.f)(g, a, st);
    }
    let slice = match arg(a, 0) {
        V::Ref(r) => match g.heap.get(*r) {
            Some(crate::heap::Obj::Slice { of, from, to }) => Some((*r, *of, *from, *to)),
            _ => None,
        },
        _ => None,
    };
    let Some((id, of, from, to)) = slice else {
        return (def.f)(g, a, st);
    };
    use crate::heap::Obj;
    let items = g.heap.items(id).unwrap_or_default().to_vec();
    let tmp = g.heap.alloc(Obj::Array(items));
    let mut args = a.to_vec();
    args[0] = V::Ref(tmp);
    let v = (def.f)(g, &args, st)?;
    let items = g.heap.items(tmp).unwrap_or_default().to_vec();
    let end = from + items.len();
    if let Some(Obj::Array(all)) = g.heap.get_mut(of) {
        let to = to.min(all.len());
        all.splice(from.min(to)..to, items);
    }
    if let Some(Obj::Slice { to, .. }) = g.heap.get_mut(id) {
        *to = end;
    }
    Ok(match v {
        V::Ref(x) if x == tmp => V::Ref(id),
        v => v,
    })
}

/// What a script does to the chart or its account: not done for the bars
/// of a request.
fn is_effect(name: &str) -> bool {
    match name.split_once('.') {
        Some(("label" | "line" | "box" | "table" | "linefill" | "polyline", f)) => f == "new" || f == "copy",
        Some(("strategy", f)) => {
            matches!(
                f,
                "entry" | "exit" | "order" | "close" | "close_all" | "cancel" | "cancel_all"
            ) || f.starts_with("risk.")
        }
        Some(("log", _)) => true,
        _ => name == "alert",
    }
}

/// The parameters that say how long a window is.
pub(crate) const LENGTHS: &[&str] = &["length", "leftbars", "rightbars"];

/// The first argument a `ta.*` call can't do without and doesn't have.
pub(crate) fn missing(d: &NatDef, given: &[bool]) -> Option<usize> {
    if !d.name.starts_with("ta.") || d.params.is_empty() {
        return None;
    }
    let optional: &[&str] = match d.name {
        "ta.change" => &["length"],
        // Short forms, and ones with defaults.
        "ta.highest" | "ta.lowest" | "ta.highestbars" | "ta.lowestbars" => &["length"],
        "ta.pivothigh" | "ta.pivotlow" => &["rightbars"],
        "ta.vwap" => &["anchor", "stdev_mult", "source"],
        "ta.tr" => &["handle_na"],
        "ta.pivot_point_levels" => &["developing"],
        _ => &["biased", "useTrueRange", "floor"],
    };
    given
        .iter()
        .enumerate()
        .position(|(k, g)| !g && !optional.contains(&d.params[k]))
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

    pub fn window(mut self) -> NatDef {
        self.state = true;
        self.window = true;
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

/// Built-in candidates for one method name. The receiver still determines
/// which family is used, after user-method overload resolution.
#[derive(Clone, Debug)]
pub(crate) struct NativeMethods {
    defs: Box<[(&'static str, &'static NatDef)]>,
    pub accepts_na: bool,
}

impl NativeMethods {
    pub fn new(name: &str) -> Self {
        const DRAWINGS: [&str; 6] = ["label", "line", "box", "table", "linefill", "polyline"];
        let mut defs = Vec::new();
        for family in ["array", "map", "matrix", "str", "color", "math"]
            .into_iter()
            .chain(DRAWINGS)
        {
            if let Some(def) = lookup(&format!("{family}.{name}")) {
                defs.push((family, def));
            }
        }
        let accepts_na = defs.iter().any(|(family, _)| DRAWINGS.contains(family));
        Self {
            defs: defs.into_boxed_slice(),
            accepts_na,
        }
    }

    pub fn for_family(&self, family: &str) -> Option<&'static NatDef> {
        self.defs.iter().find(|(key, _)| *key == family).map(|(_, def)| *def)
    }
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
        d("timestamp", &["*"], timestamp).pure(),
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
        d("math.sum", &["source", "length"], |_, a, st| ta::sum(a, st)).window(),
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
            Ok(V::str(text_of(g, arg(a, 0), arg(a, 1).as_str())))
        }),
        d("str.tonumber", &["string"], |_, a, _| {
            Ok(match arg(a, 0).as_str().map(|s| s.trim().parse::<f64>()) {
                Some(Ok(f)) => V::Float(f),
                _ => V::Na,
            })
        })
        .pure(),
        d("str.split", &["string", "separator"], |g, a, _| {
            let (s, sep) = (str_or(a, 0, ""), str_or(a, 1, ""));
            let parts: Vec<V> = if sep.is_empty() {
                s.chars().map(|c| V::str(c.to_string())).collect()
            } else {
                s.split(sep).map(V::str).collect()
            };
            Ok(V::Ref(g.heap.alloc(crate::heap::Obj::Array(parts))))
        }),
        d("str.match", &["source", "regex"], |_, a, _| {
            let re = regex_of(str_or(a, 1, ""))?;
            Ok(V::str(re.find(str_or(a, 0, "")).map_or("", |m| m.as_str())))
        }),
        d("str.format", &["*"], |g, a, _| {
            let fmt = str_or(a, 0, "");
            Ok(V::str(format_msg(g, fmt, &a[1.min(a.len())..])))
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
            let (s, sep) = (str_or(a, 0, ""), str_or(a, 2, ""));
            // Repeats of repeats would outgrow memory.
            if n.saturating_mul(s.len() + sep.len()) > 1_000_000 {
                return Err("str.repeat would make a string over a million characters.".into());
            }
            Ok(V::str(vec![s; n].join(sep)))
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
            // 00:00 UTC of the trading day.
            let day = crate::run::day_of(g.tf, g.bar().time, &g.tz);
            Ok(V::Int(day * 86_400_000))
        })
        .var(),
        d("last_bar_index", &[], |g, _, _| Ok(V::Int(g.count as i64 - 1))).var(),
        d("last_bar_time", &[], |g, _, _| {
            Ok(V::Int(crate::run::open_in(g.main_tf, g.last_time, &g.tz, g.market)))
        })
        .var(),
        d("bid", &[], |_, _, _| Ok(V::Na)).var(),
        d("ask", &[], |_, _, _| Ok(V::Na)).var(),
        // ---------------------------------------------------------------- chart
        // The theme isn't known to a script: a dark chart, as the app's default.
        d("chart.bg_color", &[], |_, _, _| Ok(V::Color(0x131722FF))).var(),
        d("chart.fg_color", &[], |_, _, _| Ok(V::Color(0xFFFFFFFF))).var(),
        d("chart.is_standard", &[], |_, _, _| Ok(V::Bool(true))).var(),
        d("chart.is_heikinashi", &[], |_, _, _| Ok(V::Bool(false))).var(),
        d("chart.is_kagi", &[], |_, _, _| Ok(V::Bool(false))).var(),
        d("chart.is_linebreak", &[], |_, _, _| Ok(V::Bool(false))).var(),
        d("chart.is_pnf", &[], |_, _, _| Ok(V::Bool(false))).var(),
        d("chart.is_range", &[], |_, _, _| Ok(V::Bool(false))).var(),
        d("chart.is_renko", &[], |_, _, _| Ok(V::Bool(false))).var(),
        // Scripts run on every bar loaded, not only the ones in view.
        d("chart.left_visible_bar_time", &[], |_, _, _| Ok(V::Int(0))).var(),
        d("chart.right_visible_bar_time", &[], |g, _, _| {
            Ok(V::Int(crate::run::open_in(g.main_tf, g.last_time, &g.tz, g.market)))
        })
        .var(),
        // ---------------------------------------------------------------- barstate
        d("barstate.isfirst", &[], |g, _, _| Ok(V::Bool(g.index == 0))).var(),
        d("barstate.islast", &[], |g, _, _| {
            Ok(V::Bool(g.forming || g.index + 1 == g.count))
        })
        .var(),
        d("barstate.ishistory", &[], |g, _, _| Ok(V::Bool(!realtime(g)))).var(),
        d("barstate.isrealtime", &[], |g, _, _| Ok(V::Bool(realtime(g)))).var(),
        d("barstate.isnew", &[], |_, _, _| Ok(V::Bool(true))).var(),
        d("barstate.isconfirmed", &[], |g, _, _| Ok(V::Bool(!g.forming))).var(),
        d("barstate.islastconfirmedhistory", &[], |g, _, _| {
            Ok(V::Bool(!realtime(g) && g.index + 2 == g.loaded))
        })
        .var(),
        d("session.isfirstbar", &[], |g, _, _| Ok(V::Bool(g.new_session))).var(),
        d("session.isfirstbar_regular", &[], |g, _, _| Ok(V::Bool(g.new_session))).var(),
        d("session.islastbar", &[], |g, _, _| Ok(V::Bool(last_of_session(g)))).var(),
        d("session.islastbar_regular", &[], |g, _, _| {
            Ok(V::Bool(last_of_session(g)))
        })
        .var(),
        d("session.ismarket", &[], |g, _, _| Ok(V::Bool(extended(g).is_none()))).var(),
        d("session.ispremarket", &[], |g, _, _| {
            Ok(V::Bool(extended(g) == Some(opentern_core::calendar::Extended::Pre)))
        })
        .var(),
        d("session.ispostmarket", &[], |g, _, _| {
            Ok(V::Bool(extended(g) == Some(opentern_core::calendar::Extended::Post)))
        })
        .var(),
        // ---------------------------------------------------------------- syminfo
        d("syminfo.ticker", &["symbol"], |g, a, _| {
            Ok(match arg(a, 0).as_str() {
                Some(s) => V::str(s.rsplit(':').next().unwrap_or(s)),
                None => V::str(g.env.ticker()),
            })
        })
        .var(),
        d("syminfo.root", &[], |g, _, _| Ok(V::str(g.env.ticker()))).var(),
        d("syminfo.tickerid", &[], |g, _, _| Ok(V::str(g.env.tickerid.as_str()))).var(),
        d("syminfo.main_tickerid", &[], |g, _, _| {
            Ok(V::str(g.env.tickerid.as_str()))
        })
        .var(),
        d("syminfo.prefix", &["symbol"], |g, a, _| {
            Ok(match arg(a, 0).as_str() {
                Some(s) => V::str(s.split_once(':').map_or("", |(p, _)| p)),
                None => V::str(g.env.prefix()),
            })
        })
        .var(),
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
        d("syminfo.mincontract", &[], |g, _, _| {
            let m = g.env.mincontract();
            // Fractional when unknown, as a tiny step.
            Ok(V::Float(if m > 0.0 { m } else { 1e-8 }))
        })
        .var(),
        d("syminfo.pointvalue", &[], |_, _, _| Ok(V::Float(1.0))).var(),
        d("syminfo.timezone", &[], |g, _, _| Ok(V::str(g.env.timezone.as_str()))).var(),
        d("syminfo.type", &[], |g, _, _| Ok(V::str(g.env.kind.as_str()))).var(),
        d("syminfo.session", &[], |_, _, _| Ok(V::str("regular"))).var(),
        d("syminfo.volumetype", &[], |_, _, _| Ok(V::str("base"))).var(),
        d("syminfo.country", &[], |_, _, _| Ok(V::str(""))).var(),
        d("syminfo.sector", &[], |_, _, _| Ok(V::str(""))).var(),
        d("syminfo.industry", &[], |_, _, _| Ok(V::str(""))).var(),
        d("syminfo.minmove", &[], |g, _, _| {
            Ok(V::Int(price_steps(g.env.mintick).0))
        })
        .var(),
        d("syminfo.pricescale", &[], |g, _, _| {
            Ok(V::Int(price_steps(g.env.mintick).1))
        })
        .var(),
        d(
            "ticker.new",
            &[
                "prefix",
                "ticker",
                "session",
                "adjustment",
                "backadjustment",
                "settlement_as_close",
            ],
            |_, a, _| Ok(V::str(format!("{}:{}", str_or(a, 0, ""), str_or(a, 1, "")))),
        )
        .pure(),
        d("ticker.standard", &["symbol"], |g, a, _| {
            let sym = str_or(a, 0, g.env.tickerid.as_str());
            Ok(V::str(sym.trim_start_matches(HEIKIN_ASHI)))
        }),
        d(
            "ticker.modify",
            &[
                "tickerid",
                "session",
                "adjustment",
                "backadjustment",
                "settlement_as_close",
            ],
            |_, a, _| Ok(arg(a, 0).clone()),
        )
        .pure(),
        d("ticker.inherit", &["from_tickerid", "symbol"], |_, a, _| {
            Ok(arg(a, 1).clone())
        })
        .pure(),
        d(
            "request.currency_rate",
            &["from", "to", "ignore_invalid_currency"],
            |_, a, _| {
                let code = |k: usize| str_or(a, k, "").trim_start_matches("currency.").to_string();
                if code(0) == code(1) {
                    return Ok(V::Float(1.0));
                }
                Err("request.currency_rate() between different currencies isn't supported yet.".into())
            },
        ),
        d("ticker.heikinashi", &["symbol"], |g, a, _| {
            let sym = str_or(a, 0, g.env.tickerid.as_str());
            Ok(V::str(format!("{HEIKIN_ASHI}{}", sym.trim_start_matches(HEIKIN_ASHI))))
        }),
        // ---------------------------------------------------------------- timeframe
        d("timeframe.period", &[], |g, _, _| {
            Ok(V::str(tf_string(g.tf, g.version)))
        })
        .var(),
        d("timeframe.main_period", &[], |g, _, _| {
            Ok(V::str(tf_string(g.main_tf, g.version)))
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
            Ok(V::Int(tf_seconds(tf) as i64))
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
            let b = if tf == g.tf { t } else { g.bucket(tf, t) };
            let prev = match st {
                Some(Ta::Int(p)) => *p,
                _ => i64::MIN,
            };
            *st = Some(Ta::Int(b));
            // False on the first bar, as `ta.change(time(tf)) != 0` is
            // (TradingView's RelativeValue library relies on it).
            Ok(V::Bool(prev != i64::MIN && prev != b))
        })
        .state(),
    ]
    .into_iter()
    .chain(UNKNOWN.iter().map(|n| d(n, &[], |_, _, _| Ok(V::Na)).var()))
    .chain(["box", "label", "line", "linefill", "table"].map(|n| d(n, &["x"], |_, a, _| Ok(arg(a, 0).clone()))))
    .collect()
}

/// Facts about a company or contract that Opentern doesn't have: `na`, as
/// they are on TradingView for symbols without them.
const UNKNOWN: &[&str] = &[
    "syminfo.current_contract",
    "syminfo.employees",
    "syminfo.expiration_date",
    "syminfo.isin",
    "syminfo.recommendations_buy",
    "syminfo.recommendations_buy_strong",
    "syminfo.recommendations_date",
    "syminfo.recommendations_hold",
    "syminfo.recommendations_sell",
    "syminfo.recommendations_sell_strong",
    "syminfo.recommendations_total",
    "syminfo.shareholders",
    "syminfo.shares_outstanding_float",
    "syminfo.shares_outstanding_total",
    "syminfo.target_price_average",
    "syminfo.target_price_date",
    "syminfo.target_price_estimates",
    "syminfo.target_price_high",
    "syminfo.target_price_low",
    "syminfo.target_price_median",
    "dividends.future_amount",
    "dividends.future_ex_date",
    "dividends.future_pay_date",
    "earnings.future_eps",
    "earnings.future_period_end_time",
    "earnings.future_revenue",
    "earnings.future_time",
];

/// (`syminfo.minmove`, `syminfo.pricescale`): 0.05 is 5/100.
fn price_steps(tick: f64) -> (i64, i64) {
    let scale = 10i64.pow(decimals_of(tick).min(12) as u32);
    (((tick * scale as f64).round() as i64).max(1), scale)
}

/// Whether the bar came in while the script was running.
fn realtime(g: &G) -> bool {
    g.forming || g.index + 1 >= g.loaded
}

/// Whether the bar is its session's last: it ends when the session does.
fn last_of_session(g: &G) -> bool {
    if !g.tf.is_intraday() {
        return true;
    }
    let end = g.close_time(g.bar().time);
    match g.market {
        opentern_core::calendar::Market::Crypto => {
            // Days end at 00:00.
            crate::run::local_day(end, &g.tz) != crate::run::local_day(g.bar().time, &g.tz)
        }
        m => opentern_core::calendar::closes_at(m, end),
    }
}

fn extended(g: &G) -> Option<opentern_core::calendar::Extended> {
    (g.tf.is_intraday() && g.market == opentern_core::calendar::Market::Us)
        .then(|| opentern_core::calendar::us_extended(g.bar().time))
        .flatten()
}

fn regex_of(s: &str) -> Result<regex::Regex, String> {
    regex::RegexBuilder::new(s)
        .size_limit(1 << 20)
        .build()
        .map_err(|_| format!("\"{s}\" isn't a pattern this engine understands."))
}

/// `str.tostring`, with arrays and matrices written out.
pub(crate) fn text_of(g: &G, v: &V, fmt: Option<&str>) -> String {
    use crate::heap::Obj;
    let t = g.env.mintick;
    let list = |xs: &[V]| xs.iter().map(|x| tostring(x, fmt, t)).collect::<Vec<_>>().join(", ");
    match v {
        V::Ref(r) => match g.heap.get(*r) {
            Some(Obj::Array(_) | Obj::Slice { .. }) => format!("[{}]", list(g.heap.items(*r).unwrap_or_default())),
            Some(Obj::Matrix { cols, data, .. }) => data
                .chunks((*cols).max(1))
                .map(|row| format!("[{}]", list(row)))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => tostring(v, fmt, t),
        },
        _ => tostring(v, fmt, t),
    }
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
/// Text in single quotes is taken as it is, and `''` is a quote.
pub(crate) fn format_msg(g: &G, fmt: &str, args: &[V]) -> String {
    let mut out = String::new();
    let c: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    let mut quoted = false;
    while i < c.len() {
        match c[i] {
            '\'' if c.get(i + 1) == Some(&'\'') => {
                out.push('\'');
                i += 2;
            }
            '\'' => {
                quoted = !quoted;
                i += 1;
            }
            '{' if !quoted => {
                let Some(e) = c[i..].iter().position(|x| *x == '}') else {
                    out.extend(&c[i..]);
                    return out;
                };
                let inner: String = c[i + 1..i + e].iter().collect();
                out.push_str(&placeholder(g, &inner, args).unwrap_or(format!("{{{inner}}}")));
                i += e + 1;
            }
            x => {
                out.push(x);
                i += 1;
            }
        }
    }
    out
}

fn placeholder(g: &G, inner: &str, args: &[V]) -> Option<String> {
    let mut parts = inner.splitn(3, ',');
    let i = parts.next()?.trim().parse::<usize>().ok()?;
    let v = args.get(i).cloned().unwrap_or_default();
    let kind = parts.next().map(|s| s.trim());
    let pat = parts.next().map(|s| s.trim());
    Some(match (kind, pat) {
        (Some("number"), Some("integer")) => value::fmt_pattern(v.f(), "#,##0"),
        (Some("number"), Some("percent")) => value::fmt_pattern(v.f(), "#,##0%"),
        (Some("number"), Some("currency")) => format!("${}", value::fmt_pattern(v.f(), "#,##0.00")),
        (Some("number"), Some(p)) => value::fmt_pattern(v.f(), p),
        (Some("number"), None) => value::fmt_pattern(v.f(), "#,##0.###"),
        (Some("date" | "time"), p) => match v.i() {
            Some(t) => format_time(t, p.unwrap_or("yyyy-MM-dd HH:mm"), &jiff::tz::TimeZone::UTC),
            None => "NaN".into(),
        },
        _ => match v {
            // Numbers are grouped and rounded to three places by default.
            V::Int(_) | V::Float(_) => value::fmt_pattern(v.f(), "#,##0.###"),
            v => text_of(g, &v, None),
        },
    })
}

fn log(g: &mut G, level: &str, a: &[V]) -> R {
    let msg = match a.first() {
        Some(V::Str(f)) if a.len() > 1 => format_msg(g, f, &a[1..]),
        Some(v) => text_of(g, v, None),
        None => String::new(),
    };
    let t = format_time(g.now(), "yyyy-MM-dd HH:mm", &g.tz);
    g.log(format!("[{t}] {level}: {msg}"));
    Ok(V::Na)
}

/// `{{close}}`-style placeholders in alert messages.
pub(crate) fn alert_placeholders(g: &G, msg: &str, plots: &[crate::compile::PlotDef]) -> String {
    if !msg.contains("{{") {
        return msg.to_string();
    }
    let b = g.bar();
    let t = g.env.mintick;
    let px = |x: f64| tostring(&V::Float(x), Some("format.mintick"), t);
    let mut msg = msg
        .replace("{{close}}", &px(b.close))
        .replace("{{open}}", &px(b.open))
        .replace("{{high}}", &px(b.high))
        .replace("{{low}}", &px(b.low))
        .replace("{{volume}}", &value::fmt_num(b.volume))
        .replace("{{ticker}}", g.env.ticker())
        .replace("{{exchange}}", g.env.prefix())
        .replace("{{interval}}", &tf_string(g.tf, 5))
        .replace("{{syminfo.currency}}", &g.env.currency)
        .replace(
            "{{time}}",
            &format_time(g.now(), "yyyy-MM-dd'T'HH:mm:ss'Z'", &jiff::tz::TimeZone::UTC),
        )
        .replace("{{timenow}}", &Timestamp::now().to_string());
    // {{plot_0}}, {{plot("Title")}}
    for (i, p) in plots.iter().enumerate() {
        let v = value::fmt_num(g.out.vals.get(i).copied().unwrap_or(f64::NAN));
        msg = msg
            .replace(&format!("{{{{plot_{i}}}}}"), &v)
            .replace(&format!("{{{{plot(\"{}\")}}}}", p.title), &v)
            .replace(&format!("{{{{plot('{}')}}}}", p.title), &v);
    }
    msg
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
    fixed_tz(rest)
}

fn time_part(g: &G, a: &[V], f: impl Fn(&jiff::Zoned) -> i64) -> V {
    let t = match arg(a, 0) {
        V::Na => g.now(),
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

/// Bar time for `time(tf, session)` / `time_close(…)`.
fn bar_time(g: &G, a: &[V], close: bool) -> R {
    let back = int_or(a, 3, 0).max(0) as usize;
    let Some(b) = g.bars.get(back).copied() else {
        return Ok(V::Na);
    };
    let tz = tz_arg(g, arg(a, 2));
    if let Some(sess) = arg(a, 1).as_str().filter(|s| !s.is_empty())
        && !in_session(g.open_time(b.time), sess, &tz)
    {
        return Ok(V::Na);
    }
    let tf = match arg(a, 0) {
        V::Str(s) if !s.is_empty() => parse_tf(s).ok_or_else(|| format!("\"{s}\" isn't a timeframe."))?,
        _ => g.tf,
    };
    let stamp = if tf == g.tf { b.time } else { g.bucket(tf, b.time) };
    Ok(V::Int(if close {
        crate::run::close_in(tf, stamp, &g.tz, g.market)
    } else {
        crate::run::open_in(tf, stamp, &g.tz, g.market)
    }))
}

/// Whether `t` falls in a session like "0930-1600" or "0930-1600:23456".
pub(crate) fn in_session(t: i64, sess: &str, tz: &jiff::tz::TimeZone) -> bool {
    let Ok(ts) = Timestamp::from_millisecond(t) else {
        return false;
    };
    let z = ts.to_zoned(tz.clone());
    let now = z.hour() as i32 * 60 + z.minute() as i32;
    let dow = z.weekday().to_sunday_one_offset() as u32;
    let hm = |s: &str| -> Option<i32> {
        let s = s.trim();
        if s.len() != 4 {
            return None;
        }
        Some(s.get(..2)?.parse::<i32>().ok()? * 60 + s.get(2..)?.parse::<i32>().ok()?)
    };
    // "0930-1200,1300-1600:23456": the days go with every range.
    let (ranges, days) = match sess.split_once(':') {
        Some((r, d)) => (r, Some(d)),
        None => (sess, None),
    };
    let on = |d: u32| days.is_none_or(|days| days.chars().any(|c| c.to_digit(10) == Some(d)));
    ranges.split(',').any(|range| {
        let Some((start, end)) = range.split_once('-').and_then(|(a, b)| Some((hm(a)?, hm(b)?))) else {
            // "24x7"
            return on(dow);
        };
        if start < end {
            return now >= start && now < end && on(dow);
        }
        if start == 0 && end == 0 {
            return on(dow);
        }
        // Overnight: the evening belongs to the next day's session.
        if now >= start {
            on(dow % 7 + 1)
        } else {
            now < end && on(dow)
        }
    })
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
        // Without a timezone of its own, text is in GMT.
        return Ok(parse_date(s, &jiff::tz::TimeZone::UTC).map(V::Int).unwrap_or(V::Na));
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
    let make = || -> Option<jiff::civil::DateTime> {
        let base = jiff::civil::Date::new(i16::try_from(y).ok()?, 1, 1)
            .ok()?
            .at(0, 0, 0, 0);
        let span = jiff::Span::new()
            .try_months(mo - 1)
            .and_then(|p| p.try_days(d - 1))
            .and_then(|p| p.try_hours(h))
            .and_then(|p| p.try_minutes(mi))
            .and_then(|p| p.try_seconds(s))
            .ok()?;
        base.checked_add(span).ok()
    };
    let Some(dt) = make() else { return Ok(V::Na) };
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
    // A timezone at the end: "+0300", "GMT-05:00", "UTC".
    let (s, tz) = match s.rsplit_once(' ') {
        Some((head, z)) if z.starts_with(['+', '-']) && z.len() >= 5 => (head, fixed_tz(z)?),
        Some((head, z)) if z.starts_with("GMT") || z.starts_with("UTC") => (
            head,
            match &z[3..] {
                "" => jiff::tz::TimeZone::UTC,
                off => fixed_tz(off)?,
            },
        ),
        _ => (s, tz.clone()),
    };
    let ms = |dt: jiff::civil::DateTime| dt.to_zoned(tz.clone()).ok().map(|z| z.timestamp().as_millisecond());
    const DATES: [&str; 6] = ["%Y-%m-%d", "%d %b %Y", "%d %B %Y", "%b %d %Y", "%B %d %Y", "%b %d, %Y"];
    for f in DATES {
        for time in [" %H:%M:%S", " %H:%M", "T%H:%M:%S", "T%H:%M"] {
            if let Ok(dt) = jiff::civil::DateTime::strptime(format!("{f}{time}"), s) {
                return ms(dt);
            }
        }
        if let Ok(d) = jiff::civil::Date::strptime(f, s) {
            return ms(d.at(0, 0, 0, 0));
        }
    }
    None
}

/// A date input's text: "2024-01-31 09:30" in the exchange's timezone.
pub(crate) fn input_date(env: &crate::Env, t: i64) -> String {
    let tz = jiff::tz::TimeZone::get(&env.timezone).unwrap_or(jiff::tz::TimeZone::UTC);
    let pat = if t.rem_euclid(60_000) == 0 {
        "yyyy-MM-dd HH:mm"
    } else {
        "yyyy-MM-dd HH:mm:ss"
    };
    format_time(t, pat, &tz)
}

pub(crate) fn date_of_input(env: &crate::Env, s: &str) -> Option<i64> {
    let tz = jiff::tz::TimeZone::get(&env.timezone).unwrap_or(jiff::tz::TimeZone::UTC);
    // A saved number is a time already.
    s.trim().parse::<i64>().ok().or_else(|| parse_date(s, &tz))
}

/// "+0300", "-05:00", "+3".
fn fixed_tz(s: &str) -> Option<jiff::tz::TimeZone> {
    let sign = if s.starts_with('-') { -1 } else { 1 };
    let body = s.trim_start_matches(['+', '-']);
    let (h, m) = match body.split_once(':') {
        Some((h, m)) => (h.parse::<i32>().ok()?, m.parse::<i32>().ok()?),
        None if body.len() == 4 => (body.get(..2)?.parse().ok()?, body.get(2..)?.parse().ok()?),
        None => (body.parse::<i32>().ok()?, 0),
    };
    jiff::tz::Offset::from_seconds(sign * (h * 3600 + m * 60))
        .ok()
        .map(jiff::tz::TimeZone::fixed)
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
        let g = G::scratch(Default::default(), 6);
        assert_eq!(
            format_msg(&g, "{0} is {1,number,#.##}", &[V::str("x"), V::Float(2.34567)]),
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
        let mut g = G::scratch(Default::default(), 6);
        g.tf = Timeframe::new(1, Unit::Minute);
        // 2020-09-13 12:26 UTC
        let t = 1_600_000_000_000;
        assert_eq!(g.bucket(Timeframe::new(1, Unit::Day), t), 1_599_955_200_000);
        assert_eq!(
            g.bucket(Timeframe::new(4, Unit::Hour), t),
            1_599_955_200_000 + 12 * 3_600_000
        );
        // Monday 2020-09-07
        assert_eq!(g.bucket(Timeframe::new(1, Unit::Week), t), 1_599_436_800_000);
        // 2020-09-01
        assert_eq!(g.bucket(Timeframe::new(1, Unit::Month), t), 1_598_918_400_000);
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

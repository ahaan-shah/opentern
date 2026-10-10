//! Runs a compiled script bar by bar. Every series variable lives in a
//! frame slot; slots that are read with `[n]` keep a ring of past values,
//! pushed once at the end of each bar in which the slot was set (so a
//! function's locals only grow history on bars where it was called, as in
//! Pine). The whole state is `Clone`, which is how the forming bar is
//! computed and rolled back.

use crate::compile::{Bin, Block, Conv, E, Field, Func, Layout, PlotHow, Program, S};
use crate::heap::{Drawings, Heap, Obj};

/// `chart.point` is a built-in object type with these fields.
pub(crate) const CHART_POINT: u16 = u16::MAX - 1;
pub(crate) const CHART_POINT_FIELDS: [&str; 3] = ["index", "time", "price"];
use crate::natives::{self, NatDef, Ta};
use crate::value::{self, V};
use crate::{Env, Error};
use opentern_core::{Bar, Timeframe, Unit};
use opentern_engine::{Broker, Props};
use opentern_series::study::{Ctx, Inputs, Out, Study, Value};
use std::collections::VecDeque;
use std::sync::Arc;

/// The most loop iterations allowed on one bar.
const MAX_OPS: u64 = 5_000_000;
/// A colour that draws nothing (alpha 0; never 0, which means "palette").
pub(crate) const INVISIBLE: u32 = 0xFFFF_FF00;

#[derive(Clone, Debug)]
pub(crate) struct Ring<T> {
    buf: VecDeque<T>,
    cap: usize,
}

impl<T: Clone> Ring<T> {
    pub fn new(cap: usize) -> Ring<T> {
        Ring {
            buf: VecDeque::new(),
            cap: cap.max(1),
        }
    }

    pub fn push(&mut self, x: T) {
        if self.buf.len() == self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(x);
    }

    /// `k` = 0 is the newest.
    pub fn get(&self, k: usize) -> Option<&T> {
        let n = self.buf.len();
        (k < n).then(|| &self.buf[n - 1 - k])
    }

    pub fn get_mut(&mut self, k: usize) -> Option<&mut T> {
        let n = self.buf.len();
        (k < n).then(|| &mut self.buf[n - 1 - k])
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.buf.iter()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Frame {
    pub v: Vec<V>,
    pub hist: Vec<Option<Ring<V>>>,
    /// The bar number each slot was last set on.
    pub touched: Vec<u32>,
    pub inited: Vec<bool>,
    pub st: Vec<Option<Ta>>,
    pub calls: Vec<Option<Box<Frame>>>,
    pub secs: Vec<Option<Box<crate::natives::SecState>>>,
}

impl Frame {
    pub fn new(l: &Layout) -> Frame {
        Frame {
            v: vec![V::Na; l.slots],
            hist: l.hist.iter().map(|&h| (h > 0).then(|| Ring::new(h))).collect(),
            touched: vec![0; l.slots],
            inited: vec![false; l.slots],
            st: vec![None; l.states],
            calls: vec![None; l.calls.len()],
            secs: vec![None; l.secs],
        }
    }

    fn hist(&self, slot: u16, k: usize) -> V {
        if k == 0 {
            return self.v[slot as usize].clone();
        }
        match &self.hist[slot as usize] {
            Some(r) => r.get(k - 1).cloned().unwrap_or(V::Na),
            None => V::Na,
        }
    }

    /// Pushes this bar's values into the histories (recursively for the
    /// call sites that ran).
    pub fn commit(&mut self, l: &Layout, funcs: &[Func], bar: u32) {
        for (i, h) in self.hist.iter_mut().enumerate() {
            if let Some(r) = h
                && self.touched[i] == bar
            {
                r.push(self.v[i].clone());
            }
        }
        for (k, c) in self.calls.iter_mut().enumerate() {
            if let Some(c) = c {
                c.commit(&funcs[l.calls[k] as usize].layout, funcs, bar);
            }
        }
    }

    pub fn refs(&self, out: &mut Vec<u32>) {
        fn walk(v: &V, out: &mut Vec<u32>) {
            match v {
                V::Ref(r) => out.push(*r),
                V::Tuple(t) => t.iter().for_each(|x| walk(x, out)),
                _ => {}
            }
        }
        self.v.iter().for_each(|v| walk(v, out));
        for r in self.hist.iter().flatten() {
            r.iter().for_each(|v| walk(v, out));
        }
        for c in self.calls.iter().flatten() {
            c.refs(out);
        }
        for s in self.secs.iter().flatten() {
            s.refs(out);
        }
        for t in self.st.iter().flatten() {
            t.refs(out);
        }
    }
}

/// This bar's outputs.
#[derive(Clone, Debug, Default)]
pub(crate) struct OutBuf {
    pub vals: Vec<f64>,
    pub cols: Vec<u8>,
    pub raw: Vec<u32>,
    pub fill_cols: Vec<u8>,
    pub fill_raw: Vec<u32>,
    pub bg: u32,
    pub bar: u32,
}

impl OutBuf {
    fn reset(&mut self) {
        self.vals.fill(f64::NAN);
        self.cols.fill(0);
        self.raw.fill(0);
        self.fill_cols.fill(0);
        self.fill_raw.fill(0);
        self.bg = 0;
        self.bar = 0;
    }
}

/// An `alert()` call or a triggered `alertcondition`.
#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    pub bar: usize,
    pub time: i64,
    pub message: String,
    /// The `alertcondition` title, empty for `alert()`.
    pub title: String,
    /// "all", "once_per_bar", "once_per_bar_close".
    pub freq: String,
}

/// Everything a running script holds besides its frames.
#[derive(Clone, Debug)]
pub(crate) struct G {
    pub env: Arc<Env>,
    pub tz: jiff::tz::TimeZone,
    pub version: u8,
    pub bars: Ring<Bar>,
    pub index: usize,
    pub bar_no: u32,
    pub count: usize,
    pub forming: bool,
    pub new_session: bool,
    pub tf: Timeframe,
    pub broker: Option<Broker>,
    pub heap: Heap,
    pub out: OutBuf,
    pub line: u32,
    pub ops: u64,
    pub logs: Vec<String>,
    pub alerts: Vec<Alert>,
    pub ntypes: usize,
    /// The first bar's time (for `strategy` date ranges, `timenow`).
    pub last_time: i64,
}

impl G {
    pub fn new(env: Arc<Env>, version: u8, bar_hist: usize) -> G {
        let tz = jiff::tz::TimeZone::get(&env.timezone).unwrap_or(jiff::tz::TimeZone::UTC);
        G {
            env,
            tz,
            version,
            bars: Ring::new(bar_hist),
            index: 0,
            bar_no: 0,
            count: 0,
            forming: false,
            new_session: false,
            tf: Timeframe::new(1, Unit::Day),
            broker: None,
            heap: Heap::default(),
            out: OutBuf::default(),
            line: 0,
            ops: 0,
            logs: Vec::new(),
            alerts: Vec::new(),
            ntypes: 0,
            last_time: 0,
        }
    }

    /// For folding constants at compile time.
    pub fn scratch(env: Arc<Env>) -> G {
        G::new(env, 6, 1)
    }

    pub fn bar(&self) -> Bar {
        self.bars.get(0).copied().unwrap_or(Bar {
            time: 0,
            open: f64::NAN,
            high: f64::NAN,
            low: f64::NAN,
            close: f64::NAN,
            volume: f64::NAN,
        })
    }

    pub fn log(&mut self, s: String) {
        if self.logs.len() >= 500 {
            self.logs.remove(0);
        }
        self.logs.push(s);
    }

    /// The close time of a bar opening at `t` on this chart's interval.
    pub fn close_time(&self, t: i64) -> i64 {
        tf_end(self.tf, t, &self.tz)
    }
}

/// The end of the `tf` bar that opens at `t`.
pub(crate) fn tf_end(tf: Timeframe, t: i64, tz: &jiff::tz::TimeZone) -> i64 {
    match tf.millis() {
        Some(ms) => t + ms,
        None => jiff::Timestamp::from_millisecond(t)
            .ok()
            .and_then(|ts| {
                ts.to_zoned(tz.clone())
                    .checked_add(jiff::Span::new().months(tf.count as i64))
                    .ok()
            })
            .map(|z| z.timestamp().as_millisecond())
            .unwrap_or(t),
    }
}

pub(crate) fn field(b: &Bar, f: Field, index: i64, g: &G) -> V {
    if b.time == 0 && b.open.is_nan() {
        return V::Na;
    }
    match f {
        Field::Open => V::Float(b.open),
        Field::High => V::Float(b.high),
        Field::Low => V::Float(b.low),
        Field::Close => V::Float(b.close),
        Field::Volume => V::Float(b.volume),
        Field::Hl2 => V::Float((b.high + b.low) / 2.0),
        Field::Hlc3 => V::Float((b.high + b.low + b.close) / 3.0),
        Field::Ohlc4 => V::Float((b.open + b.high + b.low + b.close) / 4.0),
        Field::Hlcc4 => V::Float((b.high + b.low + b.close + b.close) / 4.0),
        Field::Time => V::Int(b.time),
        Field::TimeClose => V::Int(g.close_time(b.time)),
        Field::BarIndex => V::Int(index),
    }
}

pub(crate) fn parse_hex(s: &str) -> Option<u32> {
    let h = s.trim().trim_start_matches('#');
    match h.len() {
        6 => u32::from_str_radix(h, 16).ok().map(value::opaque),
        8 => u32::from_str_radix(h, 16).ok(),
        _ => None,
    }
}

pub(crate) fn neg(v: &V) -> V {
    match v {
        V::Int(i) => V::Int(i.wrapping_neg()),
        V::Float(f) => V::Float(-f),
        _ => V::Na,
    }
}

pub(crate) fn binary(op: Bin, a: &V, b: &V) -> V {
    match op {
        Bin::Eq => V::Bool(a.equals(b)),
        // IEEE semantics, as Pine's runtime: `na != x` is true.
        Bin::Ne => V::Bool(match (a, b) {
            (V::Na, _) | (_, V::Na) => true,
            _ => !a.equals(b),
        }),
        Bin::Lt | Bin::Le | Bin::Gt | Bin::Ge => {
            if let (V::Str(x), V::Str(y)) = (a, b) {
                let o = x.cmp(y);
                return V::Bool(match op {
                    Bin::Lt => o.is_lt(),
                    Bin::Le => o.is_le(),
                    Bin::Gt => o.is_gt(),
                    _ => o.is_ge(),
                });
            }
            if let (V::Int(x), V::Int(y)) = (a, b) {
                return V::Bool(match op {
                    Bin::Lt => x < y,
                    Bin::Le => x <= y,
                    Bin::Gt => x > y,
                    _ => x >= y,
                });
            }
            let (x, y) = (a.f(), b.f());
            V::Bool(match op {
                Bin::Lt => x < y,
                Bin::Le => x <= y,
                Bin::Gt => x > y,
                _ => x >= y,
            })
        }
        Bin::Add if matches!(a, V::Str(_)) || matches!(b, V::Str(_)) => {
            let s = |v: &V| match v {
                V::Str(s) => s.to_string(),
                V::Na => "NaN".into(),
                other => value::fmt_num(other.f()),
            };
            V::str(s(a) + &s(b))
        }
        _ => {
            if a.is_na() || b.is_na() {
                return V::Na;
            }
            if let (V::Int(x), V::Int(y)) = (a, b) {
                return match op {
                    Bin::Add => V::Int(x.wrapping_add(*y)),
                    Bin::Sub => V::Int(x.wrapping_sub(*y)),
                    Bin::Mul => V::Int(x.wrapping_mul(*y)),
                    Bin::Div => {
                        if *y == 0 {
                            V::Na
                        } else {
                            V::Float(*x as f64 / *y as f64)
                        }
                    }
                    Bin::Mod => {
                        if *y == 0 {
                            V::Na
                        } else {
                            V::Int(x % y)
                        }
                    }
                    _ => V::Na,
                };
            }
            let (x, y) = (a.f(), b.f());
            V::Float(match op {
                Bin::Add => x + y,
                Bin::Sub => x - y,
                Bin::Mul => x * y,
                Bin::Div => {
                    if y == 0.0 {
                        f64::NAN
                    } else {
                        x / y
                    }
                }
                Bin::Mod => {
                    if y == 0.0 {
                        f64::NAN
                    } else {
                        x % y
                    }
                }
                _ => f64::NAN,
            })
        }
    }
}

/// Why evaluation stopped early.
#[derive(Debug)]
pub(crate) enum Stop {
    Err(String),
    Break,
    Continue,
}

impl From<String> for Stop {
    fn from(s: String) -> Stop {
        Stop::Err(s)
    }
}

type R<T> = Result<T, Stop>;

/// Call arguments, without a heap allocation for the usual few.
pub(crate) type Args = smallvec::SmallVec<[V; 6]>;

/// One step's evaluator.
pub(crate) struct X<'a> {
    pub p: &'a Program,
    pub inputs: &'a [V],
    pub srcs: &'a [Field],
    pub g: &'a mut G,
}

fn index_of(v: &V) -> R<usize> {
    match v.i() {
        Some(k) if k >= 0 => Ok(k as usize),
        Some(_) => Err(Stop::Err("History offsets can't be negative.".into())),
        None => Err(Stop::Err("The history offset is na.".into())),
    }
}

impl X<'_> {
    pub fn eval(&mut self, e: &E, f: &mut Frame, root: Option<&Frame>) -> R<V> {
        Ok(match e {
            E::K(v) => v.clone(),
            E::L(s) => f.v[*s as usize].clone(),
            E::LH(s, i) => {
                let k = index_of(&self.eval(i, f, root)?)?;
                f.hist(*s, k)
            }
            E::G(s) => root.map(|r| r.v[*s as usize].clone()).unwrap_or_default(),
            E::GH(s, i) => {
                let k = index_of(&self.eval(i, f, root)?)?;
                root.map(|r| r.hist(*s, k)).unwrap_or_default()
            }
            E::Bar(fld) => {
                let b = self.g.bar();
                field(&b, *fld, self.g.index as i64, self.g)
            }
            E::BarH(fld, i) => {
                let k = index_of(&self.eval(i, f, root)?)?;
                self.bar_back(*fld, k)
            }
            E::In(i) => self.inputs[*i as usize].clone(),
            E::InSrc(i) => {
                let b = self.g.bar();
                field(&b, self.srcs[*i as usize], self.g.index as i64, self.g)
            }
            E::InSrcH(i, k) => {
                let k = index_of(&self.eval(k, f, root)?)?;
                self.bar_back(self.srcs[*i as usize], k)
            }
            E::Neg(x) => neg(&self.eval(x, f, root)?),
            E::Not(x) => V::Bool(!self.eval(x, f, root)?.truthy()),
            E::Bin(op, a, b) => {
                // Constants are compared in place (no clone of strings).
                if let E::K(kb) = &**b {
                    let a = self.eval(a, f, root)?;
                    return Ok(binary(*op, &a, kb));
                }
                if let E::K(ka) = &**a {
                    let b = self.eval(b, f, root)?;
                    return Ok(binary(*op, ka, &b));
                }
                let a = self.eval(a, f, root)?;
                let b = self.eval(b, f, root)?;
                binary(*op, &a, &b)
            }
            E::And(a, b, lazy) => {
                let x = self.eval(a, f, root)?.truthy();
                if *lazy && !x {
                    return Ok(V::Bool(false));
                }
                let y = self.eval(b, f, root)?.truthy();
                V::Bool(x && y)
            }
            E::Or(a, b, lazy) => {
                let x = self.eval(a, f, root)?.truthy();
                if *lazy && x {
                    return Ok(V::Bool(true));
                }
                let y = self.eval(b, f, root)?.truthy();
                V::Bool(x || y)
            }
            E::Tern(c, a, b) => {
                if self.eval(c, f, root)?.truthy() {
                    self.eval(a, f, root)?
                } else {
                    self.eval(b, f, root)?
                }
            }
            E::If(branches, otherwise) => {
                for (c, b) in branches {
                    if self.eval(c, f, root)?.truthy() {
                        return self.exec(b, f, root);
                    }
                }
                match otherwise {
                    Some(b) => self.exec(b, f, root)?,
                    None => V::Na,
                }
            }
            E::Switch(subject, cases) => {
                let subj = match subject {
                    Some(s) => Some(self.eval(s, f, root)?),
                    None => None,
                };
                for (c, b) in cases {
                    let hit = match (c, &subj) {
                        (None, _) => true,
                        (Some(E::K(k)), Some(s)) => k.same(s),
                        (Some(c), Some(s)) => self.eval(c, f, root)?.same(s),
                        (Some(c), None) => self.eval(c, f, root)?.truthy(),
                    };
                    if hit {
                        return self.exec(b, f, root);
                    }
                }
                V::Na
            }
            E::Call { f: fid, site, args } => self.eval_call(*fid, *site, args, f, root)?,
            E::Nat { f: def, args, site } => self.eval_nat(def, args, *site, f, root)?,
            E::Tuple(items) => {
                let mut vals = Vec::with_capacity(items.len());
                for a in items {
                    vals.push(self.eval(a, f, root)?);
                }
                V::Tuple(vals.into())
            }
            E::HistOf { slot, val, idx } => {
                let v = self.eval(val, f, root)?;
                let s = *slot as usize;
                f.v[s] = v;
                f.touched[s] = self.g.bar_no;
                let k = index_of(&self.eval(idx, f, root)?)?;
                f.hist(*slot, k)
            }
            E::Get(obj, name) => {
                let o = self.eval(obj, f, root)?;
                self.get_field(&o, name)?
            }
            E::New(ty, vals) => {
                let mut fields = Vec::with_capacity(vals.len());
                for a in vals {
                    fields.push(self.eval(a, f, root)?);
                }
                V::Ref(self.g.heap.alloc(Obj::Udt { ty: *ty, fields }))
            }
            E::Method {
                name,
                recv,
                args,
                names,
                user,
            } => self.eval_method(name, recv, args, names, user, f, root)?,
            E::Security(sec) => natives::security(self, sec, f, root)?,
        })
    }

    // The call arms live outside `eval` so its stack frame stays small
    // (it recurses for every node).

    #[inline(never)]
    fn eval_call(&mut self, fid: u16, site: u16, args: &[E], f: &mut Frame, root: Option<&Frame>) -> R<V> {
        let mut vals = Args::new();
        for a in args {
            vals.push(self.eval(a, f, root)?);
        }
        self.call_user(fid, site, vals, f, root)
    }

    #[inline(never)]
    fn eval_nat(&mut self, def: &NatDef, args: &[E], site: u16, f: &mut Frame, root: Option<&Frame>) -> R<V> {
        let mut vals = Args::new();
        for a in args {
            vals.push(self.eval(a, f, root)?);
        }
        let mut none = None;
        let st = if site == u16::MAX {
            &mut none
        } else {
            &mut f.st[site as usize]
        };
        Ok((def.f)(self.g, &vals, st)?)
    }

    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn eval_method(
        &mut self,
        name: &str,
        recv: &E,
        args: &[E],
        names: &[Option<Arc<str>>],
        user: &[(u16, u16, u16)],
        f: &mut Frame,
        root: Option<&Frame>,
    ) -> R<V> {
        let r = self.eval(recv, f, root)?;
        let mut vals = Args::new();
        vals.push(r);
        for a in args {
            vals.push(self.eval(a, f, root)?);
        }
        self.method(name, vals, names, user, f, root)
    }
}

/// A method's arguments in parameter order: the receiver, then positional
/// ones, then named ones in their slots; `None` where nothing was passed.
fn order_args(fname: &str, params: &[&str], vals: Args, names: &[Option<Arc<str>>]) -> R<Vec<Option<V>>> {
    let mut out: Vec<Option<V>> = vec![None; params.len().max(1)];
    let mut it = vals.into_iter();
    out[0] = it.next();
    let mut pos = 1;
    for (v, n) in it.zip(names) {
        match n {
            None => {
                if pos >= out.len() {
                    out.push(Some(v));
                } else {
                    out[pos] = Some(v);
                }
                pos += 1;
            }
            Some(n) => match params.iter().position(|p| *p == n.as_ref()) {
                Some(k) => out[k] = Some(v),
                None if crate::compile::IGNORED_ARGS.contains(&n.as_ref()) => {}
                None => return Err(Stop::Err(format!("{fname}() has no argument called \"{n}\"."))),
            },
        }
    }
    Ok(out)
}

impl X<'_> {
    fn bar_back(&self, fld: Field, k: usize) -> V {
        match self.g.bars.get(k) {
            Some(b) => field(b, fld, self.g.index as i64 - k as i64, self.g),
            None => V::Na,
        }
    }

    /// The position of field `name` in an object of user type `ty`.
    pub fn field_index(&self, ty: u16, name: &str) -> R<usize> {
        let names: Vec<&str> = if ty == CHART_POINT {
            CHART_POINT_FIELDS.to_vec()
        } else {
            self.p
                .types
                .get(ty as usize)
                .map(|t| t.fields.iter().map(|s| s.as_str()).collect())
                .unwrap_or_default()
        };
        names
            .iter()
            .position(|f| *f == name)
            .ok_or_else(|| Stop::Err(format!("This object has no field called \"{name}\".")))
    }

    pub fn get_field(&self, o: &V, name: &str) -> R<V> {
        match o {
            V::Ref(r) => match self.g.heap.get(*r) {
                Some(Obj::Udt { ty, fields }) => {
                    let k = self.field_index(*ty, name)?;
                    Ok(fields.get(k).cloned().unwrap_or_default())
                }
                _ => Err(Stop::Err("This value has no fields.".into())),
            },
            V::Na => Err(Stop::Err("Can't read a field of an object that is na.".into())),
            _ => Err(Stop::Err("This value has no fields.".into())),
        }
    }

    pub fn call_user(&mut self, fid: u16, site: u16, vals: Args, f: &mut Frame, root: Option<&Frame>) -> R<V> {
        let func = &self.p.funcs[fid as usize];
        let mut child = f.calls[site as usize]
            .take()
            .unwrap_or_else(|| Box::new(Frame::new(&func.layout)));
        let bar = self.g.bar_no;
        for (k, v) in vals.into_iter().enumerate().take(child.v.len()) {
            child.v[k] = v;
            child.touched[k] = bar;
        }
        let saved_line = self.g.line;
        let r = {
            let globals: &Frame = match root {
                Some(r) => r,
                None => &*f,
            };
            self.exec(&func.body, &mut child, Some(globals))
        };
        f.calls[site as usize] = Some(child);
        self.g.line = saved_line;
        match r {
            Ok(v) => Ok(v),
            Err(Stop::Break | Stop::Continue) => Ok(V::Na),
            Err(e) => Err(e),
        }
    }

    fn method(
        &mut self,
        name: &str,
        vals: Args,
        names: &[Option<Arc<str>>],
        user: &[(u16, u16, u16)],
        f: &mut Frame,
        root: Option<&Frame>,
    ) -> R<V> {
        // The receiver's type: a user type id, or a built-in family.
        let (tid, family) = match &vals[0] {
            V::Ref(r) => match self.g.heap.get(*r) {
                Some(Obj::Udt { ty, .. }) => (Some(*ty), ""),
                Some(Obj::Array(_)) => (None, "array"),
                Some(Obj::Map(_)) => (None, "map"),
                Some(Obj::Matrix { .. }) => (None, "matrix"),
                Some(Obj::Label(_)) => (None, "label"),
                Some(Obj::Line(_)) => (None, "line"),
                Some(Obj::Box(_)) => (None, "box"),
                Some(Obj::Table(_)) => (None, "table"),
                Some(Obj::Linefill { .. }) => (None, "linefill"),
                Some(Obj::Polyline(_)) => (None, "polyline"),
                None => (None, ""),
            },
            V::Str(_) => (None, "str"),
            V::Color(_) => (None, "color"),
            V::Int(_) | V::Float(_) => (None, "math"),
            _ => (None, ""),
        };
        // A user method for this type (or for any type).
        let pick = user
            .iter()
            .find(|(_, t, _)| tid.is_some() && Some(*t) == tid)
            .or_else(|| user.iter().find(|(_, t, _)| *t == u16::MAX));
        if let Some((fid, _, site)) = pick {
            let func = &self.p.funcs[*fid as usize];
            let params: Vec<&str> = func.params.iter().map(|s| s.as_str()).collect();
            let vals = order_args(name, &params, vals, names)?
                .into_iter()
                .enumerate()
                .map(|(k, v)| {
                    v.or_else(|| func.defaults.get(k).cloned().flatten())
                        .unwrap_or_default()
                })
                .collect();
            return self.call_user(*fid, *site, vals, f, root);
        }
        // Every user-type object has copy().
        if tid.is_some() && name == "copy" {
            let V::Ref(r) = &vals[0] else { unreachable!() };
            return Ok(self.g.heap.shallow_copy(*r).map(V::Ref).unwrap_or(V::Na));
        }
        if family.is_empty() {
            if matches!(vals[0], V::Na) {
                return Err(Stop::Err(format!("Can't call {name}() on a value that is na.")));
            }
            return Err(Stop::Err(format!("No method called {name}() for this value.")));
        }
        let full = format!("{family}.{name}");
        match natives::lookup(&full) {
            Some(def) if !def.state => {
                let vals = if def.params == ["*"] {
                    vals
                } else {
                    order_args(&full, def.params, vals, names)?
                        .into_iter()
                        .map(Option::unwrap_or_default)
                        .collect()
                };
                let mut none = None;
                Ok((def.f)(self.g, &vals, &mut none)?)
            }
            _ => Err(Stop::Err(format!("No method called {name}() for {family} values."))),
        }
    }

    fn set(&mut self, f: &mut Frame, slot: u16, v: V) {
        let s = slot as usize;
        f.v[s] = v;
        f.touched[s] = self.g.bar_no;
    }

    fn tick(&mut self) -> R<()> {
        self.g.ops += 1;
        if self.g.ops > MAX_OPS {
            return Err(Stop::Err(format!(
                "A loop ran more than {} times on one bar. Is it endless?",
                MAX_OPS
            )));
        }
        Ok(())
    }

    /// Runs a block; its value is the last statement's.
    pub fn exec(&mut self, b: &Block, f: &mut Frame, root: Option<&Frame>) -> R<V> {
        let mut last = V::Na;
        for (line, s) in b {
            self.g.line = *line;
            last = self.stmt(s, f, root)?;
        }
        Ok(last)
    }

    fn stmt(&mut self, s: &S, f: &mut Frame, root: Option<&Frame>) -> R<V> {
        match s {
            S::Let(slot, e) => {
                let v = self.eval(e, f, root)?;
                self.set(f, *slot, v.clone());
                Ok(v)
            }
            S::Var(slot, e) => {
                let i = *slot as usize;
                if !f.inited[i] {
                    let v = self.eval(e, f, root)?;
                    f.v[i] = v;
                    f.inited[i] = true;
                }
                f.touched[i] = self.g.bar_no;
                Ok(f.v[i].clone())
            }
            S::LetTuple(slots, e) => {
                let v = self.eval(e, f, root)?;
                match &v {
                    V::Tuple(items) => {
                        for (k, s) in slots.iter().enumerate() {
                            if let Some(s) = s {
                                self.set(f, *s, items.get(k).cloned().unwrap_or_default());
                            }
                        }
                    }
                    V::Na => {
                        for s in slots.iter().flatten() {
                            self.set(f, *s, V::Na);
                        }
                    }
                    other => {
                        return Err(Stop::Err(format!(
                            "Expected several values on the right (a tuple), got one {}.",
                            other.type_name()
                        )));
                    }
                }
                Ok(v)
            }
            S::Set(slot, op, e) => {
                let v = self.eval(e, f, root)?;
                let nv = match op {
                    None => v,
                    Some(op) => binary(*op, &f.v[*slot as usize], &v),
                };
                self.set(f, *slot, nv.clone());
                Ok(nv)
            }
            S::SetField(obj, name, op, e) => {
                let o = self.eval(obj, f, root)?;
                let v = self.eval(e, f, root)?;
                let V::Ref(r) = o else {
                    return Err(Stop::Err("Can't set a field of an object that is na.".into()));
                };
                let ty = match self.g.heap.get(r) {
                    Some(Obj::Udt { ty, .. }) => *ty,
                    _ => return Err(Stop::Err("This value has no fields.".into())),
                };
                let k = self.field_index(ty, name)?;
                match self.g.heap.get_mut(r) {
                    Some(Obj::Udt { fields, .. }) => {
                        if k >= fields.len() {
                            fields.resize(k + 1, V::Na);
                        }
                        let nv = match op {
                            None => v,
                            Some(op) => binary(*op, &fields[k], &v),
                        };
                        fields[k] = nv.clone();
                        Ok(nv)
                    }
                    _ => Err(Stop::Err("This value has no fields.".into())),
                }
            }
            S::Expr(e) => self.eval(e, f, root),
            S::For {
                var,
                from,
                to,
                step,
                body,
            } => {
                let a = self.eval(from, f, root)?;
                let mut b = self.eval(to, f, root)?;
                if a.is_na() || b.is_na() {
                    return Ok(V::Na);
                }
                let st = match step {
                    Some(s) => self.eval(s, f, root)?,
                    None => V::Int(1),
                };
                let ints = matches!((&a, &b, &st), (V::Int(_), V::Int(_), V::Int(_)));
                let up = a.f() <= b.f();
                let stepf = st.f().abs();
                if stepf == 0.0 || stepf.is_nan() {
                    return Err(Stop::Err("A for loop's step can't be 0.".into()));
                }
                let mut i = a.f();
                let mut last = V::Na;
                loop {
                    if self.g.version >= 6 && !matches!(to, E::K(_)) && i != a.f() {
                        b = self.eval(to, f, root)?;
                    }
                    let end = b.f();
                    if (up && i > end) || (!up && i < end) || end.is_nan() {
                        break;
                    }
                    self.tick()?;
                    let iv = if ints { V::Int(i as i64) } else { V::Float(i) };
                    self.set(f, *var, iv);
                    match self.exec(body, f, root) {
                        Ok(v) => last = v,
                        Err(Stop::Break) => break,
                        Err(Stop::Continue) => {}
                        Err(e) => return Err(e),
                    }
                    i += if up { stepf } else { -stepf };
                }
                Ok(last)
            }
            S::ForIn { idx, var, over, body } => {
                let o = self.eval(over, f, root)?;
                let items: Vec<(V, V)> = match &o {
                    V::Ref(r) => match self.g.heap.get(*r) {
                        Some(Obj::Array(a)) => a
                            .iter()
                            .enumerate()
                            .map(|(k, v)| (V::Int(k as i64), v.clone()))
                            .collect(),
                        Some(Obj::Map(m)) => m.keys.iter().cloned().zip(m.vals.iter().cloned()).collect(),
                        Some(Obj::Matrix { rows, cols, data }) => {
                            let (rows, cols) = (*rows, *cols);
                            let data = data.clone();
                            (0..rows)
                                .map(|r| {
                                    let row = data[r * cols..(r + 1) * cols].to_vec();
                                    (V::Int(r as i64), V::Ref(self.g.heap.alloc(Obj::Array(row))))
                                })
                                .collect()
                        }
                        _ => return Err(Stop::Err("for…in needs an array, a map or a matrix.".into())),
                    },
                    V::Na => Vec::new(),
                    _ => return Err(Stop::Err("for…in needs an array, a map or a matrix.".into())),
                };
                let mut last = V::Na;
                for (k, v) in items {
                    self.tick()?;
                    if let Some(i) = idx {
                        self.set(f, *i, k);
                    }
                    self.set(f, *var, v);
                    match self.exec(body, f, root) {
                        Ok(v) => last = v,
                        Err(Stop::Break) => break,
                        Err(Stop::Continue) => {}
                        Err(e) => return Err(e),
                    }
                }
                Ok(last)
            }
            S::While { cond, body } => {
                let mut last = V::Na;
                while self.eval(cond, f, root)?.truthy() {
                    self.tick()?;
                    match self.exec(body, f, root) {
                        Ok(v) => last = v,
                        Err(Stop::Break) => break,
                        Err(Stop::Continue) => {}
                        Err(e) => return Err(e),
                    }
                }
                Ok(last)
            }
            S::Break => Err(Stop::Break),
            S::Continue => Err(Stop::Continue),
            S::Plot { i, val, color, how } => {
                let v = self.eval(val, f, root)?;
                let i = *i as usize;
                match how {
                    PlotHow::Value => self.g.out.vals[i] = v.f(),
                    PlotHow::Flag => {
                        self.g.out.vals[i] = if v.truthy() { 1.0 } else { f64::NAN };
                    }
                    PlotHow::Arrow => {
                        let x = v.f();
                        if x > 0.0 {
                            self.g.out.vals[i] = x;
                        } else if x < 0.0 {
                            self.g.out.vals[i + 1] = x;
                        }
                    }
                }
                if let Some(c) = color {
                    let c = self.eval(c, f, root)?;
                    self.plot_color(i, &c);
                }
                Ok(V::Na)
            }
            S::Candle { i, o, h, l, c, color } => {
                let i = *i as usize;
                let vals = [
                    self.eval(o, f, root)?,
                    self.eval(h, f, root)?,
                    self.eval(l, f, root)?,
                    self.eval(c, f, root)?,
                ];
                for (k, v) in vals.iter().enumerate() {
                    self.g.out.vals[i + k] = v.f();
                }
                if let Some(c) = color {
                    let c = self.eval(c, f, root)?;
                    self.plot_color(i, &c);
                }
                Ok(V::Na)
            }
            S::FillColor { i, color } => {
                let c = self.eval(color, f, root)?;
                let i = *i as usize;
                let pal = &self.p.fills[i].palette;
                match c.color() {
                    Some(c) => match pal.iter().position(|p| *p == c) {
                        Some(k) => self.g.out.fill_cols[i] = k as u8,
                        None => self.g.out.fill_raw[i] = if c == 0 { INVISIBLE } else { c },
                    },
                    None => self.g.out.fill_raw[i] = INVISIBLE,
                }
                Ok(V::Na)
            }
            S::Bg(c) => {
                if let Some(c) = self.eval(c, f, root)?.color()
                    && c & 0xFF != 0
                {
                    self.g.out.bg = c;
                }
                Ok(V::Na)
            }
            S::BarColor(c) => {
                if let Some(c) = self.eval(c, f, root)?.color()
                    && c & 0xFF != 0
                {
                    self.g.out.bar = c;
                }
                Ok(V::Na)
            }
            S::AlertCond { i, cond } => {
                if self.eval(cond, f, root)?.truthy() {
                    let d = &self.p.alertconditions[*i as usize];
                    let b = self.g.bar();
                    let message = natives::alert_placeholders(self.g, &d.message);
                    self.g.alerts.push(Alert {
                        bar: self.g.index,
                        time: b.time,
                        message,
                        title: d.title.clone(),
                        freq: "once_per_bar".into(),
                    });
                }
                Ok(V::Na)
            }
        }
    }

    fn plot_color(&mut self, i: usize, c: &V) {
        let pal = &self.p.plots[i].palette;
        match c.color() {
            Some(c) => match pal.iter().position(|p| *p == c) {
                Some(k) => self.g.out.cols[i] = k as u8,
                None => self.g.out.raw[i] = if c == 0 { INVISIBLE } else { c },
            },
            None => self.g.out.raw[i] = INVISIBLE,
        }
    }
}

/// A Pine script running on a series: the study the chart steps.
#[derive(Clone)]
pub struct PineStudy {
    prog: Arc<Program>,
    inputs: Arc<[V]>,
    srcs: Arc<[Field]>,
    root: Frame,
    g: G,
    error: Option<Error>,
}

impl PineStudy {
    pub(crate) fn new(prog: Arc<Program>, inputs: &Inputs) -> PineStudy {
        let mut vals = Vec::new();
        let mut srcs = Vec::new();
        for d in &prog.inputs {
            let id = d.spec.id;
            let text = |inputs: &Inputs| -> String {
                match inputs.values.get(id).unwrap_or(&d.spec.default) {
                    Value::Text(t) => t.clone(),
                    Value::Int(i) => i.to_string(),
                    Value::Float(f) => value::fmt_num(*f),
                    Value::Bool(b) => b.to_string(),
                }
            };
            let v = match d.conv {
                Conv::Int => V::Int(match inputs.values.get(id) {
                    Some(_) => inputs.int(id),
                    None => match &d.spec.default {
                        Value::Int(i) => *i,
                        _ => 0,
                    },
                }),
                Conv::Float => V::Float(match inputs.values.get(id) {
                    Some(_) => inputs.float(id),
                    None => match &d.spec.default {
                        Value::Float(f) => *f,
                        Value::Int(i) => *i as f64,
                        _ => 0.0,
                    },
                }),
                Conv::Bool => V::Bool(match inputs.values.get(id) {
                    Some(_) => inputs.bool(id),
                    None => matches!(d.spec.default, Value::Bool(true)),
                }),
                Conv::Str | Conv::Source => V::str(text(inputs)),
                Conv::Color => V::Color(parse_hex(&text(inputs)).unwrap_or(0)),
                Conv::IntChoice => V::Int(text(inputs).parse::<f64>().map(|x| x as i64).unwrap_or(0)),
                Conv::FloatChoice => V::Float(text(inputs).parse().unwrap_or(0.0)),
            };
            if d.conv == Conv::Source {
                srcs.push(Field::by_name(&text(inputs)).unwrap_or(Field::Close));
            } else {
                srcs.push(Field::Close);
            }
            vals.push(v);
        }
        let mut g = G::new(prog.env.clone(), prog.version, prog.bar_hist);
        g.ntypes = prog.types.len();
        g.heap.limits = prog.limits;
        if prog.kind == crate::Kind::Strategy {
            g.broker = Some(Broker::new(Props::from_json(&prog.props, prog.env.mintick)));
        }
        g.out = OutBuf {
            vals: vec![f64::NAN; prog.plots.len()],
            cols: vec![0; prog.plots.len()],
            raw: vec![0; prog.plots.len()],
            fill_cols: vec![0; prog.fills.len()],
            fill_raw: vec![0; prog.fills.len()],
            bg: 0,
            bar: 0,
        };
        PineStudy {
            root: Frame::new(&prog.layout),
            prog,
            inputs: vals.into(),
            srcs: srcs.into(),
            g,
            error: None,
        }
    }

    /// The runtime error that stopped the script, if any.
    pub fn error(&self) -> Option<&Error> {
        self.error.as_ref()
    }

    /// The strategy's broker (orders, trades, equity), for strategies.
    pub fn broker(&self) -> Option<&Broker> {
        self.g.broker.as_ref()
    }

    pub fn drawings(&self) -> Drawings {
        self.g.heap.drawings()
    }

    /// `log.*` output, newest last.
    pub fn logs(&self) -> &[String] {
        &self.g.logs
    }

    pub fn alerts(&self) -> &[Alert] {
        &self.g.alerts
    }

    /// Runs on a whole series at once (tests, screeners, backtests): every
    /// bar is final.
    pub fn run(&mut self, bars: &[Bar], tf: Timeframe) -> opentern_series::study::Output {
        let tz = self.g.tz.clone();
        let sessions = opentern_series::study::sessions(bars, tf, &tz);
        self.run_with(bars, &sessions, tf)
    }

    /// As [`PineStudy::run`], with the session starts already known.
    pub fn run_with(&mut self, bars: &[Bar], sessions: &[bool], tf: Timeframe) -> opentern_series::study::Output {
        let n_plots = self.prog.plots.len();
        let n_fills = self.prog.fills.len();
        let mut out = opentern_series::study::Output {
            plots: vec![Vec::with_capacity(bars.len()); n_plots],
            colors: vec![Vec::with_capacity(bars.len()); n_plots],
            fills: vec![Vec::with_capacity(bars.len()); n_fills],
            raw: vec![Vec::with_capacity(bars.len()); n_plots],
            fill_raw: vec![Vec::with_capacity(bars.len()); n_fills],
            bg: Vec::with_capacity(bars.len()),
            bar: Vec::with_capacity(bars.len()),
        };
        let mut o = Out {
            values: vec![f64::NAN; n_plots],
            colors: vec![0; n_plots],
            fills: vec![0; n_fills],
            raw: vec![0; n_plots],
            fill_raw: vec![0; n_fills],
            bg: 0,
            bar: 0,
        };
        for (i, b) in bars.iter().enumerate() {
            let c = Ctx {
                bar: b,
                index: i,
                new_session: sessions.get(i).copied().unwrap_or(false),
                tf,
                forming: false,
                count: bars.len(),
            };
            o.values.fill(f64::NAN);
            o.colors.fill(0);
            o.fills.fill(0);
            o.raw.fill(0);
            o.fill_raw.fill(0);
            o.bg = 0;
            o.bar = 0;
            self.step(&c, &mut o);
            for k in 0..n_plots {
                out.plots[k].push(o.values[k]);
                out.colors[k].push(o.colors[k]);
                out.raw[k].push(o.raw[k]);
            }
            for k in 0..n_fills {
                out.fills[k].push(o.fills[k]);
                out.fill_raw[k].push(o.fill_raw[k]);
            }
            out.bg.push(o.bg);
            out.bar.push(o.bar);
        }
        out
    }

    fn collect(&mut self) {
        let mut roots = Vec::new();
        self.root.refs(&mut roots);
        self.g.heap.gc(&mut roots.into_iter());
    }
}

impl Study for PineStudy {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        if self.error.is_some() {
            return;
        }
        let g = &mut self.g;
        g.bars.push(*c.bar);
        g.index = c.index;
        g.bar_no += 1;
        g.count = c.count;
        g.forming = c.forming;
        g.new_session = c.new_session;
        g.tf = c.tf;
        g.ops = 0;
        g.last_time = g.last_time.max(c.bar.time);
        g.out.reset();
        if let Some(b) = &mut g.broker {
            b.open_bar(c.index, c.bar);
        }
        let prog = &*self.prog;
        let mut x = X {
            p: prog,
            inputs: &self.inputs,
            srcs: &self.srcs,
            g: &mut self.g,
        };
        let r = x.exec(&prog.root, &mut self.root, None);
        if let Err(e) = r {
            let msg = match e {
                Stop::Err(m) => m,
                Stop::Break | Stop::Continue => "break/continue outside a loop.".into(),
            };
            self.error = Some(Error::at(self.g.line as usize, msg));
            return;
        }
        if let Some(b) = &mut self.g.broker {
            b.close_bar(c.bar);
        }
        self.root.commit(&prog.layout, &prog.funcs, self.g.bar_no);
        if self.g.heap.wants_gc() {
            self.collect();
        }
        let out = &self.g.out;
        for (d, v) in o.values.iter_mut().zip(&out.vals) {
            *d = if v.is_infinite() { f64::NAN } else { *v };
        }
        o.colors.copy_from_slice(&out.cols);
        o.raw.copy_from_slice(&out.raw);
        o.fills.copy_from_slice(&out.fill_cols);
        o.fill_raw.copy_from_slice(&out.fill_raw);
        o.bg = out.bg;
        o.bar = out.bar;
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn drawings(&self) -> Option<Drawings> {
        Some(self.g.heap.drawings())
    }
}

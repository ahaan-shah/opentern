//! JavaScript and TypeScript scripts (QuickJS via `rquickjs`). TypeScript
//! is stripped of its types first (`swc_ts_fast_strip`, which blanks them
//! out so line numbers stay the same). No file, network or module access;
//! a memory cap and a time budget per bar, as for Luau.
//!
//! ```ts
//! indicator("RSI", { overlay: false });
//! const len: number = input.int("Length", 14, { min: 1 });
//! hline("Overbought", 70);
//!
//! function onBar(bar: Bar) {
//!   plot(ta.rsi(bar.close, len), "RSI", { color: "#7E57C2" });
//! }
//! ```

use crate::host::TaOut;
use crate::host::{BAR_FLAGS, BAR_NUMS, COLORS, LOADTIME, RUNTIME, ScriptError, SymInfo, TA, Val};
use crate::inproc::{Lang, Phase, Shared, Vm, dispatch, line_in, with_run};
use rquickjs::context::EvalOptions;
use rquickjs::function::Rest;
use rquickjs::{Array, Context, Ctx, Exception, Function, Object, Runtime, Value};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// JavaScript, or TypeScript when `ts` is set.
pub struct Js {
    pub ts: bool,
}

/// Where the reused `bar` object lives.
const BAR_GLOBAL: &str = "__opentern_bar";
const BAR_BUDGET: Duration = Duration::from_secs(1);
const LOAD_BUDGET: Duration = Duration::from_secs(5);
const MEMORY: usize = 256 << 20;

fn epoch() -> Instant {
    static E: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *E.get_or_init(Instant::now)
}

fn now_ns() -> u64 {
    epoch().elapsed().as_nanos() as u64
}

/// TypeScript → JavaScript, types blanked out in place.
pub fn strip_types(src: &str, file: &str) -> Result<String, ScriptError> {
    use swc_common::errors::{DiagnosticBuilder, Emitter, Handler};
    use swc_common::sync::Lrc;
    use swc_common::{BytePos, SourceMap};
    type Found = Arc<std::sync::Mutex<Vec<(String, Option<BytePos>)>>>;
    struct Collect(Found);
    impl Emitter for Collect {
        fn emit(&mut self, db: &mut DiagnosticBuilder<'_>) {
            let lo = db.span.primary_span().map(|s| s.lo);
            if let Ok(mut v) = self.0.lock() {
                v.push((db.message(), lo));
            }
        }
    }
    let found = Arc::new(std::sync::Mutex::new(Vec::new()));
    let cm: Lrc<SourceMap> = Default::default();
    let handler = Handler::with_emitter(true, false, Box::new(Collect(found.clone())));
    let opts = swc_ts_fast_strip::Options {
        filename: Some(file.to_string()),
        ..Default::default()
    };
    let r = swc_common::GLOBALS.set(&Default::default(), || {
        swc_ts_fast_strip::operate(&cm, &handler, src.to_string(), opts)
    });
    let first = found.lock().ok().and_then(|v| v.first().cloned());
    match (r, first) {
        (Ok(out), None) => Ok(out.code),
        (_, Some((msg, lo))) => {
            let line = lo.map(|p| cm.lookup_char_pos(p).line).unwrap_or(0);
            Err(ScriptError::new(line, msg))
        }
        (Err(e), None) => Err(ScriptError::new(0, e.message)),
    }
}

fn to_val(v: &Value, depth: usize) -> Val {
    if depth > 32 {
        return Val::Nil;
    }
    if let Some(b) = v.as_bool() {
        return Val::Bool(b);
    }
    if let Some(n) = v.as_number() {
        return Val::Num(n);
    }
    if let Some(s) = v.as_string() {
        return Val::Str(s.to_string().unwrap_or_default());
    }
    if v.is_function() {
        return Val::Nil;
    }
    if let Some(a) = v.as_array() {
        return Val::List(
            a.iter::<Value>()
                .map(|x| x.map(|x| to_val(&x, depth + 1)).unwrap_or(Val::Nil))
                .collect(),
        );
    }
    if let Some(o) = v.as_object() {
        let mut m: Vec<(String, Val)> = o
            .props::<String, Value>()
            .flatten()
            .map(|(k, x)| (k, to_val(&x, depth + 1)))
            .collect();
        m.sort_by(|a, b| a.0.cmp(&b.0));
        return Val::Map(m);
    }
    Val::Nil
}

fn from_val<'js>(ctx: &Ctx<'js>, v: &Val) -> rquickjs::Result<Value<'js>> {
    Ok(match v {
        Val::Nil => Value::new_undefined(ctx.clone()),
        Val::Bool(b) => Value::new_bool(ctx.clone(), *b),
        Val::Num(n) => Value::new_number(ctx.clone(), *n),
        Val::Str(s) => rquickjs::String::from_str(ctx.clone(), s)?.into_value(),
        Val::List(l) => {
            let a = Array::new(ctx.clone())?;
            for (i, x) in l.iter().enumerate() {
                a.set(i, from_val(ctx, x)?)?;
            }
            a.into_value()
        }
        Val::Map(m) => {
            let o = Object::new(ctx.clone())?;
            for (k, x) in m {
                o.set(k.as_str(), from_val(ctx, x)?)?;
            }
            o.into_value()
        }
    })
}

fn host_fn<'js>(ctx: &Ctx<'js>, shared: &Shared, name: &str) -> rquickjs::Result<Function<'js>> {
    let shared = shared.clone();
    let name = name.to_string();
    Function::new(
        ctx.clone(),
        move |c: Ctx<'js>, args: Rest<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            let args: Vec<Val> = args.0.iter().map(|v| to_val(v, 0)).collect();
            match dispatch(&shared, &name, &args) {
                Ok(v) => from_val(&c, &v),
                Err(m) => Err(Exception::throw_message(&c, &m)),
            }
        },
    )
}

fn num_of(v: &Value) -> f64 {
    if let Some(n) = v.as_number() {
        n
    } else if let Some(b) = v.as_bool() {
        if b { 1.0 } else { 0.0 }
    } else {
        f64::NAN
    }
}

/// `ta.*`: numbers in, numbers out.
fn ta_fn<'js>(ctx: &Ctx<'js>, shared: &Shared, idx: usize) -> rquickjs::Result<Function<'js>> {
    let shared = shared.clone();
    let name = format!("ta.{}", TA[idx].0);
    Function::new(
        ctx.clone(),
        move |c: Ctx<'js>, args: Rest<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            let mut f = [f64::NAN; 5];
            for (d, v) in f.iter_mut().zip(args.0.iter()) {
                *d = num_of(v);
            }
            match with_run(&shared, &name, |h| h.ta_fast(idx, &f)) {
                Ok(TaOut::One(x)) => Ok(Value::new_number(c, x)),
                Ok(TaOut::Bool(b)) => Ok(Value::new_bool(c, b)),
                Ok(TaOut::Many(v, n)) => {
                    let a = Array::new(c.clone())?;
                    for (i, x) in v[..n].iter().enumerate() {
                        a.set(i, *x)?;
                    }
                    Ok(a.into_value())
                }
                Err(m) => Err(Exception::throw_message(&c, &m)),
            }
        },
    )
}

/// `plot(value, title, opts)`: only the colour is read once plots are known.
fn plot_fn<'js>(ctx: &Ctx<'js>, shared: &Shared) -> rquickjs::Result<Function<'js>> {
    let shared = shared.clone();
    Function::new(
        ctx.clone(),
        move |c: Ctx<'js>, args: Rest<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            let mut p = shared.lock().unwrap_or_else(|e| e.into_inner());
            let Phase::Run(h) = &mut *p else {
                return Err(Exception::throw_message(
                    &c,
                    "plot() goes inside onBar, called on every bar.",
                ));
            };
            let r = if h.discovering() {
                let vals: Vec<Val> = args.0.iter().map(|v| to_val(v, 0)).collect();
                h.call("plot", &vals).map(|_| ())
            } else {
                let color = args
                    .0
                    .get(2)
                    .and_then(|o| o.as_object())
                    .and_then(|o| o.get::<_, Option<String>>("color").ok().flatten())
                    .and_then(|s| crate::host::parse_color(&s));
                h.plot_fast(args.0.first().map(num_of).unwrap_or(f64::NAN), color)
            };
            match r {
                Ok(()) => Ok(Value::new_undefined(c)),
                Err(m) => Err(Exception::throw_message(&c, &m)),
            }
        },
    )
}

/// The object a dotted name lives in (`ta.sma` → the `ta` object).
fn object_for<'js>(ctx: &Ctx<'js>, root: &Object<'js>, name: &str) -> rquickjs::Result<(Object<'js>, String)> {
    match name.split_once('.') {
        Some((t, rest)) => {
            let v: Value = root.get(t)?;
            let o = match v.into_object() {
                Some(o) => o,
                None => {
                    let o = Object::new(ctx.clone())?;
                    root.set(t, o.clone())?;
                    o
                }
            };
            Ok((o, rest.to_string()))
        }
        None => Ok((root.clone(), name.to_string())),
    }
}

/// The error a failed call left behind, with its line.
fn caught(ctx: &Ctx, e: rquickjs::Error, file: &str) -> ScriptError {
    if !matches!(e, rquickjs::Error::Exception) {
        return ScriptError::new(0, e.to_string());
    }
    let v = ctx.catch();
    if let Some(ex) = v.as_exception() {
        let msg = ex.message().unwrap_or_default();
        let stack = ex.stack().unwrap_or_default();
        let msg = if msg == "interrupted" {
            "The script took too long on one bar (over a second). Look for a loop that never ends.".to_string()
        } else if msg.contains("out of memory") {
            "The script used too much memory.".to_string()
        } else {
            msg
        };
        return ScriptError::new(line_in(&stack, file), msg);
    }
    ScriptError::new(0, crate::host::show(&to_val(&v, 0)))
}

impl Lang for Js {
    fn key(&self) -> &'static str {
        if self.ts { "ts" } else { "js" }
    }

    fn create(&self, src: &str, file: &str, shared: Shared, syminfo: &SymInfo) -> Result<Box<dyn Vm>, ScriptError> {
        let code = if self.ts {
            strip_types(src, file)?
        } else {
            src.to_string()
        };
        let start = |e: rquickjs::Error| ScriptError::new(0, format!("Couldn't start JavaScript: {e}"));
        let rt = Runtime::new().map_err(start)?;
        rt.set_memory_limit(MEMORY);
        rt.set_max_stack_size(1 << 20);
        let deadline = Arc::new(AtomicU64::new(u64::MAX));
        let d = deadline.clone();
        rt.set_interrupt_handler(Some(Box::new(move || now_ns() > d.load(Ordering::Relaxed))));
        let ctx = Context::full(&rt).map_err(start)?;
        deadline.store(now_ns() + LOAD_BUDGET.as_nanos() as u64, Ordering::Relaxed);
        let on_bar = ctx.with(|ctx| -> Result<&'static str, ScriptError> {
            let setup = || -> rquickjs::Result<()> {
                let g = ctx.globals();
                for name in RUNTIME.iter().chain(LOADTIME) {
                    if *name == "strategy" || *name == "plot" {
                        continue;
                    }
                    let f = host_fn(&ctx, &shared, name)?;
                    if name.starts_with("strategy.") {
                        continue;
                    }
                    let (o, key) = object_for(&ctx, &g, name)?;
                    o.set(key, f)?;
                }
                // `strategy(...)` declares; `strategy.entry(...)` trades.
                let strat = host_fn(&ctx, &shared, "strategy")?;
                for name in RUNTIME.iter().filter_map(|n| n.strip_prefix("strategy.")) {
                    strat.set(name, host_fn(&ctx, &shared, &format!("strategy.{name}"))?)?;
                }
                strat.set("long", "long")?;
                strat.set("short", "short")?;
                g.set("strategy", strat)?;
                let ta = Object::new(ctx.clone())?;
                for (idx, (n, _)) in TA.iter().enumerate() {
                    ta.set(*n, ta_fn(&ctx, &shared, idx)?)?;
                }
                g.set("plot", plot_fn(&ctx, &shared)?)?;
                g.set("ta", ta)?;
                let (color, _) = object_for(&ctx, &g, "color.new")?;
                for (n, c) in COLORS {
                    color.set(*n, format!("#{c:06X}FF"))?;
                }
                g.set("syminfo", from_val(&ctx, &syminfo.to_val())?)?;
                g.set("state", Object::new(ctx.clone())?)?;
                // The `bar` object, refilled every bar (see `JsVm::on_bar`).
                g.set(BAR_GLOBAL, Object::new(ctx.clone())?)?;
                Ok(())
            };
            setup().map_err(|e| caught(&ctx, e, file))?;
            let mut opts = EvalOptions::default();
            opts.global = true;
            opts.strict = false;
            opts.filename = Some(file.to_string());
            ctx.eval_with_options::<(), _>(code.as_bytes().to_vec(), opts)
                .map_err(|e| caught(&ctx, e, file))?;
            let g = ctx.globals();
            let f = ["onBar", "on_bar"]
                .into_iter()
                .find(|n| g.get::<_, Value>(*n).ok().is_some_and(|v| v.is_function()));
            match f {
                Some(f) => Ok(f),
                None => Err(ScriptError::new(
                    0,
                    "Define the function that runs on every bar: function onBar(bar) { … }",
                )),
            }
        })?;
        Ok(Box::new(JsVm {
            _rt: rt,
            ctx,
            on_bar,
            deadline,
            file: file.to_string(),
        }))
    }
}

struct JsVm {
    _rt: Runtime,
    ctx: Context,
    /// The global function that runs on every bar ("onBar" or "on_bar").
    on_bar: &'static str,
    deadline: Arc<AtomicU64>,
    file: String,
}

impl Vm for JsVm {
    fn on_bar(&mut self, nums: &[f64; 11], flags: &[bool; 3]) -> Result<(), ScriptError> {
        self.deadline
            .store(now_ns() + BAR_BUDGET.as_nanos() as u64, Ordering::Relaxed);
        let file = &self.file;
        let name = self.on_bar;
        self.ctx.with(|ctx| {
            let run = || -> rquickjs::Result<()> {
                let g = ctx.globals();
                let f: Function = g.get(name)?;
                let bar: Object = g.get(BAR_GLOBAL)?;
                for (k, v) in BAR_NUMS.iter().zip(nums) {
                    bar.set(*k, *v)?;
                }
                for (k, v) in BAR_FLAGS.iter().zip(flags) {
                    bar.set(*k, *v)?;
                }
                f.call::<_, ()>((bar,))
            };
            run().map_err(|e| caught(&ctx, e, file))
        })
    }

    fn get_state(&mut self) -> Result<Val, ScriptError> {
        let file = &self.file;
        self.ctx.with(|ctx| {
            let v: Value = ctx.globals().get("state").map_err(|e| caught(&ctx, e, file))?;
            if let Some(o) = v.as_object()
                && !v.is_array()
                && o.keys::<String>().next().is_none()
            {
                return Ok(Val::Map(Vec::new()));
            }
            Ok(to_val(&v, 0))
        })
    }

    fn set_state(&mut self, v: &Val) -> Result<(), ScriptError> {
        let file = &self.file;
        self.ctx.with(|ctx| {
            let jv = match v {
                Val::Nil => Object::new(ctx.clone()).map(|o| o.into_value()),
                v => from_val(&ctx, v),
            }
            .map_err(|e| caught(&ctx, e, file))?;
            ctx.globals().set("state", jv).map_err(|e| caught(&ctx, e, file))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inproc::{Compiled, compile};
    use opentern_core::{Bar, Timeframe};
    use opentern_series::study::{Instance, sessions};

    fn comp(src: &str, ts: bool) -> Result<Compiled, ScriptError> {
        let file = if ts { "test.ts" } else { "test.js" };
        compile(Arc::new(Js { ts }), src, file, SymInfo::default())
    }

    fn run(c: &Compiled, bars: &[Bar]) -> Instance {
        let m = c.meta();
        let mut inst = Instance::with_factory(m, &m.default_inputs(), c.factory());
        let tf = Timeframe::parse("1h").expect("tf");
        inst.recompute(bars, &sessions(bars, tf, &jiff::tz::TimeZone::UTC), tf);
        inst
    }

    #[test]
    fn javascript_and_typescript_match_luau() {
        let js = comp(
            r##"
indicator("MACD JS");
const fast = input.int("Fast", 12, { min: 1 });
function onBar(bar) {
  const [m, s, h] = ta.macd(bar.close, fast, 26, 9);
  plot(m, "MACD", { color: "#2962FF" });
  plot(h, "Hist", { style: "histogram", color: h >= 0 ? "#26A69A" : "#FF5252" });
  state.n = (state.n || 0) + 1;
}
"##,
            false,
        )
        .expect("js compiles");
        let ts = comp(
            r##"
indicator("MACD TS");
const fast: number = input.int("Fast", 12, { min: 1 });
interface Bar { close: number }
function onBar(bar: Bar): void {
  const [m, s, h] = ta.macd(bar.close, fast, 26, 9) as number[];
  plot(m, "MACD", { color: "#2962FF" });
  plot(h, "Hist", { style: "histogram", color: h >= 0 ? "#26A69A" : "#FF5252" });
}
"##,
            true,
        )
        .expect("ts compiles");
        let lu = compile(
            Arc::new(crate::luau::Luau),
            r##"
indicator("MACD Luau")
local fast = input.int("Fast", 12, { min = 1 })
function on_bar(bar)
  local m, s, h = ta.macd(bar.close, fast, 26, 9)
  plot(m, "MACD")
  plot(h, "Hist", { style = "histogram" })
end
"##,
            "test.luau",
            SymInfo::default(),
        )
        .expect("luau compiles");
        assert_eq!(js.meta().plots.len(), 2);
        assert_eq!(js.meta().plots[1].style, opentern_series::study::PlotStyle::Histogram);
        let bars = crate::host::dry_bars(300);
        let (a, b, c) = (run(&js, &bars), run(&ts, &bars), run(&lu, &bars));
        for i in 0..bars.len() {
            for p in 0..2 {
                let (x, y, z) = (a.out.plots[p][i], b.out.plots[p][i], c.out.plots[p][i]);
                let same = |u: f64, v: f64| u == v || (u.is_nan() && v.is_nan());
                assert!(same(x, y) && same(y, z), "bar {i} plot {p}: {x} {y} {z}");
            }
        }
        // Per-bar colours come through as raw colours.
        assert!(a.out.raw[1].iter().any(|c| *c != 0));
    }

    #[test]
    fn errors_have_lines_and_runaways_stop() {
        let e = comp(
            "indicator(\"X\");\nfunction onBar(bar) {\n  plot(ta.nope(1));\n}\n",
            false,
        )
        .unwrap_err();
        assert_eq!(e.line, 3, "{e:?}");
        assert!(
            e.message.contains("not a function") || e.message.contains("nope"),
            "{e:?}"
        );
        let e = comp("indicator(\"X\");\nconst x: number = = 1;\n", true).unwrap_err();
        assert_eq!(e.line, 2, "{e:?}");
        let e = comp(
            "indicator(\"X\");\nfunction onBar(bar) {\n  ta.sma(bar.close, 0);\n}\n",
            false,
        )
        .unwrap_err();
        assert!(e.message.contains("length of at least 1"), "{e:?}");
        assert_eq!(e.line, 3);
        let c = comp(
            "indicator(\"X\");\nfunction onBar(bar) {\n  if (bar.index === 70) { while (true) {} }\n}\n",
            false,
        )
        .expect("compiles");
        let t = Instant::now();
        let inst = run(&c, &crate::host::dry_bars(100));
        assert!(t.elapsed() < Duration::from_secs(5));
        let st = inst
            .state()
            .and_then(|a| a.downcast_ref::<crate::ScriptStudy>())
            .expect("state");
        let e = st.error().expect("error");
        assert!(e.message.contains("too long"), "{e:?}");
        assert_eq!(e.line, 3);
    }

    #[test]
    fn a_typescript_strategy_trades() {
        let c = comp(
            r##"
strategy("TS cross", { initial_capital: 5000 });
function onBar(bar: { close: number }) {
  const f: number = ta.ema(bar.close, 8);
  const s: number = ta.ema(bar.close, 21);
  const up: boolean = ta.crossover(f, s);
  const down: boolean = ta.crossunder(f, s);
  if (up) strategy.entry("L", strategy.long, { comment: "Go" });
  if (down) strategy.close("L");
  if (strategy.position_size() > 0) log("in", strategy.position_size());
}
"##,
            true,
        )
        .expect("compiles");
        let inst = run(&c, &crate::host::dry_bars(400));
        let st = inst
            .latest_state()
            .and_then(|a| a.downcast_ref::<crate::ScriptStudy>())
            .expect("state");
        let b = st.broker().expect("broker");
        assert!(!b.trades.is_empty());
        assert_eq!(b.trades[0].entry_comment, "Go");
        assert_eq!(b.props.initial_capital, 5000.0);
        assert!(!st.logs().is_empty());
    }
}

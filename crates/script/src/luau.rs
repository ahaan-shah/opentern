//! Luau scripts (via `mlua`). Luau is sandboxed by design (no file or
//! network access); the host adds a memory cap and a time budget per bar,
//! so a runaway loop stops with a plain error instead of freezing the app.
//!
//! ```lua
//! indicator("RSI", { overlay = false })
//! local len = input.int("Length", 14, { min = 1 })
//! hline("Overbought", 70)
//!
//! function on_bar(bar)
//!   plot(ta.rsi(bar.close, len), "RSI", { color = "#7E57C2" })
//! end
//! ```

use crate::host::TaOut;
use crate::host::{BAR_FLAGS, BAR_NUMS, COLORS, LOADTIME, RUNTIME, ScriptError, SymInfo, TA, Val};
use crate::inproc::{Lang, Phase, Shared, Vm, dispatch, line_in, ta_multi, with_run};
use mlua::{Function, Lua, MultiValue, Table, Value, VmState};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct Luau;

/// Per-bar time budget, and for the top level.
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

pub(crate) fn to_val(v: &Value, depth: usize) -> Val {
    if depth > 32 {
        return Val::Nil;
    }
    match v {
        Value::Nil => Val::Nil,
        Value::Boolean(b) => Val::Bool(*b),
        Value::Integer(i) => Val::Num(*i as f64),
        Value::Number(n) => Val::Num(*n),
        Value::String(s) => Val::Str(s.to_string_lossy()),
        Value::Table(t) => {
            let n = t.raw_len();
            if n > 0 {
                Val::List(
                    (1..=n)
                        .map(|i| to_val(&t.raw_get::<Value>(i).unwrap_or(Value::Nil), depth + 1))
                        .collect(),
                )
            } else {
                let mut m = Vec::new();
                for (k, v) in t.pairs::<Value, Value>().flatten() {
                    let key = match &k {
                        Value::String(s) => s.to_string_lossy(),
                        Value::Integer(i) => i.to_string(),
                        Value::Number(n) => n.to_string(),
                        _ => continue,
                    };
                    m.push((key, to_val(&v, depth + 1)));
                }
                m.sort_by(|a, b| a.0.cmp(&b.0));
                Val::Map(m)
            }
        }
        _ => Val::Nil,
    }
}

pub(crate) fn from_val(lua: &Lua, v: &Val) -> mlua::Result<Value> {
    Ok(match v {
        Val::Nil => Value::Nil,
        Val::Bool(b) => Value::Boolean(*b),
        Val::Num(n) => Value::Number(*n),
        Val::Str(s) => Value::String(lua.create_string(s)?),
        Val::List(l) => {
            let t = lua.create_table_with_capacity(l.len(), 0)?;
            for (i, x) in l.iter().enumerate() {
                t.raw_set(i + 1, from_val(lua, x)?)?;
            }
            Value::Table(t)
        }
        Val::Map(m) => {
            let t = lua.create_table_with_capacity(0, m.len())?;
            for (k, x) in m {
                t.raw_set(k.as_str(), from_val(lua, x)?)?;
            }
            Value::Table(t)
        }
    })
}

/// The table a dotted name lives in (`ta.sma` → the `ta` table).
fn table_for(lua: &Lua, root: &Table, name: &str) -> mlua::Result<(Table, String)> {
    match name.split_once('.') {
        Some((t, rest)) => {
            let tbl: Table = match root.raw_get::<Value>(t)? {
                Value::Table(x) => x,
                _ => {
                    let x = lua.create_table()?;
                    root.raw_set(t, x.clone())?;
                    x
                }
            };
            Ok((tbl, rest.to_string()))
        }
        None => Ok((root.clone(), name.to_string())),
    }
}

fn host_fn(lua: &Lua, shared: &Shared, name: &str, spread: bool) -> mlua::Result<Function> {
    let shared = shared.clone();
    let name = name.to_string();
    lua.create_function(move |lua, args: MultiValue| {
        let args: Vec<Val> = args.iter().map(|v| to_val(v, 0)).collect();
        let r = dispatch(&shared, &name, &args).map_err(mlua::Error::runtime)?;
        match r {
            // ta.* with several results return them as several values.
            Val::List(l) if spread => l.iter().map(|x| from_val(lua, x)).collect::<mlua::Result<MultiValue>>(),
            v => Ok(MultiValue::from_iter([from_val(lua, &v)?])),
        }
    })
}

fn num_of(v: &Value) -> f64 {
    match v {
        Value::Number(n) => *n,
        Value::Integer(i) => *i as f64,
        Value::Boolean(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        _ => f64::NAN,
    }
}

/// `ta.*`: numbers in, numbers out, no allocation.
fn ta_fn(lua: &Lua, shared: &Shared, idx: usize) -> mlua::Result<Function> {
    let shared = shared.clone();
    let name = format!("ta.{}", TA[idx].0);
    if ta_multi(TA[idx].0) {
        lua.create_function(move |_, a: (Value, Value, Value, Value, Value)| {
            let f = [num_of(&a.0), num_of(&a.1), num_of(&a.2), num_of(&a.3), num_of(&a.4)];
            match with_run(&shared, &name, |h| h.ta_fast(idx, &f)).map_err(mlua::Error::runtime)? {
                TaOut::Many(v, _) => Ok((v[0], v[1], v[2])),
                TaOut::One(x) => Ok((x, f64::NAN, f64::NAN)),
                TaOut::Bool(b) => Ok((if b { 1.0 } else { 0.0 }, f64::NAN, f64::NAN)),
            }
        })
    } else {
        lua.create_function(move |_, a: (Value, Value, Value, Value, Value)| {
            let f = [num_of(&a.0), num_of(&a.1), num_of(&a.2), num_of(&a.3), num_of(&a.4)];
            match with_run(&shared, &name, |h| h.ta_fast(idx, &f)).map_err(mlua::Error::runtime)? {
                TaOut::One(x) => Ok(Value::Number(x)),
                TaOut::Bool(b) => Ok(Value::Boolean(b)),
                TaOut::Many(v, _) => Ok(Value::Number(v[0])),
            }
        })
    }
}

/// `plot(value, title, opts)`: only the colour is read once plots are known.
fn plot_fn(lua: &Lua, shared: &Shared) -> mlua::Result<Function> {
    let shared = shared.clone();
    lua.create_function(move |_, (v, title, opts): (Value, Value, Value)| {
        let mut p = shared.lock().unwrap_or_else(|e| e.into_inner());
        let Phase::Run(h) = &mut *p else {
            return Err(mlua::Error::runtime("plot() goes inside on_bar, called on every bar."));
        };
        if h.discovering() {
            let args = [to_val(&v, 0), to_val(&title, 0), to_val(&opts, 0)];
            return h.call("plot", &args).map(|_| ()).map_err(mlua::Error::runtime);
        }
        let color = match &opts {
            Value::Table(t) => match t.raw_get::<Value>("color")? {
                Value::String(s) => crate::host::parse_color(&s.to_string_lossy()),
                _ => None,
            },
            _ => None,
        };
        h.plot_fast(num_of(&v), color).map_err(mlua::Error::runtime)
    })
}

fn error_of(e: &mlua::Error, file: &str) -> ScriptError {
    let text = e.to_string();
    let line = line_in(&text, file);
    // The innermost message, without the "file:12:" prefix and traceback.
    let mut inner = e;
    while let mlua::Error::CallbackError { cause, .. } = inner {
        inner = cause;
    }
    let msg = match inner {
        mlua::Error::RuntimeError(m) | mlua::Error::SyntaxError { message: m, .. } => m.clone(),
        mlua::Error::MemoryError(_) => "The script used too much memory.".into(),
        other => other.to_string(),
    };
    let first = msg.lines().next().unwrap_or("").to_string();
    let first = match first.find(&format!("{file}:")) {
        Some(i) => {
            let rest = &first[i + file.len() + 1..];
            rest.split_once(": ")
                .map(|(_, m)| m.to_string())
                .unwrap_or(first.clone())
        }
        None => first,
    };
    ScriptError::new(line, first.trim().to_string())
}

impl Lang for Luau {
    fn key(&self) -> &'static str {
        "luau"
    }

    fn create(&self, src: &str, file: &str, shared: Shared, syminfo: &SymInfo) -> Result<Box<dyn Vm>, ScriptError> {
        let lua = Lua::new();
        let setup = |lua: &Lua| -> mlua::Result<()> {
            lua.set_memory_limit(MEMORY)?;
            let g = lua.globals();
            for name in RUNTIME.iter().chain(LOADTIME) {
                if *name == "strategy" || *name == "plot" {
                    continue;
                }
                let f = host_fn(lua, &shared, name, false)?;
                let (t, key) = table_for(lua, &g, name)?;
                t.raw_set(key, f)?;
            }
            // `strategy(...)` declares; `strategy.entry(...)` trades.
            let (strat, _) = table_for(lua, &g, "strategy.entry")?;
            let decl = host_fn(lua, &shared, "strategy", false)?;
            let mt = lua.create_table()?;
            mt.raw_set(
                "__call",
                lua.create_function(move |_, args: MultiValue| {
                    let rest: MultiValue = args.into_iter().skip(1).collect();
                    decl.call::<MultiValue>(rest)
                })?,
            )?;
            mt.raw_set(
                "__index",
                lua.create_function(|_, (_, k): (Value, Value)| -> mlua::Result<Value> {
                    let k = to_val(&k, 0);
                    Err(mlua::Error::runtime(format!(
                        "strategy.{} doesn't exist.",
                        crate::host::show(&k)
                    )))
                })?,
            )?;
            strat.set_metatable(Some(mt))?;
            strat.raw_set("long", "long")?;
            strat.raw_set("short", "short")?;
            let ta = lua.create_table()?;
            for (idx, (n, _)) in TA.iter().enumerate() {
                ta.raw_set(*n, ta_fn(lua, &shared, idx)?)?;
            }
            g.raw_set("plot", plot_fn(lua, &shared)?)?;
            g.raw_set("ta", ta)?;
            // Unknown names in the API tables say so plainly.
            for t in ["ta", "input", "color"] {
                let tbl: Table = g.raw_get(t)?;
                let mt = lua.create_table()?;
                mt.raw_set(
                    "__index",
                    lua.create_function(move |_, (_, k): (Value, Value)| -> mlua::Result<Value> {
                        let k = to_val(&k, 0);
                        Err(mlua::Error::runtime(format!(
                            "{t}.{} doesn't exist.",
                            crate::host::show(&k)
                        )))
                    })?,
                )?;
                tbl.set_metatable(Some(mt))?;
            }
            let (color, _) = table_for(lua, &g, "color.new")?;
            for (n, c) in COLORS {
                color.raw_set(*n, format!("#{c:06X}FF"))?;
            }
            g.raw_set("syminfo", from_val(lua, &syminfo.to_val())?)?;
            g.raw_set("NaN", f64::NAN)?;
            g.raw_set("state", lua.create_table()?)?;
            Ok(())
        };
        setup(&lua).map_err(|e| ScriptError::new(0, format!("Couldn't start Luau: {e}")))?;
        let deadline = Arc::new(AtomicU64::new(u64::MAX));
        let d = deadline.clone();
        let name = file.to_string();
        lua.set_interrupt(move |lua| {
            if now_ns() > d.load(Ordering::Relaxed) {
                // Interrupts carry no position: say where the script was.
                let line = (0..4)
                    .filter_map(|l| lua.inspect_stack(l, |dbg| dbg.current_line()).flatten())
                    .next();
                let at = line.map(|l| format!("{name}:{l}: ")).unwrap_or_default();
                return Err(mlua::Error::runtime(format!(
                    "{at}The script took too long on one bar (over a second). Look for a loop that never ends."
                )));
            }
            Ok(VmState::Continue)
        });
        deadline.store(now_ns() + LOAD_BUDGET.as_nanos() as u64, Ordering::Relaxed);
        lua.load(src)
            .set_name(format!("={file}"))
            .exec()
            .map_err(|e| error_of(&e, file))?;
        if !matches!(lua.globals().raw_get::<Value>("on_bar"), Ok(Value::Function(_))) {
            return Err(ScriptError::new(
                0,
                "Define the function that runs on every bar: function on_bar(bar) … end",
            ));
        }
        // One call per bar: the wrapper fills the reused `bar` table, runs
        // on_bar and says whether `state` has anything in it (crossing into
        // Lua per field costs far more than doing it in Lua).
        let fields: Vec<&str> = BAR_NUMS.iter().chain(BAR_FLAGS.iter()).copied().collect();
        let params: Vec<String> = (0..fields.len()).map(|i| format!("a{i}")).collect();
        let sets: String = fields
            .iter()
            .zip(&params)
            .map(|(f, p)| format!("bar.{f} = {p}; "))
            .collect();
        let wrapper = format!(
            "local bar = {{}}\nreturn function({})\n  {sets}\n  on_bar(bar)\n  return type(state) ~= \"table\" or next(state) ~= nil\nend",
            params.join(", ")
        );
        let step: Function = lua
            .load(wrapper)
            .set_name("=opentern")
            .eval()
            .map_err(|e| error_of(&e, file))?;
        Ok(Box::new(LuauVm {
            lua,
            step,
            deadline,
            file: file.to_string(),
            has_state: false,
        }))
    }
}

struct LuauVm {
    lua: Lua,
    /// Fills `bar`, runs `on_bar`, returns whether `state` is non-empty.
    step: Function,
    deadline: Arc<AtomicU64>,
    file: String,
    has_state: bool,
}

impl Vm for LuauVm {
    fn on_bar(&mut self, nums: &[f64; 11], flags: &[bool; 3]) -> Result<(), ScriptError> {
        self.deadline
            .store(now_ns() + BAR_BUDGET.as_nanos() as u64, Ordering::Relaxed);
        let args = (
            nums[0], nums[1], nums[2], nums[3], nums[4], nums[5], nums[6], nums[7], nums[8], nums[9], nums[10],
            flags[0], flags[1], flags[2],
        );
        self.has_state = self.step.call::<bool>(args).map_err(|e| error_of(&e, &self.file))?;
        Ok(())
    }

    fn get_state(&mut self) -> Result<Val, ScriptError> {
        if !self.has_state {
            return Ok(Val::Map(Vec::new()));
        }
        let v: Value = self
            .lua
            .globals()
            .raw_get("state")
            .map_err(|e| error_of(&e, &self.file))?;
        Ok(to_val(&v, 0))
    }

    fn set_state(&mut self, v: &Val) -> Result<(), ScriptError> {
        let lv = from_val(&self.lua, v).map_err(|e| error_of(&e, &self.file))?;
        let lv = match lv {
            Value::Nil => Value::Table(self.lua.create_table().map_err(|e| error_of(&e, &self.file))?),
            x => x,
        };
        self.lua
            .globals()
            .raw_set("state", lv)
            .map_err(|e| error_of(&e, &self.file))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inproc::{Compiled, compile};
    use opentern_core::{Bar, Timeframe};
    use opentern_series::study::{Instance, sessions};

    fn bars(n: usize) -> Vec<Bar> {
        crate::host::dry_bars(n)
    }

    fn comp(src: &str) -> Result<Compiled, ScriptError> {
        compile(Arc::new(Luau), src, "test.luau", SymInfo::default())
    }

    fn run(c: &Compiled, bars: &[Bar]) -> Instance {
        let m = c.meta();
        let mut inst = Instance::with_factory(m, &m.default_inputs(), c.factory());
        let tf = Timeframe::parse("1h").expect("tf");
        let sess = sessions(bars, tf, &jiff::tz::TimeZone::UTC);
        inst.recompute(bars, &sess, tf);
        inst
    }

    #[test]
    fn an_indicator_matches_the_builtin() {
        let c = comp(
            r##"
indicator("My RSI")
local len = input.int("Length", 14, { min = 1 })
hline("Overbought", 70)
function on_bar(bar)
  plot(ta.rsi(bar.close, len), "RSI", { color = "#7E57C2" })
  local mid, up, lo = ta.bb(bar.close, 20, 2)
  plot(up - lo, "Width")
end
"##,
        )
        .expect("compiles");
        let m = c.meta();
        assert_eq!(m.name, "My RSI");
        assert_eq!(m.plots.len(), 2);
        assert_eq!(m.plots[0].title, "RSI");
        assert_eq!(m.inputs[0].id, "length");
        assert_eq!(m.hlines.len(), 1);
        let b = bars(300);
        let inst = run(&c, &b);
        let rsi = opentern_series::builtins::by_id("rsi").expect("rsi");
        let mut r = Instance::new(rsi, &rsi.default_inputs());
        let tf = Timeframe::parse("1h").expect("tf");
        r.recompute(&b, &sessions(&b, tf, &jiff::tz::TimeZone::UTC), tf);
        for i in 0..b.len() {
            let (x, y) = (inst.out.plots[0][i], r.out.plots[0][i]);
            assert!(x == y || (x.is_nan() && y.is_nan()), "bar {i}: {x} vs {y}");
        }
    }

    #[test]
    fn state_rolls_back_on_the_forming_bar() {
        let c = comp(
            r##"
indicator("Count")
function on_bar(bar)
  state.n = (state.n or 0) + 1
  plot(state.n, "N")
end
"##,
        )
        .expect("compiles");
        let mut b = bars(50);
        let m = c.meta();
        let mut inst = Instance::with_factory(m, &m.default_inputs(), c.factory());
        let tf = Timeframe::parse("1h").expect("tf");
        let sess = vec![false; 60];
        inst.recompute(&b, &sess, tf);
        assert_eq!(inst.out.plots[0][49], 50.0);
        // Ticks on the forming bar don't add up.
        for _ in 0..5 {
            b[49].close += 1.0;
            inst.update(&b, &sess, tf);
        }
        assert_eq!(inst.out.plots[0][49], 50.0);
        b.push(bars(51)[50]);
        inst.update(&b, &sess, tf);
        assert_eq!(inst.out.plots[0][50], 51.0);
        assert_eq!(inst.out.plots[0][49], 50.0);
    }

    #[test]
    fn a_strategy_trades_through_the_broker() {
        let c = comp(
            r##"
strategy("Cross", { initial_capital = 10000 })
function on_bar(bar)
  local f = ta.sma(bar.close, 5)
  local s = ta.sma(bar.close, 20)
  local up = ta.crossover(f, s)
  local down = ta.crossunder(f, s)
  if up then strategy.entry("L", strategy.long) end
  if down then strategy.close("L") end
end
"##,
        )
        .expect("compiles");
        assert!(c.is_strategy());
        assert_eq!(c.strategy_props().expect("props")["initial_capital"], 10000.0);
        let inst = run(&c, &bars(400));
        let st = inst
            .latest_state()
            .and_then(|a| a.downcast_ref::<crate::ScriptStudy>())
            .expect("state");
        let b = st.broker().expect("broker");
        assert!(!b.trades.is_empty());
        assert_eq!(b.props.initial_capital, 10000.0);
    }

    #[test]
    fn errors_are_plain_and_have_lines() {
        let e = comp("indicator(\"X\")\nfunction on_bar(bar)\n  plot(ta.nope(1), \"A\")\nend\n").unwrap_err();
        assert_eq!(e.line, 3, "{e:?}");
        assert!(e.message.contains("ta.nope doesn't exist"), "{e:?}");
        let e = comp("indicator(\"X\")\nlocal x = = 1\n").unwrap_err();
        assert_eq!(e.line, 2, "{e:?}");
        let e = comp("function on_bar(bar) end\n").unwrap_err();
        assert!(e.message.contains("indicator("), "{e:?}");
        // A loop that never ends stops with a message instead of hanging.
        let c = comp("indicator(\"X\")\nfunction on_bar(bar)\n  if bar.index == 70 then while true do end end\nend\n")
            .expect("compiles");
        let t = Instant::now();
        let inst = run(&c, &bars(100));
        assert!(t.elapsed() < Duration::from_secs(5));
        let st = inst
            .state()
            .and_then(|a| a.downcast_ref::<crate::ScriptStudy>())
            .expect("state");
        let e = st.error().expect("error");
        assert!(e.message.contains("too long"), "{e:?}");
        assert_eq!(e.line, 3);
    }
}

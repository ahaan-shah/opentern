//! The shared host for in-process script languages (Luau, JavaScript and
//! TypeScript). A language only converts values and calls functions by
//! name; everything with meaning lives here, so the languages behave
//! identically, and the same as Pine where they overlap:
//!
//! - **Bar by bar.** The script's top level runs once (declarations:
//!   `indicator`, `strategy`, `input.*`, `hline`); then `on_bar(bar)` runs
//!   on every bar.
//! - **`ta.*` keep their state per call, by call order** (the k-th `ta`
//!   call on a bar continues the k-th call's state from the bar before),
//!   so they must run on every bar, as in Pine.
//! - **`plot(value, title, opts)`** works the same way: the k-th plot call
//!   on a bar feeds the k-th plot. Plots are found by a dry run on made-up
//!   bars when the script compiles.
//! - **State that carries across bars goes in the `state` table.** The
//!   host snapshots it after every closed bar, so the forming bar's changes
//!   are thrown away on the next tick (Pine's rollback). Other globals
//!   aren't rolled back.
//! - **Strategies** drive the same broker emulator as Pine strategies.

use opentern_core::{Bar, Timeframe};
use opentern_engine::{Broker, OcaType, Order, OrderKind, Props};
use opentern_series::Rgba;
use opentern_series::study::{
    Category, Format, HLineSpec, InputKind, InputSpec, Inputs, LineStyle, Meta, Offset, Placement, PlotSpec, PlotStyle,
    Value,
};
use opentern_series::ta::{self, Hist};
use std::sync::atomic::{AtomicU64, Ordering};

const NA: f64 = f64::NAN;

/// Bars of history `close(n)` and friends can reach (Pine's default
/// `max_bars_back`).
pub const MAX_BARS_BACK: usize = 5000;

/// A script value crossing the language boundary.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Val {
    #[default]
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<Val>),
    Map(Vec<(String, Val)>),
}

impl Val {
    pub fn num(&self) -> f64 {
        match self {
            Val::Num(v) => *v,
            Val::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            _ => NA,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Val::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            Val::Nil => false,
            Val::Bool(b) => *b,
            Val::Num(v) => !v.is_nan() && *v != 0.0,
            _ => true,
        }
    }

    /// A field of a table/object (options arguments).
    pub fn get(&self, key: &str) -> &Val {
        static NIL: Val = Val::Nil;
        match self {
            Val::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v).unwrap_or(&NIL),
            _ => &NIL,
        }
    }

    fn opt_num(&self, key: &str) -> Option<f64> {
        let v = self.get(key).num();
        (!v.is_nan()).then_some(v)
    }

    fn opt_str(&self, key: &str) -> Option<&str> {
        self.get(key).str()
    }
}

fn arg(a: &[Val], i: usize) -> &Val {
    static NIL: Val = Val::Nil;
    a.get(i).unwrap_or(&NIL)
}

/// An error with the 1-based source line (0 when unknown).
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptError {
    pub line: usize,
    pub message: String,
}

impl ScriptError {
    pub fn new(line: usize, message: impl Into<String>) -> ScriptError {
        ScriptError {
            line,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line > 0 {
            write!(f, "Line {}: {}", self.line, self.message)
        } else {
            f.write_str(&self.message)
        }
    }
}

/// What `syminfo.*` says.
#[derive(Clone, Debug)]
pub struct SymInfo {
    pub tickerid: String,
    pub ticker: String,
    pub description: String,
    pub mintick: f64,
    pub currency: String,
    pub kind: String,
    pub timezone: String,
}

impl Default for SymInfo {
    fn default() -> Self {
        SymInfo {
            tickerid: "DEMO:DEMO".into(),
            ticker: "DEMO".into(),
            description: String::new(),
            mintick: 0.01,
            currency: "USD".into(),
            kind: "crypto".into(),
            timezone: "UTC".into(),
        }
    }
}

impl SymInfo {
    pub fn to_val(&self) -> Val {
        Val::Map(vec![
            ("tickerid".into(), Val::Str(self.tickerid.clone())),
            ("ticker".into(), Val::Str(self.ticker.clone())),
            ("description".into(), Val::Str(self.description.clone())),
            ("mintick".into(), Val::Num(self.mintick)),
            ("currency".into(), Val::Str(self.currency.clone())),
            ("type".into(), Val::Str(self.kind.clone())),
            ("timezone".into(), Val::Str(self.timezone.clone())),
        ])
    }
}

// ------------------------------------------------------------- colours

/// TradingView's `color.*` palette.
pub const COLORS: &[(&str, u32)] = &[
    ("aqua", 0x00BCD4),
    ("black", 0x363A45),
    ("blue", 0x2196F3),
    ("fuchsia", 0xE040FB),
    ("gray", 0x787B86),
    ("green", 0x4CAF50),
    ("lime", 0x00E676),
    ("maroon", 0x880E4F),
    ("navy", 0x311B92),
    ("olive", 0x808000),
    ("orange", 0xFF9800),
    ("purple", 0x9C27B0),
    ("red", 0xFF5252),
    ("silver", 0xB2B5BE),
    ("teal", 0x089981),
    ("white", 0xFFFFFF),
    ("yellow", 0xFFEB3B),
];

/// "#RRGGBB", "#RRGGBBAA" or a palette name.
pub fn parse_color(s: &str) -> Option<Rgba> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('#') {
        return match h.len() {
            6 => u32::from_str_radix(h, 16).ok().map(Rgba::hex),
            8 => u32::from_str_radix(h, 16).ok().map(Rgba),
            _ => None,
        };
    }
    COLORS.iter().find(|(n, _)| *n == s).map(|(_, c)| Rgba::hex(*c))
}

fn color_text(c: Rgba) -> String {
    format!("#{:08X}", c.0)
}

/// `color.new(c, transp)`, `color.rgb(r, g, b, transp)` (transparency 0–100).
pub fn color_fn(name: &str, a: &[Val]) -> Result<Val, String> {
    let alpha =
        |t: f64| (((100.0 - if t.is_nan() { 0.0 } else { t.clamp(0.0, 100.0) }) / 100.0) * 255.0).round() as u32;
    match name {
        "new" => {
            let c = arg(a, 0)
                .str()
                .and_then(parse_color)
                .ok_or("color.new needs a colour like \"#2962FF\" or color.blue.")?;
            Ok(Val::Str(color_text(Rgba((c.0 & 0xFFFF_FF00) | alpha(arg(a, 1).num())))))
        }
        "rgb" => {
            let ch = |i: usize| arg(a, i).num().clamp(0.0, 255.0) as u32;
            Ok(Val::Str(color_text(Rgba(
                (ch(0) << 24) | (ch(1) << 16) | (ch(2) << 8) | alpha(arg(a, 3).num()),
            ))))
        }
        _ => Err(format!("color.{name} doesn't exist.")),
    }
}

// --------------------------------------------------------- declarations

#[derive(Clone, Debug)]
pub enum InKind {
    Int { min: i64, max: i64 },
    Float { min: f64, max: f64, step: f64 },
    Bool,
    Text,
    Choice(Vec<String>),
    Source,
    Color,
}

#[derive(Clone, Debug)]
pub struct InputDecl {
    pub id: String,
    pub title: String,
    pub kind: InKind,
    pub default: Value,
    pub group: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PlotDecl {
    pub title: String,
    pub color: Rgba,
    pub style: PlotStyle,
    pub width: u8,
}

#[derive(Clone, Debug)]
pub struct HLineDecl {
    pub title: String,
    pub price: f64,
    pub color: Rgba,
    pub style: LineStyle,
}

/// What a script declares: its kind and title, inputs, plots and levels.
#[derive(Clone, Debug, Default)]
pub struct Decl {
    pub declared: bool,
    pub strategy: bool,
    pub title: String,
    pub short: String,
    pub overlay: bool,
    pub precision: Option<u8>,
    /// `strategy(...)` options, as for `opentern_engine::Props::from_json`.
    pub props: serde_json::Value,
    pub inputs: Vec<InputDecl>,
    pub plots: Vec<PlotDecl>,
    pub hlines: Vec<HLineDecl>,
}

/// The palette's colours plots get in turn when none is given.
const PLOT_COLORS: [u32; 6] = [0x2962FF, 0xFF6D00, 0x7E57C2, 0x089981, 0xF23645, 0xFFB74D];

fn plot_style(s: &str) -> PlotStyle {
    match s {
        "linebr" => PlotStyle::LineBr,
        "stepline" | "step" => PlotStyle::StepLine,
        "histogram" | "hist" => PlotStyle::Histogram,
        "columns" => PlotStyle::Columns,
        "area" => PlotStyle::Area,
        "circles" => PlotStyle::Circles,
        "cross" => PlotStyle::Cross,
        _ => PlotStyle::Line,
    }
}

fn line_style(s: &str) -> LineStyle {
    match s {
        "solid" => LineStyle::Solid,
        "dotted" => LineStyle::Dotted,
        _ => LineStyle::Dashed,
    }
}

fn slug(title: &str) -> String {
    let s: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() { "input".into() } else { s }
}

fn leak(s: impl Into<String>) -> &'static str {
    Box::leak(s.into().into_boxed_str())
}

impl Decl {
    /// Load-time functions: `indicator`, `strategy`, `input.*`, `hline`.
    /// Inputs return the value from `inputs` (or the default).
    pub fn call(&mut self, name: &str, a: &[Val], inputs: &Inputs) -> Result<Val, String> {
        match name {
            "indicator" | "strategy" => {
                if self.declared {
                    return Err("indicator() or strategy() can only be called once, at the top.".into());
                }
                self.declared = true;
                self.strategy = name == "strategy";
                let opts = arg(a, 1);
                self.title = arg(a, 0).str().unwrap_or("My script").to_string();
                self.short = opts.opt_str("shorttitle").unwrap_or(&self.title).to_string();
                self.overlay =
                    opts.get("overlay").truthy() || (self.strategy && !matches!(opts.get("overlay"), Val::Bool(false)));
                self.precision = opts.opt_num("precision").map(|p| p.clamp(0.0, 12.0) as u8);
                if self.strategy {
                    let mut m = serde_json::Map::new();
                    if let Val::Map(kv) = opts {
                        for (k, v) in kv {
                            let j = match v {
                                Val::Num(x) => serde_json::json!(x),
                                Val::Bool(b) => serde_json::json!(b),
                                Val::Str(s) => serde_json::json!(s),
                                _ => continue,
                            };
                            m.insert(k.clone(), j);
                        }
                    }
                    self.props = serde_json::Value::Object(m);
                }
                Ok(Val::Nil)
            }
            "hline" => {
                let opts = arg(a, 2);
                self.hlines.push(HLineDecl {
                    title: arg(a, 0).str().unwrap_or("Level").to_string(),
                    price: arg(a, 1).num(),
                    color: opts
                        .opt_str("color")
                        .and_then(parse_color)
                        .unwrap_or(Rgba::hex(0x787B86)),
                    style: line_style(opts.opt_str("style").unwrap_or("dashed")),
                });
                Ok(Val::Nil)
            }
            n if n.starts_with("input.") => self.input(&n[6..], a, inputs),
            "plot" => Err("plot() goes inside on_bar, called on every bar.".into()),
            _ => Err(format!(
                "{name}() can't be called at the top of the script; call it inside on_bar."
            )),
        }
    }

    fn input(&mut self, kind: &str, a: &[Val], inputs: &Inputs) -> Result<Val, String> {
        let title = arg(a, 0).str().unwrap_or("Input").to_string();
        let dflt = arg(a, 1);
        let opts = arg(a, 2);
        let (kind, default) = match kind {
            "int" => (
                InKind::Int {
                    min: opts.opt_num("min").map(|v| v as i64).unwrap_or(-1_000_000_000),
                    max: opts.opt_num("max").map(|v| v as i64).unwrap_or(1_000_000_000),
                },
                Value::Int(if dflt.num().is_nan() { 0 } else { dflt.num() as i64 }),
            ),
            "float" => (
                InKind::Float {
                    min: opts.opt_num("min").unwrap_or(-1e15),
                    max: opts.opt_num("max").unwrap_or(1e15),
                    step: opts.opt_num("step").unwrap_or(0.1),
                },
                Value::Float(if dflt.num().is_nan() { 0.0 } else { dflt.num() }),
            ),
            "bool" => (InKind::Bool, Value::Bool(dflt.truthy())),
            "string" => {
                let options: Vec<String> = match opts.get("options") {
                    Val::List(l) => l.iter().filter_map(|v| v.str().map(String::from)).collect(),
                    _ => Vec::new(),
                };
                let d = dflt.str().unwrap_or("").to_string();
                if options.is_empty() {
                    (InKind::Text, Value::Text(d))
                } else {
                    let d = if options.contains(&d) { d } else { options[0].clone() };
                    (InKind::Choice(options), Value::Text(d))
                }
            }
            "source" => (InKind::Source, Value::Text(dflt.str().unwrap_or("close").to_string())),
            "color" => (
                InKind::Color,
                Value::Text(color_text(
                    dflt.str().and_then(parse_color).unwrap_or(Rgba::hex(0x2962FF)),
                )),
            ),
            k => {
                return Err(format!(
                    "input.{k} doesn't exist; use input.int, float, bool, string, source or color."
                ));
            }
        };
        let mut id = slug(&title);
        let base = id.clone();
        let mut k = 2;
        while self.inputs.iter().any(|i| i.id == id) {
            id = format!("{base}_{k}");
            k += 1;
        }
        let value = inputs.values.get(&id).cloned().unwrap_or_else(|| default.clone());
        self.inputs.push(InputDecl {
            id,
            title,
            kind,
            default,
            group: opts.opt_str("group").map(String::from),
        });
        Ok(match value {
            Value::Int(v) => Val::Num(v as f64),
            Value::Float(v) => Val::Num(v),
            Value::Bool(b) => Val::Bool(b),
            Value::Text(s) => Val::Str(s),
        })
    }

    /// The study declaration the chart uses. Leaked once per compile, like
    /// a Pine script's.
    pub fn meta(&self, id: &str) -> &'static Meta {
        let inputs = self
            .inputs
            .iter()
            .map(|i| InputSpec {
                id: leak(i.id.clone()),
                title: leak(i.title.clone()),
                kind: match &i.kind {
                    InKind::Int { min, max } => InputKind::Int { min: *min, max: *max },
                    InKind::Float { min, max, step } => InputKind::Float {
                        min: *min,
                        max: *max,
                        step: *step,
                    },
                    InKind::Bool => InputKind::Bool,
                    InKind::Text => InputKind::Text,
                    InKind::Choice(o) => InputKind::Choice(Box::leak(
                        o.iter().map(|s| leak(s.clone())).collect::<Vec<_>>().into_boxed_slice(),
                    )),
                    InKind::Source => InputKind::Source,
                    InKind::Color => InputKind::Color,
                },
                default: i.default.clone(),
                group: i.group.as_ref().map(|g| leak(g.clone())),
            })
            .collect();
        let plots = self
            .plots
            .iter()
            .map(|p| PlotSpec {
                title: leak(p.title.clone()),
                style: p.style,
                colors: vec![("Color", p.color)],
                width: p.width,
                offset: Offset::Fixed(0),
                visible: true,
                base: 0.0,
                shape: None,
            })
            .collect();
        let hlines = self
            .hlines
            .iter()
            .map(|h| HLineSpec {
                title: leak(h.title.clone()),
                value: h.price,
                color: h.color,
                style: h.style,
                width: 1,
                visible: true,
            })
            .collect();
        Box::leak(Box::new(Meta {
            id: leak(id),
            name: leak(self.title.clone()),
            short: leak(self.short.clone()),
            category: Category::Oscillators,
            placement: if self.overlay {
                Placement::Overlay
            } else {
                Placement::Pane
            },
            format: match self.precision {
                Some(p) => Format::Fixed(p),
                None => Format::Price,
            },
            inputs,
            plots,
            hlines,
            fills: vec![],
            build: |_| Box::new(opentern_series::study::NullStudy),
        }))
    }
}

// --------------------------------------------------------- run-time state

/// One `ta.*` call's state. (Sizes differ a lot; slots are few, so
/// boxing the big ones isn't worth the indirection.)
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
enum St {
    Sma(ta::Sma),
    Smooth(ta::Smooth),
    Wma(ta::Wma),
    Vwma(ta::Vwma),
    Hma(ta::Hma),
    Alma(ta::Alma),
    Linreg(ta::Linreg),
    Stdev(ta::Stdev),
    Dev(ta::Dev),
    Extreme(ta::Extreme),
    Change(ta::Change),
    Roc(ta::Roc),
    Rsi(ta::Rsi),
    Tr(ta::Tr),
    Atr(ta::Atr),
    Stoch(ta::Stoch),
    Cci(ta::Cci),
    Cum(ta::Cum),
    Mfi(ta::Mfi),
    Dmi(ta::Dmi),
    Sar(ta::Sar),
    Supertrend(ta::Supertrend),
    Vwap(ta::Vwap),
    Cross(ta::Cross),
    Macd(ta::Smooth, ta::Smooth, ta::Smooth),
    Bb(ta::Sma, ta::Stdev),
    Hist(Hist),
    FixNan(ta::FixNan),
}

#[derive(Clone, Debug)]
struct Slot {
    name: &'static str,
    params: [f64; 3],
    st: St,
}

/// The `ta.*` functions, with how many series arguments come before the
/// parameters (which, when they change, rebuild the state).
pub const TA: &[(&str, usize)] = &[
    ("sma", 1),
    ("ema", 1),
    ("rma", 1),
    ("wma", 1),
    ("vwma", 1),
    ("hma", 1),
    ("alma", 1),
    ("linreg", 1),
    ("stdev", 1),
    ("dev", 1),
    ("highest", 1),
    ("lowest", 1),
    ("change", 1),
    ("mom", 1),
    ("roc", 1),
    ("rsi", 1),
    ("tr", 0),
    ("atr", 0),
    ("stoch", 3),
    ("cci", 1),
    ("cum", 1),
    ("mfi", 1),
    ("dmi", 0),
    ("sar", 0),
    ("supertrend", 0),
    ("vwap", 1),
    ("crossover", 2),
    ("crossunder", 2),
    ("cross", 2),
    ("macd", 1),
    ("bb", 1),
    ("prev", 1),
    ("fixnan", 1),
];

fn len_of(name: &str, v: f64) -> Result<usize, String> {
    if v.is_finite() && v >= 1.0 {
        Ok(v as usize)
    } else {
        Err(format!("ta.{name} needs a length of at least 1."))
    }
}

fn build(name: &'static str, p: [f64; 3]) -> Result<St, String> {
    let n = || len_of(name, p[0]);
    let or = |v: f64, d: f64| if v.is_nan() { d } else { v };
    Ok(match name {
        "sma" => St::Sma(ta::Sma::new(n()?)),
        "ema" => St::Smooth(ta::ema(n()?)),
        "rma" => St::Smooth(ta::rma(n()?)),
        "wma" => St::Wma(ta::Wma::new(n()?)),
        "vwma" => St::Vwma(ta::Vwma::new(n()?)),
        "hma" => St::Hma(ta::Hma::new(n()?)),
        "alma" => St::Alma(ta::Alma::new(n()?, or(p[1], 0.85), or(p[2], 6.0), false)),
        "linreg" => St::Linreg(ta::Linreg::new(n()?, or(p[1], 0.0) as i64)),
        "stdev" => St::Stdev(ta::Stdev::new(n()?)),
        "dev" => St::Dev(ta::Dev::new(n()?)),
        "highest" => St::Extreme(ta::Extreme::highest(n()?)),
        "lowest" => St::Extreme(ta::Extreme::lowest(n()?)),
        "change" | "mom" => St::Change(ta::Change::new(len_of(name, or(p[0], 1.0))?)),
        "roc" => St::Roc(ta::Roc::new(n()?)),
        "rsi" => St::Rsi(ta::Rsi::new(n()?)),
        "tr" => St::Tr(ta::Tr::new(p[0] == 1.0)),
        "atr" => St::Atr(ta::Atr::new(n()?)),
        "stoch" => St::Stoch(ta::Stoch::new(n()?)),
        "cci" => St::Cci(ta::Cci::new(n()?)),
        "cum" => St::Cum(ta::Cum::default()),
        "mfi" => St::Mfi(ta::Mfi::new(n()?)),
        "dmi" => St::Dmi(ta::Dmi::new(n()?, len_of(name, p[1])?)),
        "sar" => St::Sar(ta::Sar::new(or(p[0], 0.02), or(p[1], 0.02), or(p[2], 0.2))),
        "supertrend" => St::Supertrend(ta::Supertrend::new(or(p[0], 3.0), len_of(name, or(p[1], 10.0))?)),
        "vwap" => St::Vwap(ta::Vwap::default()),
        "crossover" | "crossunder" | "cross" => St::Cross(ta::Cross::default()),
        "macd" => St::Macd(
            ta::ema(len_of(name, or(p[0], 12.0))?),
            ta::ema(len_of(name, or(p[1], 26.0))?),
            ta::ema(len_of(name, or(p[2], 9.0))?),
        ),
        "bb" => St::Bb(ta::Sma::new(n()?), ta::Stdev::new(n()?)),
        "prev" => St::Hist(Hist::new(len_of(name, or(p[0], 1.0) + 1.0)?)),
        "fixnan" => St::FixNan(ta::FixNan::default()),
        _ => return Err(format!("ta.{name} doesn't exist.")),
    })
}

/// A `ta.*` result: one number, a few (MACD, Bollinger, DMI), or a flag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TaOut {
    One(f64),
    Many([f64; 3], usize),
    Bool(bool),
}

impl TaOut {
    fn many(v: &[f64]) -> TaOut {
        let mut a = [NA; 3];
        a[..v.len()].copy_from_slice(v);
        TaOut::Many(a, v.len())
    }

    pub fn into_val(self) -> Val {
        match self {
            TaOut::One(v) => Val::Num(v),
            TaOut::Many(a, n) => Val::List(a[..n].iter().map(|x| Val::Num(*x)).collect()),
            TaOut::Bool(b) => Val::Bool(b),
        }
    }
}

/// An alert from `alert()`.
#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    pub bar: usize,
    pub time: i64,
    pub message: String,
}

/// Everything a script's run carries from bar to bar. Plain data, so the
/// forming bar runs on a copy (and the copy is thrown away).
#[derive(Clone, Debug)]
pub struct HostState {
    bars: std::collections::VecDeque<Bar>,
    slots: Vec<Slot>,
    call: usize,
    plot_call: usize,
    pub index: usize,
    pub forming: bool,
    pub count: usize,
    pub new_session: bool,
    pub tf: Option<Timeframe>,
    /// This bar's plot values and per-bar colours (0 = the declared one).
    pub values: Vec<f64>,
    pub raw: Vec<u32>,
    pub bg: u32,
    pub barcolor: u32,
    pub broker: Option<Broker>,
    pub alerts: Vec<Alert>,
    pub logs: Vec<String>,
    /// The `state` table after the last closed bar.
    pub state: Val,
    /// Which snapshot `state` is (so a VM already holding it needn't
    /// reload it).
    pub state_ver: u64,
    /// Plots seen so far (in the dry run: being discovered).
    pub plots: Vec<PlotDecl>,
    pub discovering: bool,
    pub syminfo: SymInfo,
}

static STATE_VER: AtomicU64 = AtomicU64::new(1);

/// A fresh snapshot number.
pub fn next_ver() -> u64 {
    STATE_VER.fetch_add(1, Ordering::Relaxed)
}

impl HostState {
    pub fn new(decl: &Decl, props: Option<Props>, syminfo: SymInfo) -> HostState {
        let n = decl.plots.len();
        HostState {
            bars: std::collections::VecDeque::new(),
            slots: Vec::new(),
            call: 0,
            plot_call: 0,
            index: 0,
            forming: false,
            count: 0,
            new_session: false,
            tf: None,
            values: vec![NA; n],
            raw: vec![0; n],
            bg: 0,
            barcolor: 0,
            broker: props.map(Broker::new),
            alerts: Vec::new(),
            logs: Vec::new(),
            state: Val::Map(Vec::new()),
            state_ver: 0,
            plots: decl.plots.clone(),
            discovering: false,
            syminfo,
        }
    }

    fn bar(&self) -> Bar {
        self.bars.back().copied().unwrap_or(Bar {
            time: 0,
            open: NA,
            high: NA,
            low: NA,
            close: NA,
            volume: NA,
        })
    }

    /// Before `on_bar`: the new bar goes into history; orders from the
    /// last bar fill at its open.
    pub fn begin(&mut self, bar: &Bar, index: usize, forming: bool, count: usize, new_session: bool, tf: Timeframe) {
        if self.bars.len() == MAX_BARS_BACK {
            self.bars.pop_front();
        }
        self.bars.push_back(*bar);
        self.index = index;
        self.forming = forming;
        self.count = count;
        self.new_session = new_session;
        self.tf = Some(tf);
        self.call = 0;
        self.plot_call = 0;
        self.values.iter_mut().for_each(|v| *v = NA);
        self.raw.iter_mut().for_each(|v| *v = 0);
        self.bg = 0;
        self.barcolor = 0;
        if let Some(b) = &mut self.broker {
            b.open_bar(index, bar);
        }
    }

    /// After `on_bar`.
    pub fn end(&mut self, bar: &Bar) {
        if let Some(b) = &mut self.broker {
            b.close_bar(bar);
        }
    }

    /// The `bar` argument's numbers and flags, in [`BAR_NUMS`] and
    /// [`BAR_FLAGS`] order.
    pub fn bar_fields(&self) -> ([f64; 11], [bool; 3]) {
        let b = self.bar();
        (
            [
                b.time as f64,
                b.open,
                b.high,
                b.low,
                b.close,
                b.volume,
                (b.high + b.low) / 2.0,
                (b.high + b.low + b.close) / 3.0,
                (b.open + b.high + b.low + b.close) / 4.0,
                (b.high + b.low + 2.0 * b.close) / 4.0,
                self.index as f64,
            ],
            [self.index + 1 == self.count, self.forming, self.new_session],
        )
    }

    /// The `bar` argument of `on_bar`, as a value (tests).
    pub fn bar_val(&self) -> Val {
        let b = self.bar();
        Val::Map(vec![
            ("time".into(), Val::Num(b.time as f64)),
            ("open".into(), Val::Num(b.open)),
            ("high".into(), Val::Num(b.high)),
            ("low".into(), Val::Num(b.low)),
            ("close".into(), Val::Num(b.close)),
            ("volume".into(), Val::Num(b.volume)),
            ("hl2".into(), Val::Num((b.high + b.low) / 2.0)),
            ("hlc3".into(), Val::Num((b.high + b.low + b.close) / 3.0)),
            ("ohlc4".into(), Val::Num((b.open + b.high + b.low + b.close) / 4.0)),
            ("hlcc4".into(), Val::Num((b.high + b.low + 2.0 * b.close) / 4.0)),
            ("index".into(), Val::Num(self.index as f64)),
            ("is_last".into(), Val::Bool(self.index + 1 == self.count)),
            ("is_forming".into(), Val::Bool(self.forming)),
            ("is_new_session".into(), Val::Bool(self.new_session)),
        ])
    }

    /// Run-time functions, by name.
    pub fn call(&mut self, name: &str, a: &[Val]) -> Result<Val, String> {
        if let Some(t) = name.strip_prefix("ta.") {
            return self.ta(t, a);
        }
        if let Some(s) = name.strip_prefix("strategy.") {
            return self.strategy(s, a);
        }
        if let Some(c) = name.strip_prefix("color.") {
            return color_fn(c, a);
        }
        match name {
            "plot" => self.plot(a),
            "bgcolor" => {
                self.bg = arg(a, 0).str().and_then(parse_color).map(|c| c.0).unwrap_or(0);
                Ok(Val::Nil)
            }
            "barcolor" => {
                self.barcolor = arg(a, 0).str().and_then(parse_color).map(|c| c.0).unwrap_or(0);
                Ok(Val::Nil)
            }
            "alert" => {
                if !self.discovering {
                    let b = self.bar();
                    self.alerts.push(Alert {
                        bar: self.index,
                        time: b.time,
                        message: show(arg(a, 0)),
                    });
                    if self.alerts.len() > 500 {
                        self.alerts.drain(..100);
                    }
                }
                Ok(Val::Nil)
            }
            "log" | "print" => {
                if !self.discovering {
                    let line = a.iter().map(show).collect::<Vec<_>>().join(" ");
                    self.logs.push(line);
                    if self.logs.len() > 200 {
                        self.logs.drain(..50);
                    }
                }
                Ok(Val::Nil)
            }
            "na" => Ok(Val::Bool(matches!(arg(a, 0), Val::Nil) || arg(a, 0).num().is_nan())),
            "nz" => {
                let v = arg(a, 0).num();
                Ok(Val::Num(if v.is_nan() {
                    let r = arg(a, 1).num();
                    if r.is_nan() { 0.0 } else { r }
                } else {
                    v
                }))
            }
            "open" | "high" | "low" | "close" | "volume" | "time" => {
                let n = arg(a, 0).num();
                let n = if n.is_nan() { 0 } else { n.max(0.0) as usize };
                let Some(b) = self.bars.len().checked_sub(n + 1).and_then(|i| self.bars.get(i)) else {
                    return Ok(Val::Num(NA));
                };
                Ok(Val::Num(match name {
                    "open" => b.open,
                    "high" => b.high,
                    "low" => b.low,
                    "close" => b.close,
                    "volume" => b.volume,
                    _ => b.time as f64,
                }))
            }
            "indicator" | "strategy" | "hline" => {
                Err(format!("{name}() goes at the top of the script, not inside on_bar."))
            }
            n if n.starts_with("input.") => Err("input.* goes at the top of the script, not inside on_bar.".into()),
            _ => Err(format!("{name}() doesn't exist.")),
        }
    }

    fn plot(&mut self, a: &[Val]) -> Result<Val, String> {
        let k = self.plot_call;
        self.plot_call += 1;
        let opts = arg(a, 2);
        let color = opts.opt_str("color").and_then(parse_color);
        if self.discovering && k >= self.plots.len() {
            self.plots.push(PlotDecl {
                title: arg(a, 1)
                    .str()
                    .map(String::from)
                    .unwrap_or_else(|| format!("Plot {}", k + 1)),
                color: color.unwrap_or(Rgba::hex(PLOT_COLORS[k % PLOT_COLORS.len()])),
                style: plot_style(opts.opt_str("style").unwrap_or("line")),
                width: opts.opt_num("linewidth").map(|w| w.clamp(1.0, 4.0) as u8).unwrap_or(1),
            });
            self.values.push(NA);
            self.raw.push(0);
        }
        self.plot_call -= 1;
        self.plot_fast(arg(a, 0).num(), color)?;
        Ok(Val::Nil)
    }

    /// Whether `plot` must take the slow path (it's finding the plots).
    pub fn discovering(&self) -> bool {
        self.discovering
    }

    /// `plot(value, …, { color })` once the plots are known.
    pub fn plot_fast(&mut self, v: f64, color: Option<Rgba>) -> Result<(), String> {
        let k = self.plot_call;
        self.plot_call += 1;
        if k >= self.values.len() {
            return Err(
                "plot() was called more times on this bar than on the first bars. Call plot() on every bar, outside if-blocks (plot NaN to show nothing)."
                    .into(),
            );
        }
        self.values[k] = if v.is_infinite() { NA } else { v };
        self.raw[k] = match color {
            Some(c) if c != self.plots[k].color => c.0,
            _ => 0,
        };
        Ok(())
    }

    fn ta(&mut self, name: &str, a: &[Val]) -> Result<Val, String> {
        let Some(idx) = TA.iter().position(|(n, _)| *n == name) else {
            return Err(format!("ta.{name} doesn't exist."));
        };
        let f = [
            arg(a, 0).num(),
            arg(a, 1).num(),
            arg(a, 2).num(),
            arg(a, 3).num(),
            arg(a, 4).num(),
        ];
        Ok(self.ta_fast(idx, &f)?.into_val())
    }

    /// `ta.*` by its index in [`TA`], with numeric arguments (`NaN` for
    /// missing ones): the hot path the languages call directly.
    pub fn ta_fast(&mut self, idx: usize, a: &[f64; 5]) -> Result<TaOut, String> {
        let (key, series) = TA[idx];
        let k = self.call;
        self.call += 1;
        let at = |i: usize| a.get(i).copied().unwrap_or(NA);
        let p = [at(series), at(series + 1), at(series + 2)];
        let same = |x: f64, y: f64| x == y || (x.is_nan() && y.is_nan());
        match self.slots.get(k) {
            Some(s) if s.name != key => {
                return Err(format!(
                    "ta.{key} ran where ta.{} ran on earlier bars. Call ta functions on every bar, in the same order (not inside if-blocks): store the result in a local first.",
                    s.name
                ));
            }
            Some(s) if s.params.iter().zip(&p).all(|(x, y)| same(*x, *y)) => {}
            Some(_) => {
                // A changed length starts that call's state over.
                self.slots[k] = Slot {
                    name: key,
                    params: p,
                    st: build(key, p)?,
                };
            }
            None => self.slots.push(Slot {
                name: key,
                params: p,
                st: build(key, p)?,
            }),
        }
        let b = self.bar();
        let x = a[0];
        let new_session = self.new_session;
        let st = &mut self.slots[k].st;
        Ok(match st {
            St::Sma(s) => TaOut::One(s.update(x)),
            St::Smooth(s) => TaOut::One(s.update(x)),
            St::Wma(s) => TaOut::One(s.update(x)),
            St::Vwma(s) => TaOut::One(s.update(x, b.volume)),
            St::Hma(s) => TaOut::One(s.update(x)),
            St::Alma(s) => TaOut::One(s.update(x)),
            St::Linreg(s) => TaOut::One(s.update(x)),
            St::Stdev(s) => TaOut::One(s.update(x)),
            St::Dev(s) => TaOut::One(s.update(x)),
            St::Extreme(s) => TaOut::One(s.update(x)),
            St::Change(s) => TaOut::One(s.update(x)),
            St::Roc(s) => TaOut::One(s.update(x)),
            St::Rsi(s) => TaOut::One(s.update(x)),
            St::Tr(s) => TaOut::One(s.update(b.high, b.low, b.close)),
            St::Atr(s) => TaOut::One(s.update(b.high, b.low, b.close)),
            St::Stoch(s) => TaOut::One(s.update(x, a[1], a[2])),
            St::Cci(s) => TaOut::One(s.update(x)),
            St::Cum(s) => TaOut::One(s.update(x)),
            St::Mfi(s) => TaOut::One(s.update(x, b.volume)),
            St::Dmi(s) => {
                let (plus, minus, adx) = s.update(b.high, b.low, b.close);
                TaOut::many(&[plus, minus, adx])
            }
            St::Sar(s) => TaOut::One(s.update(b.high, b.low, b.close)),
            St::Supertrend(s) => {
                let (v, dir) = s.update(b.high, b.low, b.close);
                TaOut::many(&[v, dir])
            }
            St::Vwap(s) => TaOut::One(s.update(x, b.volume, new_session).0),
            St::Cross(s) => {
                let (over, under) = s.update(x, a[1]);
                TaOut::Bool(match key {
                    "crossover" => over,
                    "crossunder" => under,
                    _ => over || under,
                })
            }
            St::Macd(f, sl, sig) => {
                let m = f.update(x) - sl.update(x);
                let s = sig.update(m);
                TaOut::many(&[m, s, m - s])
            }
            St::Bb(m, sd) => {
                let mult = if p[1].is_nan() { 2.0 } else { p[1] };
                let mid = m.update(x);
                let dev = sd.update(x) * mult;
                TaOut::many(&[mid, mid + dev, mid - dev])
            }
            St::Hist(h) => {
                h.push(x);
                TaOut::One(h.get(if p[0].is_nan() { 1 } else { p[0] as usize }))
            }
            St::FixNan(s) => TaOut::One(s.update(x)),
        })
    }

    fn broker(&mut self) -> Result<&mut Broker, String> {
        self.broker
            .as_mut()
            .ok_or_else(|| "strategy.* needs strategy(…) instead of indicator(…) at the top.".to_string())
    }

    fn strategy(&mut self, name: &str, a: &[Val]) -> Result<Val, String> {
        let long_of = |v: &Val| match v.str() {
            Some("long") => Ok(true),
            Some("short") => Ok(false),
            _ => match v {
                Val::Bool(b) => Ok(*b),
                _ => Err("The direction must be \"long\" or \"short\".".to_string()),
            },
        };
        let text = |v: &Val, k: &str| v.opt_str(k).unwrap_or("").to_string();
        match name {
            "entry" | "order" => {
                let id = arg(a, 0).str().unwrap_or("").to_string();
                let long = long_of(arg(a, 1))?;
                let opts = arg(a, 2);
                let kind = if name == "entry" {
                    OrderKind::Entry
                } else {
                    OrderKind::Order
                };
                let mut o = Order::market(&id, kind, long);
                o.qty = opts.opt_num("qty").filter(|q| *q > 0.0);
                o.limit = opts.opt_num("limit");
                o.stop = opts.opt_num("stop");
                o.oca_name = text(opts, "oca_name");
                o.oca_type = match opts.opt_str("oca_type") {
                    Some("cancel") => OcaType::Cancel,
                    Some("reduce") => OcaType::Reduce,
                    _ => OcaType::None,
                };
                o.comment = text(opts, "comment");
                if o.comment.is_empty() {
                    o.comment = id;
                }
                self.broker()?.submit(o);
                Ok(Val::Nil)
            }
            "exit" => {
                let id = arg(a, 0).str().unwrap_or("").to_string();
                let opts = arg(a, 2);
                let mut o = Order::market(&id, OrderKind::Exit, true);
                o.from_entry = arg(a, 1).str().unwrap_or("").to_string();
                o.qty = opts.opt_num("qty").filter(|q| *q > 0.0);
                o.qty_percent = opts.opt_num("qty_percent").unwrap_or(100.0).clamp(0.0, 100.0);
                o.profit_ticks = opts.opt_num("profit");
                o.limit = opts.opt_num("limit");
                o.loss_ticks = opts.opt_num("loss");
                o.stop = opts.opt_num("stop");
                o.trail_price = opts.opt_num("trail_price");
                o.trail_points = opts.opt_num("trail_points");
                o.trail_offset = opts.opt_num("trail_offset");
                o.oca_name = text(opts, "oca_name");
                o.comment = text(opts, "comment");
                if o.comment.is_empty() {
                    o.comment = id;
                }
                let b = self.broker()?;
                if let Some(old) = b.pending.iter().find(|p| p.id == o.id && p.kind == OrderKind::Exit) {
                    o.trail_best = old.trail_best;
                }
                let has_level = o.profit_ticks.is_some()
                    || o.limit.is_some()
                    || o.loss_ticks.is_some()
                    || o.stop.is_some()
                    || (o.trail_offset.is_some() && (o.trail_points.is_some() || o.trail_price.is_some()));
                if has_level {
                    b.submit(o);
                }
                Ok(Val::Nil)
            }
            "close" => {
                let id = arg(a, 0).str().unwrap_or("").to_string();
                let opts = arg(a, 1);
                let b = self.broker()?;
                if !b.lots.iter().any(|l| l.id == id) {
                    return Ok(Val::Nil);
                }
                let mut o = Order::market(&id, OrderKind::Close, true);
                o.comment = text(opts, "comment");
                o.qty = opts.opt_num("qty").filter(|q| *q > 0.0);
                o.qty_percent = opts.opt_num("qty_percent").unwrap_or(100.0).clamp(0.0, 100.0);
                o.immediately = opts.get("immediately").truthy();
                b.submit(o);
                Ok(Val::Nil)
            }
            "close_all" => {
                let opts = arg(a, 0);
                let b = self.broker()?;
                if b.lots.is_empty() {
                    return Ok(Val::Nil);
                }
                let mut o = Order::market("", OrderKind::Close, true);
                o.comment = text(opts, "comment");
                o.immediately = opts.get("immediately").truthy();
                b.submit(o);
                Ok(Val::Nil)
            }
            "cancel" => {
                let id = arg(a, 0).str().unwrap_or("").to_string();
                self.broker()?.cancel(&id);
                Ok(Val::Nil)
            }
            "cancel_all" => {
                self.broker()?.cancel_all();
                Ok(Val::Nil)
            }
            "position_size" => Ok(Val::Num(self.broker()?.position_size())),
            "position_avg_price" => {
                let b = self.broker()?;
                Ok(Val::Num(if b.lots.is_empty() { NA } else { b.position_avg_price() }))
            }
            "equity" => {
                let c = self.bar().close;
                Ok(Val::Num(self.broker()?.equity_at(c)))
            }
            "openprofit" => {
                let c = self.bar().close;
                Ok(Val::Num(self.broker()?.open_profit(c)))
            }
            "netprofit" => Ok(Val::Num(self.broker()?.net_profit)),
            "initial_capital" => Ok(Val::Num(self.broker()?.props.initial_capital)),
            "opentrades" => Ok(Val::Num(self.broker()?.lots.len() as f64)),
            "closedtrades" => Ok(Val::Num(self.broker()?.trades.len() as f64)),
            "wintrades" => Ok(Val::Num(self.broker()?.wins() as f64)),
            "losstrades" => Ok(Val::Num(self.broker()?.losses() as f64)),
            _ => Err(format!("strategy.{name} doesn't exist.")),
        }
    }
}

/// How `log()` and `alert()` print a value.
pub fn show(v: &Val) -> String {
    match v {
        Val::Nil => "nil".into(),
        Val::Bool(b) => b.to_string(),
        Val::Num(x) if x.is_nan() => "NaN".into(),
        Val::Num(x) if x.fract() == 0.0 && x.abs() < 1e15 => format!("{x:.0}"),
        Val::Num(x) => {
            let s = format!("{x:.8}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
        Val::Str(s) => s.clone(),
        Val::List(l) => format!("[{}]", l.iter().map(show).collect::<Vec<_>>().join(", ")),
        Val::Map(m) => format!(
            "{{{}}}",
            m.iter()
                .map(|(k, v)| format!("{k}: {}", show(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// The `bar` argument's number fields.
pub const BAR_NUMS: [&str; 11] = [
    "time", "open", "high", "low", "close", "volume", "hl2", "hlc3", "ohlc4", "hlcc4", "index",
];
/// … and its flags.
pub const BAR_FLAGS: [&str; 3] = ["is_last", "is_forming", "is_new_session"];

/// The run-time names every language exposes, grouped as `table.name`.
pub const RUNTIME: &[&str] = &[
    "plot",
    "bgcolor",
    "barcolor",
    "alert",
    "log",
    "na",
    "nz",
    "open",
    "high",
    "low",
    "close",
    "volume",
    "time",
    "strategy.entry",
    "strategy.order",
    "strategy.exit",
    "strategy.close",
    "strategy.close_all",
    "strategy.cancel",
    "strategy.cancel_all",
    "strategy.position_size",
    "strategy.position_avg_price",
    "strategy.equity",
    "strategy.openprofit",
    "strategy.netprofit",
    "strategy.initial_capital",
    "strategy.opentrades",
    "strategy.closedtrades",
    "strategy.wintrades",
    "strategy.losstrades",
    "color.new",
    "color.rgb",
];

/// Load-time names.
pub const LOADTIME: &[&str] = &[
    "indicator",
    "strategy",
    "hline",
    "input.int",
    "input.float",
    "input.bool",
    "input.string",
    "input.source",
    "input.color",
];

/// Made-up bars for the dry run that finds a script's plots: a gentle
/// random walk, deterministic.
pub fn dry_bars(n: usize) -> Vec<Bar> {
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let mut price = 100.0;
    (0..n)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let r = (x % 2000) as f64 / 1000.0 - 1.0;
            let open = price;
            price = (price * (1.0 + r * 0.01)).max(1.0);
            Bar {
                time: 1_700_000_000_000 + i as i64 * 3_600_000,
                open,
                high: open.max(price) * 1.004,
                low: open.min(price) * 0.996,
                close: price,
                volume: 1000.0 + (x % 500) as f64,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> HostState {
        HostState::new(&Decl::default(), Some(Props::default()), SymInfo::default())
    }

    fn step(h: &mut HostState, b: &Bar, i: usize) {
        h.begin(b, i, false, 100, i == 0, Timeframe::parse("1h").expect("tf"));
    }

    #[test]
    fn ta_calls_keep_state_by_order_and_match_the_primitives() {
        let bars = dry_bars(60);
        let mut h = host();
        let mut sma = ta::Sma::new(5);
        let mut rsi = ta::Rsi::new(14);
        for (i, b) in bars.iter().enumerate() {
            step(&mut h, b, i);
            let a = h
                .call("ta.sma", &[Val::Num(b.close), Val::Num(5.0)])
                .expect("sma")
                .num();
            let r = h
                .call("ta.rsi", &[Val::Num(b.close), Val::Num(14.0)])
                .expect("rsi")
                .num();
            let e = sma.update(b.close);
            assert!(a == e || (a.is_nan() && e.is_nan()));
            let e = rsi.update(b.close);
            assert!(r == e || (r.is_nan() && e.is_nan()));
            h.end(b);
        }
        // Swapping the order is a plain error.
        step(&mut h, &bars[0], 60);
        let e = h.call("ta.rsi", &[Val::Num(1.0), Val::Num(14.0)]).unwrap_err();
        assert!(e.contains("same order"), "{e}");
    }

    #[test]
    fn multi_value_ta_and_history() {
        let bars = dry_bars(40);
        let mut h = host();
        for (i, b) in bars.iter().enumerate() {
            step(&mut h, b, i);
            let bb = h
                .call("ta.bb", &[Val::Num(b.close), Val::Num(20.0), Val::Num(2.0)])
                .expect("bb");
            let prev = h
                .call("ta.prev", &[Val::Num(b.close), Val::Num(2.0)])
                .expect("prev")
                .num();
            let c2 = h.call("close", &[Val::Num(2.0)]).expect("close").num();
            assert!(prev == c2 || (prev.is_nan() && c2.is_nan()));
            if i >= 19 {
                let Val::List(v) = bb else { panic!() };
                assert!(v[1].num() > v[0].num() && v[0].num() > v[2].num());
            }
            h.end(b);
        }
    }

    #[test]
    fn colours_parse_and_build() {
        assert_eq!(parse_color("#FF0000"), Some(Rgba(0xFF0000FF)));
        assert_eq!(parse_color("red"), Some(Rgba::hex(0xFF5252)));
        assert_eq!(
            color_fn("new", &[Val::Str("#2962FF".into()), Val::Num(50.0)]).expect("new"),
            Val::Str("#2962FF80".into())
        );
        assert_eq!(
            color_fn("rgb", &[Val::Num(255.0), Val::Num(0.0), Val::Num(0.0)]).expect("rgb"),
            Val::Str("#FF0000FF".into())
        );
    }

    #[test]
    fn inputs_get_ids_and_values() {
        let mut d = Decl::default();
        let mut ins = Inputs::default();
        ins.set("length", Value::Int(21));
        let v = d
            .call("input.int", &[Val::Str("Length".into()), Val::Num(14.0)], &ins)
            .expect("int");
        assert_eq!(v, Val::Num(21.0));
        let v = d
            .call("input.int", &[Val::Str("Length".into()), Val::Num(9.0)], &ins)
            .expect("int");
        assert_eq!(v, Val::Num(9.0));
        assert_eq!(d.inputs[1].id, "length_2");
        let opts = Val::Map(vec![(
            "options".into(),
            Val::List(vec![Val::Str("A".into()), Val::Str("B".into())]),
        )]);
        let v = d
            .call(
                "input.string",
                &[Val::Str("Mode".into()), Val::Str("B".into()), opts],
                &ins,
            )
            .expect("str");
        assert_eq!(v, Val::Str("B".into()));
        assert!(d.call("plot", &[], &ins).is_err());
    }
}

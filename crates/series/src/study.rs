//! The study model shared by every indicator, whatever language it's
//! written in: declared inputs, plots, levels and fills (what TradingView's
//! Inputs and Style tabs show), and a per-bar `step`.
//!
//! The runtime ([`Instance`]) follows Pine's execution model: each closed
//! bar is stepped once into the committed state; the forming bar is
//! stepped on a clone of it, recomputed on every tick and thrown away.

use crate::color::Rgba;
use opentern_core::{Bar, Timeframe};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    Open,
    High,
    Low,
    Close,
    Hl2,
    Hlc3,
    Ohlc4,
    Hlcc4,
}

impl Source {
    pub const ALL: [Source; 8] = [
        Source::Open,
        Source::High,
        Source::Low,
        Source::Close,
        Source::Hl2,
        Source::Hlc3,
        Source::Ohlc4,
        Source::Hlcc4,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Source::Open => "open",
            Source::High => "high",
            Source::Low => "low",
            Source::Close => "close",
            Source::Hl2 => "hl2",
            Source::Hlc3 => "hlc3",
            Source::Ohlc4 => "ohlc4",
            Source::Hlcc4 => "hlcc4",
        }
    }

    pub fn by_name(s: &str) -> Source {
        Source::ALL.into_iter().find(|x| x.name() == s).unwrap_or(Source::Close)
    }

    pub fn of(self, b: &Bar) -> f64 {
        match self {
            Source::Open => b.open,
            Source::High => b.high,
            Source::Low => b.low,
            Source::Close => b.close,
            Source::Hl2 => (b.high + b.low) / 2.0,
            Source::Hlc3 => (b.high + b.low + b.close) / 3.0,
            Source::Ohlc4 => (b.open + b.high + b.low + b.close) / 4.0,
            Source::Hlcc4 => (b.high + b.low + b.close + b.close) / 4.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
}

#[derive(Clone, Debug)]
pub enum InputKind {
    Int {
        min: i64,
        max: i64,
    },
    Float {
        min: f64,
        max: f64,
        step: f64,
    },
    Bool,
    Source,
    Choice(&'static [&'static str]),
    /// Free text (script inputs).
    Text,
    /// A colour, stored as "#RRGGBBAA" text.
    Color,
}

#[derive(Clone, Debug)]
pub struct InputSpec {
    pub id: &'static str,
    pub title: &'static str,
    pub kind: InputKind,
    pub default: Value,
    /// A heading the input is grouped under in the Inputs tab.
    pub group: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlotStyle {
    /// A line that bridges `na` gaps (Pine's `plot.style_line`).
    Line,
    /// A line that breaks at `na` (`plot.style_linebr`).
    LineBr,
    StepLine,
    Histogram,
    Columns,
    Area,
    Circles,
    Cross,
    /// A marker per bar (`plotshape` / `plotchar`); see [`ShapeSpec`].
    Shape,
    /// `plotcandle`: this plot holds the opens; the next three (`Hidden`)
    /// hold highs, lows and closes.
    Candle,
    /// `plotbar`: as `Candle`, drawn as OHLC bars.
    OhlcBar,
    /// A value that isn't drawn by itself (the parts of a candle plot).
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShapeKind {
    TriangleUp,
    TriangleDown,
    ArrowUp,
    ArrowDown,
    Circle,
    Cross,
    XCross,
    Diamond,
    Square,
    LabelUp,
    LabelDown,
    Flag,
    /// A character (`plotchar`), from `ShapeSpec::text`.
    Char,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Location {
    AboveBar,
    BelowBar,
    Top,
    Bottom,
    Absolute,
}

#[derive(Clone, Debug)]
pub struct ShapeSpec {
    pub kind: ShapeKind,
    pub location: Location,
    pub text: &'static str,
    /// Pixels.
    pub size: f32,
}

impl PlotStyle {
    pub const ALL: [PlotStyle; 8] = [
        PlotStyle::Line,
        PlotStyle::LineBr,
        PlotStyle::StepLine,
        PlotStyle::Histogram,
        PlotStyle::Columns,
        PlotStyle::Area,
        PlotStyle::Circles,
        PlotStyle::Cross,
    ];

    pub fn name(self) -> &'static str {
        match self {
            PlotStyle::Line => "Line",
            PlotStyle::LineBr => "Line with breaks",
            PlotStyle::StepLine => "Step line",
            PlotStyle::Histogram => "Histogram",
            PlotStyle::Columns => "Columns",
            PlotStyle::Area => "Area",
            PlotStyle::Circles => "Circles",
            PlotStyle::Cross => "Cross",
            PlotStyle::Shape => "Shapes",
            PlotStyle::Candle => "Candles",
            PlotStyle::OhlcBar => "Bars",
            PlotStyle::Hidden => "Hidden",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineStyle {
    Solid,
    Dashed,
    Dotted,
}

/// How far a plot is shifted, in bars (Ichimoku's spans, the Alligator).
#[derive(Clone, Copy, Debug)]
pub enum Offset {
    Fixed(i64),
    /// `sign * input + add`.
    Input {
        id: &'static str,
        sign: i64,
        add: i64,
    },
}

#[derive(Clone, Debug)]
pub struct PlotSpec {
    pub title: &'static str,
    pub style: PlotStyle,
    /// The plot's colours: the first is the main one; studies pick one per
    /// bar by index (MACD's four histogram colours, for example).
    pub colors: Vec<(&'static str, Rgba)>,
    pub width: u8,
    pub offset: Offset,
    pub visible: bool,
    /// Columns and histograms grow from here.
    pub base: f64,
    /// For `PlotStyle::Shape`.
    pub shape: Option<ShapeSpec>,
}

#[derive(Clone, Debug)]
pub struct HLineSpec {
    pub title: &'static str,
    pub value: f64,
    pub color: Rgba,
    pub style: LineStyle,
    pub width: u8,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum FillBetween {
    Plots(usize, usize),
    HLines(usize, usize),
}

#[derive(Clone, Debug)]
pub struct FillSpec {
    pub title: &'static str,
    pub between: FillBetween,
    pub colors: Vec<(&'static str, Rgba)>,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    MovingAverages,
    Bands,
    Trend,
    Oscillators,
    Volatility,
    Volume,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::MovingAverages,
        Category::Bands,
        Category::Trend,
        Category::Oscillators,
        Category::Volatility,
        Category::Volume,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Category::MovingAverages => "Moving averages",
            Category::Bands => "Bands and channels",
            Category::Trend => "Trend",
            Category::Oscillators => "Oscillators",
            Category::Volatility => "Volatility",
            Category::Volume => "Volume",
        }
    }
}

/// Where a study draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// On the price chart, on its price scale.
    Overlay,
    /// In its own pane below.
    Pane,
    /// On the price chart, with its own scale squeezed into the bottom
    /// fifth (TradingView's volume).
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// The symbol's price precision.
    Price,
    /// A fixed number of decimals.
    Fixed(u8),
    /// 1.2K, 3.4M, as for volume.
    Volume,
}

pub struct Meta {
    pub id: &'static str,
    pub name: &'static str,
    pub short: &'static str,
    pub category: Category,
    pub placement: Placement,
    pub format: Format,
    pub inputs: Vec<InputSpec>,
    pub plots: Vec<PlotSpec>,
    pub hlines: Vec<HLineSpec>,
    pub fills: Vec<FillSpec>,
    pub build: fn(&Inputs) -> Box<dyn Study>,
}

impl Meta {
    pub fn default_inputs(&self) -> Inputs {
        Inputs {
            values: self
                .inputs
                .iter()
                .map(|i| (i.id.to_string(), i.default.clone()))
                .collect(),
        }
    }

    pub fn input(&self, id: &str) -> Option<&InputSpec> {
        self.inputs.iter().find(|i| i.id == id)
    }

    /// The bar shift of plot `i` for these inputs.
    pub fn offset(&self, i: usize, inputs: &Inputs) -> i64 {
        match self.plots[i].offset {
            Offset::Fixed(n) => n,
            Offset::Input { id, sign, add } => sign * inputs.int(id) + add,
        }
    }
}

/// Input values by id. Unknown or missing ids fall back to sensible values,
/// so an old saved template never breaks a study.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Inputs {
    pub values: BTreeMap<String, Value>,
}

impl Inputs {
    pub fn int(&self, id: &str) -> i64 {
        match self.values.get(id) {
            Some(Value::Int(v)) => *v,
            Some(Value::Float(v)) => *v as i64,
            _ => 0,
        }
    }

    pub fn len(&self, id: &str) -> usize {
        self.int(id).max(1) as usize
    }

    pub fn float(&self, id: &str) -> f64 {
        match self.values.get(id) {
            Some(Value::Float(v)) => *v,
            Some(Value::Int(v)) => *v as f64,
            _ => 0.0,
        }
    }

    pub fn bool(&self, id: &str) -> bool {
        matches!(self.values.get(id), Some(Value::Bool(true)))
    }

    pub fn text(&self, id: &str) -> &str {
        match self.values.get(id) {
            Some(Value::Text(s)) => s,
            _ => "",
        }
    }

    pub fn src(&self, id: &str) -> Source {
        Source::by_name(self.text(id))
    }

    pub fn set(&mut self, id: &str, v: Value) {
        self.values.insert(id.to_string(), v);
    }

    /// Keeps only ids the study declares, filling in defaults for missing
    /// ones and coercing values to the declared kind.
    pub fn normalized(&self, meta: &Meta) -> Inputs {
        let mut out = meta.default_inputs();
        for spec in &meta.inputs {
            let Some(v) = self.values.get(spec.id) else { continue };
            let ok = match (&spec.kind, v) {
                (InputKind::Int { min, max }, Value::Int(x)) => Some(Value::Int((*x).clamp(*min, *max))),
                (InputKind::Int { min, max }, Value::Float(x)) => Some(Value::Int((*x as i64).clamp(*min, *max))),
                (InputKind::Float { min, max, .. }, Value::Float(x)) => Some(Value::Float(x.clamp(*min, *max))),
                (InputKind::Float { min, max, .. }, Value::Int(x)) => Some(Value::Float((*x as f64).clamp(*min, *max))),
                (InputKind::Bool, Value::Bool(_)) => Some(v.clone()),
                (InputKind::Source, Value::Text(s)) if Source::ALL.iter().any(|x| x.name() == s) => Some(v.clone()),
                (InputKind::Choice(opts), Value::Text(s)) if opts.contains(&s.as_str()) => Some(v.clone()),
                (InputKind::Text, Value::Text(_)) => Some(v.clone()),
                (InputKind::Color, Value::Text(t)) if t.starts_with('#') => Some(v.clone()),
                _ => None,
            };
            if let Some(v) = ok {
                out.values.insert(spec.id.to_string(), v);
            }
        }
        out
    }

    /// The input values in the legend: "RSI 14 close".
    pub fn summary(&self, meta: &Meta) -> String {
        meta.inputs
            .iter()
            .filter(|i| !matches!(i.kind, InputKind::Color))
            .filter_map(|i| match self.values.get(i.id)? {
                Value::Int(v) => Some(v.to_string()),
                Value::Float(v) => Some(trim_float(*v)),
                Value::Text(s) => Some(s.clone()),
                Value::Bool(_) => None,
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn trim_float(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// What a study sees on each bar.
pub struct Ctx<'a> {
    pub bar: &'a Bar,
    pub index: usize,
    /// The first bar of a new trading day (for session-anchored studies).
    pub new_session: bool,
    pub tf: Timeframe,
    /// The last bar, still forming (stepped on a throwaway clone).
    pub forming: bool,
    /// Bars in the series (for `last_bar_index`).
    pub count: usize,
}

impl Ctx<'_> {
    pub fn src(&self, s: Source) -> f64 {
        s.of(self.bar)
    }
}

/// What a study writes for one bar: a value per plot (`NaN` = nothing) and
/// a colour index per plot and per fill.
pub struct Out {
    pub values: Vec<f64>,
    pub colors: Vec<u8>,
    pub fills: Vec<u8>,
    /// A colour computed by a script (0xRRGGBBAA; 0 = use the palette).
    pub raw: Vec<u32>,
    pub fill_raw: Vec<u32>,
    /// Background colour for this bar (`bgcolor`), 0 for none.
    pub bg: u32,
    /// Candle colour for this bar (`barcolor`), 0 for none.
    pub bar: u32,
}

impl Out {
    fn new(plots: usize, fills: usize) -> Out {
        Out {
            values: vec![f64::NAN; plots],
            colors: vec![0; plots],
            fills: vec![0; fills],
            raw: vec![0; plots],
            fill_raw: vec![0; fills],
            bg: 0,
            bar: 0,
        }
    }

    fn reset(&mut self) {
        self.values.fill(f64::NAN);
        self.colors.fill(0);
        self.fills.fill(0);
        self.raw.fill(0);
        self.fill_raw.fill(0);
        self.bg = 0;
        self.bar = 0;
    }
}

pub trait Study: StudyClone + Send {
    fn step(&mut self, c: &Ctx, o: &mut Out);

    /// For studies with more to say than plots (a strategy's trades).
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }
}

pub trait StudyClone {
    fn clone_box(&self) -> Box<dyn Study>;
}

impl<T: Study + Clone + 'static> StudyClone for T {
    fn clone_box(&self) -> Box<dyn Study> {
        Box::new(self.clone())
    }
}

/// A study's results, one entry per bar.
#[derive(Clone, Debug, Default)]
pub struct Output {
    pub plots: Vec<Vec<f64>>,
    pub colors: Vec<Vec<u8>>,
    pub fills: Vec<Vec<u8>>,
    pub raw: Vec<Vec<u32>>,
    pub fill_raw: Vec<Vec<u32>>,
    pub bg: Vec<u32>,
    pub bar: Vec<u32>,
}

impl Output {
    pub fn resize(&mut self, n: usize) {
        for r in &mut self.raw {
            r.resize(n, 0);
        }
        for r in &mut self.fill_raw {
            r.resize(n, 0);
        }
        self.bg.resize(n, 0);
        self.bar.resize(n, 0);
        for p in &mut self.plots {
            p.resize(n, f64::NAN);
        }
        for c in &mut self.colors {
            c.resize(n, 0);
        }
        for f in &mut self.fills {
            f.resize(n, 0);
        }
    }

    fn write(&mut self, i: usize, o: &Out) {
        // Pine has no infinities: a division by zero is na.
        for (p, v) in self.plots.iter_mut().zip(&o.values) {
            p[i] = if v.is_infinite() { f64::NAN } else { *v };
        }
        for (p, v) in self.colors.iter_mut().zip(&o.colors) {
            p[i] = *v;
        }
        for (p, v) in self.fills.iter_mut().zip(&o.fills) {
            p[i] = *v;
        }
        for (p, v) in self.raw.iter_mut().zip(&o.raw) {
            p[i] = *v;
        }
        for (p, v) in self.fill_raw.iter_mut().zip(&o.fill_raw) {
            p[i] = *v;
        }
        self.bg[i] = o.bg;
        self.bar[i] = o.bar;
    }
}

/// Builds a study for given inputs: for studies that aren't built-ins (a
/// compiled Pine script carries its program in the closure).
pub type Factory = std::sync::Arc<dyn Fn(&Inputs) -> Box<dyn Study> + Send + Sync>;

/// A study that computes nothing itself (its outputs are filled from
/// outside, e.g. by a Python process).
#[derive(Clone)]
pub struct NullStudy;

impl Study for NullStudy {
    fn step(&mut self, _: &Ctx, _: &mut Out) {}
}

/// A study running on a series.
pub struct Instance {
    pub meta: &'static Meta,
    pub inputs: Inputs,
    pub out: Output,
    factory: Option<Factory>,
    /// Outputs come from outside; `update` only keeps their length.
    external: bool,
    committed: Box<dyn Study>,
    /// The state after the forming bar (thrown away on the next update).
    forming: Option<Box<dyn Study>>,
    /// Bars stepped into `committed`.
    done: usize,
    scratch: Out,
}

impl Instance {
    pub fn new(meta: &'static Meta, inputs: &Inputs) -> Instance {
        Instance::make(meta, inputs, None, false)
    }

    /// A study built by `factory` instead of `meta.build`.
    pub fn with_factory(meta: &'static Meta, inputs: &Inputs, factory: Factory) -> Instance {
        Instance::make(meta, inputs, Some(factory), false)
    }

    /// A study whose outputs are written from outside with [`Instance::set_output`].
    pub fn external(meta: &'static Meta, inputs: &Inputs) -> Instance {
        Instance::make(meta, inputs, None, true)
    }

    fn make(meta: &'static Meta, inputs: &Inputs, factory: Option<Factory>, external: bool) -> Instance {
        let inputs = inputs.normalized(meta);
        let committed = match (&factory, external) {
            (_, true) => Box::new(NullStudy) as Box<dyn Study>,
            (Some(f), _) => f(&inputs),
            (None, _) => (meta.build)(&inputs),
        };
        Instance {
            factory,
            external,
            committed,
            forming: None,
            out: Output {
                plots: vec![Vec::new(); meta.plots.len()],
                colors: vec![Vec::new(); meta.plots.len()],
                fills: vec![Vec::new(); meta.fills.len()],
                raw: vec![Vec::new(); meta.plots.len()],
                fill_raw: vec![Vec::new(); meta.fills.len()],
                bg: Vec::new(),
                bar: Vec::new(),
            },
            done: 0,
            scratch: Out::new(meta.plots.len(), meta.fills.len()),
            meta,
            inputs,
        }
    }

    /// Starts over on a new series (new history, changed inputs).
    /// The committed study state (for strategies' trades).
    pub fn state(&self) -> Option<&dyn std::any::Any> {
        self.committed.as_any()
    }

    /// The state including the forming bar (a script's drawings and
    /// errors as they stand now).
    pub fn latest_state(&self) -> Option<&dyn std::any::Any> {
        match &self.forming {
            Some(f) => f.as_any(),
            None => self.committed.as_any(),
        }
    }

    /// The same study (built-in, factory or external) with new inputs.
    pub fn with_inputs(&self, inputs: &Inputs) -> Instance {
        Instance::make(self.meta, inputs, self.factory.clone(), self.external)
    }

    pub fn is_external(&self) -> bool {
        self.external
    }

    pub fn factory(&self) -> Option<Factory> {
        self.factory.clone()
    }

    fn build(&self) -> Box<dyn Study> {
        match (&self.factory, self.external) {
            (_, true) => Box::new(NullStudy),
            (Some(f), _) => f(&self.inputs),
            (None, _) => (self.meta.build)(&self.inputs),
        }
    }

    /// Replaces the outputs (external studies). Shorter outputs are padded
    /// to `n` bars with `na`.
    pub fn set_output(&mut self, mut out: Output, n: usize) {
        out.plots.resize(self.meta.plots.len(), Vec::new());
        out.colors.resize(self.meta.plots.len(), Vec::new());
        out.fills.resize(self.meta.fills.len(), Vec::new());
        out.raw.resize(self.meta.plots.len(), Vec::new());
        out.fill_raw.resize(self.meta.fills.len(), Vec::new());
        self.out = out;
        self.out.resize(n);
    }

    pub fn recompute(&mut self, bars: &[Bar], sessions: &[bool], tf: Timeframe) {
        if self.external {
            self.out.resize(bars.len());
            return;
        }
        self.committed = self.build();
        self.forming = None;
        self.done = 0;
        for v in &mut self.out.plots {
            v.clear();
        }
        for v in &mut self.out.colors {
            v.clear();
        }
        for v in &mut self.out.fills {
            v.clear();
        }
        for v in &mut self.out.raw {
            v.clear();
        }
        for v in &mut self.out.fill_raw {
            v.clear();
        }
        self.out.bg.clear();
        self.out.bar.clear();
        self.update(bars, sessions, tf);
    }

    /// Catches up after the forming bar changed or new bars arrived: every
    /// bar but the last is final and stepped once; the last is computed on
    /// a throwaway clone.
    pub fn update(&mut self, bars: &[Bar], sessions: &[bool], tf: Timeframe) {
        let n = bars.len();
        self.out.resize(n);
        if n == 0 || self.external {
            return;
        }
        let session = |i: usize| sessions.get(i).copied().unwrap_or(false);
        while self.done < n - 1 {
            let i = self.done;
            self.scratch.reset();
            let c = Ctx {
                bar: &bars[i],
                index: i,
                new_session: session(i),
                tf,
                forming: false,
                count: n,
            };
            self.committed.step(&c, &mut self.scratch);
            self.out.write(i, &self.scratch);
            self.done += 1;
        }
        let mut forming = self.committed.clone_box();
        self.scratch.reset();
        let c = Ctx {
            bar: &bars[n - 1],
            index: n - 1,
            new_session: session(n - 1),
            tf,
            forming: true,
            count: n,
        };
        forming.step(&c, &mut self.scratch);
        self.out.write(n - 1, &self.scratch);
        self.forming = Some(forming);
    }
}

/// Marks the first bar of each trading day in `tz` (every bar, for daily
/// and longer intervals).
pub fn sessions(bars: &[Bar], tf: Timeframe, tz: &jiff::tz::TimeZone) -> Vec<bool> {
    if !tf.is_intraday() {
        return vec![true; bars.len()];
    }
    let mut out = Vec::with_capacity(bars.len());
    let mut last = None;
    for b in bars {
        let day = jiff::Timestamp::from_millisecond(b.time)
            .map(|t| t.to_zoned(tz.clone()).date())
            .ok();
        out.push(day != last);
        last = day;
    }
    out
}

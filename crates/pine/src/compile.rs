//! Resolves the syntax tree into an executable tree: names become slot
//! numbers in a frame (the script's root, or one frame per user-function
//! call site, as Pine keeps a separate history per call site), constants
//! are folded, each stateful built-in call (`ta.*`, `math.sum`, …) gets a
//! state slot, and the declarations (`indicator`, `input.*`, `plot`, …)
//! become the study's [`Meta`].

use crate::ast::{self, Arg, Expr, Persist, Stmt};
use crate::natives::{self, NatDef};
use crate::value::{self, V};
use crate::{Env, Error};
use opentern_series::color::Rgba;
use opentern_series::study::{
    Category, FillBetween, FillSpec, Format, HLineSpec, InputKind, InputSpec, LineStyle, Location, Meta, Offset,
    Placement, PlotSpec, PlotStyle, ShapeKind, ShapeSpec, Value,
};
use std::collections::HashMap;
use std::sync::Arc;

/// History kept for a series indexed with a non-constant offset.
pub(crate) const DYN_HIST: usize = 5000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Indicator,
    Strategy,
    Library,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Field {
    Open,
    High,
    Low,
    Close,
    Volume,
    Hl2,
    Hlc3,
    Ohlc4,
    Hlcc4,
    Time,
    TimeClose,
    BarIndex,
}

impl Field {
    pub fn by_name(n: &str) -> Option<Field> {
        Some(match n {
            "open" => Field::Open,
            "high" => Field::High,
            "low" => Field::Low,
            "close" => Field::Close,
            "volume" => Field::Volume,
            "hl2" => Field::Hl2,
            "hlc3" => Field::Hlc3,
            "ohlc4" => Field::Ohlc4,
            "hlcc4" => Field::Hlcc4,
            "time" => Field::Time,
            "time_close" => Field::TimeClose,
            "bar_index" => Field::BarIndex,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bin {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Bin {
    fn of(op: &str) -> Option<Bin> {
        Some(match op {
            "+" | "+=" => Bin::Add,
            "-" | "-=" => Bin::Sub,
            "*" | "*=" => Bin::Mul,
            "/" | "/=" => Bin::Div,
            "%" | "%=" => Bin::Mod,
            "==" => Bin::Eq,
            "!=" => Bin::Ne,
            "<" => Bin::Lt,
            "<=" => Bin::Le,
            ">" => Bin::Gt,
            ">=" => Bin::Ge,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) enum E {
    K(V),
    /// A slot of the current frame, and its history.
    L(u16),
    LH(u16, Box<E>),
    /// A slot of the script's root frame, read from inside a function.
    G(u16),
    GH(u16, Box<E>),
    Bar(Field),
    BarH(Field, Box<E>),
    /// An input's value; `InSrc` is an `input.source` (a bar field).
    In(u16),
    InSrc(u16),
    InSrcH(u16, Box<E>),
    Neg(Box<E>),
    Not(Box<E>),
    Bin(Bin, Box<E>, Box<E>),
    And(Box<E>, Box<E>, bool),
    Or(Box<E>, Box<E>, bool),
    Tern(Box<E>, Box<E>, Box<E>),
    If(Vec<(E, Block)>, Option<Block>),
    Switch(Option<Box<E>>, Vec<(Option<E>, Block)>),
    /// A user function, called at its own call site (`site` indexes the
    /// current frame's child frames).
    Call {
        f: u16,
        site: u16,
        args: Vec<E>,
    },
    /// A built-in; `site` is its state slot (`u16::MAX` = stateless).
    Nat {
        f: &'static NatDef,
        args: Vec<E>,
        site: u16,
    },
    Tuple(Vec<E>),
    /// `expr[n]` for anything that isn't a plain name: the value is kept in
    /// a hidden slot with a history.
    HistOf {
        slot: u16,
        val: Box<E>,
        idx: Box<E>,
    },
    /// A field of a user-defined type (`obj.x`), found by name on the
    /// object's type at run time.
    Get(Box<E>, Arc<str>),
    /// `Type.new(...)`: field values in order.
    New(u16, Vec<E>),
    /// A method call resolved at run time on the receiver's type: the
    /// built-in families (`array.*`, `map.*`, …) or user methods.
    Method {
        name: Arc<str>,
        recv: Box<E>,
        args: Vec<E>,
        /// Argument names (`None` when positional), bound at run time to
        /// the parameters of whichever method the receiver picks.
        names: Vec<Option<Arc<str>>>,
        /// Candidate user methods: (function, type of `this` or
        /// `u16::MAX` for any, call site).
        user: Vec<(u16, u16, u16)>,
    },
    /// `request.security`: an expression evaluated on another timeframe.
    Security(Box<Sec>),
}

// Read by `request.security` (Phase 10).
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct Sec {
    /// The symbol: `None` for this chart's (`syminfo.tickerid`).
    pub symbol: Option<E>,
    pub tf: E,
    pub expr: E,
    pub gaps: E,
    pub lookahead: E,
    /// Index into the current frame's security states.
    pub site: u16,
    /// The frame the expression runs in.
    pub layout: Layout,
    pub bar_hist: usize,
    pub lower: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum S {
    Let(u16, E),
    /// `var`/`varip`: evaluated once, on the first bar it's reached.
    Var(u16, E),
    LetTuple(Vec<Option<u16>>, E),
    Set(u16, Option<Bin>, E),
    /// Assigning a field: `obj.x := v`.
    SetField(E, Arc<str>, Option<Bin>, E),
    Expr(E),
    For {
        var: u16,
        from: E,
        to: E,
        step: Option<E>,
        body: Block,
    },
    ForIn {
        idx: Option<u16>,
        var: u16,
        over: E,
        body: Block,
    },
    While {
        cond: E,
        body: Block,
    },
    Break,
    Continue,
    Plot {
        i: u16,
        val: E,
        color: Option<E>,
        how: PlotHow,
    },
    /// `plotcandle`/`plotbar`: four values in plots `i..i+4`.
    Candle {
        i: u16,
        o: E,
        h: E,
        l: E,
        c: E,
        color: Option<E>,
    },
    FillColor {
        i: u16,
        color: E,
    },
    Bg(E),
    BarColor(E),
    AlertCond {
        i: u16,
        cond: E,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlotHow {
    Value,
    /// `plotshape`/`plotchar` away from the price: shown where truthy.
    Flag,
    /// `plotarrow`: up arrow in plot `i` when > 0, down in `i + 1` when < 0.
    Arrow,
}

/// Statements with their source lines (for runtime errors).
pub(crate) type Block = Vec<(u32, S)>;

/// What a frame needs at run time.
#[derive(Clone, Debug, Default)]
pub(crate) struct Layout {
    pub slots: usize,
    /// History length per slot (0 = none).
    pub hist: Vec<usize>,
    pub states: usize,
    /// The function each call site calls.
    pub calls: Vec<u16>,
    pub secs: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct Func {
    pub name: String,
    pub params: Vec<String>,
    pub defaults: Vec<Option<V>>,
    pub body: Block,
    pub layout: Layout,
}

#[derive(Clone, Debug)]
pub(crate) struct TypeDef {
    pub name: String,
    pub fields: Vec<String>,
    pub defaults: Vec<V>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Conv {
    Int,
    Float,
    Bool,
    Str,
    Source,
    Color,
    IntChoice,
    FloatChoice,
}

#[derive(Clone, Debug)]
pub(crate) struct InputDef {
    pub spec: InputSpec,
    pub conv: Conv,
}

#[derive(Clone, Debug)]
pub(crate) struct PlotDef {
    pub title: String,
    pub style: PlotStyle,
    pub palette: Vec<u32>,
    pub width: u8,
    pub offset: Offset,
    pub visible: bool,
    pub base: f64,
    pub shape: Option<(ShapeKind, Location, String, f32)>,
}

#[derive(Clone, Debug)]
pub(crate) struct FillDef {
    pub title: String,
    pub between: FillBetween,
    pub palette: Vec<u32>,
    pub visible: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct AlertCondDef {
    pub title: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Program {
    pub kind: Kind,
    pub version: u8,
    pub title: String,
    pub short: String,
    pub overlay: bool,
    pub format: Format,
    pub props: serde_json::Value,
    pub inputs: Vec<InputDef>,
    pub plots: Vec<PlotDef>,
    pub hlines: Vec<HLineSpec>,
    pub fills: Vec<FillDef>,
    pub alertconditions: Vec<AlertCondDef>,
    pub funcs: Vec<Func>,
    pub types: Vec<TypeDef>,
    pub root: Block,
    pub layout: Layout,
    pub bar_hist: usize,
    pub env: Arc<Env>,
    pub limits: [usize; 6],
}

#[derive(Clone, Copy, Debug)]
enum Bind {
    Slot { depth: usize, slot: u16 },
    Plot(u16),
    HLine(u16),
}

struct Cx {
    version: u8,
    env: Arc<Env>,
    /// Scopes, innermost last, each with the frame depth it belongs to.
    scopes: Vec<(usize, HashMap<String, Bind>)>,
    frames: Vec<Layout>,
    /// Local-block nesting in the current frame (plots must be global).
    local: usize,
    funcs: Vec<Func>,
    fnames: HashMap<String, u16>,
    /// Methods by name: (function, type of `this`).
    methods: HashMap<String, Vec<(u16, Option<String>)>>,
    types: Vec<TypeDef>,
    /// Known values of slots declared from constants or inputs (for
    /// declarations like `plot(offset = len)`), by (depth, slot).
    known: HashMap<(usize, u16), E>,
    /// Declared types of slots, for method resolution, by (depth, slot).
    slot_types: HashMap<(usize, u16), String>,
    /// Colour constants in a variable's defining expression, so a plot
    /// coloured by that variable gets the palette (`color = zoneColor`).
    palettes: HashMap<(usize, u16), Vec<u32>>,
    line: usize,
    // declarations
    kind: Option<Kind>,
    title: String,
    short: String,
    overlay: bool,
    format: Format,
    props: serde_json::Map<String, serde_json::Value>,
    inputs: Vec<InputDef>,
    input_ids: HashMap<String, usize>,
    plots: Vec<PlotDef>,
    hlines: Vec<HLineSpec>,
    fills: Vec<FillDef>,
    alertconditions: Vec<AlertCondDef>,
    bar_hist: usize,
    limits: [usize; 6],
    /// Bar history needed inside the `request.security` being compiled.
    sec_depth: usize,
    sec_bar_hist: usize,
    /// Plain `x = expr` declarations (never reassigned), so a
    /// `request.security` expression can recompute `x` on its timeframe.
    defs: HashMap<(usize, u16), Expr>,
    /// Frame depths of the `request.security` expressions being compiled.
    sec_frames: Vec<usize>,
    /// Definitions being inlined (to catch cycles).
    inlining: Vec<(usize, u16)>,
    /// Enums: members and their titles (a member's value is its title).
    enums: HashMap<String, Vec<(String, String)>>,
    /// While compiling an imported library: its alias, so its own names
    /// resolve to `alias.name`.
    lib_prefix: Option<String>,
    /// Libraries being imported (to catch cycles).
    importing: Vec<String>,
}

type R<T> = Result<T, Error>;

pub(crate) fn compile(prog: &ast::Block, version: u8, env: Env, _file: &str) -> R<Program> {
    let mut cx = Cx {
        version,
        env: Arc::new(env),
        scopes: vec![(0, HashMap::new())],
        frames: vec![Layout::default()],
        local: 0,
        funcs: Vec::new(),
        fnames: HashMap::new(),
        methods: HashMap::new(),
        types: Vec::new(),
        known: HashMap::new(),
        slot_types: HashMap::new(),
        palettes: HashMap::new(),
        line: 0,
        kind: None,
        title: String::new(),
        short: String::new(),
        overlay: false,
        format: Format::Price,
        props: serde_json::Map::new(),
        inputs: Vec::new(),
        input_ids: HashMap::new(),
        plots: Vec::new(),
        hlines: Vec::new(),
        fills: Vec::new(),
        alertconditions: Vec::new(),
        bar_hist: 1,
        limits: [50, 50, 50, 40, 50, 50],
        sec_depth: 0,
        sec_bar_hist: 1,
        defs: HashMap::new(),
        sec_frames: Vec::new(),
        inlining: Vec::new(),
        enums: HashMap::new(),
        lib_prefix: None,
        importing: Vec::new(),
    };
    let root = cx.block(prog)?;
    let Some(kind) = cx.kind else {
        return Err(Error::at(
            1,
            "Start the script with indicator(\"Name\") or strategy(\"Name\").",
        ));
    };
    let layout = cx.frames.pop().unwrap_or_default();
    Ok(Program {
        kind,
        version,
        title: cx.title,
        short: cx.short,
        overlay: cx.overlay,
        format: cx.format,
        props: serde_json::Value::Object(cx.props),
        inputs: cx.inputs,
        plots: cx.plots,
        hlines: cx.hlines,
        fills: cx.fills,
        alertconditions: cx.alertconditions,
        funcs: cx.funcs,
        types: cx.types,
        root,
        layout,
        bar_hist: cx.bar_hist,
        env: cx.env,
        limits: cx.limits,
    })
}

fn leak(s: impl Into<String>) -> &'static str {
    Box::leak(s.into().into_boxed_str())
}

/// The study declaration for a compiled program.
pub(crate) fn meta(p: &Program, file: &str) -> &'static Meta {
    let pal = |palette: &[u32], what: &str| -> Vec<(&'static str, Rgba)> {
        if palette.is_empty() {
            return vec![(leak(what), Rgba(0))];
        }
        palette
            .iter()
            .enumerate()
            .map(|(k, c)| {
                let name = if palette.len() == 1 {
                    what.to_string()
                } else {
                    format!("{what} {k}")
                };
                (leak(name), Rgba(*c))
            })
            .collect()
    };
    let plots = p
        .plots
        .iter()
        .map(|d| PlotSpec {
            title: leak(d.title.clone()),
            style: d.style,
            colors: pal(&d.palette, "Color"),
            width: d.width,
            offset: d.offset,
            visible: d.visible,
            base: d.base,
            shape: d.shape.as_ref().map(|(kind, location, text, size)| ShapeSpec {
                kind: *kind,
                location: *location,
                text: leak(text.clone()),
                size: *size,
            }),
        })
        .collect();
    let fills = p
        .fills
        .iter()
        .map(|f| FillSpec {
            title: leak(f.title.clone()),
            between: f.between,
            colors: pal(&f.palette, "Color"),
            visible: f.visible,
        })
        .collect();
    Box::leak(Box::new(Meta {
        id: leak(format!("pine:{file}")),
        name: leak(p.title.clone()),
        short: leak(if p.short.is_empty() {
            p.title.clone()
        } else {
            p.short.clone()
        }),
        category: Category::Oscillators,
        placement: if p.overlay { Placement::Overlay } else { Placement::Pane },
        format: p.format,
        inputs: p.inputs.iter().map(|i| i.spec.clone()).collect(),
        plots,
        hlines: p.hlines.clone(),
        fills,
        build: |_| Box::new(opentern_series::study::NullStudy),
    }))
}

// ------------------------------------------------------------ helpers

/// Matches call arguments to parameter names: positional first, then
/// named. Returns one entry per parameter.
fn bind<'a>(fname: &str, params: &[&str], args: &'a [Arg], line: usize) -> R<Vec<Option<&'a Expr>>> {
    let mut out: Vec<Option<&Expr>> = vec![None; params.len()];
    let mut pos = 0;
    for a in args {
        match &a.name {
            None => {
                if pos >= params.len() {
                    return Err(Error::at(line, format!("Too many arguments for {fname}().")));
                }
                out[pos] = Some(&a.value);
                pos += 1;
            }
            Some(n) => match params.iter().position(|p| p == n) {
                Some(k) => out[k] = Some(&a.value),
                // Arguments that only matter on TradingView's servers.
                None if IGNORED_ARGS.contains(&n.as_str()) => {}
                None => return Err(Error::at(line, format!("{fname}() has no argument called \"{n}\"."))),
            },
        }
    }
    Ok(out)
}

pub(crate) const IGNORED_ARGS: &[&str] = &[
    "editable",
    "show_last",
    "tooltip",
    "inline",
    "confirm",
    "display",
    "force_overlay",
    "alert_message",
    "disable_alert",
    "explicit_plot_zorder",
    "behind_chart",
    "dynamic_requests",
    "calc_bars_count",
    "format",
    "precision",
    "active",
];

fn slug(title: &str) -> String {
    let mut s = String::new();
    for c in title.chars() {
        if c.is_alphanumeric() {
            s.extend(c.to_lowercase());
        } else if !s.ends_with('_') && !s.is_empty() {
            s.push('_');
        }
    }
    let s = s.trim_end_matches('_').to_string();
    if s.is_empty() { "input".into() } else { s }
}

/// A constant's value (`color.red`, `plot.style_line`, `math.pi`, …).
pub(crate) fn constant(name: &str) -> Option<V> {
    if let Some(c) = name.strip_prefix("color.")
        && let Some((_, rgb)) = value::COLORS.iter().find(|(n, _)| *n == c)
    {
        return Some(V::Color(value::opaque(*rgb)));
    }
    let v = match name {
        "math.pi" => V::Float(std::f64::consts::PI),
        "math.e" => V::Float(std::f64::consts::E),
        "math.phi" => V::Float(1.618_033_988_749_895),
        "math.rphi" => V::Float(0.618_033_988_749_895),
        "dayofweek.sunday" => V::Int(1),
        "dayofweek.monday" => V::Int(2),
        "dayofweek.tuesday" => V::Int(3),
        "dayofweek.wednesday" => V::Int(4),
        "dayofweek.thursday" => V::Int(5),
        "dayofweek.friday" => V::Int(6),
        "dayofweek.saturday" => V::Int(7),
        "barmerge.gaps_on" | "barmerge.lookahead_on" => V::Bool(true),
        "barmerge.gaps_off" | "barmerge.lookahead_off" => V::Bool(false),
        "strategy.long" => V::str("long"),
        "strategy.short" => V::str("short"),
        "order.ascending" => V::Int(1),
        "order.descending" => V::Int(-1),
        _ => {
            const ENUMS: &[&str] = &[
                "plot.style_",
                "plot.linestyle_",
                "shape.",
                "location.",
                "size.",
                "display.",
                "format.",
                "hline.style_",
                "line.style_",
                "extend.",
                "xloc.",
                "yloc.",
                "label.style_",
                "position.",
                "text.align_",
                "text.wrap_",
                "text.format_",
                "font.family_",
                "currency.",
                "alert.freq_",
                "strategy.commission.",
                "strategy.direction.",
                "strategy.oca.",
                "adjustment.",
                "session.",
                "scale.",
                "settlement_as_close.",
                "backadjustment.",
                "dividends.",
                "earnings.",
                "splits.",
            ];
            const STRATEGY: &[&str] = &["strategy.fixed", "strategy.cash", "strategy.percent_of_equity"];
            if ENUMS.iter().any(|p| name.starts_with(p)) || STRATEGY.contains(&name) {
                // Constants that are functions on the prefix are excluded by
                // the caller (it tries natives first).
                V::str(name)
            } else {
                return None;
            }
        }
    };
    Some(v)
}

/// `array<float>` → `array`; user and primitive types as written.
fn base_type(t: &str) -> String {
    t.split('<').next().unwrap_or(t).to_string()
}

fn const_of(e: &E) -> Option<V> {
    match e {
        E::K(v) => Some(v.clone()),
        _ => None,
    }
}

impl Cx {
    fn err(&self, msg: impl Into<String>) -> Error {
        Error::at(self.line, msg)
    }

    fn depth(&self) -> usize {
        self.frames.len() - 1
    }

    fn frame(&mut self) -> &mut Layout {
        self.frames.last_mut().expect("a frame")
    }

    fn new_slot(&mut self) -> u16 {
        let f = self.frame();
        f.slots += 1;
        f.hist.push(0);
        (f.slots - 1) as u16
    }

    fn new_state(&mut self) -> u16 {
        let f = self.frame();
        f.states += 1;
        (f.states - 1) as u16
    }

    fn need_hist(&mut self, depth: usize, slot: u16, n: usize) {
        let h = &mut self.frames[depth].hist[slot as usize];
        *h = (*h).max(n);
    }

    fn need_bar_hist(&mut self, n: usize) {
        if self.sec_depth > 0 {
            self.sec_bar_hist = self.sec_bar_hist.max(n + 1);
        } else {
            self.bar_hist = self.bar_hist.max(n + 1);
        }
    }

    fn declare(&mut self, name: &str, b: Bind) {
        if name == "_" {
            return;
        }
        if let Some((_, s)) = self.scopes.last_mut() {
            s.insert(name.to_string(), b);
        }
    }

    fn lookup(&self, name: &str) -> Option<Bind> {
        for (_, s) in self.scopes.iter().rev() {
            if let Some(b) = s.get(name) {
                return Some(*b);
            }
        }
        None
    }

    fn push_scope(&mut self) {
        let d = self.depth();
        self.scopes.push((d, HashMap::new()));
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// A slot reference from the current frame.
    fn slot_ref(&self, depth: usize, slot: u16) -> R<E> {
        if depth == self.depth() {
            Ok(E::L(slot))
        } else if depth == 0 {
            Ok(E::G(slot))
        } else {
            Err(self.err("A function can't use another function's variables."))
        }
    }

    // -------------------------------------------------------- statements

    fn block(&mut self, b: &ast::Block) -> R<Block> {
        let mut out = Vec::with_capacity(b.len());
        for s in b {
            if let Some(st) = self.stmt(s)? {
                out.push((self.line as u32, st));
            }
        }
        Ok(out)
    }

    /// A nested block (if/for/while body) with its own scope.
    fn local_block(&mut self, b: &ast::Block) -> R<Block> {
        self.push_scope();
        self.local += 1;
        let r = self.block(b);
        self.local -= 1;
        self.pop_scope();
        r
    }

    fn stmt(&mut self, s: &Stmt) -> R<Option<S>> {
        match s {
            Stmt::Decl {
                persist,
                names,
                ty,
                value,
                line,
            } => {
                self.line = *line;
                self.decl(*persist, names, ty.as_deref(), value).map(Some)
            }
            Stmt::Assign { name, op, value, line } => {
                self.line = *line;
                let e = self.expr(value)?;
                let op = if *op == ":=" { None } else { Bin::of(op) };
                // Field assignment: obj.field := v
                if self.lookup(name).is_none()
                    && let Some((obj, field)) = name.rsplit_once('.')
                {
                    let (oe, fi) = self.field_path(obj, field)?;
                    return Ok(Some(S::SetField(oe, fi, op, e)));
                }
                match self.lookup(name) {
                    Some(Bind::Slot { depth, slot }) => {
                        if depth != self.depth() {
                            return Err(self.err(format!(
                                "\"{name}\" is a global variable; functions can't change it (Pine's rule)."
                            )));
                        }
                        self.known.remove(&(depth, slot));
                        self.defs.remove(&(depth, slot));
                        Ok(Some(S::Set(slot, op, e)))
                    }
                    _ => Err(self.err(format!("\"{name}\" isn't declared yet. Declare it with \"=\" first."))),
                }
            }
            Stmt::Expr(e, line) => {
                self.line = *line;
                if let Expr::Call { callee, args } = e
                    && let Some(st) = self.output_call(callee, args)?
                {
                    return Ok(st);
                }
                Ok(Some(S::Expr(self.expr(e)?)))
            }
            Stmt::For {
                var,
                from,
                to,
                step,
                body,
                line,
            } => {
                self.line = *line;
                let from = self.expr(from)?;
                let to = self.expr(to)?;
                let step = step.as_ref().map(|s| self.expr(s)).transpose()?;
                self.push_scope();
                let slot = self.new_slot();
                let d = self.depth();
                self.declare(var, Bind::Slot { depth: d, slot });
                self.local += 1;
                let body = self.block(body);
                self.local -= 1;
                self.pop_scope();
                Ok(Some(S::For {
                    var: slot,
                    from,
                    to,
                    step,
                    body: body?,
                }))
            }
            Stmt::ForIn {
                idx,
                var,
                over,
                body,
                line,
            } => {
                self.line = *line;
                let over = self.expr(over)?;
                self.push_scope();
                let d = self.depth();
                let idx = idx.as_ref().map(|n| {
                    let s = self.new_slot();
                    self.declare(n, Bind::Slot { depth: d, slot: s });
                    s
                });
                let slot = self.new_slot();
                self.declare(var, Bind::Slot { depth: d, slot });
                self.local += 1;
                let body = self.block(body);
                self.local -= 1;
                self.pop_scope();
                Ok(Some(S::ForIn {
                    idx,
                    var: slot,
                    over,
                    body: body?,
                }))
            }
            Stmt::While { cond, body, line } => {
                self.line = *line;
                let cond = self.expr(cond)?;
                let body = self.local_block(body)?;
                Ok(Some(S::While { cond, body }))
            }
            Stmt::Break => Ok(Some(S::Break)),
            Stmt::Continue => Ok(Some(S::Continue)),
            Stmt::Func {
                name,
                params,
                body,
                line,
                method,
            } => {
                self.line = *line;
                self.func(name, params, body, *method)?;
                Ok(None)
            }
            Stmt::Type { name, fields, line } => {
                self.line = *line;
                let mut defaults = Vec::new();
                for (_, d) in fields {
                    let v = match d {
                        Some(e) => {
                            let e = self.expr(e)?;
                            const_of(&e).ok_or_else(|| self.err("A field's default value must be a constant."))?
                        }
                        None => V::Na,
                    };
                    defaults.push(v);
                }
                self.types.push(TypeDef {
                    name: name.clone(),
                    fields: fields.iter().map(|(n, _)| n.clone()).collect(),
                    defaults,
                });
                Ok(None)
            }
            Stmt::Enum { name, members, line } => {
                self.line = *line;
                let mut seen = std::collections::HashSet::new();
                for (_, t) in members {
                    if !seen.insert(t) {
                        return Err(self.err(format!("Two members of {name} have the title \"{t}\".")));
                    }
                }
                self.enums.insert(name.clone(), members.clone());
                Ok(None)
            }
            Stmt::Import { path, alias, line } => {
                self.line = *line;
                self.import(path, alias)?;
                Ok(None)
            }
        }
    }

    /// `import user/Name/1 as alias`: the library's functions, methods,
    /// types and enums, under `alias.`. Libraries are scripts in the user's
    /// scripts folder, found by their `library("Name")` title.
    fn import(&mut self, path: &str, alias: &str) -> R<()> {
        let name = path.split('/').nth(1).unwrap_or(path).to_string();
        let Some(src) = self
            .env
            .libraries
            .iter()
            .find(|(t, _)| *t == name)
            .map(|(_, s)| s.clone())
        else {
            return Err(self.err(format!(
                "There's no library called \"{name}\" in your scripts. Add a script that starts with library(\"{name}\")."
            )));
        };
        if self.importing.contains(&name) {
            return Err(self.err(format!("The library \"{name}\" imports itself.")));
        }
        let line = self.line;
        let ast = crate::lexer::lines(&src)
            .and_then(|l| crate::parser::Parser::new(l).program())
            .map_err(|e| self.err(format!("In the library \"{name}\": {e}")))?;
        self.importing.push(name.clone());
        let saved = self.lib_prefix.replace(alias.to_string());
        let r = (|| {
            for s in &ast {
                match s {
                    Stmt::Func {
                        name: f,
                        params,
                        body,
                        line,
                        method,
                    } => {
                        self.line = *line;
                        let full = if *method { f.clone() } else { format!("{alias}.{f}") };
                        self.func(&full, params, body, *method)?;
                    }
                    Stmt::Type { name: t, .. } | Stmt::Enum { name: t, .. } => {
                        // Registered under the alias.
                        let renamed = match s.clone() {
                            Stmt::Type { fields, line, .. } => Stmt::Type {
                                name: format!("{alias}.{t}"),
                                fields,
                                line,
                            },
                            Stmt::Enum { members, line, .. } => Stmt::Enum {
                                name: format!("{alias}.{t}"),
                                members,
                                line,
                            },
                            _ => unreachable!(),
                        };
                        self.stmt(&renamed)?;
                    }
                    Stmt::Import { path, alias: a, line } => {
                        self.line = *line;
                        self.import(path, a)?;
                    }
                    // library(…), constants and the rest stay in the library.
                    _ => {}
                }
            }
            Ok(())
        })();
        self.lib_prefix = saved;
        self.importing.pop();
        self.line = line;
        r.map_err(|e: Error| {
            if e.message.starts_with("In the library") {
                e
            } else {
                Error::at(
                    line,
                    format!("In the library \"{name}\" (line {}): {}", e.line, e.message),
                )
            }
        })
    }

    /// `name` as the library being compiled knows it (`alias.name`), when
    /// `exists` finds that; else `name`.
    fn in_lib(&self, name: &str, exists: impl Fn(&Self, &str) -> bool) -> String {
        if let Some(p) = &self.lib_prefix {
            let full = format!("{p}.{name}");
            if exists(self, &full) {
                return full;
            }
        }
        name.to_string()
    }

    fn type_id(&self, t: &str) -> Option<usize> {
        let t = self.in_lib(t, |cx, n| cx.types.iter().any(|x| x.name == n));
        self.types.iter().position(|x| x.name == t)
    }

    fn decl(&mut self, persist: Persist, names: &[String], declared: Option<&str>, value: &Expr) -> R<S> {
        // Outputs that return a handle: p = plot(...), h = hline(...).
        if names.len() == 1
            && let Expr::Call { callee, args } = value
            && matches!(callee.as_str(), "plot" | "hline")
        {
            let st = self.output_call(callee, args)?;
            let b = if callee == "plot" {
                Bind::Plot((self.plots.len() - 1) as u16)
            } else {
                Bind::HLine((self.hlines.len() - 1) as u16)
            };
            self.declare(&names[0], b);
            // plot() is a statement; hline() has no per-bar part.
            return Ok(st.flatten().unwrap_or(S::Expr(E::K(V::Na))));
        }
        let e = self.expr(value)?;
        let d = self.depth();
        if names.len() == 1 {
            let ty = declared.map(base_type).or_else(|| self.type_of_expr(value));
            let slot = self.new_slot();
            let colour_input = matches!(&e, E::In(i) if self.inputs[*i as usize].conv == Conv::Color);
            if persist == Persist::No && (colour_input || self.eval_default(&e).is_some() || matches!(e, E::InSrc(_))) {
                self.known.insert((d, slot), e.clone());
            } else if persist == Persist::No {
                self.defs.insert((d, slot), value.clone());
            }
            if let Some(t) = ty {
                self.slot_types.insert((d, slot), t);
            }
            let mut pal = Vec::new();
            self.palette(&e, &mut pal);
            if !pal.is_empty() {
                self.palettes.insert((d, slot), pal);
            }
            self.declare(&names[0], Bind::Slot { depth: d, slot });
            return Ok(match persist {
                Persist::No => S::Let(slot, e),
                _ => S::Var(slot, e),
            });
        }
        let slots: Vec<Option<u16>> = names
            .iter()
            .map(|n| {
                (n != "_").then(|| {
                    let s = self.new_slot();
                    self.declare(n, Bind::Slot { depth: d, slot: s });
                    s
                })
            })
            .collect();
        Ok(S::LetTuple(slots, e))
    }

    /// The user type of a declaration's value, when it's evident.
    /// (Declared types win; see `decl`.)
    fn type_of_expr(&self, e: &Expr) -> Option<String> {
        match e {
            Expr::Call { callee, .. } => {
                if let Some(t) = callee.strip_suffix(".new")
                    && let Some(i) = self.type_id(t)
                {
                    return Some(self.types[i].name.clone());
                }
                for fam in [
                    "array", "map", "matrix", "label", "line", "box", "table", "linefill", "polyline",
                ] {
                    if callee.starts_with(&format!("{fam}.")) {
                        return Some(fam.to_string());
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn func(
        &mut self,
        name: &str,
        params: &[(String, Option<Expr>, Option<String>)],
        body: &ast::Block,
        method: bool,
    ) -> R<()> {
        if self.depth() != 0 || self.local > 0 {
            return Err(self.err("Functions can only be declared at the top level of the script."));
        }
        let mut defaults = Vec::new();
        for (_, d, _) in params {
            defaults.push(match d {
                Some(e) => {
                    let e = self.expr(e)?;
                    Some(const_of(&e).ok_or_else(|| self.err("A parameter's default value must be a constant."))?)
                }
                None => None,
            });
        }
        // Register first, so the body can't see itself (Pine forbids
        // recursion) but later code can call it.
        self.frames.push(Layout::default());
        self.scopes.push((self.depth(), HashMap::new()));
        let d = self.depth();
        for (p, _, ty) in params {
            let slot = self.new_slot();
            if let Some(t) = ty {
                self.slot_types.insert((d, slot), t.clone());
            }
            self.declare(p, Bind::Slot { depth: d, slot });
        }
        let saved_known = std::mem::take(&mut self.known);
        let saved_defs = std::mem::take(&mut self.defs);
        let body = self.block(body);
        self.known = saved_known;
        self.defs = saved_defs;
        self.scopes.pop();
        let layout = self.frames.pop().unwrap_or_default();
        let body = body?;
        let id = self.funcs.len() as u16;
        let method_of = if method {
            let t = params.first().and_then(|p| p.2.clone()).unwrap_or_default();
            Some(match self.type_id(&t) {
                Some(i) => self.types[i].name.clone(),
                None => t,
            })
        } else {
            None
        };
        self.funcs.push(Func {
            name: name.to_string(),
            params: params.iter().map(|p| p.0.clone()).collect(),
            defaults,
            body,
            layout,
        });
        if method {
            let ty = method_of.filter(|t| !t.is_empty());
            self.methods.entry(name.to_string()).or_default().push((id, ty));
        } else {
            self.fnames.insert(name.to_string(), id);
        }
        Ok(())
    }

    // -------------------------------------------------------- expressions

    fn expr(&mut self, e: &Expr) -> R<E> {
        Ok(match e {
            Expr::Int(i) => E::K(V::Int(*i)),
            Expr::Float(f) => E::K(V::Float(*f)),
            Expr::Str(s) => E::K(V::str(s.as_str())),
            Expr::Bool(b) => E::K(V::Bool(*b)),
            Expr::Color(c) => E::K(V::Color(*c)),
            Expr::Na => E::K(V::Na),
            Expr::Name(n) => self.name(n)?,
            Expr::Call { callee, args } => self.call(callee, args)?,
            Expr::Index(target, idx) => self.index(target, idx)?,
            Expr::Unary(op, x) => {
                let x = self.expr(x)?;
                let neg = *op == "-";
                if let E::K(v) = &x {
                    return Ok(E::K(if neg { crate::run::neg(v) } else { V::Bool(!v.truthy()) }));
                }
                if neg { E::Neg(Box::new(x)) } else { E::Not(Box::new(x)) }
            }
            Expr::Binary(op, a, b) => {
                let (a, b) = (self.expr(a)?, self.expr(b)?);
                let lazy = self.version >= 6;
                match *op {
                    "and" => E::And(Box::new(a), Box::new(b), lazy),
                    "or" => E::Or(Box::new(a), Box::new(b), lazy),
                    _ => {
                        let bop = Bin::of(op).ok_or_else(|| self.err(format!("Unknown operator {op}.")))?;
                        if let (E::K(x), E::K(y)) = (&a, &b) {
                            // v5 divides constant ints as ints.
                            if bop == Bin::Div
                                && self.version < 6
                                && let (V::Int(p), V::Int(q)) = (x, y)
                                && *q != 0
                            {
                                return Ok(E::K(V::Int(p / q)));
                            }
                            return Ok(E::K(crate::run::binary(bop, x, y)));
                        }
                        E::Bin(bop, Box::new(a), Box::new(b))
                    }
                }
            }
            Expr::Ternary(c, a, b) => {
                let c = self.expr(c)?;
                let (a, b) = (self.expr(a)?, self.expr(b)?);
                if let E::K(v) = &c {
                    return Ok(if v.truthy() { a } else { b });
                }
                E::Tern(Box::new(c), Box::new(a), Box::new(b))
            }
            Expr::If(branches, otherwise) => {
                let mut bs = Vec::new();
                for (c, b) in branches {
                    let c = self.expr(c)?;
                    bs.push((c, self.local_block(b)?));
                }
                let o = otherwise.as_ref().map(|b| self.local_block(b)).transpose()?;
                E::If(bs, o)
            }
            Expr::Switch(subject, cases) => {
                let subj = subject.as_ref().map(|s| self.expr(s)).transpose()?.map(Box::new);
                let mut cs = Vec::new();
                for (c, b) in cases {
                    let c = c.as_ref().map(|c| self.expr(c)).transpose()?;
                    cs.push((c, self.local_block(b)?));
                }
                E::Switch(subj, cs)
            }
            Expr::Tuple(items) => E::Tuple(items.iter().map(|x| self.expr(x)).collect::<R<_>>()?),
            Expr::Dot(obj, field) => {
                let o = self.expr(obj)?;
                self.check_field(field)?;
                E::Get(Box::new(o), Arc::from(field.as_str()))
            }
            Expr::MethodCall { recv, name, args } => {
                let r = self.expr(recv)?;
                self.method(r, name, args)?
            }
        })
    }

    /// A variable from outside the `request.security` expression being
    /// compiled: constants and inputs are read as they are; series are
    /// recomputed from their definition on the requested timeframe.
    fn sec_outer(&mut self, n: &str, depth: usize, slot: u16) -> Option<R<E>> {
        let sd = *self.sec_frames.last()?;
        if depth >= sd {
            return None;
        }
        if let Some(k) = self.known.get(&(depth, slot)) {
            return Some(Ok(k.clone()));
        }
        let Some(def) = self.defs.get(&(depth, slot)).cloned() else {
            return Some(Err(self.err(format!(
                "request.security() can't recompute \"{n}\" on another timeframe because it changes with := or var. Put the calculation in a function and call that inside request.security()."
            ))));
        };
        if self.inlining.contains(&(depth, slot)) {
            return Some(Err(self.err(format!("\"{n}\" depends on itself."))));
        }
        self.inlining.push((depth, slot));
        let e = self.expr(&def);
        self.inlining.pop();
        Some(e)
    }

    fn name(&mut self, n: &str) -> R<E> {
        match self.lookup(n) {
            Some(Bind::Slot { depth, slot }) => {
                if let Some(e) = self.sec_outer(n, depth, slot) {
                    return e;
                }
                return self.slot_ref(depth, slot);
            }
            Some(Bind::Plot(_) | Bind::HLine(_)) => {
                return Err(self.err(format!("\"{n}\" is a plot; use it in fill().")));
            }
            None => {}
        }
        if let Some(f) = Field::by_name(n) {
            if self.sec_depth > 0 {
                self.need_bar_hist(0);
            }
            return Ok(E::Bar(f));
        }
        // A built-in variable (`ta.tr`, `barstate.islast`, `syminfo.mintick`).
        if let Some(def) = natives::lookup(n)
            && def.var
        {
            let args = match n {
                "ta.vwap" => vec![E::Bar(Field::Hlc3)],
                _ => vec![E::K(V::Na); def.params.len()],
            };
            return Ok(self.nat(def, args));
        }
        if let Some(v) = constant(n) {
            return Ok(E::K(v));
        }
        if let Some((en, m)) = n.rsplit_once('.')
            && let en = self.in_lib(en, |cx, e| cx.enums.contains_key(e))
            && let Some(members) = self.enums.get(&en)
        {
            return match members.iter().find(|(name, _)| name == m) {
                Some((_, t)) => Ok(E::K(V::str(t.clone()))),
                None => Err(self.err(format!("{en} has no member called \"{m}\"."))),
            };
        }
        // obj.field
        if let Some((obj, field)) = n.rsplit_once('.')
            && self.lookup(obj.split('.').next().unwrap_or(obj)).is_some()
        {
            let (oe, fi) = self.field_path(obj, field)?;
            return Ok(E::Get(Box::new(oe), fi));
        }
        if natives::lookup(n).is_some() || self.fnames.contains_key(n) {
            return Err(self.err(format!("{n} is a function; call it with brackets: {n}(…).")));
        }
        Err(self.err(format!("Unknown name \"{n}\".")))
    }

    /// `a.b.c` → (expression for `a.b`, field `c`), checking that some
    /// type has such a field.
    fn field_path(&mut self, obj: &str, field: &str) -> R<(E, Arc<str>)> {
        let oe = self.name(obj)?;
        self.check_field(field)?;
        Ok((oe, Arc::from(field)))
    }

    fn check_field(&self, field: &str) -> R<()> {
        let known = crate::run::CHART_POINT_FIELDS.contains(&field)
            || self.types.iter().any(|t| t.fields.iter().any(|f| f == field));
        if known {
            Ok(())
        } else {
            Err(self.err(format!("No type has a field called \"{field}\".")))
        }
    }

    fn nat(&mut self, f: &'static NatDef, args: Vec<E>) -> E {
        if f.pure && args.iter().all(|a| matches!(a, E::K(_))) {
            let vals: Vec<V> = args.iter().map(|a| const_of(a).unwrap_or_default()).collect();
            let mut g = crate::run::G::scratch(self.env.clone());
            let mut st = None;
            if let Ok(v) = (f.f)(&mut g, &vals, &mut st) {
                return E::K(v);
            }
        }
        let site = if f.state { self.new_state() } else { u16::MAX };
        E::Nat { f, args, site }
    }

    fn index(&mut self, target: &Expr, idx: &Expr) -> R<E> {
        let i = self.expr(idx)?;
        let n = match &i {
            E::K(v) => match v.i() {
                Some(k) if k >= 0 => k as usize,
                Some(_) => return Err(self.err("History offsets can't be negative.")),
                None => 0,
            },
            _ => DYN_HIST,
        };
        if let Expr::Name(name) = target {
            let outer = |cx: &Self, depth: usize| cx.sec_frames.last().is_some_and(|sd| depth < *sd);
            match self.lookup(name) {
                Some(Bind::Slot { depth, slot })
                    if !outer(self, depth) || matches!(self.known.get(&(depth, slot)), Some(E::InSrc(_))) =>
                {
                    // An input.source variable reads bar history directly.
                    if let Some(E::InSrc(k)) = self.known.get(&(depth, slot)) {
                        let k = *k;
                        self.need_bar_hist(n);
                        return Ok(E::InSrcH(k, Box::new(i)));
                    }
                    self.need_hist(depth, slot, n);
                    let r = self.slot_ref(depth, slot)?;
                    return Ok(match r {
                        E::L(s) => E::LH(s, Box::new(i)),
                        E::G(s) => E::GH(s, Box::new(i)),
                        _ => unreachable!(),
                    });
                }
                None => {
                    if let Some(f) = Field::by_name(name) {
                        self.need_bar_hist(n);
                        return Ok(E::BarH(f, Box::new(i)));
                    }
                }
                _ => {}
            }
        }
        let val = self.expr(target)?;
        if let E::InSrc(k) = val {
            self.need_bar_hist(n);
            return Ok(E::InSrcH(k, Box::new(i)));
        }
        if let E::K(_) = val {
            return Ok(val);
        }
        let slot = self.new_slot();
        let d = self.depth();
        self.need_hist(d, slot, n);
        Ok(E::HistOf {
            slot,
            val: Box::new(val),
            idx: Box::new(i),
        })
    }

    fn call(&mut self, callee: &str, args: &[Arg]) -> R<E> {
        // Generic arguments only matter to the type checker.
        let plain;
        let callee = match callee.find('<') {
            Some(k) => {
                plain = callee[..k].to_string();
                plain.as_str()
            }
            None => callee,
        };
        // User functions shadow built-ins.
        let user = self.in_lib(callee, |cx, n| cx.fnames.contains_key(n));
        if let Some(&f) = self.fnames.get(&user) {
            return self.user_call(f, args);
        }
        if let Some(t) = callee.strip_suffix(".new")
            && let Some(ti) = self.type_id(t)
        {
            return self.new_udt(ti, args);
        }
        if callee.starts_with("input") && (callee == "input" || callee.starts_with("input.")) {
            return self.input(callee, args);
        }
        if matches!(
            callee,
            "plot"
                | "plotshape"
                | "plotchar"
                | "plotarrow"
                | "plotcandle"
                | "plotbar"
                | "hline"
                | "fill"
                | "bgcolor"
                | "barcolor"
                | "alertcondition"
                | "indicator"
                | "strategy"
                | "study"
                | "library"
        ) {
            return Err(self.err(format!("{callee}() can't be used inside an expression.")));
        }
        if matches!(callee, "request.security" | "security" | "request.security_lower_tf") {
            return self.security(callee, args);
        }
        // Method call on a variable: `arr.push(x)`, `obj.method()`.
        if let Some((obj, m)) = callee.rsplit_once('.')
            && natives::lookup(callee).is_none()
            && (self.lookup(obj.split('.').next().unwrap_or(obj)).is_some() || obj.contains('('))
        {
            let recv = self.name(obj)?;
            return self.method(recv, m, args);
        }
        if let Some(def) = natives::lookup(callee) {
            let mut argv = self.nat_args(def, args)?;
            // Overloads with a different first argument.
            match callee {
                "ta.pivothigh" | "ta.pivotlow" if args.iter().filter(|a| a.name.is_none()).count() == 2 => {
                    argv.rotate_right(1);
                    argv[0] = E::Bar(if callee == "ta.pivothigh" {
                        Field::High
                    } else {
                        Field::Low
                    });
                }
                "ta.highest" | "ta.lowest" | "ta.highestbars" | "ta.lowestbars"
                    if args.len() == 1 && args[0].name.is_none() =>
                {
                    argv[1] = argv[0].clone();
                    argv[0] = E::Bar(if callee.starts_with("ta.highest") {
                        Field::High
                    } else {
                        Field::Low
                    });
                }
                _ => {}
            }
            return Ok(self.nat(def, argv));
        }
        if self.methods.contains_key(callee) {
            return Err(self.err(format!("{callee} is a method; call it on a value: x.{callee}(…).")));
        }
        Err(self.err(format!("Unknown function \"{callee}\".")))
    }

    fn nat_args(&mut self, def: &'static NatDef, args: &[Arg]) -> R<Vec<E>> {
        if def.params == ["*"] {
            return args.iter().map(|a| self.expr(&a.value)).collect();
        }
        let bound = bind(def.name, def.params, args, self.line)?;
        bound
            .into_iter()
            .map(|a| match a {
                Some(e) => self.expr(e),
                None => Ok(E::K(V::Na)),
            })
            .collect()
    }

    fn user_call(&mut self, f: u16, args: &[Arg]) -> R<E> {
        let func = self.funcs[f as usize].clone();
        let params: Vec<&str> = func.params.iter().map(|s| s.as_str()).collect();
        let bound = bind(&func.name, &params, args, self.line)?;
        let mut argv = Vec::new();
        for (k, a) in bound.into_iter().enumerate() {
            argv.push(match a {
                Some(e) => self.expr(e)?,
                None => match &func.defaults[k] {
                    Some(v) => E::K(v.clone()),
                    None => {
                        return Err(self.err(format!("{}() needs a value for \"{}\".", func.name, func.params[k])));
                    }
                },
            });
        }
        let frame = self.frame();
        frame.calls.push(f);
        let site = (frame.calls.len() - 1) as u16;
        Ok(E::Call { f, site, args: argv })
    }

    fn new_udt(&mut self, ti: usize, args: &[Arg]) -> R<E> {
        let t = self.types[ti].clone();
        let params: Vec<&str> = t.fields.iter().map(|s| s.as_str()).collect();
        let bound = bind(&format!("{}.new", t.name), &params, args, self.line)?;
        let mut vals = Vec::new();
        for (k, a) in bound.into_iter().enumerate() {
            vals.push(match a {
                Some(e) => self.expr(e)?,
                None => E::K(t.defaults[k].clone()),
            });
        }
        Ok(E::New(ti as u16, vals))
    }

    fn method(&mut self, recv: E, m: &str, args: &[Arg]) -> R<E> {
        let user: Vec<(u16, u16)> = self
            .methods
            .get(m)
            .map(|v| {
                v.iter()
                    .map(|(f, ty)| {
                        let tid = ty
                            .as_ref()
                            .and_then(|t| self.types.iter().position(|x| &x.name == t))
                            .map(|i| i as u16)
                            .unwrap_or(u16::MAX);
                        (*f, tid)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut argv = Vec::new();
        let mut names = Vec::new();
        for a in args {
            names.push(a.name.as_deref().map(Arc::from));
            argv.push(self.expr(&a.value)?);
        }
        // One call site (its own frame) per candidate user method.
        let mut cands = Vec::new();
        for (f, t) in user {
            let frame = self.frame();
            frame.calls.push(f);
            cands.push((f, t, (frame.calls.len() - 1) as u16));
        }
        Ok(E::Method {
            name: Arc::from(m),
            recv: Box::new(recv),
            args: argv,
            names,
            user: cands,
        })
    }

    fn security(&mut self, callee: &str, args: &[Arg]) -> R<E> {
        let lower = callee == "request.security_lower_tf";
        let params: &[&str] = if lower {
            &[
                "symbol",
                "timeframe",
                "expression",
                "ignore_invalid_symbol",
                "currency",
                "ignore_invalid_timeframe",
                "calc_bars_count",
            ]
        } else {
            &[
                "symbol",
                "timeframe",
                "expression",
                "gaps",
                "lookahead",
                "ignore_invalid_symbol",
                "currency",
                "calc_bars_count",
            ]
        };
        let b = bind(callee, params, args, self.line)?;
        let (Some(sym), Some(tf), Some(expr)) = (b[0], b[1], b[2]) else {
            return Err(self.err(format!("{callee}() needs a symbol, a timeframe and an expression.")));
        };
        let sym = self.expr(sym)?;
        // This chart's symbol is known now; anything else is looked up when
        // the script runs (its bars are fetched on demand).
        let symbol = match &sym {
            E::K(V::Str(s)) => {
                let same = s.as_ref() == self.env.tickerid || s.as_ref() == self.env.ticker() || s.is_empty();
                (!same).then_some(sym)
            }
            E::Nat { f, .. } if f.name == "syminfo.tickerid" || f.name == "syminfo.ticker" => None,
            _ => Some(sym),
        };
        let tf = self.expr(tf)?;
        let gaps = match b.get(3).copied().flatten().filter(|_| !lower) {
            Some(g) => self.expr(g)?,
            None => E::K(V::Bool(false)),
        };
        let lookahead = match b.get(4).copied().flatten().filter(|_| !lower) {
            Some(g) => self.expr(g)?,
            None => E::K(V::Bool(false)),
        };
        // The expression gets its own frame, as if it were a function
        // called on the other timeframe's bars.
        self.frames.push(Layout::default());
        self.scopes.push((self.depth(), HashMap::new()));
        self.sec_depth += 1;
        self.sec_frames.push(self.depth());
        let saved_bar = self.sec_bar_hist;
        self.sec_bar_hist = 1;
        let e = self.expr(expr);
        let bar_hist = self.sec_bar_hist;
        self.sec_bar_hist = saved_bar;
        self.sec_depth -= 1;
        self.sec_frames.pop();
        self.scopes.pop();
        let layout = self.frames.pop().unwrap_or_default();
        let e = e?;
        let frame = self.frame();
        frame.secs += 1;
        let site = (frame.secs - 1) as u16;
        Ok(E::Security(Box::new(Sec {
            symbol,
            tf,
            expr: e,
            gaps,
            lookahead,
            site,
            layout,
            bar_hist,
            lower,
        })))
    }

    // -------------------------------------------------------- inputs

    fn input(&mut self, callee: &str, args: &[Arg]) -> R<E> {
        let params: &[&str] = match callee {
            "input" => &[
                "defval", "title", "type", "minval", "maxval", "step", "options", "group",
            ],
            "input.int" | "input.float" => &["defval", "title", "minval", "maxval", "step", "options", "group"],
            "input.string" => &["defval", "title", "options", "group"],
            "input.bool" | "input.color" | "input.source" | "input.symbol" | "input.session" | "input.text_area"
            | "input.time" | "input.price" | "input.timeframe" => &["defval", "title", "options", "group"],
            "input.enum" => &["defval", "title", "options", "group"],
            _ => return Err(self.err(format!("Unknown input type \"{callee}\"."))),
        };
        let b = bind(callee, params, args, self.line)?;
        let get = |cx: &mut Cx, name: &str| -> R<Option<V>> {
            let Some(k) = params.iter().position(|p| *p == name) else {
                return Ok(None);
            };
            match b[k] {
                Some(e) => {
                    let e = cx.expr(e)?;
                    Ok(Some(const_of(&e).ok_or_else(|| {
                        cx.err(format!("The input's \"{name}\" must be a constant."))
                    })?))
                }
                None => Ok(None),
            }
        };
        let title = get(self, "title")?.and_then(|v| v.as_str().map(|s| s.to_string()));
        let group = get(self, "group")?.and_then(|v| v.as_str().map(|s| s.to_string()));
        let minval = get(self, "minval")?;
        let maxval = get(self, "maxval")?;
        let step = get(self, "step")?;
        let options: Vec<V> = match params.iter().position(|p| *p == "options").and_then(|k| b[k]) {
            Some(Expr::Tuple(items)) => {
                let mut out = Vec::new();
                for it in items {
                    let e = self.expr(it)?;
                    out.push(const_of(&e).ok_or_else(|| self.err("Input options must be constants."))?);
                }
                out
            }
            Some(_) => return Err(self.err("Input options go in square brackets: options = [\"A\", \"B\"].")),
            None => Vec::new(),
        };
        // input.source's default is a bar field, not a constant.
        let defval_expr = b[0].ok_or_else(|| self.err(format!("{callee}() needs a default value.")))?;
        // input.enum offers the enum's titles (or the listed members).
        let options = if callee == "input.enum" {
            let en = match defval_expr {
                Expr::Name(n) => n.rsplit_once('.').map(|(e, _)| e.to_string()),
                _ => None,
            }
            .filter(|e| self.enums.contains_key(e))
            .ok_or_else(|| self.err("input.enum() needs an enum member as its default, like Signal.buy."))?;
            if options.is_empty() {
                self.enums[&en].iter().map(|(_, t)| V::str(t.clone())).collect()
            } else {
                options
            }
        } else {
            options
        };
        let src_default = match defval_expr {
            Expr::Name(n) if Field::by_name(n).is_some() => Some(n.clone()),
            _ => None,
        };
        let is_source = callee == "input.source" || (callee == "input" && src_default.is_some());
        let defval = if is_source {
            V::str(src_default.clone().unwrap_or_else(|| "close".into()))
        } else {
            let e = self.expr(defval_expr)?;
            const_of(&e).ok_or_else(|| self.err("An input's default value must be a constant."))?
        };
        let opts_text = |opts: &[V]| -> &'static [&'static str] {
            let v: Vec<&'static str> = opts
                .iter()
                .map(|o| {
                    leak(match o {
                        V::Str(s) => s.to_string(),
                        other => value::fmt_num(other.f()),
                    })
                })
                .collect();
            Box::leak(v.into_boxed_slice())
        };
        let (kind, default, conv) = if is_source {
            (
                InputKind::Source,
                Value::Text(defval.as_str().unwrap_or("close").into()),
                Conv::Source,
            )
        } else {
            match (callee, &defval) {
                ("input.color", _) | ("input", V::Color(_)) => (
                    InputKind::Color,
                    Value::Text(format!("#{:08X}", defval.color().unwrap_or(0))),
                    Conv::Color,
                ),
                ("input.bool", _) | ("input", V::Bool(_)) => {
                    (InputKind::Bool, Value::Bool(defval.truthy()), Conv::Bool)
                }
                ("input.int", _) | ("input", V::Int(_)) if !options.is_empty() => (
                    InputKind::Choice(opts_text(&options)),
                    Value::Text(value::fmt_num(defval.f())),
                    Conv::IntChoice,
                ),
                ("input.float", _) | ("input", V::Float(_)) if !options.is_empty() => (
                    InputKind::Choice(opts_text(&options)),
                    Value::Text(value::fmt_num(defval.f())),
                    Conv::FloatChoice,
                ),
                ("input.int", _) | ("input", V::Int(_)) => (
                    InputKind::Int {
                        min: minval.as_ref().and_then(|v| v.i()).unwrap_or(-1_000_000_000_000),
                        max: maxval.as_ref().and_then(|v| v.i()).unwrap_or(1_000_000_000_000),
                    },
                    Value::Int(defval.i().unwrap_or(0)),
                    Conv::Int,
                ),
                ("input.float" | "input.price", _) | ("input", V::Float(_)) => (
                    InputKind::Float {
                        min: minval.as_ref().map(|v| v.f()).filter(|f| !f.is_nan()).unwrap_or(-1e15),
                        max: maxval.as_ref().map(|v| v.f()).filter(|f| !f.is_nan()).unwrap_or(1e15),
                        step: step.as_ref().map(|v| v.f()).filter(|f| !f.is_nan()).unwrap_or(0.1),
                    },
                    Value::Float(defval.f()),
                    Conv::Float,
                ),
                ("input.time", _) => (
                    InputKind::Int {
                        min: i64::MIN / 2,
                        max: i64::MAX / 2,
                    },
                    Value::Int(defval.i().unwrap_or(0)),
                    Conv::Int,
                ),
                (_, _) if !options.is_empty() => (
                    InputKind::Choice(opts_text(&options)),
                    Value::Text(defval.as_str().unwrap_or("").into()),
                    Conv::Str,
                ),
                (_, _) => (
                    InputKind::Text,
                    Value::Text(defval.as_str().unwrap_or("").into()),
                    Conv::Str,
                ),
            }
        };
        let title = title.unwrap_or_else(|| match conv {
            Conv::Source => "Source".into(),
            _ => format!("Input {}", self.inputs.len() + 1),
        });
        let mut id = slug(&title);
        let base = id.clone();
        let mut k = 2;
        while self.input_ids.contains_key(&id) {
            id = format!("{base}_{k}");
            k += 1;
        }
        self.input_ids.insert(id.clone(), self.inputs.len());
        self.inputs.push(InputDef {
            spec: InputSpec {
                id: leak(id),
                title: leak(title),
                kind,
                default,
                group: group.map(leak),
            },
            conv,
        });
        let i = (self.inputs.len() - 1) as u16;
        Ok(if conv == Conv::Source { E::InSrc(i) } else { E::In(i) })
    }

    /// The input default behind an expression (for declarations).
    fn const_or_default(&self, e: &E) -> Option<V> {
        match e {
            E::K(v) => Some(v.clone()),
            E::In(i) => Some(self.input_default(*i)),
            E::L(s) | E::G(s) => {
                let depth = if matches!(e, E::G(_)) { 0 } else { self.depth() };
                self.known.get(&(depth, *s)).and_then(|k| self.const_or_default(k))
            }
            E::Neg(x) => self.const_or_default(x).map(|v| crate::run::neg(&v)),
            _ => None,
        }
    }

    fn input_default(&self, i: u16) -> V {
        let d = &self.inputs[i as usize];
        match (&d.spec.default, d.conv) {
            (Value::Int(x), _) => V::Int(*x),
            (Value::Float(x), _) => V::Float(*x),
            (Value::Bool(x), _) => V::Bool(*x),
            (Value::Text(t), Conv::IntChoice) => V::Int(t.parse().unwrap_or(0)),
            (Value::Text(t), Conv::FloatChoice) => V::Float(t.parse().unwrap_or(0.0)),
            (Value::Text(t), Conv::Color) => V::Color(crate::run::parse_hex(t).unwrap_or(0)),
            (Value::Text(t), _) => V::str(t.as_str()),
        }
    }

    /// `±input + k`, as (input, sign, k); constants have no input.
    fn linear(&self, e: &E) -> Option<(Option<u16>, i64, i64)> {
        match e {
            E::K(v) => Some((None, 0, v.i()?)),
            E::In(i) => Some((Some(*i), 1, 0)),
            E::L(s) | E::G(s) => {
                let depth = if matches!(e, E::G(_)) { 0 } else { self.depth() };
                self.linear(self.known.get(&(depth, *s))?)
            }
            E::Neg(x) => {
                let (i, s, k) = self.linear(x)?;
                Some((i, -s, -k))
            }
            E::Bin(op @ (Bin::Add | Bin::Sub), a, b) => {
                let (ia, sa, ka) = self.linear(a)?;
                let (ib, sb, kb) = self.linear(b)?;
                let (sb, kb) = if *op == Bin::Sub { (-sb, -kb) } else { (sb, kb) };
                match (ia, ib) {
                    (Some(_), Some(_)) => None,
                    (Some(i), None) => Some((Some(i), sa, ka + kb)),
                    (None, Some(i)) => Some((Some(i), sb, ka + kb)),
                    (None, None) => Some((None, 0, ka + kb)),
                }
            }
            _ => None,
        }
    }

    /// A plot offset: a constant or a linear function of one input
    /// (`offset = -displacement + 1`).
    fn offset_of(&self, e: &E) -> R<Offset> {
        match self.linear(e) {
            Some((None, _, k)) => Ok(Offset::Fixed(k)),
            Some((Some(i), sign, add)) => Ok(Offset::Input {
                id: self.inputs[i as usize].spec.id,
                sign,
                add,
            }),
            // Anything else is evaluated once with the inputs' defaults.
            None => match self.eval_default(e).and_then(|v| v.i()) {
                Some(k) => Ok(Offset::Fixed(k)),
                None => Err(self.err("A plot's offset must be a constant or an input (plus or minus a constant).")),
            },
        }
    }

    /// Evaluates an expression of constants and inputs with the inputs'
    /// default values.
    fn eval_default(&self, e: &E) -> Option<V> {
        Some(match e {
            E::K(v) => v.clone(),
            E::In(i) => self.input_default(*i),
            E::L(s) | E::G(s) => {
                let depth = if matches!(e, E::G(_)) { 0 } else { self.depth() };
                return self.eval_default(self.known.get(&(depth, *s))?);
            }
            E::Neg(x) => crate::run::neg(&self.eval_default(x)?),
            E::Not(x) => V::Bool(!self.eval_default(x)?.truthy()),
            E::Bin(op, a, b) => crate::run::binary(*op, &self.eval_default(a)?, &self.eval_default(b)?),
            E::Tern(c, a, b) => {
                if self.eval_default(c)?.truthy() {
                    self.eval_default(a)?
                } else {
                    self.eval_default(b)?
                }
            }
            _ => return None,
        })
    }

    // -------------------------------------------------------- outputs

    /// Declaration and output calls. `Ok(None)` = not one of them;
    /// `Ok(Some(None))` = handled, nothing to run per bar.
    fn output_call(&mut self, callee: &str, args: &[Arg]) -> R<Option<Option<S>>> {
        let line = self.line;
        let is_output = matches!(
            callee,
            "plot"
                | "plotshape"
                | "plotchar"
                | "plotarrow"
                | "plotcandle"
                | "plotbar"
                | "hline"
                | "fill"
                | "bgcolor"
                | "barcolor"
                | "alertcondition"
        );
        if is_output && (self.local > 0 || self.depth() > 0) {
            return Err(self.err(format!(
                "{callee}() can't be used in a local block or function (Pine's rule)."
            )));
        }
        Ok(Some(match callee {
            "indicator" | "study" | "strategy" | "library" => {
                self.declaration(callee, args)?;
                None
            }
            "plot" => Some(self.plot(args)?),
            "plotshape" | "plotchar" => Some(self.plotshape(callee, args)?),
            "plotarrow" => Some(self.plotarrow(args)?),
            "plotcandle" | "plotbar" => Some(self.plotcandle(callee, args)?),
            "hline" => {
                self.hline(args)?;
                None
            }
            "fill" => self.fill(args)?,
            "bgcolor" | "barcolor" => {
                let b = bind(callee, &["color", "offset", "title", "transp"], args, line)?;
                let Some(c) = b[0] else {
                    return Err(self.err(format!("{callee}() needs a colour.")));
                };
                let mut c = self.expr(c)?;
                if let Some(t) = b[3] {
                    let t = self.expr(t)?;
                    let f = natives::lookup("color.new").expect("color.new");
                    c = self.nat(f, vec![c, t]);
                }
                Some(if callee == "bgcolor" { S::Bg(c) } else { S::BarColor(c) })
            }
            "alertcondition" => {
                let b = bind(callee, &["condition", "title", "message"], args, line)?;
                let Some(cond) = b[0] else {
                    return Err(self.err("alertcondition() needs a condition."));
                };
                let cond = self.expr(cond)?;
                let text = |cx: &mut Cx, e: Option<&Expr>| -> R<String> {
                    Ok(match e {
                        Some(e) => {
                            let e = cx.expr(e)?;
                            const_of(&e)
                                .and_then(|v| v.as_str().map(|s| s.to_string()))
                                .unwrap_or_default()
                        }
                        None => String::new(),
                    })
                };
                let title = text(self, b[1])?;
                let message = text(self, b[2])?;
                let title = if title.is_empty() {
                    format!("Alert {}", self.alertconditions.len() + 1)
                } else {
                    title
                };
                self.alertconditions.push(AlertCondDef { title, message });
                Some(S::AlertCond {
                    i: (self.alertconditions.len() - 1) as u16,
                    cond,
                })
            }
            _ => return Ok(None),
        }))
    }

    fn declaration(&mut self, callee: &str, args: &[Arg]) -> R<()> {
        if self.kind.is_some() {
            return Err(self.err("A script can only declare indicator(), strategy() or library() once."));
        }
        self.kind = Some(match callee {
            "strategy" => Kind::Strategy,
            "library" => Kind::Library,
            _ => Kind::Indicator,
        });
        let params: &[&str] = match callee {
            "strategy" => &[
                "title",
                "shorttitle",
                "overlay",
                "format",
                "precision",
                "scale",
                "pyramiding",
                "calc_on_order_fills",
                "calc_on_every_tick",
                "max_bars_back",
                "backtest_fill_limits_assumption",
                "default_qty_type",
                "default_qty_value",
                "initial_capital",
                "currency",
                "slippage",
                "commission_type",
                "commission_value",
                "process_orders_on_close",
                "close_entries_rule",
                "margin_long",
                "margin_short",
                "max_lines_count",
                "max_labels_count",
                "max_boxes_count",
                "risk_free_rate",
                "use_bar_magnifier",
                "fill_orders_on_standard_ohlc",
                "max_polylines_count",
                "timeframe",
                "timeframe_gaps",
            ],
            "library" => &["title", "overlay", "dynamic_requests"],
            _ => &[
                "title",
                "shorttitle",
                "overlay",
                "format",
                "precision",
                "scale",
                "max_bars_back",
                "timeframe",
                "timeframe_gaps",
                "max_lines_count",
                "max_labels_count",
                "max_boxes_count",
                "max_polylines_count",
            ],
        };
        // `format` and `precision` are ignored elsewhere but read here.
        let mut named: Vec<Arg> = Vec::new();
        let mut fmt = None;
        let mut precision = None;
        for a in args {
            match a.name.as_deref() {
                Some("format") => fmt = Some(a.value.clone()),
                Some("precision") => precision = Some(a.value.clone()),
                _ => named.push(a.clone()),
            }
        }
        let b = bind(callee, params, &named, self.line)?;
        for (k, p) in params.iter().enumerate() {
            let Some(e) = b[k] else { continue };
            let e = self.expr(e)?;
            let Some(v) = self.const_or_default(&e) else {
                return Err(self.err(format!("{callee}()'s \"{p}\" must be a constant.")));
            };
            match *p {
                "title" => self.title = v.as_str().unwrap_or("").to_string(),
                "shorttitle" => self.short = v.as_str().unwrap_or("").to_string(),
                "overlay" => self.overlay = v.truthy(),
                "format" => fmt = Some(Expr::Str(v.as_str().unwrap_or("").to_string())),
                "precision" => precision = Some(Expr::Int(v.i().unwrap_or(2))),
                "max_lines_count" => self.limits[1] = v.i_or(50).clamp(1, 500) as usize,
                "max_labels_count" => self.limits[0] = v.i_or(50).clamp(1, 500) as usize,
                "max_boxes_count" => self.limits[2] = v.i_or(50).clamp(1, 500) as usize,
                "max_polylines_count" => self.limits[5] = v.i_or(50).clamp(1, 100) as usize,
                _ => {
                    let j = match &v {
                        V::Int(i) => serde_json::json!(i),
                        V::Float(f) => serde_json::json!(f),
                        V::Bool(b) => serde_json::json!(b),
                        V::Str(s) => serde_json::json!(s.as_ref()),
                        _ => continue,
                    };
                    self.props.insert(p.to_string(), j);
                }
            }
        }
        if let Some(f) = fmt {
            let e = self.expr(&f)?;
            if let Some(V::Str(s)) = self.const_or_default(&e) {
                self.format = match s.as_ref() {
                    "format.volume" => Format::Volume,
                    "format.percent" => Format::Fixed(2),
                    _ => self.format,
                };
            }
        }
        if let Some(p) = precision {
            let e = self.expr(&p)?;
            if let Some(v) = self.const_or_default(&e).and_then(|v| v.i()) {
                self.format = Format::Fixed(v.clamp(0, 16) as u8);
            }
        }
        if self.title.is_empty() {
            return Err(self.err(format!("{callee}() needs a title, e.g. {callee}(\"My script\").")));
        }
        if self.version >= 6 && callee == "study" {
            return Err(self.err("study() is from Pine v4; use indicator()."));
        }
        Ok(())
    }

    /// The palette of a colour expression: every colour constant in it.
    fn palette(&self, e: &E, out: &mut Vec<u32>) {
        match e {
            E::K(V::Color(c)) => {
                if !out.contains(c) {
                    out.push(*c);
                }
            }
            E::In(i) if self.inputs[*i as usize].conv == Conv::Color => {
                if let Some(c) = self.input_default(*i).color()
                    && !out.contains(&c)
                {
                    out.push(c);
                }
            }
            E::Tern(_, a, b) => {
                self.palette(a, out);
                self.palette(b, out);
            }
            E::If(bs, o) => {
                for (_, b) in bs {
                    if let Some((_, S::Expr(x))) = b.last() {
                        self.palette(x, out);
                    }
                }
                if let Some(b) = o
                    && let Some((_, S::Expr(x))) = b.last()
                {
                    self.palette(x, out);
                }
            }
            E::Switch(_, cs) => {
                for (_, b) in cs {
                    if let Some((_, S::Expr(x))) = b.last() {
                        self.palette(x, out);
                    }
                }
            }
            E::L(s) | E::G(s) => {
                let depth = if matches!(e, E::G(_)) { 0 } else { self.depth() };
                if let Some(k) = self.known.get(&(depth, *s)) {
                    self.palette(&k.clone(), out);
                }
                for c in self.palettes.get(&(depth, *s)).into_iter().flatten() {
                    if !out.contains(c) {
                        out.push(*c);
                    }
                }
            }
            _ => {}
        }
    }

    fn color_arg(&mut self, e: Option<&Expr>, default: u32) -> R<(Option<E>, Vec<u32>)> {
        match e {
            None => Ok((None, vec![default])),
            Some(e) => {
                let c = self.expr(e)?;
                let mut pal = Vec::new();
                self.palette(&c, &mut pal);
                if let E::K(V::Na) = c {
                    // Invisible.
                    return Ok((Some(c), vec![0]));
                }
                if pal.is_empty() {
                    pal.push(default);
                }
                // A constant colour needs no per-bar work.
                let dynamic = !matches!(c, E::K(_));
                Ok((dynamic.then_some(c), pal))
            }
        }
    }

    fn const_str(&mut self, e: Option<&Expr>) -> R<Option<String>> {
        match e {
            None => Ok(None),
            Some(e) => {
                let e = self.expr(e)?;
                Ok(self
                    .const_or_default(&e)
                    .and_then(|v| v.as_str().map(|s| s.to_string())))
            }
        }
    }

    fn const_num(&mut self, e: Option<&Expr>) -> R<Option<f64>> {
        match e {
            None => Ok(None),
            Some(e) => {
                let e = self.expr(e)?;
                Ok(self.const_or_default(&e).map(|v| v.f()).filter(|f| !f.is_nan()))
            }
        }
    }

    fn plot_title(&self, t: Option<String>) -> String {
        t.filter(|s| !s.is_empty()).unwrap_or_else(|| "Plot".into())
    }

    fn plot(&mut self, args: &[Arg]) -> R<S> {
        let b = bind(
            "plot",
            &[
                "series",
                "title",
                "color",
                "linewidth",
                "style",
                "trackprice",
                "histbase",
                "offset",
                "join",
                "linestyle",
                "transp",
            ],
            args,
            self.line,
        )?;
        let Some(series) = b[0] else {
            return Err(self.err("plot() needs a series to plot."));
        };
        let val = self.expr(series)?;
        let title = self.const_str(b[1])?;
        let (mut color, mut palette) = self.color_arg(b[2], opentern_series::color::tv::BLUE.0)?;
        if let Some(t) = b[10] {
            // v4's transp argument.
            let t = self.expr(t)?;
            if let Some(tv) = self.const_or_default(&t) {
                palette = palette.iter().map(|c| value::with_transp(*c, tv.f())).collect();
            }
            if let Some(c) = color.take() {
                let f = natives::lookup("color.new").expect("color.new");
                color = Some(self.nat(f, vec![c, t]));
            }
        }
        let width = self.const_num(b[3])?.unwrap_or(1.0).clamp(1.0, 10.0) as u8;
        let style = match self.const_str(b[4])?.as_deref() {
            Some("plot.style_linebr") => PlotStyle::LineBr,
            Some("plot.style_stepline" | "plot.style_stepline_diamond" | "plot.style_steplinebr") => {
                PlotStyle::StepLine
            }
            Some("plot.style_histogram") => PlotStyle::Histogram,
            Some("plot.style_columns") => PlotStyle::Columns,
            Some("plot.style_area" | "plot.style_areabr") => PlotStyle::Area,
            Some("plot.style_circles") => PlotStyle::Circles,
            Some("plot.style_cross") => PlotStyle::Cross,
            _ => PlotStyle::Line,
        };
        let base = self.const_num(b[6])?.unwrap_or(0.0);
        let offset = match b[7] {
            Some(e) => {
                let e = self.expr(e)?;
                self.offset_of(&e)?
            }
            None => Offset::Fixed(0),
        };
        let visible = !self.display_none(args)?;
        self.plots.push(PlotDef {
            title: self.plot_title(title),
            style,
            palette,
            width,
            offset,
            visible,
            base,
            shape: None,
        });
        Ok(S::Plot {
            i: (self.plots.len() - 1) as u16,
            val,
            color,
            how: PlotHow::Value,
        })
    }

    fn display_none(&mut self, args: &[Arg]) -> R<bool> {
        for a in args {
            if a.name.as_deref() == Some("display") {
                let e = self.expr(&a.value)?;
                return Ok(matches!(self.const_or_default(&e), Some(V::Str(s)) if s.as_ref() == "display.none"));
            }
        }
        Ok(false)
    }

    fn shape_size(s: Option<&str>) -> f32 {
        match s {
            Some("size.tiny") => 6.0,
            Some("size.small") => 9.0,
            Some("size.normal") => 13.0,
            Some("size.large") => 19.0,
            Some("size.huge") => 26.0,
            _ => 8.0,
        }
    }

    fn plotshape(&mut self, callee: &str, args: &[Arg]) -> R<S> {
        let shape = callee == "plotshape";
        let params: &[&str] = if shape {
            &[
                "series",
                "title",
                "style",
                "location",
                "color",
                "offset",
                "text",
                "textcolor",
                "size",
                "transp",
            ]
        } else {
            &[
                "series",
                "title",
                "char",
                "location",
                "color",
                "offset",
                "text",
                "textcolor",
                "size",
                "transp",
            ]
        };
        let b = bind(callee, params, args, self.line)?;
        let Some(series) = b[0] else {
            return Err(self.err(format!("{callee}() needs a series.")));
        };
        let val = self.expr(series)?;
        let title = self.const_str(b[1])?;
        let style = self.const_str(b[2])?;
        let (kind, ch) = if shape {
            let k = match style.as_deref() {
                Some("shape.cross") => ShapeKind::Cross,
                Some("shape.triangleup") => ShapeKind::TriangleUp,
                Some("shape.triangledown") => ShapeKind::TriangleDown,
                Some("shape.flag") => ShapeKind::Flag,
                Some("shape.circle") => ShapeKind::Circle,
                Some("shape.arrowup") => ShapeKind::ArrowUp,
                Some("shape.arrowdown") => ShapeKind::ArrowDown,
                Some("shape.labelup") => ShapeKind::LabelUp,
                Some("shape.labeldown") => ShapeKind::LabelDown,
                Some("shape.square") => ShapeKind::Square,
                Some("shape.diamond") => ShapeKind::Diamond,
                _ => ShapeKind::XCross,
            };
            (k, String::new())
        } else {
            (ShapeKind::Char, style.unwrap_or_else(|| "★".into()))
        };
        let location = match self.const_str(b[3])?.as_deref() {
            Some("location.belowbar") => Location::BelowBar,
            Some("location.top") => Location::Top,
            Some("location.bottom") => Location::Bottom,
            Some("location.absolute") => Location::Absolute,
            _ => Location::AboveBar,
        };
        let (mut color, mut palette) = self.color_arg(b[4], opentern_series::color::tv::BLUE.0)?;
        if let Some(t) = b[9] {
            let t = self.expr(t)?;
            if let Some(tv) = self.const_or_default(&t) {
                palette = palette.iter().map(|c| value::with_transp(*c, tv.f())).collect();
            }
            if let Some(c) = color.take() {
                let f = natives::lookup("color.new").expect("color.new");
                color = Some(self.nat(f, vec![c, t]));
            }
        }
        let offset = match b[5] {
            Some(e) => {
                let e = self.expr(e)?;
                self.offset_of(&e)?
            }
            None => Offset::Fixed(0),
        };
        let text = self.const_str(b[6])?.unwrap_or_default();
        let size = Self::shape_size(self.const_str(b[8])?.as_deref());
        let visible = !self.display_none(args)?;
        let label = if kind == ShapeKind::Char {
            if text.is_empty() { ch } else { format!("{ch}\n{text}") }
        } else {
            text
        };
        self.plots.push(PlotDef {
            title: self.plot_title(title),
            style: PlotStyle::Shape,
            palette,
            width: 1,
            offset,
            visible,
            base: 0.0,
            shape: Some((kind, location, label, size)),
        });
        Ok(S::Plot {
            i: (self.plots.len() - 1) as u16,
            val,
            color,
            how: if location == Location::Absolute {
                PlotHow::Value
            } else {
                PlotHow::Flag
            },
        })
    }

    fn plotarrow(&mut self, args: &[Arg]) -> R<S> {
        let b = bind(
            "plotarrow",
            &[
                "series",
                "title",
                "colorup",
                "colordown",
                "offset",
                "minheight",
                "maxheight",
            ],
            args,
            self.line,
        )?;
        let Some(series) = b[0] else {
            return Err(self.err("plotarrow() needs a series."));
        };
        let val = self.expr(series)?;
        let title = self.const_str(b[1])?;
        let title = self.plot_title(title);
        let (_, up) = self.color_arg(b[2], value::opaque(0x4CAF50))?;
        let (_, down) = self.color_arg(b[3], value::opaque(0xFF5252))?;
        let offset = match b[4] {
            Some(e) => {
                let e = self.expr(e)?;
                self.offset_of(&e)?
            }
            None => Offset::Fixed(0),
        };
        for (suffix, kind, loc, pal) in [
            ("up", ShapeKind::ArrowUp, Location::BelowBar, up),
            ("down", ShapeKind::ArrowDown, Location::AboveBar, down),
        ] {
            self.plots.push(PlotDef {
                title: format!("{title} {suffix}"),
                style: PlotStyle::Shape,
                palette: pal,
                width: 1,
                offset,
                visible: true,
                base: 0.0,
                shape: Some((kind, loc, String::new(), 13.0)),
            });
        }
        Ok(S::Plot {
            i: (self.plots.len() - 2) as u16,
            val,
            color: None,
            how: PlotHow::Arrow,
        })
    }

    fn plotcandle(&mut self, callee: &str, args: &[Arg]) -> R<S> {
        let b = bind(
            callee,
            &[
                "open",
                "high",
                "low",
                "close",
                "title",
                "color",
                "wickcolor",
                "bordercolor",
            ],
            args,
            self.line,
        )?;
        let get = |k: usize, cx: &mut Cx| -> R<E> {
            match b[k] {
                Some(e) => cx.expr(e),
                None => Err(cx.err(format!("{callee}() needs open, high, low and close."))),
            }
        };
        let o = get(0, self)?;
        let h = get(1, self)?;
        let l = get(2, self)?;
        let c = get(3, self)?;
        let title = self.const_str(b[4])?;
        let title = self.plot_title(title);
        let (color, palette) = self.color_arg(b[5], opentern_series::color::tv::BLUE.0)?;
        let style = if callee == "plotcandle" {
            PlotStyle::Candle
        } else {
            PlotStyle::OhlcBar
        };
        let visible = !self.display_none(args)?;
        for (k, part) in ["", " high", " low", " close"].iter().enumerate() {
            self.plots.push(PlotDef {
                title: format!("{title}{part}"),
                style: if k == 0 { style } else { PlotStyle::Hidden },
                palette: if k == 0 { palette.clone() } else { vec![0] },
                width: 1,
                offset: Offset::Fixed(0),
                visible,
                base: 0.0,
                shape: None,
            });
        }
        Ok(S::Candle {
            i: (self.plots.len() - 4) as u16,
            o,
            h,
            l,
            c,
            color,
        })
    }

    fn hline(&mut self, args: &[Arg]) -> R<()> {
        let b = bind(
            "hline",
            &["price", "title", "color", "linestyle", "linewidth"],
            args,
            self.line,
        )?;
        let price = self
            .const_num(b[0])?
            .ok_or_else(|| self.err("hline() needs a constant price (or an input)."))?;
        let title = self.const_str(b[1])?.unwrap_or_else(|| "Price Line".into());
        let color = match b[2] {
            Some(e) => {
                let e = self.expr(e)?;
                self.const_or_default(&e)
                    .and_then(|v| v.color())
                    .unwrap_or(value::opaque(0x787B86))
            }
            None => value::opaque(0x787B86),
        };
        let style = match self.const_str(b[3])?.as_deref() {
            Some("hline.style_solid") => LineStyle::Solid,
            Some("hline.style_dotted") => LineStyle::Dotted,
            _ => LineStyle::Dashed,
        };
        let width = self.const_num(b[4])?.unwrap_or(1.0).clamp(1.0, 10.0) as u8;
        let visible = !self.display_none(args)?;
        self.hlines.push(HLineSpec {
            title: leak(title),
            value: price,
            color: Rgba(color),
            style,
            width,
            visible,
        });
        Ok(())
    }

    fn fill(&mut self, args: &[Arg]) -> R<Option<S>> {
        let b = bind(
            "fill",
            &[
                "hline1",
                "hline2",
                "color",
                "title",
                "fillgaps",
                "top_value",
                "bottom_value",
                "top_color",
                "bottom_color",
                "transp",
            ],
            args,
            self.line,
        )?;
        let handle = |cx: &mut Cx, e: Option<&Expr>| -> R<Bind> {
            match e {
                Some(Expr::Name(n)) => match cx.lookup(n) {
                    Some(b @ (Bind::Plot(_) | Bind::HLine(_))) => Ok(b),
                    _ => Err(cx.err(format!("fill() takes plots or hlines; \"{n}\" isn't one."))),
                },
                _ => Err(cx.err("fill() takes two plots or two hlines, by name (p1 = plot(…)).")),
            }
        };
        let a = handle(self, b[0])?;
        let c = handle(self, b[1])?;
        let between = match (a, c) {
            (Bind::Plot(x), Bind::Plot(y)) => FillBetween::Plots(x as usize, y as usize),
            (Bind::HLine(x), Bind::HLine(y)) => FillBetween::HLines(x as usize, y as usize),
            _ => return Err(self.err("fill() needs two plots or two hlines, not one of each.")),
        };
        let default = value::with_transp(value::opaque(0x2196F3), 90.0);
        // Gradient fills use the top colour.
        let colour_arg = b[2].or(b[7]);
        let (mut color, mut palette) = self.color_arg(colour_arg, default)?;
        if let Some(t) = b[9] {
            let t = self.expr(t)?;
            if let Some(tv) = self.const_or_default(&t) {
                palette = palette.iter().map(|c| value::with_transp(*c, tv.f())).collect();
            }
            if let Some(c) = color.take() {
                let f = natives::lookup("color.new").expect("color.new");
                color = Some(self.nat(f, vec![c, t]));
            }
        }
        let title = self.const_str(b[3])?.unwrap_or_else(|| "Background".into());
        let visible = !self.display_none(args)?;
        self.fills.push(FillDef {
            title,
            between,
            palette,
            visible,
        });
        let i = (self.fills.len() - 1) as u16;
        Ok(color.map(|color| S::FillColor { i, color }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("RSI Length"), "rsi_length");
        assert_eq!(slug("  %K smoothing "), "k_smoothing");
        assert_eq!(slug("…"), "input");
    }

    #[test]
    fn constants() {
        assert!(matches!(constant("color.red"), Some(V::Color(0xFF5252FF))));
        assert!(matches!(constant("plot.style_line"), Some(V::Str(_))));
        assert!(matches!(constant("dayofweek.monday"), Some(V::Int(2))));
        assert!(constant("foo.bar").is_none());
    }
}

//! Resolves the syntax tree into an executable tree: names become slot
//! numbers in a frame (the script's root, or one frame per user-function
//! call site, as Pine keeps a separate history per call site), constants
//! are folded, each stateful built-in call (`ta.*`, `math.sum`, …) gets a
//! state slot, and the declarations (`indicator`, `input.*`, `plot`, …)
//! become the study's [`Meta`].

use crate::ast::{self, Arg, Expr, Param, Persist, Stmt};
use crate::natives::{self, NatDef};
use crate::value::{self, V};
use crate::{Env, Error};
use opentern_series::color::Rgba;
use opentern_series::study::{
    Category, FillBetween, FillSpec, Format, HLineSpec, InputKind, InputSpec, LineStyle, Location, Meta, Offset,
    Placement, PlotSpec, PlotStyle, ShapeKind, ShapeSpec, Value,
};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
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
        user: Vec<MethodCand>,
        /// The receiver's type when it's plain from the source (for a
        /// receiver that's `na`).
        hint: Option<Arc<str>>,
    },
    /// A loop used as a value.
    Loop(Box<S>),
    /// `request.security`: an expression evaluated on another timeframe.
    Security(Box<Sec>),
}

/// A user method a call may pick.
#[derive(Clone, Debug)]
pub(crate) struct MethodCand {
    pub f: u16,
    pub this: This,
    pub site: u16,
    /// Default arguments, compiled for this call.
    pub defaults: Vec<Option<E>>,
}

/// What a method's first parameter takes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum This {
    Any,
    /// A user type.
    Udt(u16),
    /// A built-in type ("array", "string", "float", …), with an array's
    /// element type.
    Of(Arc<str>, Option<Arc<str>>),
}

/// A `request.security` call.
#[derive(Clone, Debug)]
pub(crate) struct Sec {
    /// The symbol: `None` for this chart's (`syminfo.tickerid`).
    pub symbol: Option<E>,
    pub tf: E,
    pub body: Body,
    pub gaps: E,
    pub lookahead: E,
    /// Index into the current frame's security states.
    pub site: u16,
    pub lower: bool,
}

/// What a request evaluates: everything its expression depends on,
/// recomputed on the requested bars.
#[derive(Clone, Debug)]
pub(crate) enum Body {
    Here(Box<SecBody>),
    /// Compiled after the rest of the script (`Program::late`), as it
    /// depends on statements below the request.
    Late(u16),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SecBody {
    /// The script's variables the expression uses, then the expression,
    /// then what changes those variables below it.
    pub prelude: Block,
    pub expr: Option<E>,
    pub post: Block,
    /// It contains other requests, which run on this one's bars.
    pub nested: bool,
    /// The frame it runs in.
    pub layout: Layout,
    pub bar_hist: usize,
}

impl Sec {
    pub fn body<'a>(&'a self, p: &'a Program) -> &'a SecBody {
        match &self.body {
            Body::Here(b) => b,
            Body::Late(k) => &p.late[*k as usize],
        }
    }
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
    /// `bgcolor`/`barcolor` (true) with a negative offset: this many bars back.
    Past(bool, usize, E),
    Many(Vec<S>),
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
    /// `varip` slots: they keep what a bar's earlier ticks left in them.
    pub varips: Vec<u16>,
}

#[derive(Clone, Debug)]
pub(crate) struct Func {
    pub name: String,
    pub params: Vec<String>,
    pub body: Block,
    pub layout: Layout,
}

#[derive(Clone, Debug)]
pub(crate) struct TypeDef {
    pub name: String,
    pub fields: Vec<String>,
    /// Declared types, where written.
    pub ftypes: Vec<Option<String>>,
    /// Fields that keep their value between order-fill recalculations.
    pub varip_fields: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Conv {
    Int,
    Float,
    Bool,
    Str,
    Source,
    Color,
    /// A date, as text in the exchange's timezone.
    Time,
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
    /// The hlines' prices and colours, in terms of the inputs.
    pub levels: Vec<(E, Option<E>)>,
    pub fills: Vec<FillDef>,
    pub alertconditions: Vec<AlertCondDef>,
    pub funcs: Vec<Func>,
    pub types: Vec<TypeDef>,
    pub root: Block,
    pub layout: Layout,
    pub bar_hist: usize,
    pub env: Arc<Env>,
    pub limits: [usize; 6],
    /// `indicator(timeframe = …)`: the script runs on these bars.
    pub timeframe: Option<String>,
    pub timeframe_gaps: bool,
    /// It makes requests.
    pub requests: bool,
    /// Request bodies compiled last (see [`Body::Late`]).
    pub late: Vec<SecBody>,
}

#[derive(Clone, Copy, Debug)]
enum Bind {
    Slot { depth: usize, slot: u16 },
    Plot(u16),
    HLine(u16),
}

/// A variable: its frame (numbered as compiled) and slot.
type Var = (u32, u16);
type Scope = (usize, HashMap<String, Bind>);
/// An argument, and the scopes it's read in.
type CallArg = (Rc<Expr>, Rc<Vec<Scope>>);

/// Where a variable that's never reassigned gets its value, so that
/// `request.security` can compute it on other bars.
#[derive(Clone)]
enum Def {
    /// `x = value`
    Value(Rc<Expr>, usize),
    /// One of `[x, y] = value`
    Tuple(Rc<Expr>, usize, Rc<Vec<Option<Var>>>),
    /// A parameter: the argument, and the scopes it's read in.
    Arg(Rc<Expr>, usize, Rc<Vec<Scope>>),
    /// A parameter of a function that's being compiled by itself.
    Param,
}

/// A `request.security` expression being compiled.
struct SecCx {
    /// The depth of its frame.
    depth: usize,
    /// The script's variables recomputed in its frame, and where.
    hoisted: HashMap<Var, u16>,
    prelude: Block,
    nested: bool,
}

/// A function being compiled for one call: its parameters, the call's
/// arguments and its body.
type Calling = (Rc<Vec<Param>>, Vec<Rc<Expr>>, Rc<ast::Block>);

/// A request to compile once the whole script has been, and where it is.
struct LateRequest {
    expr: Expr,
    before: Vec<Stmt>,
    after: Vec<Stmt>,
    scopes: Vec<Scope>,
    uids: Vec<u32>,
    lib: Option<String>,
    mutable: Rc<HashSet<(usize, String)>>,
    calls: Vec<Calling>,
    line: usize,
    at: usize,
}

/// A function's source, to compile it again for one call.
#[derive(Clone)]
struct FnInfo {
    params: Rc<Vec<Param>>,
    body: Rc<ast::Block>,
    line: usize,
    lib: Option<String>,
    mutable: Rc<HashSet<(usize, String)>>,
    flags: Flags,
}

#[derive(Clone, Copy, Default)]
struct Flags {
    /// Reads the script's own variables, so a `request.security` needs its
    /// own copy of the function that reads them on its bars.
    globals: bool,
    /// Requests something that depends on its arguments, so each call
    /// needs its own copy.
    per_call: bool,
    /// Contains a request.
    requests: bool,
}

/// The most copies of functions made for `request.security`.
const MAX_INSTANCES: usize = 4000;

/// The variables some `:=` changes, by declaration (line, name).
fn reassigned(b: &ast::Block) -> HashSet<(usize, String)> {
    type Names = Vec<HashMap<String, usize>>;
    fn declare(s: &mut Names, n: &str, line: usize) {
        if let Some(t) = s.last_mut() {
            t.insert(n.to_string(), line);
        }
    }
    fn block(b: &ast::Block, s: &mut Names, out: &mut HashSet<(usize, String)>) {
        s.push(HashMap::new());
        for st in b {
            stmt(st, s, out);
        }
        s.pop();
    }
    fn stmt(st: &Stmt, s: &mut Names, out: &mut HashSet<(usize, String)>) {
        match st {
            Stmt::Decl { names, value, line, .. } => {
                expr(value, s, out);
                for n in names {
                    declare(s, n, *line);
                }
            }
            Stmt::Assign { name, value, .. } => {
                expr(value, s, out);
                if let Some(line) = s.iter().rev().find_map(|t| t.get(name)) {
                    out.insert((*line, name.clone()));
                }
            }
            Stmt::AssignField { obj, value, .. } => {
                expr(obj, s, out);
                expr(value, s, out);
            }
            Stmt::Expr(e, _) => expr(e, s, out),
            Stmt::For {
                var,
                from,
                to,
                step,
                body,
                line,
            } => {
                for e in [Some(from), Some(to), step.as_ref()].into_iter().flatten() {
                    expr(e, s, out);
                }
                s.push(HashMap::from([(var.clone(), *line)]));
                block(body, s, out);
                s.pop();
            }
            Stmt::ForIn { over, body, .. } => {
                expr(over, s, out);
                block(body, s, out);
            }
            Stmt::While { cond, body, .. } => {
                expr(cond, s, out);
                block(body, s, out);
            }
            Stmt::Func { params, body, line, .. } => {
                s.push(params.iter().map(|p| (p.0.clone(), *line)).collect());
                block(body, s, out);
                s.pop();
            }
            _ => {}
        }
    }
    fn expr(e: &Expr, s: &mut Names, out: &mut HashSet<(usize, String)>) {
        match e {
            Expr::Call { args, .. } => args.iter().for_each(|a| expr(&a.value, s, out)),
            Expr::MethodCall { recv, args, .. } => {
                expr(recv, s, out);
                args.iter().for_each(|a| expr(&a.value, s, out));
            }
            Expr::Index(a, b) | Expr::Binary(_, a, b) => {
                expr(a, s, out);
                expr(b, s, out);
            }
            Expr::Dot(a, _) | Expr::Unary(_, a) => expr(a, s, out),
            Expr::Ternary(a, b, c) => [a, b, c].into_iter().for_each(|x| expr(x, s, out)),
            Expr::If(branches, o) => {
                for (c, b) in branches {
                    expr(c, s, out);
                    block(b, s, out);
                }
                if let Some(b) = o {
                    block(b, s, out);
                }
            }
            Expr::Switch(subj, cases) => {
                if let Some(x) = subj {
                    expr(x, s, out);
                }
                for (c, b) in cases {
                    if let Some(c) = c {
                        expr(c, s, out);
                    }
                    block(b, s, out);
                }
            }
            Expr::Loop(l) => stmt(l, s, out),
            Expr::Tuple(xs) => xs.iter().for_each(|x| expr(x, s, out)),
            _ => {}
        }
    }
    let mut out = HashSet::new();
    block(b, &mut Vec::new(), &mut out);
    out
}

struct Cx {
    version: u8,
    env: Arc<Env>,
    /// Scopes, innermost last, each with the frame depth it belongs to.
    scopes: Vec<Scope>,
    frames: Vec<Layout>,
    /// Each frame's number.
    uids: Vec<u32>,
    next_uid: u32,
    /// Local-block nesting in the current frame (plots must be global).
    local: usize,
    funcs: Vec<Func>,
    /// The source of the functions in `funcs`.
    fn_info: Vec<FnInfo>,
    instances: usize,
    /// The script, and the top-level statement being compiled.
    ast: Rc<ast::Block>,
    root_at: usize,
    slices: Option<Rc<crate::slice::Program>>,
    /// Where the top-level variables are, by declaration (line, name).
    root_slots: HashMap<(usize, String), u16>,
    /// A request needs a variable that's reassigned.
    wants_slice: bool,
    late: Vec<LateRequest>,
    /// The functions being compiled for one call: their arguments, bodies.
    calls: Vec<Calling>,
    /// What the functions being compiled do, innermost last.
    flags: Vec<Flags>,
    /// Variables this script (or library) reassigns.
    mutable: Rc<HashSet<(usize, String)>>,
    fnames: HashMap<String, u16>,
    /// Every function declared under a name, in order (overloads).
    overloads: HashMap<String, Vec<u16>>,
    /// Methods by name.
    methods: HashMap<String, Vec<u16>>,
    types: Vec<TypeDef>,
    /// The types' field defaults.
    type_defaults: Vec<Vec<Option<Rc<Expr>>>>,
    /// What variables made only of constants and inputs are, in terms of
    /// those.
    known: HashMap<Var, E>,
    /// Types of variables, for overloads and methods.
    slot_types: HashMap<Var, String>,
    /// Colour constants in a variable's defining expression, so a plot
    /// coloured by that variable gets the palette (`color = zoneColor`).
    palettes: HashMap<Var, Vec<u32>>,
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
    levels: Vec<(E, Option<E>)>,
    fills: Vec<FillDef>,
    alertconditions: Vec<AlertCondDef>,
    bar_hist: usize,
    limits: [usize; 6],
    /// Bar history needed inside the `request.security` being compiled.
    sec_depth: usize,
    sec_bar_hist: usize,
    defs: HashMap<Var, Def>,
    /// The `request.security` expressions being compiled.
    secs: Vec<SecCx>,
    /// Definitions being recomputed (to catch cycles).
    inlining: Vec<Var>,
    /// Compiling source for a second time: its inputs exist already.
    again: usize,
    input_cache: HashMap<(usize, String), E>,
    /// Constants of the imported libraries, as `alias.NAME`.
    lib_consts: HashMap<String, V>,
    timeframe: Option<String>,
    timeframe_gaps: bool,
    requests: bool,
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
        uids: vec![0],
        next_uid: 1,
        local: 0,
        funcs: Vec::new(),
        fn_info: Vec::new(),
        instances: 0,
        ast: Rc::new(prog.clone()),
        root_at: 0,
        slices: None,
        root_slots: HashMap::new(),
        wants_slice: false,
        late: Vec::new(),
        calls: Vec::new(),
        flags: Vec::new(),
        mutable: Rc::new(reassigned(prog)),
        fnames: HashMap::new(),
        overloads: HashMap::new(),
        methods: HashMap::new(),
        types: Vec::new(),
        type_defaults: Vec::new(),
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
        levels: Vec::new(),
        fills: Vec::new(),
        alertconditions: Vec::new(),
        bar_hist: 1,
        limits: [50, 50, 50, 40, 50, 50],
        sec_depth: 0,
        sec_bar_hist: 1,
        defs: HashMap::new(),
        secs: Vec::new(),
        inlining: Vec::new(),
        again: 0,
        input_cache: HashMap::new(),
        lib_consts: HashMap::new(),
        timeframe: None,
        timeframe_gaps: true,
        requests: false,
        enums: HashMap::new(),
        lib_prefix: None,
        importing: Vec::new(),
    };
    let mut root = Vec::new();
    for (k, s) in prog.iter().enumerate() {
        cx.root_at = k;
        root.extend(cx.block(std::slice::from_ref(s))?);
    }
    let Some(kind) = cx.kind else {
        return Err(Error::at(
            1,
            "Start the script with indicator(\"Name\") or strategy(\"Name\").",
        ));
    };
    let layout = cx.frames.remove(0);
    let late = cx.compile_late()?;
    if kind == Kind::Strategy {
        // The broker follows this version's rules (margin, exit levels).
        cx.props.insert("pine_version".into(), serde_json::json!(version));
    }
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
        levels: cx.levels,
        fills: cx.fills,
        alertconditions: cx.alertconditions,
        funcs: cx.funcs,
        types: cx.types,
        root,
        layout,
        bar_hist: cx.bar_hist,
        env: cx.env,
        limits: cx.limits,
        timeframe: cx.timeframe,
        timeframe_gaps: cx.timeframe_gaps,
        requests: cx.requests,
        late,
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

/// [`constant`] in a Pine version: v6 changed a few colours.
pub(crate) fn constant_in(version: u8, name: &str) -> Option<V> {
    if version >= 6 {
        let rgb = match name {
            "color.red" => 0xF23645,
            "color.teal" => 0x089981,
            "color.yellow" => 0xFDD835,
            _ => return constant(name),
        };
        return Some(V::Color(value::opaque(rgb)));
    }
    constant(name)
}

/// The v5 name of a built-in from v4 or before (`sma` → `ta.sma`).
fn renamed(name: &str) -> Option<String> {
    if name.contains('.') {
        return None;
    }
    let full = match name {
        "n" => "bar_index".to_string(),
        "tickerid" | "ticker" => format!("syminfo.{name}"),
        "period" | "interval" | "isintraday" | "isdaily" | "isweekly" | "ismonthly" | "isdwm" => match name {
            "interval" => "timeframe.multiplier".into(),
            _ => format!("timeframe.{name}"),
        },
        "heikinashi" => "ticker.heikinashi".into(),
        _ => ["math", "ta", "str"]
            .iter()
            .map(|ns| format!("{ns}.{name}"))
            .find(|f| natives::lookup(f).is_some())?,
    };
    Some(full)
}

/// Constants v3 had without a namespace (`red`, `histogram`, `integer`).
fn bare_constant(name: &str) -> Option<V> {
    if let Some(c) = constant(&format!("color.{name}")).filter(|v| matches!(v, V::Color(_))) {
        return Some(c);
    }
    Some(match name {
        "line" | "stepline" | "histogram" | "cross" | "area" | "columns" | "circles" | "linebr" | "areabr" => {
            V::str(format!("plot.style_{name}"))
        }
        "dashed" | "dotted" | "solid" => V::str(format!("hline.style_{name}")),
        "integer" | "float" | "bool" | "source" | "resolution" | "string" | "symbol" | "session" => {
            V::str(format!("input.{name}"))
        }
        _ => return None,
    })
}

/// `array<float>` → `array`; user and primitive types as written.
fn base_type(t: &str) -> String {
    t.split('<').next().unwrap_or(t).to_string()
}

fn has_source(e: &E) -> bool {
    match e {
        E::InSrc(_) => true,
        E::Neg(x) | E::Not(x) => has_source(x),
        E::Bin(_, a, b) | E::And(a, b, _) | E::Or(a, b, _) => has_source(a) || has_source(b),
        E::Tern(a, b, c) => has_source(a) || has_source(b) || has_source(c),
        E::Nat { args, .. } => args.iter().any(has_source),
        _ => false,
    }
}

fn const_of(e: &E) -> Option<V> {
    match e {
        E::K(v) => Some(v.clone()),
        _ => None,
    }
}

/// Whether an expression could be a constant: no calls but pure ones.
fn constant_like(e: &Expr) -> bool {
    match e {
        Expr::Call { callee, args } => {
            natives::lookup(callee).is_some_and(|d| d.pure) && args.iter().all(|a| constant_like(&a.value))
        }
        Expr::Unary(_, a) => constant_like(a),
        Expr::Binary(_, a, b) => constant_like(a) && constant_like(b),
        Expr::Ternary(a, b, c) => [a, b, c].into_iter().all(|x| constant_like(x)),
        Expr::Int(_) | Expr::Float(_) | Expr::Str(_) | Expr::Bool(_) | Expr::Color(_) | Expr::Na | Expr::Name(_) => {
            true
        }
        _ => false,
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

    fn uid(&self) -> u32 {
        self.uids[self.depth()]
    }

    fn push_frame(&mut self) {
        self.frames.push(Layout::default());
        self.uids.push(self.next_uid);
        self.next_uid += 1;
    }

    fn pop_frame(&mut self) -> Layout {
        self.uids.pop();
        self.frames.pop().unwrap_or_default()
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
        self.lookup_at(name).map(|(b, _)| b)
    }

    /// What `name` is, and the scope it's declared in.
    fn lookup_at(&self, name: &str) -> Option<(Bind, usize)> {
        let k = self.scopes.iter().rposition(|(_, s)| s.contains_key(name))?;
        Some((self.scopes[k].1[name], k))
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

    fn block(&mut self, b: &[Stmt]) -> R<Block> {
        let mut out = Vec::with_capacity(b.len());
        for s in b {
            let line = match s {
                Stmt::Decl { line, .. }
                | Stmt::Assign { line, .. }
                | Stmt::AssignField { line, .. }
                | Stmt::Expr(_, line)
                | Stmt::For { line, .. }
                | Stmt::ForIn { line, .. }
                | Stmt::While { line, .. } => *line,
                _ => self.line,
            };
            if let Some(st) = self.stmt(s)? {
                out.push((line as u32, st));
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
                        Ok(Some(S::Set(slot, op, e)))
                    }
                    _ => Err(self.err(format!("\"{name}\" isn't declared yet. Declare it with \"=\" first."))),
                }
            }
            Stmt::AssignField {
                obj,
                field,
                op,
                value,
                line,
            } => {
                self.line = *line;
                let o = self.expr(obj)?;
                self.check_field(field)?;
                let e = self.expr(value)?;
                let op = if *op == ":=" { None } else { Bin::of(op) };
                Ok(Some(S::SetField(o, Arc::from(field.as_str()), op, e)))
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
                let kind = self.infer(over);
                let over = self.expr(over)?;
                self.push_scope();
                let d = self.depth();
                let idx = idx.as_ref().map(|n| {
                    let s = self.new_slot();
                    self.declare(n, Bind::Slot { depth: d, slot: s });
                    s
                });
                let slot = self.new_slot();
                if let Some(t) = kind.and_then(|k| self.method_type(&k, "get")) {
                    let var = (self.uid(), slot);
                    self.slot_types.insert(var, t);
                }
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
                for field in fields {
                    if let Some(e) = &field.default {
                        self.expr(e)?;
                    }
                }
                self.types.push(TypeDef {
                    name: name.clone(),
                    fields: fields.iter().map(|field| field.name.clone()).collect(),
                    ftypes: fields.iter().map(|field| field.ty.clone()).collect(),
                    varip_fields: fields
                        .iter()
                        .enumerate()
                        .filter_map(|(i, field)| field.varip.then_some(i))
                        .collect(),
                });
                self.type_defaults
                    .push(fields.iter().map(|field| field.default.clone().map(Rc::new)).collect());
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
    /// scripts folder, found by their `library("Name")` title, or ones that
    /// ship with the app (`TradingView/ta/9`), matched by the whole path.
    fn import(&mut self, path: &str, alias: &str) -> R<()> {
        let name = path.split('/').nth(1).unwrap_or(path).to_string();
        let Some(src) = crate::libs::bundled(path).map(str::to_string).or_else(|| {
            self.env
                .libraries
                .iter()
                .find(|(t, _)| *t == name)
                .map(|(_, s)| s.clone())
        }) else {
            let publisher_name = path.rsplit_once('/').map_or(path, |(pn, _)| pn);
            if let Some(have) = crate::libs::bundled_versions(publisher_name).first() {
                return Err(self.err(format!(
                    "Opentern includes {have}, not {path}. Change the import to {have}."
                )));
            }
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
        let saved_mutable = std::mem::replace(&mut self.mutable, Rc::new(reassigned(&ast)));
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
                    // Its constants, for its functions and (exported) for us.
                    Stmt::Decl {
                        persist: Persist::No,
                        names,
                        value,
                        line,
                        ..
                    } if names.len() == 1 && !self.mutable.contains(&(*line, names[0].clone())) => {
                        self.line = *line;
                        if constant_like(value)
                            && let Ok(e) = self.expr(value)
                            && let Some(v) = self.const_value(&e)
                        {
                            self.lib_consts.insert(format!("{alias}.{}", names[0]), v);
                        }
                    }
                    _ => {}
                }
            }
            Ok(())
        })();
        self.lib_prefix = saved;
        self.mutable = saved_mutable;
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
        // Other outputs give nothing to keep.
        if names.len() == 1
            && let Expr::Call { callee, args } = value
            && matches!(
                callee.as_str(),
                "plotshape" | "plotchar" | "plotarrow" | "plotcandle" | "plotbar" | "fill" | "bgcolor" | "barcolor"
            )
            && !self.fnames.contains_key(callee)
        {
            let st = self.output_call(callee, args)?;
            return Ok(st.flatten().unwrap_or(S::Expr(E::K(V::Na))));
        }
        let mut e = self.expr(value)?;
        let d = self.depth();
        let uid = self.uid();
        let line = self.line;
        // A `var` that's a constant and never changes is one too.
        let constant = persist == Persist::Var && names.len() == 1 && self.const_value(&e).is_some();
        let fixed =
            |cx: &Cx, n: &str| (persist == Persist::No || constant) && !cx.mutable.contains(&(line, n.to_string()));
        if names.len() == 1 {
            let ty = declared.map(str::to_string).or_else(|| self.infer(value));
            // `float x = 5` is 5.0 (it matters to v5's division).
            if declared == Some("float")
                && let Some(V::Int(i)) = self.const_value(&e)
            {
                e = E::K(V::Float(i as f64));
            }
            let known = self.resolve(&e);
            let slot = self.new_slot();
            let var = (uid, slot);
            if d == 0 && self.local == 0 {
                self.root_slots.insert((line, names[0].clone()), slot);
            }
            if fixed(self, &names[0]) {
                if let Some(k) = known {
                    self.known.insert(var, k);
                }
                self.defs.insert(var, Def::Value(Rc::new(value.clone()), line));
            }
            if let Some(t) = ty {
                self.slot_types.insert(var, t);
            }
            let mut pal = Vec::new();
            self.palette(&e, &mut pal);
            if !pal.is_empty() {
                self.palettes.insert(var, pal);
            }
            self.declare(&names[0], Bind::Slot { depth: d, slot });
            if persist == Persist::Varip {
                self.frame().varips.push(slot);
            }
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
                    if d == 0 && self.local == 0 {
                        self.root_slots.insert((line, n.clone()), s);
                    }
                    s
                })
            })
            .collect();
        let vars: Rc<Vec<Option<Var>>> = Rc::new(slots.iter().map(|s| s.map(|s| (uid, s))).collect());
        let value = Rc::new(value.clone());
        for (n, v) in names.iter().zip(vars.iter()) {
            if let Some(v) = v
                && fixed(self, n)
            {
                self.defs.insert(*v, Def::Tuple(value.clone(), line, vars.clone()));
            }
        }
        Ok(S::LetTuple(slots, e))
    }

    /// The type of field `f` of a value of type `t`.
    fn field_type(&self, t: &str, f: &str) -> Option<String> {
        if t == "chart.point" {
            return Some(if f == "price" { "float" } else { "int" }.into());
        }
        let d = &self.types[self.type_id(t)?];
        let k = d.fields.iter().position(|x| x == f)?;
        let ty = d.ftypes[k].clone()?;
        // A library's own types are known by its alias here.
        if let Some((lib, _)) = d.name.rsplit_once('.') {
            let full = format!("{lib}.{ty}");
            if self.types.iter().any(|x| x.name == full) {
                return Some(full);
            }
        }
        Some(ty)
    }

    /// What a built-in method of a value of type `t` returns.
    fn method_type(&self, t: &str, m: &str) -> Option<String> {
        let elem = || Some(t.strip_prefix("array<")?.strip_suffix('>')?.to_string());
        match m {
            "copy" | "concat" | "slice" => Some(t.to_string()),
            "get" | "first" | "last" | "pop" | "shift" | "remove" if t.starts_with("array<") => elem(),
            "get" | "remove" | "put" if t.starts_with("map<") => {
                let v = t.strip_prefix("map<")?.strip_suffix('>')?.split_once(',')?.1;
                Some(v.trim().to_string())
            }
            "get" if t.starts_with("matrix<") => Some(t.strip_prefix("matrix<")?.strip_suffix('>')?.to_string()),
            "row" | "col" if t.starts_with("matrix<") => {
                Some(format!("array<{}>", t.strip_prefix("matrix<")?.strip_suffix('>')?))
            }
            "size" | "indexof" | "rows" | "columns" => Some("int".into()),
            _ => None,
        }
    }

    /// The type of an expression, when it's plain from the source.
    fn infer(&self, e: &Expr) -> Option<String> {
        let t = match e {
            Expr::Int(_) => "int",
            Expr::Float(_) => "float",
            Expr::Str(_) => "string",
            Expr::Bool(_) => "bool",
            Expr::Color(_) => "color",
            Expr::Na => "na",
            Expr::Unary("not", _) => "bool",
            Expr::Unary(_, x) => return self.infer(x),
            Expr::Binary("==" | "!=" | "<" | ">" | "<=" | ">=" | "and" | "or", ..) => "bool",
            Expr::Binary(op, a, b) => match (self.infer(a)?.as_str(), self.infer(b)?.as_str()) {
                ("string", _) | (_, "string") if *op == "+" => "string",
                ("int", "int") if *op != "/" => "int",
                ("int" | "float", "int" | "float") => "float",
                _ => return None,
            },
            Expr::Ternary(_, a, b) => match (self.infer(a)?, self.infer(b)?) {
                (a, b) if a == b || b == "na" => return Some(a),
                (a, b) if a == "na" => return Some(b),
                (a, b) if (a == "int" || a == "float") && (b == "int" || b == "float") => "float",
                _ => return None,
            },
            Expr::Index(x, _) => return self.infer(x),
            Expr::Name(n) => {
                // obj.field.field
                if let Some((obj, f)) = n.rsplit_once('.')
                    && self.lookup(n.split('.').next().unwrap_or(n)).is_some()
                {
                    let t = self.infer(&Expr::Name(obj.to_string()))?;
                    return self.field_type(&t, f);
                }
                if let Some(b) = self.lookup(n) {
                    return match b {
                        Bind::Slot { depth, slot } => self.slot_types.get(&(self.uids[depth], slot)).cloned(),
                        _ => None,
                    };
                }
                match n.as_str() {
                    "time" | "time_close" | "bar_index" | "last_bar_index" | "timenow" | "year" | "month"
                    | "dayofmonth" | "dayofweek" | "hour" | "minute" | "second" => "int",
                    n if Field::by_name(n).is_some() => "float",
                    n if n.starts_with("barstate.") || n.starts_with("timeframe.is") || n.starts_with("session.is") => {
                        "bool"
                    }
                    "syminfo.mintick" | "syminfo.pointvalue" => "float",
                    "timeframe.multiplier" => "int",
                    n if n.starts_with("syminfo.") || n.starts_with("timeframe.") => "string",
                    n => match constant_in(self.version, n) {
                        Some(V::Color(_)) => "color",
                        Some(V::Float(_)) => "float",
                        Some(V::Int(_)) => "int",
                        Some(V::Str(_)) => "string",
                        _ => return None,
                    },
                }
            }
            Expr::Call { callee, args } => {
                let (name, generic) = match callee.split_once('<') {
                    Some((n, g)) => (n, g.strip_suffix('>')),
                    None => (callee.as_str(), None),
                };
                if let Some(t) = name.strip_suffix(".new")
                    && let Some(i) = self.type_id(t)
                {
                    return Some(self.types[i].name.clone());
                }
                if let Some((obj, m)) = name.rsplit_once('.')
                    && natives::lookup(name).is_none()
                    && self.lookup(obj.split('.').next().unwrap_or(obj)).is_some()
                {
                    let t = self.infer(&Expr::Name(obj.to_string()))?;
                    return self.method_type(&t, m);
                }
                if let Some((fam @ ("array" | "map" | "matrix"), m)) = name.split_once('.')
                    && let Some(t) = args.first().and_then(|a| self.infer(&a.value))
                    && t.starts_with(fam)
                    && let Some(t) = self.method_type(&t, m)
                {
                    return Some(t);
                }
                if self
                    .fnames
                    .contains_key(&self.in_lib(name, |cx, n| cx.fnames.contains_key(n)))
                {
                    return None;
                }
                match name {
                    "array.new" => return Some(format!("array<{}>", generic?)),
                    "matrix.new" | "map.new" => return Some(format!("{}<{}>", name.strip_suffix(".new")?, generic?)),
                    "array.from" => return Some(format!("array<{}>", self.infer(&args.first()?.value)?)),
                    n if n.starts_with("array.new_") => return Some(format!("array<{}>", &n[10..])),
                    "label.new" | "line.new" | "box.new" | "table.new" | "linefill.new" | "polyline.new" => {
                        name.strip_suffix(".new")?
                    }
                    "input.int" | "input.time" | "int" | "math.floor" | "math.ceil" | "ta.barssince"
                    | "ta.highestbars" | "ta.lowestbars" | "timestamp" | "str.length" | "array.size" => "int",
                    "math.round" if args.len() == 1 => "int",
                    "input.float" | "input.price" | "input.source" | "float" => "float",
                    "input.bool" | "bool" | "na" | "ta.crossover" | "ta.crossunder" | "ta.cross" | "ta.rising"
                    | "ta.falling" | "timeframe.change" | "str.contains" | "str.startswith" | "str.endswith" => "bool",
                    "input.string" | "input.symbol" | "input.timeframe" | "input.session" | "input.text_area"
                    | "string" | "str.tostring" | "str.format" | "str.lower" | "str.upper" | "str.replace"
                    | "str.replace_all" | "str.substring" | "str.trim" | "str.repeat" | "str.format_time"
                    | "str.match" | "array.join" => "string",
                    "input.color" | "color.new" | "color.rgb" | "color.from_gradient" => "color",
                    "input" | "nz" | "fixnan" | "ta.valuewhen" => return self.infer(&args.first()?.value),
                    "ta.change" | "math.abs" | "math.max" | "math.min" => match self.infer(&args.first()?.value)? {
                        t if t == "bool" => "bool",
                        _ => "float",
                    },
                    "ta.macd" | "ta.bb" | "ta.kc" | "ta.dmi" | "ta.supertrend" | "ta.vwap" => return None,
                    n if n.starts_with("ta.") || n.starts_with("math.") || n.starts_with("color.") => "float",
                    _ => return None,
                }
            }
            Expr::Dot(obj, f) => return self.field_type(&self.infer(obj)?, f),
            Expr::MethodCall { recv, name, .. } => return self.method_type(&self.infer(recv)?, name),
            _ => return None,
        };
        Some(t.to_string())
    }

    /// `e` in terms of constants and inputs only, if that's all it's made
    /// of: an expression that means the same in any frame.
    fn resolve(&self, e: &E) -> Option<E> {
        let sub = |x: &E| self.resolve(x).map(Box::new);
        Some(match e {
            E::K(_) | E::In(_) | E::InSrc(_) => e.clone(),
            E::L(s) => self.known.get(&(self.uid(), *s))?.clone(),
            E::G(s) => self.known.get(&(0, *s))?.clone(),
            E::Neg(x) => E::Neg(sub(x)?),
            E::Not(x) => E::Not(sub(x)?),
            E::Bin(op, a, b) => E::Bin(*op, sub(a)?, sub(b)?),
            E::And(a, b, l) => E::And(sub(a)?, sub(b)?, *l),
            E::Or(a, b, l) => E::Or(sub(a)?, sub(b)?, *l),
            E::Tern(c, a, b) => E::Tern(sub(c)?, sub(a)?, sub(b)?),
            E::Nat { f, args, site } if f.pure => E::Nat {
                f,
                args: args.iter().map(|a| self.resolve(a)).collect::<Option<_>>()?,
                site: *site,
            },
            _ => return None,
        })
        // A series mixed into the rest isn't fixed.
        .filter(|r| matches!(r, E::InSrc(_)) || !has_source(r))
    }

    /// A compile-time constant's value, through named constants. An
    /// input's value isn't one.
    fn const_value(&self, e: &E) -> Option<V> {
        fn inputs(e: &E) -> bool {
            match e {
                E::In(_) | E::InSrc(_) => true,
                E::Neg(x) | E::Not(x) => inputs(x),
                E::Bin(_, a, b) | E::And(a, b, _) | E::Or(a, b, _) => inputs(a) || inputs(b),
                E::Tern(a, b, c) => inputs(a) || inputs(b) || inputs(c),
                E::Nat { args, .. } => args.iter().any(inputs),
                _ => false,
            }
        }
        let r = self.resolve(e)?;
        (!inputs(&r)).then(|| self.eval_default(&r)).flatten()
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
        for (_, d, _) in params {
            if let Some(e) = d {
                let e = self.expr(e)?;
                // Pine allows built-in series (`source = close`) too.
                let ok = match &e {
                    E::Bar(_) => true,
                    E::Nat { args, .. } => args.iter().all(|a| matches!(a, E::K(_) | E::Bar(_))),
                    e => self.const_value(e).is_some(),
                };
                if !ok {
                    return Err(self.err("A parameter's default value must be a constant."));
                }
            }
        }
        // Registered after its body, so it can't see itself (Pine forbids
        // recursion) but later code can call it.
        let line = self.line;
        let (body_s, layout, flags) = self.func_body(params, body, line, None)?;
        let id = self.funcs.len() as u16;
        self.funcs.push(Func {
            name: name.to_string(),
            params: params.iter().map(|p| p.0.clone()).collect(),
            body: body_s,
            layout,
        });
        self.fn_info.push(FnInfo {
            params: Rc::new(params.to_vec()),
            body: Rc::new(body.clone()),
            line,
            lib: self.lib_prefix.clone(),
            mutable: self.mutable.clone(),
            flags,
        });
        if method {
            self.methods.entry(name.to_string()).or_default().push(id);
        } else {
            self.fnames.insert(name.to_string(), id);
            self.overloads.entry(name.to_string()).or_default().push(id);
        }
        Ok(())
    }

    /// Compiles a function's body in a frame of its own. `args` are one
    /// call's arguments (and the scopes they're read in).
    fn func_body(
        &mut self,
        params: &[Param],
        body: &ast::Block,
        line: usize,
        args: Option<&[CallArg]>,
    ) -> R<(Block, Layout, Flags)> {
        self.push_frame();
        let d = self.depth();
        let uid = self.uid();
        self.scopes.push((d, HashMap::new()));
        self.flags.push(Flags::default());
        let local = std::mem::take(&mut self.local);
        for (k, (p, _, ty)) in params.iter().enumerate() {
            let slot = self.new_slot();
            if let Some(t) = ty {
                self.slot_types.insert((uid, slot), t.clone());
            }
            if !self.mutable.contains(&(line, p.clone())) {
                let def = match args {
                    Some(a) => Def::Arg(a[k].0.clone(), self.line, a[k].1.clone()),
                    None => Def::Param,
                };
                self.defs.insert((uid, slot), def);
            }
            self.declare(p, Bind::Slot { depth: d, slot });
        }
        let body = self.block(body);
        self.local = local;
        self.scopes.pop();
        let layout = self.pop_frame();
        let flags = self.flags.pop().unwrap_or_default();
        Ok((body?, layout, flags))
    }

    /// A copy of function `f` for one call.
    fn instance(&mut self, f: u16, args: &[CallArg]) -> R<u16> {
        self.instances += 1;
        if self.instances > MAX_INSTANCES {
            return Err(self.err("Too many functions calling request.security() inside each other."));
        }
        let info = self.fn_info[f as usize].clone();
        let globals = vec![self.scopes[0].clone()];
        let scopes = std::mem::replace(&mut self.scopes, globals);
        let lib = std::mem::replace(&mut self.lib_prefix, info.lib.clone());
        let mutable = std::mem::replace(&mut self.mutable, info.mutable.clone());
        let line = self.line;
        self.again += 1;
        self.calls.push((
            info.params.clone(),
            args.iter().map(|a| a.0.clone()).collect(),
            info.body.clone(),
        ));
        let r = self.func_body(&info.params, &info.body, info.line, Some(args));
        self.calls.pop();
        self.again -= 1;
        self.line = line;
        self.mutable = mutable;
        self.lib_prefix = lib;
        self.scopes = scopes;
        let (body, layout, flags) = r?;
        // What it does, its caller does.
        if let Some(top) = self.flags.last_mut() {
            top.per_call |= flags.per_call;
        }
        let base = &self.funcs[f as usize];
        let func = Func {
            name: base.name.clone(),
            params: base.params.clone(),
            body,
            layout,
        };
        self.funcs.push(func);
        self.fn_info.push(info);
        Ok((self.funcs.len() - 1) as u16)
    }

    /// The function to call for `f` here: itself, or a copy for this call
    /// when a `request.security` depends on it.
    fn callee(&mut self, f: u16, args: &[Option<&Expr>]) -> R<u16> {
        let info = &self.fn_info[f as usize];
        let flags = info.flags;
        let in_sec = !self.secs.is_empty();
        if let Some(top) = self.flags.last_mut() {
            top.globals |= flags.globals;
            top.requests |= flags.requests;
        }
        if flags.requests
            && let Some(s) = self.secs.last_mut()
        {
            s.nested = true;
        }
        if !(flags.per_call || (in_sec && flags.globals)) {
            return Ok(f);
        }
        let params = info.params.clone();
        let here = Rc::new(self.scopes.clone());
        let globals = Rc::new(vec![self.scopes[0].clone()]);
        let args: Vec<CallArg> = params
            .iter()
            .enumerate()
            .map(|(k, p)| match args.get(k).copied().flatten() {
                Some(e) => (Rc::new(e.clone()), here.clone()),
                None => (Rc::new(p.1.clone().unwrap_or(Expr::Na)), globals.clone()),
            })
            .collect();
        self.instance(f, &args)
    }

    /// A default argument, compiled for a call.
    fn default_arg(&mut self, e: &Expr, lib: Option<String>) -> R<E> {
        let tail = self.scopes.split_off(1);
        let lib = std::mem::replace(&mut self.lib_prefix, lib);
        let r = self.expr(e);
        self.lib_prefix = lib;
        self.scopes.extend(tail);
        r
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
                        // v5 divides constant ints as ints.
                        if bop == Bin::Div
                            && self.version < 6
                            && let (Some(V::Int(p)), Some(V::Int(q))) = (self.const_value(&a), self.const_value(&b))
                            && q != 0
                        {
                            return Ok(E::K(V::Int(p.wrapping_div(q))));
                        }
                        if let (E::K(x), E::K(y)) = (&a, &b) {
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
            Expr::Loop(l) => match self.stmt(l)? {
                Some(s) => E::Loop(Box::new(s)),
                None => E::K(V::Na),
            },
            Expr::Tuple(items) => E::Tuple(items.iter().map(|x| self.expr(x)).collect::<R<_>>()?),
            Expr::Dot(obj, field) => {
                let o = self.expr(obj)?;
                self.check_field(field)?;
                E::Get(Box::new(o), Arc::from(field.as_str()))
            }
            Expr::MethodCall { recv, name, args } => self.method(recv, name, args)?,
        })
    }

    /// Reads a variable found in scope `k`.
    fn var(&mut self, n: &str, k: usize, depth: usize, slot: u16) -> R<E> {
        let known = self.known.get(&(self.uids[depth], slot));
        if let Some(sd) = self.secs.last().map(|s| s.depth)
            && depth < sd
        {
            if let Some(e) = known {
                return Ok(e.clone());
            }
            let s = self.hoist(n, k, depth, slot)?;
            return Ok(if self.depth() == sd { E::L(s) } else { E::G(s) });
        }
        if depth == 0 && self.depth() > 0 {
            // Constants and inputs are the same everywhere.
            if let Some(e @ (E::K(_) | E::In(_) | E::InSrc(_))) = known {
                return Ok(e.clone());
            }
            if let Some(f) = self.flags.last_mut() {
                f.globals = true;
            }
        }
        self.slot_ref(depth, slot)
    }

    /// Recomputes a variable from outside the `request.security` being
    /// compiled in its frame, from the variable's definition, and returns
    /// its slot there.
    fn hoist(&mut self, n: &str, k: usize, depth: usize, slot: u16) -> R<u16> {
        let var = (self.uids[depth], slot);
        let sec = self.secs.last().expect("in a request");
        if let Some(s) = sec.hoisted.get(&var) {
            return Ok(*s);
        }
        let sd = sec.depth;
        let Some(def) = self.defs.get(&var).cloned() else {
            self.wants_slice |= depth == 0;
            return Err(self.err(format!(
                "request.security() can't recompute \"{n}\" on other bars because it changes with := or var. Put the calculation in a function and call that inside request.security()."
            )));
        };
        if self.inlining.contains(&var) {
            return Err(self.err(format!("\"{n}\" depends on itself.")));
        }
        self.inlining.push(var);
        // Compiled as if written in the request, at the top of its frame.
        let above = (self.frames.split_off(sd + 1), self.uids.split_off(sd + 1));
        let line = self.line;
        let local = std::mem::take(&mut self.local);
        self.again += 1;
        let mut tail = self.scopes.split_off(k + 1);
        let r = (|| match &def {
            Def::Param => {
                // Not known until it's called.
                self.flags.iter_mut().for_each(|f| f.per_call = true);
                Ok(vec![(var, self.new_slot())])
            }
            Def::Value(e, at) => {
                self.line = *at;
                let v = self.in_request(sd, e)?;
                let s = self.new_slot();
                self.prelude(*at, S::Let(s, v));
                Ok(vec![(var, s)])
            }
            Def::Arg(e, at, scopes) => {
                self.line = *at;
                let own = std::mem::replace(&mut self.scopes, (**scopes).clone());
                let v = self.in_request(sd, e);
                self.scopes = own;
                let v = v?;
                let s = self.new_slot();
                self.prelude(*at, S::Let(s, v));
                Ok(vec![(var, s)])
            }
            Def::Tuple(e, at, vars) => {
                self.line = *at;
                let v = self.in_request(sd, e)?;
                let slots: Vec<Option<u16>> = vars.iter().map(|v| v.map(|_| self.new_slot())).collect();
                self.prelude(*at, S::LetTuple(slots.clone(), v));
                Ok(vars
                    .iter()
                    .zip(&slots)
                    .filter_map(|(v, s)| Some(((*v)?, (*s)?)))
                    .collect())
            }
        })();
        self.scopes.append(&mut tail);
        self.again -= 1;
        self.local = local;
        self.line = line;
        self.frames.extend(above.0);
        self.uids.extend(above.1);
        self.inlining.pop();
        let uid = self.uids[sd];
        for (v, s) in r? {
            if let Some(t) = self.slot_types.get(&v).cloned() {
                self.slot_types.insert((uid, s), t);
            }
            if let Some(sec) = self.secs.last_mut() {
                sec.hoisted.insert(v, s);
            }
        }
        match self.secs.last().and_then(|s| s.hoisted.get(&var)) {
            Some(s) => Ok(*s),
            None => Err(self.err(format!("request.security() can't recompute \"{n}\"."))),
        }
    }

    fn in_request(&mut self, depth: usize, e: &Expr) -> R<E> {
        self.scopes.push((depth, HashMap::new()));
        let r = self.expr(e);
        self.scopes.pop();
        r
    }

    fn prelude(&mut self, line: usize, s: S) {
        if let Some(sec) = self.secs.last_mut() {
            sec.prelude.push((line as u32, s));
        }
    }

    fn name(&mut self, n: &str) -> R<E> {
        match self.lookup_at(n) {
            Some((Bind::Slot { depth, slot }, k)) => return self.var(n, k, depth, slot),
            Some((Bind::Plot(_) | Bind::HLine(_), _)) => {
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
        if let Some(v) = constant_in(self.version, n) {
            return Ok(E::K(v));
        }
        let lib = self.in_lib(n, |cx, n| cx.lib_consts.contains_key(n));
        if let Some(v) = self.lib_consts.get(&lib) {
            return Ok(E::K(v.clone()));
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
        if self.version < 4
            && let Some(v) = bare_constant(n)
        {
            return Ok(E::K(v));
        }
        if natives::lookup(n).is_some() || self.fnames.contains_key(n) {
            return Err(self.err(format!("{n} is a function; call it with brackets: {n}(…).")));
        }
        if let Some(full) = renamed(n)
            && (full == "bar_index" || natives::lookup(&full).is_some_and(|d| d.var))
        {
            if self.version < 5 {
                return self.name(&full);
            }
            return Err(self.err(format!("Unknown name \"{n}\". Did you mean {full}?")));
        }
        if self.version < 4
            && let Some(v) = bare_constant(n)
        {
            return Ok(E::K(v));
        }
        if self.version < 5 && n.starts_with("input.") {
            return Ok(E::K(V::str(n)));
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

    fn nat(&mut self, f: &'static NatDef, mut args: Vec<E>) -> E {
        // A window whose length can change from bar to bar keeps the values
        // a longer one will need.
        if f.window
            && f.params
                .iter()
                .zip(&args)
                .any(|(p, a)| natives::LENGTHS.contains(p) && self.resolve(a).is_none())
        {
            args.resize(f.params.len(), E::K(V::Na));
            args.push(E::K(V::Bool(true)));
        }
        if f.pure && args.iter().all(|a| matches!(a, E::K(_))) {
            let vals: Vec<V> = args.iter().map(|a| const_of(a).unwrap_or_default()).collect();
            let mut g = crate::run::G::scratch(self.env.clone(), self.version);
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
            match self.lookup_at(name) {
                Some((Bind::Slot { depth, slot }, k)) => {
                    // An input.source variable reads bar history directly.
                    if let Some(E::InSrc(k)) = self.known.get(&(self.uids[depth], slot)) {
                        let k = *k;
                        self.need_bar_hist(n);
                        return Ok(E::InSrcH(k, Box::new(i)));
                    }
                    let sd = self.secs.last().map(|s| s.depth).filter(|sd| depth < *sd);
                    let Some(sd) = sd else {
                        self.need_hist(depth, slot, n);
                        if depth == 0
                            && self.depth() > 0
                            && let Some(f) = self.flags.last_mut()
                        {
                            f.globals = true;
                        }
                        return Ok(match self.slot_ref(depth, slot)? {
                            E::L(s) => E::LH(s, Box::new(i)),
                            E::G(s) => E::GH(s, Box::new(i)),
                            e => e,
                        });
                    };
                    if !self.known.contains_key(&(self.uids[depth], slot)) {
                        let s = self.hoist(name, k, depth, slot)?;
                        self.need_hist(sd, s, n);
                        return Ok(if self.depth() == sd {
                            E::LH(s, Box::new(i))
                        } else {
                            E::GH(s, Box::new(i))
                        });
                    }
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
        if self.fnames.contains_key(&user) {
            let f = self.overload(&user, args)?;
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
        {
            let var = self.lookup(obj.split('.').next().unwrap_or(obj)).is_some();
            let built_in = self.methods.contains_key(m)
                && (Field::by_name(obj).is_some()
                    || natives::lookup(obj).is_some_and(|d| d.var)
                    || constant_in(self.version, obj).is_some());
            if var || built_in {
                return self.method(&Expr::Name(obj.to_string()), m, args);
            }
            // Type.copy(obj)
            if m == "copy" && self.type_id(obj).is_some() && args.len() == 1 {
                return self.method(&args[0].value, "copy", &[]);
            }
        }
        // v3's color(red, 80).
        if callee == "color" && args.len() == 2 {
            return self.call("color.new", args);
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
        // A method called as a function: `f(x, …)` is `x.f(…)`.
        if self.methods.contains_key(callee)
            && let Some(first) = args.first().filter(|a| a.name.is_none())
        {
            return self.method(&first.value, callee, &args[1..]);
        }
        if self.methods.contains_key(callee) {
            return Err(self.err(format!("{callee} is a method; call it on a value: x.{callee}(…).")));
        }
        if self.version < 5 {
            match (callee, args) {
                ("iff", [c, a, b]) => {
                    return self.expr(&Expr::Ternary(
                        Box::new(c.value.clone()),
                        Box::new(a.value.clone()),
                        Box::new(b.value.clone()),
                    ));
                }
                ("offset", [x, k]) => {
                    return self.expr(&Expr::Index(Box::new(x.value.clone()), Box::new(k.value.clone())));
                }
                _ => {}
            }
        }
        if let Some(full) = renamed(callee)
            && natives::lookup(&full).is_some()
        {
            if self.version < 5 {
                // tostring(x, y)'s names.
                let mut args = args.to_vec();
                if full == "str.tostring" {
                    for a in &mut args {
                        match a.name.as_deref() {
                            Some("x") => a.name = Some("value".into()),
                            Some("y") => a.name = Some("format".into()),
                            _ => {}
                        }
                    }
                }
                return self.call(&full, &args);
            }
            return Err(self.err(format!("Unknown function \"{callee}\". Did you mean {full}()?")));
        }
        Err(self.err(format!("Unknown function \"{callee}\".")))
    }

    fn nat_args(&mut self, def: &'static NatDef, args: &[Arg]) -> R<Vec<E>> {
        if def.name == "timestamp" && args.iter().any(|a| a.name.is_some()) {
            const PARTS: &[&str] = &["timezone", "year", "month", "day", "hour", "minute", "second"];
            let zoned = args.iter().any(|a| a.name.as_deref() == Some("timezone"));
            let params = if zoned { PARTS } else { &PARTS[1..] };
            let mut params = params.to_vec();
            if args.len() == 1 {
                params = vec!["dateString"];
            }
            let bound = bind(def.name, &params, args, self.line)?;
            let last = bound.iter().rposition(|b| b.is_some()).map_or(0, |k| k + 1);
            return bound[..last]
                .iter()
                .enumerate()
                .map(|(k, a)| match a {
                    Some(e) => self.expr(e),
                    None => Ok(E::K(V::Int(if params[k] == "day" { 1 } else { 0 }))),
                })
                .collect();
        }
        if def.params == ["*"] {
            return args.iter().map(|a| self.expr(&a.value)).collect();
        }
        // v4's strategy.entry(long = true).
        let mut own: Vec<Arg>;
        let mut args = args;
        if self.version < 5 && matches!(def.name, "strategy.entry" | "strategy.order") {
            own = args.to_vec();
            for a in &mut own {
                if a.name.as_deref() == Some("long") {
                    a.name = Some("direction".into());
                }
            }
            args = &own;
        }
        let bound = bind(def.name, def.params, args, self.line)?;
        if let Some(k) = natives::missing(def, &bound.iter().map(|b| b.is_some()).collect::<Vec<_>>()) {
            return Err(self.err(format!("{}() needs a value for \"{}\".", def.name, def.params[k])));
        }
        bound
            .into_iter()
            .map(|a| match a {
                Some(e) => self.expr(e),
                None => Ok(E::K(V::Na)),
            })
            .collect()
    }

    /// Which of the functions declared as `name` takes `args`: the first
    /// whose parameters bind them and whose declared types don't clash
    /// with the arguments' evident types.
    fn overload(&self, name: &str, args: &[Arg]) -> R<u16> {
        let all = &self.overloads[name];
        if all.len() < 2 {
            return Ok(self.fnames[name]);
        }
        let fits = |f: u16| {
            let func = &self.funcs[f as usize];
            let info = &self.fn_info[f as usize];
            let params: Vec<&str> = func.params.iter().map(|s| s.as_str()).collect();
            let Ok(bound) = bind(name, &params, args, self.line) else {
                return None;
            };
            // How many conversions it takes.
            let mut cost = 0;
            for (k, a) in bound.iter().enumerate() {
                match a {
                    None if info.params[k].1.is_some() => {}
                    None => return None,
                    Some(e) => match (self.infer(e), info.params[k].2.as_deref()) {
                        (Some(have), Some(want)) if have == want || have == "na" => {}
                        (Some(have), Some("float")) if have == "int" => cost += 1,
                        (Some(have), Some(want)) if base_type(&have) == want || have == base_type(want) => cost += 1,
                        (Some(_), Some(_)) => return None,
                        _ => cost += 1,
                    },
                }
            }
            Some(cost)
        };
        all.iter()
            .copied()
            .filter_map(|f| Some((fits(f)?, f)))
            .min_by_key(|(c, _)| *c)
            .map(|(_, f)| f)
            .ok_or_else(|| self.err(format!("No version of {name}() takes these arguments.")))
    }

    fn user_call(&mut self, f: u16, args: &[Arg]) -> R<E> {
        let name = self.funcs[f as usize].name.clone();
        let info = self.fn_info[f as usize].clone();
        let params: Vec<&str> = info.params.iter().map(|p| p.0.as_str()).collect();
        let bound = bind(&name, &params, args, self.line)?;
        let mut argv = Vec::new();
        for (k, a) in bound.iter().enumerate() {
            argv.push(match (a, &info.params[k].1) {
                (Some(e), _) => self.expr(e)?,
                (None, Some(e)) => self.default_arg(e, info.lib.clone())?,
                (None, None) => {
                    return Err(self.err(format!("{name}() needs a value for \"{}\".", params[k])));
                }
            });
        }
        let f = self.callee(f, &bound)?;
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
                None => match self.type_defaults[ti][k].clone() {
                    Some(d) => self.default_arg(&d, t.name.rsplit_once('.').map(|(p, _)| p.to_string()))?,
                    None => E::K(V::Na),
                },
            });
        }
        Ok(E::New(ti as u16, vals))
    }

    /// What a method's first parameter takes.
    fn this_of(&self, f: u16) -> This {
        let info = &self.fn_info[f as usize];
        let Some(t) = info.params.first().and_then(|p| p.2.as_deref()) else {
            return This::Any;
        };
        let full = match &info.lib {
            Some(p) if self.types.iter().any(|x| x.name == format!("{p}.{t}")) => format!("{p}.{t}"),
            _ => t.to_string(),
        };
        if let Some(i) = self.types.iter().position(|x| x.name == full) {
            return This::Udt(i as u16);
        }
        let elem = t
            .strip_prefix("array<")
            .and_then(|x| x.strip_suffix('>'))
            .map(Arc::from);
        This::Of(Arc::from(base_type(t)), elem)
    }

    fn method(&mut self, recv: &Expr, m: &str, args: &[Arg]) -> R<E> {
        let r = self.expr(recv)?;
        let hint = self.infer(recv).map(Arc::from);
        let mut argv = Vec::new();
        let mut names = Vec::new();
        for a in args {
            names.push(a.name.as_deref().map(Arc::from));
            argv.push(self.expr(&a.value)?);
        }
        // One call site (its own frame) per method it could be.
        let mut user = Vec::new();
        for f in self.methods.get(m).cloned().unwrap_or_default() {
            let info = self.fn_info[f as usize].clone();
            let params: Vec<&str> = info.params.iter().skip(1).map(|p| p.0.as_str()).collect();
            let Ok(bound) = bind(m, &params, args, self.line) else {
                continue;
            };
            let this = self.this_of(f);
            let all: Vec<Option<&Expr>> = std::iter::once(Some(recv)).chain(bound).collect();
            let mut defaults = Vec::new();
            for p in info.params.iter() {
                defaults.push(match &p.1 {
                    Some(e) => Some(self.default_arg(e, info.lib.clone())?),
                    None => None,
                });
            }
            let f = self.callee(f, &all)?;
            let frame = self.frame();
            frame.calls.push(f);
            user.push(MethodCand {
                f,
                this,
                site: (frame.calls.len() - 1) as u16,
                defaults,
            });
        }
        Ok(E::Method {
            name: Arc::from(m),
            recv: Box::new(r),
            args: argv,
            names,
            user,
            hint,
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
        self.requests = true;
        if let Some(f) = self.flags.last_mut() {
            f.requests = true;
        }
        if let Some(s) = self.secs.last_mut() {
            s.nested = true;
        }
        // The expression gets its own frame, as if it were a function
        // called on the other timeframe's bars.
        let outer = std::mem::take(&mut self.wants_slice);
        let mut done = self.request_body(expr, &[], &[]).map(|b| Body::Here(Box::new(b)));
        if done.is_err() && std::mem::take(&mut self.wants_slice) {
            if !self.flags.is_empty() && self.calls.is_empty() {
                // In a function: what the state is depends on where it's
                // called.
                self.flags.iter_mut().for_each(|f| f.per_call = true);
                done = Ok(Body::Here(Box::default()));
            } else {
                let (before, after) = self.slice_for(expr);
                if after.is_empty() {
                    self.again += 1;
                    done = self.request_body(expr, &before, &[]).map(|b| Body::Here(Box::new(b)));
                    self.again -= 1;
                } else if self.secs.is_empty() {
                    self.late.push(LateRequest {
                        expr: expr.clone(),
                        before,
                        after,
                        scopes: self.scopes.clone(),
                        uids: self.uids.clone(),
                        lib: self.lib_prefix.clone(),
                        mutable: self.mutable.clone(),
                        calls: self.calls.clone(),
                        line: self.line,
                        at: self.root_at,
                    });
                    done = Ok(Body::Late((self.late.len() - 1) as u16));
                }
            }
        }
        self.wants_slice = outer;
        let body = done?;
        let frame = self.frame();
        frame.secs += 1;
        let site = (frame.secs - 1) as u16;
        Ok(E::Security(Box::new(Sec {
            symbol,
            tf,
            body,
            gaps,
            lookahead,
            site,
            lower,
        })))
    }

    /// Compiles the requests that were left for last.
    fn compile_late(&mut self) -> R<Vec<SecBody>> {
        let globals = self.scopes[0].clone();
        let mut out = Vec::new();
        self.again += 1;
        for l in std::mem::take(&mut self.late) {
            self.frames = vec![Layout::default(); l.uids.len()];
            self.uids = l.uids;
            self.scopes = l.scopes;
            // Everything declared by now, for the statements below it.
            self.scopes[0] = globals.clone();
            self.lib_prefix = l.lib;
            self.mutable = l.mutable;
            self.calls = l.calls;
            self.line = l.line;
            self.root_at = l.at;
            self.local = 0;
            out.push(self.request_body(&l.expr, &l.before, &l.after)?);
        }
        Ok(out)
    }

    /// The top-level statements a request's expression depends on, when it
    /// uses variables that are reassigned.
    fn slice_for(&mut self, expr: &Expr) -> (Vec<Stmt>, Vec<Stmt>) {
        let ast = self.ast.clone();
        let p = match &self.slices {
            Some(p) => p.clone(),
            None => {
                let p = Rc::new(crate::slice::Program::new(&ast));
                self.slices = Some(p.clone());
                p
            }
        };
        let mut names = p.names(std::iter::once(expr));
        // Through the functions it's in, innermost first.
        for (params, args, body) in self.calls.iter().rev() {
            names = p.outside(names, params, args, body);
        }
        let (before, after): (Vec<usize>, Vec<usize>) = p
            .needed(self.root_at, names)
            .into_iter()
            .filter(|k| {
                !matches!(&ast[*k], Stmt::Decl { value: Expr::Call { callee, .. }, .. } if callee == "plot" || callee == "hline")
            })
            .partition(|k| *k < self.root_at);
        let get = |ks: Vec<usize>| ks.into_iter().map(|k| ast[k].clone()).collect();
        (get(before), get(after))
    }

    /// Compiles a request's expression, between the top-level statements it
    /// depends on (see [`Cx::slice_for`]).
    fn request_body(&mut self, expr: &Expr, before: &[Stmt], after: &[Stmt]) -> R<SecBody> {
        if self.secs.len() >= 4 {
            return Err(self.err("Too many request.security() calls inside each other."));
        }
        self.push_frame();
        let depth = self.depth();
        self.sec_depth += 1;
        self.secs.push(SecCx {
            depth,
            hoisted: HashMap::new(),
            prelude: Vec::new(),
            nested: false,
        });
        let saved_bar = std::mem::replace(&mut self.sec_bar_hist, 1);
        let line = self.line;
        let r = self.request_parts(expr, before, after, depth);
        self.line = line;
        let bar_hist = std::mem::replace(&mut self.sec_bar_hist, saved_bar);
        self.sec_depth -= 1;
        let cx = self.secs.pop();
        let layout = self.pop_frame();
        let (e, post) = r?;
        let (prelude, nested) = cx.map(|c| (c.prelude, c.nested)).unwrap_or_default();
        Ok(SecBody {
            prelude,
            expr: Some(e),
            post,
            nested,
            layout,
            bar_hist,
        })
    }

    fn request_parts(&mut self, expr: &Expr, before: &[Stmt], after: &[Stmt], depth: usize) -> R<(E, Block)> {
        let mut own = (depth, HashMap::new());
        let pre = self.request_stmts(before, &mut own)?;
        if let Some(sec) = self.secs.last_mut() {
            // Variables the statements recomputed come before them.
            sec.prelude.extend(pre);
        }
        self.scopes.push((depth, HashMap::new()));
        let e = self.expr(expr);
        self.scopes.pop();
        let e = e?;
        let post = self.request_stmts(after, &mut own)?;
        Ok((e, post))
    }

    /// Top-level statements, compiled into a request's frame.
    fn request_stmts(&mut self, stmts: &[Stmt], own: &mut Scope) -> R<Block> {
        if stmts.is_empty() {
            return Ok(Vec::new());
        }
        let scopes = vec![self.scopes[0].clone(), std::mem::take(own)];
        let scopes = std::mem::replace(&mut self.scopes, scopes);
        let local = std::mem::take(&mut self.local);
        let mut out = Vec::new();
        let r = stmts.iter().try_for_each(|s| -> R<()> {
            let at = self.secs.last().map_or(0, |c| c.prelude.len());
            let compiled = self.block(std::slice::from_ref(s))?;
            // What it needed recomputed, then it.
            if let Some(sec) = self.secs.last_mut() {
                out.extend(sec.prelude.drain(at..));
            }
            out.extend(compiled);
            // From here on the script's variable is this one.
            if let Stmt::Decl { names, line, .. } = s {
                for n in names {
                    let Some(g) = self.root_slots.get(&(*line, n.clone())).copied() else {
                        continue;
                    };
                    if let Some(Bind::Slot { slot, .. }) = self.scopes[1].1.get(n).copied()
                        && let Some(sec) = self.secs.last_mut()
                    {
                        sec.hoisted.insert((0, g), slot);
                    }
                }
            }
            Ok(())
        });
        *own = std::mem::take(&mut self.scopes[1]);
        self.scopes = scopes;
        self.local = local;
        r.map(|_| out)
    }

    // -------------------------------------------------------- inputs

    fn input(&mut self, callee: &str, args: &[Arg]) -> R<E> {
        // Source compiled a second time (for request.security) means the
        // inputs it made the first time.
        let cache = (self.line, format!("{callee}{args:?}"));
        if self.again > 0
            && let Some(e) = self.input_cache.get(&cache)
        {
            return Ok(e.clone());
        }
        // Positions as in the manual; the ones that only lay out the
        // Inputs tab are read past.
        const NUMBER: &[&str] = &[
            "defval", "title", "minval", "maxval", "step", "tooltip", "inline", "group", "confirm", "display", "active",
        ];
        const CHOICE: &[&str] = &[
            "defval", "title", "options", "tooltip", "inline", "group", "confirm", "display", "active",
        ];
        const PLAIN: &[&str] = &[
            "defval", "title", "tooltip", "inline", "group", "confirm", "display", "active",
        ];
        const V4: &[&str] = &[
            "defval", "title", "type", "minval", "maxval", "confirm", "step", "options", "tooltip", "inline", "group",
        ];
        const ANY: &[&str] = &["defval", "title", "tooltip", "inline", "group", "display", "active"];
        let options_third = matches!(
            args.get(2),
            Some(Arg {
                name: None,
                value: Expr::Tuple(_)
            })
        ) || args.iter().any(|a| a.name.as_deref() == Some("options"));
        let params: &[&str] = match callee {
            "input"
                if args.iter().any(|a| {
                    matches!(
                        a.name.as_deref(),
                        Some("type" | "minval" | "maxval" | "step" | "options")
                    )
                }) =>
            {
                V4
            }
            "input" => ANY,
            "input.int" | "input.float" if options_third => CHOICE,
            "input.int" | "input.float" => NUMBER,
            "input.string" | "input.session" | "input.timeframe" | "input.enum" => CHOICE,
            "input.bool" | "input.color" | "input.source" | "input.symbol" | "input.text_area" | "input.time"
            | "input.price" => PLAIN,
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
                    Ok(Some(cx.const_value(&e).ok_or_else(|| {
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
        let confirm = get(self, "confirm")?;
        let options: Vec<V> = match params.iter().position(|p| *p == "options").and_then(|k| b[k]) {
            Some(Expr::Tuple(items)) => {
                let mut out = Vec::new();
                for it in items {
                    let e = self.expr(it)?;
                    out.push(
                        self.const_value(&e)
                            .ok_or_else(|| self.err("Input options must be constants."))?,
                    );
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
            self.const_value(&e)
                .ok_or_else(|| self.err("An input's default value must be a constant."))?
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
                        step: step.as_ref().and_then(|v| v.i()).filter(|s| *s > 0).unwrap_or(1),
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
                ("input.symbol", _) => (
                    InputKind::Symbol {
                        confirm: confirm.as_ref().is_some_and(|v| v.truthy()),
                    },
                    Value::Text(defval.as_str().unwrap_or("").into()),
                    Conv::Str,
                ),
                ("input.time", _) => (
                    InputKind::Text,
                    Value::Text(natives::input_date(&self.env, defval.i().unwrap_or(0))),
                    Conv::Time,
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
        let e = if conv == Conv::Source { E::InSrc(i) } else { E::In(i) };
        self.input_cache.insert(cache, e.clone());
        Ok(e)
    }

    /// An expression of constants and inputs, with the inputs' defaults
    /// (for declarations).
    fn const_or_default(&self, e: &E) -> Option<V> {
        self.eval_default(e)
    }

    fn input_default(&self, i: u16) -> V {
        let d = &self.inputs[i as usize];
        match (&d.spec.default, d.conv) {
            (Value::Int(x), _) => V::Int(*x),
            (Value::Float(x), _) => V::Float(*x),
            (Value::Bool(x), _) => V::Bool(*x),
            (Value::Text(t), Conv::Time) => natives::date_of_input(&self.env, t).map_or(V::Na, V::Int),
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
            E::L(_) | E::G(_) => self.linear(&self.resolve(e)?),
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
            E::L(_) | E::G(_) => return self.eval_default(&self.resolve(e)?),
            E::Neg(x) => crate::run::neg(&self.eval_default(x)?),
            E::Not(x) => V::Bool(!self.eval_default(x)?.truthy()),
            E::Bin(op, a, b) => crate::run::binary(*op, &self.eval_default(a)?, &self.eval_default(b)?),
            E::And(a, b, _) => V::Bool(self.eval_default(a)?.truthy() && self.eval_default(b)?.truthy()),
            E::Or(a, b, _) => V::Bool(self.eval_default(a)?.truthy() || self.eval_default(b)?.truthy()),
            E::Tern(c, a, b) => {
                if self.eval_default(c)?.truthy() {
                    self.eval_default(a)?
                } else {
                    self.eval_default(b)?
                }
            }
            E::Nat { f, args, .. } if f.pure => {
                let vals: Vec<V> = args.iter().map(|a| self.eval_default(a)).collect::<Option<_>>()?;
                let mut g = crate::run::G::scratch(self.env.clone(), self.version);
                (f.f)(&mut g, &vals, &mut None).ok()?
            }
            E::If(branches, otherwise) => {
                let one = |b: &Block| match b.as_slice() {
                    [(_, S::Expr(x))] => self.eval_default(x),
                    _ => None,
                };
                for (c, b) in branches {
                    if self.eval_default(c)?.truthy() {
                        return one(b);
                    }
                }
                return one(otherwise.as_ref()?);
            }
            E::Switch(subj, cases) => {
                let subj = match subj {
                    Some(x) => Some(self.eval_default(x)?),
                    None => None,
                };
                for (c, b) in cases {
                    let hit = match (c, &subj) {
                        (None, _) => true,
                        (Some(c), Some(v)) => self.eval_default(c)?.same(v),
                        (Some(c), None) => self.eval_default(c)?.truthy(),
                    };
                    if hit {
                        return match b.as_slice() {
                            [(_, S::Expr(x))] => self.eval_default(x),
                            _ => None,
                        };
                    }
                }
                return None;
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
                let b = bind(
                    callee,
                    &[
                        "color",
                        "offset",
                        "editable",
                        "show_last",
                        "title",
                        "display",
                        "force_overlay",
                        "transp",
                    ],
                    args,
                    line,
                )?;
                let Some(c) = b[0] else {
                    return Err(self.err(format!("{callee}() needs a colour.")));
                };
                let mut c = self.expr(c)?;
                if let Some(t) = b[7] {
                    let t = self.expr(t)?;
                    let f = natives::lookup("color.new").expect("color.new");
                    c = self.nat(f, vec![c, t]);
                }
                if self.display_none(args)? {
                    c = E::K(V::Na);
                }
                let mut back = 0;
                // Shifted right: the colour of `offset` bars ago.
                if let Some(o) = b[1] {
                    let o = self.expr(o)?;
                    let k = self.eval_default(&o).and_then(|v| v.i()).unwrap_or(0);
                    back = k.min(0).unsigned_abs() as usize;
                    if k > 0 && !matches!(c, E::K(_)) {
                        let slot = self.new_slot();
                        let d = self.depth();
                        self.need_hist(d, slot, k as usize);
                        c = E::HistOf {
                            slot,
                            val: Box::new(c),
                            idx: Box::new(E::K(V::Int(k))),
                        };
                    }
                }
                Some(if back > 0 {
                    S::Past(callee == "barcolor", back, c)
                } else if callee == "bgcolor" {
                    S::Bg(c)
                } else {
                    S::BarColor(c)
                })
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
                            cx.eval_default(&e)
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
                "explicit_plot_zorder",
                "max_lines_count",
                "max_labels_count",
                "max_boxes_count",
                "calc_bars_count",
                "risk_free_rate",
                "use_bar_magnifier",
                "fill_orders_on_standard_ohlc",
                "max_polylines_count",
                "dynamic_requests",
                "behind_chart",
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
                "explicit_plot_zorder",
                "max_lines_count",
                "max_labels_count",
                "max_boxes_count",
                "calc_bars_count",
                "max_polylines_count",
                "dynamic_requests",
                "behind_chart",
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
        // v4's name for it.
        for a in &mut named {
            match a.name.as_deref() {
                Some("resolution") => a.name = Some("timeframe".into()),
                Some("resolution_gaps") => a.name = Some("timeframe_gaps".into()),
                _ => {}
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
                "timeframe" => self.timeframe = v.as_str().filter(|t| !t.is_empty()).map(str::to_string),
                "timeframe_gaps" => self.timeframe_gaps = v.truthy(),
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
                let uid = if matches!(e, E::G(_)) { 0 } else { self.uid() };
                if let Some(k) = self.known.get(&(uid, *s)) {
                    self.palette(k, out);
                }
                for c in self.palettes.get(&(uid, *s)).into_iter().flatten() {
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
                "editable",
                "show_last",
                "display",
                "format",
                "precision",
                "force_overlay",
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
        if let Some(t) = b[16] {
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

    /// Whether `display =` keeps an output off the chart (it may still be
    /// for the data window or an alert).
    fn display_none(&mut self, args: &[Arg]) -> R<bool> {
        for a in args {
            if a.name.as_deref() == Some("display") {
                return Ok(!self.on_chart(&a.value)?);
            }
        }
        Ok(false)
    }

    fn on_chart(&mut self, e: &Expr) -> R<bool> {
        Ok(match e {
            // display.pane + display.data_window
            Expr::Binary("+", a, b) => self.on_chart(a)? || self.on_chart(b)?,
            // display.all - display.status_line
            Expr::Binary("-", a, b) => self.on_chart(a)? && !self.on_chart(b)?,
            Expr::Ternary(c, a, b) => {
                let c = self.expr(c)?;
                match self.eval_default(&c) {
                    Some(v) if !v.truthy() => self.on_chart(b)?,
                    _ => self.on_chart(a)?,
                }
            }
            e => {
                let e = self.expr(e)?;
                match self.eval_default(&e) {
                    Some(V::Str(s)) => matches!(s.as_ref(), "display.all" | "display.pane"),
                    _ => true,
                }
            }
        })
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
                "editable",
                "size",
                "show_last",
                "display",
                "format",
                "precision",
                "force_overlay",
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
                "editable",
                "size",
                "show_last",
                "display",
                "format",
                "precision",
                "force_overlay",
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
        if let Some(t) = b[15] {
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
        let size = Self::shape_size(self.const_str(b[9])?.as_deref());
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
                "editable",
                "show_last",
                "display",
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
            if callee == "plotbar" {
                &[
                    "open",
                    "high",
                    "low",
                    "close",
                    "title",
                    "color",
                    "editable",
                    "show_last",
                    "display",
                    "format",
                    "precision",
                    "force_overlay",
                ]
            } else {
                &[
                    "open",
                    "high",
                    "low",
                    "close",
                    "title",
                    "color",
                    "wickcolor",
                    "editable",
                    "show_last",
                    "bordercolor",
                    "display",
                    "format",
                    "precision",
                    "force_overlay",
                ]
            },
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
            &[
                "price",
                "title",
                "color",
                "linestyle",
                "linewidth",
                "editable",
                "display",
            ],
            args,
            self.line,
        )?;
        let price_e = match b[0] {
            Some(e) => {
                let e = self.expr(e)?;
                self.resolve(&e)
            }
            None => None,
        };
        let price = price_e
            .as_ref()
            .and_then(|e| self.eval_default(e))
            .map(|v| v.f())
            .filter(|f| !f.is_nan())
            .ok_or_else(|| self.err("hline() needs a constant price (or an input)."))?;
        let color_e = match b[2] {
            Some(e) => {
                let e = self.expr(e)?;
                self.resolve(&e)
            }
            None => None,
        };
        self.levels.push((price_e.unwrap_or(E::K(V::Na)), color_e));
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
        // The gradient form: two values and two colours.
        let named = |n: &str| args.iter().any(|a| a.name.as_deref() == Some(n));
        let plain = |k: usize| args.get(k).filter(|a| a.name.is_none());
        let gradient = named("top_color")
            || named("top_value")
            || (plain(4).is_some() && plain(5).is_some() && {
                // fill(p1, p2, 100, 70, color.red, color.blue)
                !matches!(plain(5).map(|a| &a.value), Some(Expr::Bool(_)))
                    && plain(2).is_some_and(|a| self.infer(&a.value).is_none_or(|t| t != "color"))
            });
        let params: &[&str] = if gradient {
            &[
                "plot1",
                "plot2",
                "top_value",
                "bottom_value",
                "top_color",
                "bottom_color",
                "title",
                "display",
                "fillgaps",
                "editable",
            ]
        } else {
            &[
                "plot1",
                "plot2",
                "color",
                "title",
                "editable",
                "show_last",
                "fillgaps",
                "display",
                "transp",
            ]
        };
        let args: Vec<Arg> = args
            .iter()
            .map(|a| Arg {
                name: a.name.as_deref().map(|n| match n {
                    "hline1" => "plot1".to_string(),
                    "hline2" => "plot2".to_string(),
                    n => n.to_string(),
                }),
                value: a.value.clone(),
            })
            .collect();
        let b = bind("fill", params, &args, self.line)?;
        let mut before = Vec::new();
        let mut handle = |cx: &mut Cx, e: Option<&Expr>| -> R<Bind> {
            match e {
                Some(Expr::Name(n)) => match cx.lookup(n) {
                    Some(b @ (Bind::Plot(_) | Bind::HLine(_))) => Ok(b),
                    _ => Err(cx.err(format!("fill() takes plots or hlines; \"{n}\" isn't one."))),
                },
                Some(Expr::Call { callee, args }) if callee == "plot" || callee == "hline" => {
                    before.extend(cx.output_call(callee, args)?.flatten());
                    Ok(if callee == "plot" {
                        Bind::Plot((cx.plots.len() - 1) as u16)
                    } else {
                        Bind::HLine((cx.hlines.len() - 1) as u16)
                    })
                }
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
        let colour_arg = if gradient { b[4].or(b[5]) } else { b[2] };
        let (mut color, mut palette) = self.color_arg(colour_arg, default)?;
        if !gradient && let Some(t) = b[8] {
            let t = self.expr(t)?;
            if let Some(tv) = self.const_or_default(&t) {
                palette = palette.iter().map(|c| value::with_transp(*c, tv.f())).collect();
            }
            if let Some(c) = color.take() {
                let f = natives::lookup("color.new").expect("color.new");
                color = Some(self.nat(f, vec![c, t]));
            }
        }
        let title = self
            .const_str(if gradient { b[6] } else { b[3] })?
            .unwrap_or_else(|| "Background".into());
        let visible = !self.display_none(&args)?;
        self.fills.push(FillDef {
            title,
            between,
            palette,
            visible,
        });
        let i = (self.fills.len() - 1) as u16;
        before.extend(color.map(|color| S::FillColor { i, color }));
        Ok(match before.len() {
            0 | 1 => before.pop(),
            _ => Some(S::Many(before)),
        })
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

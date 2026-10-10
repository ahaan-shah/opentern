//! The syntax tree.

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Color(u32),
    Na,
    /// A name, possibly dotted: `close`, `color.red`, `ta.tr`, `p.x`.
    Name(String),
    Call {
        callee: String,
        args: Vec<Arg>,
    },
    /// `x[n]`
    Index(Box<Expr>, Box<Expr>),
    /// `expr.field` after a call or index (`a.get(0).x`).
    Dot(Box<Expr>, String),
    /// `expr.method(args)` after a call or index.
    MethodCall {
        recv: Box<Expr>,
        name: String,
        args: Vec<Arg>,
    },
    Unary(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    If(Vec<(Expr, Block)>, Option<Block>),
    Switch(Option<Box<Expr>>, Vec<(Option<Expr>, Block)>),
    /// `[a, b]` as a value (tuple returns, array literals in options).
    Tuple(Vec<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Expr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Persist {
    No,
    Var,
    Varip,
}

/// A function parameter: name, default, declared type.
pub type Param = (String, Option<Expr>, Option<String>);

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Decl {
        persist: Persist,
        names: Vec<String>,
        /// The declared type, if written (`float x = …`, `Point p = …`).
        ty: Option<String>,
        value: Expr,
        line: usize,
    },
    /// `name := value` (also `+=` etc.); `name` may be dotted (`p.x`).
    Assign {
        name: String,
        op: &'static str,
        value: Expr,
        line: usize,
    },
    Expr(Expr, usize),
    For {
        var: String,
        from: Expr,
        to: Expr,
        step: Option<Expr>,
        body: Block,
        line: usize,
    },
    /// `for x in arr` / `for [i, x] in arr`.
    ForIn {
        idx: Option<String>,
        var: String,
        over: Expr,
        body: Block,
        line: usize,
    },
    While {
        cond: Expr,
        body: Block,
        line: usize,
    },
    Break,
    Continue,
    Func {
        name: String,
        params: Vec<Param>,
        body: Block,
        line: usize,
        method: bool,
    },
    /// `type Name` with its fields (name, default).
    Type {
        name: String,
        fields: Vec<(String, Option<Expr>)>,
        line: usize,
    },
    /// `enum Name` with members and their titles (the member's name when
    /// no title is given).
    Enum {
        name: String,
        members: Vec<(String, String)>,
        line: usize,
    },
    Import {
        path: String,
        alias: String,
        line: usize,
    },
}

pub type Block = Vec<Stmt>;

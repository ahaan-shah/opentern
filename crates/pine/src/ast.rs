//! The syntax tree.

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Color(u32),
    Na,
    /// A name, possibly dotted: `close`, `color.red`, `ta.tr`.
    Name(String),
    Call {
        callee: String,
        args: Vec<Arg>,
    },
    /// `x[n]`
    Index(Box<Expr>, Box<Expr>),
    Unary(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    If(Vec<(Expr, Block)>, Option<Block>),
    Switch(Option<Box<Expr>>, Vec<(Option<Expr>, Block)>),
    /// `[a, b]` as a value (tuple returns).
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

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Decl {
        persist: Persist,
        names: Vec<String>,
        value: Expr,
        line: usize,
    },
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
    While {
        cond: Expr,
        body: Block,
        line: usize,
    },
    Break,
    Continue,
    Func {
        name: String,
        params: Vec<(String, Option<Expr>)>,
        body: Block,
        line: usize,
    },
}

pub type Block = Vec<Stmt>;

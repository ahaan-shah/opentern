//! Which of the script's top-level statements a `request.security`
//! expression depends on when some of it is built up with `var` and `:=`:
//! those statements run on the requested bars, in order, before the
//! expression does.

use crate::ast::*;
use std::collections::{HashMap, HashSet};

type Names = HashSet<String>;

/// Methods and functions that change the object they're given.
const CHANGES: [&str; 24] = [
    "push",
    "unshift",
    "pop",
    "shift",
    "insert",
    "remove",
    "set",
    "clear",
    "sort",
    "reverse",
    "fill",
    "concat",
    "put",
    "put_all",
    "add_row",
    "add_col",
    "remove_row",
    "remove_col",
    "swap_rows",
    "swap_columns",
    "reshape",
    "first",
    "last",
    "get",
];

#[derive(Clone, Default, Debug)]
struct Use {
    reads: Names,
    writes: Names,
    /// User functions and methods called.
    calls: Names,
}

#[derive(Clone, Default, Debug)]
struct FnUse {
    u: Use,
    /// It changes an object passed to it.
    changes_args: bool,
}

/// What each top-level statement reads and changes.
pub(crate) struct Program {
    stmts: Vec<Use>,
    fns: HashMap<String, FnUse>,
}

fn root(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}

struct Walk<'a> {
    fns: &'a HashMap<String, FnUse>,
    u: Use,
    /// Names declared in the blocks being walked: not the script's own.
    locals: Vec<Names>,
}

impl Walk<'_> {
    fn block(&mut self, b: &Block) {
        self.locals.push(Names::new());
        b.iter().for_each(|s| self.stmt(s));
        self.locals.pop();
    }

    fn read(&mut self, n: &str) {
        let n = root(n);
        if !self.locals.iter().any(|l| l.contains(n)) {
            self.u.reads.insert(n.to_string());
        }
    }

    fn write(&mut self, n: &str) {
        let n = root(n);
        if !self.locals.iter().any(|l| l.contains(n)) {
            self.u.writes.insert(n.to_string());
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Decl { names, value, .. } => {
                self.expr(value);
                match self.locals.last_mut() {
                    Some(l) => l.extend(names.iter().cloned()),
                    None => self.u.writes.extend(names.iter().cloned()),
                }
            }
            Stmt::Assign { name, value, .. } => {
                self.write(name);
                self.read(name);
                self.expr(value);
            }
            Stmt::AssignField { obj, value, .. } => {
                self.changed(obj);
                self.expr(obj);
                self.expr(value);
            }
            Stmt::Expr(e, _) => self.expr(e),
            Stmt::For {
                var,
                from,
                to,
                step,
                body,
                ..
            } => {
                [Some(from), Some(to), step.as_ref()]
                    .into_iter()
                    .flatten()
                    .for_each(|e| self.expr(e));
                self.locals.push(Names::from([var.clone()]));
                self.block(body);
                self.locals.pop();
            }
            Stmt::ForIn { over, body, .. } => {
                self.expr(over);
                // Its items may be changed through the loop's variable.
                self.changed(over);
                self.block(body);
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond);
                self.block(body);
            }
            _ => {}
        }
    }

    /// The variable an object expression starts from is changed.
    fn changed(&mut self, e: &Expr) {
        match e {
            Expr::Name(n) => self.write(n),
            Expr::Dot(x, _) | Expr::Index(x, _) => self.changed(x),
            Expr::MethodCall { recv, .. } => self.changed(recv),
            Expr::Call { callee, args } => {
                if let Some(a) = args.first() {
                    self.changed(&a.value);
                }
                if callee.contains('.') {
                    self.write(callee);
                }
            }
            _ => {}
        }
    }

    fn call(&mut self, name: &str, recv: Option<&Expr>, args: &[Arg]) {
        self.u.calls.insert(name.to_string());
        if self.fns.get(name).is_some_and(|f| f.changes_args) {
            recv.into_iter()
                .chain(args.iter().map(|a| &a.value))
                .for_each(|e| self.changed(e));
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Name(n) => self.read(n),
            Expr::Call { callee, args } => {
                let plain = callee.split('<').next().unwrap_or(callee);
                let (obj, m) = plain.rsplit_once('.').unwrap_or(("", plain));
                // x.push(…) or array.push(x, …)
                if CHANGES.contains(&m) && !obj.is_empty() {
                    if matches!(obj, "array" | "map" | "matrix") {
                        if let Some(a) = args.first() {
                            self.changed(&a.value);
                        }
                    } else {
                        self.write(obj);
                    }
                }
                if !obj.is_empty() {
                    self.read(obj);
                    let recv = Expr::Name(obj.to_string());
                    self.call(m, Some(&recv), args);
                }
                self.call(plain, None, args);
                args.iter().for_each(|a| self.expr(&a.value));
            }
            Expr::MethodCall { recv, name, args } => {
                if CHANGES.contains(&name.as_str()) {
                    self.changed(recv);
                }
                self.call(name, Some(recv), args);
                self.expr(recv);
                args.iter().for_each(|a| self.expr(&a.value));
            }
            Expr::Index(a, b) | Expr::Binary(_, a, b) => {
                self.expr(a);
                self.expr(b);
            }
            Expr::Dot(a, _) | Expr::Unary(_, a) => self.expr(a),
            Expr::Ternary(a, b, c) => [a, b, c].into_iter().for_each(|x| self.expr(x)),
            Expr::If(branches, o) => {
                for (c, b) in branches {
                    self.expr(c);
                    self.block(b);
                }
                if let Some(b) = o {
                    self.block(b);
                }
            }
            Expr::Switch(subj, cases) => {
                if let Some(x) = subj {
                    self.expr(x);
                }
                for (c, b) in cases {
                    if let Some(c) = c {
                        self.expr(c);
                    }
                    self.block(b);
                }
            }
            Expr::Loop(s) => self.stmt(s),
            Expr::Tuple(xs) => xs.iter().for_each(|x| self.expr(x)),
            _ => {}
        }
    }
}

impl Program {
    pub fn new(b: &Block) -> Program {
        let mut fns: HashMap<String, FnUse> = HashMap::new();
        // Functions that call functions that change what they're given…
        for _ in 0..8 {
            let was: usize = fns.values().filter(|f| f.changes_args).count();
            let mut next: HashMap<String, FnUse> = HashMap::new();
            for s in b {
                if let Stmt::Func { name, params, body, .. } = s {
                    let mut w = Walk {
                        fns: &fns,
                        u: Use::default(),
                        locals: Vec::new(),
                    };
                    w.block(body);
                    let f = next.entry(name.clone()).or_default();
                    f.changes_args |= params.iter().any(|p| w.u.writes.contains(&p.0));
                    f.u.reads.extend(w.u.reads);
                    f.u.writes.extend(w.u.writes);
                    f.u.calls.extend(w.u.calls);
                }
            }
            let now = next.values().filter(|f| f.changes_args).count();
            fns = next;
            if now == was && was > 0 {
                break;
            }
        }
        // …and what they read, through the functions they call.
        loop {
            let mut grew = false;
            let names: Vec<String> = fns.keys().cloned().collect();
            for n in &names {
                let called: Vec<Use> = fns[n]
                    .u
                    .calls
                    .iter()
                    .filter_map(|c| fns.get(c))
                    .map(|f| f.u.clone())
                    .collect();
                if let Some(f) = fns.get_mut(n) {
                    let before = f.u.reads.len() + f.u.writes.len() + f.u.calls.len();
                    for c in called {
                        f.u.reads.extend(c.reads);
                        f.u.writes.extend(c.writes);
                        f.u.calls.extend(c.calls);
                    }
                    grew |= f.u.reads.len() + f.u.writes.len() + f.u.calls.len() > before;
                }
            }
            if !grew {
                break;
            }
        }
        let mut p = Program { stmts: Vec::new(), fns };
        p.stmts = b
            .iter()
            .map(|s| match s {
                Stmt::Func { .. } => Use::default(),
                s => {
                    let mut w = Walk {
                        fns: &p.fns,
                        u: Use::default(),
                        locals: Vec::new(),
                    };
                    w.stmt(s);
                    p.through(w.u)
                }
            })
            .collect();
        p
    }

    /// With what the functions it calls read and change.
    fn through(&self, mut u: Use) -> Use {
        for f in u.calls.clone().iter().filter_map(|c| self.fns.get(c)) {
            u.reads.extend(f.u.reads.iter().cloned());
            u.writes.extend(f.u.writes.iter().cloned());
        }
        u
    }

    /// The names an expression depends on, through functions.
    pub fn names<'a>(&self, es: impl Iterator<Item = &'a Expr>) -> Names {
        let mut w = Walk {
            fns: &self.fns,
            u: Use::default(),
            locals: Vec::new(),
        };
        es.for_each(|e| w.expr(e));
        self.through(w.u).reads
    }

    /// What `names`, read in a function's body, depend on outside it: its
    /// locals and parameters are followed to what they're made of.
    pub fn outside(&self, mut names: Names, params: &[Param], args: &[std::rc::Rc<Expr>], body: &Block) -> Names {
        let uses: Vec<Use> = body
            .iter()
            .map(|s| {
                let mut w = Walk {
                    fns: &self.fns,
                    u: Use::default(),
                    locals: Vec::new(),
                };
                w.stmt(s);
                self.through(w.u)
            })
            .collect();
        let mut locals = Names::new();
        loop {
            let before = names.len() + locals.len();
            for u in &uses {
                if u.writes.iter().any(|w| names.contains(w)) {
                    names.extend(u.reads.iter().cloned());
                }
            }
            for s in body {
                if let Stmt::Decl { names: n, .. } = s {
                    locals.extend(n.iter().filter(|n| names.contains(*n)).cloned());
                }
            }
            if names.len() + locals.len() == before {
                break;
            }
        }
        for n in &locals {
            names.remove(n);
        }
        for (p, a) in params.iter().zip(args) {
            if names.remove(&p.0) {
                names.extend(self.names(std::iter::once(&**a)));
            }
        }
        names
    }

    /// The statements (but the one at `at`) that `names` depend on, in order.
    pub fn needed(&self, at: usize, mut names: Names) -> Vec<usize> {
        let mut take = vec![false; self.stmts.len()];
        loop {
            let mut grew = false;
            for (i, u) in self.stmts.iter().enumerate().filter(|(i, _)| *i != at) {
                if !take[i] && u.writes.iter().any(|w| names.contains(w)) {
                    take[i] = true;
                    grew = true;
                    names.extend(u.reads.iter().cloned());
                }
            }
            if !grew {
                break;
            }
        }
        (0..self.stmts.len()).filter(|i| take[*i]).collect()
    }
}

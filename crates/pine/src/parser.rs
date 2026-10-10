//! Parses logical lines into statements. Blocks are lines indented one
//! level deeper than the line that opens them.

use crate::Error;
use crate::ast::*;
use crate::lexer::{Line, Tok};

struct Cursor<'a> {
    toks: &'a [Tok],
    pos: usize,
    line: usize,
}

impl<'a> Cursor<'a> {
    fn peek(&self) -> Option<&'a Tok> {
        self.toks.get(self.pos)
    }

    fn peek_at(&self, k: usize) -> Option<&'a Tok> {
        self.toks.get(self.pos + k)
    }

    fn next(&mut self) -> Option<&'a Tok> {
        let t = self.toks.get(self.pos);
        self.pos += 1;
        t
    }

    fn done(&self) -> bool {
        self.pos >= self.toks.len()
    }

    fn eat_op(&mut self, op: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Op(o)) if *o == op) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_word(&mut self, w: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Ident(x)) if x == w) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_op(&mut self, op: &str) -> Result<(), Error> {
        if self.eat_op(op) {
            Ok(())
        } else {
            Err(self.err(format!("Expected '{op}'.")))
        }
    }

    fn err(&self, msg: impl Into<String>) -> Error {
        Error::at(self.line, msg)
    }

    fn ident(&mut self, what: &str) -> Result<String, Error> {
        match self.next() {
            Some(Tok::Ident(n)) => Ok(n.clone()),
            _ => Err(self.err(format!("Expected {what}."))),
        }
    }
}

pub struct Parser {
    lines: Vec<Line>,
    i: usize,
}

const QUALIFIERS: [&str; 3] = ["series", "simple", "const"];
/// Words that can't be a type or a variable name.
const KEYWORDS: [&str; 18] = [
    "and", "or", "not", "if", "else", "for", "to", "by", "in", "while", "switch", "var", "varip", "method", "type",
    "export", "import", "true",
];

fn is_name(t: Option<&Tok>) -> bool {
    matches!(t, Some(Tok::Ident(w)) if !KEYWORDS.contains(&w.as_str()) && w != "false")
}

/// Tries to read a type annotation that's followed by a name:
/// `float x`, `series int n`, `array<float> a`, `map<string, int> m`,
/// `float[] a`, `Point p`. Leaves the cursor on the name, or restores it.
fn try_type(c: &mut Cursor) -> Option<String> {
    let save = c.pos;
    while matches!(c.peek(), Some(Tok::Ident(w)) if QUALIFIERS.contains(&w.as_str())) && is_name(c.peek_at(1)) {
        c.pos += 1;
    }
    if !is_name(c.peek()) {
        c.pos = save;
        return None;
    }
    let Some(Tok::Ident(t)) = c.next() else {
        c.pos = save;
        return None;
    };
    let mut ty = t.clone();
    // Dotted types (library.Type).
    while matches!(c.peek(), Some(Tok::Op("."))) && is_name(c.peek_at(1)) {
        c.pos += 1;
        if let Some(Tok::Ident(p)) = c.next() {
            ty.push('.');
            ty.push_str(p);
        }
    }
    if c.eat_op("<") {
        let mut depth = 1;
        ty.push('<');
        while depth > 0 {
            match c.next() {
                Some(Tok::Op("<")) => {
                    depth += 1;
                    ty.push('<');
                }
                Some(Tok::Op(">")) => {
                    depth -= 1;
                    ty.push('>');
                }
                Some(Tok::Ident(w)) => ty.push_str(w),
                Some(Tok::Op(",")) => ty.push(','),
                _ => {
                    c.pos = save;
                    return None;
                }
            }
        }
    } else if matches!(c.peek(), Some(Tok::Op("["))) && matches!(c.peek_at(1), Some(Tok::Op("]"))) {
        c.pos += 2;
        ty = format!("array<{ty}>");
    }
    if is_name(c.peek()) {
        Some(ty)
    } else {
        c.pos = save;
        None
    }
}

impl Parser {
    pub fn new(lines: Vec<Line>) -> Parser {
        // `a = 1, b = 2` declares two variables: split at top-level commas.
        let mut out = Vec::with_capacity(lines.len());
        for l in lines {
            let mut depth = 0i32;
            let mut parts: Vec<Vec<Tok>> = vec![Vec::new()];
            let mut prev: Option<Tok> = None;
            for t in l.toks {
                // Generic arguments: map.new<string, float>, map<int, int> m.
                let generic_open = t == Tok::Op("<")
                    && matches!(&prev, Some(Tok::Ident(w)) if matches!(w.as_str(), "new" | "array" | "map" | "matrix"));
                match &t {
                    Tok::Op("(" | "[") => depth += 1,
                    Tok::Op(")" | "]") => depth -= 1,
                    Tok::Op("<") if generic_open => depth += 1,
                    Tok::Op(">") if depth > 0 && generic_depth(&parts) > 0 => depth -= 1,
                    _ => {}
                }
                prev = Some(t.clone());
                if depth == 0 && t == Tok::Op(",") {
                    parts.push(Vec::new());
                } else if let Some(p) = parts.last_mut() {
                    p.push(t);
                }
            }
            for toks in parts.into_iter().filter(|p| !p.is_empty()) {
                out.push(Line {
                    indent: l.indent,
                    toks,
                    line: l.line,
                });
            }
        }
        Parser { lines: out, i: 0 }
    }

    pub fn program(&mut self) -> Result<Block, Error> {
        self.block(0)
    }

    /// Statements at exactly `indent`, until a line indented less.
    fn block(&mut self, indent: usize) -> Result<Block, Error> {
        let mut out = Vec::new();
        while self.i < self.lines.len() {
            let l = &self.lines[self.i];
            if l.indent < indent {
                break;
            }
            if l.indent > indent {
                return Err(Error::at(l.line, "This line is indented more than it should be."));
            }
            out.push(self.statement(indent)?);
        }
        Ok(out)
    }

    /// The indented block after the current line, if any.
    fn sub_block(&mut self, indent: usize) -> Result<Block, Error> {
        if self.i < self.lines.len() && self.lines[self.i].indent > indent {
            let inner = self.lines[self.i].indent;
            if inner != indent + 1 {
                return Err(Error::at(
                    self.lines[self.i].line,
                    "Indent blocks by 4 spaces (or one tab).",
                ));
            }
            self.block(inner)
        } else {
            Ok(Vec::new())
        }
    }

    fn statement(&mut self, indent: usize) -> Result<Stmt, Error> {
        let line = self.lines[self.i].clone();
        self.i += 1;
        let mut c = Cursor {
            toks: &line.toks,
            pos: 0,
            line: line.line,
        };
        let ln = line.line;
        // Library exports are ordinary declarations here.
        c.eat_word("export");
        if c.eat_word("import") {
            let mut path = String::new();
            while let Some(t) = c.peek() {
                match t {
                    Tok::Ident(w) if w == "as" => break,
                    Tok::Ident(w) => path.push_str(w),
                    Tok::Int(i) => path.push_str(&i.to_string()),
                    Tok::Op(o) => path.push_str(o),
                    _ => {}
                }
                c.pos += 1;
            }
            let alias = if c.eat_word("as") {
                c.ident("a name after 'as'")?
            } else {
                path.split('/').nth(1).unwrap_or("lib").to_string()
            };
            return Ok(Stmt::Import { path, alias, line: ln });
        }
        if matches!(c.peek(), Some(Tok::Ident(w)) if w == "type") && is_name(c.peek_at(1)) && c.toks.len() - c.pos == 2
        {
            c.pos += 1;
            let name = c.ident("the type's name")?;
            let mut fields = Vec::new();
            for f in self.sub_lines(indent) {
                let mut fc = Cursor {
                    toks: &f.toks,
                    pos: 0,
                    line: f.line,
                };
                let _ = try_type(&mut fc);
                let fname = fc.ident("a field name")?;
                let default = if fc.eat_op("=") {
                    Some(Self::expr(&mut fc)?)
                } else {
                    None
                };
                fields.push((fname, default));
            }
            return Ok(Stmt::Type { name, fields, line: ln });
        }
        if matches!(c.peek(), Some(Tok::Ident(w)) if w == "enum") && is_name(c.peek_at(1)) && c.toks.len() - c.pos == 2
        {
            c.pos += 1;
            let name = c.ident("the enum's name")?;
            let mut members = Vec::new();
            for f in self.sub_lines(indent) {
                let mut fc = Cursor {
                    toks: &f.toks,
                    pos: 0,
                    line: f.line,
                };
                let m = fc.ident("a member name")?;
                let title = if fc.eat_op("=") {
                    match fc.peek() {
                        Some(Tok::Str(s)) => {
                            let s = s.to_string();
                            fc.pos += 1;
                            s
                        }
                        _ => return Err(fc.err("An enum member's title must be a string.")),
                    }
                } else {
                    m.clone()
                };
                if !fc.done() {
                    return Err(fc.err("Expected one enum member per line: name or name = \"Title\"."));
                }
                members.push((m, title));
            }
            if members.is_empty() {
                return Err(c.err("An enum needs at least one member."));
            }
            return Ok(Stmt::Enum {
                name,
                members,
                line: ln,
            });
        }
        let method = matches!(c.peek(), Some(Tok::Ident(w)) if w == "method") && is_name(c.peek_at(1));
        if method {
            c.pos += 1;
        }
        // Function definition: name(params) => …
        if let (Some(Tok::Ident(name)), Some(Tok::Op("("))) = (c.peek(), c.peek_at(1))
            && Self::is_func_def(&c.toks[c.pos..])
        {
            let name = name.clone();
            c.pos += 2;
            let mut params = Vec::new();
            while !c.eat_op(")") {
                let ty = try_type(&mut c);
                let p = c.ident("a parameter name")?;
                let default = if c.eat_op("=") { Some(Self::expr(&mut c)?) } else { None };
                params.push((p, default, ty));
                if !c.eat_op(",") && !matches!(c.peek(), Some(Tok::Op(")"))) {
                    return Err(c.err("Expected ',' or ')' in the parameters."));
                }
            }
            c.expect_op("=>")?;
            let body = if c.done() {
                self.sub_block(indent)?
            } else {
                let e = self.expr_tail(&mut c, indent)?;
                vec![Stmt::Expr(e, ln)]
            };
            return Ok(Stmt::Func {
                name,
                params,
                body,
                line: ln,
                method,
            });
        }
        if method {
            return Err(c.err("Expected a method definition: method name(Type this, …) =>"));
        }
        if c.eat_word("break") {
            return Ok(Stmt::Break);
        }
        if c.eat_word("continue") {
            return Ok(Stmt::Continue);
        }
        if c.eat_word("for") {
            // for [i, x] in arr
            if c.eat_op("[") {
                let i = c.ident("the index variable")?;
                c.expect_op(",")?;
                let v = c.ident("the item variable")?;
                c.expect_op("]")?;
                if !c.eat_word("in") {
                    return Err(c.err("Expected 'in'."));
                }
                let over = Self::expr(&mut c)?;
                let body = self.sub_block(indent)?;
                return Ok(Stmt::ForIn {
                    idx: Some(i),
                    var: v,
                    over,
                    body,
                    line: ln,
                });
            }
            let var = c.ident("the loop variable")?;
            if c.eat_word("in") {
                let over = Self::expr(&mut c)?;
                let body = self.sub_block(indent)?;
                return Ok(Stmt::ForIn {
                    idx: None,
                    var,
                    over,
                    body,
                    line: ln,
                });
            }
            c.expect_op("=")?;
            let from = Self::expr(&mut c)?;
            if !c.eat_word("to") {
                return Err(c.err("Expected 'to'."));
            }
            let to = Self::expr(&mut c)?;
            let step = if c.eat_word("by") {
                Some(Self::expr(&mut c)?)
            } else {
                None
            };
            let body = self.sub_block(indent)?;
            return Ok(Stmt::For {
                var,
                from,
                to,
                step,
                body,
                line: ln,
            });
        }
        if c.eat_word("while") {
            let cond = Self::expr(&mut c)?;
            let body = self.sub_block(indent)?;
            return Ok(Stmt::While { cond, body, line: ln });
        }
        // Declarations: [var|varip] [type] name = …, or [a, b] = …
        let save = c.pos;
        let persist = if c.eat_word("varip") {
            Persist::Varip
        } else if c.eat_word("var") {
            Persist::Var
        } else {
            Persist::No
        };
        let ty = try_type(&mut c);
        if ty.is_none() && c.eat_op("[") {
            // [a, b] = … only if the brackets hold plain names.
            let mut names = Vec::new();
            let mut ok = true;
            loop {
                match c.peek() {
                    Some(Tok::Ident(n)) => {
                        names.push(n.clone());
                        c.pos += 1;
                    }
                    _ => {
                        ok = false;
                        break;
                    }
                }
                if c.eat_op("]") {
                    break;
                }
                if !c.eat_op(",") {
                    ok = false;
                    break;
                }
            }
            if ok && c.eat_op("=") {
                let value = self.expr_tail(&mut c, indent)?;
                return Ok(Stmt::Decl {
                    persist,
                    names,
                    ty: None,
                    value,
                    line: ln,
                });
            }
            if persist != Persist::No {
                return Err(c.err("Expected '=' after the names."));
            }
            // Not a tuple declaration: an expression starting with '['.
            c.pos = save;
        } else {
            // A possibly dotted name followed by an assignment operator.
            let start = c.pos;
            let mut name = String::new();
            if let Some(Tok::Ident(n)) = c.peek() {
                name = n.clone();
                c.pos += 1;
                while matches!(c.peek(), Some(Tok::Op("."))) && matches!(c.peek_at(1), Some(Tok::Ident(_))) {
                    c.pos += 1;
                    if let Some(Tok::Ident(p)) = c.next() {
                        name.push('.');
                        name.push_str(p);
                    }
                }
            }
            if !name.is_empty()
                && let Some(Tok::Op(op)) = c.peek()
            {
                match *op {
                    "=" if !name.contains('.') => {
                        c.pos += 1;
                        let value = self.expr_tail(&mut c, indent)?;
                        return Ok(Stmt::Decl {
                            persist,
                            names: vec![name],
                            ty,
                            value,
                            line: ln,
                        });
                    }
                    ":=" | "+=" | "-=" | "*=" | "/=" | "%=" if persist == Persist::No && ty.is_none() => {
                        c.pos += 1;
                        let value = self.expr_tail(&mut c, indent)?;
                        return Ok(Stmt::Assign {
                            name,
                            op,
                            value,
                            line: ln,
                        });
                    }
                    _ => {}
                }
            }
            c.pos = start;
        }
        if persist != Persist::No || ty.is_some() {
            return Err(c.err("Expected a name and '=' here."));
        }
        c.pos = save;
        let e = self.expr_tail(&mut c, indent)?;
        Ok(Stmt::Expr(e, ln))
    }

    /// The lines of the block below the current line, raw (type fields).
    fn sub_lines(&mut self, indent: usize) -> Vec<Line> {
        let mut out = Vec::new();
        while self.i < self.lines.len() && self.lines[self.i].indent > indent {
            out.push(self.lines[self.i].clone());
            self.i += 1;
        }
        out
    }

    fn is_func_def(toks: &[Tok]) -> bool {
        // name ( … ) => with the closing bracket matching the first one.
        let mut depth = 0;
        for (k, t) in toks.iter().enumerate().skip(1) {
            match t {
                Tok::Op("(") => depth += 1,
                Tok::Op(")") => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(toks.get(k + 1), Some(Tok::Op("=>")));
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// An expression that may be an `if` or `switch` with blocks below.
    fn expr_tail(&mut self, c: &mut Cursor, indent: usize) -> Result<Expr, Error> {
        if c.eat_word("if") {
            return self.if_expr(c, indent);
        }
        if c.eat_word("switch") {
            let subject = if c.done() { None } else { Some(Box::new(Self::expr(c)?)) };
            let mut cases = Vec::new();
            if self.i < self.lines.len() && self.lines[self.i].indent == indent + 1 {
                while self.i < self.lines.len() && self.lines[self.i].indent == indent + 1 {
                    let line = self.lines[self.i].clone();
                    self.i += 1;
                    let mut lc = Cursor {
                        toks: &line.toks,
                        pos: 0,
                        line: line.line,
                    };
                    let cond = if lc.eat_op("=>") {
                        None
                    } else {
                        let e = Self::expr(&mut lc)?;
                        lc.expect_op("=>")?;
                        Some(e)
                    };
                    let body = if lc.done() {
                        self.sub_block(indent + 1)?
                    } else {
                        vec![self.case_stmt(&mut lc, indent + 1, line.line)?]
                    };
                    cases.push((cond, body));
                }
            }
            return Ok(Expr::Switch(subject, cases));
        }
        let e = Self::expr(c)?;
        if !c.done() {
            return Err(c.err(format!("Unexpected {} here.", describe(c.peek().expect("not done")))));
        }
        Ok(e)
    }

    /// A one-line switch case body: an expression, or an assignment
    /// (`=> x := 1`).
    fn case_stmt(&mut self, c: &mut Cursor, indent: usize, ln: usize) -> Result<Stmt, Error> {
        if let (Some(Tok::Ident(name)), Some(Tok::Op(op))) = (c.peek(), c.peek_at(1))
            && matches!(*op, ":=" | "+=" | "-=" | "*=" | "/=" | "%=")
        {
            let name = name.clone();
            c.pos += 2;
            let value = self.expr_tail(c, indent)?;
            return Ok(Stmt::Assign {
                name,
                op,
                value,
                line: ln,
            });
        }
        Ok(Stmt::Expr(self.expr_tail(c, indent)?, ln))
    }

    fn if_expr(&mut self, c: &mut Cursor, indent: usize) -> Result<Expr, Error> {
        let cond = Self::expr(c)?;
        if !c.done() {
            return Err(c.err("Put the 'if' body on the following lines, indented."));
        }
        let mut branches = vec![(cond, self.sub_block(indent)?)];
        let mut otherwise = None;
        while self.i < self.lines.len() && self.lines[self.i].indent == indent {
            let line = self.lines[self.i].clone();
            if !matches!(line.toks.first(), Some(Tok::Ident(w)) if w == "else") {
                break;
            }
            self.i += 1;
            let mut lc = Cursor {
                toks: &line.toks,
                pos: 1,
                line: line.line,
            };
            if lc.eat_word("if") {
                let cond = Self::expr(&mut lc)?;
                branches.push((cond, self.sub_block(indent)?));
            } else {
                otherwise = Some(self.sub_block(indent)?);
                break;
            }
        }
        Ok(Expr::If(branches, otherwise))
    }

    // ---- expressions, by precedence

    fn expr(c: &mut Cursor) -> Result<Expr, Error> {
        Self::ternary(c)
    }

    fn ternary(c: &mut Cursor) -> Result<Expr, Error> {
        let cond = Self::or(c)?;
        if c.eat_op("?") {
            let a = Self::ternary(c)?;
            c.expect_op(":")?;
            let b = Self::ternary(c)?;
            return Ok(Expr::Ternary(Box::new(cond), Box::new(a), Box::new(b)));
        }
        Ok(cond)
    }

    fn or(c: &mut Cursor) -> Result<Expr, Error> {
        let mut e = Self::and(c)?;
        while c.eat_word("or") {
            e = Expr::Binary("or", Box::new(e), Box::new(Self::and(c)?));
        }
        Ok(e)
    }

    fn and(c: &mut Cursor) -> Result<Expr, Error> {
        let mut e = Self::not(c)?;
        while c.eat_word("and") {
            e = Expr::Binary("and", Box::new(e), Box::new(Self::not(c)?));
        }
        Ok(e)
    }

    fn not(c: &mut Cursor) -> Result<Expr, Error> {
        if c.eat_word("not") {
            return Ok(Expr::Unary("not", Box::new(Self::not(c)?)));
        }
        Self::equality(c)
    }

    fn equality(c: &mut Cursor) -> Result<Expr, Error> {
        let mut e = Self::comparison(c)?;
        while let Some(Tok::Op(o @ ("==" | "!="))) = c.peek() {
            let op = *o;
            c.pos += 1;
            e = Expr::Binary(op, Box::new(e), Box::new(Self::comparison(c)?));
        }
        Ok(e)
    }

    fn comparison(c: &mut Cursor) -> Result<Expr, Error> {
        let mut e = Self::additive(c)?;
        while let Some(Tok::Op(o @ ("<" | ">" | "<=" | ">="))) = c.peek() {
            let op = *o;
            c.pos += 1;
            e = Expr::Binary(op, Box::new(e), Box::new(Self::additive(c)?));
        }
        Ok(e)
    }

    fn additive(c: &mut Cursor) -> Result<Expr, Error> {
        let mut e = Self::multiplicative(c)?;
        while let Some(Tok::Op(o @ ("+" | "-"))) = c.peek() {
            let op = *o;
            c.pos += 1;
            e = Expr::Binary(op, Box::new(e), Box::new(Self::multiplicative(c)?));
        }
        Ok(e)
    }

    fn multiplicative(c: &mut Cursor) -> Result<Expr, Error> {
        let mut e = Self::unary(c)?;
        while let Some(Tok::Op(o @ ("*" | "/" | "%"))) = c.peek() {
            let op = *o;
            c.pos += 1;
            e = Expr::Binary(op, Box::new(e), Box::new(Self::unary(c)?));
        }
        Ok(e)
    }

    fn unary(c: &mut Cursor) -> Result<Expr, Error> {
        if c.eat_op("-") {
            return Ok(Expr::Unary("-", Box::new(Self::unary(c)?)));
        }
        if c.eat_op("+") {
            return Self::unary(c);
        }
        if c.eat_op("!") {
            return Ok(Expr::Unary("not", Box::new(Self::unary(c)?)));
        }
        Self::postfix(c)
    }

    fn postfix(c: &mut Cursor) -> Result<Expr, Error> {
        let mut e = Self::primary(c)?;
        loop {
            if c.eat_op("[") {
                let i = Self::expr(c)?;
                c.expect_op("]")?;
                e = Expr::Index(Box::new(e), Box::new(i));
            } else if matches!(c.peek(), Some(Tok::Op("."))) && matches!(c.peek_at(1), Some(Tok::Ident(_))) {
                c.pos += 1;
                let name = c.ident("a field or method name")?;
                if c.eat_op("(") {
                    let args = Self::args(c)?;
                    e = Expr::MethodCall {
                        recv: Box::new(e),
                        name,
                        args,
                    };
                } else {
                    e = Expr::Dot(Box::new(e), name);
                }
            } else {
                break;
            }
        }
        Ok(e)
    }

    /// Call arguments after the opening bracket.
    fn args(c: &mut Cursor) -> Result<Vec<Arg>, Error> {
        let mut args = Vec::new();
        if c.eat_op(")") {
            return Ok(args);
        }
        loop {
            let named = matches!((c.peek(), c.peek_at(1)), (Some(Tok::Ident(_)), Some(Tok::Op("="))));
            let name = if named {
                let n = c.ident("an argument name")?;
                c.pos += 1;
                Some(n)
            } else {
                None
            };
            args.push(Arg {
                name,
                value: Self::expr(c)?,
            });
            if c.eat_op(")") {
                break;
            }
            c.expect_op(",")?;
        }
        Ok(args)
    }

    fn primary(c: &mut Cursor) -> Result<Expr, Error> {
        let Some(t) = c.next() else {
            return Err(c.err("The line ends too early."));
        };
        match t {
            Tok::Int(v) => Ok(Expr::Int(*v)),
            Tok::Float(v) => Ok(Expr::Float(*v)),
            Tok::Str(s) => Ok(Expr::Str(s.clone())),
            Tok::Color(v) => Ok(Expr::Color(*v)),
            Tok::Op("(") => {
                let e = Self::expr(c)?;
                c.expect_op(")")?;
                Ok(e)
            }
            Tok::Op("[") => {
                let mut items = Vec::new();
                if !c.eat_op("]") {
                    loop {
                        items.push(Self::expr(c)?);
                        if c.eat_op("]") {
                            break;
                        }
                        c.expect_op(",")?;
                    }
                }
                Ok(Expr::Tuple(items))
            }
            Tok::Ident(w) => {
                match w.as_str() {
                    "true" => return Ok(Expr::Bool(true)),
                    "false" => return Ok(Expr::Bool(false)),
                    "na" if !matches!(c.peek(), Some(Tok::Op("("))) => return Ok(Expr::Na),
                    "if" | "switch" => {
                        return Err(c.err(format!(
                            "Put '{w}' at the start of a line, with its body indented below."
                        )));
                    }
                    _ => {}
                }
                let mut name = w.clone();
                while matches!(c.peek(), Some(Tok::Op("."))) && matches!(c.peek_at(1), Some(Tok::Ident(_))) {
                    c.pos += 1;
                    if let Some(Tok::Ident(part)) = c.next() {
                        name.push('.');
                        name.push_str(part);
                    }
                }
                // Generic type arguments (array.new<float>, map.new<string, int>).
                if name.ends_with(".new") && matches!(c.peek(), Some(Tok::Op("<"))) {
                    let mut depth = 0;
                    let mut k = c.pos;
                    while let Some(t) = c.toks.get(k) {
                        match t {
                            Tok::Op("<") => depth += 1,
                            Tok::Op(">") => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            Tok::Ident(_) | Tok::Op(",") | Tok::Op(".") => {}
                            _ => break,
                        }
                        k += 1;
                    }
                    if depth == 0 && matches!(c.toks.get(k + 1), Some(Tok::Op("("))) {
                        let generic: String = c.toks[c.pos + 1..k]
                            .iter()
                            .map(|t| match t {
                                Tok::Ident(w) => w.clone(),
                                Tok::Op(o) => o.to_string(),
                                _ => String::new(),
                            })
                            .collect();
                        c.pos = k + 1;
                        // Keep the element type for array.new<string> etc.
                        name = format!("{name}<{generic}>");
                    }
                }
                if c.eat_op("(") {
                    let args = Self::args(c)?;
                    return Ok(Expr::Call { callee: name, args });
                }
                Ok(Expr::Name(name))
            }
            other => Err(c.err(format!("Unexpected {}.", describe(other)))),
        }
    }
}

/// Open generic brackets in the statement being split.
fn generic_depth(parts: &[Vec<Tok>]) -> i32 {
    let toks = parts.last().map(|p| p.as_slice()).unwrap_or(&[]);
    let mut d = 0;
    for w in toks.windows(2) {
        if w[1] == Tok::Op("<")
            && matches!(&w[0], Tok::Ident(x) if matches!(x.as_str(), "new" | "array" | "map" | "matrix"))
        {
            d += 1;
        } else if w[1] == Tok::Op(">") && d > 0 {
            d -= 1;
        }
    }
    d
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(w) => format!("\"{w}\""),
        Tok::Int(i) => i.to_string(),
        Tok::Float(f) => f.to_string(),
        Tok::Str(s) => format!("\"{s}\""),
        Tok::Color(c) => format!("#{c:08X}"),
        Tok::Op(o) => format!("'{o}'"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lines;

    fn parse(s: &str) -> Block {
        Parser::new(lines(s).unwrap()).program().unwrap()
    }

    #[test]
    fn statements() {
        let b = parse(
            "indicator(\"T\", overlay = true)\nvar float acc = 0.0\nacc := acc + 1\n[m, s, h] = ta.macd(close, 12, 26, 9)\n\
             f(x, n = 2) => x * n\ng(a) =>\n    b = a + 1\n    b * 2\nx = if close > open\n    1\nelse if close < open\n    -1\nelse\n    0\n\
             for i = 0 to 3 by 1\n    acc += i\ny = switch\n    close > 1 => 1\n    => 0\nplot(close[1] + f(2), color = close > open ? color.green : color.red)",
        );
        assert_eq!(b.len(), 10);
        assert!(
            matches!(&b[1], Stmt::Decl { persist: Persist::Var, names, ty: Some(t), .. } if names == &["acc"] && t == "float")
        );
        assert!(matches!(&b[2], Stmt::Assign { op: ":=", .. }));
        assert!(matches!(&b[3], Stmt::Decl { names, .. } if names.len() == 3));
        assert!(matches!(&b[4], Stmt::Func { params, .. } if params.len() == 2 && params[1].1.is_some()));
        assert!(matches!(&b[5], Stmt::Func { body, .. } if body.len() == 2));
        assert!(matches!(&b[6], Stmt::Decl { value: Expr::If(br, Some(_)), .. } if br.len() == 2));
        assert!(matches!(&b[7], Stmt::For { body, .. } if body.len() == 1));
        assert!(matches!(&b[8], Stmt::Decl { value: Expr::Switch(None, cases), .. } if cases.len() == 2));
        let Stmt::Expr(Expr::Call { args, .. }, _) = &b[9] else {
            panic!()
        };
        assert_eq!(args[1].name.as_deref(), Some("color"));
        assert!(matches!(args[1].value, Expr::Ternary(..)));
    }

    #[test]
    fn types_methods_and_collections() {
        let b = parse(
            "type Point\n    float x = 0.0\n    int y\nmethod shift(Point this, float d) =>\n    this.x += d\n\
             Point p = Point.new(1.0, 2)\np.x := 3\na = array.new<float>(0)\nm = map.new<string, float>()\n\
             for [i, v] in a\n    p.shift(v)\nfor v in a\n    x = v\nv = a.get(0).x\nexport f(series float s) => s",
        );
        assert!(matches!(&b[0], Stmt::Type { fields, .. } if fields.len() == 2 && fields[0].1.is_some()));
        assert!(matches!(&b[1], Stmt::Func { method: true, params, .. } if params[0].2.as_deref() == Some("Point")));
        assert!(matches!(&b[2], Stmt::Decl { ty: Some(t), .. } if t == "Point"));
        assert!(matches!(&b[3], Stmt::Assign { name, .. } if name == "p.x"));
        assert!(matches!(&b[4], Stmt::Decl { value: Expr::Call { callee, .. }, .. } if callee == "array.new<float>"));
        assert!(
            matches!(&b[5], Stmt::Decl { value: Expr::Call { callee, .. }, .. } if callee == "map.new<string,float>")
        );
        assert!(matches!(&b[6], Stmt::ForIn { idx: Some(_), .. }));
        assert!(matches!(&b[7], Stmt::ForIn { idx: None, .. }));
        assert!(matches!(
            &b[8],
            Stmt::Decl {
                value: Expr::Dot(..),
                ..
            }
        ));
        assert!(matches!(&b[9], Stmt::Func { params, .. } if params[0].2.as_deref() == Some("float")));
    }

    #[test]
    fn errors_have_lines() {
        let e = Parser::new(lines("x = 1\nx = (2\n").unwrap()).program().unwrap_err();
        assert_eq!(e.line, 2);
    }

    #[test]
    fn comparisons_are_not_generics() {
        let b = parse("x = close < open\ny = a.new < b");
        assert!(matches!(
            &b[0],
            Stmt::Decl {
                value: Expr::Binary("<", ..),
                ..
            }
        ));
        assert!(matches!(
            &b[1],
            Stmt::Decl {
                value: Expr::Binary("<", ..),
                ..
            }
        ));
    }
}

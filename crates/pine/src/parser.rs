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
}

pub struct Parser {
    lines: Vec<Line>,
    i: usize,
}

const TYPES: [&str; 8] = ["int", "float", "bool", "string", "color", "line", "label", "box"];
const QUALIFIERS: [&str; 3] = ["series", "simple", "const"];

impl Parser {
    pub fn new(lines: Vec<Line>) -> Parser {
        Parser { lines, i: 0 }
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
                return Err(Error::at(self.lines[self.i].line, "Indent blocks by 4 spaces (or one tab)."));
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
        // Function definition: name(params) => …
        if let (Some(Tok::Ident(name)), Some(Tok::Op("("))) = (c.peek(), c.peek_at(1))
            && Self::is_func_def(&line.toks)
        {
            let name = name.clone();
            c.pos += 2;
            let mut params = Vec::new();
            while !c.eat_op(")") {
                // Optional type annotations: `simple int len = 14`.
                while matches!(c.peek_at(1), Some(Tok::Ident(_)))
                    && matches!(c.peek(), Some(Tok::Ident(w)) if TYPES.contains(&w.as_str()) || QUALIFIERS.contains(&w.as_str()))
                {
                    c.pos += 1;
                }
                let Some(Tok::Ident(p)) = c.next() else {
                    return Err(c.err("Expected a parameter name."));
                };
                let default = if c.eat_op("=") { Some(Self::expr(&mut c)?) } else { None };
                params.push((p.clone(), default));
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
            });
        }
        if c.eat_word("break") {
            return Ok(Stmt::Break);
        }
        if c.eat_word("continue") {
            return Ok(Stmt::Continue);
        }
        if c.eat_word("for") {
            let Some(Tok::Ident(var)) = c.next() else {
                return Err(c.err("Expected the loop variable."));
            };
            let var = var.clone();
            if c.eat_word("in") {
                return Err(c.err("for…in loops over arrays aren't supported yet."));
            }
            c.expect_op("=")?;
            let from = Self::expr(&mut c)?;
            if !c.eat_word("to") {
                return Err(c.err("Expected 'to'."));
            }
            let to = Self::expr(&mut c)?;
            let step = if c.eat_word("by") { Some(Self::expr(&mut c)?) } else { None };
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
        while matches!(c.peek(), Some(Tok::Ident(w)) if QUALIFIERS.contains(&w.as_str()))
            && matches!(c.peek_at(1), Some(Tok::Ident(_)))
        {
            c.pos += 1;
        }
        if matches!(c.peek(), Some(Tok::Ident(w)) if TYPES.contains(&w.as_str()))
            && matches!(c.peek_at(1), Some(Tok::Ident(_)))
        {
            c.pos += 1;
        }
        if c.eat_op("[") {
            let mut names = Vec::new();
            loop {
                let Some(Tok::Ident(n)) = c.next() else {
                    return Err(c.err("Expected a name in the tuple."));
                };
                names.push(n.clone());
                if c.eat_op("]") {
                    break;
                }
                c.expect_op(",")?;
            }
            c.expect_op("=")?;
            let value = self.expr_tail(&mut c, indent)?;
            return Ok(Stmt::Decl {
                persist,
                names,
                value,
                line: ln,
            });
        }
        if let (Some(Tok::Ident(name)), Some(Tok::Op(op))) = (c.peek(), c.peek_at(1)) {
            let name = name.clone();
            match *op {
                "=" => {
                    c.pos += 2;
                    let value = self.expr_tail(&mut c, indent)?;
                    return Ok(Stmt::Decl {
                        persist,
                        names: vec![name],
                        value,
                        line: ln,
                    });
                }
                ":=" | "+=" | "-=" | "*=" | "/=" | "%=" if persist == Persist::No => {
                    c.pos += 2;
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
        if persist != Persist::No {
            return Err(c.err("Expected a name after 'var'."));
        }
        c.pos = save;
        let e = self.expr_tail(&mut c, indent)?;
        Ok(Stmt::Expr(e, ln))
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
                        vec![Stmt::Expr(self.expr_tail(&mut lc, indent + 1)?, line.line)]
                    };
                    cases.push((cond, body));
                }
            }
            return Ok(Expr::Switch(subject, cases));
        }
        let e = Self::expr(c)?;
        if !c.done() {
            return Err(c.err(format!("Unexpected {:?} here.", c.peek().expect("not done"))));
        }
        Ok(e)
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
        loop {
            let op = match c.peek() {
                Some(Tok::Op(o @ ("==" | "!="))) => *o,
                _ => break,
            };
            c.pos += 1;
            e = Expr::Binary(op, Box::new(e), Box::new(Self::comparison(c)?));
        }
        Ok(e)
    }

    fn comparison(c: &mut Cursor) -> Result<Expr, Error> {
        let mut e = Self::additive(c)?;
        loop {
            let op = match c.peek() {
                Some(Tok::Op(o @ ("<" | ">" | "<=" | ">="))) => *o,
                _ => break,
            };
            c.pos += 1;
            e = Expr::Binary(op, Box::new(e), Box::new(Self::additive(c)?));
        }
        Ok(e)
    }

    fn additive(c: &mut Cursor) -> Result<Expr, Error> {
        let mut e = Self::multiplicative(c)?;
        loop {
            let op = match c.peek() {
                Some(Tok::Op(o @ ("+" | "-"))) => *o,
                _ => break,
            };
            c.pos += 1;
            e = Expr::Binary(op, Box::new(e), Box::new(Self::multiplicative(c)?));
        }
        Ok(e)
    }

    fn multiplicative(c: &mut Cursor) -> Result<Expr, Error> {
        let mut e = Self::unary(c)?;
        loop {
            let op = match c.peek() {
                Some(Tok::Op(o @ ("*" | "/" | "%"))) => *o,
                _ => break,
            };
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
        while c.eat_op("[") {
            let i = Self::expr(c)?;
            c.expect_op("]")?;
            e = Expr::Index(Box::new(e), Box::new(i));
        }
        Ok(e)
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
                // Generic type arguments (array.new<float>) are skipped.
                if matches!(c.peek(), Some(Tok::Op("<")))
                    && matches!(c.peek_at(1), Some(Tok::Ident(_)))
                    && matches!(c.peek_at(2), Some(Tok::Op(">")))
                {
                    c.pos += 3;
                }
                if c.eat_op("(") {
                    let mut args = Vec::new();
                    if !c.eat_op(")") {
                        loop {
                            let named = matches!(
                                (c.peek(), c.peek_at(1)),
                                (Some(Tok::Ident(_)), Some(Tok::Op("=")))
                            );
                            let name = if named {
                                let Some(Tok::Ident(n)) = c.next() else { unreachable!() };
                                c.pos += 1;
                                Some(n.clone())
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
                    }
                    return Ok(Expr::Call { callee: name, args });
                }
                Ok(Expr::Name(name))
            }
            other => Err(c.err(format!("Unexpected {other:?}."))),
        }
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
        assert!(matches!(&b[1], Stmt::Decl { persist: Persist::Var, names, .. } if names == &["acc"]));
        assert!(matches!(&b[2], Stmt::Assign { op: ":=", .. }));
        assert!(matches!(&b[3], Stmt::Decl { names, .. } if names.len() == 3));
        assert!(matches!(&b[4], Stmt::Func { params, .. } if params.len() == 2 && params[1].1.is_some()));
        assert!(matches!(&b[5], Stmt::Func { body, .. } if body.len() == 2));
        assert!(matches!(&b[6], Stmt::Decl { value: Expr::If(br, Some(_)), .. } if br.len() == 2));
        assert!(matches!(&b[7], Stmt::For { body, .. } if body.len() == 1));
        assert!(matches!(&b[8], Stmt::Decl { value: Expr::Switch(None, cases), .. } if cases.len() == 2));
        let Stmt::Expr(Expr::Call { args, .. }, _) = &b[9] else { panic!() };
        assert_eq!(args[1].name.as_deref(), Some("color"));
        assert!(matches!(args[1].value, Expr::Ternary(..)));
    }

    #[test]
    fn errors_have_lines() {
        let e = Parser::new(lines("x = 1\nx = (2\n").unwrap()).program().unwrap_err();
        assert_eq!(e.line, 2);
    }
}

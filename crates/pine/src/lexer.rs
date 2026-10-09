//! Turns Pine source into logical lines of tokens with their indentation.
//! A statement may wrap: inside brackets, after an operator or comma, or
//! on lines indented by something other than a multiple of four (Pine's
//! rule); wrapped lines are joined.

use crate::Error;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(i64),
    Float(f64),
    Str(String),
    /// 0xRRGGBBAA
    Color(u32),
    Op(&'static str),
}

#[derive(Clone, Debug)]
pub struct Line {
    pub indent: usize,
    pub toks: Vec<Tok>,
    /// 1-based source line where the statement starts.
    pub line: usize,
}

const OPS: [&str; 30] = [
    ":=", "+=", "-=", "*=", "/=", "%=", "==", "!=", "<=", ">=", "=>", "=", "<", ">", "+", "-", "*", "/", "%", "?", ":",
    ",", "(", ")", "[", "]", ".", "!", "{", "}",
];

fn lex_line(s: &str, line: usize) -> Result<Vec<Tok>, Error> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '/' && c.get(i + 1) == Some(&'/') {
            break;
        }
        if ch == '"' || ch == '\'' {
            let mut j = i + 1;
            let mut text = String::new();
            while j < c.len() && c[j] != ch {
                if c[j] == '\\' && j + 1 < c.len() {
                    j += 1;
                    text.push(match c[j] {
                        'n' => '\n',
                        't' => '\t',
                        x => x,
                    });
                } else {
                    text.push(c[j]);
                }
                j += 1;
            }
            if j >= c.len() {
                return Err(Error::at(line, "This string isn't closed."));
            }
            out.push(Tok::Str(text));
            i = j + 1;
            continue;
        }
        if ch == '#' {
            let mut j = i + 1;
            while j < c.len() && c[j].is_ascii_hexdigit() {
                j += 1;
            }
            let hex: String = c[i + 1..j].iter().collect();
            let v = match hex.len() {
                6 => u32::from_str_radix(&hex, 16).map(|v| (v << 8) | 0xFF),
                8 => u32::from_str_radix(&hex, 16),
                _ => return Err(Error::at(line, format!("\"#{hex}\" isn't a colour (use #RRGGBB or #RRGGBBAA)."))),
            }
            .map_err(|_| Error::at(line, "Bad colour."))?;
            out.push(Tok::Color(v));
            i = j;
            continue;
        }
        if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let mut j = i;
            let mut float = false;
            while j < c.len() && (c[j].is_ascii_digit() || c[j] == '.' || c[j] == '_') {
                if c[j] == '.' {
                    float = true;
                }
                j += 1;
            }
            if j < c.len() && (c[j] == 'e' || c[j] == 'E') {
                float = true;
                j += 1;
                if j < c.len() && (c[j] == '+' || c[j] == '-') {
                    j += 1;
                }
                while j < c.len() && c[j].is_ascii_digit() {
                    j += 1;
                }
            }
            let text: String = c[i..j].iter().filter(|x| **x != '_').collect();
            if float {
                out.push(Tok::Float(text.parse().map_err(|_| Error::at(line, format!("Bad number {text}.")))?));
            } else {
                out.push(Tok::Int(text.parse().map_err(|_| Error::at(line, format!("Bad number {text}.")))?));
            }
            i = j;
            continue;
        }
        if ch.is_alphabetic() || ch == '_' {
            let mut j = i;
            while j < c.len() && (c[j].is_alphanumeric() || c[j] == '_') {
                j += 1;
            }
            out.push(Tok::Ident(c[i..j].iter().collect()));
            i = j;
            continue;
        }
        let rest: String = c[i..c.len().min(i + 2)].iter().collect();
        match OPS.iter().find(|op| rest.starts_with(**op)) {
            Some(op) => {
                out.push(Tok::Op(op));
                i += op.len();
            }
            None => return Err(Error::at(line, format!("Unexpected character '{ch}'."))),
        }
    }
    Ok(out)
}

fn ends_open(toks: &[Tok]) -> bool {
    match toks.last() {
        Some(Tok::Op(op)) => matches!(
            *op,
            "," | "(" | "[" | "+" | "-" | "*" | "/" | "%" | "?" | ":" | "=" | ":=" | "==" | "!=" | "<" | ">" | "<=" | ">="
        ),
        Some(Tok::Ident(w)) => w == "and" || w == "or" || w == "not",
        _ => false,
    }
}

fn depth(toks: &[Tok]) -> i32 {
    toks.iter()
        .map(|t| match t {
            Tok::Op("(" | "[") => 1,
            Tok::Op(")" | "]") => -1,
            _ => 0,
        })
        .sum()
}

/// Splits source into logical lines.
pub fn lines(src: &str) -> Result<Vec<Line>, Error> {
    let mut out: Vec<Line> = Vec::new();
    let mut open = false;
    for (n, raw) in src.lines().enumerate() {
        let line_no = n + 1;
        let expanded = raw.replace('\t', "    ");
        let spaces = expanded.len() - expanded.trim_start().len();
        let toks = lex_line(&expanded, line_no)?;
        if toks.is_empty() {
            continue;
        }
        // A continuation: an unfinished previous line, or Pine's "not a
        // multiple of four" indent.
        let cont = open || (spaces % 4 != 0 && !out.is_empty());
        if cont && let Some(prev) = out.last_mut() {
            prev.toks.extend(toks);
            open = depth(&prev.toks) > 0 || ends_open(&prev.toks);
            continue;
        }
        open = depth(&toks) > 0 || ends_open(&toks);
        out.push(Line {
            indent: spaces / 4,
            toks,
            line: line_no,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_and_wrapping() {
        let src = "//@version=6\nindicator(\"X\", overlay=true)\nma = ta.sma(close,\n     20) // c\nc = #FF000080\nif ma > 1\n    plot(ma)\nx = 1 +\n  2";
        let ls = lines(src).unwrap();
        assert_eq!(ls.len(), 6);
        assert_eq!(ls[1].toks.len(), 10, "joined wrapped call");
        assert_eq!(ls[2].toks[2], Tok::Color(0xFF000080));
        assert_eq!(ls[4].indent, 1);
        assert_eq!(ls[5].toks, vec![Tok::Ident("x".into()), Tok::Op("="), Tok::Int(1), Tok::Op("+"), Tok::Int(2)]);
        assert!(lines("s = \"open").is_err());
    }
}

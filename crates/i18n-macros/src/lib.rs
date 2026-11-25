//! `trf!`: `format!` with a translated template.
//!
//! `trf!("Saved to {}", path.display())` formats as usual in English. In any
//! other language it looks up `"Saved to {0}"` and fills in the arguments,
//! which a translation may put in any order. Format specs stay out of the
//! key: `{x:.2}` is `{x}` there.

use proc_macro::{Delimiter, Group, Literal, TokenStream, TokenTree};

#[proc_macro]
pub fn trf(input: TokenStream) -> TokenStream {
    let mut it = input.clone().into_iter();
    let Some(TokenTree::Literal(lit)) = it.next() else {
        return error("trf! needs a string literal first");
    };
    let Some(text) = unescape(&lit.to_string()) else {
        return error("trf! needs a plain string literal");
    };
    let (key, holes) = match parse(&text) {
        Ok(x) => x,
        Err(e) => return error(e),
    };
    // What follows the literal, split at top-level commas.
    let mut args: Vec<Vec<TokenTree>> = Vec::new();
    for t in it {
        match &t {
            TokenTree::Punct(p) if p.as_char() == ',' => args.push(Vec::new()),
            _ => match args.last_mut() {
                Some(a) => a.push(t),
                None => return error("expected a comma after the template"),
            },
        }
    }
    args.retain(|a| !a.is_empty());
    let mut positional = Vec::new();
    let mut named = Vec::new();
    for a in args {
        match &a[..] {
            [TokenTree::Ident(n), TokenTree::Punct(p), rest @ ..]
                if p.as_char() == '=' && p.spacing() == proc_macro::Spacing::Alone && !rest.is_empty() =>
            {
                named.push((n.to_string(), rest.to_vec()))
            }
            _ => positional.push(a),
        }
    }
    let mut pairs = TokenStream::new();
    for (name, spec) in holes {
        let value: Option<Vec<TokenTree>> = match name.parse::<usize>() {
            Ok(n) => match positional.get(n) {
                Some(e) => Some(e.clone()),
                None => return error("trf!: not enough arguments"),
            },
            Err(_) => named.iter().find(|(n, _)| *n == name).map(|(_, e)| e.clone()),
        };
        // Captured names (`{path}`) are plain identifiers in scope.
        let value = value.unwrap_or_else(|| ts(&name).into_iter().collect());
        let mut call: TokenStream = TokenTree::from(Literal::string(&format!("{{{spec}}}"))).into();
        call.extend(ts(","));
        call.extend([TokenTree::from(Group::new(
            Delimiter::None,
            value.into_iter().collect(),
        ))]);
        let mut pair: TokenStream = TokenTree::from(Literal::string(&name)).into();
        pair.extend(ts(", ::std::format!"));
        pair.extend([TokenTree::from(Group::new(Delimiter::Parenthesis, call))]);
        pairs.extend([TokenTree::from(Group::new(Delimiter::Parenthesis, pair))]);
        pairs.extend(ts(","));
    }
    let mut out = ts("if crate::i18n::english()");
    let mut english = ts("::std::format!");
    english.extend([TokenTree::from(Group::new(Delimiter::Parenthesis, input))]);
    out.extend([TokenTree::from(Group::new(Delimiter::Brace, english))]);
    out.extend(ts("else"));
    let mut call: TokenStream = TokenTree::from(Literal::string(&key)).into();
    call.extend(ts(", &"));
    call.extend([TokenTree::from(Group::new(Delimiter::Bracket, pairs))]);
    let mut other = ts("crate::i18n::fill");
    other.extend([TokenTree::from(Group::new(Delimiter::Parenthesis, call))]);
    out.extend([TokenTree::from(Group::new(Delimiter::Brace, other))]);
    TokenTree::from(Group::new(Delimiter::Brace, out)).into()
}

fn ts(s: &str) -> TokenStream {
    s.parse().expect("valid tokens")
}

fn error(m: &str) -> TokenStream {
    ts(&format!("::std::compile_error!({m:?})"))
}

/// A hole: its name and ":spec".
type Hole = (String, String);

/// The key and each hole's (name, ":spec"). Doubled braces stay doubled in
/// the key, as `fill` expects.
fn parse(text: &str) -> Result<(String, Vec<Hole>), &'static str> {
    let mut key = String::new();
    let mut holes: Vec<Hole> = Vec::new();
    let mut next = 0;
    let mut cs = text.chars().peekable();
    while let Some(c) = cs.next() {
        key.push(c);
        match c {
            '{' | '}' if cs.peek() == Some(&c) => {
                key.push(c);
                cs.next();
            }
            '{' => {
                let mut hole = String::new();
                loop {
                    match cs.next() {
                        Some('}') => break,
                        Some(c) => hole.push(c),
                        None => return Err("trf!: unclosed {"),
                    }
                }
                let (name, spec) = match hole.split_once(':') {
                    Some((n, s)) => (n.trim().to_string(), format!(":{s}")),
                    None => (hole.trim().to_string(), String::new()),
                };
                if spec.contains('$') {
                    return Err("trf! can't take a width or precision from an argument");
                }
                let name = if name.is_empty() {
                    next += 1;
                    (next - 1).to_string()
                } else {
                    name
                };
                key.push_str(&name);
                key.push('}');
                if !holes.iter().any(|(n, _)| *n == name) {
                    holes.push((name, spec));
                }
            }
            _ => {}
        }
    }
    Ok((key, holes))
}

/// A string literal's value (`"…"` or `r#"…"#`).
fn unescape(lit: &str) -> Option<String> {
    if let Some(r) = lit.strip_prefix('r') {
        let r = r.trim_matches('#');
        return Some(r.strip_prefix('"')?.strip_suffix('"')?.to_string());
    }
    let body = lit.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut cs = body.chars().peekable();
    while let Some(c) = cs.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match cs.next()? {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            'u' => {
                let hex: String = cs.by_ref().skip(1).take_while(|c| *c != '}').collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
            }
            '\n' => {
                while cs.peek().is_some_and(|c| c.is_whitespace()) {
                    cs.next();
                }
            }
            c => out.push(c),
        }
    }
    Some(out)
}

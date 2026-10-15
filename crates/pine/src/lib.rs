//! Opentern's Pine Script®-compatible engine.
//!
//! Source goes through [`lexer`] (logical lines), [`parser`] (syntax tree),
//! then [`compile`] resolves every name to a slot, folds constants, reads
//! the declarations (`indicator`, `input.*`, `plot`, …) into a study
//! [`Meta`], and gives each `ta.*` call and each user-function call site
//! its own state, as Pine does. [`run`] executes the result bar by bar on
//! the shared series runtime; its state is a plain `Clone` value, so the
//! forming bar is computed on a copy and thrown away (Pine's rollback).
//!
//! Pine Script® and TradingView® are trademarks of TradingView, Inc. This is
//! an independent implementation written from the public reference manual.

mod ast;
mod compile;
// Arrays, maps, matrices and drawings land with Phase 10.
#[allow(dead_code)]
mod heap;
mod lexer;
mod libs;
mod natives;
mod parser;
mod run;
mod value;

use opentern_series::study::{Factory, Inputs, Meta};
use std::sync::Arc;

pub use compile::Kind;
pub use opentern_series::drawings::{Drawings, Label, Line, Linefill, PineBox, Polyline, Table, TableCell, XLoc};
pub use run::{Alert, PineStudy};
pub use value::V;

/// A compile or runtime error, with the 1-based source line.
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub line: usize,
    pub message: String,
}

impl Error {
    pub fn at(line: usize, message: impl Into<String>) -> Error {
        Error {
            line,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line > 0 {
            write!(f, "Line {}: {}", self.line, self.message)
        } else {
            f.write_str(&self.message)
        }
    }
}

/// What a script knows about the chart it runs on (`syminfo.*`).
#[derive(Clone, Debug)]
pub struct Env {
    /// `EXCHANGE:TICKER`.
    pub tickerid: String,
    pub description: String,
    pub mintick: f64,
    pub currency: String,
    /// "crypto", "stock", "fund", "index", "forex", "futures".
    pub kind: String,
    /// The exchange's timezone (IANA name).
    pub timezone: String,
    /// Libraries `import` can use: (title, source), from the user's
    /// scripts folder.
    pub libraries: Vec<(String, String)>,
    /// Bars of other symbols and timeframes for `request.security`, filled
    /// by the app on demand (see [`PineStudy::wanted`]).
    pub feeds: Feeds,
}

/// Other series by [`feed_key`]: shared between the app (which fills it)
/// and every script compiled with it.
pub type Feeds = Arc<std::sync::RwLock<std::collections::HashMap<String, Arc<Vec<opentern_core::Bar>>>>>;

/// The key of a feed: the symbol as the script wrote it, and the timeframe.
pub fn feed_key(symbol: &str, tf: opentern_core::Timeframe) -> String {
    format!("{symbol}|{}", tf.label())
}

/// A timeframe as Pine writes it ("60", "D", "1W", "15S").
pub fn parse_timeframe(s: &str) -> Option<opentern_core::Timeframe> {
    natives::parse_tf(s)
}

impl Default for Env {
    fn default() -> Self {
        Env {
            tickerid: "DEMO:DEMO".into(),
            description: String::new(),
            mintick: 0.01,
            currency: "USD".into(),
            kind: "crypto".into(),
            timezone: "UTC".into(),
            libraries: Vec::new(),
            feeds: Feeds::default(),
        }
    }
}

impl Env {
    pub fn ticker(&self) -> &str {
        self.tickerid.rsplit(':').next().unwrap_or(&self.tickerid)
    }

    pub fn prefix(&self) -> &str {
        self.tickerid.split_once(':').map(|(a, _)| a).unwrap_or("")
    }
}

/// A compiled script, ready to run on any series.
#[derive(Clone)]
pub struct Script {
    prog: Arc<compile::Program>,
    meta: &'static Meta,
}

impl Script {
    /// The study declaration: inputs, plots, levels and fills.
    pub fn meta(&self) -> &'static Meta {
        self.meta
    }

    pub fn kind(&self) -> Kind {
        self.prog.kind
    }

    /// `strategy(...)` arguments, as JSON for `opentern_engine::Props`.
    pub fn strategy_props(&self) -> Option<&serde_json::Value> {
        (self.prog.kind == Kind::Strategy).then_some(&self.prog.props)
    }

    /// The same script with some `strategy(...)` properties replaced (the
    /// Strategy Tester's Properties): `overrides` is a JSON object with
    /// the same keys as the declaration (`initial_capital`,
    /// `default_qty_type`, `commission_value`, …).
    pub fn with_props(&self, overrides: &serde_json::Value) -> Script {
        let Some(over) = overrides.as_object().filter(|o| !o.is_empty()) else {
            return self.clone();
        };
        let mut prog = (*self.prog).clone();
        if let serde_json::Value::Object(m) = &mut prog.props {
            for (k, v) in over {
                m.insert(k.clone(), v.clone());
            }
        } else {
            prog.props = overrides.clone();
        }
        Script {
            prog: Arc::new(prog),
            meta: self.meta,
        }
    }

    /// Builds a fresh study for given inputs.
    pub fn factory(&self) -> Factory {
        let prog = self.prog.clone();
        Arc::new(move |inputs: &Inputs| Box::new(PineStudy::new(prog.clone(), inputs)) as _)
    }

    /// A study with the default inputs (tests, screeners).
    pub fn study(&self, inputs: &Inputs) -> PineStudy {
        PineStudy::new(self.prog.clone(), inputs)
    }

    /// The `alertcondition` titles, in order.
    pub fn alert_conditions(&self) -> Vec<String> {
        self.prog.alertconditions.iter().map(|a| a.title.clone()).collect()
    }
}

/// Compiles Pine source. `file` names the study (`pine:<file>`).
pub fn compile(src: &str, file: &str, env: Env) -> Result<Script, Error> {
    let lines = lexer::lines(src)?;
    let ast = parser::Parser::new(lines).program()?;
    let version = version_of(src);
    let prog = compile::compile(&ast, version, env, file)?;
    let meta = compile::meta(&prog, file);
    Ok(Script {
        prog: Arc::new(prog),
        meta,
    })
}

/// The title of a library script (`library("Title")`), if it is one.
pub fn library_title(src: &str) -> Option<String> {
    let lines = lexer::lines(src).ok()?;
    let ast = parser::Parser::new(lines).program().ok()?;
    ast.iter().find_map(|s| match s {
        ast::Stmt::Expr(ast::Expr::Call { callee, args }, _) if callee == "library" => {
            let a = args
                .iter()
                .find(|a| a.name.as_deref() == Some("title"))
                .or(args.first())?;
            match &a.value {
                ast::Expr::Str(s) => Some(s.to_string()),
                _ => None,
            }
        }
        _ => None,
    })
}

/// `//@version=N`, 5 when missing (and for anything older, which is
/// close enough for most v4 scripts that compile at all).
fn version_of(src: &str) -> u8 {
    for l in src.lines().take(30) {
        let t = l.trim().trim_start_matches('/').trim();
        if let Some(v) = t.strip_prefix("@version=") {
            return v.trim().parse().unwrap_or(5);
        }
    }
    5
}

#[cfg(test)]
mod tests;

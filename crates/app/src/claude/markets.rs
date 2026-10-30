//! The agent's tools for the Seasonality and Indices tabs: the active
//! chart's monthly returns, and custom indices (list, read the board,
//! make, edit, rebalance, compare, delete, show). Routed from `tools.rs`;
//! slow answers wait in `control::Wait`.

use super::control::{Batch, Wait, Waiting, norm};
use super::tools::{Then, error, pretty, text};
use crate::app::App;
use crate::indices::{IndexDef, Metric, Weighting};
use opentern_core::Symbol;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// What a batch of index member lookups is for.
pub struct IndexJob {
    /// The index being changed (its ticker), or a new one.
    pub editing: Option<String>,
    pub name: String,
    pub members: Vec<Symbol>,
    pub weighting: Weighting,
}

fn st<'a>(a: &'a Value, k: &str) -> &'a str {
    a.get(k).and_then(Value::as_str).unwrap_or("")
}

fn strings(a: &Value, k: &str) -> Vec<String> {
    match a.get(k) {
        Some(Value::Array(v)) => v.iter().filter_map(|x| x.as_str()).map(str::to_string).collect(),
        Some(Value::String(s)) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn round(v: f64) -> Value {
    if v.is_finite() {
        json!((v * 100.0).round() / 100.0)
    } else {
        Value::Null
    }
}

fn weighting(s: &str) -> Option<Weighting> {
    match norm(s).as_str() {
        "freefloat" | "freefloatmarketcap" | "float" => Some(Weighting::FreeFloat),
        "marketcap" | "fullmarketcap" | "cap" => Some(Weighting::MarketCap),
        "equal" | "equalweight" => Some(Weighting::Equal),
        _ => None,
    }
}

fn metrics_json(values: &std::collections::HashMap<Metric, f64>) -> Value {
    let mut m = serde_json::Map::new();
    for k in Metric::ALL {
        let v = values.get(&k).copied().unwrap_or(f64::NAN);
        m.insert(
            k.name().into(),
            if v.is_finite() { json!(k.format(v)) } else { Value::Null },
        );
    }
    Value::Object(m)
}

impl App {
    // ------------------------------------------------------ seasonality

    pub(crate) fn tool_seasonality(&mut self, (chat, request, id): (u64, &str, &Value), a: &Value) -> Option<Value> {
        let year = |k: &str| a.get(k).and_then(Value::as_i64).map(|y| y as i16);
        let (from, to) = (year("from_year"), year("to_year"));
        match self.seasonal_report(from, to) {
            Some(r) => Some(seasonal_answer(r)),
            None => {
                self.claude.waiting.push(Waiting {
                    chat,
                    request: request.to_string(),
                    id: id.clone(),
                    wait: Wait::Seasonal { from, to },
                    since: Instant::now(),
                });
                None
            }
        }
    }

    // --------------------------------------------------- custom indices

    /// An index by its name, ticker or chart id.
    fn index_named(&self, name: &str) -> Option<IndexDef> {
        let n = norm(name.strip_prefix(opentern_data::expr::CUSTOM).unwrap_or(name));
        self.indices
            .defs
            .iter()
            .find(|d| norm(&d.name) == n || norm(&d.ticker) == n)
            .cloned()
    }

    fn no_index(&self, name: &str) -> Value {
        let names: Vec<&str> = self.indices.defs.iter().map(|d| d.name.as_str()).collect();
        if names.is_empty() {
            error("There are no custom indices yet. Make one with action create.")
        } else {
            error(format!("No index called \"{name}\". Indices: {}.", names.join(", ")))
        }
    }

    /// One index's board: its own numbers, then every member's.
    pub(crate) fn index_detail(&mut self, ticker: &str) -> Value {
        let Some(def) = self.indices.by_ticker(ticker).cloned() else {
            return error("That index was deleted.");
        };
        let Some(board) = self.indices.board(ticker) else {
            return error("That index was deleted.");
        };
        let now = self.index_now(&def);
        let members: Vec<Value> = board
            .rows
            .iter()
            .map(|r| {
                let mut m = json!({
                    "symbol": r.symbol.id,
                    "name": r.symbol.description,
                    "weight_pct": round(r.weight),
                    "price": round(r.price),
                    "day_change_pct": round(r.change),
                    "market_cap": if r.market_cap.is_finite() { json!(r.market_cap.round()) } else { Value::Null },
                    "metrics": metrics_json(&r.values),
                });
                if let Some(e) = &r.error {
                    m["problem"] = json!(e);
                }
                m
            })
            .collect();
        let mut out = json!({
            "name": def.name,
            "symbol": def.id(),
            "method": crate::indices::method_note(&def),
            "value": round(now.map_or(board.value, |n| n.0)),
            "day_change_pct": round(now.map_or(board.change, |n| n.1)),
            "metrics": metrics_json(&board.index),
            "members": members,
        });
        if board.waiting > 0 || def.pending || !def.ready() {
            out["still_loading"] = json!(format!(
                "{} of {} members' data is still coming in; weights and numbers may change.",
                board.waiting,
                def.members.len()
            ));
        }
        pretty(&out)
    }

    /// Whether an index's board has everything it asked for.
    pub(crate) fn index_settled(&mut self, ticker: &str, rebalance: bool) -> bool {
        self.indices_want();
        let Some(def) = self.indices.by_ticker(ticker).cloned() else {
            return true;
        };
        if rebalance && self.rebalance_running(&def) {
            return false;
        }
        let waiting = self.indices.board(ticker).map_or(0, |b| b.waiting);
        waiting == 0 && !def.pending && (def.ready() || def.members.is_empty())
    }

    fn wait_index(&mut self, (chat, request, id): (u64, &str, &Value), ticker: &str, rebalance: bool) -> Option<Value> {
        if self.index_settled(ticker, rebalance) {
            return Some(self.index_detail(ticker));
        }
        self.claude.waiting.push(Waiting {
            chat,
            request: request.to_string(),
            id: id.clone(),
            wait: Wait::Index {
                ticker: ticker.to_string(),
                rebalance,
                note: String::new(),
            },
            since: Instant::now(),
        });
        None
    }

    pub(crate) fn tool_custom_index(&mut self, ctx: (u64, &str, &Value), a: &Value) -> Option<Value> {
        let action = norm(st(a, "action"));
        let name = st(a, "name");
        let def = self.index_named(name);
        Some(match action.as_str() {
            "list" => {
                let defs = self.indices.defs.clone();
                let list: Vec<Value> = defs
                    .iter()
                    .map(|d| {
                        let now = self.index_now(d);
                        json!({
                            "name": d.name,
                            "symbol": d.id(),
                            "weighting": d.weighting.name(),
                            "members": d.members.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
                            "value": round(now.map_or(f64::NAN, |n| n.0)),
                            "day_change_pct": round(now.map_or(f64::NAN, |n| n.1)),
                        })
                    })
                    .collect();
                if list.is_empty() {
                    text("There are no custom indices yet.")
                } else {
                    pretty(&json!(list))
                }
            }
            "get" => match def {
                Some(d) => return self.wait_index(ctx, &d.ticker, false),
                None => self.no_index(name),
            },
            "compare" => {
                self.indices_want();
                let defs = self.indices.defs.clone();
                let rows: Vec<Value> = defs
                    .iter()
                    .filter_map(|d| {
                        let b = self.indices.board(&d.ticker)?;
                        let now = self.index_now(d);
                        Some(json!({
                            "name": d.name,
                            "symbol": d.id(),
                            "members": d.members.len(),
                            "value": round(now.map_or(b.value, |n| n.0)),
                            "day_change_pct": round(now.map_or(b.change, |n| n.1)),
                            "total_market_cap": if b.total_cap > 0.0 { json!(b.total_cap.round()) } else { Value::Null },
                            "metrics": metrics_json(&b.index),
                            "still_loading": b.waiting > 0,
                        }))
                    })
                    .collect();
                if rows.is_empty() {
                    text("There are no custom indices yet.")
                } else {
                    pretty(&json!(rows))
                }
            }
            "create" => {
                if name.trim().is_empty() {
                    return Some(error("create needs a \"name\"."));
                }
                if def.is_some() {
                    return Some(error(format!(
                        "There's already an index called \"{name}\". Use action edit to change it."
                    )));
                }
                let symbols = strings(a, "symbols");
                if symbols.is_empty() {
                    return Some(error("create needs \"symbols\", e.g. [\"NSE:MARUTI\", \"NSE:M&M\"]."));
                }
                let w = match st(a, "weighting") {
                    "" => Weighting::default(),
                    s => match weighting(s) {
                        Some(w) => w,
                        None => return Some(error("weighting is free_float, market_cap or equal.")),
                    },
                };
                let job = IndexJob {
                    editing: None,
                    name: name.trim().to_string(),
                    members: Vec::new(),
                    weighting: w,
                };
                return self.index_lookups(ctx, symbols, job);
            }
            "edit" => {
                let Some(d) = def else {
                    return Some(self.no_index(name));
                };
                let w = match st(a, "weighting") {
                    "" => d.weighting,
                    s => match weighting(s) {
                        Some(w) => w,
                        None => return Some(error("weighting is free_float, market_cap or equal.")),
                    },
                };
                let replace = strings(a, "symbols");
                let add = strings(a, "add");
                let mut members = if replace.is_empty() {
                    d.members.clone()
                } else {
                    Vec::new()
                };
                let mut missing = Vec::new();
                for r in strings(a, "remove") {
                    let (before, n) = (members.len(), norm(&r));
                    members.retain(|m| norm(&m.id) != n && norm(m.ticker()) != n);
                    if members.len() == before {
                        missing.push(r.clone());
                    }
                }
                let new_name = st(a, "new_name").trim();
                let job = IndexJob {
                    editing: Some(d.ticker.clone()),
                    name: if new_name.is_empty() {
                        d.name.clone()
                    } else {
                        new_name.to_string()
                    },
                    members,
                    weighting: w,
                };
                let lookups: Vec<String> = replace.into_iter().chain(add).collect();
                if !lookups.is_empty() {
                    return self.index_lookups(ctx, lookups, job);
                }
                if job.members.is_empty() {
                    return Some(error("An index needs at least one member."));
                }
                self.upsert_index(job.editing.as_deref(), &job.name, job.members, job.weighting);
                let mut msg = format!("{} is saved; the weights update once the data is in.", job.name);
                if !missing.is_empty() {
                    msg.push_str(&format!(" Not a member: {}.", missing.join(", ")));
                }
                text(msg)
            }
            "rebalance" => match def {
                Some(d) => {
                    self.rebalance_index(&d.ticker);
                    return self.wait_index(ctx, &d.ticker, true);
                }
                None => self.no_index(name),
            },
            "delete" => match def {
                Some(d) => {
                    self.delete_index(&d.ticker);
                    text(format!("Deleted {}.", d.name))
                }
                None => self.no_index(name),
            },
            "open" | "show" => {
                if let Some(v) = st(a, "view").get(..1) {
                    self.indices.view = if v == "m" {
                        crate::indices::View::Map
                    } else {
                        crate::indices::View::Table
                    };
                }
                let m = norm(st(a, "metric"));
                if !m.is_empty() {
                    match Metric::ALL.into_iter().find(|k| norm(k.name()).starts_with(&m)) {
                        Some(k) => self.indices.metric = k,
                        None => {
                            return Some(error(format!(
                                "No metric \"{}\". Metrics: {}.",
                                st(a, "metric"),
                                Metric::ALL.map(|k| k.name()).join(", ")
                            )));
                        }
                    }
                }
                self.layout.tab = Some(crate::shell::Tab::Indices);
                self.last_tab = crate::shell::Tab::Indices;
                if name.is_empty() || norm(name) == "compare" {
                    if self.indices.defs.len() < 2 {
                        return Some(error("Compare needs two indices or more."));
                    }
                    self.indices.compare = true;
                    return Some(text("The Indices tab shows every index side by side."));
                }
                let Some(d) = def else {
                    return Some(self.no_index(name));
                };
                self.indices.compare = false;
                self.indices.open = Some(d.ticker.clone());
                if d.ready() {
                    self.open_symbol_synced(d.symbol());
                } else {
                    self.indices.chart_when_ready = Some(d.ticker.clone());
                }
                text(format!("{} is on the board and the chart ({}).", d.name, d.id()))
            }
            _ => error("action is list, get, compare, create, edit, rebalance, delete or open."),
        })
    }

    /// Looks up an index's new members, then saves it (`index_found`).
    fn index_lookups(
        &mut self,
        (chat, request, id): (u64, &str, &Value),
        symbols: Vec<String>,
        job: IndexJob,
    ) -> Option<Value> {
        self.claude.batch_seq += 1;
        let b = self.claude.batch_seq;
        self.claude.batches.insert(
            b,
            Batch {
                chat,
                request: request.to_string(),
                id: id.clone(),
                left: symbols.len(),
                added: Vec::new(),
                missed: Vec::new(),
                index: Some(job),
            },
        );
        for s in symbols {
            self.search_then((chat, request, id), &s, Then::Index { batch: b });
        }
        None
    }

    /// One index member lookup came back.
    pub(crate) fn index_found(&mut self, batch: u64, query: &str, found: Option<Symbol>) {
        let Some(b) = self.claude.batches.get_mut(&batch) else {
            return;
        };
        b.left = b.left.saturating_sub(1);
        match found {
            Some(s) => {
                b.added.push(s.id.clone());
                if let Some(job) = &mut b.index
                    && !job.members.iter().any(|m| m.id == s.id)
                {
                    job.members.push(s);
                }
            }
            None => b.missed.push(query.to_string()),
        }
        if b.left > 0 {
            return;
        }
        let Some(b) = self.claude.batches.remove(&batch) else {
            return;
        };
        let Some(job) = b.index else {
            return;
        };
        let missed = if b.missed.is_empty() {
            String::new()
        } else {
            format!(" No match for: {}.", b.missed.join(", "))
        };
        if job.members.is_empty() {
            let r = error(format!("No members found, so nothing was saved.{missed}"));
            self.tool_answer(b.chat, &b.request, &b.id, r);
            return;
        }
        let ticker = self.upsert_index(job.editing.as_deref(), &job.name, job.members, job.weighting);
        // Answer with the board once prices and share counts are in.
        self.claude.waiting.push(Waiting {
            chat: b.chat,
            request: b.request,
            id: b.id,
            wait: Wait::Index {
                ticker,
                rebalance: false,
                note: format!("Saved.{missed}"),
            },
            since: Instant::now(),
        });
    }
}

pub(crate) fn seasonal_answer(r: Result<Value, String>) -> Value {
    match r {
        Ok(v) => pretty(&v),
        Err(e) => error(e),
    }
}

/// How long an index's board may take before the agent gets what's there.
pub const INDEX_WAIT: Duration = Duration::from_secs(90);

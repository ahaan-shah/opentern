//! The app's own tools for Claude: an MCP server that lives in the app and
//! answers over the Claude Code pipe (`mcp__opentern__*`). Each call runs on
//! the UI thread against the active chart. Symbol searches answer later,
//! when the hub's results come back.

use super::process::SERVER;
use crate::app::App;
use crate::scripting::Lang;
use opentern_core::Timeframe;
use serde_json::{Value, json};

/// Hub search numbers for Claude's lookups (clear of the symbol dialog's).
pub const SEARCH_BASE: u64 = 1 << 43;

/// A tool call waiting for search results.
pub struct Pending {
    pub chat: u64,
    pub request: String,
    pub id: Value,
    pub seq: u64,
    pub query: String,
    /// Open the best match on the chart (else just list them).
    pub open: bool,
}

fn tool(name: &str, description: &str, props: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {"type": "object", "properties": props, "required": required},
    })
}

fn tools() -> Value {
    let s = |d: &str| json!({"type": "string", "description": d});
    json!([
        tool(
            "get_chart",
            "What the active chart shows: symbol, timeframe, chart type, the last bars' prices, every indicator and script on it with its latest values and any errors, and the script open in the editor.",
            json!({}),
            &[]
        ),
        tool(
            "get_bars",
            "The active chart's most recent bars as CSV (time in UTC, open, high, low, close, volume).",
            json!({"count": {"type": "integer", "description": "How many bars, newest last (default 100, at most 1000)."}}),
            &[]
        ),
        tool(
            "search_symbols",
            "Searches the symbols Opentern can chart (crypto exchanges, US and Indian stocks, forex, commodities). Returns ids like BINANCE:BTCUSDT.",
            json!({"query": s("A ticker or name, e.g. 'BTC', 'AAPL', 'RELIANCE'.")}),
            &["query"]
        ),
        tool(
            "open_symbol",
            "Shows a symbol on the active chart. Takes a full id (BINANCE:ETHUSDT, NASDAQ:AAPL) or a ticker to look up.",
            json!({"symbol": s("The symbol id or ticker.")}),
            &["symbol"]
        ),
        tool(
            "set_timeframe",
            "Changes the active chart's timeframe.",
            json!({"timeframe": s("e.g. 1m, 5m, 15m, 1h, 4h, 1D, 1W.")}),
            &["timeframe"]
        ),
        tool(
            "list_scripts",
            "The scripts in the user's scripts folder (the working directory), with their language and whether they're on the chart.",
            json!({}),
            &[]
        ),
        tool(
            "check_script",
            "Compiles a script (Pine, Luau, JavaScript or TypeScript) the way the chart would, and returns its errors with line numbers, or what it declares. Use it after every edit.",
            json!({"file": s("Path relative to the scripts folder, e.g. 'rsi.pine'.")}),
            &["file"]
        ),
        tool(
            "add_script_to_chart",
            "Puts a script from the scripts folder on the active chart. Scripts on the chart reload by themselves when their file is saved.",
            json!({"file": s("Path relative to the scripts folder.")}),
            &["file"]
        ),
        tool(
            "remove_from_chart",
            "Removes an indicator or script from the active chart.",
            json!({"name": s("Its title as get_chart shows it, or the script's file.")}),
            &["name"]
        ),
        tool(
            "list_indicators",
            "Opentern's built-in indicators (TradingView's built-ins, ported exactly).",
            json!({"query": s("Optional filter, e.g. 'rsi' or 'volume'.")}),
            &[]
        ),
        tool(
            "add_indicator",
            "Adds a built-in indicator to the active chart.",
            json!({"id": s("Its id or name from list_indicators.")}),
            &["id"]
        ),
        tool(
            "strategy_report",
            "The Strategy Report of a strategy on the active chart: TradingView's metrics (net profit, profit factor, drawdown, Sharpe, …) and the latest trades.",
            json!({"file": s("Optional: the strategy's script file, when there's more than one on the chart.")}),
            &[]
        ),
        tool(
            "open_in_editor",
            "Opens a script in Opentern's Script Editor so the user can see it.",
            json!({"file": s("Path relative to the scripts folder.")}),
            &["file"]
        ),
        tool(
            "scripting_guide",
            "How to write Opentern scripts in Luau, JavaScript, TypeScript and Python (Pine follows TradingView's Pine Script v5/v6). Read it before writing a non-Pine script.",
            json!({}),
            &[]
        ),
    ])
}

fn text(t: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": t.into()}]})
}

fn error(t: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": t.into()}], "isError": true})
}

fn pretty(v: &Value) -> Value {
    text(serde_json::to_string_pretty(v).unwrap_or_default())
}

impl App {
    /// One MCP message from Claude. Returns the JSON-RPC reply, or `None`
    /// when it will come later (a symbol search).
    pub(crate) fn claude_mcp(&mut self, chat: u64, request: &str, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let reply = |result: Value| Some(json!({"jsonrpc": "2.0", "id": id, "result": result}));
        match msg["method"].as_str().unwrap_or("") {
            "initialize" => reply(json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": SERVER, "version": env!("CARGO_PKG_VERSION")},
            })),
            "tools/list" => reply(json!({"tools": tools()})),
            "tools/call" => {
                let name = msg["params"]["name"].as_str().unwrap_or("").to_string();
                let args = msg["params"]["arguments"].clone();
                crate::diag::crumb(format!("claude tool {name}"));
                match self.claude_tool(chat, request, &id, &name, &args) {
                    Some(r) => reply(r),
                    None => None,
                }
            }
            // Notifications and anything else: an empty result.
            _ => reply(json!({})),
        }
    }

    fn claude_tool(&mut self, chat: u64, request: &str, id: &Value, name: &str, a: &Value) -> Option<Value> {
        let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
        Some(match name {
            "get_chart" => pretty(&self.chart_summary()),
            "get_bars" => {
                let n = a.get("count").and_then(Value::as_u64).unwrap_or(100).clamp(1, 1000) as usize;
                let bars = &self.chart.bars;
                let mut csv = String::from("time,open,high,low,close,volume\n");
                for b in &bars[bars.len().saturating_sub(n)..] {
                    let t = jiff::Timestamp::from_millisecond(b.time)
                        .map(|t| t.to_string())
                        .unwrap_or_default();
                    csv.push_str(&format!(
                        "{t},{},{},{},{},{}\n",
                        b.open, b.high, b.low, b.close, b.volume
                    ));
                }
                text(csv)
            }
            "search_symbols" | "open_symbol" => {
                let open = name == "open_symbol";
                let q = s(if open { "symbol" } else { "query" });
                if q.is_empty() {
                    return Some(error("Give a symbol or search text."));
                }
                self.claude.search_seq += 1;
                let seq = SEARCH_BASE + self.claude.search_seq;
                self.hub.send(opentern_data::Request::Search { seq, query: q.clone() });
                self.claude.pending.push(Pending {
                    chat,
                    request: request.to_string(),
                    id: id.clone(),
                    seq,
                    query: q,
                    open,
                });
                return None;
            }
            "set_timeframe" => match Timeframe::parse(&s("timeframe")) {
                Some(tf) => {
                    self.set_interval_synced(tf);
                    text(format!("The chart is on {} now.", tf.label()))
                }
                None => error(format!(
                    "\"{}\" isn't a timeframe. Try 1m, 15m, 1h, 4h, 1D or 1W.",
                    s("timeframe")
                )),
            },
            "list_scripts" => {
                let on: Vec<&str> = self.scripts.iter().map(|r| r.file.as_str()).collect();
                let list: Vec<Value> = self
                    .script_files()
                    .iter()
                    .map(|f| {
                        json!({
                            "file": f,
                            "language": Lang::of(f).map(|l| format!("{l:?}")).unwrap_or_default(),
                            "on_chart": on.contains(&f.as_str()),
                        })
                    })
                    .collect();
                pretty(&json!({"folder": self.scripts_dir().to_string_lossy(), "scripts": list}))
            }
            "check_script" => {
                let f = self.script_arg(a);
                self.check_script(&f)
            }
            "add_script_to_chart" => {
                let f = self.script_arg(a);
                if !self.script_files().contains(&f) {
                    return Some(error(format!("There's no {f} in the scripts folder.")));
                }
                self.add_script(&f);
                match self.scripts.last().and_then(|r| r.compile_error.clone()) {
                    Some(e) => error(format!("Added, but it doesn't compile: {e}")),
                    None => text(format!("{f} is on the chart.")),
                }
            }
            "remove_from_chart" => {
                let n = s("name").to_lowercase();
                let by_file = self
                    .scripts
                    .iter()
                    .find(|r| r.file.to_lowercase() == n)
                    .and_then(|r| r.uid);
                let uid = by_file.or_else(|| {
                    self.chart
                        .studies
                        .iter()
                        .find(|st| st.title(true).to_lowercase() == n || st.title(false).to_lowercase() == n)
                        .or_else(|| {
                            self.chart
                                .studies
                                .iter()
                                .find(|st| st.title(true).to_lowercase().contains(&n))
                        })
                        .map(|st| st.uid)
                });
                match uid {
                    Some(u) => {
                        let title = self
                            .chart
                            .studies
                            .iter()
                            .find(|st| st.uid == u)
                            .map(|st| st.title(true));
                        self.chart.remove_study(u);
                        self.sync_scripts();
                        text(format!("Removed {}.", title.unwrap_or_default()))
                    }
                    None => error(format!("Nothing called \"{}\" is on the chart.", s("name"))),
                }
            }
            "list_indicators" => {
                let q = s("query").to_lowercase();
                let list: Vec<Value> = opentern_series::builtins::all()
                    .iter()
                    .filter(|m| q.is_empty() || m.name.to_lowercase().contains(&q) || m.id.contains(&q))
                    .map(|m| json!({"id": m.id, "name": m.name}))
                    .collect();
                pretty(&json!(list))
            }
            "add_indicator" => {
                let q = s("id").to_lowercase();
                let all = opentern_series::builtins::all();
                let m = opentern_series::builtins::by_id(&q)
                    .or_else(|| all.iter().find(|m| m.name.to_lowercase() == q))
                    .or_else(|| all.iter().find(|m| m.name.to_lowercase().contains(&q)));
                match m {
                    Some(m) => {
                        self.add_study(m);
                        text(format!("Added {} to the chart.", m.name))
                    }
                    None => error(format!(
                        "There's no built-in called \"{}\". Try list_indicators.",
                        s("id")
                    )),
                }
            }
            "strategy_report" => match self.strategy_for(a) {
                Some(run) => {
                    let csv = crate::strategy::trades_csv(&run.broker, &self.chart.tz);
                    let lines: Vec<&str> = csv.lines().collect();
                    let mut trades = lines.first().map(|h| format!("{h}\n")).unwrap_or_default();
                    for l in &lines[lines.len().saturating_sub(20).max(1)..] {
                        trades.push_str(l);
                        trades.push('\n');
                    }
                    pretty(&json!({
                        "strategy": run.name,
                        "symbol": self.chart.symbol.id,
                        "timeframe": self.chart.tf.label(),
                        "bars": self.chart.bars.len(),
                        "units": "Money is in the account currency. Fields ending in _pct are already percentages (12.5 means 12.5%); drawdown and run-up _pct are relative to the initial capital, as in TradingView.",
                        "report": run.report,
                        "latest_trades_csv": trades,
                    }))
                }
                None => error("There's no strategy on the chart. Add a strategy script first."),
            },
            "open_in_editor" => {
                let f = self.script_arg(a);
                if !self.script_files().contains(&f) {
                    return Some(error(format!("There's no {f} in the scripts folder.")));
                }
                self.open_in_editor(&f);
                self.layout.tab = Some(crate::shell::Tab::Editor);
                self.last_tab = crate::shell::Tab::Editor;
                text(format!("{f} is open in the Script Editor."))
            }
            "scripting_guide" => text(GUIDE),
            other => error(format!("Opentern has no tool called {other}.")),
        })
    }

    /// A script argument, however Claude names it (`file`, `path`,
    /// `file_path`), relative to the scripts folder.
    fn script_arg(&self, a: &Value) -> String {
        let raw = ["file", "path", "file_path", "name"]
            .iter()
            .find_map(|k| a.get(*k).and_then(Value::as_str))
            .unwrap_or("")
            .trim();
        let dir = self.scripts_dir();
        std::path::Path::new(raw)
            .strip_prefix(&dir)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| raw.to_string())
    }

    /// The strategy run a call names by its file, else the current one.
    fn strategy_for(&self, a: &Value) -> Option<&crate::strategy::StrategyRun> {
        let f = self.script_arg(a);
        let uid = self
            .scripts
            .iter()
            .find(|r| !f.is_empty() && r.file == f)
            .and_then(|r| r.uid);
        match uid {
            Some(u) => self.strategies.runs.iter().find(|r| r.uid == u),
            None => self.strategies.current(),
        }
    }

    fn check_script(&mut self, file: &str) -> Value {
        if file.is_empty() {
            return error("Say which script: check_script needs \"file\", a path relative to the scripts folder.");
        }
        let path = self.scripts_dir().join(file);
        let Ok(src) = std::fs::read_to_string(&path) else {
            return error(format!("Couldn't read {file} from the scripts folder."));
        };
        let Some(lang) = Lang::of(file) else {
            return error("That isn't a script Opentern runs (.pine, .luau, .js, .ts or .py).");
        };
        let meta = match lang {
            Lang::Python => {
                return text(
                    "Python scripts are checked when they run: add it to the chart and call get_chart for errors.",
                );
            }
            Lang::Pine => opentern_pine::compile(&src, file, self.pine_env())
                .map(|s| {
                    let kind = format!("{:?}", s.kind()).to_lowercase();
                    (kind, s.meta())
                })
                .map_err(|e| e.to_string()),
            _ => opentern_script::inproc::compile(
                crate::pine_host::inproc_lang(lang),
                &src,
                file,
                crate::pine_host::syminfo_of(&self.chart.symbol),
            )
            .map(|s| ("script".to_string(), s.meta()))
            .map_err(|e| e.to_string()),
        };
        match meta {
            Ok((kind, m)) => pretty(&json!({
                "ok": true,
                "kind": kind,
                "title": m.name,
                "inputs": m.inputs.iter().map(|i| i.title.to_string()).collect::<Vec<_>>(),
                "plots": m.plots.iter().map(|p| p.title.to_string()).collect::<Vec<_>>(),
            })),
            Err(e) => error(format!("{file} doesn't compile. {e}")),
        }
    }

    /// The active chart, for `get_chart` and each message's context.
    pub(crate) fn chart_summary(&self) -> Value {
        let c = &self.chart;
        let prec = c.precision();
        let last: Vec<Value> = c.bars[c.bars.len().saturating_sub(3)..]
            .iter()
            .map(|b| {
                json!({
                    "time": jiff::Timestamp::from_millisecond(b.time).map(|t| t.to_string()).unwrap_or_default(),
                    "open": b.open, "high": b.high, "low": b.low, "close": b.close, "volume": b.volume,
                })
            })
            .collect();
        let studies: Vec<Value> = c
            .studies
            .iter()
            .map(|st| {
                let n = c.bars.len();
                let values: serde_json::Map<String, Value> = st
                    .meta()
                    .plots
                    .iter()
                    .enumerate()
                    .filter_map(|(i, p)| {
                        let v = st.inst.out.plots.get(i)?.get(n.checked_sub(1)?)?;
                        Some((
                            p.title.to_string(),
                            if v.is_finite() {
                                json!(crate::format::price(*v, prec.max(2)))
                            } else {
                                Value::Null
                            },
                        ))
                    })
                    .collect();
                let run = self.scripts.iter().find(|r| r.uid == Some(st.uid));
                json!({
                    "title": st.title(true),
                    "script_file": run.map(|r| r.file.clone()),
                    "kind": run.map(|r| r.kind.clone()).filter(|k| !k.is_empty()).unwrap_or_else(|| "indicator".into()),
                    "visible": st.visible,
                    "latest_values": values,
                    "error": run.and_then(|r| r.error.clone()),
                })
            })
            .collect();
        // Scripts that failed before they ever got on the chart.
        let failing: Vec<Value> = self
            .scripts
            .iter()
            .filter(|r| r.uid.is_none())
            .filter_map(|r| r.error.as_ref().map(|e| json!({"script_file": r.file, "error": e})))
            .collect();
        json!({
            "symbol": c.symbol.id,
            "description": c.symbol.description,
            "kind": format!("{:?}", c.symbol.kind).to_lowercase(),
            "currency": c.symbol.currency,
            "timeframe": c.tf.label(),
            "chart_type": c.style.kind.name(),
            "bars_loaded": c.bars.len(),
            "last_bars": last,
            "studies": studies,
            "scripts_not_on_chart_because_of_errors": failing,
            "editor_file": self.editor.doc().filter(|d| !d.readonly).map(|d| d.key.clone()),
            "charts_in_layout": self.chart_count(),
        })
    }

    /// Search results for one of Claude's lookups.
    pub(crate) fn claude_search_done(&mut self, seq: u64, results: Vec<opentern_core::Symbol>) {
        let Some(i) = self.claude.pending.iter().position(|p| p.seq == seq) else {
            return;
        };
        let p = self.claude.pending.remove(i);
        let result = if p.open {
            let q = p.query.to_uppercase();
            let best = results
                .iter()
                .find(|s| s.id.to_uppercase() == q)
                .or_else(|| results.iter().find(|s| s.ticker().to_uppercase() == q))
                .or(results.first())
                .cloned();
            match best {
                Some(sym) => {
                    let msg = format!("The chart shows {} ({}) now.", sym.id, sym.description);
                    self.open_symbol_synced(sym);
                    text(msg)
                }
                None => error(format!("Nothing matches \"{}\".", p.query)),
            }
        } else {
            let list: Vec<Value> = results
                .iter()
                .take(25)
                .map(|s| json!({"id": s.id, "description": s.description, "kind": format!("{:?}", s.kind).to_lowercase()}))
                .collect();
            if list.is_empty() {
                error(format!("Nothing matches \"{}\".", p.query))
            } else {
                pretty(&json!(list))
            }
        };
        if let Some(c) = self.claude.clients.get(&p.chat) {
            c.mcp_reply(&p.request, json!({"jsonrpc": "2.0", "id": p.id, "result": result}));
        }
    }
}

/// `scripting_guide`: the scripting API and the Pine notes.
const GUIDE: &str = concat!(
    "# Pine Script\n\nOpentern runs Pine Script v5 and v6 natively (indicator, strategy and library scripts, ",
    "request.security, drawings, arrays/maps/matrices, UDTs, methods, enums). `import TradingView/ta/9` works: ",
    "it's bundled. Other libraries must be scripts in the scripts folder (found by their library(\"Name\") title).\n\n",
    include_str!("../../../../docs/scripting.md")
);

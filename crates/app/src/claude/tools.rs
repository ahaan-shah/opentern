//! The app's own tools for the agents: an MCP server that lives in the app
//! and answers over the Claude Code pipe (`mcp__opentern__*`), and the same
//! tools as Codex's dynamic tools (`codex.rs`). Each call runs on
//! the UI thread against the active chart. Symbol searches answer later,
//! when the hub's results come back.

use super::process::SERVER;
use crate::app::App;
use crate::scripting::Lang;
use opentern_core::Timeframe;
use serde_json::{Value, json};

/// Hub search numbers for the agent's lookups (clear of the symbol
/// dialog's, and at or past `SEARCH_ALL` so a batch isn't coalesced).
pub const SEARCH_BASE: u64 = 1 << 43;
const _: () = assert!(SEARCH_BASE >= opentern_data::hub::SEARCH_ALL);

/// What to do with a symbol search's best match.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Then {
    /// Answer with the matches.
    List,
    /// Show it on the active chart.
    Open,
    Compare(crate::chart::CompareMode),
    /// Add it to watchlist `list` (one of a batch of lookups).
    Watch {
        batch: u64,
        list: i64,
    },
    /// A member for a custom index (one of a batch of lookups).
    Index {
        batch: u64,
    },
}

/// A tool call waiting for search results.
pub struct Pending {
    pub chat: u64,
    pub request: String,
    pub id: Value,
    pub seq: u64,
    pub query: String,
    pub then: Then,
}

fn tool(name: &str, description: &str, props: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {"type": "object", "properties": props, "required": required},
    })
}

/// The app's tools: name, description and input schema.
pub fn specs() -> Value {
    let s = |d: &str| json!({"type": "string", "description": d});
    let t = |ty: &str| json!({"type": ty});
    json!([
        tool(
            "get_chart",
            "What the active chart shows: symbol, timeframe, chart type, the last bars' prices, every indicator and script on it with its latest values and any errors, and the script open in the editor.",
            json!({}),
            &[]
        ),
        tool(
            "get_bars",
            "The active chart's bars as CSV (time in UTC, open, high, low, close, volume): the latest ones, or a date range (older history loads as needed).",
            json!({
                "count": {"type": "integer", "description": "How many bars (default 100, at most 1000)."},
                "from": s("Optional start, e.g. 2024-03-01 or 2024-03-01T14:00Z (UTC)."),
                "to": s("Optional end."),
            }),
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
            "chart_view",
            "Changes how the active chart looks. Actions: chart_type {type: candles, hollow candles, bars, heikin ashi, line, area, baseline, renko, …}; go_to {from, to?} scrolls and zooms to a date range, loading older history; reset (back to the latest bar); scale {log?, percent?, auto?}; layout {charts: 1-8} splits the window into that many charts; activate {chart: 1-based number} picks which one the tools act on; compare {symbol, mode: percent|scale|pane}; remove_compare {symbol}; indicator_template {name}.",
            json!({
                "action": s("chart_type, go_to, reset, scale, layout, activate, compare, remove_compare or indicator_template."),
                "type": t("string"), "from": t("string"), "to": t("string"),
                "log": t("boolean"), "percent": t("boolean"), "auto": t("boolean"),
                "charts": t("integer"), "chart": t("integer"),
                "symbol": t("string"), "mode": t("string"), "name": t("string"),
            }),
            &["action"]
        ),
        tool(
            "indicator",
            "One indicator or script on the active chart, by its title from get_chart. Actions: get (inputs with ids, values and kinds; plots), set {inputs: {id or title: value}}, show, hide.",
            json!({
                "action": s("get, set, show or hide."),
                "name": s("Its title or script file."),
                "inputs": {"type": "object", "description": "For set: input id or title → new value."},
            }),
            &["action", "name"]
        ),
        tool(
            "drawings",
            "Draws on the active chart, as the user would with the drawing tools (saved per symbol; the user can undo). Actions: add {drawings: [{tool, points: [{time | bars_ago, price}], text?, color?, width?, line_style?, fill?, extend_left?, extend_right?, font_size?}]}; list; remove {ids}; clear (every drawing on the symbol, the user's too). Tools: trend_line, ray, extended_line, arrow, horizontal_line, horizontal_ray, vertical_line, cross_line, parallel_channel, fib_retracement, fib_extension, pitchfork, path, triangle, long_position, short_position, price_range, date_range, date_price_range, rectangle, ellipse, brush, text, arrow_up, arrow_down, anchored_vwap, fixed_range_volume_profile, anchored_volume_profile. Points: 1 for horizontal/vertical lines, text, arrows, anchored tools; 3 for parallel_channel, fib_extension, pitchfork, triangle and long/short_position (entry, target, stop; the last two at the box's end time); 2+ for path and brush; else 2. price may be a number or open/high/low/close of that bar; without time or bars_ago a point is on the latest bar. text labels any of them. Colours: #RRGGBB or red, green, orange, purple, yellow, gray.",
            json!({
                "action": s("add, list, remove or clear."),
                "drawings": {"type": "array", "items": {"type": "object"}, "description": "For add."},
                "ids": {"type": "array", "items": {"type": "integer"}, "description": "For remove."},
            }),
            &["action"]
        ),
        tool(
            "replay",
            "Bar replay on the active chart, as TradingView's: the chart cuts back to a bar and steps forward; indicators and strategies only see replayed bars. Actions: start {time} (the last bar shown), step {bars?}, play {speed?: 10s, 3s, 1s, 0.3s, 0.1s or 0.03s per bar}, pause, stop (back to live), status. Use snapshot between steps to see the chart.",
            json!({
                "action": s("start, step, play, pause, stop or status."),
                "time": s("For start: a date or UTC time."),
                "bars": {"type": "integer", "description": "For step (default 1)."},
                "speed": s("For play."),
            }),
            &["action"]
        ),
        tool(
            "snapshot",
            "A picture of the charts as the user sees them now (drawings, indicators, replay). Use it to check your drawings or to look at a pattern.",
            json!({}),
            &[]
        ),
        tool(
            "watchlist",
            "The user's watchlists. Actions: list (every list, and the open one's symbols with prices), show {name}, create {name, symbols?}, switch {name}, rename {name, new_name}, delete {name}, add {symbols, name?}, remove {symbols, name?}, add_section {name, list?}. Without name, the open list.",
            json!({
                "action": s("list, show, create, switch, rename, delete, add, remove or add_section."),
                "name": s("The list (or the new section's name for add_section)."),
                "symbols": {"type": "array", "items": {"type": "string"}, "description": "Tickers or ids, e.g. BTCUSDT, NASDAQ:AAPL."},
                "new_name": t("string"), "list": t("string"),
            }),
            &["action"]
        ),
        tool(
            "screener",
            "The Screener tab. Actions: get (setup, presets, fields); set or run, either taking {universe?: a watchlist or preset name, timeframe?, filters?: [{field, op: above|below, value}], script?: {builtin, plot?} or {file, plot?}}; results. run waits for the results (filtered rows, best change first). Fields: last, change, perf_week, perf_month, volume, rel_volume, rsi, sma50, sma200, atr, script.",
            json!({
                "action": s("get, set, run or results."),
                "universe": t("string"), "timeframe": t("string"),
                "filters": {"type": "array", "items": {"type": "object"}},
                "script": t("object"), "top": t("integer"),
            }),
            &["action"]
        ),
        tool(
            "strategy_settings",
            "The Strategy settings of a strategy on the chart (what strategy(...) declares, overridden as the user would in the dialog). Without set, shows them. set {initial_capital, default_qty_type: fixed|cash|percent_of_equity, default_qty_value, pyramiding, commission_type: percent|cash_per_contract|cash_per_order, commission_value, slippage, process_orders_on_close, margin_long, margin_short}; reset: true goes back to the script's.",
            json!({
                "file": s("Optional: the strategy's script file."),
                "set": {"type": "object"},
                "reset": {"type": "boolean"},
            }),
            &[]
        ),
        tool(
            "strategy_lab",
            "The Strategy Lab on the chart's strategy. Actions: get (setup and its inputs), set or run, either taking {mode: optimize|walk_forward|monte_carlo|portfolio, params?: [{input, on?, from?, to?, step?}], method?: grid|random|bayesian, trials?, objective?: net_profit|profit_factor|sharpe|sortino|percent_profitable|max_drawdown|return_over_drawdown|cagr, min_trades?, in_sample?, out_sample?, anchored?, mc_runs?, mc_method?: shuffle|bootstrap, ruin_pct?, universe?, bars?}; results; apply {rank?} puts an optimizer result's inputs on the chart; stop. run waits for the results.",
            json!({
                "action": s("get, set, run, results, apply or stop."),
                "mode": t("string"), "method": t("string"), "objective": t("string"),
                "params": {"type": "array", "items": {"type": "object"}},
                "trials": t("integer"), "min_trades": t("integer"),
                "in_sample": t("integer"), "out_sample": t("integer"), "anchored": t("boolean"),
                "mc_runs": t("integer"), "mc_method": t("string"), "ruin_pct": t("number"),
                "universe": t("string"), "bars": t("integer"), "rank": t("integer"),
            }),
            &["action"]
        ),
        tool(
            "paper_trading",
            "The paper trading account (simulated; never real money). Actions: account (balance, positions, orders, fills), order {symbol?, side: buy|sell, qty, type?: market|limit|stop|stop_limit|trailing, price?, limit?, offset?, take_profit?, stop_loss?}, cancel {order_id}, modify {order_id, price}, close {symbol?}. symbol defaults to the chart's.",
            json!({
                "action": s("account, order, cancel, modify or close."),
                "symbol": t("string"), "side": t("string"), "qty": t("number"), "type": t("string"),
                "price": t("number"), "limit": t("number"), "offset": t("number"),
                "take_profit": t("number"), "stop_loss": t("number"), "order_id": t("integer"),
            }),
            &["action"]
        ),
        tool(
            "alerts",
            "The user's alerts. Actions: list; create {value, op?: crossing|crossing_up|crossing_down|greater_than|less_than, symbol?, indicator?, plot?, freq?: once|once_per_bar|once_per_bar_close|every_time, message?, name?} (a price alert, or with indicator: a plot of a study on the chart against value, or against the price without value); delete {id}; pause {id}; resume {id}.",
            json!({
                "action": s("list, create, delete, pause or resume."),
                "value": t("number"), "op": t("string"), "symbol": t("string"),
                "indicator": t("string"), "plot": t("string"), "freq": t("string"),
                "message": t("string"), "name": t("string"), "id": t("integer"),
            }),
            &["action"]
        ),
        tool(
            "open_panel",
            "Opens a bottom-panel tab so the user sees it: screener, seasonality, indices, editor, report (Strategy Report), lab (Strategy Lab), paper, alerts (log); none closes the panel.",
            json!({"tab": s("The tab.")}),
            &["tab"]
        ),
        tool(
            "seasonality",
            "The Seasonality tab: the active chart symbol's monthly returns (% each month vs the month before, whole_year vs last year's close), a row per year, and per column how many rises and falls and the average. Open another symbol first to see its. from_year/to_year pick the years (and move the tab's slider); without them, the years the tab shows.",
            json!({"from_year": t("integer"), "to_year": t("integer")}),
            &[]
        ),
        tool(
            "custom_index",
            "The user's custom indices (Indices tab): baskets weighted like NSE's indices (free-float market cap by default, base 1000, rebalanced 1 April and 1 October keeping the level), charted as CUSTOM:<TICKER>. Actions: list; get {name} (value, day change, the index's metrics and every member's weight, price and metrics: returns, P/E, EV/EBITDA, growth, margins, RSI, vs 50/200 SMA); compare (every index's numbers side by side); create {name, symbols, weighting?: free_float|market_cap|equal}; edit {name, new_name?, symbols? (replaces the members), add?, remove?, weighting?} (a member change rebalances); rebalance {name} (fresh share counts, new weights, same level); delete {name}; open {name or \"compare\", view?: table|map, metric?} shows it on the Indices tab and the chart. create, edit and rebalance answer with the board once the members' data is in.",
            json!({
                "action": s("list, get, compare, create, edit, rebalance, delete or open."),
                "name": s("The index's name or ticker."),
                "symbols": {"type": "array", "items": {"type": "string"}, "description": "Tickers or ids, e.g. NSE:MARUTI, NASDAQ:AAPL."},
                "add": {"type": "array", "items": {"type": "string"}},
                "remove": {"type": "array", "items": {"type": "string"}},
                "new_name": t("string"), "weighting": t("string"),
                "view": t("string"), "metric": t("string"),
            }),
            &["action"]
        ),
        tool(
            "scripting_guide",
            "How to write Opentern scripts in Luau, JavaScript, TypeScript and Python (Pine follows TradingView's Pine Script v5/v6). Read it before writing a non-Pine script.",
            json!({}),
            &[]
        ),
    ])
}

pub(super) fn text(t: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": t.into()}]})
}

pub(super) fn error(t: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": t.into()}], "isError": true})
}

pub(super) fn pretty(v: &Value) -> Value {
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
            "tools/list" => reply(json!({"tools": specs()})),
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

    pub(crate) fn claude_tool(&mut self, chat: u64, request: &str, id: &Value, name: &str, a: &Value) -> Option<Value> {
        let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
        Some(match name {
            "get_chart" => pretty(&self.chart_summary()),
            "get_bars" => {
                let n = a.get("count").and_then(Value::as_u64).unwrap_or(100).clamp(1, 1000) as usize;
                let from = a.get("from").and_then(super::control::parse_time);
                let to = a.get("to").and_then(super::control::parse_time);
                if self
                    .wait_for_history((chat, request, id), from.unwrap_or(i64::MAX), name, a)
                    .is_some()
                {
                    return None;
                }
                let bars = &self.chart.bars;
                let hi = to.map_or(bars.len(), |t| bars.partition_point(|b| b.time <= t));
                let lo = match from {
                    Some(f) => bars.partition_point(|b| b.time < f).min(hi),
                    None => hi.saturating_sub(n),
                };
                // At most `count` bars, from the start of the range.
                let hi = hi.min(lo + n);
                let mut csv = String::from("time,open,high,low,close,volume\n");
                for b in &bars[lo..hi] {
                    csv.push_str(&format!(
                        "{},{},{},{},{},{}\n",
                        super::control::iso(b.time),
                        b.open,
                        b.high,
                        b.low,
                        b.close,
                        b.volume
                    ));
                }
                if bars.is_empty() {
                    return Some(error("The chart has no bars yet."));
                }
                if from.is_some_and(|f| bars[0].time > f) {
                    csv.push_str(&format!("# history starts at {}\n", super::control::iso(bars[0].time)));
                }
                text(csv)
            }
            "search_symbols" | "open_symbol" => {
                let open = name == "open_symbol";
                let q = s(if open { "symbol" } else { "query" });
                let then = if open { Then::Open } else { Then::List };
                return self.search_then((chat, request, id), &q, then);
            }
            // A deferred answer that was only waiting for bars.
            "_say" => text(s("text")),
            "set_timeframe" => match Timeframe::parse(&s("timeframe")) {
                Some(tf) => {
                    self.set_interval_synced(tf);
                    // Answer once the new bars are in.
                    let say = json!({"text": format!("The chart is on {} now.", tf.label())});
                    if self
                        .wait_for_history((chat, request, id), i64::MAX, "_say", &say)
                        .is_some()
                    {
                        return None;
                    }
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
                let uid = self.find_study(&s("name"));
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
            "chart_view" => return self.tool_chart_view((chat, request, id), a),
            "indicator" => self.tool_indicator(a),
            "drawings" => return self.tool_drawings((chat, request, id), a),
            "replay" => return self.tool_replay((chat, request, id), a),
            "snapshot" => return self.tool_snapshot((chat, request, id)),
            "watchlist" => return self.tool_watchlist((chat, request, id), a),
            "screener" => return self.tool_screener((chat, request, id), a),
            "strategy_settings" => self.tool_strategy_settings(a),
            "strategy_lab" => return self.tool_strategy_lab((chat, request, id), a),
            "paper_trading" => self.tool_paper(a),
            "alerts" => self.tool_alerts(a),
            "open_panel" => self.tool_open_panel(a),
            "seasonality" => return self.tool_seasonality((chat, request, id), a),
            "custom_index" => return self.tool_custom_index((chat, request, id), a),
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

    /// Starts a symbol search whose best match `then` gets used. Answers
    /// later (`None`), or at once with an error.
    pub(crate) fn search_then(
        &mut self,
        (chat, request, id): (u64, &str, &Value),
        q: &str,
        then: Then,
    ) -> Option<Value> {
        let q = q.trim();
        if q.is_empty() {
            return Some(error("Give a symbol or search text."));
        }
        self.claude.search_seq += 1;
        let seq = SEARCH_BASE + self.claude.search_seq;
        self.hub.send(opentern_data::Request::Search {
            seq,
            query: q.to_string(),
        });
        self.claude.pending.push(Pending {
            chat,
            request: request.to_string(),
            id: id.clone(),
            seq,
            query: q.to_string(),
            then,
        });
        None
    }

    /// Search results for one of the agent's lookups.
    pub(crate) fn claude_search_done(&mut self, seq: u64, results: Vec<opentern_core::Symbol>) {
        let Some(i) = self.claude.pending.iter().position(|p| p.seq == seq) else {
            return;
        };
        let p = self.claude.pending.remove(i);
        let q = p.query.to_uppercase();
        let best = results
            .iter()
            .find(|s| s.id.to_uppercase() == q)
            .or_else(|| results.iter().find(|s| s.ticker().to_uppercase() == q))
            .or(results.first())
            .cloned();
        let result = match (p.then, best) {
            (Then::Watch { batch, list }, best) => {
                self.watch_found(batch, list, &p.query, best);
                return;
            }
            (Then::Index { batch }, best) => {
                self.index_found(batch, &p.query, best);
                return;
            }
            (Then::List, _) => {
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
            }
            (_, None) => error(format!("Nothing matches \"{}\".", p.query)),
            (Then::Open, Some(sym)) => {
                let msg = format!("The chart shows {} ({}) now.", sym.id, sym.description);
                self.open_symbol_synced(sym);
                // Answer once its bars are in, so the next call sees them.
                self.claude.waiting.push(super::control::Waiting {
                    chat: p.chat,
                    request: p.request,
                    id: p.id,
                    wait: super::control::Wait::History {
                        from: i64::MAX,
                        name: "_say".into(),
                        args: json!({"text": msg}),
                        asked_at: None,
                    },
                    since: std::time::Instant::now(),
                });
                return;
            }
            (Then::Compare(mode), Some(sym)) => {
                let msg = format!("Comparing with {} ({}).", sym.id, sym.description);
                self.add_compare(sym, mode);
                text(msg)
            }
        };
        self.tool_answer(p.chat, &p.request, &p.id, result);
    }
}

/// `scripting_guide`: the scripting API and the Pine notes.
const GUIDE: &str = concat!(
    "# Pine Script\n\nOpentern runs Pine Script v5 and v6 natively (indicator, strategy and library scripts, ",
    "request.security, drawings, arrays/maps/matrices, UDTs, methods, enums). `import TradingView/ta/9` works: ",
    "it's bundled. Other libraries must be scripts in the scripts folder (found by their library(\"Name\") title).\n\n",
    include_str!("../../../../docs/scripting.md")
);

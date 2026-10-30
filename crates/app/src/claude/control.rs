//! The agent's hands: tools that do what the user does with clicks
//! (chart view, indicators' inputs, drawings, bar replay, snapshots,
//! watchlists, the screener, strategy settings, the Strategy Lab, paper
//! trading, alerts and the bottom panel). `tools.rs` lists them and routes
//! calls here. Each area is one tool with an `action`, which keeps the tool
//! list short (it goes with every request).
//!
//! Slow work answers later (`Wait`): loading older history, a screener or
//! lab run, a snapshot. `App::poll_tool_waits` replies when it's done.

use super::tools::{Then, error, pretty, text};
use crate::app::App;
use crate::chart::draw::{Anchor, DrawStyle, Drawing, Tool};
use crate::shell::Tab;
use opentern_series::study::{InputKind, Value as In};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// Something a tool call waits for before it answers.
pub enum Wait {
    /// Bars back to `from` on the active chart (or any bars at all), then
    /// the call `name(args)` runs again.
    History {
        from: i64,
        name: String,
        args: Value,
        /// How many bars there were when older ones were last asked for.
        asked_at: Option<usize>,
    },
    Screener,
    Lab,
    Snapshot,
    /// The Seasonality tab's monthly bars.
    Seasonal {
        from: Option<i16>,
        to: Option<i16>,
    },
    /// A custom index's board (after a save or rebalance), answered with
    /// `note` first.
    Index {
        ticker: String,
        rebalance: bool,
        note: String,
    },
}

pub struct Waiting {
    pub chat: u64,
    pub request: String,
    pub id: Value,
    pub wait: Wait,
    pub since: Instant,
}

/// Several symbols being looked up for one call (watchlist adds).
pub struct Batch {
    pub chat: u64,
    pub request: String,
    pub id: Value,
    pub left: usize,
    pub added: Vec<String>,
    pub missed: Vec<String>,
    /// Lookups for a custom index's members, saved once all are back.
    pub index: Option<super::markets::IndexJob>,
}

/// Letters and digits only, lowercased: "Heikin Ashi" = "heikin_ashi".
pub fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// `TrendLine` → `trend_line`.
fn snake(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// A time as the agent writes it: ms or seconds since 1970, an RFC 3339
/// timestamp, or a date / date-time read as UTC.
pub fn parse_time(v: &Value) -> Option<i64> {
    if let Some(n) = v.as_f64() {
        return Some(if n.abs() > 1e11 { n as i64 } else { (n * 1000.0) as i64 });
    }
    let s = v.as_str()?.trim();
    if let Ok(t) = s.parse::<jiff::Timestamp>() {
        return Some(t.as_millisecond());
    }
    let utc = jiff::tz::TimeZone::UTC;
    if let Ok(dt) = s.replace(' ', "T").parse::<jiff::civil::DateTime>() {
        return dt.to_zoned(utc).ok().map(|z| z.timestamp().as_millisecond());
    }
    s.parse::<jiff::civil::Date>()
        .ok()
        .and_then(|d| d.to_zoned(utc).ok())
        .map(|z| z.timestamp().as_millisecond())
}

pub fn iso(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|t| t.to_string())
        .unwrap_or_default()
}

fn num(a: &Value, k: &str) -> Option<f64> {
    a.get(k)
        .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())))
}

fn flag(a: &Value, k: &str) -> Option<bool> {
    a.get(k).and_then(Value::as_bool)
}

fn st<'a>(a: &'a Value, k: &str) -> &'a str {
    a.get(k).and_then(Value::as_str).unwrap_or("").trim()
}

/// A list argument given as an array or as one comma-separated string.
fn list(a: &Value, k: &str) -> Vec<String> {
    match a.get(k) {
        Some(Value::Array(v)) => v
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// The drawing tools the agent can use, by their snake_case name.
fn drawing_tools() -> impl Iterator<Item = Tool> {
    Tool::ALL.into_iter().filter(|t| t.is_drawing())
}

impl App {
    /// Whether the active chart lacks bars back to `from` (or any bars)
    /// and could load them.
    pub(crate) fn needs_history(&self, from: i64) -> bool {
        match self.chart.bars.first() {
            None => true,
            Some(b) => !self.chart.complete && !self.chart.replaying() && b.time > from,
        }
    }

    /// Defers the call until the chart has bars back to `from`. `None`:
    /// it's already there (or was waited for), go ahead.
    pub(crate) fn wait_for_history(
        &mut self,
        (chat, request, id): (u64, &str, &Value),
        from: i64,
        name: &str,
        a: &Value,
    ) -> Option<()> {
        if a.get("_waited").is_some() || !self.needs_history(from) {
            return None;
        }
        self.claude.waiting.push(Waiting {
            chat,
            request: request.to_string(),
            id: id.clone(),
            wait: Wait::History {
                from,
                name: name.to_string(),
                args: a.clone(),
                asked_at: None,
            },
            since: Instant::now(),
        });
        Some(())
    }

    pub(crate) fn tool_answer(&mut self, chat: u64, request: &str, id: &Value, result: Value) {
        match self.claude.clients.get(&chat) {
            Some(c) => c.mcp_reply(request, json!({"jsonrpc": "2.0", "id": id, "result": result})),
            #[cfg(test)]
            None => self.claude.answers.push(result),
            #[cfg(not(test))]
            None => {}
        }
    }

    /// Answers the calls whose wait is over. Every frame.
    pub(crate) fn poll_tool_waits(&mut self) {
        if self.claude.waiting.is_empty() {
            return;
        }
        let waiting = std::mem::take(&mut self.claude.waiting);
        let mut keep = Vec::new();
        for mut w in waiting {
            let age = w.since.elapsed();
            let result = match &mut w.wait {
                Wait::History {
                    from,
                    name,
                    args,
                    asked_at,
                } => {
                    if !self.needs_history(*from) || age > Duration::from_secs(45) {
                        let mut args = args.clone();
                        args["_waited"] = json!(true);
                        let name = name.clone();
                        self.claude_tool(w.chat, &w.request, &w.id, &name, &args)
                    } else {
                        // Older bars, a page at a time, once the last page is in.
                        let n = self.chart.bars.len();
                        if n > 0 && *asked_at != Some(n) {
                            *asked_at = Some(n);
                            self.hub.send(opentern_data::Request::More { chart: self.chart.id });
                        }
                        None
                    }
                }
                Wait::Screener => {
                    if self.screener.pending == 0 {
                        Some(self.screener_results(50))
                    } else if age > Duration::from_secs(180) {
                        Some(error(format!(
                            "The screener is still loading ({}). Call screener with action results later.",
                            self.screener.status
                        )))
                    } else {
                        None
                    }
                }
                Wait::Lab => {
                    if !self.lab_busy() {
                        Some(self.lab_results())
                    } else if age > Duration::from_secs(600) {
                        Some(text(
                            "The Strategy Lab is still running. Call strategy_lab with action results later.",
                        ))
                    } else {
                        None
                    }
                }
                Wait::Seasonal { from, to } => {
                    let (from, to) = (*from, *to);
                    match self.seasonal_report(from, to) {
                        Some(r) => Some(super::markets::seasonal_answer(r)),
                        None if age > Duration::from_secs(60) => {
                            Some(error("The monthly history is still loading. Try again in a moment."))
                        }
                        None => None,
                    }
                }
                Wait::Index {
                    ticker,
                    rebalance,
                    note,
                } => {
                    let (ticker, rebalance, note) = (ticker.clone(), *rebalance, note.clone());
                    if self.index_settled(&ticker, rebalance) || age > super::markets::INDEX_WAIT {
                        let mut r = self.index_detail(&ticker);
                        if !note.is_empty()
                            && let Some(t) = r["content"][0]["text"].as_str()
                        {
                            r["content"][0]["text"] = json!(format!("{note}\n{t}"));
                        }
                        Some(r)
                    } else {
                        None
                    }
                }
                Wait::Snapshot => match self.claude.picture.take() {
                    Some(png) => Some(json!({"content": [
                        {"type": "image", "data": png, "mimeType": "image/png"},
                        {"type": "text", "text": "The charts as the user sees them now."},
                    ]})),
                    None if age > Duration::from_secs(5) => {
                        self.snap = None;
                        Some(error(
                            "Couldn't take a picture of the chart (is the window minimised?).",
                        ))
                    }
                    None => None,
                },
            };
            match result {
                Some(r) => self.tool_answer(w.chat, &w.request, &w.id, r),
                None if matches!(w.wait, Wait::History { .. }) && age > Duration::from_secs(45) => {}
                None => keep.push(w),
            }
        }
        // Calls made while answering (a rerun that waits again) go after.
        keep.append(&mut self.claude.waiting);
        self.claude.waiting = keep;
        if !self.claude.waiting.is_empty() {
            self.egui_ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// A study on the active chart by its title (with or without inputs)
    /// or its script's file.
    pub(crate) fn find_study(&self, name: &str) -> Option<u64> {
        let n = name.trim().to_lowercase();
        if n.is_empty() {
            return None;
        }
        let by_file = self
            .scripts
            .iter()
            .find(|r| r.file.to_lowercase() == n)
            .and_then(|r| r.uid);
        by_file.or_else(|| {
            let studies = &self.chart.studies;
            studies
                .iter()
                .find(|s| s.title(true).to_lowercase() == n || s.title(false).to_lowercase() == n)
                .or_else(|| studies.iter().find(|s| s.meta().name.to_lowercase() == n))
                .or_else(|| studies.iter().find(|s| s.title(true).to_lowercase().contains(&n)))
                .map(|s| s.uid)
        })
    }

    // ------------------------------------------------------- chart_view

    pub(crate) fn tool_chart_view(&mut self, ctx: (u64, &str, &Value), a: &Value) -> Option<Value> {
        Some(match st(a, "action") {
            "chart_type" => {
                let want = norm(st(a, "type"));
                match crate::chart::style::SeriesKind::ALL
                    .into_iter()
                    .find(|k| norm(k.name()) == want || norm(&format!("{k:?}")) == want)
                {
                    Some(k) => {
                        let mut s = self.chart.style.clone();
                        s.kind = k;
                        self.set_chart_style(s);
                        text(format!("The chart shows {} now.", k.name()))
                    }
                    None => error(format!(
                        "No chart type \"{}\". Use one of: {}.",
                        st(a, "type"),
                        crate::chart::style::SeriesKind::ALL.map(|k| k.name()).join(", ")
                    )),
                }
            }
            "go_to" => {
                let Some(from) = a.get("from").and_then(parse_time) else {
                    return Some(error("go_to needs \"from\" (a date like 2024-03-15, or a UTC time)."));
                };
                let to = a.get("to").and_then(parse_time);
                if self.wait_for_history(ctx, from, "chart_view", a).is_some() {
                    return None;
                }
                let bars = &self.chart.bars;
                if bars.is_empty() {
                    return Some(error("The chart has no bars yet."));
                }
                let i0 = bars.partition_point(|b| b.time < from).min(bars.len() - 1);
                let i1 = match to {
                    Some(t) => bars.partition_point(|b| b.time <= t).saturating_sub(1).max(i0),
                    None => (i0 + 60).min(bars.len() - 1),
                };
                let width = self.chart_ui.main_rect.width();
                let width = if width > 50.0 { width } else { 900.0 };
                let span = (i1 - i0 + 1) as f32;
                self.chart.view.bar_w = (width / (span + 6.0)).clamp(0.4, 80.0);
                self.chart.view.right = i1 as f64 + 3.0;
                for p in &mut self.chart.panes {
                    p.scale.auto = true;
                }
                let note = if bars[0].time > from {
                    " (that's as far back as this symbol's history goes)"
                } else {
                    ""
                };
                text(format!(
                    "Showing {} to {}{note}.",
                    iso(bars[i0].time),
                    iso(bars[i1].time)
                ))
            }
            "reset" => {
                self.chart.reset_view();
                text("Back to the latest bars.")
            }
            "scale" => {
                if let Some(v) = flag(a, "log") {
                    self.chart.log = v;
                    if v {
                        self.chart.percent = false;
                    }
                }
                if let Some(v) = flag(a, "percent") {
                    self.chart.percent = v;
                    if v {
                        self.chart.log = false;
                    }
                }
                if let Some(v) = flag(a, "auto")
                    && let Some(p) = self.chart.panes.first_mut()
                {
                    p.scale.auto = v;
                }
                self.chart.touch();
                text(format!(
                    "Scale: {}{}.",
                    if self.chart.log {
                        "log"
                    } else if self.chart.percent {
                        "percent"
                    } else {
                        "linear"
                    },
                    if self.chart.panes.first().is_some_and(|p| p.scale.auto) {
                        ", auto"
                    } else {
                        ""
                    }
                ))
            }
            "layout" => {
                use crate::multi::Grid;
                let n = num(a, "charts").unwrap_or(1.0).round() as usize;
                let named = norm(st(a, "grid"));
                let g = Grid::ALL
                    .into_iter()
                    .find(|g| !named.is_empty() && norm(&format!("{g:?}")) == named)
                    .or_else(|| Grid::ALL.into_iter().find(|g| g.count() == n));
                match g {
                    Some(g) => {
                        self.set_grid(g);
                        text(format!(
                            "{} chart{} in the layout. Tools act on the active chart (number {}).",
                            g.count(),
                            if g.count() == 1 { "" } else { "s" },
                            self.active + 1
                        ))
                    }
                    None => error("Layouts hold 1 to 8 charts."),
                }
            }
            "activate" => {
                let k = num(a, "chart").unwrap_or(1.0).round() as usize;
                if k == 0 || k > self.chart_count() {
                    return Some(error(format!(
                        "There are {} charts; pick 1 to {}.",
                        self.chart_count(),
                        self.chart_count()
                    )));
                }
                self.activate(k - 1);
                text(format!(
                    "Chart {k} ({} {}) is active.",
                    self.chart.symbol.id,
                    self.chart.tf.label()
                ))
            }
            "compare" => {
                use crate::chart::CompareMode;
                let mode = match norm(st(a, "mode")).as_str() {
                    "scale" | "overlay" => CompareMode::Scale,
                    "pane" => CompareMode::Pane,
                    _ => CompareMode::Percent,
                };
                return self.search_then(ctx, st(a, "symbol"), Then::Compare(mode));
            }
            "remove_compare" => {
                let s = st(a, "symbol").to_uppercase();
                match self
                    .chart
                    .compares
                    .iter()
                    .position(|c| c.symbol.id.to_uppercase() == s || c.symbol.ticker().to_uppercase() == s)
                {
                    Some(i) => {
                        self.remove_compare(i);
                        text(format!("Removed {s} from the chart."))
                    }
                    None => error(format!("{s} isn't compared on this chart.")),
                }
            }
            "indicator_template" => {
                let name = st(a, "name");
                let all: Vec<String> = self
                    .store
                    .templates("indicators")
                    .iter()
                    .map(|t| t.name.clone())
                    .collect();
                match all.iter().find(|n| n.eq_ignore_ascii_case(name)) {
                    Some(n) => {
                        let n = n.clone();
                        self.apply_indicator_template(&n);
                        text(format!("Applied the {n} template."))
                    }
                    None if all.is_empty() => error("There are no indicator templates yet."),
                    None => error(format!("Templates: {}.", all.join(", "))),
                }
            }
            other => error(format!(
                "chart_view has no action \"{other}\". Use chart_type, go_to, reset, scale, layout, activate, compare, remove_compare or indicator_template."
            )),
        })
    }

    // -------------------------------------------------------- indicator

    pub(crate) fn tool_indicator(&mut self, a: &Value) -> Value {
        let Some(uid) = self.find_study(st(a, "name")) else {
            return error(format!(
                "Nothing called \"{}\" is on the chart. get_chart lists what is.",
                st(a, "name")
            ));
        };
        let Some(sv) = self.chart.studies.iter().find(|s| s.uid == uid) else {
            return error("That study just went away.");
        };
        let meta = sv.meta();
        match st(a, "action") {
            "show" | "hide" => {
                let on = st(a, "action") == "show";
                if let Some(s) = self.chart.study_mut(uid) {
                    s.visible = on;
                }
                self.chart.touch();
                text(format!("{} {}.", meta.short, if on { "shown" } else { "hidden" }))
            }
            "set" => {
                let Some(vals) = a.get("inputs").and_then(Value::as_object) else {
                    return error("set needs \"inputs\": an object of input id or title → value.");
                };
                let mut ins = sv.inst.inputs.clone();
                let mut bad = Vec::new();
                for (k, v) in vals {
                    let Some(spec) = meta
                        .inputs
                        .iter()
                        .find(|s| s.id == k || s.title.eq_ignore_ascii_case(k) || norm(s.title) == norm(k))
                    else {
                        bad.push(format!("no input \"{k}\""));
                        continue;
                    };
                    let as_text = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                    let val = match &spec.kind {
                        InputKind::Int { min, max } => v
                            .as_f64()
                            .or_else(|| as_text.parse().ok())
                            .map(|x| In::Int((x.round() as i64).clamp(*min, *max))),
                        InputKind::Float { min, max, .. } => v
                            .as_f64()
                            .or_else(|| as_text.parse().ok())
                            .map(|x| In::Float(x.clamp(*min, *max))),
                        InputKind::Bool => v.as_bool().or_else(|| as_text.parse().ok()).map(In::Bool),
                        InputKind::Choice(opts) => opts
                            .iter()
                            .find(|o| o.eq_ignore_ascii_case(&as_text))
                            .map(|o| In::Text(o.to_string())),
                        _ => Some(In::Text(as_text.clone())),
                    };
                    match val {
                        Some(x) => ins.set(spec.id, x),
                        None => bad.push(format!("{}: can't use {v}", spec.title)),
                    }
                }
                self.chart.set_inputs(uid, &ins);
                let title = self
                    .chart
                    .studies
                    .iter()
                    .find(|s| s.uid == uid)
                    .map(|s| s.title(true))
                    .unwrap_or_default();
                if bad.is_empty() {
                    text(format!("Now {title}."))
                } else {
                    error(format!("Now {title}. Skipped: {}.", bad.join("; ")))
                }
            }
            _ => {
                let inputs: Vec<Value> = meta
                    .inputs
                    .iter()
                    .map(|s| {
                        let cur = match sv.inst.inputs.values.get(s.id).unwrap_or(&s.default) {
                            In::Int(v) => json!(v),
                            In::Float(v) => json!(v),
                            In::Bool(v) => json!(v),
                            In::Text(v) => json!(v),
                        };
                        let kind = match &s.kind {
                            InputKind::Int { .. } => json!("int"),
                            InputKind::Float { .. } => json!("float"),
                            InputKind::Bool => json!("bool"),
                            InputKind::Source => json!("source: open, high, low, close, hl2, hlc3, ohlc4, hlcc4"),
                            InputKind::Choice(o) => json!({ "one_of": o }),
                            InputKind::Color => json!("colour #RRGGBBAA"),
                            _ => json!("text"),
                        };
                        json!({"id": s.id, "title": s.title, "value": cur, "kind": kind})
                    })
                    .collect();
                pretty(&json!({
                    "title": sv.title(true),
                    "visible": sv.visible,
                    "plots": meta.plots.iter().map(|p| p.title.to_string()).collect::<Vec<_>>(),
                    "inputs": inputs,
                }))
            }
        }
    }

    // --------------------------------------------------------- drawings

    /// A point given as {time | bars_ago, price}; price may be a number or
    /// "open"/"high"/"low"/"close" of the bar at that time.
    fn anchor_of(&self, p: &Value) -> Result<Anchor, String> {
        let bars = &self.chart.bars;
        let last = bars.last().ok_or("The chart has no bars yet.")?;
        let t = if let Some(k) = p.get("bars_ago").and_then(Value::as_f64) {
            let k = k.round() as i64;
            let i = bars.len() as i64 - 1 - k;
            if i >= 0 {
                bars[i as usize].time
            } else {
                return Err(format!("bars_ago {k} is before the first loaded bar."));
            }
        } else if p.get("time").is_none() {
            // Only a price (a horizontal line): the latest bar.
            last.time
        } else {
            let t = p
                .get("time")
                .and_then(parse_time)
                .ok_or("A point's \"time\" must be a date, a UTC time or ms since 1970.")?;
            // On a bar: snap to its open time.
            let i = bars.partition_point(|b| b.time <= t);
            if i > 0 && t <= last.time { bars[i - 1].time } else { t }
        };
        let bar = bars.get(bars.partition_point(|b| b.time < t)).filter(|b| b.time == t);
        let price = match p.get("price") {
            Some(Value::String(s)) if bar.is_some() => {
                let b = bar.expect("checked");
                match norm(s).as_str() {
                    "open" => b.open,
                    "high" => b.high,
                    "low" => b.low,
                    "close" => b.close,
                    _ => s.trim().parse().map_err(|_| format!("\"{s}\" isn't a price."))?,
                }
            }
            Some(v) => v
                .as_f64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
                .ok_or("A point's price must be a number, or open/high/low/close of its bar.")?,
            None => return Err("Each point needs a \"price\".".into()),
        };
        Ok(Anchor { t, p: price })
    }

    fn drawing_from(&self, d: &Value, id: u64) -> Result<Drawing, String> {
        let want = norm(st(d, "tool"));
        let tool = drawing_tools()
            .find(|t| norm(&format!("{t:?}")) == want)
            .ok_or_else(|| {
                format!(
                    "No drawing tool \"{}\". Use one of: {}.",
                    st(d, "tool"),
                    drawing_tools()
                        .map(|t| snake(&format!("{t:?}")))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
        let pts: Vec<Anchor> = d
            .get("points")
            .and_then(Value::as_array)
            .ok_or("Each drawing needs \"points\".")?
            .iter()
            .map(|p| self.anchor_of(p))
            .collect::<Result<_, _>>()?;
        let need = match tool {
            Tool::LongPosition | Tool::ShortPosition => 3,
            Tool::Path | Tool::Brush => 2,
            t => t.points(),
        };
        let ok = match tool {
            Tool::Path | Tool::Brush => pts.len() >= need,
            _ => pts.len() == need,
        };
        if !ok {
            let what = match tool {
                Tool::LongPosition | Tool::ShortPosition => {
                    "3 points: entry, then target and stop (both at the time the box ends)".to_string()
                }
                Tool::Path | Tool::Brush => "2 or more points".to_string(),
                _ => format!("{need} point{}", if need == 1 { "" } else { "s" }),
            };
            return Err(format!("{} takes {what}.", tool.name()));
        }
        let mut style = self
            .draw_styles
            .get(&tool)
            .cloned()
            .unwrap_or_else(|| DrawStyle::for_tool(tool));
        if let Some(c) = d.get("color").and_then(Value::as_str) {
            style.color = crate::scripting::parse_color(c);
            style.fill = style.color.transp(80);
        }
        if let Some(c) = d.get("fill").and_then(Value::as_str) {
            style.fill = crate::scripting::parse_color(c);
        }
        if let Some(w) = num(d, "width") {
            style.width = w.clamp(1.0, 4.0) as u8;
        }
        if let Some(f) = num(d, "font_size") {
            style.font = f.clamp(8.0, 40.0) as u8;
        }
        use opentern_series::study::LineStyle;
        match norm(st(d, "line_style")).as_str() {
            "dashed" => style.line = LineStyle::Dashed,
            "dotted" => style.line = LineStyle::Dotted,
            "solid" => style.line = LineStyle::Solid,
            _ => {}
        }
        if let Some(v) = flag(d, "extend_left") {
            style.extend_left = v;
        }
        if let Some(v) = flag(d, "extend_right") {
            style.extend_right = v;
        }
        Ok(Drawing {
            id,
            tool,
            pts,
            style,
            text: st(d, "text").to_string(),
            locked: false,
            hidden: false,
        })
    }

    pub(crate) fn tool_drawings(&mut self, ctx: (u64, &str, &Value), a: &Value) -> Option<Value> {
        Some(match st(a, "action") {
            "add" => {
                let items: Vec<Value> = match a.get("drawings") {
                    Some(Value::Array(v)) => v.clone(),
                    _ if a.get("tool").is_some() => vec![a.clone()],
                    _ => return Some(error("add needs \"drawings\": a list of {tool, points, …}.")),
                };
                // Points far back need those bars loaded first.
                let earliest = items
                    .iter()
                    .flat_map(|d| d["points"].as_array().cloned().unwrap_or_default())
                    .filter_map(|p| p.get("time").and_then(parse_time))
                    .min()
                    .unwrap_or(i64::MAX);
                if self.wait_for_history(ctx, earliest, "drawings", a).is_some() {
                    return None;
                }
                let mut made = Vec::new();
                let mut id = self.chart.next_drawing_id();
                for (k, d) in items.iter().enumerate() {
                    match self.drawing_from(d, id) {
                        Ok(dr) => {
                            made.push(dr);
                            id += 1;
                        }
                        Err(e) => {
                            return Some(error(format!(
                                "Drawing {} wasn't added (nor any after it in this call): {e}",
                                k + 1
                            )));
                        }
                    }
                }
                self.chart.push_undo();
                let ids: Vec<u64> = made.iter().map(|d| d.id).collect();
                self.chart.drawings.extend(made);
                self.chart.drawings_changed += 1;
                self.layout.hide_drawings = false;
                pretty(&json!({"added": ids, "note": "The user can undo with Ctrl+Z."}))
            }
            "remove" => {
                let ids: Vec<u64> = a
                    .get("ids")
                    .and_then(Value::as_array)
                    .map(|v| v.iter().filter_map(Value::as_u64).collect())
                    .unwrap_or_default();
                if ids.is_empty() {
                    return Some(error("remove needs \"ids\" (from list or add)."));
                }
                self.chart.push_undo();
                let before = self.chart.drawings.len();
                self.chart.drawings.retain(|d| !ids.contains(&d.id));
                self.chart.drawings_changed += 1;
                if self.chart_ui.draw.selected.is_some_and(|s| ids.contains(&s)) {
                    self.chart_ui.draw.selected = None;
                }
                text(format!("Removed {}.", before - self.chart.drawings.len()))
            }
            "clear" => {
                let n = self.chart.drawings.len();
                self.chart.push_undo();
                self.chart.drawings.clear();
                self.chart.drawings_changed += 1;
                self.chart_ui.draw.selected = None;
                text(format!(
                    "Removed all {n} drawings on this symbol (Ctrl+Z brings them back)."
                ))
            }
            _ => {
                let list: Vec<Value> = self
                    .chart
                    .drawings
                    .iter()
                    .map(|d| {
                        json!({
                            "id": d.id,
                            "tool": snake(&format!("{:?}", d.tool)),
                            "points": d.pts.iter().map(|p| json!({"time": iso(p.t), "price": p.p})).collect::<Vec<_>>(),
                            "text": d.text,
                            "hidden": d.hidden,
                        })
                    })
                    .collect();
                pretty(&json!({"symbol": self.chart.symbol.id, "drawings": list}))
            }
        })
    }

    // ----------------------------------------------------------- replay

    fn replay_status(&self) -> Value {
        let c = &self.chart;
        let Some(r) = c.replay.as_ref().filter(|r| !r.selecting) else {
            return json!({"replaying": false});
        };
        let b = c.bars.last();
        json!({
            "replaying": true,
            "playing": r.playing,
            "speed": crate::chart::replay::SPEEDS[r.speed.min(5)].1,
            "bars_left": r.left(),
            "current_bar": b.map(|b| json!({
                "time": iso(b.time), "open": b.open, "high": b.high, "low": b.low, "close": b.close, "volume": b.volume,
            })),
        })
    }

    pub(crate) fn tool_replay(&mut self, ctx: (u64, &str, &Value), a: &Value) -> Option<Value> {
        Some(match st(a, "action") {
            "start" => {
                let Some(t) = a.get("time").and_then(parse_time) else {
                    return Some(error("start needs \"time\": the last bar to show before stepping."));
                };
                if self.chart.replaying() {
                    self.chart.exit_replay();
                }
                if self.wait_for_history(ctx, t, "replay", a).is_some() {
                    return None;
                }
                let bars = &self.chart.bars;
                if bars.is_empty() {
                    return Some(error("The chart has no bars yet."));
                }
                let i = bars.partition_point(|b| b.time <= t).saturating_sub(1);
                if i + 1 >= bars.len() {
                    return Some(error("That's the latest bar; start earlier."));
                }
                self.chart.replay_from(i);
                pretty(&self.replay_status())
            }
            "step" => {
                if !self.chart.replaying() {
                    return Some(error("Start a replay first."));
                }
                let n = num(a, "bars").unwrap_or(1.0).clamp(1.0, 1000.0) as usize;
                let mut stepped = 0;
                while stepped < n && self.chart.replay_step() {
                    stepped += 1;
                }
                let mut s = self.replay_status();
                s["stepped"] = json!(stepped);
                pretty(&s)
            }
            "play" | "pause" => {
                let play = st(a, "action") == "play";
                let speed = st(a, "speed").to_string();
                let Some(r) = self.chart.replay.as_mut().filter(|r| !r.selecting) else {
                    return Some(error("Start a replay first."));
                };
                r.playing = play;
                if let Some(k) = crate::chart::replay::SPEEDS
                    .iter()
                    .position(|(_, l)| *l == speed || speed.trim_end_matches(" per bar") == *l)
                {
                    r.speed = k;
                }
                self.egui_ctx.request_repaint();
                pretty(&self.replay_status())
            }
            "stop" => {
                self.chart.exit_replay();
                text("Replay over; the chart is live again.")
            }
            _ => pretty(&self.replay_status()),
        })
    }

    // --------------------------------------------------------- snapshot

    pub(crate) fn tool_snapshot(&mut self, (chat, request, id): (u64, &str, &Value)) -> Option<Value> {
        if crate::app::headless() && std::env::var_os("OPENTERN_CLAUDE").is_none() {
            return Some(error("There's no window to take a picture of."));
        }
        self.claude.picture = None;
        self.take_snapshot(crate::snapshot::Snap::Agent);
        self.claude.waiting.push(Waiting {
            chat,
            request: request.to_string(),
            id: id.clone(),
            wait: Wait::Snapshot,
            since: Instant::now(),
        });
        None
    }

    // -------------------------------------------------------- watchlist

    fn list_index(&self, name: &str) -> Option<usize> {
        if name.is_empty() {
            return Some(self.active_list);
        }
        self.watchlists
            .iter()
            .position(|w| w.name.eq_ignore_ascii_case(name))
            .or_else(|| self.watchlists.iter().position(|w| norm(&w.name) == norm(name)))
    }

    fn watchlist_json(&self, k: usize) -> Value {
        use crate::watchlists::Item;
        let w = &self.watchlists[k];
        let items: Vec<Value> = w
            .items
            .iter()
            .map(|i| match i {
                Item::Section { section, .. } => json!({"section": section}),
                Item::Symbol(s) => {
                    let q = self.quotes.get(&s.id);
                    json!({
                        "symbol": s.id,
                        "last": q.map(|q| q.0),
                        "change_pct": q.filter(|q| q.1 != 0.0).map(|q| ((q.0 / q.1 - 1.0) * 10_000.0).round() / 100.0),
                    })
                }
            })
            .collect();
        json!({"name": w.name, "active": k == self.active_list, "items": items})
    }

    pub(crate) fn tool_watchlist(&mut self, ctx: (u64, &str, &Value), a: &Value) -> Option<Value> {
        use crate::watchlists::{Item, Watchlist};
        let name = st(a, "name");
        Some(match st(a, "action") {
            "list" => pretty(&json!({
                "lists": self.watchlists.iter().enumerate().map(|(k, w)| json!({
                    "name": w.name, "symbols": w.symbols().len(), "active": k == self.active_list,
                })).collect::<Vec<_>>(),
                "open": self.watchlist_json(self.active_list),
            })),
            "show" => match self.list_index(name) {
                Some(k) => pretty(&self.watchlist_json(k)),
                None => error(format!("No watchlist called \"{name}\".")),
            },
            "create" => {
                if name.is_empty() {
                    return Some(error("create needs a \"name\"."));
                }
                let id = self.watchlists.iter().map(|w| w.id).max().unwrap_or(0) + 1;
                self.watchlists.push(Watchlist {
                    id,
                    name: name.to_string(),
                    items: Vec::new(),
                });
                self.active_list = self.watchlists.len() - 1;
                self.save_watchlists();
                if list(a, "symbols").is_empty() {
                    text(format!("Made the watchlist {name}; it's open."))
                } else {
                    return self.watch_add(ctx, a, id);
                }
            }
            "switch" => match self.list_index(name) {
                Some(k) => {
                    self.active_list = k;
                    self.save_watchlists();
                    text(format!("{} is open.", self.watchlists[k].name))
                }
                None => error(format!("No watchlist called \"{name}\".")),
            },
            "rename" => {
                let new = st(a, "new_name");
                match self.list_index(name) {
                    Some(k) if !new.is_empty() => {
                        self.watchlists[k].name = new.to_string();
                        self.save_watchlists();
                        text(format!("Renamed it {new}."))
                    }
                    Some(_) => error("rename needs \"new_name\"."),
                    None => error(format!("No watchlist called \"{name}\".")),
                }
            }
            "delete" => match self.list_index(name) {
                _ if self.watchlists.len() <= 1 => error("That's the only watchlist; it can't go."),
                Some(k) if !name.is_empty() => {
                    let gone = self.watchlists.remove(k).name;
                    self.active_list = self.active_list.min(self.watchlists.len() - 1);
                    if k < self.active_list {
                        self.active_list -= 1;
                    }
                    self.save_watchlists();
                    text(format!("Deleted {gone}."))
                }
                _ => error("delete needs the list's \"name\"."),
            },
            "add" => {
                let Some(k) = self.list_index(name) else {
                    return Some(error(format!("No watchlist called \"{name}\".")));
                };
                let id = self.watchlists[k].id;
                return self.watch_add(ctx, a, id);
            }
            "remove" => {
                let Some(k) = self.list_index(name) else {
                    return Some(error(format!("No watchlist called \"{name}\".")));
                };
                let gone: Vec<String> = list(a, "symbols").iter().map(|s| s.to_uppercase()).collect();
                let w = &mut self.watchlists[k];
                let before = w.items.len();
                w.items.retain(|i| match i {
                    Item::Symbol(s) => {
                        !gone.contains(&s.id.to_uppercase()) && !gone.contains(&s.ticker().to_uppercase())
                    }
                    Item::Section { .. } => true,
                });
                let n = before - w.items.len();
                self.save_watchlists();
                text(format!("Removed {n} symbol{}.", if n == 1 { "" } else { "s" }))
            }
            "add_section" => {
                let Some(k) = self.list_index(st(a, "list")) else {
                    return Some(error(format!("No watchlist called \"{}\".", st(a, "list"))));
                };
                if name.is_empty() {
                    return Some(error("add_section needs the section's \"name\"."));
                }
                self.watchlists[k].items.push(Item::Section {
                    section: name.to_string(),
                    collapsed: false,
                });
                self.save_watchlists();
                text(format!("Added the section {name}; symbols added next go under it."))
            }
            other => error(format!(
                "watchlist has no action \"{other}\". Use list, show, create, switch, rename, delete, add, remove or add_section."
            )),
        })
    }

    /// Looks up each symbol and adds the matches to watchlist `list`.
    fn watch_add(&mut self, (chat, request, id): (u64, &str, &Value), a: &Value, list_id: i64) -> Option<Value> {
        let symbols = list(a, "symbols");
        if symbols.is_empty() {
            return Some(error("add needs \"symbols\", e.g. [\"BTCUSDT\", \"NASDAQ:AAPL\"]."));
        }
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
                index: None,
            },
        );
        for s in symbols {
            self.search_then(
                (chat, request, id),
                &s,
                Then::Watch {
                    batch: b,
                    list: list_id,
                },
            );
        }
        None
    }

    /// One watchlist lookup came back.
    pub(crate) fn watch_found(&mut self, batch: u64, list_id: i64, query: &str, found: Option<opentern_core::Symbol>) {
        use crate::watchlists::Item;
        let Some(b) = self.claude.batches.get_mut(&batch) else {
            return;
        };
        b.left = b.left.saturating_sub(1);
        match found {
            Some(s) => {
                b.added.push(s.id.clone());
                if let Some(w) = self.watchlists.iter_mut().find(|w| w.id == list_id)
                    && !w.items.iter().any(|i| matches!(i, Item::Symbol(x) if x.id == s.id))
                {
                    w.items.push(Item::Symbol(s));
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
        self.save_watchlists();
        let name = self
            .watchlists
            .iter()
            .find(|w| w.id == list_id)
            .map(|w| w.name.clone())
            .unwrap_or_default();
        let mut msg = format!("Added to {name}: {}.", b.added.join(", "));
        if b.added.is_empty() {
            msg = format!("Nothing was added to {name}.");
        }
        if !b.missed.is_empty() {
            msg.push_str(&format!(" No match for: {}.", b.missed.join(", ")));
        }
        let result = if b.added.is_empty() { error(msg) } else { text(msg) };
        self.tool_answer(b.chat, &b.request, &b.id, result);
    }

    // --------------------------------------------------------- screener

    pub(crate) fn screener_results(&self, top: usize) -> Value {
        use crate::screener::Field;
        let sc = &self.screener;
        let setup = &sc.setup;
        let mut rows: Vec<&crate::screener::Row> = sc.rows.iter().filter(|r| r.passes(&setup.filters)).collect();
        rows.sort_by(|a, b| {
            b.get(Field::Change)
                .partial_cmp(&a.get(Field::Change))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let out: Vec<Value> = rows
            .iter()
            .take(top)
            .map(|r| {
                let mut o = serde_json::Map::new();
                o.insert("symbol".into(), json!(r.symbol.id));
                for f in Field::ALL {
                    let v = r.get(f);
                    if v.is_finite() {
                        o.insert(snake(&format!("{f:?}")), json!((v * 1e4).round() / 1e4));
                    }
                }
                Value::Object(o)
            })
            .collect();
        let errors: Vec<Value> = sc
            .rows
            .iter()
            .filter_map(|r| r.error.as_ref().map(|e| json!({"symbol": r.symbol.id, "error": e})))
            .collect();
        pretty(&json!({
            "status": sc.status,
            "universe": self.universe_name(),
            "timeframe": setup.tf,
            "symbols": sc.rows.len(),
            "passing_filters": rows.len(),
            "rows": out,
            "errors": errors,
            "fields": "change, perf_week, perf_month, sma50, sma200 and atr are percentages; rsi is RSI(14); script is the script column",
        }))
    }

    pub(crate) fn tool_screener(&mut self, (chat, request, id): (u64, &str, &Value), a: &Value) -> Option<Value> {
        use crate::screener::{Field, Filter, Op, ScreenScript, Universe};
        self.poll_screener();
        let action = st(a, "action");
        if matches!(action, "set" | "run") {
            let u = st(a, "universe");
            if !u.is_empty() {
                let preset = crate::screener::presets()
                    .into_iter()
                    .map(|(n, _)| n)
                    .find(|n| norm(n) == norm(u));
                let list = self.watchlists.iter().find(|w| norm(&w.name) == norm(u)).map(|w| w.id);
                self.screener.setup.universe = match (list, preset) {
                    (Some(id), _) => Universe::Watchlist(id),
                    (None, Some(p)) => Universe::Preset(p.to_string()),
                    _ => {
                        return Some(error(format!(
                            "No watchlist or preset called \"{u}\". Presets: {}.",
                            crate::screener::presets()
                                .iter()
                                .map(|(n, _)| *n)
                                .collect::<Vec<_>>()
                                .join(", ")
                        )));
                    }
                };
            }
            let tf = st(a, "timeframe");
            if !tf.is_empty() {
                match opentern_core::Timeframe::parse(tf) {
                    Some(t) => self.screener.setup.tf = t.label(),
                    None => return Some(error(format!("\"{tf}\" isn't a timeframe."))),
                }
            }
            if let Some(fs) = a.get("filters").and_then(Value::as_array) {
                let mut out = Vec::new();
                for f in fs {
                    let want = norm(st(f, "field"));
                    let Some(field) = Field::ALL
                        .into_iter()
                        .find(|x| norm(&format!("{x:?}")) == want || norm(x.name()) == want)
                    else {
                        return Some(error(format!(
                            "No screener field \"{}\". Fields: {}.",
                            st(f, "field"),
                            Field::ALL.map(|x| snake(&format!("{x:?}"))).join(", ")
                        )));
                    };
                    let op = if matches!(norm(st(f, "op")).as_str(), "below" | "lt" | "lessthan") {
                        Op::Below
                    } else {
                        Op::Above
                    };
                    let Some(value) = num(f, "value") else {
                        return Some(error("Each filter needs a numeric \"value\"."));
                    };
                    out.push(Filter { field, op, value });
                }
                self.screener.setup.filters = out;
            }
            if let Some(s) = a.get("script") {
                let plot = num(s, "plot").unwrap_or(0.0) as usize;
                let b = st(s, "builtin");
                let f = st(s, "file");
                self.screener.setup.script = if !f.is_empty() {
                    ScreenScript::Pine(f.to_string(), plot)
                } else if !b.is_empty() {
                    let all = opentern_series::builtins::all();
                    match opentern_series::builtins::by_id(&b.to_lowercase())
                        .or_else(|| all.iter().find(|m| norm(m.name) == norm(b)))
                    {
                        Some(m) => ScreenScript::Builtin(m.id.to_string(), plot),
                        None => return Some(error(format!("There's no built-in called \"{b}\"."))),
                    }
                } else {
                    ScreenScript::None
                };
            }
            self.save_screener();
        }
        Some(match action {
            "run" => {
                self.run_screener();
                self.layout.tab = Some(Tab::Screener);
                self.last_tab = Tab::Screener;
                if self.screener.pending == 0 {
                    return Some(self.screener_results(50));
                }
                self.claude.waiting.push(Waiting {
                    chat,
                    request: request.to_string(),
                    id: id.clone(),
                    wait: Wait::Screener,
                    since: Instant::now(),
                });
                return None;
            }
            "results" => self.screener_results(num(a, "top").unwrap_or(50.0) as usize),
            _ => {
                let s = &self.screener.setup;
                pretty(&json!({
                    "universe": self.universe_name(),
                    "timeframe": s.tf,
                    "filters": s.filters.iter().map(|f| json!({
                        "field": snake(&format!("{:?}", f.field)),
                        "op": if f.op == Op::Above { "above" } else { "below" },
                        "value": f.value,
                    })).collect::<Vec<_>>(),
                    "script": match &s.script {
                        ScreenScript::None => Value::Null,
                        ScreenScript::Builtin(id, p) => json!({"builtin": id, "plot": p}),
                        ScreenScript::Pine(f, p) => json!({"file": f, "plot": p}),
                    },
                    "presets": crate::screener::presets().iter().map(|(n, _)| *n).collect::<Vec<_>>(),
                    "watchlists": self.watchlists.iter().map(|w| w.name.clone()).collect::<Vec<_>>(),
                    "fields": Field::ALL.map(|x| snake(&format!("{x:?}"))),
                }))
            }
        })
    }

    // ------------------------------------------------ strategy_settings

    pub(crate) fn tool_strategy_settings(&mut self, a: &Value) -> Value {
        let f = st(a, "file");
        let run = self
            .scripts
            .iter()
            .find(|r| !f.is_empty() && (r.file == f || r.file.ends_with(f)))
            .or_else(|| {
                let uid = self.strategies.current().map(|r| r.uid);
                self.scripts.iter().find(|r| r.uid.is_some() && r.uid == uid)
            });
        let Some(run) = run else {
            return error("There's no strategy on the chart. Add a strategy script first.");
        };
        let (file, props) = (run.file.clone(), run.props.clone());
        if flag(a, "reset") == Some(true) {
            self.set_strategy_overrides(&file, json!({}));
        } else if let Some(set) = a.get("set").and_then(Value::as_object) {
            let mut over = self.strategy_overrides(&file);
            for (k, v) in set {
                over[k] = v.clone();
            }
            self.set_strategy_overrides(&file, over);
        }
        let over = self.strategy_overrides(&file);
        let p = opentern_engine::Props::from_json(
            &opentern_engine::merge_json(&props, &over),
            self.chart.symbol.tick_size,
            crate::pine_host::min_qty_of(&self.chart.symbol),
        );
        pretty(&json!({
            "file": file,
            "settings": p.to_json(),
            "changed_from_script": over,
            "note": "Changing settings reruns the strategy; call strategy_report for the new results.",
        }))
    }

    // ----------------------------------------------------- strategy_lab

    pub(crate) fn tool_strategy_lab(&mut self, (chat, request, id): (u64, &str, &Value), a: &Value) -> Option<Value> {
        let action = st(a, "action");
        if matches!(action, "set" | "run")
            && let Err(e) = self.lab_configure(a)
        {
            return Some(error(e));
        }
        Some(match action {
            "run" => {
                if self.lab_busy() {
                    return Some(error("The lab is already running."));
                }
                self.lab_run();
                self.layout.tab = Some(Tab::Lab);
                self.last_tab = Tab::Lab;
                if !self.lab_busy() {
                    // It didn't start: the status says why.
                    return Some(error(self.lab.status.clone()));
                }
                self.claude.waiting.push(Waiting {
                    chat,
                    request: request.to_string(),
                    id: id.clone(),
                    wait: Wait::Lab,
                    since: Instant::now(),
                });
                return None;
            }
            "results" => self.lab_results(),
            "apply" => {
                let rank = num(a, "rank").unwrap_or(1.0).max(1.0) as usize;
                match self.lab_apply_rank(rank) {
                    Ok(m) => text(m),
                    Err(e) => error(e),
                }
            }
            "stop" => {
                self.lab_stop();
                text("Stopped.")
            }
            _ => self.lab_describe(),
        })
    }

    // ---------------------------------------------------- paper_trading

    pub(crate) fn tool_paper(&mut self, a: &Value) -> Value {
        use opentern_engine::paper::{Kind, Ticket};
        let now = crate::alerts::now_ms();
        let resolve = |app: &App, s: &str| -> String {
            if s.is_empty() {
                return app.chart.symbol.id.clone();
            }
            let up = s.to_uppercase();
            if up.contains(':') {
                return up;
            }
            if app.chart.symbol.ticker().eq_ignore_ascii_case(s) {
                return app.chart.symbol.id.clone();
            }
            let known: Vec<opentern_core::Symbol> = app.watchlists.iter().flat_map(|w| w.symbols()).collect();
            opentern_data::expr::resolve(&up, &known)
        };
        let r = match st(a, "action") {
            "order" => {
                let symbol = resolve(self, st(a, "symbol"));
                let buy = !matches!(norm(st(a, "side")).as_str(), "sell" | "short");
                let Some(qty) = num(a, "qty") else {
                    return error("order needs \"qty\".");
                };
                let price = num(a, "price").unwrap_or(0.0);
                let kind = match norm(st(a, "type")).as_str() {
                    "" | "market" => Kind::Market,
                    "limit" => Kind::Limit(price),
                    "stop" => Kind::Stop(price),
                    "stoplimit" => Kind::StopLimit {
                        stop: price,
                        limit: num(a, "limit").unwrap_or(0.0),
                        armed: false,
                    },
                    "trailing" | "trailingstop" => Kind::Trailing {
                        offset: num(a, "offset").unwrap_or(0.0),
                        best: 0.0,
                    },
                    other => return error(format!("No order type \"{other}\".")),
                };
                let last = self.last_price(&symbol);
                if matches!(kind, Kind::Market) && last.is_none() {
                    return error(format!(
                        "No price for {symbol} yet. Open it on the chart (open_symbol) first, then order."
                    ));
                }
                let ticket = Ticket {
                    symbol: symbol.clone(),
                    buy,
                    qty,
                    kind,
                    take_profit: num(a, "take_profit"),
                    stop_loss: num(a, "stop_loss"),
                };
                self.paper.account.place(ticket, last, now).map(|oid| {
                    format!(
                        "Paper {} order {oid} for {qty} {symbol} {}.",
                        if buy { "buy" } else { "sell" },
                        if matches!(kind, Kind::Market) {
                            "filled"
                        } else {
                            "placed"
                        }
                    )
                })
            }
            "cancel" => match num(a, "order_id") {
                Some(o) if self.paper.account.cancel(o as u64) => Ok(format!("Cancelled order {o}.")),
                _ => Err("No working order with that order_id.".into()),
            },
            "modify" => match (num(a, "order_id"), num(a, "price")) {
                (Some(o), Some(p)) => self
                    .paper
                    .account
                    .modify(o as u64, p)
                    .map(|_| format!("Order {o} moved to {p}.")),
                _ => Err("modify needs \"order_id\" and \"price\".".into()),
            },
            "close" => {
                let symbol = resolve(self, st(a, "symbol"));
                match (self.paper.account.position(&symbol).is_some(), self.last_price(&symbol)) {
                    (false, _) => Err(format!("No open position in {symbol}.")),
                    (true, None) => Err(format!("No price for {symbol} yet.")),
                    (true, Some(p)) => {
                        self.paper.account.close_position(&symbol, p, now);
                        Ok(format!("Closed {symbol} at {p}."))
                    }
                }
            }
            _ => {
                let acc = &self.paper.account;
                let price = |s: &str| self.last_price(s);
                return pretty(&json!({
                    "account": acc.name,
                    "initial": acc.initial,
                    "balance": acc.balance,
                    "equity": acc.equity(price),
                    "unrealized": acc.unrealized(price),
                    "positions": acc.positions.iter().map(|p| json!({
                        "symbol": p.symbol, "qty": p.qty, "avg_price": p.avg, "last": price(&p.symbol),
                    })).collect::<Vec<_>>(),
                    "orders": acc.orders.iter().map(|o| json!({
                        "order_id": o.id, "symbol": o.symbol, "side": if o.buy { "buy" } else { "sell" },
                        "qty": o.qty, "type": o.kind.name(), "price": o.kind.price(o.buy),
                        "take_profit": o.take_profit, "stop_loss": o.stop_loss,
                    })).collect::<Vec<_>>(),
                    "recent_fills": acc.fills.iter().rev().take(20).map(|f| json!({
                        "symbol": f.symbol, "side": if f.buy { "buy" } else { "sell" }, "qty": f.qty,
                        "price": f.price, "time": iso(f.time), "realized": f.realized,
                    })).collect::<Vec<_>>(),
                    "other_accounts": self.paper.others.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
                }));
            }
        };
        match r {
            Ok(m) => {
                self.paper.status = m.clone();
                self.save_paper_now();
                text(m)
            }
            Err(e) => error(e),
        }
    }

    // ----------------------------------------------------------- alerts

    pub(crate) fn tool_alerts(&mut self, a: &Value) -> Value {
        use opentern_engine::alerts::{Alert, Condition, Freq, Op};
        let find = |app: &App| num(a, "id").and_then(|id| app.alerts.list.iter().position(|(x, _)| x.id == id as i64));
        match st(a, "action") {
            "create" => {
                let op = match norm(st(a, "op")).as_str() {
                    "crossingup" | "crossesup" | "up" => Op::CrossingUp,
                    "crossingdown" | "crossesdown" | "down" => Op::CrossingDown,
                    "greaterthan" | "above" | "gt" => Op::GreaterThan,
                    "lessthan" | "below" | "lt" => Op::LessThan,
                    _ => Op::Crossing,
                };
                let freq = match norm(st(a, "freq")).as_str() {
                    "onceperbar" => Freq::OncePerBar,
                    "onceperbarclose" => Freq::OncePerBarClose,
                    "everytime" => Freq::EveryTime,
                    _ => Freq::Once,
                };
                let ind = st(a, "indicator");
                let condition = if ind.is_empty() {
                    let Some(value) = num(a, "value") else {
                        return error("A price alert needs \"value\".");
                    };
                    Condition::Price { op, value }
                } else {
                    let Some(uid) = self.find_study(ind) else {
                        return error(format!("\"{ind}\" isn't on the chart."));
                    };
                    let Some(sv) = self.chart.studies.iter().find(|s| s.uid == uid) else {
                        return error("That study just went away.");
                    };
                    let m = sv.meta();
                    let pt = st(a, "plot");
                    let k = m.plots.iter().position(|p| norm(p.title) == norm(pt)).unwrap_or(0);
                    let Some(p) = m.plots.get(k) else {
                        return error(format!("{} has no plots to alert on.", m.short));
                    };
                    Condition::Plot {
                        uid,
                        study: m.short.to_string(),
                        plot: k,
                        title: format!("{} {}", m.short, p.title),
                        op,
                        value: num(a, "value"),
                    }
                };
                let symbol = if ind.is_empty() && !st(a, "symbol").is_empty() {
                    st(a, "symbol").to_uppercase()
                } else {
                    self.chart.symbol.id.clone()
                };
                let alert = Alert {
                    name: st(a, "name").to_string(),
                    symbol: symbol.clone(),
                    tf: self.chart.tf.label(),
                    message: st(a, "message").to_string(),
                    freq,
                    condition: condition.clone(),
                    ..Alert::default()
                };
                self.save_alert(alert);
                text(format!("Alert on {symbol}: {}.", condition.summary()))
            }
            "delete" => match find(self) {
                Some(i) => {
                    let id = self.alerts.list[i].0.id;
                    self.delete_alert(id);
                    text("Deleted.")
                }
                None => error("No alert with that id."),
            },
            "pause" | "resume" => match find(self) {
                Some(i) => {
                    let mut x = self.alerts.list[i].0.clone();
                    x.active = st(a, "action") == "resume";
                    self.save_alert(x);
                    text(if st(a, "action") == "resume" {
                        "Resumed."
                    } else {
                        "Paused."
                    })
                }
                None => error("No alert with that id."),
            },
            _ => pretty(&json!(
                self.alerts
                    .list
                    .iter()
                    .map(|(x, _)| json!({
                        "id": x.id,
                        "name": x.name,
                        "symbol": x.symbol,
                        "condition": x.condition.summary(),
                        "active": x.active,
                        "fired": x.fired,
                        "last_fired": x.last_fired.map(iso),
                    }))
                    .collect::<Vec<_>>()
            )),
        }
    }

    // -------------------------------------------------------- open_panel

    pub(crate) fn tool_open_panel(&mut self, a: &Value) -> Value {
        let want = norm(st(a, "tab"));
        if matches!(want.as_str(), "" | "none" | "close") {
            self.layout.tab = None;
            return text("The bottom panel is closed.");
        }
        let tab = Tab::ALL.into_iter().find(|t| {
            let n = norm(t.title());
            n == want || n.starts_with(&want) || norm(&format!("{t:?}")) == want
        });
        match tab {
            Some(t) => {
                self.layout.tab = Some(t);
                self.last_tab = t;
                text(format!("{} is open.", t.title()))
            }
            None => error(format!(
                "No tab \"{}\". Tabs: {}.",
                st(a, "tab"),
                Tab::ALL.map(|t| t.title()).join(", ")
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_and_names() {
        assert_eq!(parse_time(&json!("1970-01-02")), Some(86_400_000));
        assert_eq!(parse_time(&json!("1970-01-01 01:00")), Some(3_600_000));
        assert_eq!(parse_time(&json!("1970-01-01T00:00:01Z")), Some(1000));
        assert_eq!(parse_time(&json!(1_700_000_000)), Some(1_700_000_000_000));
        assert_eq!(parse_time(&json!(1_700_000_000_000i64)), Some(1_700_000_000_000));
        assert_eq!(snake("FibRetracement"), "fib_retracement");
        assert_eq!(norm("Heikin Ashi"), norm("heikin_ashi"));
        assert_eq!(list(&json!({"s": "A, B"}), "s"), ["A", "B"]);
    }
}

#[cfg(test)]
mod drive_tests {
    use crate::drive::Driver;
    use serde_json::{Value, json};

    /// A tool call: its answer now, or the next deferred one.
    fn call(d: &mut Driver, name: &str, args: Value) -> Value {
        let n = d.app.claude.answers.len();
        if let Some(r) = d.app.claude_tool(1, "r", &json!(1), name, &args) {
            return r;
        }
        eprintln!("waiting on {name} {args}");
        d.wait(|a| a.claude.answers.len() > n);
        d.app.claude.answers[n].clone()
    }

    fn ok(r: &Value) -> String {
        let t = r["content"][0]["text"].as_str().unwrap_or("").to_string();
        assert!(!r["isError"].as_bool().unwrap_or(false), "tool failed: {t}");
        t
    }

    #[test]
    fn the_agent_drives_the_app() {
        let mut d = Driver::new("agent-tools");
        let n = d.app.chart.bars.len();
        assert!(n > 100);

        // Chart view.
        ok(&call(
            &mut d,
            "chart_view",
            json!({"action": "chart_type", "type": "heikin_ashi"}),
        ));
        assert_eq!(d.app.chart.style.kind, crate::chart::style::SeriesKind::HeikinAshi);
        ok(&call(
            &mut d,
            "chart_view",
            json!({"action": "chart_type", "type": "Candles"}),
        ));
        let t40 = super::iso(d.app.chart.bars[n - 40].time);
        let t20 = super::iso(d.app.chart.bars[n - 20].time);
        ok(&call(
            &mut d,
            "chart_view",
            json!({"action": "go_to", "from": t40, "to": t20}),
        ));
        assert!((d.app.chart.view.right - (n - 20) as f64).abs() < 5.0);
        ok(&call(&mut d, "chart_view", json!({"action": "scale", "log": true})));
        assert!(d.app.chart.log);

        // A range of bars.
        let csv = ok(&call(&mut d, "get_bars", json!({"from": t40, "to": t20})));
        assert_eq!(csv.lines().count(), 22, "{csv}");

        // Indicators: add, read and change inputs, hide.
        ok(&call(&mut d, "add_indicator", json!({"id": "rsi"})));
        let got = ok(&call(&mut d, "indicator", json!({"action": "get", "name": "RSI"})));
        assert!(got.contains("\"length\"") || got.contains("Length"), "{got}");
        ok(&call(
            &mut d,
            "indicator",
            json!({"action": "set", "name": "RSI", "inputs": {"Length": 21}}),
        ));
        let rsi = d.app.chart.studies.iter().find(|s| s.meta().id == "rsi").unwrap();
        assert!(rsi.title(true).contains("21"), "{}", rsi.title(true));
        ok(&call(&mut d, "indicator", json!({"action": "hide", "name": "RSI"})));

        // Drawings: several at once, on bars' highs and lows.
        let r = ok(&call(
            &mut d,
            "drawings",
            json!({"action": "add", "drawings": [
                {"tool": "rectangle", "points": [{"bars_ago": 12, "price": "high"}, {"bars_ago": 9, "price": "low"}], "text": "Engulfing", "color": "green"},
                {"tool": "arrow_up", "points": [{"bars_ago": 9, "price": "low"}], "text": "Here"},
                {"tool": "trend_line", "points": [{"time": t40, "price": 100}, {"time": t20, "price": 110}]},
            ]}),
        ));
        assert!(r.contains("added"));
        assert_eq!(d.app.chart.drawings.len(), 3);
        assert_eq!(d.app.chart.drawings[0].text, "Engulfing");
        assert_eq!(d.app.chart.drawings[0].pts[0].p, d.app.chart.bars[n - 13].high);
        let bad = call(
            &mut d,
            "drawings",
            json!({"action": "add", "drawings": [{"tool": "trend_line", "points": [{"bars_ago": 1, "price": 1}]}]}),
        );
        assert!(bad["isError"].as_bool().unwrap_or(false));
        ok(&call(&mut d, "drawings", json!({"action": "remove", "ids": [3]})));
        assert_eq!(d.app.chart.drawings.len(), 2);

        // Replay: start back in time, step, stop.
        let start = super::iso(d.app.chart.bars[n - 30].time);
        let r = ok(&call(&mut d, "replay", json!({"action": "start", "time": start})));
        assert!(r.contains("\"replaying\": true"), "{r}");
        assert_eq!(d.app.chart.bars.len(), n - 29);
        ok(&call(&mut d, "replay", json!({"action": "step", "bars": 5})));
        assert_eq!(d.app.chart.bars.len(), n - 24);
        ok(&call(&mut d, "replay", json!({"action": "stop"})));
        assert!(!d.app.chart.replaying());

        // Watchlists: a new list, filled by lookups.
        let r = ok(&call(
            &mut d,
            "watchlist",
            json!({"action": "create", "name": "Majors", "symbols": ["BTCUSDT", "ETHUSDT"]}),
        ));
        assert!(r.contains("BTCUSDT"), "{r}");
        let list = d.app.watchlists.iter().find(|w| w.name == "Majors").unwrap();
        assert_eq!(list.symbols().len(), 2);
        ok(&call(
            &mut d,
            "watchlist",
            json!({"action": "remove", "name": "Majors", "symbols": ["ETHUSDT"]}),
        ));
        assert_eq!(d.app.watchlist().len(), 1);

        // The screener on that list.
        let r = ok(&call(
            &mut d,
            "screener",
            json!({"action": "run", "universe": "Majors", "timeframe": "1D", "filters": [{"field": "rsi", "op": "above", "value": 0}]}),
        ));
        assert!(r.contains("\"symbols\": 1"), "{r}");
        assert_eq!(d.app.layout.tab, Some(crate::shell::Tab::Screener));

        // Paper trading and alerts.
        ok(&call(
            &mut d,
            "paper_trading",
            json!({"action": "order", "side": "buy", "qty": 2}),
        ));
        assert_eq!(d.app.paper.account.positions.len(), 1);
        ok(&call(&mut d, "paper_trading", json!({"action": "close"})));
        assert!(d.app.paper.account.positions.is_empty());
        ok(&call(
            &mut d,
            "alerts",
            json!({"action": "create", "value": 123.0, "op": "crossing_up"}),
        ));
        assert_eq!(d.app.alerts.list.len(), 1);
        ok(&call(&mut d, "open_panel", json!({"tab": "paper"})));
        assert_eq!(d.app.layout.tab, Some(crate::shell::Tab::Paper));
    }

    #[test]
    fn the_agent_tunes_a_strategy() {
        let mut d = Driver::new("agent-lab");
        let dir = d.app.scripts_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("cross.pine"),
            "//@version=5\nstrategy(\"Cross\", overlay = true)\n\
             fast = input.int(5, \"Fast\", minval = 1)\nslow = input.int(20, \"Slow\", minval = 1)\n\
             f = ta.sma(close, fast)\ns = ta.sma(close, slow)\n\
             if ta.crossover(f, s)\n    strategy.entry(\"L\", strategy.long)\nif ta.crossunder(f, s)\n    strategy.close(\"L\")\n",
        )
        .unwrap();
        ok(&call(&mut d, "add_script_to_chart", json!({"file": "cross.pine"})));
        d.wait(|a| a.strategies.runs.iter().any(|r| !r.broker.trades.is_empty()));

        let r = ok(&call(
            &mut d,
            "strategy_settings",
            json!({"set": {"initial_capital": 5000}}),
        ));
        assert!(r.contains("5000"), "{r}");
        d.wait(|a| {
            a.strategies
                .runs
                .first()
                .is_some_and(|r| r.broker.props.initial_capital == 5000.0)
        });

        let r = ok(&call(
            &mut d,
            "strategy_lab",
            json!({"action": "run", "mode": "optimize", "objective": "net_profit",
                   "params": [{"input": "Fast", "from": 3, "to": 9, "step": 2}, {"input": "slow", "from": 15, "to": 30, "step": 5}]}),
        ));
        assert!(r.contains("\"runs\": 16"), "{r}");
        ok(&call(&mut d, "strategy_lab", json!({"action": "apply", "rank": 1})));
        let r = ok(&call(
            &mut d,
            "strategy_lab",
            json!({"action": "run", "mode": "monte_carlo", "mc_runs": 200}),
        ));
        assert!(r.contains("chance_of_loss_pct"), "{r}");
    }

    #[test]
    fn the_agent_reads_seasonality_and_runs_custom_indices() {
        let mut d = Driver::new("agent-markets");
        let r = ok(&call(&mut d, "seasonality", json!({})));
        let v: Value = serde_json::from_str(&r).unwrap();
        assert!(!v["returns_pct"].as_array().unwrap().is_empty(), "{r}");
        assert_eq!(v["per_column"].as_array().unwrap().len(), 13);
        let newest = v["years_available"][1].as_i64().unwrap();
        let r = ok(&call(
            &mut d,
            "seasonality",
            json!({"from_year": newest, "to_year": newest}),
        ));
        let v: Value = serde_json::from_str(&r).unwrap();
        assert_eq!(v["returns_pct"].as_array().unwrap().len(), 1, "{r}");
        assert_eq!(d.app.seasonal.range, Some((newest as i16, newest as i16)));

        // Make an index, read its board, change it, compare, delete.
        let r = ok(&call(
            &mut d,
            "custom_index",
            json!({"action": "create", "name": "Big Tech", "symbols": ["AAPL", "NVDA", "NOSUCHTHINGXYZ"], "weighting": "equal"}),
        ));
        assert!(r.contains("No match for: NOSUCHTHINGXYZ"), "{r}");
        let v: Value = serde_json::from_str(r.split_once('\n').unwrap().1).unwrap();
        assert_eq!(v["members"].as_array().unwrap().len(), 2, "{r}");
        assert!(v["value"].as_f64().is_some(), "{r}");
        assert!(d.app.indices.defs[0].ready());
        let r = ok(&call(
            &mut d,
            "custom_index",
            json!({"action": "get", "name": "BIGTECH"}),
        ));
        assert!(r.contains("\"P/E\""), "{r}");
        ok(&call(
            &mut d,
            "custom_index",
            json!({"action": "edit", "name": "Big Tech", "remove": ["NVDA"], "new_name": "Apple only"}),
        ));
        assert_eq!(d.app.indices.defs[0].name, "Apple only");
        assert_eq!(d.app.indices.defs[0].members.len(), 1);
        ok(&call(
            &mut d,
            "custom_index",
            json!({"action": "rebalance", "name": "Apple only"}),
        ));
        let r = call(&mut d, "custom_index", json!({"action": "open", "name": "compare"}));
        assert!(r["isError"].as_bool().unwrap_or(false), "one index can't be compared");
        ok(&call(
            &mut d,
            "custom_index",
            json!({"action": "open", "name": "Apple only", "view": "map"}),
        ));
        assert_eq!(d.app.layout.tab, Some(crate::shell::Tab::Indices));
        assert!(ok(&call(&mut d, "custom_index", json!({"action": "compare"}))).contains("Apple only"));
        ok(&call(
            &mut d,
            "custom_index",
            json!({"action": "delete", "name": "Apple only"}),
        ));
        assert!(d.app.indices.defs.is_empty());
    }
}

//! The agent's tools for the newer parts of the app: workspaces, the
//! drawings' text placement, Fib levels and position sizing, editing a
//! drawing, the drawing defaults (Settings → Drawings) and templates, the
//! multi-chart's maximize and sizes, alert lines, and moving or clearing
//! alerts.

use super::control::{flag, norm, num, snake, st};
use super::tools::{error, pretty, text};
use crate::app::App;
use crate::chart::draw::{Along, DrawStyle, Drawing, FibLevel, Side, Sizing, TextAt, Tool};
use crate::dialogs::TemplateKind;
use opentern_series::Rgba;
use opentern_series::study::LineStyle;
use serde_json::{Value, json};

fn hex(c: Rgba) -> String {
    format!("#{:06X}", c.0 >> 8)
}

fn colour(v: &Value) -> Option<Rgba> {
    v.as_str().map(crate::scripting::parse_color)
}

/// Sets the parts of a drawing's look that `d` names (the same names `add`
/// takes). Colours: `color` is the line (a position's target, a volume
/// profile's up volume); `fill` the second colour (a box's fill, a
/// position's stop, a profile's down volume).
pub fn style_from(tool: Tool, d: &Value, style: &mut DrawStyle) -> Result<(), String> {
    if let Some(c) = d.get("color").and_then(colour) {
        style.color = c;
        // A box's fill follows its colour, see-through, unless one is given.
        if tool.two_colors()
            && !matches!(tool, Tool::LongPosition | Tool::ShortPosition)
            && !matches!(tool, Tool::FixedRangeVolumeProfile | Tool::AnchoredVolumeProfile)
        {
            style.fill = c.transp(80);
        }
    }
    for k in ["fill", "stop_color", "down_color"] {
        if let Some(c) = d.get(k).and_then(colour) {
            style.fill = c;
        }
    }
    for k in ["target_color", "up_color"] {
        if let Some(c) = d.get(k).and_then(colour) {
            style.color = c;
        }
    }
    if let Some(w) = num(d, "width") {
        style.width = w.clamp(1.0, 4.0) as u8;
    }
    if let Some(f) = num(d, "font_size") {
        style.font = f.clamp(8.0, 120.0) as u8;
    }
    match norm(st(d, "line_style")).as_str() {
        "dashed" => style.line = LineStyle::Dashed,
        "dotted" => style.line = LineStyle::Dotted,
        "solid" => style.line = LineStyle::Solid,
        "" => {}
        other => return Err(format!("line_style is solid, dashed or dotted, not \"{other}\".")),
    }
    if let Some(v) = flag(d, "extend_left") {
        style.extend_left = v;
    }
    if let Some(v) = flag(d, "extend_right") {
        style.extend_right = v;
    }
    if let Some(v) = flag(d, "bold") {
        style.bold = v;
    }
    if let Some(v) = flag(d, "price_labels") {
        style.labels = v;
    }
    if let Some(a) = num(d, "angle") {
        if tool != Tool::Text {
            return Err("angle turns the text tool only (other tools' text follows their line).".into());
        }
        style.angle = a as f32;
    }
    // Where the text of a line or a box sits.
    if let Some(p) = d.get("text_position") {
        if !tool.carries_text() {
            return Err(format!("{} has no text position.", tool.name()));
        }
        let mut at = style.text_at(tool);
        let pick = |k: &str| p.get(k).and_then(Value::as_str).map(norm).unwrap_or_default();
        match pick("along").as_str() {
            "start" | "left" => at.along = Along::Start,
            "middle" | "center" | "centre" => at.along = Along::Middle,
            "end" | "right" => at.along = Along::End,
            "" => {}
            o => return Err(format!("text_position.along is start, middle or end, not \"{o}\".")),
        }
        match pick("side").as_str() {
            "above" | "top" => at.side = Side::Above,
            "on" | "inside" | "online" => at.side = Side::On,
            "below" | "bottom" => at.side = Side::Below,
            "" => {}
            o => {
                return Err(format!(
                    "text_position.side is above, on (inside for a rectangle) or below, not \"{o}\"."
                ));
            }
        }
        style.text_at = (at != TextAt::usual(tool)).then_some(at);
    }
    // Fib levels: numbers, or {value, color?, on?}; an empty list removes
    // them all; "standard" goes back to TradingView's.
    if let Some(l) = d.get("levels") {
        if !matches!(tool, Tool::FibRetracement | Tool::FibExtension) {
            return Err("levels are for fib_retracement and fib_extension.".into());
        }
        if l.as_str().is_some_and(|s| norm(s) == "standard") {
            style.levels = None;
        } else {
            let standard = FibLevel::standard(tool);
            let items = l.as_array().ok_or("levels is a list (or \"standard\").")?;
            let mut out = Vec::new();
            for v in items {
                let value = v
                    .as_f64()
                    .or_else(|| v.get("value").and_then(Value::as_f64))
                    .ok_or("Each level is a number or {value, color?, on?}.")?;
                let color = v
                    .get("color")
                    .and_then(colour)
                    .or_else(|| {
                        standard
                            .iter()
                            .find(|s| (s.value - value).abs() < 1e-9)
                            .map(|s| s.color)
                    })
                    .unwrap_or(style.color);
                let on = v.get("on").and_then(Value::as_bool).unwrap_or(true);
                out.push(FibLevel { value, color, on });
            }
            out.sort_by(|a, b| a.value.total_cmp(&b.value));
            style.levels = Some(out);
        }
    }
    // A position's size: the account, the risk and the lot.
    if let Some(z) = d.get("sizing") {
        if !matches!(tool, Tool::LongPosition | Tool::ShortPosition) {
            return Err("sizing is for long_position and short_position.".into());
        }
        let mut s = style.sizing.unwrap_or_default();
        if let Some(v) = num(z, "account") {
            s.account = v.max(0.0);
        }
        if let Some(v) = num(z, "risk") {
            s.risk = v.max(0.0);
        }
        match norm(st(z, "risk_unit")).as_str() {
            "percent" | "pct" | "" => {
                if z.get("risk_unit").is_some() {
                    s.risk_pct = true;
                }
            }
            "amount" | "cash" | "currency" => s.risk_pct = false,
            o => return Err(format!("sizing.risk_unit is percent or amount, not \"{o}\".")),
        }
        if s.risk_pct {
            s.risk = s.risk.min(100.0);
        }
        if let Some(v) = num(z, "lot") {
            s.lot = v.max(0.0);
        }
        style.sizing = (s != Sizing::default()).then_some(s);
    }
    Ok(())
}

/// A drawing's look, as `add` and `edit` take it, plus a position's size.
pub fn style_json(d: &Drawing) -> Value {
    let s = &d.style;
    let mut out = json!({
        "color": hex(s.color),
        "width": s.width,
        "line_style": snake(&format!("{:?}", s.line)),
    });
    if d.tool.two_colors() {
        out["fill"] = json!(hex(s.fill));
    }
    if matches!(d.tool, Tool::Text) || !d.text.is_empty() {
        out["font_size"] = json!(s.font);
        out["bold"] = json!(s.bold);
    }
    if d.tool == Tool::Text && s.angle != 0.0 {
        out["angle"] = json!(s.angle);
    }
    if d.tool.carries_text() && !d.text.is_empty() {
        let at = s.text_at(d.tool);
        out["text_position"] = json!({
            "along": snake(&format!("{:?}", at.along)),
            "side": snake(&format!("{:?}", at.side)),
        });
    }
    if matches!(d.tool, Tool::FibRetracement | Tool::FibExtension) {
        let levels = s.levels.clone().unwrap_or_else(|| FibLevel::standard(d.tool));
        out["levels"] = json!(
            levels
                .iter()
                .map(|l| json!({"value": l.value, "color": hex(l.color), "on": l.on}))
                .collect::<Vec<_>>()
        );
    }
    if matches!(d.tool, Tool::LongPosition | Tool::ShortPosition) && d.pts.len() >= 3 {
        let z = s.sizing.unwrap_or_default();
        let (e, t, stop) = (d.pts[0].p, d.pts[1].p, d.pts[2].p);
        let mut size = json!({
            "account": z.account,
            "risk": z.risk,
            "risk_unit": if z.risk_pct { "percent" } else { "amount" },
            "lot": z.lot,
            "risk_reward": if (e - stop).abs() > 0.0 { ((t - e) / (e - stop)).abs() } else { 0.0 },
        });
        if let Some((q, risk)) = z.qty(e, stop) {
            size["qty"] = json!(q);
            size["risk_amount"] = json!(risk);
            size["reward_amount"] = json!(q * (t - e).abs());
        }
        out["sizing"] = size;
    }
    out
}

fn tool_named(name: &str) -> Result<Tool, String> {
    let want = norm(name);
    super::control::drawing_tools()
        .find(|t| norm(&format!("{t:?}")) == want)
        .ok_or_else(|| format!("No drawing tool \"{name}\"."))
}

impl App {
    /// `drawings` actions beyond add, list, remove and clear: edit,
    /// defaults, templates and template.
    pub(crate) fn tool_drawings_more(&mut self, a: &Value) -> Option<Value> {
        Some(match st(a, "action") {
            "edit" => {
                let ids: Vec<u64> = match (a.get("ids").and_then(Value::as_array), num(a, "id")) {
                    (Some(v), _) => v.iter().filter_map(Value::as_u64).collect(),
                    (None, Some(id)) => vec![id as u64],
                    _ => return Some(error("edit needs \"id\" (or \"ids\"), from list.")),
                };
                let points = match a.get("points").and_then(Value::as_array) {
                    Some(p) => match p.iter().map(|x| self.anchor_of(x)).collect::<Result<Vec<_>, _>>() {
                        Ok(v) => Some(v),
                        Err(e) => return Some(error(e)),
                    },
                    None => None,
                };
                let mut list = self.chart.drawings.clone();
                let mut done = 0;
                for d in list.iter_mut().filter(|d| ids.contains(&d.id)) {
                    if let Err(e) = style_from(d.tool, a, &mut d.style) {
                        return Some(error(e));
                    }
                    if let Some(t) = a.get("text").and_then(Value::as_str) {
                        d.text = t.to_string();
                    }
                    if let Some(v) = flag(a, "locked") {
                        d.locked = v;
                    }
                    if let Some(v) = flag(a, "hidden") {
                        d.hidden = v;
                    }
                    if let Some(p) = &points {
                        if p.len() != d.pts.len() {
                            return Some(error(format!(
                                "{} has {} points; give all of them.",
                                d.tool.name(),
                                d.pts.len()
                            )));
                        }
                        d.pts = p.clone();
                    }
                    done += 1;
                }
                if done == 0 {
                    return Some(error("No drawing with that id on this chart (see list)."));
                }
                self.chart.push_undo();
                self.chart.drawings = list;
                self.chart.drawings_changed += 1;
                text(format!("Changed {done}. The user can undo with Ctrl+Z."))
            }
            // Settings → Drawings: the look new drawings of a kind get, and
            // what happens after a drawing.
            "defaults" => {
                match norm(st(a, "after_drawing")).as_str() {
                    "cursor" | "backtothecursor" => self.set_after_drawing(true),
                    "keep" | "keepthetool" => self.set_after_drawing(false),
                    "" => {}
                    o => return Some(error(format!("after_drawing is keep or cursor, not \"{o}\"."))),
                }
                let name = st(a, "tool");
                if !name.is_empty() {
                    let tool = match tool_named(name) {
                        Ok(t) => t,
                        Err(e) => return Some(error(e)),
                    };
                    if flag(a, "reset") == Some(true) {
                        self.reset_drawing_default(tool);
                    } else {
                        let mut style = self
                            .draw_styles
                            .get(&tool)
                            .cloned()
                            .unwrap_or_else(|| DrawStyle::for_tool(tool));
                        if let Err(e) = style_from(tool, a, &mut style) {
                            return Some(error(e));
                        }
                        self.save_drawing_default(tool, &style);
                    }
                }
                let own: Vec<Value> = self
                    .draw_styles
                    .iter()
                    .map(|(t, s)| {
                        let d = Drawing {
                            id: 0,
                            tool: *t,
                            pts: Vec::new(),
                            style: s.clone(),
                            text: String::new(),
                            locked: false,
                            hidden: false,
                        };
                        json!({"tool": snake(&format!("{t:?}")), "style": style_json(&d)})
                    })
                    .collect();
                pretty(&json!({
                    "after_drawing": if self.back_to_cursor { "cursor" } else { "keep" },
                    "tools_with_their_own_default": own,
                }))
            }
            // Named drawing templates (the bar's palette button).
            "templates" => {
                let tools: Vec<Tool> = match st(a, "tool") {
                    "" => super::control::drawing_tools().collect(),
                    n => match tool_named(n) {
                        Ok(t) => vec![t],
                        Err(e) => return Some(error(e)),
                    },
                };
                let list: Vec<Value> = tools
                    .into_iter()
                    .filter_map(|t| {
                        let names: Vec<String> = self
                            .store
                            .templates(&TemplateKind::Drawing(t).key())
                            .iter()
                            .map(|x| x.name.clone())
                            .collect();
                        (!names.is_empty()).then(|| json!({"tool": snake(&format!("{t:?}")), "templates": names}))
                    })
                    .collect();
                pretty(&json!(list))
            }
            "template" => {
                let Some(id) = num(a, "id").map(|v| v as u64) else {
                    return Some(error("template needs \"id\" (the drawing) and \"name\"."));
                };
                let Some(d) = self.chart.drawings.iter().find(|d| d.id == id).cloned() else {
                    return Some(error("No drawing with that id on this chart."));
                };
                let name = st(a, "name");
                if name.is_empty() {
                    return Some(error("template needs \"name\"."));
                }
                match norm(st(a, "do")).as_str() {
                    "save" => {
                        let kind = TemplateKind::Drawing(d.tool);
                        match serde_json::to_string(&d.style) {
                            Ok(j) => {
                                let _ = self.store.save_template(&kind.key(), name, &j);
                                text(format!("Saved {}'s look as \"{name}\".", d.tool.name()))
                            }
                            Err(e) => error(e.to_string()),
                        }
                    }
                    "delete" => {
                        self.delete_template(TemplateKind::Drawing(d.tool), name);
                        text(format!("Deleted \"{name}\"."))
                    }
                    _ => match self.drawing_template(d.tool, name) {
                        Some(style) => {
                            self.chart.push_undo();
                            if let Some(x) = self.chart.drawings.iter_mut().find(|x| x.id == id) {
                                x.style = style;
                            }
                            self.chart.drawings_changed += 1;
                            text(format!("Applied \"{name}\"."))
                        }
                        None => error(format!("{} has no template \"{name}\" (see templates).", d.tool.name())),
                    },
                }
            }
            _ => return None,
        })
    }

    /// Settings → Drawings: after drawing, back to the cursor or not.
    pub(crate) fn set_after_drawing(&mut self, cursor: bool) {
        self.back_to_cursor = cursor;
        let _ = self
            .store
            .set_value(crate::settings::AFTER_DRAWING, if cursor { "cursor" } else { "keep" });
    }

    /// `chart_view` actions for the multi-chart and alert lines.
    pub(crate) fn tool_chart_view_more(&mut self, a: &Value) -> Option<Value> {
        Some(match st(a, "action") {
            "maximize" => {
                if self.chart_count() < 2 {
                    return Some(error("There's one chart; maximize is for a multi-chart layout."));
                }
                let want = flag(a, "on").unwrap_or(self.multi.maximized.is_none());
                if want != self.multi.maximized.is_some() {
                    self.toggle_maximized();
                }
                text(if want {
                    "The active chart fills the chart area (the others are kept)."
                } else {
                    "All charts are shown."
                })
            }
            "reset_sizes" => {
                self.multi.cells = None;
                self.multi.changed += 1;
                text("The charts are back to even sizes.")
            }
            "alert_lines" => {
                let s = &mut self.chart.style;
                if let Some(v) = flag(a, "on") {
                    s.alert_lines = v;
                }
                if let Some(c) = a.get("color").and_then(colour) {
                    s.alert_color = c;
                }
                if let Some(w) = num(a, "width") {
                    s.alert_width = w.clamp(1.0, 4.0) as u8;
                }
                match norm(st(a, "line_style")).as_str() {
                    "dashed" => s.alert_style = LineStyle::Dashed,
                    "dotted" => s.alert_style = LineStyle::Dotted,
                    "solid" => s.alert_style = LineStyle::Solid,
                    _ => {}
                }
                let out = json!({
                    "alert_lines": s.alert_lines,
                    "color": hex(s.alert_color),
                    "width": s.alert_width,
                    "line_style": snake(&format!("{:?}", s.alert_style)),
                });
                self.chart.touch();
                pretty(&out)
            }
            _ => return None,
        })
    }

    /// `alerts` actions: move one to a price, delete them all.
    pub(crate) fn tool_alerts_more(&mut self, a: &Value) -> Option<Value> {
        Some(match st(a, "action") {
            "move" => {
                let (Some(id), Some(price)) = (num(a, "id"), num(a, "price")) else {
                    return Some(error("move needs \"id\" and \"price\"."));
                };
                let id = id as i64;
                let price_alert = self.alerts.list.iter().any(|(x, _)| {
                    x.id == id && matches!(x.condition, opentern_engine::alerts::Condition::Price { .. })
                });
                if !price_alert {
                    return Some(error("No price alert with that id."));
                }
                self.move_alert(id, price);
                text(format!("Moved it to {price}."))
            }
            "delete_all" => {
                if flag(a, "confirm") != Some(true) {
                    return Some(error(
                        "This deletes every alert and can't be undone. Ask the user first, then call again with \"confirm\": true.",
                    ));
                }
                let ids: Vec<i64> = self.alerts.list.iter().map(|(x, _)| x.id).collect();
                let n = ids.len();
                for id in ids {
                    self.delete_alert(id);
                }
                text(format!("Deleted all {n} alerts."))
            }
            _ => return None,
        })
    }

    /// The `workspaces` tool: the numbered pills above the top bar.
    pub(crate) fn tool_workspaces(&mut self, a: &Value) -> Value {
        let n = self.ws.list.len();
        // 1-based numbers, as on the pills.
        let pick = |k: &str| {
            num(a, k)
                .map(|v| v.round() as usize)
                .filter(|v| (1..=n).contains(v))
                .map(|v| v - 1)
        };
        let out_of_range = || error(format!("There are {n} workspaces; use a number from 1 to {n}."));
        match st(a, "action") {
            "switch" => match pick("number") {
                Some(i) => {
                    self.switch_workspace(i);
                    text(format!(
                        "Workspace {} ({}) is shown; the tools act on its charts.",
                        i + 1,
                        self.ws_title(i)
                    ))
                }
                None => out_of_range(),
            },
            "new" => {
                self.new_workspace();
                // The agent picks the symbol itself (open_symbol).
                self.modal = None;
                text(format!(
                    "Workspace {} is new and shown, with a blank chart: use open_symbol to put a symbol on it.",
                    self.ws.current + 1
                ))
            }
            "close" => match pick("number").or(a.get("number").is_none().then_some(self.ws.current)) {
                Some(_) if n == 1 => error("The last workspace can't be closed."),
                Some(i) => {
                    let name = self.ws_title(i);
                    self.close_workspace(i);
                    text(format!("Closed {name} (the user can reopen it with Ctrl+Shift+T)."))
                }
                None => out_of_range(),
            },
            "reopen" => {
                let before = self.ws.list.len();
                self.reopen_workspace();
                if self.ws.list.len() > before {
                    text(format!("Reopened {}.", self.ws_title(self.ws.current)))
                } else {
                    error("No workspace was closed in this run.")
                }
            }
            "rename" => match pick("number").or(a.get("number").is_none().then_some(self.ws.current)) {
                Some(i) => {
                    self.rename_workspace(i, st(a, "name"));
                    text(format!("Workspace {} is \"{}\".", i + 1, self.ws_title(i)))
                }
                None => out_of_range(),
            },
            "duplicate" => match pick("number").or(a.get("number").is_none().then_some(self.ws.current)) {
                Some(i) => {
                    self.duplicate_workspace(i);
                    text(format!("Copied it to workspace {}, now shown.", self.ws.current + 1))
                }
                None => out_of_range(),
            },
            "move" => match (pick("number"), pick("to")) {
                (Some(i), Some(to)) => {
                    self.move_workspace(i, to);
                    text(format!("Moved it to place {}.", to + 1))
                }
                _ => out_of_range(),
            },
            _ => {
                let list: Vec<Value> = (0..n)
                    .map(|i| {
                        let charts = if i == self.ws.current {
                            self.chart_count()
                        } else {
                            self.ws.list[i].saved.as_ref().map_or(1, |s| s.charts.len())
                        };
                        let main = if i == self.ws.current {
                            let c = if self.active == 0 {
                                Some(&self.chart)
                            } else {
                                self.slots.first().and_then(|s| s.as_ref()).map(|s| &s.chart)
                            };
                            c.map(|c| (c.symbol.id.clone(), c.tf.label()))
                        } else {
                            self.ws.list[i]
                                .saved
                                .as_ref()
                                .and_then(|s| s.charts.first())
                                .map(|c| (c.symbol.id.clone(), c.tf.label()))
                        };
                        json!({
                            "number": i + 1,
                            "name": self.ws_title(i),
                            "renamed": self.ws.list[i].name.is_some(),
                            "symbol": main.as_ref().map(|m| m.0.clone()),
                            "interval": main.as_ref().map(|m| m.1.clone()),
                            "charts": charts,
                            "shown": i == self.ws.current,
                        })
                    })
                    .collect();
                pretty(&json!({"workspaces": list}))
            }
        }
    }
}

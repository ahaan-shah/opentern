//! How hard an agent thinks. Each model lists the levels it takes (Claude
//! Code in its `initialize` answer, Codex in `model/list`); a picked level
//! goes to Claude Code as `--effort` and to Codex with each turn. Nothing
//! is passed until one is picked, so a chat runs as it would in a terminal.

use super::{Agent, accounts, style};
use crate::i18n::{nt, tr};
use crate::motion;
use crate::theme::{self, Theme};
use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A model as its agent lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub levels: Levels,
}

/// The efforts a model takes, lowest first, and the one it runs at when
/// none is picked (when known).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Levels {
    pub all: Vec<String>,
    #[serde(default)]
    pub default: Option<String>,
}

impl Levels {
    pub fn has(&self, level: &str) -> bool {
        self.all.iter().any(|l| l == level)
    }

    /// `level` if the model takes it, else its default, else its highest.
    pub fn fit(&self, level: &str) -> String {
        if level.is_empty() || self.all.is_empty() {
            return String::new();
        }
        if self.has(level) {
            return level.to_string();
        }
        self.default
            .clone()
            .filter(|d| self.has(d))
            .or_else(|| self.all.last().cloned())
            .unwrap_or_default()
    }
}

/// What every current Claude model takes, until Claude Code lists its own.
pub fn claude_levels() -> Levels {
    Levels {
        all: ["low", "medium", "high", "xhigh", "max"].map(String::from).to_vec(),
        default: None,
    }
}

/// An entry of Claude Code's model list.
pub fn of_claude(m: &Value) -> Levels {
    let all: Vec<String> = match m["supportsEffort"].as_bool() {
        Some(false) => Vec::new(),
        _ => strings(&m["supportedEffortLevels"]),
    };
    Levels { all, default: None }
}

/// An entry of Codex's model list.
pub fn of_codex(m: &Value) -> Levels {
    Levels {
        all: m["supportedReasoningEfforts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| e["reasoningEffort"].as_str())
            .map(String::from)
            .collect(),
        default: m["defaultReasoningEffort"].as_str().map(String::from),
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect()
}

/// A level's name. One the app doesn't know yet shows as its agent writes it.
pub fn name(level: &str) -> String {
    let known = match level {
        "none" => nt!("None"),
        "minimal" => nt!("Minimal"),
        "low" => nt!("Low"),
        "medium" => nt!("Medium"),
        "high" => nt!("High"),
        "xhigh" => nt!("Extra high"),
        "max" => nt!("Max"),
        "ultra" => nt!("Ultra"),
        _ => {
            let mut c = level.chars();
            return c
                .next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default();
        }
    };
    tr(known).to_string()
}

/// The effort an agent's own settings name.
pub fn configured(a: Agent) -> Option<String> {
    let dir = accounts::folder(a, &accounts::current(a));
    let level = match a {
        Agent::Claude => std::env::var("CLAUDE_CODE_EFFORT_LEVEL").ok().or_else(|| {
            let text = std::fs::read_to_string(dir?.join("settings.json")).ok()?;
            let v: Value = serde_json::from_str(&text).ok()?;
            v["effortLevel"].as_str().map(String::from)
        }),
        Agent::Codex => {
            let text = std::fs::read_to_string(dir?.join("config.toml")).ok()?;
            super::toml_value(&text, "model_reasoning_effort")
        }
    }?;
    let level = level.trim().to_lowercase();
    (!level.is_empty() && level != "auto" && level != "unset").then_some(level)
}

/// The slider's motion: the thumb is at `base` plus an `off` that dies away,
/// so grabbing, letting go and a change from elsewhere all glide from where
/// it is, and under the pointer it keeps exactly up.
#[derive(Clone, Copy)]
struct Glide {
    base: f32,
    off: f32,
    held: bool,
    /// The level let go of and what was on show then: it stands until the
    /// pick comes back as the level on show.
    kept: Option<(usize, Option<usize>, f64)>,
}

/// How fast `off` dies away, per second.
const SETTLE: f32 = 16.0;

/// The effort slider: its level's name, then a bar filled up to a thumb. The
/// thumb follows the pointer and glides to the nearest level when let go.
/// `now` is the level on show (none: not known yet). Returns the one picked,
/// once the pointer lets go.
pub fn slider(ui: &mut Ui, t: &Theme, levels: &[String], now: Option<&str>) -> Option<String> {
    let n = levels.len();
    if n < 2 {
        return None;
    }
    let last = (n - 1) as f32;
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click_and_drag());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    #[cfg(test)]
    ui.data_mut(|d| d.insert_temp(egui::Id::new("effort-slider"), r));
    let bar = Rect::from_center_size(r.center(), vec2(r.width() - 24.0, 12.0));
    // The thumb's middle stays a little inside the bar's ends.
    let (x0, x1) = (bar.left() + 5.0, bar.right() - 5.0);
    let at = now.and_then(|l| levels.iter().position(|x| x == l));
    let pointer = resp
        .interact_pointer_pos()
        .map(|p| ((p.x - x0) / (x1 - x0) * last).clamp(0.0, last));
    let released = resp.drag_stopped() || resp.clicked();
    let held = !released && pointer.is_some() && (resp.is_pointer_button_down_on() || resp.dragged());
    let time = ui.input(|i| i.time);
    let id = resp.id.with("glide");
    let before: Option<Glide> = ui.data(|d| d.get_temp(id));
    let mut kept = before.and_then(|g| g.kept);
    let mut picked = None;
    if released && let Some(v) = pointer {
        let k = v.round() as usize;
        kept = Some((k, at, time));
        if at != Some(k) {
            picked = Some(levels[k].clone());
        }
    }
    // Taken up, changed elsewhere, or refused: what's on show stands.
    if let Some((k, was, since)) = kept
        && !released
        && (held || at == Some(k) || (at.is_some() && (at != was || time - since > 0.5)))
    {
        kept = None;
    }
    let level = kept.map(|(k, ..)| k).or(at).map(|k| k.min(n - 1));
    let target = if held { pointer } else { level.map(|k| k as f32) };
    let pos = target.map(|base| {
        let mut g = before.unwrap_or(Glide {
            base,
            off: 0.0,
            held,
            kept,
        });
        if !(held && g.held) && g.base != base {
            g.off += g.base - base;
        }
        let dt = ui.input(|i| i.stable_dt).min(0.05);
        g.off *= (-dt * SETTLE).exp();
        if g.off.abs() < 0.002 {
            g.off = 0.0;
        } else {
            ui.ctx().request_repaint();
        }
        (g.base, g.held, g.kept) = (base, held, kept);
        ui.data_mut(|d| d.insert_temp(id, g));
        let v = (g.base + g.off).clamp(0.0, last);
        #[cfg(test)]
        ui.data_mut(|d| d.insert_temp(egui::Id::new("effort-pos"), v));
        v
    });
    if target.is_none() {
        ui.data_mut(|d| d.remove::<Glide>(id));
    }
    let p = ui.painter();
    p.text(
        pos2(head.left() + 12.0, head.center().y),
        Align2::LEFT_CENTER,
        tr("Effort"),
        theme::medium(12.5),
        t.text2,
    );
    // The level it would be, as it's dragged.
    if let Some(v) = target {
        p.text(
            pos2(head.right() - 12.0, head.center().y),
            Align2::RIGHT_CENTER,
            name(&levels[v.round() as usize]),
            theme::semibold(12.5),
            t.accent,
        );
    }
    p.rect_filled(bar, 6, style::rail(t));
    if let Some(v) = pos {
        let x = x0 + (x1 - x0) * v / last;
        let fill = Rect::from_min_max(bar.min, pos2((x + 1.0).max(bar.left() + 12.0), bar.bottom()));
        p.rect_filled(fill, 6, t.accent);
        let grow = motion::hover_if(&resp, held || resp.hovered());
        let thumb = Rect::from_center_size(pos2(x, bar.center().y), vec2(5.0, 22.0 + 3.0 * grow));
        p.rect_filled(thumb.expand2(vec2(3.0, 0.0)), 0, ui.visuals().window_fill);
        p.rect_filled(thumb, 2.5, t.text);
    }
    picked
}

/// A hairline across a menu.
pub fn rule(ui: &mut Ui, t: &Theme) {
    ui.add_space(5.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        r.x_range().shrink(8.0),
        r.center().y,
        Stroke::new(1.0, t.border.gamma_multiply(0.8)),
    );
    ui.add_space(5.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn levels_come_from_the_model_lists() {
        let c = of_claude(&json!({"supportsEffort": true, "supportedEffortLevels": ["low", "high"]}));
        assert_eq!(c.all, ["low", "high"]);
        assert!(
            of_claude(&json!({"supportsEffort": false, "supportedEffortLevels": ["low"]}))
                .all
                .is_empty()
        );
        let x = of_codex(&json!({"defaultReasoningEffort": "medium", "supportedReasoningEfforts":
            [{"reasoningEffort": "minimal", "description": ""}, {"reasoningEffort": "medium", "description": ""}]}));
        assert_eq!(x.all, ["minimal", "medium"]);
        assert_eq!(x.default.as_deref(), Some("medium"));
    }

    /// Let go of, the thumb glides on to its level: no jump back toward the
    /// one before while the pick is on its way.
    #[test]
    fn the_thumb_never_jumps_back() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx, "");
        let t = theme::ThemeState::new(&ctx, "").current;
        let levels: Vec<String> = ["low", "medium", "high", "xhigh", "max"].map(String::from).to_vec();
        let mut now = "low".to_string();
        let mut frame = 0;
        let mut step = |events: Vec<egui::Event>, taken: bool, now: &mut String| -> f32 {
            frame += 1;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(230.0, 100.0))),
                time: Some(f64::from(frame) / 60.0),
                events,
                ..Default::default()
            };
            let mut picked = None;
            let mut out = ctx.run_ui(raw, |ui| {
                picked = slider(ui, &t, &levels, Some(now.as_str())).or(picked.take())
            });
            out.textures_delta.clear();
            if let Some(p) = picked.filter(|_| taken) {
                *now = p;
            }
            ctx.data(|d| d.get_temp(egui::Id::new("effort-pos"))).unwrap()
        };
        step(vec![], true, &mut now);
        step(vec![], true, &mut now);
        let r: Rect = ctx.data(|d| d.get_temp(egui::Id::new("effort-slider"))).unwrap();
        let at = |x: f32| pos2(r.left() + r.width() * x, r.center().y);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        step(vec![egui::Event::PointerMoved(at(0.06))], true, &mut now);
        step(vec![button(at(0.06), true)], true, &mut now);
        let mut prev = 0.0;
        for k in 1..=20 {
            let v = step(
                vec![egui::Event::PointerMoved(at(0.06 + 0.032 * k as f32))],
                true,
                &mut now,
            );
            assert!(v >= prev - 0.001, "{prev} → {v} while dragged right");
            prev = v;
        }
        let dropped = prev;
        let mut v = step(vec![button(at(0.70), false)], true, &mut now);
        assert_eq!(now, "xhigh");
        for _ in 0..60 {
            assert!(v >= prev - 0.001, "{prev} → {v} after letting go at {dropped}");
            prev = v;
            v = step(vec![], true, &mut now);
        }
        assert_eq!(v, 3.0);
    }

    #[test]
    fn a_level_the_model_lacks_becomes_its_default() {
        let l = Levels {
            all: vec!["low".into(), "medium".into(), "high".into()],
            default: Some("medium".into()),
        };
        assert_eq!(l.fit("high"), "high");
        assert_eq!(l.fit("max"), "medium");
        assert_eq!(l.fit(""), "", "nothing picked stays that way");
        assert_eq!(Levels { default: None, ..l }.fit("max"), "high");
        assert_eq!(Levels::default().fit("high"), "", "a model without efforts");
    }
}

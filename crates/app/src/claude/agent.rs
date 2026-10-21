//! The coding agents the panel can talk to: Claude Code and Codex, each the
//! CLI the user installed and signed in to. Names, logos, colours and where
//! to find each CLI live here; the protocols are in `process.rs` (Claude
//! Code) and `codex.rs`.
//!
//! Codex's mark is Phosphor's `OPEN_AI_LOGO` glyph (MIT, bundled with the
//! icon font). OpenAI and Codex are trademarks of OpenAI; the mark is used
//! only to name the integration with the user's own Codex CLI.

use super::mark;
use crate::icons::ph;
use crate::theme::Theme;
use egui::{Align2, Color32, Rect, Response, Sense, Ui, vec2};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Agent {
    #[default]
    Claude,
    Codex,
}

impl Agent {
    pub const ALL: [Agent; 2] = [Agent::Claude, Agent::Codex];

    pub fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude",
            Agent::Codex => "Codex",
        }
    }

    /// The CLI's name, as people know it.
    pub fn product(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
        }
    }

    /// What to do when the CLI isn't found.
    pub fn install_hint(self) -> &'static str {
        match self {
            Agent::Claude => "Install Claude Code, then run claude once in a terminal to sign in.",
            Agent::Codex => "Install the Codex CLI, then run codex once in a terminal to sign in.",
        }
    }

    /// The env var that points at the CLI (also lets headless tests run it).
    pub fn env(self) -> &'static str {
        match self {
            Agent::Claude => "OPENTERN_CLAUDE",
            Agent::Codex => "OPENTERN_CODEX",
        }
    }

    /// The agent's colour: Claude's terracotta; Codex is monochrome.
    pub fn accent(self, t: &Theme) -> Color32 {
        match self {
            Agent::Claude => mark::CLAUDE,
            Agent::Codex => t.text,
        }
    }

    /// Text and icons drawn on the accent.
    pub fn on_accent(self, t: &Theme) -> Color32 {
        match self {
            Agent::Claude => Color32::WHITE,
            Agent::Codex => t.bg,
        }
    }

    /// The logo's own colour (the Codex mark follows the text colour).
    pub fn tint(self, t: &Theme) -> Color32 {
        self.accent(t)
    }

    /// Where the CLI is: its env var, then the PATH, then the places the
    /// installers put it (a desktop launch may not have them on the PATH).
    pub fn find(self) -> Option<PathBuf> {
        if let Some(p) = std::env::var_os(self.env()) {
            return Some(PathBuf::from(p)).filter(|p| p.is_file());
        }
        let names: &[&str] = match (self, cfg!(windows)) {
            (Agent::Claude, true) => &["claude.exe", "claude.cmd"],
            (Agent::Claude, false) => &["claude"],
            (Agent::Codex, true) => &["codex.exe", "codex.cmd"],
            (Agent::Codex, false) => &["codex"],
        };
        let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            let home = PathBuf::from(home);
            dirs.push(home.join(".local").join("bin"));
            dirs.push(home.join(".claude").join("local"));
            dirs.push(home.join(".npm-global").join("bin"));
            dirs.push(home.join(".cargo").join("bin"));
        }
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.iter()
            .flat_map(|d| names.iter().map(move |n| d.join(n)))
            .find(|p| p.is_file())
    }
}

/// Paints an agent's logo in `rect`, tinted, turned by `angle` radians.
pub fn paint(ui: &Ui, rect: Rect, agent: Agent, color: Color32, angle: f32) {
    match agent {
        Agent::Claude => mark::paint(ui, rect, color, angle),
        Agent::Codex => {
            let size = rect.height() * 1.02;
            let galley =
                ui.painter()
                    .layout_no_wrap(ph::OPEN_AI_LOGO.to_string(), egui::FontId::proportional(size), color);
            // Turn about the glyph's centre: its top-left goes round it.
            let half = galley.size() / 2.0;
            let c = rect.center() + vec2(0.0, size * 0.02);
            let pos = c - egui::emath::Rot2::from_angle(angle) * half;
            ui.painter()
                .add(egui::epaint::TextShape::new(pos, galley, color).with_angle(angle));
        }
    }
}

/// Paints an agent's logo upright on any painter (an overlay layer).
pub fn paint_flat(p: &egui::Painter, rect: Rect, agent: Agent, color: Color32) {
    match agent {
        Agent::Claude => mark::paint_flat(p, rect, color),
        Agent::Codex => {
            let size = rect.height() * 1.02;
            p.text(
                rect.center() + vec2(0.0, size * 0.02),
                Align2::CENTER_CENTER,
                ph::OPEN_AI_LOGO,
                egui::FontId::proportional(size),
                color,
            );
        }
    }
}

/// The logo as a widget of `size` points.
pub fn show(ui: &mut Ui, size: f32, agent: Agent, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    paint(ui, rect, agent, color, 0.0);
    resp
}

/// The spinner while the agent works: its logo, turning.
pub fn spinner(ui: &mut Ui, size: f32, agent: Agent, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let t = ui.input(|i| i.time) as f32;
    paint(ui, rect, agent, color, t * 2.2);
    ui.ctx().request_repaint();
    resp
}

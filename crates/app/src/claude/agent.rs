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

    /// The CLI's own install page, linked when it isn't found.
    pub fn install_url(self) -> &'static str {
        match self {
            Agent::Claude => "https://code.claude.com/docs/en/setup",
            Agent::Codex => "https://developers.openai.com/codex/cli",
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
        if let Some(app) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(app).join("npm"));
        }
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        self.find_in(&dirs, cfg!(windows))
    }

    fn find_in(self, dirs: &[PathBuf], windows: bool) -> Option<PathBuf> {
        let name = match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        };
        if !windows {
            return dirs.iter().map(|d| d.join(name)).find(|p| p.is_file());
        }
        // A native install wins from any folder, even with an npm shim
        // earlier on the PATH.
        let exe = format!("{name}.exe");
        if let Some(p) = dirs.iter().map(|d| d.join(&exe)).find(|p| p.is_file()) {
            return Some(p);
        }
        // npm's `claude.cmd`/`codex.cmd` would run us through cmd.exe, which
        // can't carry our arguments (JSON, newlines). Both packages carry the
        // real binary, so run that.
        let cmd = format!("{name}.cmd");
        dirs.iter()
            .filter(|d| d.join(&cmd).is_file())
            .find_map(|d| self.npm_binary(d))
    }

    /// The native binary inside an npm global install whose shims are in
    /// `dir`. Claude Code's postinstall copies it over a small placeholder
    /// (`bin/claude.exe`); Codex keeps it in a platform package, nested or
    /// hoisted (older versions: in its own `vendor`).
    fn npm_binary(self, dir: &std::path::Path) -> Option<PathBuf> {
        let at = |parts: &[&str]| parts.iter().fold(dir.join("node_modules"), |p, s| p.join(s));
        let (arch, triple) = if cfg!(target_arch = "aarch64") {
            ("arm64", "aarch64-pc-windows-msvc")
        } else {
            ("x64", "x86_64-pc-windows-msvc")
        };
        let pkg = format!("codex-win32-{arch}");
        let tail = ["vendor", triple, "bin", "codex.exe"];
        let found = match self {
            Agent::Claude => vec![at(&["@anthropic-ai", "claude-code", "bin", "claude.exe"])],
            Agent::Codex => [
                &["@openai", "codex", "node_modules", "@openai", &pkg][..],
                &["@openai", &pkg][..],
                &["@openai", "codex"][..],
            ]
            .iter()
            .map(|root| at(&[*root, &tail[..]].concat()))
            .collect(),
        };
        found
            .into_iter()
            .find(|p| p.metadata().is_ok_and(|m| m.is_file() && m.len() > 1 << 20))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: PathBuf, len: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::File::create(&path).unwrap().set_len(len).unwrap();
    }

    #[test]
    fn windows_runs_the_binary_inside_an_npm_install() {
        let root = std::env::temp_dir().join(format!("opentern-agent-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (npm, local) = (root.join("npm"), root.join("local"));
        let big = 2 << 20;
        let (arch, triple) = if cfg!(target_arch = "aarch64") {
            ("arm64", "aarch64-pc-windows-msvc")
        } else {
            ("x64", "x86_64-pc-windows-msvc")
        };
        let m = npm.join("node_modules");
        file(npm.join("claude.cmd"), 10);
        file(npm.join("codex.cmd"), 10);
        // Claude's placeholder before its postinstall has run: not usable.
        let claude = m
            .join("@anthropic-ai")
            .join("claude-code")
            .join("bin")
            .join("claude.exe");
        file(claude.clone(), 500);
        let dirs = [npm.clone(), local.clone()];
        assert_eq!(Agent::Claude.find_in(&dirs, true), None);
        file(claude.clone(), big);
        assert_eq!(Agent::Claude.find_in(&dirs, true), Some(claude));
        // Codex's platform package, nested under the wrapper.
        let codex = m
            .join("@openai")
            .join("codex")
            .join("node_modules")
            .join("@openai")
            .join(format!("codex-win32-{arch}"))
            .join("vendor")
            .join(triple)
            .join("bin")
            .join("codex.exe");
        file(codex.clone(), big);
        assert_eq!(Agent::Codex.find_in(&dirs, true), Some(codex));
        // A native install wins even when it comes later.
        file(local.join("claude.exe"), big);
        assert_eq!(Agent::Claude.find_in(&dirs, true), Some(local.join("claude.exe")));
        let _ = std::fs::remove_dir_all(&root);
    }
}

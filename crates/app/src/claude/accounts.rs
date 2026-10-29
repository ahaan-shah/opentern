//! Which account an agent's CLI runs as. Claude Code keeps an account in a
//! config folder named by `CLAUDE_CONFIG_DIR` (unset: `~/.claude`, with its
//! settings in `~/.claude.json`), Codex in `CODEX_HOME` (unset: `~/.codex`).
//! People with a work and a personal account keep a second folder beside
//! the first (`~/.claude-work`, `~/.codex-personal`); the panel lists them
//! and passes the chosen one to every CLI it starts.
//!
//! An account is its folder as a string; "" is the CLI's own default (the
//! variable removed, never set to the default folder: Claude Code would
//! then look for `.claude.json` inside it).

use super::Agent;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::RwLock;

/// One account on this machine.
#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    /// The config folder, or "" for the default one.
    pub dir: String,
    /// The folder's name without the CLI's prefix ("work" for
    /// `.claude-work`), or "Default".
    pub label: String,
}

/// The variable that names the config folder.
pub fn var(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "CLAUDE_CONFIG_DIR",
        Agent::Codex => "CODEX_HOME",
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn prefix(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => ".claude",
        Agent::Codex => ".codex",
    }
}

/// `dir` as an account: the default folder becomes "".
fn normalise(agent: Agent, dir: &str) -> String {
    let d = dir.trim().trim_end_matches(['/', '\\']);
    match home() {
        Some(h) if Path::new(d) == h.join(prefix(agent)) => String::new(),
        _ => d.to_string(),
    }
}

/// The account the app was launched with (its environment).
pub fn launched(agent: Agent) -> String {
    std::env::var(var(agent))
        .map(|d| normalise(agent, &d))
        .unwrap_or_default()
}

/// A folder holds a signed-in (or once signed-in) account.
fn is_account(agent: Agent, dir: &Path) -> bool {
    let marks: &[&str] = match agent {
        Agent::Claude => &[".credentials.json", ".claude.json", "settings.json"],
        Agent::Codex => &["auth.json", "config.toml"],
    };
    dir.is_dir() && marks.iter().any(|m| dir.join(m).exists())
}

/// Every account found: the default, the launch one, and the
/// `~/.claude-*` / `~/.codex-*` folders beside the default.
pub fn list(agent: Agent) -> Vec<Account> {
    let mut out = vec![Account {
        dir: String::new(),
        label: "Default".into(),
    }];
    let mut add = |dir: String| {
        if !dir.is_empty() && !out.iter().any(|a| a.dir == dir) {
            out.push(Account {
                label: label(agent, &dir),
                dir,
            });
        }
    };
    add(launched(agent));
    if let Some(h) = home()
        && let Ok(rd) = std::fs::read_dir(&h)
    {
        let p = format!("{}-", prefix(agent));
        let mut found: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&p) && n.len() > p.len())
            })
            .filter(|path| is_account(agent, path))
            .collect();
        found.sort();
        for f in found {
            add(normalise(agent, &f.to_string_lossy()));
        }
    }
    out
}

/// The name shown for an account folder.
pub fn label(agent: Agent, dir: &str) -> String {
    if dir.is_empty() {
        return "Default".into();
    }
    let name = Path::new(dir)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(dir)
        .to_string();
    let p = format!("{}-", prefix(agent));
    name.strip_prefix(&p)
        .map(str::to_string)
        .unwrap_or_else(|| name.trim_start_matches('.').to_string())
}

fn chosen() -> &'static RwLock<HashMap<Agent, String>> {
    static C: std::sync::OnceLock<RwLock<HashMap<Agent, String>>> = std::sync::OnceLock::new();
    C.get_or_init(|| RwLock::new(HashMap::new()))
}

/// The account new CLIs run as (the launch one until one is picked).
pub fn current(agent: Agent) -> String {
    chosen()
        .read()
        .ok()
        .and_then(|m| m.get(&agent).cloned())
        .unwrap_or_else(|| launched(agent))
}

pub fn set_current(agent: Agent, dir: &str) {
    if let Ok(mut m) = chosen().write() {
        m.insert(agent, normalise(agent, dir));
    }
}

/// Points `cmd` at `dir`'s account.
pub fn apply(cmd: &mut Command, agent: Agent, dir: &str) {
    if dir.is_empty() {
        cmd.env_remove(var(agent));
    } else {
        cmd.env(var(agent), dir);
    }
}

/// The account's config folder on disk.
pub fn folder(agent: Agent, dir: &str) -> Option<PathBuf> {
    if dir.is_empty() {
        home().map(|h| h.join(prefix(agent)))
    } else {
        Some(PathBuf::from(dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_come_from_folder_names() {
        assert_eq!(label(Agent::Claude, ""), "Default");
        assert_eq!(label(Agent::Claude, "/h/.claude-work"), "work");
        assert_eq!(label(Agent::Codex, "/h/.codex-personal"), "personal");
        assert_eq!(label(Agent::Claude, "/h/other"), "other");
    }

    #[test]
    fn the_default_folder_means_no_variable() {
        if let Some(h) = home() {
            let d = h.join(".claude").to_string_lossy().to_string();
            assert_eq!(normalise(Agent::Claude, &d), "");
            assert_eq!(normalise(Agent::Claude, &format!("{d}/")), "");
        }
        let mut cmd = Command::new("true");
        apply(&mut cmd, Agent::Claude, "");
        assert!(cmd.get_envs().any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v.is_none()));
        apply(&mut cmd, Agent::Codex, "/x/.codex-b");
        assert!(
            cmd.get_envs()
                .any(|(k, v)| k == "CODEX_HOME" && v == Some(std::ffi::OsStr::new("/x/.codex-b")))
        );
    }
}

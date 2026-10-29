//! Whether the user is signed in to an agent's CLI, and signing in from the
//! panel. Claude Code answers `claude auth status` (JSON, `loggedIn`), Codex
//! `codex login status` (exit 0 when signed in). Both checked against
//! claude-code 2.1.311 and codex-cli 0.164.
//!
//! Signing in runs the CLI's own login (`claude auth login`, `codex
//! login`), which opens the browser. Claude Code may then ask for the code
//! the browser shows; the panel passes it on through stdin.

use super::Agent;
use crossbeam_channel::{Receiver, unbounded};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};

/// What the panel knows about an agent's sign-in.
pub enum Auth {
    Checking(Receiver<bool>),
    In,
    Out,
    SigningIn(Login),
}

/// The CLI with the app's usual launch rules: no console window on Windows,
/// not nested in a Claude Code that launched the app, dies with the app,
/// runs as the account picked in the panel.
fn command(agent: Agent, exe: &Path) -> Command {
    let mut cmd = Command::new(exe);
    super::accounts::apply(&mut cmd, agent, &super::accounts::current(agent));
    for (k, _) in std::env::vars_os() {
        if let Some(k) = k.to_str()
            && (k == "CLAUDECODE" || k.starts_with("CLAUDE_CODE_"))
        {
            cmd.env_remove(k);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: prctl is async-signal-safe and touches no memory.
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
    }
    cmd
}

/// Asks the CLI on a thread. Only a clear "signed out" counts: an older CLI
/// without the command, or one that hangs, isn't held against the user.
pub fn check(agent: Agent, exe: &Path, wake: impl Fn() + Send + 'static) -> Receiver<bool> {
    let (tx, rx) = unbounded();
    let mut cmd = command(agent, exe);
    match agent {
        Agent::Claude => cmd.args(["auth", "status", "--json"]),
        Agent::Codex => cmd.args(["login", "status"]),
    };
    let _ = std::thread::Builder::new().name("agent-auth".into()).spawn(move || {
        let signed_in = match cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(mut child) => {
                let start = std::time::Instant::now();
                let status = loop {
                    match child.try_wait() {
                        Ok(Some(s)) => break Some(s),
                        Ok(None) if start.elapsed().as_secs() < 15 => {
                            std::thread::sleep(std::time::Duration::from_millis(50))
                        }
                        _ => {
                            let _ = child.kill();
                            let _ = child.wait();
                            break None;
                        }
                    }
                };
                let mut out = String::new();
                if let Some(mut o) = child.stdout.take() {
                    let _ = o.read_to_string(&mut out);
                }
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut out);
                }
                status.is_none_or(|s| signed_in(agent, s.success(), &out))
            }
            Err(_) => true,
        };
        let _ = tx.send(signed_in);
        wake();
    });
    rx
}

/// The answer to a status command (see `check`).
fn signed_in(agent: Agent, ok: bool, out: &str) -> bool {
    match agent {
        Agent::Claude => {
            let start = out.find('{').unwrap_or(0);
            match serde_json::from_str::<serde_json::Value>(&out[start..]) {
                Ok(v) => v["loggedIn"].as_bool() != Some(false),
                Err(_) => true,
            }
        }
        Agent::Codex => ok || !out.to_lowercase().contains("not logged in"),
    }
}

/// A running login.
pub struct Login {
    child: Child,
    stdin: Option<ChildStdin>,
    /// The sign-in page, once the CLI prints it (for "open it again").
    pub url: Option<String>,
    lines: Receiver<String>,
    /// The code field (Claude Code only).
    pub code: String,
    /// A code was sent; waiting for the CLI to finish.
    pub sent: bool,
}

impl Login {
    pub fn start(agent: Agent, exe: &Path, wake: impl Fn() + Send + Sync + 'static) -> std::io::Result<Login> {
        let mut cmd = command(agent, exe);
        match agent {
            Agent::Claude => cmd.args(["auth", "login"]),
            Agent::Codex => cmd.args(["login"]),
        };
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let (tx, lines) = unbounded();
        let wake = std::sync::Arc::new(wake);
        let pipes: [Option<Box<dyn Read + Send>>; 2] = [
            child.stdout.take().map(|o| Box::new(o) as _),
            child.stderr.take().map(|e| Box::new(e) as _),
        ];
        for pipe in pipes.into_iter().flatten() {
            let tx = tx.clone();
            let wake = wake.clone();
            let _ = std::thread::Builder::new().name("agent-login".into()).spawn(move || {
                for line in BufReader::new(pipe).lines() {
                    let Ok(line) = line else { break };
                    let _ = tx.send(line);
                    wake();
                }
            });
        }
        Ok(Login {
            stdin: child.stdin.take(),
            child,
            url: None,
            lines,
            code: String::new(),
            sent: false,
        })
    }

    /// Reads what the CLI printed; `Some(())` once it has exited.
    pub fn poll(&mut self) -> Option<()> {
        while let Ok(line) = self.lines.try_recv() {
            if self.url.is_none()
                && let Some(i) = line.find("https://")
            {
                let url = line[i..].split_whitespace().next().unwrap_or_default();
                self.url = Some(url.to_string());
            }
        }
        matches!(self.child.try_wait(), Ok(Some(_))).then_some(())
    }

    pub fn send_code(&mut self) {
        let code = self.code.trim().to_string();
        if let Some(sin) = &mut self.stdin
            && !code.is_empty()
        {
            let _ = writeln!(sin, "{code}");
            let _ = sin.flush();
            self.sent = true;
        }
    }
}

impl Drop for Login {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_answers() {
        let out = r#"{"loggedIn": false, "authMethod": "none"}"#;
        assert!(!signed_in(Agent::Claude, false, out));
        assert!(signed_in(Agent::Claude, true, r#"{"loggedIn": true}"#));
        // An older CLI that doesn't know the command isn't a sign-out.
        assert!(signed_in(Agent::Claude, false, "error: unknown command 'auth'"));
        assert!(!signed_in(Agent::Codex, false, "WARNING: …\nNot logged in\n"));
        assert!(signed_in(Agent::Codex, true, "Logged in using ChatGPT"));
        assert!(signed_in(Agent::Codex, false, "error: unrecognized subcommand"));
    }
}

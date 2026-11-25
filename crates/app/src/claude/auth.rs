//! Whether the user is signed in to an agent's CLI. Claude Code answers
//! `claude auth status` (JSON, `loggedIn`), Codex `codex login status` (exit
//! 0 when signed in). Both checked against claude-code 2.1.332 and codex-cli
//! 0.172.
//!
//! Signing in happens outside the app: the panel shows the CLI's own login
//! command (`claude auth login`, `codex login`) to run in a terminal. The app
//! never starts a login, and never sees a code or a token.

use super::Agent;
use crossbeam_channel::{Receiver, unbounded};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

/// What the panel knows about an agent's sign-in.
pub enum Auth {
    Checking(Receiver<bool>),
    In,
    /// Signed out; the command that signs in.
    Out(String),
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

/// The command that signs in to the account picked in the panel, for the
/// user to run in their own terminal (PowerShell on Windows). The panel
/// never runs it: Anthropic doesn't let apps offer Claude's sign-in.
pub fn login_command(agent: Agent, exe: &Path) -> String {
    let dir = super::accounts::current(agent);
    let inherited = std::env::var_os(super::accounts::var(agent)).is_some();
    login_text(
        agent,
        &exe.to_string_lossy(),
        agent.on_path(exe),
        &dir,
        inherited,
        cfg!(windows),
    )
}

/// `login_command`'s text. `plain`: the bare name finds the CLI in a
/// terminal. `inherited`: the account variable is set where the app was
/// started, so a terminal likely has it too, and the default account means
/// taking it away.
fn login_text(agent: Agent, exe: &str, plain: bool, dir: &str, inherited: bool, windows: bool) -> String {
    let var = super::accounts::var(agent);
    let quote = |s: &str| quoted(s, windows);
    let cli = match (plain, windows) {
        (true, _) => agent.command().to_string(),
        // PowerShell runs a quoted path only with `&`.
        (false, true) => format!("& {}", quote(exe)),
        (false, false) => quote(exe),
    };
    let login = match agent {
        Agent::Claude => format!("{cli} auth login"),
        Agent::Codex => format!("{cli} login"),
    };
    match (dir.is_empty(), windows) {
        (true, _) if !inherited => login,
        (true, true) => format!("$env:{var}=$null; {login}"),
        (true, false) => format!("env -u {var} {login}"),
        (false, true) => format!("$env:{var}={}; {login}", quote(dir)),
        (false, false) => format!("{var}={} {login}", quote(dir)),
    }
}

/// `s` as one word of a command. PowerShell's is always quoted.
fn quoted(s: &str, windows: bool) -> String {
    if windows {
        return format!("'{}'", s.replace('\'', "''"));
    }
    let safe = |c: char| c.is_ascii_alphanumeric() || "/._-+:@%,".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
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

    #[test]
    fn login_commands_for_a_shell() {
        let cmd = |a, exe, plain, dir, inherited| login_text(a, exe, plain, dir, inherited, false);
        use Agent::{Claude, Codex};
        assert_eq!(cmd(Claude, "/usr/bin/claude", true, "", false), "claude auth login");
        assert_eq!(cmd(Codex, "/usr/bin/codex", true, "", false), "codex login");
        // Another account's folder goes in front.
        assert_eq!(
            cmd(Claude, "/usr/bin/claude", true, "/h/.claude-work", false),
            "CLAUDE_CONFIG_DIR=/h/.claude-work claude auth login"
        );
        assert_eq!(
            cmd(Codex, "/usr/bin/codex", true, "/h/.codex-work", true),
            "CODEX_HOME=/h/.codex-work codex login"
        );
        // The default account, from a terminal set to another one.
        assert_eq!(
            cmd(Claude, "/usr/bin/claude", true, "", true),
            "env -u CLAUDE_CONFIG_DIR claude auth login"
        );
        // Not on the PATH, and names a shell would split or expand.
        assert_eq!(
            cmd(Codex, "/opt/my tools/codex", false, "/h/it's $x", false),
            r"CODEX_HOME='/h/it'\''s $x' '/opt/my tools/codex' login"
        );
    }

    #[test]
    fn login_commands_for_powershell() {
        let cmd = |a, exe, plain, dir, inherited| login_text(a, exe, plain, dir, inherited, true);
        use Agent::{Claude, Codex};
        assert_eq!(cmd(Claude, r"C:\x\claude.exe", true, "", false), "claude auth login");
        assert_eq!(
            cmd(Claude, r"C:\a b\claude.exe", false, "", false),
            r"& 'C:\a b\claude.exe' auth login"
        );
        assert_eq!(
            cmd(Codex, r"C:\x\codex.exe", true, r"D:\o'neil\.codex-work", false),
            r"$env:CODEX_HOME='D:\o''neil\.codex-work'; codex login"
        );
        assert_eq!(
            cmd(Claude, r"C:\x\claude.exe", true, "", true),
            "$env:CLAUDE_CONFIG_DIR=$null; claude auth login"
        );
    }

    /// The shell reads a quoted command back as it was meant.
    #[cfg(unix)]
    #[test]
    fn a_shell_runs_the_command() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("opentern login's {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("claude");
        std::fs::write(&exe, "#!/bin/sh\necho \"${CLAUDE_CONFIG_DIR-unset}|$*\"\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let run = |account: &str, inherited| {
            let text = login_text(Agent::Claude, &exe.to_string_lossy(), false, account, inherited, false);
            let out = Command::new("sh")
                .args(["-c", &text])
                .env("CLAUDE_CONFIG_DIR", "/other")
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        assert_eq!(run("/h/a $b 'c'", false), "/h/a $b 'c'|auth login");
        assert_eq!(run("", true), "unset|auth login");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

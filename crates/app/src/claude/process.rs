//! Talks to the Claude Code CLI the user has installed: one `claude -p`
//! process per chat, streaming JSON both ways (the same protocol Claude
//! Code's SDKs use). Permission prompts come back to us
//! (`--permission-prompt-tool stdio`), and the app's own tools are an
//! in-process MCP server the CLI reaches over the same pipe (`mcp_message`).

use crossbeam_channel::{Receiver, Sender, unbounded};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

/// The MCP server name the app's tools live under (`mcp__opentern__…`).
pub const SERVER: &str = "opentern";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Mode {
    /// Claude asks before anything that changes files or runs commands.
    Manual,
    /// Claude Code's auto mode: it decides what's safe and asks less (the
    /// default, as in a terminal set up that way).
    #[default]
    Auto,
}

impl Mode {
    pub fn cli(self) -> &'static str {
        match self {
            Mode::Manual => "manual",
            Mode::Auto => "auto",
        }
    }
}

pub struct Launch {
    pub agent: super::agent::Agent,
    pub exe: PathBuf,
    /// The workspace's scripts folder: where Claude reads and edits.
    pub cwd: PathBuf,
    pub mode: Mode,
    /// A model id, or `None` for the CLI's default.
    pub model: Option<String>,
    /// The CLI's session (Claude Code) or thread (Codex) id, to continue a chat.
    pub resume: Option<String>,
    pub system: String,
    /// The account's config folder ("" = the CLI's default; `accounts.rs`).
    pub account: String,
}

/// One window of the account's usage limits (`five_hour`, `seven_day`, …).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Window {
    pub name: String,
    /// 0–1.
    pub used: f64,
    /// Unix seconds.
    pub resets: i64,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Usage {
    fn of(v: &Value) -> Usage {
        let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        Usage {
            input: n("input_tokens"),
            output: n("output_tokens"),
            cache_read: n("cache_read_input_tokens"),
            cache_write: n("cache_creation_input_tokens"),
        }
    }

    pub fn add(&mut self, o: &Usage) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
    }

    /// Tokens the model read for one request: its context size.
    pub fn context(&self) -> u64 {
        self.input + self.cache_read + self.cache_write
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Init {
        session: String,
        model: String,
    },
    /// A new text block starts (the next `Text` begins a new paragraph run).
    TextStart,
    Text(String),
    Thinking,
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        id: String,
        text: String,
        error: bool,
    },
    /// One model request's usage (the latest is the context size).
    Request(Usage),
    Done {
        usage: Usage,
        cost: f64,
        error: bool,
        /// Why it failed, when it did.
        message: String,
    },
    Limits(Vec<Window>),
    /// The models the account can use: (id, name). Codex only.
    Models(Vec<(String, String)>),
    Permission {
        request: String,
        tool: String,
        input: Value,
        description: String,
        /// `permission_suggestions`, sent back to allow "always".
        suggestions: Value,
    },
    Mcp {
        request: String,
        message: Value,
    },
    /// The CLI withdrew a permission prompt (the turn was interrupted).
    Cancel(String),
    Exited(String),
}

/// One running agent CLI: a chat's connection, whichever agent it is.
pub enum Client {
    Claude(ClaudeCode),
    Codex(super::codex::Codex),
}

impl Client {
    pub fn start(l: &Launch, wake: Arc<dyn Fn() + Send + Sync>) -> std::io::Result<Client> {
        Ok(match l.agent {
            super::agent::Agent::Claude => Client::Claude(ClaudeCode::start(l, wake)?),
            super::agent::Agent::Codex => Client::Codex(super::codex::Codex::start(l, wake)?),
        })
    }

    pub fn events(&self) -> &Receiver<Event> {
        match self {
            Client::Claude(c) => &c.events,
            Client::Codex(c) => &c.events,
        }
    }

    /// A user turn: a context block the agent sees first, the attached
    /// files, then the visible text.
    pub fn send(&self, context: &str, files: &[String], text: &str) {
        match self {
            Client::Claude(c) => c.send(context, super::attach::blocks(files), text),
            Client::Codex(c) => c.send(context, files, text),
        }
    }

    pub fn allow(&self, request: &str, input: &Value, always: Option<&Value>) {
        match self {
            Client::Claude(c) => c.allow(request, input, always),
            Client::Codex(c) => c.answer(request, true, always.is_some()),
        }
    }

    pub fn deny(&self, request: &str) {
        match self {
            Client::Claude(c) => c.deny(request),
            Client::Codex(c) => c.answer(request, false, false),
        }
    }

    /// The reply to one of the app's tool calls (a JSON-RPC MCP reply).
    pub fn mcp_reply(&self, request: &str, response: Value) {
        match self {
            Client::Claude(c) => c.mcp_reply(request, response),
            Client::Codex(c) => c.tool_reply(request, &response),
        }
    }

    pub fn interrupt(&mut self) {
        match self {
            Client::Claude(c) => c.interrupt(),
            Client::Codex(c) => c.interrupt(),
        }
    }

    pub fn set_mode(&mut self, mode: Mode) {
        match self {
            Client::Claude(c) => c.set_mode(mode),
            Client::Codex(c) => c.set_mode(mode),
        }
    }
}

/// One running `claude` process.
pub struct ClaudeCode {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    pub events: Receiver<Event>,
    seq: u64,
}

impl ClaudeCode {
    pub fn start(l: &Launch, wake: Arc<dyn Fn() + Send + Sync>) -> std::io::Result<ClaudeCode> {
        let mcp = json!({"mcpServers": {SERVER: {"type": "sdk", "name": SERVER}}}).to_string();
        let mut cmd = Command::new(&l.exe);
        cmd.args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-prompt-tool",
            "stdio",
            "--permission-mode",
            l.mode.cli(),
            "--mcp-config",
            &mcp,
            "--allowedTools",
            &format!("mcp__{SERVER}"),
            "--append-system-prompt",
            &l.system,
        ]);
        if let Some(m) = &l.model {
            cmd.args(["--model", m]);
        }
        if let Some(s) = &l.resume {
            cmd.args(["--resume", s]);
        }
        super::accounts::apply(&mut cmd, l.agent, &l.account);
        // A session started from inside Claude Code (a terminal, a test)
        // would otherwise think it's nested.
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
        // If the app dies without closing it (a crash, a kill), the kernel
        // ends Claude Code too, instead of leaving it waiting.
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
        let mut child = cmd
            .current_dir(&l.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = Arc::new(Mutex::new(
            child.stdin.take().ok_or_else(|| std::io::Error::other("no stdin"))?,
        ));
        let stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?;
        let mut stderr = child.stderr.take().ok_or_else(|| std::io::Error::other("no stderr"))?;
        let (tx, events) = unbounded();
        // stderr is only read for the exit message (kept short).
        let err_text = Arc::new(Mutex::new(String::new()));
        let et = err_text.clone();
        let _ = std::thread::Builder::new().name("claude-err".into()).spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            if let Ok(mut e) = et.lock() {
                *e = s;
            }
        });
        let _ = std::thread::Builder::new()
            .name("claude".into())
            .spawn(move || read(stdout, tx, wake, err_text));
        Ok(ClaudeCode {
            child,
            stdin,
            events,
            seq: 0,
        })
    }

    fn write(&self, v: &Value) {
        if let Ok(mut s) = self.stdin.lock() {
            let _ = writeln!(s, "{v}");
            let _ = s.flush();
        }
    }

    fn control(&mut self, request: Value) {
        self.seq += 1;
        self.write(&json!({"type": "control_request", "request_id": format!("ot-{}", self.seq), "request": request}));
    }

    /// A user turn: a context block Claude sees first, attached files'
    /// blocks, then the visible text.
    pub fn send(&self, context: &str, files: Vec<Value>, text: &str) {
        let mut content = Vec::new();
        if !context.is_empty() {
            content.push(json!({"type": "text", "text": context}));
        }
        content.extend(files);
        if !text.is_empty() {
            content.push(json!({"type": "text", "text": text}));
        }
        self.write(&json!({"type": "user", "message": {"role": "user", "content": content}}));
    }

    pub fn allow(&self, request: &str, input: &Value, always: Option<&Value>) {
        let mut r = json!({"behavior": "allow", "updatedInput": input});
        if let Some(s) = always.filter(|s| s.as_array().is_some_and(|a| !a.is_empty())) {
            r["updatedPermissions"] = s.clone();
        }
        self.respond(request, r);
    }

    pub fn deny(&self, request: &str) {
        self.respond(
            request,
            json!({"behavior": "deny", "message": "The user declined this in Opentern."}),
        );
    }

    pub fn mcp_reply(&self, request: &str, response: Value) {
        self.respond(request, json!({"mcp_response": response}));
    }

    fn respond(&self, request: &str, response: Value) {
        self.write(&json!({"type": "control_response", "response": {
            "subtype": "success", "request_id": request, "response": response
        }}));
    }

    pub fn interrupt(&mut self) {
        self.control(json!({"subtype": "interrupt"}));
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.control(json!({"subtype": "set_permission_mode", "mode": mode.cli()}));
    }
}

impl Drop for ClaudeCode {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read(out: impl Read, tx: Sender<Event>, wake: Arc<dyn Fn() + Send + Sync>, err_text: Arc<Mutex<String>>) {
    for line in BufReader::new(out).lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let mut any = false;
        for e in parse(&v) {
            any |= tx.send(e).is_ok();
        }
        if any {
            wake();
        }
    }
    // Give stderr a moment to finish.
    std::thread::sleep(std::time::Duration::from_millis(100));
    let err = err_text.lock().map(|e| e.trim().to_string()).unwrap_or_default();
    let _ = tx.send(Event::Exited(last_lines(&err, 6)));
    wake();
}

fn last_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// What one line of the CLI's output means to the panel. Subagents' traffic
/// (`parent_tool_use_id` set) is left out: the panel shows the main thread.
pub fn parse(v: &Value) -> Vec<Event> {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("");
    let nested = v.get("parent_tool_use_id").is_some_and(|p| !p.is_null());
    let mut out = Vec::new();
    match s("type") {
        "system" if s("subtype") == "init" => out.push(Event::Init {
            session: s("session_id").to_string(),
            model: s("model").to_string(),
        }),
        "stream_event" if !nested => {
            let e = &v["event"];
            match e["type"].as_str() {
                Some("content_block_start") => match e["content_block"]["type"].as_str() {
                    Some("text") => out.push(Event::TextStart),
                    Some("thinking") => out.push(Event::Thinking),
                    _ => {}
                },
                Some("content_block_delta") if e["delta"]["type"] == "text_delta" => {
                    if let Some(t) = e["delta"]["text"].as_str() {
                        out.push(Event::Text(t.to_string()));
                    }
                }
                _ => {}
            }
        }
        "assistant" if !nested => {
            let m = &v["message"];
            for b in m["content"].as_array().into_iter().flatten() {
                if b["type"] == "tool_use" {
                    out.push(Event::ToolUse {
                        id: b["id"].as_str().unwrap_or("").to_string(),
                        name: b["name"].as_str().unwrap_or("").to_string(),
                        input: b["input"].clone(),
                    });
                }
            }
            if m.get("usage").is_some() {
                out.push(Event::Request(Usage::of(&m["usage"])));
            }
        }
        "user" if !nested => {
            for b in v["message"]["content"].as_array().into_iter().flatten() {
                if b["type"] == "tool_result" {
                    out.push(Event::ToolResult {
                        id: b["tool_use_id"].as_str().unwrap_or("").to_string(),
                        text: text_of(&b["content"]),
                        error: b["is_error"].as_bool().unwrap_or(false),
                    });
                }
            }
        }
        "result" => {
            let error = v["is_error"].as_bool().unwrap_or(false);
            let message = if !error {
                String::new()
            } else if s("subtype") == "error_during_execution" {
                "Stopped.".to_string()
            } else {
                let r = s("result");
                if r.is_empty() {
                    s("subtype").replace('_', " ")
                } else {
                    r.to_string()
                }
            };
            out.push(Event::Done {
                usage: Usage::of(&v["usage"]),
                cost: v["total_cost_usd"].as_f64().unwrap_or(0.0),
                error,
                message,
            });
        }
        "rate_limit_event" => {
            let info = &v["rate_limit_info"];
            let mut ws: Vec<Window> = info["unifiedWindows"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, w)| Window {
                    name: k.clone(),
                    used: w["utilization"].as_f64().unwrap_or(0.0),
                    resets: w["resetsAt"].as_i64().unwrap_or(0),
                })
                .collect();
            if ws.is_empty()
                && let Some(k) = info["rateLimitType"].as_str()
            {
                ws.push(Window {
                    name: k.to_string(),
                    used: info["utilization"].as_f64().unwrap_or(0.0),
                    resets: info["resetsAt"].as_i64().unwrap_or(0),
                });
            }
            ws.sort_by_key(|w| order(&w.name));
            out.push(Event::Limits(ws));
        }
        "control_request" => {
            let r = &v["request"];
            let id = s("request_id").to_string();
            match r["subtype"].as_str() {
                Some("can_use_tool") => out.push(Event::Permission {
                    request: id,
                    tool: r["tool_name"].as_str().unwrap_or("").to_string(),
                    input: r["input"].clone(),
                    description: r["description"].as_str().unwrap_or("").to_string(),
                    suggestions: r["permission_suggestions"].clone(),
                }),
                Some("mcp_message") if r["server_name"] == SERVER => out.push(Event::Mcp {
                    request: id,
                    message: r["message"].clone(),
                }),
                _ => {}
            }
        }
        "control_cancel_request" => out.push(Event::Cancel(s("request_id").to_string())),
        _ => {}
    }
    out
}

/// Short windows first.
fn order(name: &str) -> u8 {
    match name {
        "five_hour" => 0,
        "seven_day" => 1,
        _ => 2,
    }
}

/// A plain name for a usage window (Codex's are `w<minutes>`).
pub fn window_name(name: &str) -> String {
    if let Some(m) = name.strip_prefix('w').and_then(|m| m.parse::<i64>().ok()) {
        return match m {
            300 => "5-hour limit".into(),
            10080 => "Weekly limit".into(),
            43200 => "Monthly limit".into(),
            m if m % 1440 == 0 => format!("{}-day limit", m / 1440),
            m if m % 60 == 0 => format!("{}-hour limit", m / 60),
            m => format!("{m}-minute limit"),
        };
    }
    match name {
        "five_hour" => "5-hour limit".into(),
        "seven_day" => "Weekly limit".into(),
        "seven_day_opus" => "Weekly (Opus)".into(),
        "seven_day_sonnet" => "Weekly (Sonnet)".into(),
        other => other.replace('_', " "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Vec<Event> {
        parse(&serde_json::from_str(s).unwrap())
    }

    #[test]
    fn parses_the_stream() {
        assert_eq!(
            p(r#"{"type":"system","subtype":"init","session_id":"s1","model":"m"}"#),
            vec![Event::Init {
                session: "s1".into(),
                model: "m".into()
            }]
        );
        assert_eq!(
            p(
                r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}}"#
            ),
            vec![Event::Text("Hi".into())]
        );
        // A subagent's text isn't the main thread's.
        assert!(
            p(
                r#"{"type":"stream_event","parent_tool_use_id":"t9","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"x"}}}"#
            )
            .is_empty()
        );
        let e = p(
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":2,"cache_read_input_tokens":100,"output_tokens":5}}}"#,
        );
        assert_eq!(
            e[0],
            Event::ToolUse {
                id: "t1".into(),
                name: "Bash".into(),
                input: json!({"command": "ls"})
            }
        );
        assert_eq!(
            e[1],
            Event::Request(Usage {
                input: 2,
                output: 5,
                cache_read: 100,
                cache_write: 0
            })
        );
        assert_eq!(
            p(
                r#"{"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"a.pine"}],"is_error":false}]}}"#
            ),
            vec![Event::ToolResult {
                id: "t1".into(),
                text: "a.pine".into(),
                error: false
            }]
        );
        let lim = p(
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","unifiedWindows":{"seven_day":{"utilization":0.32,"resetsAt":20},"five_hour":{"utilization":0.26,"resetsAt":10}}}}"#,
        );
        assert_eq!(
            lim,
            vec![Event::Limits(vec![
                Window {
                    name: "five_hour".into(),
                    used: 0.26,
                    resets: 10
                },
                Window {
                    name: "seven_day".into(),
                    used: 0.32,
                    resets: 20
                },
            ])]
        );
        match &p(
            r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Write","input":{"file_path":"a"},"description":"a","permission_suggestions":[{"type":"setMode","mode":"acceptEdits","destination":"session"}]}}"#,
        )[0]
        {
            Event::Permission {
                request,
                tool,
                suggestions,
                ..
            } => {
                assert_eq!((request.as_str(), tool.as_str()), ("r1", "Write"));
                assert!(suggestions.is_array());
            }
            e => panic!("{e:?}"),
        }
        assert!(matches!(
            &p(r#"{"type":"control_request","request_id":"r2","request":{"subtype":"mcp_message","server_name":"opentern","message":{"method":"tools/list","id":1}}}"#)[0],
            Event::Mcp { request, .. } if request == "r2"
        ));
        match &p(
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"total_cost_usd":0.5,"usage":{"input_tokens":1,"output_tokens":2}}"#,
        )[0]
        {
            Event::Done {
                error, message, cost, ..
            } => {
                assert!(*error);
                assert_eq!(message, "Stopped.");
                assert_eq!(*cost, 0.5);
            }
            e => panic!("{e:?}"),
        }
    }
}

//! Talks to the Codex CLI the user has installed: one `codex app-server`
//! per chat, JSON-RPC lines both ways (the protocol Codex's own IDE
//! extensions use; checked against codex-cli 0.163). The app's tools are
//! the thread's dynamic tools, answered on the UI thread like Claude's MCP
//! calls; approvals come back as requests. Everything is turned into the
//! same `Event`s the panel already shows for Claude Code.

use super::effort::Model;
use super::process::{Event, Launch, Mode, Usage, Window};
use crossbeam_channel::{Receiver, Sender, unbounded};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

fn policy(mode: Mode) -> &'static str {
    match mode {
        // Asks before anything it doesn't know is safe.
        Mode::Manual => "untrusted",
        // Works freely in the scripts folder; asks to go outside it.
        Mode::Auto => "on-request",
    }
}

/// What the reader and the UI thread share.
#[derive(Default)]
struct State {
    next: u64,
    /// Our requests waiting for a reply: id → what it was.
    asked: HashMap<u64, &'static str>,
    thread: Option<String>,
    turn: Option<String>,
    /// Turns sent before the thread was ready.
    queued: Vec<Value>,
    mode: Mode,
    /// The effort picked, sent with each turn.
    effort: Option<String>,
    /// Requests from Codex waiting for the user or the app: our key → its id.
    open: HashMap<String, Value>,
    /// File changes by item, for their approval cards.
    changes: HashMap<String, Value>,
    /// Commands by item.
    commands: HashMap<String, String>,
    /// Agent messages that streamed (the rest arrive whole).
    streamed: HashSet<String>,
    total: Usage,
    base: Usage,
    error: Option<String>,
    launch: Option<(Value, Option<String>)>,
    /// Pages of the model list so far.
    pages: Vec<Value>,
}

pub struct Codex {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    st: Arc<Mutex<State>>,
    pub events: Receiver<Event>,
}

fn write(stdin: &Mutex<ChildStdin>, v: &Value) {
    if let Ok(mut s) = stdin.lock() {
        let _ = writeln!(s, "{v}");
        let _ = s.flush();
    }
}

fn request(stdin: &Mutex<ChildStdin>, st: &mut State, what: &'static str, method: &str, params: Value) {
    st.next += 1;
    st.asked.insert(st.next, what);
    write(stdin, &json!({"id": st.next, "method": method, "params": params}));
}

impl Codex {
    pub fn start(l: &Launch, wake: Arc<dyn Fn() + Send + Sync>) -> std::io::Result<Codex> {
        let mut cmd = Command::new(&l.exe);
        cmd.arg("app-server");
        super::accounts::apply(&mut cmd, l.agent, &l.account);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        // If the app dies without closing it, the kernel ends Codex too.
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
        let tools: Vec<Value> = super::tools::specs()
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t["name"],
                    "description": t["description"],
                    "inputSchema": t["inputSchema"],
                })
            })
            .collect();
        let cwd = l.cwd.to_string_lossy().into_owned();
        let mut start = json!({
            "cwd": cwd,
            "approvalPolicy": policy(l.mode),
            "sandbox": "workspace-write",
            "developerInstructions": l.system,
            "dynamicTools": tools,
        });
        if let Some(m) = &l.model {
            start["model"] = json!(m);
        }
        let st = Arc::new(Mutex::new(State {
            mode: l.mode,
            effort: l.effort.clone(),
            launch: Some((start, l.resume.clone())),
            ..Default::default()
        }));
        if let Ok(mut s) = st.lock() {
            request(
                &stdin,
                &mut s,
                "init",
                "initialize",
                json!({
                    "clientInfo": {"name": "opentern", "title": "Opentern", "version": env!("CARGO_PKG_VERSION")},
                    "capabilities": {"experimentalApi": true},
                }),
            );
        }
        let (tx, events) = unbounded();
        let err_text = Arc::new(Mutex::new(String::new()));
        let et = err_text.clone();
        let _ = std::thread::Builder::new().name("codex-err".into()).spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            if let Ok(mut e) = et.lock() {
                *e = s;
            }
        });
        let (sin, sst) = (stdin.clone(), st.clone());
        let _ = std::thread::Builder::new()
            .name("codex".into())
            .spawn(move || read(stdout, sin, sst, tx, wake, err_text));
        Ok(Codex {
            child,
            stdin,
            st,
            events,
        })
    }

    pub fn send(&self, context: &str, files: &[String], text: &str) {
        let mut input = Vec::new();
        if !context.is_empty() {
            input.push(json!({"type": "text", "text": context}));
        }
        input.extend(inputs(files));
        if !text.is_empty() {
            input.push(json!({"type": "text", "text": text}));
        }
        let Ok(mut st) = self.st.lock() else { return };
        match st.thread.clone() {
            Some(thread) => {
                let p = turn(&thread, json!(input), &st);
                request(&self.stdin, &mut st, "turn", "turn/start", p);
            }
            None => st.queued.push(json!(input)),
        }
    }

    /// Answers an approval (`always` = for the rest of this chat).
    pub fn answer(&self, key: &str, allow: bool, always: bool) {
        let Ok(mut st) = self.st.lock() else { return };
        let Some(id) = st.open.remove(key) else { return };
        let decision = match (allow, always) {
            (true, true) => "acceptForSession",
            (true, false) => "accept",
            _ => "decline",
        };
        write(&self.stdin, &json!({"id": id, "result": {"decision": decision}}));
    }

    /// The app's answer to a tool call, given as an MCP reply.
    pub fn tool_reply(&self, key: &str, reply: &Value) {
        let Ok(mut st) = self.st.lock() else { return };
        let Some(id) = st.open.remove(key) else { return };
        let r = &reply["result"];
        let content = r["content"].as_array().cloned().unwrap_or_default();
        let text: Vec<&str> = content.iter().filter_map(|c| c["text"].as_str()).collect();
        let mut items = vec![json!({"type": "inputText", "text": text.join("\n")})];
        // Pictures (the snapshot tool) go as data URLs.
        for c in content.iter().filter(|c| c["type"] == "image") {
            let (mime, data) = (
                c["mimeType"].as_str().unwrap_or("image/png"),
                c["data"].as_str().unwrap_or(""),
            );
            items.push(json!({"type": "inputImage", "imageUrl": format!("data:{mime};base64,{data}")}));
        }
        let success = !r["isError"].as_bool().unwrap_or(false) && reply.get("error").is_none();
        write(
            &self.stdin,
            &json!({"id": id, "result": {
                "contentItems": items,
                "success": success,
            }}),
        );
    }

    pub fn interrupt(&mut self) {
        let Ok(mut st) = self.st.lock() else { return };
        if let (Some(thread), Some(turn)) = (st.thread.clone(), st.turn.clone()) {
            request(
                &self.stdin,
                &mut st,
                "interrupt",
                "turn/interrupt",
                json!({"threadId": thread, "turnId": turn}),
            );
        }
    }

    /// Takes effect from the next message.
    pub fn set_mode(&mut self, mode: Mode) {
        if let Ok(mut st) = self.st.lock() {
            st.mode = mode;
        }
    }

    /// Takes effect from the next message.
    pub fn set_effort(&mut self, level: &str) {
        if let Ok(mut st) = self.st.lock() {
            st.effort = (!level.is_empty()).then(|| level.to_string());
        }
    }
}

/// A `turn/start`'s parameters.
fn turn(thread: &str, input: Value, st: &State) -> Value {
    let mut p = json!({"threadId": thread, "input": input, "approvalPolicy": policy(st.mode)});
    if let Some(e) = &st.effort {
        p["effort"] = json!(e);
    }
    p
}

/// The visible models of `model/list`'s pages, the server's default first
/// (the panel takes the first as the default when nothing else names one).
fn listed(pages: &[Value]) -> Vec<Model> {
    let mut v: Vec<&Value> = pages
        .iter()
        .filter_map(|p| p["data"].as_array())
        .flatten()
        .filter(|m| m["hidden"] != json!(true))
        .collect();
    v.sort_by_key(|m| m["isDefault"] != json!(true));
    v.into_iter()
        .filter_map(|m| {
            let id = m["id"].as_str().filter(|s| !s.is_empty())?;
            Some(Model {
                id: id.to_string(),
                name: m["displayName"].as_str().unwrap_or(id).to_string(),
                levels: super::effort::of_codex(m),
            })
        })
        .collect()
}

/// The parameters for the page after `page`, if there's one.
fn next_page(page: &Value) -> Option<Value> {
    let c = page["nextCursor"].as_str().filter(|c| !c.is_empty())?;
    Some(json!({"cursor": c}))
}

/// The most pages of models read.
const MAX_PAGES: usize = 20;

/// The account's models, asked of a short-lived `codex app-server` (the
/// model menu opened before any chat ran). Empty on any failure.
pub fn models(exe: std::path::PathBuf, cwd: std::path::PathBuf) -> Receiver<Vec<Model>> {
    let (tx, rx) = unbounded();
    let _ = std::thread::Builder::new().name("codex-models".into()).spawn(move || {
        let _ = tx.send(ask_models(&exe, &cwd, std::time::Duration::from_secs(20)));
    });
    rx
}

fn ask_models(exe: &std::path::Path, cwd: &std::path::Path, limit: std::time::Duration) -> Vec<Model> {
    // A new workspace has no scripts folder until something is saved.
    let _ = std::fs::create_dir_all(cwd);
    let mut cmd = Command::new(exe);
    super::accounts::apply(
        &mut cmd,
        super::Agent::Codex,
        &super::accounts::current(super::Agent::Codex),
    );
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let child = cmd
        .arg("app-server")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return Vec::new() };
    let (Some(mut sin), Some(sout)) = (child.stdin.take(), child.stdout.take()) else {
        return Vec::new();
    };
    // Read on a thread of its own, so a Codex that says nothing is given up
    // on (killing it ends the thread).
    let (lines_tx, lines) = unbounded();
    let _ = std::thread::Builder::new()
        .name("codex-models-read".into())
        .spawn(move || {
            for line in BufReader::new(sout).lines() {
                let Ok(line) = line else { break };
                if let Ok(v) = serde_json::from_str::<Value>(&line)
                    && lines_tx.send(v).is_err()
                {
                    break;
                }
            }
        });
    let mut say = |v: Value| {
        let _ = writeln!(sin, "{v}");
        let _ = sin.flush();
    };
    say(
        json!({"id": 1, "method": "initialize", "params": {"clientInfo": {"name": "opentern", "title": "Opentern", "version": env!("CARGO_PKG_VERSION")}}}),
    );
    let end = std::time::Instant::now() + limit;
    let mut pages: Vec<Value> = Vec::new();
    let mut whole = false;
    while let Ok(v) = lines.recv_deadline(end) {
        // Only replies: Codex's own requests have ids too.
        if v.get("method").is_some() {
            continue;
        }
        match v["id"].as_u64() {
            Some(_) if v.get("error").is_some() => break,
            Some(1) => {
                say(json!({"method": "initialized"}));
                say(json!({"id": 2, "method": "model/list", "params": {}}));
            }
            Some(id) if id == pages.len() as u64 + 2 => {
                pages.push(v["result"].clone());
                match next_page(&v["result"]) {
                    Some(params) if pages.len() < MAX_PAGES => {
                        say(json!({"id": pages.len() + 2, "method": "model/list", "params": params}));
                    }
                    _ => {
                        whole = true;
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    if whole { listed(&pages) } else { Vec::new() }
}

impl Drop for Codex {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Attached files as Codex inputs: images by path, small text files inline,
/// anything else named so Codex can read it.
fn inputs(files: &[String]) -> Vec<Value> {
    let mut out = Vec::new();
    for b in super::attach::blocks(files) {
        if b["type"] == "image" {
            continue;
        }
        out.push(b);
    }
    // `blocks` names each image in a text block first; Codex reads it from disk.
    for f in files {
        if super::attach::is_image(f) {
            out.push(json!({"type": "localImage", "path": f}));
        }
    }
    out
}

fn text_of(items: &Value) -> String {
    items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `bash -lc 'ls -la'` (any shell, any path) → `ls -la`.
fn plain_command(c: &str) -> String {
    let mut parts = c.trim().splitn(3, ' ');
    let (Some(shell), Some(flag), Some(rest)) = (parts.next(), parts.next(), parts.next()) else {
        return c.to_string();
    };
    let name = shell.rsplit('/').next().unwrap_or(shell);
    if !name.ends_with("sh") || !matches!(flag, "-c" | "-lc") {
        return c.to_string();
    }
    let r = rest.trim();
    if r.len() >= 2 && r.starts_with('\'') && r.ends_with('\'') {
        return r[1..r.len() - 1].to_string();
    }
    if r.len() >= 2 && r.starts_with('"') && r.ends_with('"') {
        return r[1..r.len() - 1].replace("\\\"", "\"").replace("\\$", "$");
    }
    r.to_string()
}

/// A file change as `+ ` / `- ` lines for the approval card.
fn diff_of(changes: &Value) -> String {
    let mut out = Vec::new();
    for c in changes.as_array().into_iter().flatten() {
        let path = c["path"].as_str().unwrap_or("");
        let diff = c["diff"].as_str().unwrap_or("");
        if changes.as_array().is_some_and(|a| a.len() > 1) {
            out.push(format!("  {}", super::file_name(path)));
        }
        let add = c["kind"]["type"] == "add";
        for l in diff.lines() {
            if add {
                out.push(format!("+ {l}"));
            } else if l.starts_with("+++") || l.starts_with("---") || l.starts_with("@@") {
                continue;
            } else if let Some(x) = l.strip_prefix('+') {
                out.push(format!("+ {x}"));
            } else if let Some(x) = l.strip_prefix('-') {
                out.push(format!("- {x}"));
            } else {
                out.push(format!("  {}", l.strip_prefix(' ').unwrap_or(l)));
            }
        }
    }
    let n = out.len();
    out.truncate(40);
    if n > 40 {
        out.push(format!("  … {} more lines", n - 40));
    }
    out.join("\n")
}

/// A tool step for one of Codex's items (`None`: not shown as a step).
fn step(item: &Value, st: &mut State) -> Option<(String, Value)> {
    let id = item["id"].as_str().unwrap_or("").to_string();
    Some(match item["type"].as_str()? {
        "commandExecution" => {
            let c = plain_command(item["command"].as_str().unwrap_or(""));
            st.commands.insert(id, c.clone());
            ("Bash".into(), json!({"command": c}))
        }
        "fileChange" => {
            let ch = item["changes"].clone();
            st.changes.insert(id, ch.clone());
            let first = ch[0]["path"].as_str().unwrap_or("").to_string();
            let adds = ch
                .as_array()
                .is_some_and(|a| a.iter().all(|c| c["kind"]["type"] == "add"));
            (if adds { "Write" } else { "Edit" }.into(), json!({"file_path": first}))
        }
        "dynamicToolCall" => (
            format!("mcp__opentern__{}", item["tool"].as_str().unwrap_or("")),
            item["arguments"].clone(),
        ),
        "mcpToolCall" => (
            format!(
                "mcp__{}__{}",
                item["server"].as_str().unwrap_or(""),
                item["tool"].as_str().unwrap_or("")
            ),
            item["arguments"].clone(),
        ),
        "webSearch" => ("WebSearch".into(), json!({"query": item["query"]})),
        _ => return None,
    })
}

/// What a finished item returned, and whether it failed.
fn outcome(item: &Value) -> Option<(String, bool)> {
    let failed = matches!(item["status"].as_str(), Some("failed" | "declined"));
    Some(match item["type"].as_str()? {
        "commandExecution" => (
            item["aggregatedOutput"].as_str().unwrap_or("").to_string(),
            failed || item["exitCode"].as_i64().is_some_and(|c| c != 0),
        ),
        "fileChange" => (
            item["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["path"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            failed,
        ),
        "dynamicToolCall" => (
            text_of(&item["contentItems"]),
            failed || item["success"] == json!(false),
        ),
        "mcpToolCall" => match item.get("error").filter(|e| !e.is_null()) {
            Some(e) => (e.to_string(), true),
            None => (text_of(&item["result"]["content"]), failed),
        },
        "webSearch" => (String::new(), failed),
        _ => return None,
    })
}

fn usage_of(v: &Value) -> Usage {
    let n = |k: &str| v[k].as_u64().unwrap_or(0);
    Usage {
        input: n("inputTokens").saturating_sub(n("cachedInputTokens")),
        output: n("outputTokens"),
        cache_read: n("cachedInputTokens"),
        cache_write: 0,
    }
}

fn minus(a: &Usage, b: &Usage) -> Usage {
    Usage {
        input: a.input.saturating_sub(b.input),
        output: a.output.saturating_sub(b.output),
        cache_read: a.cache_read.saturating_sub(b.cache_read),
        cache_write: 0,
    }
}

fn read(
    out: impl Read,
    stdin: Arc<Mutex<ChildStdin>>,
    st: Arc<Mutex<State>>,
    tx: Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
    err_text: Arc<Mutex<String>>,
) {
    for line in BufReader::new(out).lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let evs = {
            let Ok(mut s) = st.lock() else { break };
            handle(&v, &stdin, &mut s)
        };
        let mut any = false;
        for e in evs {
            any |= tx.send(e).is_ok();
        }
        if any {
            wake();
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(100));
    let err = err_text.lock().map(|e| e.trim().to_string()).unwrap_or_default();
    let lines: Vec<&str> = err.lines().filter(|l| !l.trim().is_empty()).collect();
    let _ = tx.send(Event::Exited(lines[lines.len().saturating_sub(6)..].join("\n")));
    wake();
}

/// One message from Codex: what it means to the panel, answering what the
/// protocol needs answered on the way.
fn handle(v: &Value, stdin: &Mutex<ChildStdin>, st: &mut State) -> Vec<Event> {
    let mut out = Vec::new();
    let method = v["method"].as_str().unwrap_or("");
    let p = &v["params"];
    // Replies to our requests.
    if method.is_empty() {
        let Some(id) = v["id"].as_u64() else { return out };
        let what = st.asked.remove(&id).unwrap_or("");
        let err = v
            .get("error")
            .map(|e| e["message"].as_str().unwrap_or("error").to_string());
        match (what, err) {
            ("init", None) => {
                write(stdin, &json!({"method": "initialized"}));
                request(stdin, st, "models", "model/list", json!({}));
                if let Some((start, resume)) = st.launch.clone() {
                    match resume {
                        Some(thread) => {
                            let mut p = start.clone();
                            if let Some(o) = p.as_object_mut() {
                                o.remove("dynamicTools");
                            }
                            p["threadId"] = json!(thread);
                            request(stdin, st, "resume", "thread/resume", p);
                        }
                        None => request(stdin, st, "start", "thread/start", start),
                    }
                }
            }
            ("start" | "resume", None) => {
                let thread = v["result"]["thread"]["id"].as_str().unwrap_or("").to_string();
                st.thread = Some(thread.clone());
                out.push(Event::Init {
                    session: thread.clone(),
                    model: v["result"]["model"].as_str().unwrap_or("").to_string(),
                });
                // What it runs at by itself.
                if st.effort.is_none()
                    && let (Some(model), Some(level)) =
                        (v["result"]["model"].as_str(), v["result"]["reasoningEffort"].as_str())
                {
                    out.push(Event::Effort {
                        model: model.to_string(),
                        level: level.to_string(),
                    });
                }
                for input in std::mem::take(&mut st.queued) {
                    let p = turn(&thread, input, st);
                    request(stdin, st, "turn", "turn/start", p);
                }
            }
            // An old thread that's gone: start a new one.
            ("resume", Some(_)) => {
                if let Some((start, _)) = st.launch.clone() {
                    request(stdin, st, "start", "thread/start", start);
                }
            }
            ("models", None) => {
                st.pages.push(v["result"].clone());
                match next_page(&v["result"]) {
                    Some(params) if st.pages.len() < MAX_PAGES => {
                        request(stdin, st, "models", "model/list", params);
                    }
                    _ => {
                        let list = listed(&std::mem::take(&mut st.pages));
                        if !list.is_empty() {
                            out.push(Event::Models(list));
                        }
                    }
                }
            }
            ("init" | "start" | "turn", Some(e)) => {
                st.queued.clear();
                out.push(Event::Done {
                    usage: Usage::default(),
                    cost: 0.0,
                    error: true,
                    message: format!("Codex couldn't start: {e}"),
                });
            }
            _ => {}
        }
        return out;
    }
    // Requests from Codex.
    if let Some(id) = v.get("id").filter(|i| !i.is_null()) {
        let key = format!("cx-{id}");
        match method {
            "item/tool/call" => {
                st.open.insert(key.clone(), id.clone());
                out.push(Event::Mcp {
                    request: key.clone(),
                    message: json!({
                        "jsonrpc": "2.0",
                        "id": key,
                        "method": "tools/call",
                        "params": {"name": p["tool"], "arguments": p["arguments"]},
                    }),
                });
            }
            "item/commandExecution/requestApproval" | "execCommandApproval" => {
                st.open.insert(key.clone(), id.clone());
                let item = p["itemId"].as_str().unwrap_or("");
                let command = p["command"]
                    .as_str()
                    .map(plain_command)
                    .or_else(|| st.commands.get(item).cloned())
                    .unwrap_or_default();
                let reason = p["reason"].as_str().unwrap_or("").to_string();
                out.push(Event::Permission {
                    request: key,
                    tool: "Bash".into(),
                    input: json!({"command": command, "description": reason}),
                    description: reason,
                    suggestions: json!(["session"]),
                });
            }
            "item/fileChange/requestApproval" | "applyPatchApproval" => {
                st.open.insert(key.clone(), id.clone());
                let ch = st
                    .changes
                    .get(p["itemId"].as_str().unwrap_or(""))
                    .cloned()
                    .unwrap_or(Value::Null);
                let adds = ch
                    .as_array()
                    .is_some_and(|a| a.iter().all(|c| c["kind"]["type"] == "add"));
                out.push(Event::Permission {
                    request: key,
                    tool: if adds { "Write" } else { "Edit" }.into(),
                    input: json!({"file_path": ch[0]["path"], "diff": diff_of(&ch)}),
                    description: p["reason"].as_str().unwrap_or("").to_string(),
                    suggestions: json!(["session"]),
                });
            }
            "currentTime/read" => {
                let now = jiff::Timestamp::now().as_second();
                write(stdin, &json!({"id": id, "result": {"currentTimeAt": now}}));
            }
            _ => write(
                stdin,
                &json!({"id": id, "error": {"code": -32601, "message": "Opentern doesn't support this."}}),
            ),
        }
        return out;
    }
    // Notifications.
    match method {
        "turn/started" => {
            st.turn = p["turn"]["id"].as_str().map(str::to_string);
            st.base = st.total.clone();
            st.error = None;
        }
        "item/started" => {
            let item = &p["item"];
            match item["type"].as_str() {
                Some("agentMessage") => out.push(Event::TextStart),
                Some("reasoning") => out.push(Event::Thinking),
                _ => {
                    if let Some((name, input)) = step(item, st) {
                        out.push(Event::ToolUse {
                            id: item["id"].as_str().unwrap_or("").to_string(),
                            name,
                            input,
                        });
                    }
                }
            }
        }
        "item/agentMessage/delta" => {
            st.streamed.insert(p["itemId"].as_str().unwrap_or("").to_string());
            out.push(Event::Text(p["delta"].as_str().unwrap_or("").to_string()));
        }
        "item/completed" => {
            let item = &p["item"];
            let id = item["id"].as_str().unwrap_or("").to_string();
            if item["type"] == "agentMessage" {
                if !st.streamed.remove(&id) {
                    out.push(Event::Text(item["text"].as_str().unwrap_or("").to_string()));
                }
            } else if let Some((text, error)) = outcome(item) {
                out.push(Event::ToolResult { id, text, error });
            }
        }
        "thread/tokenUsage/updated" => {
            let u = &p["tokenUsage"];
            st.total = usage_of(&u["total"]);
            out.push(Event::Request(usage_of(&u["last"])));
        }
        "account/rateLimits/updated" => {
            let r = &p["rateLimits"];
            let ws: Vec<Window> = ["primary", "secondary"]
                .iter()
                .filter_map(|k| r.get(*k).filter(|w| w.is_object()))
                .map(|w| Window {
                    name: format!("w{}", w["windowDurationMins"].as_i64().unwrap_or(0)),
                    used: w["usedPercent"].as_f64().unwrap_or(0.0) / 100.0,
                    resets: w["resetsAt"].as_i64().unwrap_or(0),
                })
                .collect();
            if !ws.is_empty() {
                out.push(Event::Limits(ws));
            }
        }
        "error" if p["willRetry"] != json!(true) => {
            st.error = p["error"]["message"].as_str().map(str::to_string);
        }
        "turn/completed" => {
            let turn = &p["turn"];
            st.turn = None;
            let (error, message) = match turn["status"].as_str() {
                Some("interrupted") => (true, "Stopped.".to_string()),
                Some("failed") => (
                    true,
                    turn["error"]["message"]
                        .as_str()
                        .map(str::to_string)
                        .or(st.error.take())
                        .unwrap_or_else(|| "Codex couldn't finish.".into()),
                ),
                _ => (false, String::new()),
            };
            out.push(Event::Done {
                usage: minus(&st.total, &st.base),
                cost: 0.0,
                error,
                message,
            });
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_commands_and_diffs() {
        assert_eq!(plain_command("/bin/bash -lc 'ls -la'"), "ls -la");
        assert_eq!(plain_command("rg foo"), "rg foo");
        assert_eq!(plain_command("/usr/bin/zsh -lc pwd"), "pwd");
        assert_eq!(
            plain_command(r#"/usr/bin/zsh -lc "echo \"hi\" > a.pine""#),
            r#"echo "hi" > a.pine"#
        );
        let ch = json!([{"path": "/s/a.pine", "kind": {"type": "update"}, "diff": "@@ -1 +1 @@\n-a\n+b\n c"}]);
        assert_eq!(diff_of(&ch), "- a\n+ b\n  c");
        let ch = json!([{"path": "/s/hi.txt", "kind": {"type": "add"}, "diff": "hi\n"}]);
        assert_eq!(diff_of(&ch), "+ hi");
    }

    #[test]
    fn a_picked_effort_goes_with_each_turn() {
        let mut st = State::default();
        assert!(turn("t", json!([]), &st).get("effort").is_none());
        st.effort = Some("high".into());
        assert_eq!(turn("t", json!([]), &st)["effort"], "high");
    }

    /// A stand-in `codex` whose replies are `script`'s.
    #[cfg(unix)]
    fn fake(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let exe = dir.join("codex");
        let src = format!(
            "#!/usr/bin/env python3\nimport json, sys, time\ndef say(x): print(json.dumps(x), flush=True)\nfor line in sys.stdin:\n    q = json.loads(line)\n    i, m, p = q.get('id'), q.get('method'), q.get('params') or {{}}\n{body}"
        );
        std::fs::write(&exe, src).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        exe
    }

    #[cfg(unix)]
    fn ask(body: &str, secs: u64) -> Vec<(String, String)> {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("opentern-codex-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = fake(&dir, body);
        let list = ask_models(&exe, &dir, std::time::Duration::from_secs(secs));
        let _ = std::fs::remove_dir_all(&dir);
        list.into_iter().map(|m| (m.id, m.name)).collect()
    }

    /// The installed Codex's own list (no model request is made).
    #[test]
    #[ignore = "needs Codex, signed in"]
    fn live_models() {
        let exe = super::super::Agent::Codex.find().expect("codex");
        let list = ask_models(&exe, &std::env::temp_dir(), std::time::Duration::from_secs(20));
        eprintln!("{list:?}");
        assert!(!list.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn every_page_of_models_is_read() {
        let list = ask(
            "    if m == 'initialize':\n        say({'method': 'note', 'params': {}})\n        say({'id': 2, 'method': 'a/request', 'params': {}})\n        say({'id': i, 'result': {}})\n    elif m == 'model/list' and 'cursor' not in p:\n        say({'id': i, 'result': {'data': [{'id': 'old', 'displayName': 'Old'}, {'id': 'secret', 'hidden': True}], 'nextCursor': 'c2'}})\n    elif m == 'model/list':\n        say({'id': i, 'result': {'data': [{'id': 'new', 'displayName': 'New', 'isDefault': True}, {'id': 'plain'}], 'nextCursor': None}})\n",
            20,
        );
        let s = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(list, [s("new", "New"), s("old", "Old"), s("plain", "plain")]);
    }

    #[cfg(unix)]
    #[test]
    fn a_codex_that_fails_or_hangs_gives_no_models() {
        let t = std::time::Instant::now();
        assert!(ask("    pass\n", 1).is_empty());
        assert!(t.elapsed().as_secs() < 10);
        assert!(ask("    say({'id': i, 'error': {'message': 'signed out'}})\n", 20).is_empty());
        assert!(ask("    sys.exit(1)\n", 20).is_empty());
        // Half a list isn't one.
        assert!(
            ask(
                "    if m == 'initialize': say({'id': i, 'result': {}})\n    elif 'cursor' not in p and m == 'model/list': say({'id': i, 'result': {'data': [{'id': 'a'}], 'nextCursor': 'x'}})\n    elif m == 'model/list': say({'id': i, 'error': {'message': 'no'}})\n",
                20
            )
            .is_empty()
        );
    }
}

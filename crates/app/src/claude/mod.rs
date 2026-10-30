//! The agent panel: chats with a coding agent the user has installed and
//! signed in to (Claude Code or Codex, `agent.rs`), next to the chart. The
//! agent works in the scripts folder and drives the chart through the app's
//! own tools (`tools.rs`); the protocols are in `process.rs` (Claude Code)
//! and `codex.rs`.
//!
//! Nothing runs until the user sends a message: no process, no network.

pub mod accounts;
pub mod agent;
pub mod attach;
pub mod auth;
pub mod codex;
pub mod control;
pub mod mark;
pub mod markets;
pub mod process;
pub mod tools;

use crate::app::App;
use crate::icons::ph;
use crate::theme::{self, Theme};
use crate::widgets as w;
pub use agent::Agent;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, CornerRadius, FontId, Layout, Margin, RichText, Sense, Stroke, Ui, vec2};
use process::{Client, Event, Launch, Mode, Usage, Window};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AskState {
    Waiting,
    Allowed,
    Denied,
    /// The turn ended (or the app restarted) before an answer.
    Expired,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Item {
    User(String),
    /// Files sent with the next `User` message (full paths).
    Attached(Vec<String>),
    Assistant(String),
    Tool {
        id: String,
        /// The tool's name (for its icon).
        #[serde(default)]
        tool: String,
        label: String,
        result: Option<String>,
        error: bool,
    },
    Ask {
        request: String,
        #[serde(default)]
        tool: String,
        label: String,
        detail: String,
        input: Value,
        suggestions: Value,
        state: AskState,
    },
    Note {
        text: String,
        error: bool,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chat {
    pub id: u64,
    /// Who the chat is with (chats saved before Codex are Claude's).
    #[serde(default)]
    pub agent: Agent,
    pub title: String,
    /// Claude Code's session id (to resume it).
    pub session: Option<String>,
    pub created: i64,
    pub updated: i64,
    pub mode: Mode,
    /// A model alias, or empty for Claude Code's default.
    pub model: String,
    /// The model Claude Code reported.
    #[serde(default)]
    pub model_name: String,
    pub items: Vec<Item>,
    pub usage: Usage,
    pub cost: f64,
    /// The latest request's context size, in tokens.
    pub context: u64,
    /// The account it runs as (`accounts.rs`); chats saved before accounts
    /// ran as the app's launch account.
    #[serde(default)]
    pub account: Option<String>,
    #[serde(skip)]
    pub busy: bool,
    #[serde(skip)]
    pub thinking: bool,
}

impl Chat {
    fn new(id: u64, agent: Agent, mode: Mode, model: &str) -> Chat {
        let now = now_ms();
        Chat {
            id,
            agent,
            title: "New chat".into(),
            session: None,
            created: now,
            updated: now,
            mode,
            model: model.to_string(),
            model_name: String::new(),
            items: Vec::new(),
            usage: Usage::default(),
            cost: 0.0,
            context: 0,
            account: Some(accounts::current(agent)),
            busy: false,
            thinking: false,
        }
    }
}

/// What's saved to `<workspace>/claude/chats.json`.
#[derive(Default, Serialize, Deserialize)]
struct Saved {
    chats: Vec<Chat>,
    active: Option<u64>,
    mode: Mode,
    model: String,
    #[serde(default)]
    limits: Vec<Window>,
    #[serde(default)]
    limits_at: i64,
    #[serde(default)]
    show_usage: bool,
    /// Whether the mode was picked by hand (else Auto, the default).
    #[serde(default)]
    mode_picked: bool,
    /// The agent picked on the panel (none yet: the picker shows).
    #[serde(default)]
    agent: Option<Agent>,
    #[serde(default)]
    codex_model: String,
    #[serde(default)]
    codex_models: Vec<(String, String)>,
    #[serde(default)]
    codex_limits: Vec<Window>,
    /// Each agent's open chat, for switching back.
    #[serde(default)]
    last: Vec<(Agent, u64)>,
    /// The model each agent ran when none was picked, as last reported.
    #[serde(default)]
    defaults: Vec<(Agent, String)>,
}

/// The move from the picker (or the other agent) into an agent's panel.
#[derive(Clone, Copy, Debug)]
pub struct Anim {
    pub to: Agent,
    pub start: f64,
    /// Picked on the picker: where the card and its logo were.
    pub from: Option<(egui::Rect, egui::Rect)>,
}

/// How long the move into an agent's panel takes.
const ANIM: f32 = 0.6;

/// What the panel shows: a chat, or the list of chats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Chat,
    Chats,
}

#[derive(Default)]
pub struct Claude {
    /// The agent the panel shows; `None` until one is picked.
    pub agent: Option<Agent>,
    pub anim: Option<Anim>,
    /// The picker's cards, last frame (the move starts from them).
    cards: Vec<(Agent, egui::Rect, egui::Rect)>,
    /// Where the header's logo was last frame (the move ends there).
    header_logo: Option<egui::Rect>,
    /// Each agent's open chat, for switching back.
    last: HashMap<Agent, u64>,
    pub codex_model: String,
    pub codex_models: Vec<(String, String)>,
    pub codex_limits: Vec<Window>,
    codex_cli: Option<Option<PathBuf>>,
    /// When the picker first showed (its entrance).
    picker_shown: Option<f64>,
    /// Codex's model list, while it's being fetched.
    models_rx: Option<crossbeam_channel::Receiver<Vec<(String, String)>>>,
    /// The model list was asked for while the menu is open.
    models_asked: bool,
    pub view: View,
    /// The chats list's search text.
    pub chat_query: String,
    /// A chat being renamed: (id, the new title so far).
    renaming: Option<(u64, String)>,
    /// Where the composer was last frame: menus open above it.
    composer: Option<egui::Rect>,
    /// `OPENTERN_STILL=claude-menu:<mode|model|usage>`: open it (screenshots).
    pub still_menu: Option<String>,
    /// Where the composer's controls landed last frame (tests check they
    /// don't overlap).
    pub footer: Vec<egui::Rect>,
    /// Files waiting to go with the next message (full paths).
    pub attached: Vec<String>,
    /// The file picker, while it's open.
    picker: Option<crossbeam_channel::Receiver<Vec<PathBuf>>>,
    /// A short note under the composer (a paste that found nothing).
    attach_note: Option<(String, f64)>,
    pub chats: Vec<Chat>,
    pub active: Option<u64>,
    pub clients: HashMap<u64, Client>,
    pub pending: Vec<tools::Pending>,
    pub search_seq: u64,
    /// Tool calls that answer once something is done (`control.rs`).
    pub waiting: Vec<control::Waiting>,
    pub batches: HashMap<u64, control::Batch>,
    pub batch_seq: u64,
    /// The `snapshot` tool's picture (PNG, base64) once it's taken.
    pub picture: Option<String>,
    /// Tests: tool answers that had no running CLI to go to.
    #[cfg(test)]
    pub answers: Vec<Value>,
    /// The account's usage windows, from the latest reply.
    pub limits: Vec<Window>,
    pub limits_at: i64,
    pub input: String,
    /// The mode and model new chats start with (the last ones picked).
    pub mode: Mode,
    pub model: String,
    pub show_usage: bool,
    mode_picked: bool,
    /// Tool rows opened to show their result: (chat, item index).
    expanded: HashSet<(u64, usize)>,
    /// Step boxes opened or folded by hand: (chat, first item) → open.
    folds: HashMap<(u64, usize), bool>,
    /// The scripts folder's files, for links (looked at every 2 s).
    scripts: Option<(std::time::Instant, Vec<String>)>,
    cli: Option<Option<PathBuf>>,
    /// The model each agent runs when none is picked: what its own config
    /// names, read at load (`config_model`).
    configured: HashMap<Agent, String>,
    /// The same, as a chat last reported it (Init), for when the config
    /// doesn't say.
    defaults: HashMap<Agent, String>,
    /// Each agent's sign-in, checked when its panel first shows.
    auth: HashMap<Agent, auth::Auth>,
    loaded: bool,
    dirty: bool,
}

/// The models to pick from: (id passed to `--model`, name). Exact ids, so
/// the names stay true when newer models come out.
pub const MODELS: [(&str, &str); 5] = [
    ("", "Default"),
    ("claude-fable-5-1", "Fable 5.1"),
    ("claude-opus-5-5", "Opus 5.5"),
    ("claude-sonnet-5-5", "Sonnet 5.5"),
    ("claude-haiku-5-5", "Haiku 5.5"),
];

/// The exact id for an alias saved by an older version ("haiku").
pub fn model_id(m: &str) -> String {
    MODELS
        .iter()
        .find(|(k, _)| !k.is_empty() && k.split('-').nth(1) == Some(m))
        .map(|(k, _)| k.to_string())
        .unwrap_or_else(|| m.to_string())
}

/// A model id or alias as people say it: `claude-opus-5-5` → "Opus 5.5".
pub fn model_name(id: &str) -> String {
    if let Some((_, l)) = MODELS.iter().find(|(k, _)| *k == id && !k.is_empty()) {
        return l.to_string();
    }
    let core = id.strip_prefix("claude-").unwrap_or(id);
    let core = core.split('[').next().unwrap_or(core);
    let mut words: Vec<String> = Vec::new();
    let mut nums: Vec<&str> = Vec::new();
    for part in core.split('-') {
        if part.chars().all(|c| c.is_ascii_digit()) {
            // Version parts; a long one is a date stamp.
            if part.len() <= 2 {
                nums.push(part);
            }
        } else {
            let mut c = part.chars();
            words.push(
                c.next()
                    .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                    .unwrap_or_default(),
            );
        }
    }
    let mut s = words.join(" ");
    if !nums.is_empty() {
        s.push(' ');
        s.push_str(&nums.join("."));
    }
    s
}

fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

impl Claude {
    /// Whether the open chat is waiting for a permission answer.
    pub fn waiting(&self) -> bool {
        self.view == View::Chat
            && self.chat().is_some_and(|c| {
                c.items.iter().any(|i| {
                    matches!(
                        i,
                        Item::Ask {
                            state: AskState::Waiting,
                            ..
                        }
                    )
                })
            })
    }

    pub fn chat(&self) -> Option<&Chat> {
        self.chats.iter().find(|c| Some(c.id) == self.active)
    }

    /// The agent the panel shows (Claude until one is picked).
    pub fn current(&self) -> Agent {
        self.agent.unwrap_or_default()
    }

    /// The model new chats with `a` start on.
    fn model_for(&self, a: Agent) -> &str {
        match a {
            Agent::Claude => &self.model,
            Agent::Codex => &self.codex_model,
        }
    }

    fn limits_for(&self, a: Agent) -> &[Window] {
        match a {
            Agent::Claude => &self.limits,
            Agent::Codex => &self.codex_limits,
        }
    }

    /// A model id as people say it.
    fn model_label(&self, a: Agent, id: &str) -> String {
        match a {
            Agent::Claude => model_name(id),
            Agent::Codex => self
                .codex_models
                .iter()
                .find(|(k, _)| k == id)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| codex_name(id)),
        }
    }

    /// The model `a` runs when none is picked: its config, else what it
    /// last reported, else (Codex) the account's default.
    fn default_model(&self, a: Agent) -> Option<String> {
        self.configured
            .get(&a)
            .or_else(|| self.defaults.get(&a))
            .cloned()
            .or_else(|| match a {
                Agent::Codex => self.codex_models.first().map(|(k, _)| k.clone()),
                Agent::Claude => None,
            })
    }

    /// The name of the model a chat on `model` runs ("" = the default).
    fn current_label(&self, a: Agent, model: &str) -> String {
        if !model.is_empty() {
            return self.model_label(a, model);
        }
        match self.default_model(a) {
            Some(m) => self.model_label(a, &m),
            None => "Default".to_string(),
        }
    }

    /// The models to pick from for `a`: (id, name). The default is listed
    /// by its name once it's known, else as "Default" first.
    fn models_for(&self, a: Agent) -> Vec<(String, String)> {
        let known = self.default_model(a);
        let mut v = Vec::new();
        if known.is_none() {
            v.push((String::new(), "Default".to_string()));
        }
        match a {
            Agent::Claude => v.extend(
                MODELS
                    .iter()
                    .filter(|(k, _)| !k.is_empty())
                    .map(|(k, l)| (k.to_string(), l.to_string())),
            ),
            Agent::Codex => v.extend(self.codex_models.iter().cloned()),
        }
        if let Some(m) = known
            && !v.iter().any(|(k, _)| *k == m)
        {
            v.insert(0, (m.clone(), self.model_label(a, &m)));
        }
        v
    }

    fn chat_mut(&mut self, id: u64) -> Option<&mut Chat> {
        self.chats.iter_mut().find(|c| c.id == id)
    }

    fn cli_for(&mut self, a: Agent) -> Option<PathBuf> {
        match a {
            Agent::Claude => self.cli.get_or_insert_with(|| a.find()).clone(),
            Agent::Codex => self.codex_cli.get_or_insert_with(|| a.find()).clone(),
        }
    }
}

impl App {
    fn claude_file(&self) -> PathBuf {
        self.store.dir().join("claude").join("chats.json")
    }

    pub(crate) fn claude_load(&mut self) {
        if self.claude.loaded {
            return;
        }
        self.claude.loaded = true;
        for a in Agent::ALL {
            if let Some(d) = self.store.value(&account_key(a)) {
                accounts::set_current(a, &d);
            }
            if let Some(m) = config_model(a) {
                self.claude.configured.insert(a, m);
            }
        }
        let Ok(text) = std::fs::read_to_string(self.claude_file()) else {
            return;
        };
        let Ok(mut s) = serde_json::from_str::<Saved>(&text) else {
            return;
        };
        for c in &mut s.chats {
            for it in &mut c.items {
                if let Item::Ask { state, .. } = it
                    && *state == AskState::Waiting
                {
                    *state = AskState::Expired;
                }
            }
        }
        let c = &mut self.claude;
        c.chats = s.chats;
        c.active = s.active.filter(|a| c.chats.iter().any(|x| x.id == *a));
        // Auto is the default, as in the user's terminal; a mode stays only
        // once it's been picked by hand (earlier versions saved the old
        // default, Ask first, for every chat).
        c.mode_picked = s.mode_picked;
        c.mode = if s.mode_picked { s.mode } else { Mode::Auto };
        if !s.mode_picked {
            for chat in &mut c.chats {
                chat.mode = Mode::Auto;
            }
        }
        c.model = model_id(&s.model);
        for chat in &mut c.chats {
            chat.model = model_id(&chat.model);
        }
        c.limits = s.limits;
        c.limits_at = s.limits_at;
        c.show_usage = s.show_usage;
        c.agent = s.agent;
        c.codex_model = s.codex_model;
        c.codex_models = s.codex_models;
        c.codex_limits = s.codex_limits;
        c.last = s.last.into_iter().collect();
        c.defaults = s.defaults.into_iter().collect();
        // The open chat belongs to the agent on show.
        if let Some(a) = c.agent
            && c.chat().is_some_and(|x| x.agent != a)
        {
            c.active = c.last.get(&a).copied().filter(|id| c.chats.iter().any(|x| x.id == *id));
        }
    }

    fn claude_save(&mut self) {
        // Never write over chats that haven't been read yet.
        self.claude_load();
        let c = &self.claude;
        let s = Saved {
            chats: c.chats.clone(),
            active: c.active,
            mode: c.mode,
            model: c.model.clone(),
            limits: c.limits.clone(),
            limits_at: c.limits_at,
            show_usage: c.show_usage,
            mode_picked: c.mode_picked,
            agent: c.agent,
            codex_model: c.codex_model.clone(),
            codex_models: c.codex_models.clone(),
            codex_limits: c.codex_limits.clone(),
            last: c.last.iter().map(|(a, id)| (*a, *id)).collect(),
            defaults: c.defaults.iter().map(|(a, m)| (*a, m.clone())).collect(),
        };
        let path = self.claude_file();
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(j) = serde_json::to_string(&s) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, j).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
        self.claude.dirty = false;
    }

    fn new_chat(&mut self) -> u64 {
        self.claude_load();
        let id = self.claude.chats.iter().map(|c| c.id).max().unwrap_or(0) + 1;
        let agent = self.claude.current();
        let chat = Chat::new(id, agent, self.claude.mode, self.claude.model_for(agent));
        self.claude.chats.push(chat);
        self.claude.active = Some(id);
        self.claude.dirty = true;
        id
    }

    fn delete_chat(&mut self, id: u64) {
        self.claude.clients.remove(&id);
        self.claude.chats.retain(|c| c.id != id);
        self.claude.last.retain(|_, v| *v != id);
        if self.claude.active == Some(id) {
            let a = self.claude.current();
            self.claude.active = self
                .claude
                .chats
                .iter()
                .filter(|c| c.agent == a)
                .max_by_key(|c| c.updated)
                .map(|c| c.id);
        }
        self.claude.dirty = true;
    }

    /// Shows `to`'s panel, with the chat it had open (if any). `from` is
    /// the picker card it was picked from, for the move.
    pub(crate) fn claude_switch(&mut self, to: Agent, from: Option<(egui::Rect, egui::Rect)>, now: f64) {
        self.claude_load();
        let c = &mut self.claude;
        if let (Some(a), Some(id)) = (c.agent, c.active) {
            c.last.insert(a, id);
        }
        if c.agent != Some(to) || from.is_some() {
            c.anim = Some(Anim { to, start: now, from });
        }
        c.agent = Some(to);
        c.view = View::Chat;
        c.renaming = None;
        let keep = c.chat().is_some_and(|x| x.agent == to);
        if !keep {
            c.active = c
                .last
                .get(&to)
                .copied()
                .filter(|id| c.chats.iter().any(|x| x.id == *id && x.agent == to));
        }
        c.dirty = true;
    }

    /// What Claude sees before each message: the chart right now.
    fn claude_context(&self) -> String {
        let c = &self.chart;
        let mut s = format!(
            "<opentern_context>\nActive chart: {} ({}), {}, {}",
            c.symbol.id,
            c.symbol.description,
            c.tf.label(),
            c.style.kind.name().to_lowercase()
        );
        if let Some(b) = c.bars.last() {
            s.push_str(&format!(
                ", last price {}",
                crate::format::price(b.close, c.precision())
            ));
        }
        s.push_str(".\n");
        let on: Vec<String> = c
            .studies
            .iter()
            .map(|st| {
                let run = self.scripts.iter().find(|r| r.uid == Some(st.uid));
                match run {
                    Some(r) => match &r.error {
                        Some(e) => format!("{} ({}, error: {e})", r.file, r.kind),
                        None => format!("{} ({})", r.file, if r.kind.is_empty() { "script" } else { &r.kind }),
                    },
                    None => st.title(true),
                }
            })
            .collect();
        if !on.is_empty() {
            s.push_str(&format!("On the chart: {}.\n", on.join("; ")));
        }
        if let Some(d) = self.editor.doc().filter(|d| !d.readonly) {
            s.push_str(&format!("Open in the Script Editor: {}.\n", d.key));
        }
        s.push_str("</opentern_context>");
        s
    }

    fn claude_system(&self, agent: Agent) -> String {
        let p = match agent {
            Agent::Claude => "mcp__opentern__",
            Agent::Codex => "",
        };
        let text = format!(
            "You're running inside Opentern, a local market analysis terminal (TradingView-style charts, Pine Script \
             and other scripting languages, backtests, alerts and paper trading). The user talks to you from a \
             panel next to their chart.\n\n\
             Your working directory is the user's scripts folder ({}). Scripts there can go on the chart. \
             Pine scripts (.pine) follow TradingView's Pine Script v5/v6. Luau (.luau), JavaScript (.js), \
             TypeScript (.ts) and Python (.py) scripts use Opentern's own API: call mcp__opentern__scripting_guide \
             before writing one.\n\n\
             The mcp__opentern tools let you do what the user can do with clicks: see the chart (get_chart, \
             get_bars, snapshot for a picture), change it (open_symbol, set_timeframe, chart_view for chart type, \
             dates, scales, multi-chart layouts and compares), indicators and scripts (list_indicators, \
             add_indicator, indicator for inputs, add_script_to_chart, remove_from_chart, check_script), drawings \
             (drawings), bar replay (replay), watchlists, the screener, seasonality, custom indices \
             (custom_index), strategies (strategy_report, strategy_settings, strategy_lab), paper trading, \
             alerts, and the bottom panel's tabs (open_panel). \
             To show the user an example from history, find it with get_bars over a date range, then use \
             chart_view go_to, drawings and replay, and snapshot to check what they see. After you write or edit \
             a script, run check_script on it and fix every error before you reply. Scripts on the chart reload by \
             themselves when their file changes. When you create a script the user will want to see, add it \
             to the chart.\n\n\
             Strategies: write a Pine strategy() script (or a strategy in another language, see the guide), \
             check it, add it to the chart, then call strategy_report for its results (TradingView's metrics \
             and the latest trades). The report is for the symbol and timeframe on the chart; switch them with \
             open_symbol and set_timeframe to test elsewhere. Iterate on the script until the user is happy.\n\n\
             Each user message starts with an <opentern_context> block: the app wrote it, not the user, and it \
             describes the chart at that moment.\n\n\
             This is analysis and paper trading only. You can't place real orders; never say you did.",
            self.scripts_dir().display()
        );
        text.replace("mcp__opentern__", p)
            .replace("The mcp__opentern tools", "Opentern's tools")
    }

    /// Sends the input box's text in the active chat (starting one, and
    /// Claude Code, as needed).
    pub(crate) fn claude_send(&mut self) {
        self.claude_load();
        let text = self.claude.input.trim().to_string();
        if text.is_empty() && self.claude.attached.is_empty() {
            return;
        }
        let id = match self.claude.active {
            Some(id) if self.claude.chat().is_some() => id,
            _ => self.new_chat(),
        };
        let agent = self.claude.chat().map(|c| c.agent).unwrap_or_default();
        let Some(exe) = self.claude.cli_for(agent) else {
            return;
        };
        if self.claude.chat().is_some_and(|c| c.busy) {
            return;
        }
        if !self.claude.clients.contains_key(&id) {
            if crate::app::headless() && std::env::var_os(agent.env()).is_none() {
                return;
            }
            let chat = self.claude.chat().expect("active chat");
            let launch = Launch {
                agent,
                exe,
                cwd: self.scripts_dir(),
                mode: chat.mode,
                model: (!chat.model.is_empty()).then(|| chat.model.clone()),
                resume: chat.session.clone(),
                system: self.claude_system(agent),
                account: chat.account.clone().unwrap_or_else(|| accounts::launched(agent)),
            };
            let _ = std::fs::create_dir_all(&launch.cwd);
            let ctx = self.egui_ctx.clone();
            match Client::start(&launch, std::sync::Arc::new(move || ctx.request_repaint())) {
                Ok(c) => {
                    self.claude.clients.insert(id, c);
                }
                Err(e) => {
                    if let Some(chat) = self.claude.chat_mut(id) {
                        chat.items.push(Item::Note {
                            text: format!("Couldn't start {}: {e}", agent.product()),
                            error: true,
                        });
                    }
                    return;
                }
            }
        }
        let context = self.claude_context();
        let files = std::mem::take(&mut self.claude.attached);
        if let Some(c) = self.claude.clients.get(&id) {
            c.send(&context, &files, &text);
        }
        crate::diag::crumb("claude send");
        let chat = self.claude.chat_mut(id).expect("active chat");
        if chat.items.is_empty() {
            chat.title = match files.first() {
                Some(f) if text.is_empty() => title_of(file_name(f)),
                _ => title_of(&text),
            };
        }
        if !files.is_empty() {
            chat.items.push(Item::Attached(files));
        }
        if !text.is_empty() {
            chat.items.push(Item::User(text));
        }
        chat.busy = true;
        chat.thinking = true;
        chat.updated = now_ms();
        self.claude.input.clear();
        self.claude.dirty = true;
    }

    /// Reads every running chat's events; answers the app's tool calls.
    pub(crate) fn poll_claude(&mut self) {
        self.poll_tool_waits();
        if self.claude.clients.is_empty() {
            if self.claude.dirty {
                self.claude_save();
            }
            return;
        }
        let mut evs: Vec<(u64, Event)> = Vec::new();
        for (id, c) in &self.claude.clients {
            evs.extend(c.events().try_iter().map(|e| (*id, e)));
        }
        for (id, e) in evs {
            self.claude_event(id, e);
        }
        // A chat in the background that's done its turn gives its process
        // back (sending again resumes it).
        let active = self.claude.active;
        let idle: Vec<u64> = self
            .claude
            .clients
            .keys()
            .copied()
            .filter(|id| Some(*id) != active && !self.claude.chats.iter().any(|c| c.id == *id && c.busy))
            .collect();
        for id in idle {
            self.claude.clients.remove(&id);
        }
        if self.claude.dirty && !self.claude.chats.iter().any(|c| c.busy) {
            self.claude_save();
        }
    }

    fn claude_event(&mut self, id: u64, e: Event) {
        if let Event::Mcp { request, message } = &e {
            if let Some(reply) = self.claude_mcp(id, request, message)
                && let Some(c) = self.claude.clients.get(&id)
            {
                c.mcp_reply(request, reply);
            }
            return;
        }
        let agent = self
            .claude
            .chats
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.agent)
            .unwrap_or_default();
        if let Event::Limits(ws) = e {
            match agent {
                Agent::Claude => self.claude.limits = ws,
                Agent::Codex => self.claude.codex_limits = ws,
            }
            self.claude.limits_at = now_ms();
            self.claude.dirty = true;
            return;
        }
        if let Event::Models(list) = e {
            self.claude.codex_models = list;
            self.claude.dirty = true;
            return;
        }
        let mut exited = false;
        let Some(chat) = self.claude.chat_mut(id) else { return };
        match e {
            Event::Init { session, model } => {
                chat.session = Some(session);
                if chat.model.is_empty() && !model.is_empty() {
                    let a = chat.agent;
                    self.claude.defaults.insert(a, model.clone());
                }
                let Some(chat) = self.claude.chat_mut(id) else { return };
                chat.model_name = model;
            }
            Event::TextStart => {
                chat.thinking = false;
                if !matches!(chat.items.last(), Some(Item::Assistant(t)) if t.is_empty()) {
                    chat.items.push(Item::Assistant(String::new()));
                }
            }
            Event::Text(t) => {
                chat.thinking = false;
                match chat.items.last_mut() {
                    Some(Item::Assistant(s)) => s.push_str(&t),
                    _ => chat.items.push(Item::Assistant(t)),
                }
            }
            Event::Thinking => chat.thinking = true,
            Event::ToolUse { id: tid, name, input } => {
                chat.thinking = false;
                if let Some(label) = tool_label(&name, &input, false) {
                    chat.items.push(Item::Tool {
                        id: tid,
                        tool: name,
                        label,
                        result: None,
                        error: false,
                    });
                }
            }
            Event::ToolResult { id: tid, text, error } => {
                chat.thinking = true;
                for it in chat.items.iter_mut().rev() {
                    if let Item::Tool {
                        id: i,
                        result,
                        error: err,
                        ..
                    } = it
                        && *i == tid
                    {
                        *result = Some(text);
                        *err = error;
                        break;
                    }
                }
            }
            Event::Request(u) => chat.context = u.context(),
            Event::Done {
                usage,
                cost,
                error,
                message,
            } => {
                chat.usage.add(&usage);
                chat.cost += cost;
                chat.busy = false;
                chat.thinking = false;
                chat.updated = now_ms();
                if error {
                    chat.items.push(Item::Note {
                        text: message,
                        error: true,
                    });
                }
                expire_asks(chat);
                drop_empty(chat);
            }
            Event::Permission {
                request,
                tool,
                input,
                description,
                suggestions,
            } => {
                chat.thinking = false;
                let label = tool_label(&tool, &input, true).unwrap_or(tool.clone());
                let detail = ask_detail(&tool, &input, &description);
                chat.items.push(Item::Ask {
                    request,
                    tool,
                    label,
                    detail,
                    input,
                    suggestions,
                    state: AskState::Waiting,
                });
            }
            Event::Cancel(r) => {
                for it in &mut chat.items {
                    if let Item::Ask { request, state, .. } = it
                        && *request == r
                        && *state == AskState::Waiting
                    {
                        *state = AskState::Expired;
                    }
                }
            }
            Event::Exited(err) => {
                exited = true;
                if chat.busy {
                    chat.items.push(Item::Note {
                        text: exit_message(agent, &err),
                        error: true,
                    });
                }
                chat.busy = false;
                chat.thinking = false;
                expire_asks(chat);
                drop_empty(chat);
            }
            Event::Mcp { .. } | Event::Limits(_) | Event::Models(_) => {}
        }
        self.claude.dirty = true;
        if exited {
            self.claude.clients.remove(&id);
            // It may have stopped because the user signed out: ask again.
            self.claude.auth.remove(&agent);
        }
    }

    fn claude_answer(&mut self, chat: u64, index: usize, allow: bool, always: bool) {
        let Some(c) = self.claude.chats.iter_mut().find(|c| c.id == chat) else {
            return;
        };
        let Some(Item::Ask {
            request,
            input,
            suggestions,
            state,
            ..
        }) = c.items.get_mut(index)
        else {
            return;
        };
        if *state != AskState::Waiting {
            return;
        }
        if let Some(client) = self.claude.clients.get(&chat) {
            if allow {
                client.allow(request, input, always.then_some(&*suggestions));
            } else {
                client.deny(request);
            }
        }
        *state = if allow { AskState::Allowed } else { AskState::Denied };
        c.thinking = true;
        self.claude.dirty = true;
    }

    fn claude_stop(&mut self) {
        if let Some(id) = self.claude.active
            && let Some(c) = self.claude.clients.get_mut(&id)
        {
            c.interrupt();
        }
    }

    fn claude_set_mode(&mut self, mode: Mode) {
        self.claude.mode = mode;
        self.claude.mode_picked = true;
        if let Some(id) = self.claude.active {
            if let Some(c) = self.claude.clients.get_mut(&id) {
                c.set_mode(mode);
            }
            if let Some(chat) = self.claude.chat_mut(id) {
                chat.mode = mode;
            }
        }
        self.claude.dirty = true;
    }

    fn claude_set_model(&mut self, model: &str) {
        match self.claude.current() {
            Agent::Claude => self.claude.model = model.to_string(),
            Agent::Codex => self.claude.codex_model = model.to_string(),
        }
        if let Some(id) = self.claude.active {
            let busy = self.claude.chat().is_some_and(|c| c.busy);
            if let Some(chat) = self.claude.chat_mut(id)
                && !busy
            {
                chat.model = model.to_string();
                // The next message restarts Claude Code on the new model.
                self.claude.clients.remove(&id);
            }
        }
        self.claude.dirty = true;
    }

    // --------------------------------------------------------------- UI

    /// The scripts folder's files, re-read at most every 2 seconds.
    fn claude_scripts(&mut self) -> Vec<String> {
        if let Some((at, list)) = &self.claude.scripts
            && at.elapsed() < std::time::Duration::from_secs(2)
        {
            return list.clone();
        }
        let list = self.script_files();
        self.claude.scripts = Some((std::time::Instant::now(), list.clone()));
        list
    }

    pub(crate) fn claude_widget(&mut self, ui: &mut Ui, t: &Theme) {
        self.claude_load();
        // Thin, quiet scrollbars: hidden until you scroll or point at them.
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle {
            bar_width: 6.0,
            floating_width: 3.0,
            dormant_handle_opacity: 0.0,
            dormant_background_opacity: 0.0,
            active_handle_opacity: 0.25,
            active_background_opacity: 0.0,
            interact_handle_opacity: 0.45,
            interact_background_opacity: 0.0,
            foreground_color: false,
            ..egui::style::ScrollStyle::floating()
        };
        let now = ui.input(|i| i.time);
        let full = ui.max_rect();
        if self.claude.agent.is_none() {
            self.claude_picker(ui, t, now);
            return;
        }
        // The move into this agent's panel.
        let anim = self.claude.anim;
        let p = match anim {
            Some(a) => ((now - a.start) as f32 / ANIM).clamp(0.0, 1.0),
            _ => 1.0,
        };
        if p >= 1.0 {
            self.claude.anim = None;
        } else {
            ui.ctx().request_repaint();
        }
        if self.claude.view == View::Chats {
            self.claude_chats_view(ui, t);
            return;
        }
        let agent = self.claude.current();
        self.claude_header(ui, t, p, anim.is_some_and(|a| a.from.is_some()), now);
        // The body fades in and rises after the header settles.
        let body = crate::motion::ease_out(((p - 0.25) / 0.75).clamp(0.0, 1.0));
        ui.multiply_opacity(body);
        ui.add_space(16.0 * (1.0 - body));
        if self.claude.cli_for(agent).is_none() {
            ui.add_space(48.0);
            ui.vertical_centered(|ui| {
                halo(ui, t, agent, 38.0);
                ui.add_space(14.0);
                ui.label(RichText::new(format!("{} isn't installed", agent.product())).font(theme::semibold(15.0)));
                ui.add_space(4.0);
                // "Install X" links to the CLI's own install page.
                let mut job = LayoutJob::default();
                let font = FontId::proportional(12.5);
                job.append(
                    &format!("Install {}", agent.product()),
                    0.0,
                    TextFormat {
                        font_id: font.clone(),
                        color: t.text,
                        underline: Stroke::new(1.0, t.text2),
                        ..Default::default()
                    },
                );
                job.append(
                    " and sign in with your account.",
                    0.0,
                    TextFormat::simple(font, t.text2),
                );
                let link = ui.add(egui::Label::new(job).wrap().sense(Sense::click()));
                if link
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(agent.install_url())
                    .clicked()
                {
                    crate::app::open_url(agent.install_url());
                }
                ui.add_space(14.0);
                if w::secondary(ui, "Check again").clicked() {
                    match agent {
                        Agent::Claude => self.claude.cli = None,
                        Agent::Codex => self.claude.codex_cli = None,
                    }
                }
            });
        } else if self.claude_signed_in(ui, t, agent) {
            self.claude_take_files(ui, t);
            let composer_h = self.claude_composer_height(ui.available_width());
            let list_h = (ui.available_height() - composer_h - 10.0).max(80.0);
            // The messages fill a fixed slot so the composer sits at the bottom.
            let (slot, _) = ui.allocate_exact_size(vec2(ui.available_width(), list_h), Sense::hover());
            ui.scope_builder(egui::UiBuilder::new().max_rect(slot), |ui| self.claude_messages(ui, t));
            ui.add_space(10.0);
            self.claude_composer(ui, t);
        }
        if let Some(a) = anim
            && let Some(from) = a.from
            && p < 1.0
        {
            self.claude_pick_overlay(ui, t, a.to, from, p, full);
        }
    }

    /// Whether `agent`'s CLI is signed in; otherwise shows the sign-in gate
    /// (or a spinner while the CLI is asked) and returns false.
    fn claude_signed_in(&mut self, ui: &mut Ui, t: &Theme, agent: Agent) -> bool {
        if crate::app::headless() && std::env::var_os(agent.env()).is_none() {
            return true;
        }
        let Some(exe) = self.claude.cli_for(agent) else {
            return true;
        };
        let ctx = self.egui_ctx.clone();
        let wake = move || ctx.request_repaint();
        let state = self
            .claude
            .auth
            .entry(agent)
            .or_insert_with(|| auth::Auth::Checking(auth::check(agent, &exe, wake.clone())));
        let mut done = false;
        match state {
            auth::Auth::In => return true,
            auth::Auth::Checking(rx) => match rx.try_recv() {
                Ok(true) => {
                    *state = auth::Auth::In;
                    return true;
                }
                Ok(false) => *state = auth::Auth::Out,
                Err(_) => {
                    ui.add_space(96.0);
                    ui.vertical_centered(|ui| agent::spinner(ui, 26.0, agent, agent.accent(t)));
                    ui.ctx().request_repaint();
                    return false;
                }
            },
            auth::Auth::SigningIn(l) => done = l.poll().is_some(),
            auth::Auth::Out => {}
        }
        if done {
            *state = auth::Auth::Checking(auth::check(agent, &exe, wake.clone()));
        }
        ui.add_space(48.0);
        let mut action = None;
        ui.vertical_centered(|ui| {
            halo(ui, t, agent, 38.0);
            ui.add_space(14.0);
            ui.label(RichText::new(format!("Sign in to {}", agent.product())).font(theme::semibold(15.0)));
            ui.add_space(14.0);
            match self.claude.auth.get_mut(&agent) {
                Some(auth::Auth::SigningIn(l)) => {
                    ui.label(
                        RichText::new("Finish signing in in your browser.")
                            .color(t.text2)
                            .size(12.5),
                    );
                    if agent == Agent::Claude {
                        ui.add_space(12.0);
                        let w = (ui.available_width() - 32.0).min(260.0);
                        let r = ui.add_sized(
                            [w, 34.0],
                            w::field(&mut l.code)
                                .hint_text("Paste the code, if asked")
                                .interactive(!l.sent),
                        );
                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            l.send_code();
                        }
                        ui.add_space(8.0);
                        let ready = !l.sent && !l.code.trim().is_empty();
                        if w::button(ui, w::Kind::Primary, ready, None, "Continue").clicked() {
                            l.send_code();
                        }
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let url = l.url.clone();
                        let width = if url.is_some() { 200.0 } else { 90.0 };
                        ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                        if let Some(url) = url
                            && w::secondary(ui, "Open page").clicked()
                        {
                            crate::app::open_url(&url);
                        }
                        if w::secondary(ui, "Cancel").clicked() {
                            action = Some(false);
                        }
                    });
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                }
                _ => {
                    if w::primary(ui, "Sign in").clicked() {
                        action = Some(true);
                    }
                    ui.add_space(8.0);
                    if w::secondary(ui, "Check again").clicked() {
                        action = Some(false);
                    }
                }
            }
        });
        match action {
            Some(true) => {
                let state = match auth::Login::start(agent, &exe, wake) {
                    Ok(l) => auth::Auth::SigningIn(l),
                    Err(_) => auth::Auth::Out,
                };
                self.claude.auth.insert(agent, state);
            }
            Some(false) => {
                self.claude.auth.remove(&agent);
            }
            None => {}
        }
        false
    }

    /// The first open: a card per agent. Picking one moves into its panel.
    fn claude_picker(&mut self, ui: &mut Ui, t: &Theme, now: f64) {
        let shown = *self.claude.picker_shown.get_or_insert(now);
        let ctx = ui.ctx().clone();
        let head = crate::motion::appear(&ctx, shown, 0.0, crate::motion::EMPHASIS);
        ui.add_space((ui.available_height() * 0.16).clamp(16.0, 90.0) + 10.0 * (1.0 - head));
        ui.scope(|ui| {
            ui.multiply_opacity(head);
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                let blink = blink_at(now - shown);
                for (k, a) in [(30.0, 0.05), (24.0, 0.07)] {
                    ui.painter()
                        .circle_filled(r.center(), k, crate::motion::with_alpha(t.text, a));
                }
                mark::face(ui, r.center(), 19.0, t, 1.0, blink);
                if blink > 0.0 {
                    ui.ctx().request_repaint();
                } else {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_secs_f64(blink_wait(now - shown)));
                }
                ui.add_space(14.0);
                ui.label(RichText::new("Pick your agent").font(theme::semibold(17.0)));
            });
        });
        ui.add_space(22.0);
        let mut picked = None;
        self.claude.cards.clear();
        for (k, a) in Agent::ALL.into_iter().enumerate() {
            let e = crate::motion::appear(&ctx, shown, 0.08 + k as f32 * 0.07, crate::motion::EMPHASIS);
            let installed = self.claude.cli_for(a).is_some();
            let (slot, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::click());
            let hover = crate::motion::toggle(&ctx, resp.id, resp.hovered(), crate::motion::MICRO);
            let r = slot.translate(vec2(0.0, 12.0 * (1.0 - e) - 1.5 * hover));
            let p = ui.painter();
            let alpha = e;
            p.rect(
                r,
                14,
                crate::motion::with_alpha(crate::motion::lerp_color(t.card, t.elevated, hover), alpha),
                Stroke::new(
                    1.0,
                    crate::motion::with_alpha(
                        crate::motion::lerp_color(t.border, a.accent(t).gamma_multiply(0.7), hover),
                        alpha,
                    ),
                ),
                egui::StrokeKind::Inside,
            );
            let badge = egui::Rect::from_center_size(egui::pos2(r.left() + 37.0, r.center().y), vec2(42.0, 42.0));
            p.circle_filled(
                badge.center(),
                21.0,
                crate::motion::with_alpha(a.accent(t), alpha * if a == Agent::Codex { 0.08 } else { 0.14 }),
            );
            let logo = egui::Rect::from_center_size(badge.center(), vec2(22.0, 22.0));
            agent::paint(ui, logo, a, crate::motion::with_alpha(a.tint(t), alpha), 0.0);
            let p = ui.painter();
            let x = r.left() + 70.0;
            p.text(
                egui::pos2(x, r.center().y),
                egui::Align2::LEFT_CENTER,
                a.name(),
                theme::semibold(14.5),
                crate::motion::with_alpha(t.text, alpha),
            );
            if installed {
                p.text(
                    egui::pos2(r.right() - 18.0 + 3.0 * hover, r.center().y),
                    egui::Align2::CENTER_CENTER,
                    ph::CARET_RIGHT,
                    theme::regular(14.0),
                    crate::motion::with_alpha(crate::motion::lerp_color(t.text3, t.text, hover), alpha),
                );
            } else {
                p.text(
                    egui::pos2(r.right() - 16.0, r.center().y),
                    egui::Align2::RIGHT_CENTER,
                    "Not installed",
                    theme::regular(11.5),
                    crate::motion::with_alpha(t.text3, alpha),
                );
            }
            self.claude.cards.push((a, r, logo));
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                picked = Some((a, r, logo));
            }
            ui.add_space(10.0);
        }
        if let Some((a, r, logo)) = picked {
            self.claude.picker_shown = None;
            self.claude_switch(a, Some((r, logo)), now);
        }
    }

    /// The move from the picker: the picked card shrinks into the header
    /// while its logo flies to the header's, and the other cards sink away.
    fn claude_pick_overlay(
        &self,
        ui: &Ui,
        t: &Theme,
        to: Agent,
        from: (egui::Rect, egui::Rect),
        p: f32,
        clip: egui::Rect,
    ) {
        use crate::motion::{ease_in_out, ease_out, lerp, with_alpha};
        let painter = ui
            .ctx()
            .layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("agent-move")))
            .with_clip_rect(clip);
        let e = ease_in_out(p);
        let fade = 1.0 - ease_out((p / 0.55).min(1.0));
        let lerp_rect = |a: egui::Rect, b: egui::Rect, k: f32| {
            egui::Rect::from_min_max(
                egui::pos2(lerp(a.left(), b.left(), k), lerp(a.top(), b.top(), k)),
                egui::pos2(lerp(a.right(), b.right(), k), lerp(a.bottom(), b.bottom(), k)),
            )
        };
        for (a, r, _) in &self.claude.cards {
            if *a == to {
                continue;
            }
            let r = r.translate(vec2(0.0, 28.0 * ease_out(p)));
            painter.rect(
                r,
                14,
                with_alpha(t.card, fade),
                Stroke::new(1.0, with_alpha(t.border, fade)),
                egui::StrokeKind::Inside,
            );
        }
        let target_logo = self
            .claude
            .header_logo
            .unwrap_or_else(|| egui::Rect::from_min_size(clip.min + vec2(0.0, 6.0), vec2(18.0, 18.0)));
        let target_card = target_logo.expand2(vec2(6.0, 6.0));
        let card = lerp_rect(from.0, target_card, e);
        painter.rect(
            card,
            lerp(14.0, 9.0, e),
            with_alpha(t.card, fade),
            Stroke::new(1.0, with_alpha(to.accent(t).gamma_multiply(0.7), fade)),
            egui::StrokeKind::Inside,
        );
        let logo = lerp_rect(from.1, target_logo, e);
        agent::paint_flat(&painter, logo, to, to.tint(t));
    }

    fn claude_header(&mut self, ui: &mut Ui, t: &Theme, p: f32, moving: bool, now: f64) {
        let agent = self.claude.current();
        let title = self
            .claude
            .chat()
            .filter(|c| !c.items.is_empty())
            .map(|c| c.title.clone());
        let mut new = false;
        let mut list = false;
        let mut switch = None;
        let mut account = None;
        ui.horizontal(|ui| {
            ui.set_height(32.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if w::tool_button(ui, t, ph::NOTE_PENCIL, "New chat", false, 28.0).clicked() {
                    new = true;
                }
                if w::tool_button(ui, t, ph::CHATS_CIRCLE, "Chats", false, 28.0).clicked() {
                    list = true;
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    // The agent: logo, name and a caret; click for the other.
                    let name_g = ui
                        .painter()
                        .layout_no_wrap(agent.name().to_string(), theme::semibold(14.0), t.text);
                    let bw = 8.0 + 18.0 + 8.0 + name_g.size().x + 6.0 + 12.0 + 8.0;
                    let (r, resp) = ui.allocate_exact_size(vec2(bw, 30.0), Sense::click());
                    let hover = crate::motion::toggle(ui.ctx(), resp.id, resp.hovered(), crate::motion::MICRO);
                    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
                    let pnt = ui.painter();
                    pnt.rect_filled(
                        r,
                        9,
                        crate::motion::with_alpha(t.hover_wash(), hover.max(if open { 1.0 } else { 0.0 })),
                    );
                    let logo =
                        egui::Rect::from_center_size(egui::pos2(r.left() + 17.0, r.center().y), vec2(18.0, 18.0));
                    self.claude.header_logo = Some(logo);
                    if !(moving && p < 1.0) {
                        // A switch pops the new logo in.
                        let k = if p < 1.0 {
                            0.6 + 0.4 * crate::motion::ease_out_back(p)
                        } else {
                            1.0
                        };
                        agent::paint(ui, logo.scale_from_center(k), agent, agent.tint(t), 0.0);
                    }
                    let pnt = ui.painter();
                    pnt.galley(
                        egui::pos2(logo.right() + 8.0, r.center().y - name_g.size().y / 2.0),
                        name_g,
                        t.text,
                    );
                    pnt.text(
                        egui::pos2(r.right() - 14.0, r.center().y + 1.0),
                        egui::Align2::CENTER_CENTER,
                        ph::CARET_DOWN,
                        theme::regular(10.5),
                        crate::motion::lerp_color(t.text3, t.text2, hover),
                    );
                    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                    egui::Popup::menu(&resp)
                        .align(egui::RectAlign::BOTTOM_START)
                        .gap(6.0)
                        .width(200.0)
                        .show(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for a in Agent::ALL {
                                let installed = self.claude.cli_for(a).is_some();
                                if agent_row(ui, t, a, a == agent, installed).clicked() {
                                    switch = Some(a);
                                    ui.close();
                                }
                            }
                            // Accounts, when there's more than one folder.
                            let list = accounts::list(agent);
                            if list.len() > 1 {
                                ui.separator();
                                let now = accounts::current(agent);
                                for acc in list {
                                    let tip = accounts::folder(agent, &acc.dir)
                                        .map(|f| f.to_string_lossy().to_string())
                                        .unwrap_or_default();
                                    if w::option(ui, acc.dir == now, format!("{}  {}", ph::USER_CIRCLE, acc.label))
                                        .on_hover_text(tip)
                                        .clicked()
                                    {
                                        account = Some(acc.dir);
                                        ui.close();
                                    }
                                }
                            }
                        });
                    // The chat's title, after the agent.
                    if let Some(title) = &title {
                        ui.label(RichText::new("/").color(t.text3).size(13.0));
                        let editing = self
                            .claude
                            .renaming
                            .as_ref()
                            .is_some_and(|(id, _)| Some(*id) == self.claude.active);
                        if editing {
                            let h = 30.0;
                            let r = egui::Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), h));
                            let r = r.translate(vec2(0.0, (ui.available_height() - h) / 2.0));
                            self.claude_rename_field(ui, t, r);
                        } else {
                            let resp = ui.add(
                                egui::Label::new(RichText::new(title).font(theme::medium(13.0)).color(t.text2))
                                    .truncate()
                                    .sense(Sense::click()),
                            );
                            if let Some(id) = self.claude.active
                                && resp.on_hover_text("Double-click to rename").double_clicked()
                            {
                                self.claude.renaming = Some((id, title.clone()));
                            }
                        }
                    }
                });
            });
        });
        if new {
            self.claude_new_chat();
        }
        if list {
            self.claude.view = View::Chats;
            self.claude.chat_query.clear();
        }
        if let Some(a) = switch
            && a != agent
        {
            self.claude_switch(a, None, now);
        }
        if let Some(dir) = account {
            self.claude_set_account(agent, &dir);
        }
        // A hairline under the header.
        ui.add_space(6.0);
        let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().hline(
            line.x_range(),
            line.center().y,
            Stroke::new(1.0, t.border.gamma_multiply(0.6)),
        );
        ui.add_space(6.0);
    }

    /// The title being edited, in `rect`: Enter or clicking away keeps it,
    /// Esc puts the old one back.
    fn claude_rename_field(&mut self, ui: &mut Ui, t: &Theme, rect: egui::Rect) {
        let Some((id, mut text)) = self.claude.renaming.clone() else {
            return;
        };
        let field_id = egui::Id::new(("claude-rename", id));
        let resp = ui.put(
            rect,
            egui::TextEdit::singleline(&mut text)
                .id(field_id)
                .font(theme::medium(13.0))
                .margin(Margin::symmetric(8, 6))
                .vertical_align(Align::Center)
                .background_color(t.card),
        );
        if !resp.has_focus() && !resp.lost_focus() {
            resp.request_focus();
        }
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if esc {
            self.claude.renaming = None;
        } else if resp.lost_focus() {
            let name = text.trim().to_string();
            if !name.is_empty()
                && let Some(c) = self.claude.chat_mut(id)
            {
                c.title = name;
                self.claude.dirty = true;
            }
            self.claude.renaming = None;
        } else {
            self.claude.renaming = Some((id, text));
        }
    }

    /// Runs `agent` as another account from now on: its sign-in, model and
    /// usage are the new account's, and the open chat (if it has started)
    /// gives way to a new one. Older chats keep their own account.
    pub(crate) fn claude_set_account(&mut self, agent: Agent, dir: &str) {
        if accounts::current(agent) == dir {
            return;
        }
        accounts::set_current(agent, dir);
        let _ = self.store.set_value(&account_key(agent), dir);
        let c = &mut self.claude;
        c.auth.remove(&agent);
        c.defaults.remove(&agent);
        match config_model(agent) {
            Some(m) => c.configured.insert(agent, m),
            None => c.configured.remove(&agent),
        };
        match agent {
            Agent::Claude => c.limits.clear(),
            Agent::Codex => {
                c.codex_limits.clear();
                c.codex_models.clear();
                c.models_asked = false;
            }
        }
        if self
            .claude
            .chat()
            .is_some_and(|ch| ch.agent == agent && ch.account.as_deref() != Some(dir) && !ch.items.is_empty())
        {
            self.new_chat();
        } else if let Some(id) = self.claude.active
            && let Some(ch) = self.claude.chat_mut(id)
            && ch.items.is_empty()
        {
            ch.account = Some(dir.to_string());
        }
    }

    /// "New chat": an empty chat that's already open is reused.
    pub(crate) fn claude_new_chat(&mut self) {
        self.claude_load();
        self.claude.view = View::Chat;
        if self.claude.chat().is_some_and(|c| c.items.is_empty()) {
            return;
        }
        self.new_chat();
    }

    // ----------------------------------------------------------- chats

    fn claude_chats_view(&mut self, ui: &mut Ui, t: &Theme) {
        let mut back = false;
        let mut new = false;
        ui.horizontal(|ui| {
            ui.set_height(30.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::tool_button(ui, t, ph::NOTE_PENCIL, "New chat", false, 28.0).clicked() {
                    new = true;
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if w::tool_button(ui, t, ph::CARET_LEFT, "Back", false, 28.0).clicked() {
                        back = true;
                    }
                    ui.label(RichText::new("Chats").font(theme::semibold(14.0)));
                });
            });
        });
        ui.add_space(6.0);
        let width = ui.available_width();
        w::search_field(ui, t, &mut self.claude.chat_query, "Search chats", width);
        ui.add_space(8.0);
        let q = self.claude.chat_query.to_lowercase();
        let mut list: Vec<Chat> = self
            .claude
            .chats
            .iter()
            .filter(|c| !c.items.is_empty())
            .filter(|c| {
                q.is_empty()
                    || c.title.to_lowercase().contains(&q)
                    || c.items.iter().any(|i| match i {
                        Item::User(s) | Item::Assistant(s) => s.to_lowercase().contains(&q),
                        _ => false,
                    })
            })
            .cloned()
            .collect();
        list.sort_by_key(|c| -c.updated);
        let mut open = None;
        let mut delete = None;
        if list.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(ph::CHATS_CIRCLE)
                        .font(theme::regular(30.0))
                        .color(t.text3),
                );
                ui.add_space(4.0);
                let (head, body) = if q.is_empty() {
                    ("No chats yet", "Your conversations with your agents show up here.")
                } else {
                    ("Nothing found", "No chat mentions that.")
                };
                ui.label(RichText::new(head).font(theme::semibold(13.5)));
                ui.label(RichText::new(body).color(t.text2));
            });
        }
        egui::ScrollArea::vertical()
            .id_salt("claude-chats")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let today = jiff::Zoned::now().date();
                let mut group = "";
                for c in &list {
                    let day = jiff::Timestamp::from_millisecond(c.updated)
                        .map(|ts| ts.to_zoned(jiff::tz::TimeZone::system()).date())
                        .unwrap_or(today);
                    let days = (today - day).get_days();
                    let g = match days {
                        ..=0 => "Today",
                        1..=7 => "Previous 7 days",
                        _ => "Older",
                    };
                    if g != group {
                        group = g;
                        ui.add_space(6.0);
                        ui.label(RichText::new(g).font(theme::semibold(11.0)).color(t.text3));
                        ui.add_space(2.0);
                    }
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::click());
                    let on = Some(c.id) == self.claude.active;
                    let hovered = ui.rect_contains_pointer(r);
                    let editing = self.claude.renaming.as_ref().is_some_and(|(id, _)| *id == c.id);
                    let p = ui.painter();
                    if on {
                        p.rect_filled(r, 8, t.accent_soft());
                    } else if hovered || editing {
                        p.rect_filled(r, 8, t.hover_wash());
                    }
                    let pad = 12.0;
                    // Whose chat it is.
                    agent::paint(
                        ui,
                        egui::Rect::from_center_size(egui::pos2(r.left() + 20.0, r.top() + 19.0), vec2(14.0, 14.0)),
                        c.agent,
                        c.agent.tint(t),
                        0.0,
                    );
                    let p = ui.painter();
                    let pad_l = 34.0;
                    // Rename and delete on hover.
                    let bin =
                        egui::Rect::from_center_size(egui::pos2(r.right() - 20.0, r.center().y), vec2(26.0, 26.0));
                    let pen = bin.translate(vec2(-28.0, 0.0));
                    let tools = hovered && !editing;
                    let right_edge = if tools { pen.left() - 4.0 } else { r.right() - pad };
                    let font = theme::medium(13.0);
                    let when = short_time(c.updated);
                    let when_g = p.layout_no_wrap(when, theme::regular(11.0), t.text3);
                    if editing {
                        let edit = egui::Rect::from_min_max(
                            egui::pos2(r.left() + pad_l - 6.0, r.top() + 4.0),
                            egui::pos2(r.right() - pad + 6.0, r.top() + 34.0),
                        );
                        self.claude_rename_field(ui, t, edit);
                    } else {
                        let title_w = right_edge - (r.left() + pad_l) - if tools { 0.0 } else { when_g.size().x + 8.0 };
                        let title = crate::format::elide_to(p, &c.title, &font, title_w);
                        p.text(
                            egui::pos2(r.left() + pad_l, r.top() + 19.0),
                            egui::Align2::LEFT_CENTER,
                            title,
                            font,
                            t.text,
                        );
                        if !tools {
                            p.galley(
                                egui::pos2(r.right() - pad - when_g.size().x, r.top() + 12.0),
                                when_g,
                                t.text3,
                            );
                        }
                    }
                    let preview = preview_of(c);
                    let pf = theme::regular(12.0);
                    let preview = crate::format::elide_to(ui.painter(), &preview, &pf, right_edge - (r.left() + pad_l));
                    ui.painter().text(
                        egui::pos2(r.left() + pad_l, r.top() + 42.0),
                        egui::Align2::LEFT_CENTER,
                        preview,
                        pf,
                        t.text2,
                    );
                    let mut on_tool = false;
                    if tools {
                        for (rect, glyph, tip, what) in [
                            (pen, ph::PENCIL_SIMPLE, "Rename", 0),
                            (bin, ph::TRASH, "Delete chat", 1),
                        ] {
                            let b = ui.interact(rect, ui.id().with(("chat-tool", c.id, what)), Sense::click());
                            let over = b.hovered();
                            on_tool |= over;
                            if over {
                                ui.painter().rect_filled(rect, 6, t.hover_wash());
                            }
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                glyph,
                                theme::regular(14.0),
                                if over { t.text } else { t.text2 },
                            );
                            if b.on_hover_text(tip).clicked() {
                                if what == 0 {
                                    self.claude.renaming = Some((c.id, c.title.clone()));
                                } else {
                                    delete = Some(c.id);
                                }
                            }
                        }
                    }
                    if resp.clicked() && !on_tool && !editing {
                        open = Some(c.id);
                    }
                }
            });
        if back {
            self.claude.view = View::Chat;
        }
        if new {
            self.claude_new_chat();
        }
        if let Some(id) = open {
            // Another agent's chat: switch to that agent first.
            if let Some(a) = self.claude.chats.iter().find(|c| c.id == id).map(|c| c.agent)
                && a != self.claude.current()
            {
                let now = ui.input(|i| i.time);
                self.claude_switch(a, None, now);
            }
            self.claude.active = Some(id);
            self.claude.view = View::Chat;
            self.claude.dirty = true;
        }
        if let Some(id) = delete {
            self.delete_chat(id);
        }
    }

    // -------------------------------------------------------- messages

    fn claude_messages(&mut self, ui: &mut Ui, t: &Theme) {
        let chat = self.claude.chat().cloned().filter(|c| !c.items.is_empty());
        let Some(chat) = chat else {
            self.claude_welcome(ui, t);
            return;
        };
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new("agent-on-show"), chat.agent));
        let mut answer = None;
        let mut toggle = None;
        let mut fold = None;
        let scripts = self.claude_scripts();
        let mut open_file: Option<String> = None;
        let mut open_attachment: Option<String> = None;
        // Keys 1-3 answer a waiting prompt (when no text field has focus).
        let waiting = chat.items.iter().rposition(is_waiting);
        if let Some(i) = waiting
            && !ui.ctx().egui_wants_keyboard_input()
        {
            let can_always = matches!(&chat.items[i], Item::Ask { suggestions, .. } if suggestions.as_array().is_some_and(|a| !a.is_empty()));
            ui.input_mut(|inp| {
                if inp.consume_key(egui::Modifiers::NONE, egui::Key::Num1) {
                    answer = Some((i, true, false));
                } else if can_always && inp.consume_key(egui::Modifiers::NONE, egui::Key::Num2) {
                    answer = Some((i, true, true));
                } else if inp.consume_key(
                    egui::Modifiers::NONE,
                    if can_always { egui::Key::Num3 } else { egui::Key::Num2 },
                ) {
                    answer = Some((i, false, false));
                }
            });
        }
        egui::ScrollArea::vertical()
            .id_salt(("claude-msgs", chat.id))
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                // Room for the floating scrollbar on the right.
                let width = ui.available_width() - 8.0;
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 8.0;
                let items = &chat.items;
                let mut i = 0;
                while i < items.len() {
                    let it = &items[i];
                    // The agent's byline starts each of its turns.
                    let mine = |x: &Item| matches!(x, Item::User(_) | Item::Attached(_));
                    let starts_turn = !mine(it) && (i == 0 || mine(&items[i - 1]));
                    if starts_turn {
                        ui.add_space(8.0);
                        byline(ui, t, chat.agent);
                    }
                    // A run of tool calls (and answered prompts) is one box of steps.
                    if is_step(it) {
                        let start = i;
                        while i < items.len() && is_step(&items[i]) {
                            i += 1;
                        }
                        let live = chat.busy && i == items.len();
                        let key = (chat.id, start);
                        let n = i - start;
                        let open = self.claude.folds.get(&key).copied().unwrap_or(live || n < 3);
                        let r = steps(
                            ui,
                            t,
                            &items[start..i],
                            start,
                            open,
                            n >= 3,
                            &self.claude.expanded,
                            chat.id,
                        );
                        if r.fold {
                            fold = Some((key, !open));
                        }
                        if let Some(k) = r.toggle {
                            toggle = Some(k);
                        }
                        continue;
                    }
                    match it {
                        Item::User(text) => {
                            if !matches!(i.checked_sub(1).map(|k| &items[k]), Some(Item::Attached(_))) {
                                ui.add_space(10.0);
                            }
                            user_bubble(ui, t, text, width);
                            ui.add_space(4.0);
                        }
                        Item::Attached(files) => {
                            ui.add_space(10.0);
                            if let Chip::Open(k) = file_chips(ui, t, files, width, true, false) {
                                open_attachment = Some(files[k].clone());
                            }
                        }
                        Item::Assistant(text) => markdown(ui, t, text, (chat.id, i), &scripts, &mut open_file),
                        Item::Ask {
                            tool,
                            label,
                            detail,
                            suggestions,
                            ..
                        } => {
                            let can_always = suggestions.as_array().is_some_and(|a| !a.is_empty());
                            if let Some((allow, always)) =
                                ask_card(ui, t, chat.agent, tool, label, detail, can_always, (chat.id, i))
                            {
                                answer = Some((i, allow, always));
                            }
                        }
                        Item::Note { text, error } => {
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                let c = if *error { t.warn } else { t.text3 };
                                ui.label(RichText::new(ph::INFO).color(c).size(14.0));
                                ui.add(egui::Label::new(RichText::new(text).color(t.text2).size(12.5)).wrap());
                            });
                        }
                        Item::Tool { .. } => {}
                    }
                    i += 1;
                }
                if chat.busy {
                    ui.add_space(2.0);
                    let waiting = chat.items.iter().any(is_waiting);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if waiting {
                            agent::show(ui, 15.0, chat.agent, chat.agent.tint(t));
                            ui.label(RichText::new("Waiting for your answer").color(t.text3).size(12.5));
                        } else {
                            agent::spinner(ui, 15.0, chat.agent, chat.agent.tint(t));
                            let word = if chat.thinking { "Thinking" } else { "Working" };
                            shimmer(ui, t, word);
                        }
                    });
                }
                ui.add_space(6.0);
            });
        if let Some(k) = toggle
            && !self.claude.expanded.remove(&k)
        {
            self.claude.expanded.insert(k);
        }
        if let Some((k, open)) = fold {
            self.claude.folds.insert(k, open);
        }
        if let Some(f) = open_attachment {
            // A script in the scripts folder opens in the editor; anything
            // else in the system's own app.
            let dir = self.scripts_dir();
            match std::path::Path::new(&f).strip_prefix(&dir) {
                Ok(rel) if crate::scripting::Lang::of(&rel.to_string_lossy()).is_some() => {
                    open_file = Some(rel.to_string_lossy().replace('\\', "/"));
                }
                _ => crate::app::open_path(std::path::Path::new(&f)),
            }
        }
        if let Some(f) = open_file {
            self.open_in_editor(&f);
            self.layout.tab = Some(crate::shell::Tab::Editor);
            self.last_tab = crate::shell::Tab::Editor;
        }
        if let Some((i, allow, always)) = answer {
            self.claude_answer(chat.id, i, allow, always);
        }
    }

    /// An empty chat: the agent's logo and name.
    fn claude_welcome(&mut self, ui: &mut Ui, t: &Theme) {
        let agent = self.claude.current();
        ui.add_space((ui.available_height() * 0.3).clamp(16.0, 160.0));
        ui.vertical_centered(|ui| {
            halo(ui, t, agent, 40.0);
            ui.add_space(14.0);
            ui.label(RichText::new(agent.name()).font(theme::semibold(18.0)));
        });
    }

    // -------------------------------------------------------- composer

    /// The composer's height: as tall as it was last frame (wrapped lines
    /// count, which a line count of the text misses), else an estimate.
    fn claude_composer_height(&self, width: f32) -> f32 {
        if let Some(r) = self.claude.composer
            && (r.width() - width).abs() < 1.0
        {
            return r.height();
        }
        let lines = (self.claude.input.lines().count() as f32).clamp(2.0, MAX_LINES);
        let n = self.claude.attached.len();
        let per_row = ((width - 20.0) / 176.0).floor().max(1.0) as usize;
        let chips = if n == 0 {
            0.0
        } else {
            n.div_ceil(per_row) as f32 * 34.0 + 2.0
        };
        let note = if self.claude.attach_note.is_some() { 20.0 } else { 0.0 };
        lines * LINE_H + 64.0 + chips + note
    }

    /// Adds files to the next message (each once).
    fn claude_attach(&mut self, files: Vec<PathBuf>) {
        for f in files {
            let f = f.to_string_lossy().into_owned();
            if !self.claude.attached.contains(&f) {
                self.claude.attached.push(f);
            }
        }
    }

    /// The clipboard's files, or its image (saved in the workspace).
    fn claude_paste(&mut self, now: f64) {
        let dir = self.store.dir().join("claude").join("attachments");
        match attach::from_clipboard(&dir) {
            Ok(files) if !files.is_empty() => self.claude_attach(files),
            Ok(_) => self.claude.attach_note = Some(("There's no image or file on the clipboard.".into(), now)),
            Err(e) => self.claude.attach_note = Some((e, now)),
        }
    }

    /// Files from the picker, or dropped on the window while the panel is
    /// open; and the "drop to attach" overlay while they hover.
    fn claude_take_files(&mut self, ui: &Ui, t: &Theme) {
        if let Some(rx) = &self.claude.picker {
            match rx.try_recv() {
                Ok(files) => {
                    self.claude.picker = None;
                    self.claude_attach(files);
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => self.claude.picker = None,
                Err(_) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(200)),
            }
        }
        let (dropped, hovering) = ui.input(|i| {
            (
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| f.path().to_path_buf())
                    .filter(|p| p.is_file())
                    .collect::<Vec<_>>(),
                !i.raw.hovered_files.is_empty(),
            )
        });
        if !dropped.is_empty() {
            self.claude_attach(dropped);
        }
        if hovering {
            let r = ui.max_rect();
            let p = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("claude-drop"),
            ));
            p.rect(
                r.shrink(2.0),
                12,
                t.bg.gamma_multiply(0.92),
                Stroke::new(1.5, self.claude.current().accent(t)),
                egui::StrokeKind::Inside,
            );
            p.text(
                r.center() - vec2(0.0, 14.0),
                egui::Align2::CENTER_CENTER,
                ph::PAPERCLIP,
                theme::regular(28.0),
                self.claude.current().accent(t),
            );
            p.text(
                r.center() + vec2(0.0, 18.0),
                egui::Align2::CENTER_CENTER,
                "Drop files to attach",
                theme::medium(13.5),
                t.text,
            );
        }
    }

    fn claude_composer(&mut self, ui: &mut Ui, t: &Theme) {
        let busy = self.claude.chat().is_some_and(|c| c.busy);
        let agent = self.claude.current();
        let accent = agent.accent(t);
        let mode = self.claude.chat().map(|c| c.mode).unwrap_or(self.claude.mode);
        let model = self
            .claude
            .chat()
            .map(|c| c.model.clone())
            .unwrap_or(self.claude.model_for(agent).to_string());
        let limits = self.claude.limits_for(agent).to_vec();
        let mut send = false;
        let mut stop = false;
        let mut set_mode = None;
        let mut set_model = None;
        let input_id = egui::Id::new("claude-input");
        let focused = ui.memory(|m| m.has_focus(input_id));
        let focus = crate::motion::toggle(ui.ctx(), input_id.with("ring"), focused, crate::motion::STANDARD);
        let top = self.claude.composer.map_or(0.0, |r| r.top());
        let frame = egui::Frame::new()
            .fill(if t.dark { t.elevated } else { t.card })
            .stroke(Stroke::new(
                1.0,
                crate::motion::lerp_color(
                    t.border,
                    accent.gamma_multiply(if agent == Agent::Codex { 0.45 } else { 0.6 }),
                    focus,
                ),
            ))
            .shadow(egui::Shadow {
                offset: [0, 6],
                blur: 22,
                spread: 0,
                color: Color32::from_black_alpha(if t.dark { 70 } else { 22 }),
            })
            .corner_radius(CornerRadius::same(18))
            .inner_margin(Margin {
                left: 12,
                right: 8,
                top: 10,
                bottom: 8,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let now = ui.input(|i| i.time);
                // Pasted files (a file manager copies them as paths) attach;
                // Alt+V attaches the clipboard's image (egui's Ctrl+V only
                // carries text).
                if focused {
                    let mut files = None;
                    ui.input_mut(|i| {
                        i.events.retain(|e| match e {
                            egui::Event::Paste(text) => match attach::pasted_files(text) {
                                Some(f) => {
                                    files = Some(f);
                                    false
                                }
                                None => true,
                            },
                            _ => true,
                        })
                    });
                    if let Some(f) = files {
                        self.claude_attach(f);
                    }
                    // Alt+V also arrives as typed text ("v"): drop it, or
                    // the paste types a letter too.
                    let alt_v = ui.input_mut(|i| {
                        let hit = i.consume_key(egui::Modifiers::ALT, egui::Key::V);
                        if hit {
                            i.events.retain(|e| !matches!(e, egui::Event::Text(_)));
                        }
                        hit
                    });
                    if alt_v {
                        self.claude_paste(now);
                    }
                }
                if !self.claude.attached.is_empty() {
                    let files = self.claude.attached.clone();
                    match file_chips(ui, t, &files, ui.available_width(), false, true) {
                        Chip::Remove(k) => {
                            self.claude.attached.remove(k);
                        }
                        Chip::Open(k) => crate::app::open_path(std::path::Path::new(&files[k])),
                        Chip::None => {}
                    }
                    ui.add_space(4.0);
                }
                // Enter sends; Shift+Enter is a new line.
                let enter = focused && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                // A softer cursor than the app's fields.
                let cursor = &mut ui.visuals_mut().text_cursor.stroke;
                cursor.color = crate::motion::with_alpha(t.text, 0.45);
                cursor.width = cursor.width.min(1.5);
                // Long prompts scroll inside the box (at most `MAX_LINES`
                // tall) instead of growing it out of the pane.
                egui::ScrollArea::vertical()
                    .id_salt("claude-input-scroll")
                    .max_height(MAX_LINES * LINE_H)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.claude.input)
                                .id(input_id)
                                // A little air between the cursor and the hint.
                                .hint_text(RichText::new(" What can I help you with?").color(t.text3))
                                .font(theme::regular(13.5))
                                .desired_rows(2)
                                .frame(egui::Frame::NONE)
                                .desired_width(ui.available_width()),
                        );
                    });
                if enter && !busy {
                    send = true;
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.set_height(30.0);
                    ui.spacing_mut().item_spacing.x = 4.0;
                    // Mode.
                    let (icon, word) = match mode {
                        Mode::Manual => (ph::HAND_PALM, "Ask first"),
                        Mode::Auto => (ph::LIGHTNING, "Auto"),
                    };
                    let mut label = self.claude.current_label(agent, &model);
                    // A narrow pane: the mode shrinks to its icon, then the
                    // model drops its version. The send button and the ring
                    // always keep their room.
                    let clip = pill(ui, t, Some(ph::PAPERCLIP), "", false)
                        .on_hover_text("Attach files (or drop them here; Alt+V pastes an image)");
                    egui::Popup::menu(&clip)
                        .anchor(above(&clip, top))
                        .align(egui::RectAlign::TOP_START)
                        .gap(6.0)
                        .width(220.0)
                        .show(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            if menu_pick(ui, t, &format!("{}  Add files", ph::FILE_PLUS), "", false).clicked() {
                                if self.claude.picker.is_none() {
                                    self.claude.picker = Some(attach::pick());
                                }
                                ui.close();
                            }
                            if menu_pick(ui, t, &format!("{}  Paste image", ph::CLIPBOARD_TEXT), "Alt+V", false)
                                .clicked()
                            {
                                self.claude_paste(now);
                                ui.close();
                            }
                        });
                    // The send button (and the ring, while there's room) keep
                    // theirs; the rest gives way in steps.
                    let full_label = label.clone();
                    let room = ui.available_width() - 66.0;
                    let mut show_ring = true;
                    let mut mode_text = word;
                    let mut model_icon = None;
                    let fits = |ui: &Ui, mode_text: &str, icon_m: Option<&str>, label: &str, room: f32| {
                        pill_width(ui, Some(icon), mode_text, true) + pill_width(ui, icon_m, label, true) + 4.0 <= room
                    };
                    if !fits(ui, mode_text, None, &label, room) {
                        mode_text = "";
                    }
                    if !fits(ui, mode_text, None, &label, room) {
                        label = match label.split_once(' ') {
                            Some((first, _)) => first.to_string(),
                            // One word ("GPT-5.6-Terra"): drop the family.
                            None => match label.split_once('-') {
                                Some((_, rest)) if rest.contains('-') => rest.to_string(),
                                _ => label,
                            },
                        };
                    }
                    if !fits(ui, mode_text, None, &label, room) {
                        label.clear();
                        model_icon = Some(ph::CPU);
                    }
                    if !fits(ui, mode_text, model_icon, &label, room) {
                        // Very narrow: the ring steps aside (its numbers go
                        // in the model button's tooltip).
                        show_ring = false;
                    }
                    let resp = pill(ui, t, Some(icon), mode_text, true);
                    self.claude.footer.clear();
                    self.claude.footer.push(clip.rect);
                    self.claude.footer.push(resp.rect);
                    let resp = if mode_text.is_empty() {
                        resp.on_hover_text(word)
                    } else {
                        resp
                    };
                    self.claude_still_menu(ui, &resp, "mode");
                    egui::Popup::menu(&resp)
                        .anchor(above(&resp, top))
                        .align(egui::RectAlign::TOP_START)
                        .gap(6.0)
                        .width(160.0)
                        .show(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (m, icon, title, body) in [
                                (
                                    Mode::Manual,
                                    ph::HAND_PALM,
                                    "Ask first",
                                    match agent {
                                        Agent::Claude => "Claude asks before it edits files or runs commands.",
                                        Agent::Codex => "Codex asks before it edits files or runs anything it doesn't know is safe.",
                                    },
                                ),
                                (
                                    Mode::Auto,
                                    ph::LIGHTNING,
                                    "Auto",
                                    match agent {
                                        Agent::Claude => {
                                            "Claude Code decides what's safe and asks only when it isn't sure."
                                        }
                                        Agent::Codex => {
                                            "Codex works freely in your scripts folder and asks before going outside it."
                                        }
                                    },
                                ),
                            ] {
                                if menu_pick(ui, t, &format!("{icon}  {title}"), "", m == mode)
                                    .on_hover_text(body)
                                    .clicked()
                                {
                                    set_mode = Some(m);
                                    ui.close();
                                }
                            }
                        });
                    // Model.
                    // The model a default chat runs is the one ticked.
                    let ticked = if model.is_empty() {
                        self.claude.default_model(agent).unwrap_or_default()
                    } else {
                        model.clone()
                    };
                    let models = self.claude.models_for(agent);
                    let resp = ui
                        .add_enabled_ui(!busy, |ui| pill(ui, t, model_icon, &label, true))
                        .inner;
                    let resp = if label.is_empty() || !show_ring {
                        let usage: String = if show_ring {
                            String::new()
                        } else {
                            limits
                                .iter()
                                .map(|w| format!("\n{}: {:.0}% used", process::window_name(&w.name), w.used * 100.0))
                                .collect()
                        };
                        resp.on_hover_text(format!("{full_label}{usage}"))
                    } else {
                        resp
                    };
                    self.claude.footer.push(resp.rect);
                    self.claude_still_menu(ui, &resp, "model");
                    // Codex's models come from the account: ask once, when
                    // the menu first opens.
                    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
                    if agent == Agent::Codex
                        && self.claude.codex_models.is_empty()
                        && self.claude.models_rx.is_none()
                        && !self.claude.models_asked
                        && open
                        && let Some(exe) = self.claude.cli_for(agent)
                    {
                        // Once per opening of the menu, even if it fails.
                        self.claude.models_asked = true;
                        self.claude.models_rx = Some(codex::models(exe, self.scripts_dir()));
                    }
                    if !open {
                        self.claude.models_asked = false;
                    }
                    egui::Popup::menu(&resp)
                        .anchor(above(&resp, top))
                        .align(egui::RectAlign::TOP_END)
                        .gap(6.0)
                        .width(176.0)
                        .show(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (k, l) in &models {
                                if menu_pick(ui, t, l, "", *k == ticked).clicked() {
                                    set_model = Some(k.clone());
                                    ui.close();
                                }
                            }
                            if self.claude.models_rx.is_some() && agent == Agent::Codex {
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(12.0);
                                    ui.add(egui::Spinner::new().size(11.0).color(t.text3));
                                    ui.add_space(4.0);
                                    ui.label(RichText::new("Loading models").color(t.text3).size(12.0));
                                });
                                ui.add_space(6.0);
                            }
                        });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let can_send = !self.claude.input.trim().is_empty() || !self.claude.attached.is_empty();
                        let send_resp = send_button(ui, t, agent, busy, can_send);
                        self.claude.footer.push(send_resp.rect);
                        if send_resp.clicked() {
                            if busy {
                                stop = true;
                            } else if can_send {
                                send = true;
                            }
                        }
                        if show_ring {
                            self.claude_usage_ring(ui, t, top);
                        }
                    });
                });
                // A paste that found nothing says so for a few seconds.
                match &self.claude.attach_note {
                    Some((text, at)) if now - at < 4.0 => {
                        ui.label(RichText::new(text).color(t.text3).size(11.5));
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                    }
                    Some(_) => self.claude.attach_note = None,
                    None => {}
                }
            });
        // Grown or shrunk: lay out again so the messages make room.
        if self
            .claude
            .composer
            .is_some_and(|r| (r.height() - frame.response.rect.height()).abs() > 0.5)
        {
            ui.ctx().request_repaint();
        }
        self.claude.composer = Some(frame.response.rect);
        if let Some(m) = set_mode {
            self.claude_set_mode(m);
        }
        if let Some(k) = set_model {
            self.claude_set_model(&k);
        }
        if let Some(rx) = &self.claude.models_rx {
            match rx.try_recv() {
                Ok(list) => {
                    self.claude.models_rx = None;
                    if !list.is_empty() {
                        self.claude.codex_models = list;
                        self.claude.dirty = true;
                    }
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => self.claude.models_rx = None,
                Err(_) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(200)),
            }
        }
        if send {
            self.claude_send();
        }
        if stop {
            self.claude_stop();
        }
    }

    /// A small ring for the 5-hour limit; hover for the numbers, click for
    /// the account and this chat's usage.
    fn claude_usage_ring(&mut self, ui: &mut Ui, t: &Theme, top: f32) {
        let limits = self.claude.limits_for(self.claude.current()).to_vec();
        let used = limits.first().map(|w| w.used).unwrap_or(0.0) as f32;
        let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
        self.claude.footer.push(r);
        let hover = ui.rect_contains_pointer(r);
        let p = ui.painter();
        if hover {
            p.circle_filled(r.center(), 13.0, t.hover_wash());
        }
        let c = r.center();
        let rad = 7.5;
        p.circle_stroke(c, rad, Stroke::new(2.2, t.border));
        if used > 0.0 {
            let n = 48;
            let end = used.clamp(0.0, 1.0) * std::f32::consts::TAU;
            let pts: Vec<egui::Pos2> = (0..=n)
                .map(|k| {
                    let a = -std::f32::consts::FRAC_PI_2 + end * k as f32 / n as f32;
                    c + vec2(a.cos(), a.sin()) * rad
                })
                .collect();
            let col = if used >= 0.85 { t.warn } else { t.text2 };
            p.add(egui::Shape::line(pts, Stroke::new(2.2, col)));
        }
        let tip = if limits.is_empty() {
            "Usage shows after your first message.".to_string()
        } else {
            limits
                .iter()
                .map(|w| format!("{}: {:.0}% used", process::window_name(&w.name), w.used * 100.0))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
        self.claude_still_menu(ui, &resp, "usage");
        // Anchored to the composer's right edge, so the card stays in the pane.
        let edge = self.claude.composer.map_or(resp.rect.right(), |c| c.right());
        let y = above(&resp, top).top();
        egui::Popup::menu(&resp)
            .anchor(egui::Rect::from_min_max(egui::pos2(edge, y), egui::pos2(edge, y)))
            .align(egui::RectAlign::TOP_END)
            .gap(6.0)
            .width(236.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                self.claude_usage_card(ui, t);
            });
    }

    /// Opens a composer menu for a screenshot, once the composer has been
    /// laid out (menus anchor to its top edge).
    fn claude_still_menu(&mut self, ui: &Ui, resp: &egui::Response, name: &str) {
        if self.claude.composer.is_some() && self.claude.still_menu.as_deref() == Some(name) {
            self.claude.still_menu = None;
            egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(resp));
        }
    }

    fn claude_usage_card(&self, ui: &mut Ui, t: &Theme) {
        egui::Frame::new().inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.spacing_mut().interact_size.y = 18.0;
            let row = |ui: &mut Ui, k: &str, v: String, dim: bool| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(k).color(if dim { t.text2 } else { t.text }).size(12.5));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(v).color(t.text).size(12.5));
                    });
                });
            };
            let agent = self.claude.current();
            let limits = self.claude.limits_for(agent);
            if limits.is_empty() {
                ui.label(
                    RichText::new("Shows after your first message.")
                        .color(t.text3)
                        .size(12.0),
                );
            }
            for (k, wd) in limits.iter().enumerate() {
                if k > 0 {
                    ui.add_space(6.0);
                }
                row(
                    ui,
                    &process::window_name(&wd.name),
                    format!("{:.0}%", wd.used * 100.0),
                    false,
                );
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
                ui.painter().rect_filled(r, 2, t.border);
                let mut f = r;
                f.set_width(r.width() * wd.used.clamp(0.0, 1.0) as f32);
                ui.painter()
                    .rect_filled(f, 2, if wd.used >= 0.85 { t.warn } else { agent.accent(t) });
                if wd.resets > 0 {
                    ui.label(
                        RichText::new(format!("Resets {}", reset_time(wd.resets)))
                            .color(t.text3)
                            .size(11.0),
                    );
                }
            }
            if let Some(c) = self
                .claude
                .chat()
                .filter(|c| c.usage != Usage::default() || c.context > 0)
            {
                ui.add_space(8.0);
                let line = ui
                    .allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover())
                    .0;
                ui.painter()
                    .hline(line.x_range(), line.center().y, Stroke::new(1.0, t.border));
                ui.add_space(6.0);
                if !c.model_name.is_empty() {
                    row(ui, "Model", self.claude.model_label(c.agent, &c.model_name), true);
                }
                row(ui, "Context now", tokens(c.context), true);
                row(ui, "Input", tokens(c.usage.input + c.usage.cache_write), true);
                row(ui, "Read from cache", tokens(c.usage.cache_read), true);
                row(ui, "Output", tokens(c.usage.output), true);
                if c.cost > 0.0 {
                    row(ui, "API price equivalent", format!("${:.2}", c.cost), true);
                }
            }
        });
    }
}

fn expire_asks(chat: &mut Chat) {
    for it in &mut chat.items {
        if let Item::Ask { state, .. } = it
            && *state == AskState::Waiting
        {
            *state = AskState::Expired;
        }
    }
}

/// An empty assistant block left by a stop.
fn drop_empty(chat: &mut Chat) {
    chat.items
        .retain(|i| !matches!(i, Item::Assistant(t) if t.trim().is_empty()));
}

fn title_of(text: &str) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    let mut s: String = first.chars().take(48).collect();
    if first.chars().count() > 48 {
        s.push('…');
    }
    if s.is_empty() { "New chat".into() } else { s }
}

/// The composer's text: line height, and the most lines it grows to.
const LINE_H: f32 = 18.5;
const MAX_LINES: f32 = 8.0;

/// The settings key holding an agent's account.
fn account_key(a: Agent) -> String {
    format!("agent_account:{}", a.name().to_lowercase())
}

/// The model the agent's own config names, used when no `--model` is
/// passed: Claude Code's `ANTHROPIC_MODEL` or `settings.json` `model`
/// (aliases like "opus" mapped to their exact id), Codex's `config.toml`
/// top-level `model`.
fn config_model(a: Agent) -> Option<String> {
    let m = match a {
        Agent::Claude => std::env::var("ANTHROPIC_MODEL").ok().or_else(|| {
            let dir = accounts::folder(a, &accounts::current(a))?;
            let text = std::fs::read_to_string(dir.join("settings.json")).ok()?;
            let v: Value = serde_json::from_str(&text).ok()?;
            v["model"].as_str().map(str::to_string)
        }),
        Agent::Codex => {
            let dir = accounts::folder(a, &accounts::current(a))?;
            let text = std::fs::read_to_string(dir.join("config.toml")).ok()?;
            toml_model(&text)
        }
    }?;
    let m = m.trim().to_string();
    (!m.is_empty()).then(|| match a {
        Agent::Claude => model_id(&m),
        Agent::Codex => m,
    })
}

/// A Codex model id as its list names it, for before the list has come:
/// `gpt-5.6-terra` → "GPT-5.6-Terra".
fn codex_name(id: &str) -> String {
    id.split('-')
        .map(|w| {
            if w.eq_ignore_ascii_case("gpt") {
                return "GPT".to_string();
            }
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default()
        })
        .collect::<Vec<String>>()
        .join("-")
}

/// `model = "…"` before the first table of a TOML file.
fn toml_model(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            break;
        }
        if let Some((k, v)) = line.split_once('=')
            && k.trim() == "model"
        {
            let v = v.split('#').next().unwrap_or("").trim();
            return Some(v.trim_matches(|c| c == '"' || c == '\'').to_string());
        }
    }
    None
}

fn exit_message(agent: Agent, err: &str) -> String {
    let low = err.to_lowercase();
    let (name, cmd) = match agent {
        Agent::Claude => ("Claude Code", "claude"),
        Agent::Codex => ("Codex", "codex"),
    };
    if low.contains("login") || low.contains("log in") || low.contains("auth") || low.contains("api key") {
        format!("{name} isn't signed in. Run {cmd} in a terminal once to sign in, then try again.")
    } else if err.is_empty() {
        format!("{name} stopped unexpectedly. Send your message again to pick up where it left off.")
    } else {
        format!("{name} stopped: {err}")
    }
}

fn tokens(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.2}M", n as f64 / 1e6),
    }
}

fn reset_time(unix_s: i64) -> String {
    let Ok(ts) = jiff::Timestamp::from_second(unix_s) else {
        return String::new();
    };
    let z = ts.to_zoned(jiff::tz::TimeZone::system());
    let today = jiff::Zoned::now().date();
    if z.date() == today {
        format!("at {}", z.strftime("%H:%M"))
    } else {
        z.strftime("%a %H:%M").to_string()
    }
}

fn short_time(ms: i64) -> String {
    let Ok(ts) = jiff::Timestamp::from_millisecond(ms) else {
        return String::new();
    };
    let z = ts.to_zoned(jiff::tz::TimeZone::system());
    if z.date() == jiff::Zoned::now().date() {
        z.strftime("%H:%M").to_string()
    } else {
        z.strftime("%b %-d").to_string()
    }
}

fn file_name(p: &str) -> &str {
    p.rsplit(['/', '\\']).next().unwrap_or(p)
}

/// A plain line for a tool call (`None` hides it: internal plumbing).
/// `asking` words it as what Claude wants to do.
pub fn tool_label(name: &str, input: &Value, asking: bool) -> Option<String> {
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    // Scripts: whichever name Claude used for the file.
    let file = || {
        ["file", "path", "file_path"]
            .iter()
            .find_map(|k| input.get(*k).and_then(Value::as_str))
            .map(|f| file_name(f).to_string())
            .unwrap_or_default()
    };
    let first_line = |x: String| x.lines().next().unwrap_or("").chars().take(80).collect::<String>();
    let (done, ask): (String, String) = match name.strip_prefix("mcp__opentern__").unwrap_or(name) {
        "ToolSearch" => return None,
        "Read" => (
            format!("Read {}", file_name(&s("file_path"))),
            format!("Read {}", s("file_path")),
        ),
        "Write" => (
            format!("Wrote {}", file_name(&s("file_path"))),
            format!("Create {}", file_name(&s("file_path"))),
        ),
        "Edit" | "MultiEdit" | "NotebookEdit" => (
            format!("Edited {}", file_name(&s("file_path"))),
            format!("Edit {}", file_name(&s("file_path"))),
        ),
        "Bash" | "PowerShell" => (
            format!("Ran {}", first_line(s("command"))),
            if s("description").is_empty() {
                "Run a command".to_string()
            } else {
                s("description")
            },
        ),
        "Glob" => (
            format!("Found files: {}", s("pattern")),
            format!("Find files: {}", s("pattern")),
        ),
        "Grep" => (
            format!("Searched for {}", s("pattern")),
            format!("Search for {}", s("pattern")),
        ),
        "WebSearch" => (
            format!("Searched the web: {}", s("query")),
            format!("Search the web: {}", s("query")),
        ),
        "WebFetch" => (format!("Read {}", s("url")), format!("Open {}", s("url"))),
        "TodoWrite" => ("Updated its plan".into(), "Update its plan".into()),
        "Task" | "Agent" => (
            format!("Started a helper: {}", s("description")),
            format!("Start a helper: {}", s("description")),
        ),
        "get_chart" => ("Looked at the chart".into(), "Look at the chart".into()),
        "get_bars" => ("Read the chart's bars".into(), "Read the chart's bars".into()),
        "search_symbols" => (
            format!("Searched symbols: {}", s("query")),
            format!("Search symbols: {}", s("query")),
        ),
        "open_symbol" => (format!("Opened {}", s("symbol")), format!("Open {}", s("symbol"))),
        "set_timeframe" => (
            format!("Switched to {}", s("timeframe")),
            format!("Switch to {}", s("timeframe")),
        ),
        "list_scripts" => ("Listed your scripts".into(), "List your scripts".into()),
        "check_script" => (format!("Checked {}", file()), format!("Check {}", file())),
        "add_script_to_chart" => (
            format!("Added {} to the chart", file()),
            format!("Add {} to the chart", file()),
        ),
        "remove_from_chart" => (format!("Removed {}", s("name")), format!("Remove {}", s("name"))),
        "list_indicators" => ("Looked up indicators".into(), "Look up indicators".into()),
        "add_indicator" => (format!("Added {}", s("id")), format!("Add {}", s("id"))),
        "strategy_report" => ("Read the Strategy Report".into(), "Read the Strategy Report".into()),
        "open_in_editor" => (
            format!("Opened {} in the editor", file()),
            format!("Open {} in the editor", file()),
        ),
        "scripting_guide" => ("Read the scripting guide".into(), "Read the scripting guide".into()),
        "chart_view" => {
            let what = match s("action").as_str() {
                "chart_type" => format!("chart type to {}", s("type")),
                "go_to" => format!("view to {}", s("from")),
                "layout" => "chart layout".to_string(),
                "compare" => format!("{} comparison", s("symbol")),
                _ => "chart view".to_string(),
            };
            (format!("Changed the {what}"), format!("Change the {what}"))
        }
        "indicator" => match s("action").as_str() {
            "set" => (
                format!("Changed {}'s inputs", s("name")),
                format!("Change {}'s inputs", s("name")),
            ),
            "get" => (format!("Looked at {}", s("name")), format!("Look at {}", s("name"))),
            a => (format!("{a} {}", s("name")), format!("{a} {}", s("name"))),
        },
        "drawings" => match s("action").as_str() {
            "add" => {
                let n = input["drawings"].as_array().map_or(1, |v| v.len());
                let what = if n == 1 {
                    "a drawing".to_string()
                } else {
                    format!("{n} drawings")
                };
                (format!("Drew {what}"), format!("Draw {what}"))
            }
            "remove" | "clear" => ("Removed drawings".into(), "Remove drawings".into()),
            _ => ("Looked at the drawings".into(), "Look at the drawings".into()),
        },
        "replay" => match s("action").as_str() {
            "start" => (
                format!("Started a replay at {}", s("time")),
                format!("Start a replay at {}", s("time")),
            ),
            "step" => ("Stepped the replay".into(), "Step the replay".into()),
            "stop" => ("Ended the replay".into(), "End the replay".into()),
            a => (format!("Replay: {a}"), format!("Replay: {a}")),
        },
        "snapshot" => (
            "Took a picture of the chart".into(),
            "Take a picture of the chart".into(),
        ),
        "watchlist" => (
            format!("Watchlist: {}", s("action")),
            format!("Watchlist: {}", s("action")),
        ),
        "screener" => match s("action").as_str() {
            "run" => ("Ran the screener".into(), "Run the screener".into()),
            _ => ("Used the screener".into(), "Use the screener".into()),
        },
        "strategy_settings" => ("Strategy settings".into(), "Change the strategy settings".into()),
        "strategy_lab" => match s("action").as_str() {
            "run" => ("Ran the Strategy Lab".into(), "Run the Strategy Lab".into()),
            "apply" => ("Applied a lab result".into(), "Apply a lab result".into()),
            _ => ("Used the Strategy Lab".into(), "Use the Strategy Lab".into()),
        },
        "paper_trading" => match s("action").as_str() {
            "order" => (
                format!("Paper {} {} {}", s("side"), input["qty"], s("symbol")),
                format!("Paper {} {} {}", s("side"), input["qty"], s("symbol")),
            ),
            "account" => ("Looked at the paper account".into(), "Look at the paper account".into()),
            a => (format!("Paper trading: {a}"), format!("Paper trading: {a}")),
        },
        "alerts" => (format!("Alerts: {}", s("action")), format!("Alerts: {}", s("action"))),
        "open_panel" => (format!("Opened {}", s("tab")), format!("Open {}", s("tab"))),
        "seasonality" => ("Read the seasonality".into(), "Read the seasonality".into()),
        "custom_index" => match s("action").as_str() {
            "create" => (
                format!("Made the index {}", s("name")),
                format!("Make the index {}", s("name")),
            ),
            "edit" => (format!("Edited {}", s("name")), format!("Edit {}", s("name"))),
            "rebalance" => (format!("Rebalanced {}", s("name")), format!("Rebalance {}", s("name"))),
            "delete" => (format!("Deleted {}", s("name")), format!("Delete {}", s("name"))),
            "compare" => ("Compared the indices".into(), "Compare the indices".into()),
            "list" => ("Listed the indices".into(), "List the indices".into()),
            _ if s("name").is_empty() => ("Used the indices".into(), "Use the indices".into()),
            _ => (format!("Looked at {}", s("name")), format!("Look at {}", s("name"))),
        },
        other => (other.replace('_', " "), other.replace('_', " ")),
    };
    Some(if asking { ask } else { done })
}

/// The part of a permission prompt worth showing: the command, or the
/// change.
fn ask_detail(tool: &str, input: &Value, description: &str) -> String {
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    // Codex sends its change as a ready diff.
    if !s("diff").is_empty() {
        return s("diff");
    }
    match tool {
        "Bash" | "PowerShell" => s("command"),
        "Write" => {
            let content = s("content");
            let n = content.lines().count();
            let mut out: String = content.lines().take(14).collect::<Vec<_>>().join("\n");
            if n > 14 {
                out.push_str(&format!("\n… {} more lines", n - 14));
            }
            out
        }
        "Edit" => {
            let (old, new) = (s("old_string"), s("new_string"));
            let mut out = String::new();
            for l in old.lines().take(12) {
                out.push_str(&format!("- {l}\n"));
            }
            for l in new.lines().take(12) {
                out.push_str(&format!("+ {l}\n"));
            }
            out.trim_end().to_string()
        }
        "WebFetch" => s("url"),
        _ if description.is_empty() => serde_json::to_string_pretty(input).unwrap_or_default(),
        _ => description.to_string(),
    }
}

/// A permission choice: its key, words and answer.
type Choice = (&'static str, &'static str, Option<(bool, bool)>);

/// A popup anchor at `resp`'s left and right, on the composer's top edge
/// (`top`, from last frame), so menus sit above the composer, not over it.
fn above(resp: &egui::Response, top: f32) -> egui::Rect {
    let y = if top > 0.0 {
        top.min(resp.rect.top())
    } else {
        resp.rect.top()
    };
    egui::Rect::from_min_max(egui::pos2(resp.rect.left(), y), egui::pos2(resp.rect.right(), y))
}

/// The icon for a tool.
fn tool_icon(name: &str) -> &'static str {
    match name.strip_prefix("mcp__opentern__").unwrap_or(name) {
        "Read" => ph::FILE_TEXT,
        "Write" => ph::FILE_PLUS,
        "Edit" | "MultiEdit" | "NotebookEdit" => ph::PENCIL_SIMPLE,
        "Bash" | "PowerShell" => ph::TERMINAL_WINDOW,
        "Glob" => ph::FOLDER_OPEN,
        "Grep" | "search_symbols" => ph::MAGNIFYING_GLASS,
        "WebSearch" | "WebFetch" => ph::GLOBE,
        "TodoWrite" => ph::LIST_CHECKS,
        "Task" | "Agent" => ph::ROBOT,
        "get_chart" => ph::CHART_LINE,
        "get_bars" => ph::TABLE,
        "open_symbol" => ph::CHART_LINE_UP,
        "set_timeframe" => ph::CLOCK,
        "list_scripts" => ph::FILES,
        "check_script" => ph::SEAL_CHECK,
        "add_script_to_chart" => ph::PLUS_CIRCLE,
        "remove_from_chart" => ph::MINUS_CIRCLE,
        "list_indicators" | "add_indicator" => ph::FUNCTION,
        "strategy_report" => ph::CHART_BAR,
        "open_in_editor" => ph::CODE,
        "scripting_guide" => ph::BOOK_OPEN,
        "chart_view" => ph::SLIDERS_HORIZONTAL,
        "indicator" => ph::FUNCTION,
        "drawings" => ph::PENCIL_LINE,
        "replay" => ph::REWIND,
        "snapshot" => ph::CAMERA,
        "watchlist" => ph::LIST_BULLETS,
        "screener" => ph::FUNNEL,
        "strategy_settings" => ph::GEAR_SIX,
        "strategy_lab" => ph::FLASK,
        "paper_trading" => ph::WALLET,
        "alerts" => ph::ALARM,
        "open_panel" => ph::LAYOUT,
        "seasonality" => ph::CALENDAR_BLANK,
        "custom_index" => ph::CHART_PIE_SLICE,
        _ => ph::WRENCH,
    }
}

fn is_waiting(i: &Item) -> bool {
    matches!(
        i,
        Item::Ask {
            state: AskState::Waiting,
            ..
        }
    )
}

/// A tool call, or a prompt already answered: shown as a step.
fn is_step(i: &Item) -> bool {
    matches!(i, Item::Tool { .. }) || matches!(i, Item::Ask { state, .. } if *state != AskState::Waiting)
}

/// What a click in a box of steps asked for.
struct Steps {
    /// Fold or unfold the box.
    fold: bool,
    /// Show or hide one step's result: (chat, item index).
    toggle: Option<(u64, usize)>,
}

/// A run of steps in one quiet box. Long finished runs fold to one line
/// ("5 steps · the last one"); a click opens them.
#[allow(clippy::too_many_arguments)]
fn steps(
    ui: &mut Ui,
    t: &Theme,
    items: &[Item],
    first: usize,
    open: bool,
    foldable: bool,
    expanded: &HashSet<(u64, usize)>,
    chat: u64,
) -> Steps {
    let mut out = Steps {
        fold: false,
        toggle: None,
    };
    egui::Frame::new()
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.border.gamma_multiply(0.7)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(4, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            if foldable {
                let last = items.iter().rev().find_map(step_label).unwrap_or_default();
                let errors = items
                    .iter()
                    .filter(|i| matches!(i, Item::Tool { error: true, .. }))
                    .count();
                let head = format!("{} steps", items.len());
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                let hover = ui.rect_contains_pointer(r);
                let p = ui.painter();
                if hover {
                    p.rect_filled(r, 7, t.hover_wash());
                }
                p.text(
                    egui::pos2(r.left() + 14.0, r.center().y),
                    egui::Align2::CENTER_CENTER,
                    ph::LIST_CHECKS,
                    theme::regular(14.0),
                    t.text2,
                );
                let hf = theme::medium(12.5);
                let hg = p.layout_no_wrap(head, hf, t.text);
                let hw = hg.size().x;
                p.galley(
                    egui::pos2(r.left() + 30.0, r.center().y - hg.size().y / 2.0),
                    hg,
                    t.text,
                );
                let mut x = r.left() + 30.0 + hw + 8.0;
                if errors > 0 {
                    let g = p.text(
                        egui::pos2(x, r.center().y),
                        egui::Align2::LEFT_CENTER,
                        format!("{} {errors}", ph::WARNING),
                        theme::regular(12.0),
                        t.warn,
                    );
                    x = g.right() + 8.0;
                }
                if !open {
                    let f = theme::regular(12.5);
                    let rest = crate::format::elide_to(p, &last, &f, r.right() - 30.0 - x);
                    p.text(egui::pos2(x, r.center().y), egui::Align2::LEFT_CENTER, rest, f, t.text3);
                }
                p.text(
                    egui::pos2(r.right() - 12.0, r.center().y),
                    egui::Align2::CENTER_CENTER,
                    if open { ph::CARET_UP } else { ph::CARET_DOWN },
                    theme::regular(12.0),
                    t.text3,
                );
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    out.fold = true;
                }
                if !open {
                    return;
                }
                let line = ui
                    .allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover())
                    .0;
                ui.painter().hline(
                    line.x_range().shrink(8.0),
                    line.center().y,
                    Stroke::new(1.0, t.border.gamma_multiply(0.6)),
                );
            }
            for (k, it) in items.iter().enumerate() {
                let index = first + k;
                match it {
                    Item::Tool {
                        tool,
                        label,
                        result,
                        error,
                        ..
                    } => {
                        let shown = expanded.contains(&(chat, index));
                        if step_row(ui, t, tool_icon(tool), label, result.is_some(), *error, shown) && result.is_some()
                        {
                            out.toggle = Some((chat, index));
                        }
                        if shown && let Some(r) = result {
                            let text: String = r.chars().take(4000).collect();
                            egui::Frame::new().inner_margin(Margin::symmetric(6, 4)).show(ui, |ui| {
                                code_block(ui, t, &text, "", (chat, index));
                            });
                        }
                    }
                    Item::Ask { label, state, .. } => {
                        let (icon, word) = match state {
                            AskState::Allowed => (ph::CHECK, "Allowed"),
                            AskState::Denied => (ph::PROHIBIT, "Declined"),
                            _ => (ph::CLOCK, "Not answered"),
                        };
                        step_row(ui, t, icon, &format!("{word}: {label}"), true, false, false);
                    }
                    _ => {}
                }
            }
        });
    out
}

fn step_label(i: &Item) -> Option<String> {
    match i {
        Item::Tool { label, .. } => Some(label.clone()),
        _ => None,
    }
}

/// One step: icon, what it did, and its state. Returns whether it was
/// clicked.
fn step_row(ui: &mut Ui, t: &Theme, icon: &str, label: &str, done: bool, error: bool, open: bool) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let hover = done && ui.rect_contains_pointer(r);
    let p = ui.painter();
    if hover {
        p.rect_filled(r, 7, t.hover_wash());
    }
    p.text(
        egui::pos2(r.left() + 14.0, r.center().y),
        egui::Align2::CENTER_CENTER,
        icon,
        theme::regular(13.5),
        if error { t.warn } else { t.text3 },
    );
    let right = egui::pos2(r.right() - 12.0, r.center().y);
    if !done {
        let a = ui.input(|i| i.time) as f32 * 6.0;
        let pts: Vec<egui::Pos2> = (0..=18)
            .map(|k| {
                let ang = a + k as f32 / 18.0 * 4.4;
                right + vec2(ang.cos(), ang.sin()) * 5.0
            })
            .collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.6, t.text3)));
        ui.ctx().request_repaint();
    } else if error {
        p.text(
            right,
            egui::Align2::CENTER_CENTER,
            ph::WARNING,
            theme::regular(12.5),
            t.warn,
        );
    } else if hover || open {
        let g = if open { ph::CARET_UP } else { ph::CARET_DOWN };
        p.text(right, egui::Align2::CENTER_CENTER, g, theme::regular(12.0), t.text3);
    }
    let f = theme::regular(12.5);
    let text = crate::format::elide_to(p, label, &f, r.width() - 30.0 - 30.0);
    p.text(
        egui::pos2(r.left() + 30.0, r.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        f,
        if hover { t.text } else { t.text2 },
    );
    done && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// What a click on file chips asked for.
enum Chip {
    None,
    Open(usize),
    Remove(usize),
}

/// Attached files as chips that wrap: right-aligned under a message, or
/// left-aligned with a remove button in the composer.
fn file_chips(ui: &mut Ui, t: &Theme, files: &[String], width: f32, right: bool, removable: bool) -> Chip {
    let font = theme::regular(12.0);
    let (h, gap) = (28.0, 6.0);
    // Sizes, then rows.
    let sizes: Vec<(String, f32)> = files
        .iter()
        .map(|f| {
            let name = crate::format::elide_to(ui.painter(), file_name(f), &font, 150.0);
            let w = ui.painter().layout_no_wrap(name.clone(), font.clone(), t.text).size().x
                + 36.0
                + if removable { 20.0 } else { 0.0 };
            (name, w.min(width))
        })
        .collect();
    let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
    let mut x = 0.0;
    for (k, (_, w)) in sizes.iter().enumerate() {
        if x > 0.0 && x + w > width {
            rows.push(Vec::new());
            x = 0.0;
        }
        rows.last_mut().unwrap().push(k);
        x += w + gap;
    }
    let total_h = rows.len() as f32 * (h + gap) - gap;
    let (area, _) = ui.allocate_exact_size(vec2(width, total_h), Sense::hover());
    let mut out = Chip::None;
    for (r, row) in rows.iter().enumerate() {
        let row_w: f32 = row.iter().map(|k| sizes[*k].1).sum::<f32>() + gap * (row.len().saturating_sub(1)) as f32;
        let mut x = if right { area.right() - row_w } else { area.left() };
        let y = area.top() + r as f32 * (h + gap);
        for &k in row {
            let (name, w) = &sizes[k];
            let rect = egui::Rect::from_min_size(egui::pos2(x, y), vec2(*w, h));
            let resp = ui.interact(rect, ui.id().with(("chip", k, &files[k])), Sense::click());
            let hover = resp.hovered();
            let p = ui.painter();
            p.rect(
                rect,
                8,
                if hover { t.hover_wash() } else { t.card },
                Stroke::new(1.0, t.border.gamma_multiply(0.8)),
                egui::StrokeKind::Inside,
            );
            p.text(
                egui::pos2(rect.left() + 15.0, rect.center().y),
                egui::Align2::CENTER_CENTER,
                attach::icon(&files[k]),
                theme::regular(14.0),
                t.text2,
            );
            p.text(
                egui::pos2(rect.left() + 28.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                name,
                font.clone(),
                t.text,
            );
            let mut on_x = false;
            if removable {
                let xr =
                    egui::Rect::from_center_size(egui::pos2(rect.right() - 13.0, rect.center().y), vec2(18.0, 18.0));
                let xb = ui.interact(xr, ui.id().with(("chip-x", k, &files[k])), Sense::click());
                on_x = xb.hovered();
                if on_x {
                    ui.painter().rect_filled(xr, 5, t.hover_wash());
                }
                ui.painter().text(
                    xr.center(),
                    egui::Align2::CENTER_CENTER,
                    ph::X,
                    theme::regular(11.5),
                    if on_x { t.text } else { t.text3 },
                );
                if xb.on_hover_text("Remove").clicked() {
                    out = Chip::Remove(k);
                }
            }
            if !on_x
                && resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(&files[k])
                    .clicked()
            {
                out = Chip::Open(k);
            }
            x += w + gap;
        }
    }
    out
}

/// The user's message: a soft bubble at the right, text left-aligned.
fn user_bubble(ui: &mut Ui, t: &Theme, text: &str, width: f32) {
    let pad = vec2(14.0, 10.0);
    let max_text = (width * 0.82 - pad.x * 2.0).max(60.0);
    let mut job = LayoutJob::single_section(
        text.trim().to_string(),
        TextFormat {
            font_id: theme::regular(13.0),
            color: t.text,
            line_height: Some(19.0),
            ..Default::default()
        },
    );
    job.wrap.max_width = max_text;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let size = galley.size() + pad * 2.0;
    let (row, _) = ui.allocate_exact_size(vec2(width, size.y), Sense::hover());
    let r = egui::Rect::from_min_size(egui::pos2(row.right() - size.x, row.top()), size);
    let resp = ui.interact(
        r,
        ui.id().with(("bubble", text.len(), row.top() as i32)),
        Sense::click(),
    );
    let p = ui.painter();
    p.rect_filled(r, 14, if t.dark { t.hover } else { t.sidebar });
    p.galley(r.min + pad, galley, t.text);
    resp.context_menu(|ui| {
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(text.to_string());
            ui.close();
        }
    });
}

/// A permission prompt, Claude Code style: what it wants to do, the change,
/// and numbered choices (1-3 on the keyboard). Returns (allow, always).
#[allow(clippy::too_many_arguments)]
fn ask_card(
    ui: &mut Ui,
    t: &Theme,
    agent: Agent,
    tool: &str,
    label: &str,
    detail: &str,
    can_always: bool,
    key: (u64, usize),
) -> Option<(bool, bool)> {
    let mut out = None;
    egui::Frame::new()
        .fill(t.card)
        .stroke(Stroke::new(1.0, agent.accent(t).gamma_multiply(0.55)))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new(tool_icon(tool)).color(agent.accent(t)).size(15.0));
                ui.add(egui::Label::new(RichText::new(label).font(theme::semibold(13.0))).truncate());
            });
            if !detail.is_empty() {
                ui.add_space(4.0);
                if matches!(tool, "Edit" | "MultiEdit") || detail.starts_with("+ ") || detail.starts_with("- ") {
                    diff_block(ui, t, detail);
                } else {
                    code_block(ui, t, detail, "", key);
                }
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!("Allow {} to do this?", agent.name()))
                    .color(t.text2)
                    .size(12.5),
            );
            ui.add_space(2.0);
            let mut choices: Vec<Choice> = vec![("1", "Allow", Some((true, false)))];
            if can_always {
                choices.push(("2", "Allow for this chat", Some((true, true))));
            }
            let n = if can_always { "3" } else { "2" };
            choices.push((n, "Deny", Some((false, false))));
            for (k, text, v) in choices {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                let hover = ui.rect_contains_pointer(r);
                let p = ui.painter();
                if hover {
                    p.rect_filled(r, 7, t.hover_wash());
                }
                let badge = egui::Rect::from_center_size(egui::pos2(r.left() + 16.0, r.center().y), vec2(18.0, 18.0));
                p.rect_stroke(badge, 5, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                p.text(
                    badge.center(),
                    egui::Align2::CENTER_CENTER,
                    k,
                    FontId::monospace(10.5),
                    t.text3,
                );
                p.text(
                    egui::pos2(r.left() + 34.0, r.center().y),
                    egui::Align2::LEFT_CENTER,
                    text,
                    theme::medium(12.5),
                    if hover { t.text } else { t.text2 },
                );
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    out = v;
                }
            }
        });
    out
}

/// Removed and added lines, tinted.
fn diff_block(ui: &mut Ui, t: &Theme, diff: &str) {
    egui::Frame::new()
        .fill(t.bg)
        .stroke(Stroke::new(1.0, t.border.gamma_multiply(0.6)))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(0, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            for line in diff.lines() {
                let (bg, fg) = if line.starts_with("- ") {
                    (t.neg.gamma_multiply(0.16), t.text)
                } else if line.starts_with("+ ") {
                    (t.pos.gamma_multiply(0.16), t.text)
                } else {
                    (Color32::TRANSPARENT, t.text2)
                };
                egui::Frame::new()
                    .fill(bg)
                    .inner_margin(Margin::symmetric(8, 1))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(egui::Label::new(RichText::new(line).font(FontId::monospace(11.5)).color(fg)).wrap());
                    });
            }
        });
}

/// A rounded, quiet button with an optional icon and a caret.
/// How wide `pill` draws.
fn pill_width(ui: &Ui, icon: Option<&str>, text: &str, caret: bool) -> f32 {
    let text_w = if text.is_empty() {
        -4.0
    } else {
        ui.painter()
            .layout_no_wrap(text.to_string(), theme::medium(12.0), Color32::WHITE)
            .size()
            .x
    };
    text_w + 20.0 + if icon.is_some() { 18.0 } else { 0.0 } + if caret { 14.0 } else { 0.0 }
}

fn pill(ui: &mut Ui, t: &Theme, icon: Option<&str>, text: &str, caret: bool) -> egui::Response {
    let f = theme::medium(12.0);
    let w = pill_width(ui, icon, text, caret);
    let (r, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
    let enabled = ui.is_enabled();
    let hover = enabled && ui.rect_contains_pointer(r);
    let p = ui.painter();
    if hover {
        p.rect_filled(r, 8, t.hover_wash());
    }
    let col = if !enabled {
        t.text3
    } else if hover {
        t.text
    } else {
        t.text2
    };
    let mut x = r.left() + 10.0;
    if let Some(i) = icon {
        p.text(
            egui::pos2(x, r.center().y),
            egui::Align2::LEFT_CENTER,
            i,
            theme::regular(13.0),
            col,
        );
        x += 18.0;
    }
    p.text(egui::pos2(x, r.center().y), egui::Align2::LEFT_CENTER, text, f, col);
    if caret {
        p.text(
            egui::pos2(r.right() - 10.0, r.center().y),
            egui::Align2::RIGHT_CENTER,
            ph::CARET_DOWN,
            theme::regular(10.5),
            t.text3,
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The round send button (Claude's colour when there's something to
/// send), or Stop while Claude works.
fn send_button(ui: &mut Ui, t: &Theme, agent: Agent, busy: bool, can_send: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    let hover = ui.rect_contains_pointer(r);
    let p = ui.painter();
    let c = r.center();
    if busy {
        p.circle_filled(c, 15.0, if hover { t.text } else { t.text2 });
        p.rect_filled(egui::Rect::from_center_size(c, vec2(10.0, 10.0)), 2, t.card);
    } else if can_send {
        let fill = if hover {
            agent.accent(t).gamma_multiply(0.85)
        } else {
            agent.accent(t)
        };
        p.circle_filled(c, 15.0, fill);
        p.text(
            c,
            egui::Align2::CENTER_CENTER,
            ph::ARROW_UP,
            theme::semibold(15.0),
            agent.on_accent(t),
        );
    } else {
        p.circle_filled(c, 15.0, t.hover);
        p.text(
            c,
            egui::Align2::CENTER_CENTER,
            ph::ARROW_UP,
            theme::semibold(15.0),
            t.text3,
        );
    }
    let tip = if busy { "Stop" } else { "Send (Enter)" };
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

/// A compact menu row: a name, an optional note, a check when picked.
fn menu_pick(ui: &mut Ui, t: &Theme, label: &str, hint: &str, on: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let hover = ui.rect_contains_pointer(r);
    let p = ui.painter();
    if hover || on {
        p.rect_filled(r, 7, if on { t.accent_soft() } else { t.hover_wash() });
    }
    let g = p.text(
        egui::pos2(r.left() + 10.0, r.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        theme::medium(12.5),
        t.text,
    );
    if !hint.is_empty() {
        p.text(
            egui::pos2(g.right() + 8.0, r.center().y),
            egui::Align2::LEFT_CENTER,
            hint,
            theme::regular(11.5),
            t.text3,
        );
    }
    if on {
        p.text(
            egui::pos2(r.right() - 10.0, r.center().y),
            egui::Align2::RIGHT_CENTER,
            ph::CHECK,
            theme::regular(13.0),
            t.accent,
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The last thing said in a chat, for the chats list.
fn preview_of(c: &Chat) -> String {
    c.items
        .iter()
        .rev()
        .find_map(|i| match i {
            Item::User(s) | Item::Assistant(s) if !s.trim().is_empty() => Some(
                s.lines()
                    .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with("```"))
                    .unwrap_or("")
                    .trim()
                    .trim_start_matches('#')
                    .trim()
                    .replace("**", "")
                    .replace('`', ""),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

/// Code (or a tool's output) in a box, with its language and a copy
/// button.
fn code_block(ui: &mut Ui, t: &Theme, text: &str, lang: &str, key: impl std::hash::Hash + std::fmt::Debug) {
    let id = ui.id().with(("code", key, text.len()));
    let has_lang = !lang.trim().is_empty();
    let out = egui::Frame::new()
        .fill(t.bg)
        .stroke(Stroke::new(1.0, t.border.gamma_multiply(0.6)))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin {
            left: 10,
            right: 28,
            top: if has_lang { 6 } else { 8 },
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            if has_lang {
                ui.label(RichText::new(lang.trim()).color(t.text3).size(11.0));
            }
            ui.add(
                egui::Label::new(
                    RichText::new(text.trim_end())
                        .font(FontId::monospace(11.5))
                        .color(t.text),
                )
                .wrap(),
            );
        });
    // The copy button floats at the top right; it takes no row of its own.
    let frame = out.response.rect;
    let btn = egui::Rect::from_min_size(egui::pos2(frame.right() - 26.0, frame.top() + 4.0), vec2(22.0, 22.0));
    let resp = ui.interact(btn, id.with("copy"), Sense::click());
    let now = ui.input(|i| i.time);
    let copied = ui.data(|d| d.get_temp::<f64>(id)).is_some_and(|c| now - c < 1.5);
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(btn, 5, t.hover_wash());
    }
    let (g, tip) = if copied {
        (ph::CHECK, "Copied")
    } else {
        (ph::COPY, "Copy")
    };
    p.text(
        btn.center(),
        egui::Align2::CENTER_CENTER,
        g,
        theme::regular(12.5),
        if resp.hovered() { t.text } else { t.text3 },
    );
    if resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip)
        .clicked()
    {
        ui.ctx().copy_text(text.trim_end().to_string());
        ui.data_mut(|d| d.insert_temp(id, now));
    }
    if copied {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
    }
}

/// Enough Markdown for chat replies: paragraphs, headings, lists, code
/// blocks, `inline code` and **bold**.
fn markdown(ui: &mut Ui, t: &Theme, text: &str, key: (u64, usize), scripts: &[String], open: &mut Option<String>) {
    let mut in_code = false;
    let mut lang = String::new();
    let mut code = String::new();
    let mut table: Vec<&str> = Vec::new();
    let mut n = 0;
    for line in text.lines() {
        let trimmed = line.trim_start();
        // A table is a run of lines that start with '|'.
        if !in_code && trimmed.starts_with('|') {
            table.push(trimmed);
            continue;
        }
        if !table.is_empty() {
            md_table(ui, t, &table);
            table.clear();
        }
        if let Some(rest) = trimmed.strip_prefix("```") {
            if in_code {
                code_block(ui, t, &code, &lang, (key, n));
                n += 1;
                code.clear();
            } else {
                lang = rest.trim().to_string();
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            code.push_str(line);
            code.push('\n');
        } else if trimmed.is_empty() {
            ui.add_space(4.0);
        } else if trimmed.len() >= 3 && trimmed.chars().all(|c| matches!(c, '-' | '*' | '_' | ' ')) {
            // A horizontal rule.
            ui.add_space(2.0);
            let r = ui
                .allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover())
                .0;
            ui.painter()
                .hline(r.x_range(), r.center().y, Stroke::new(1.0, t.border));
            ui.add_space(2.0);
        } else if let Some(q) = trimmed.strip_prefix('>') {
            // A quote: a bar at the left, the text a little dimmer.
            let r = ui
                .horizontal_top(|ui| {
                    ui.add_space(12.0);
                    ui.vertical(|ui| md_line(ui, t, q.trim_start(), scripts, open));
                })
                .response
                .rect;
            ui.painter()
                .vline(r.left() + 3.0, r.y_range(), Stroke::new(2.5, t.border));
        } else {
            md_line(ui, t, line, scripts, open);
        }
    }
    if !table.is_empty() {
        md_table(ui, t, &table);
    }
    if in_code {
        // Still streaming: show what's there.
        code_block(ui, t, &code, &lang, (key, n));
    }
}

/// A Markdown table's cells (`| a | b |`).
fn table_cells(line: &str) -> Vec<String> {
    let l = line.trim();
    let l = l.strip_prefix('|').unwrap_or(l);
    let l = l.strip_suffix('|').unwrap_or(l);
    l.split('|').map(|c| c.trim().to_string()).collect()
}

/// A table cell's text with **bold** and `code` styled.
fn cell_job(code_c: Color32, text: &str, font: FontId, color: Color32, wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let base = TextFormat {
        font_id: font.clone(),
        color,
        line_height: Some((font.size * 1.45).round()),
        ..Default::default()
    };
    let mut bold = false;
    let mut code = false;
    let mut rest = text;
    while !rest.is_empty() {
        let next = [rest.find("**"), rest.find('`')].into_iter().flatten().min();
        let end = next.unwrap_or(rest.len());
        if end > 0 {
            let f = if code {
                TextFormat {
                    font_id: FontId::monospace(font.size - 1.0),
                    color: code_c,
                    ..base.clone()
                }
            } else if bold {
                TextFormat {
                    font_id: theme::semibold(font.size),
                    ..base.clone()
                }
            } else {
                base.clone()
            };
            job.append(&rest[..end], 0.0, f);
        }
        rest = &rest[end..];
        if let Some(r) = rest.strip_prefix("**") {
            bold = !bold;
            rest = r;
        } else if let Some(r) = rest.strip_prefix('`') {
            code = !code;
            rest = r;
        }
    }
    job.wrap.max_width = wrap;
    job
}

/// A Markdown table: header shaded, rows ruled, columns sized to their
/// content and squeezed (text wraps) when the pane is narrow.
fn md_table(ui: &mut Ui, t: &Theme, lines: &[&str]) {
    let is_rule = |l: &str| l.contains('-') && l.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '));
    let rows: Vec<Vec<String>> = lines.iter().filter(|l| !is_rule(l)).map(|l| table_cells(l)).collect();
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if cols == 0 {
        return;
    }
    let pad = vec2(10.0, 6.0);
    let font = theme::regular(12.5);
    let head = theme::semibold(12.5);
    let avail = ui.available_width();
    // Natural widths, then shrunk to fit.
    let mut natural = vec![0.0f32; cols];
    for (ri, r) in rows.iter().enumerate() {
        for (c, cell) in r.iter().enumerate() {
            let f = if ri == 0 { head.clone() } else { font.clone() };
            let code_c = code_color(ui.ctx(), t);
            let g = ui.fonts_mut(|fo| fo.layout_job(cell_job(code_c, cell, f, t.text, f32::INFINITY)));
            natural[c] = natural[c].max(g.size().x + pad.x * 2.0);
        }
    }
    let total: f32 = natural.iter().sum();
    let widths: Vec<f32> = if total <= avail {
        natural.clone()
    } else {
        let min = 56.0f32;
        let flex: f32 = natural.iter().map(|w| (w - min).max(0.0)).sum();
        let spare = (avail - min * cols as f32).max(0.0);
        natural
            .iter()
            .map(|w| min + (w - min).max(0.0) * if flex > 0.0 { spare / flex } else { 0.0 })
            .collect()
    };
    let width: f32 = widths.iter().sum();
    // Lay out every cell, then paint.
    let mut grid = Vec::new();
    let mut heights = Vec::new();
    for (ri, r) in rows.iter().enumerate() {
        let mut row = Vec::new();
        let mut h = 0.0f32;
        for (c, w) in widths.iter().enumerate() {
            let cell = r.get(c).map(String::as_str).unwrap_or("");
            let (f, color) = if ri == 0 {
                (head.clone(), t.text)
            } else {
                (font.clone(), t.text)
            };
            let code_c = code_color(ui.ctx(), t);
            let g = ui.fonts_mut(|fo| fo.layout_job(cell_job(code_c, cell, f, color, w - pad.x * 2.0)));
            h = h.max(g.size().y);
            row.push(g);
        }
        heights.push(h + pad.y * 2.0);
        grid.push(row);
    }
    let height: f32 = heights.iter().sum();
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let p = ui.painter();
    let line = Stroke::new(1.0, t.border.gamma_multiply(0.8));
    p.rect_filled(rect, 8, t.card);
    let head_rect = egui::Rect::from_min_size(rect.min, vec2(width, heights[0]));
    p.rect_filled(
        head_rect,
        CornerRadius {
            nw: 8,
            ne: 8,
            sw: 0,
            se: 0,
        },
        t.hover_wash(),
    );
    let mut y = rect.top();
    for (ri, row) in grid.into_iter().enumerate() {
        let mut x = rect.left();
        for (c, g) in row.into_iter().enumerate() {
            p.galley(egui::pos2(x + pad.x, y + pad.y), g, t.text);
            x += widths[c];
            if c + 1 < cols {
                p.vline(
                    x,
                    (y + 4.0)..=(y + heights[ri] - 4.0),
                    Stroke::new(1.0, t.border.gamma_multiply(0.4)),
                );
            }
        }
        y += heights[ri];
        if ri + 1 < heights.len() {
            p.hline(rect.x_range(), y, line);
        }
    }
    p.rect_stroke(rect, 8, line, egui::StrokeKind::Inside);
}

fn md_line(ui: &mut Ui, t: &Theme, line: &str, scripts: &[String], open: &mut Option<String>) {
    let trimmed = line.trim_start();
    let indent = (line.len() - trimmed.len()) as f32 * 4.0;
    // A list item: a marker, then the text wrapped in its own column.
    let numbered = trimmed
        .split_once(". ")
        .filter(|(n, _)| !n.is_empty() && n.len() <= 3 && n.chars().all(|c| c.is_ascii_digit()));
    let item = if let Some(b) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
        Some(("•".to_string(), b))
    } else {
        numbered.map(|(n, b)| (format!("{n}."), b))
    };
    if let Some((marker, body)) = item {
        ui.horizontal_top(|ui| {
            ui.add_space(indent + 2.0);
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(marker).color(t.text3).size(13.5));
            inline(ui, t, body, theme::regular(13.5), scripts, open);
        });
        return;
    }
    if let Some(h) = trimmed.strip_prefix('#') {
        ui.add_space(6.0);
        inline(
            ui,
            t,
            h.trim_start_matches('#').trim(),
            theme::semibold(14.0),
            scripts,
            open,
        );
        return;
    }
    inline(ui, t, trimmed, theme::regular(13.5), scripts, open);
}

/// The script a word names, if it's a file in the scripts folder: its
/// path there, or a unique file with that name.
fn script_link(word: &str, scripts: &[String]) -> Option<String> {
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && !matches!(c, '_' | '-' | '.' | '/'));
    let w = w.trim_end_matches('.').trim_start_matches("./");
    if !matches!(w.rsplit('.').next(), Some("pine" | "luau" | "lua" | "js" | "ts" | "py")) {
        return None;
    }
    if scripts.iter().any(|s| s == w) {
        return Some(w.to_string());
    }
    let mut by_name = scripts.iter().filter(|s| file_name(s) == file_name(w));
    match (by_name.next(), by_name.next()) {
        (Some(s), None) => Some(s.clone()),
        _ => None,
    }
}

/// A wrapped run with `code` and **bold** spans. Script names are links:
/// a click puts the script's path in `open`.
fn inline(ui: &mut Ui, t: &Theme, body: &str, font: FontId, scripts: &[String], open: &mut Option<String>) {
    let mut job = LayoutJob::default();
    let line_height = Some((font.size * 1.55).round());
    let base = TextFormat {
        font_id: font.clone(),
        color: t.text,
        line_height,
        ..Default::default()
    };
    let code = TextFormat {
        font_id: FontId::monospace(font.size - 1.0),
        color: code_color(ui.ctx(), t),
        line_height,
        ..Default::default()
    };
    // (first char, end char, script) of each link.
    let mut links: Vec<(usize, usize, String)> = Vec::new();
    let mut chars = 0;
    let mut push = |job: &mut LayoutJob, text: &str, fmt: TextFormat, whole: bool, chars: &mut usize| {
        if whole {
            if let Some(f) = script_link(text, scripts) {
                links.push((*chars, *chars + text.chars().count(), f));
            }
        } else {
            // Plain text: look at each word.
            let mut at = *chars;
            for word in text.split_inclusive(char::is_whitespace) {
                let w = word.trim_end();
                if let Some(f) = script_link(w, scripts) {
                    let lead = w
                        .chars()
                        .take_while(|c| !c.is_alphanumeric() && *c != '.' && *c != '_')
                        .count();
                    let core = w.trim_matches(|c: char| !c.is_alphanumeric() && !matches!(c, '_' | '-' | '.' | '/'));
                    let core = core.trim_end_matches('.');
                    links.push((at + lead, at + lead + core.chars().count(), f));
                }
                at += word.chars().count();
            }
        }
        *chars += text.chars().count();
        job.append(text, 0.0, fmt);
    };
    let mut rest = body;
    while !rest.is_empty() {
        let next_code = rest.find('`');
        let next_bold = rest.find("**");
        match (next_code, next_bold) {
            (Some(c), b) if b.is_none_or(|b| c < b) => {
                push(&mut job, &rest[..c], base.clone(), false, &mut chars);
                let after = &rest[c + 1..];
                let end = after.find('`').unwrap_or(after.len());
                push(&mut job, &after[..end], code.clone(), true, &mut chars);
                rest = after.get(end + 1..).unwrap_or("");
            }
            (_, Some(b)) => {
                push(&mut job, &rest[..b], base.clone(), false, &mut chars);
                let after = &rest[b + 2..];
                let end = after.find("**").unwrap_or(after.len());
                let bold = TextFormat {
                    font_id: theme::semibold(font.size),
                    ..base.clone()
                };
                push(&mut job, &after[..end], bold, false, &mut chars);
                rest = after.get(end + 2..).unwrap_or("");
            }
            _ => {
                push(&mut job, rest, base.clone(), false, &mut chars);
                rest = "";
            }
        }
    }
    job.wrap.max_width = ui.available_width();
    if links.is_empty() {
        ui.add(egui::Label::new(job).wrap());
        return;
    }
    // With links: lay it out here, so a click can be traced to a word.
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let (rect, resp) = ui.allocate_exact_size(galley.size(), Sense::click());
    let hovered = resp.hover_pos().and_then(|p| {
        let at = galley.cursor_from_pos(p - rect.min).index.0;
        links.iter().find(|(a, b, _)| at >= *a && at < *b).cloned()
    });
    let p = ui.painter();
    p.galley(rect.min, galley.clone(), t.text);
    if let Some((a, b, f)) = &hovered {
        // Underline the link (it may wrap over two rows).
        let ra = galley.pos_from_cursor(egui::text::CCursor::new(*a));
        let rb = galley.pos_from_cursor(egui::text::CCursor::new(*b));
        let stroke = Stroke::new(1.0, code.color);
        if (ra.top() - rb.top()).abs() < 1.0 {
            let y = rect.top() + ra.bottom() - 3.0;
            p.hline((rect.left() + ra.left())..=(rect.left() + rb.left()), y, stroke);
        }
        let resp = resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!("Open {f} in the Script Editor"));
        if resp.clicked() {
            *open = Some(f.clone());
        }
    }
}

/// Inline code's colour for the chat on show: a soft terracotta for
/// Claude (like its own apps), a soft blue for Codex.
fn code_color(ctx: &egui::Context, t: &Theme) -> Color32 {
    let codex = ctx.data(|d| d.get_temp::<Agent>(egui::Id::new("agent-on-show"))) == Some(Agent::Codex);
    match (codex, t.dark) {
        (false, true) => Color32::from_rgb(0xE8, 0xA0, 0x84),
        (false, false) => Color32::from_rgb(0xB0, 0x4E, 0x30),
        (true, true) => Color32::from_rgb(0xA6, 0xC4, 0xEC),
        (true, false) => Color32::from_rgb(0x2F, 0x5C, 0x99),
    }
}

/// The agent's logo over its turn.
fn byline(ui: &mut Ui, t: &Theme, agent: Agent) {
    agent::show(ui, 16.0, agent, agent.tint(t));
}

/// A word with a soft light passing over it, while the agent works.
fn shimmer(ui: &mut Ui, t: &Theme, word: &str) {
    let now = ui.input(|i| i.time) as f32;
    let n = word.chars().count() as f32;
    // The highlight sweeps across, then rests a moment.
    let head = (now * 1.1).rem_euclid(1.6) / 1.2 * (n + 6.0) - 3.0;
    let mut job = LayoutJob::default();
    for (k, ch) in word.chars().enumerate() {
        let d = (k as f32 - head).abs();
        let glow = (1.0 - d / 3.0).clamp(0.0, 1.0);
        job.append(
            &ch.to_string(),
            0.0,
            TextFormat {
                font_id: theme::medium(12.5),
                color: crate::motion::lerp_color(t.text3, t.text, glow * 0.85),
                ..Default::default()
            },
        );
    }
    ui.label(job);
    ui.ctx().request_repaint();
}

/// An agent's logo on a soft halo of its colour.
fn halo(ui: &mut Ui, t: &Theme, agent: Agent, size: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(size * 1.8, size * 1.8), Sense::hover());
    let c = agent.accent(t);
    let k = if agent == Agent::Codex { 0.6 } else { 1.0 };
    for (rad, a) in [(0.9, 0.05), (0.72, 0.08), (0.56, 0.1)] {
        ui.painter()
            .circle_filled(r.center(), size * rad, crate::motion::with_alpha(c, a * k));
    }
    agent::paint(
        ui,
        egui::Rect::from_center_size(r.center(), vec2(size * 0.62, size * 0.62)),
        agent,
        agent.tint(t),
        0.0,
    );
}

/// The face's blink, `since` seconds after it showed: a quick one every
/// few seconds.
fn blink_at(since: f64) -> f32 {
    let period = 4.2;
    let k = (since - 1.2).rem_euclid(period);
    if since < 1.2 || k > 0.18 {
        0.0
    } else {
        (k / 0.18 * std::f64::consts::PI).sin() as f32
    }
}

/// Seconds until the next blink.
fn blink_wait(since: f64) -> f64 {
    let period = 4.2;
    if since < 1.2 {
        return 1.2 - since;
    }
    (period - (since - 1.2).rem_euclid(period)).max(0.02)
}

/// A row in the agent menu: logo, name, maker, a check when it's on.
fn agent_row(ui: &mut Ui, t: &Theme, a: Agent, on: bool, installed: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    let hover = ui.rect_contains_pointer(r);
    if hover || on {
        ui.painter().rect_filled(
            r,
            8,
            if hover {
                t.hover_wash()
            } else {
                t.hover_wash().gamma_multiply(0.6)
            },
        );
    }
    let badge = egui::pos2(r.left() + 22.0, r.center().y);
    ui.painter().circle_filled(
        badge,
        15.0,
        a.accent(t).gamma_multiply(if a == Agent::Codex { 0.08 } else { 0.14 }),
    );
    agent::paint(
        ui,
        egui::Rect::from_center_size(badge, vec2(17.0, 17.0)),
        a,
        a.tint(t),
        0.0,
    );
    let p = ui.painter();
    p.text(
        egui::pos2(r.left() + 46.0, r.center().y),
        egui::Align2::LEFT_CENTER,
        a.name(),
        theme::medium(13.0),
        t.text,
    );
    if !installed && !on {
        p.text(
            egui::pos2(r.right() - 12.0, r.center().y),
            egui::Align2::RIGHT_CENTER,
            "Not installed",
            theme::regular(11.5),
            t.text3,
        );
    }
    if on {
        p.text(
            egui::pos2(r.right() - 12.0, r.center().y),
            egui::Align2::RIGHT_CENTER,
            ph::CHECK,
            theme::regular(13.0),
            t.text,
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn labels() {
        assert_eq!(
            tool_label("Edit", &json!({"file_path": "/x/scripts/rsi.pine"}), false).as_deref(),
            Some("Edited rsi.pine")
        );
        assert_eq!(
            tool_label("mcp__opentern__open_symbol", &json!({"symbol": "ETHUSDT"}), true).as_deref(),
            Some("Open ETHUSDT")
        );
        assert_eq!(tool_label("ToolSearch", &json!({}), false), None);
        assert_eq!(title_of("Write me an RSI script\nwith bands"), "Write me an RSI script");
        assert_eq!(tokens(12_345), "12.3k");
        assert_eq!(model_name("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(model_name("claude-sonnet-4-5-20250929"), "Sonnet 4.5");
        assert_eq!(model_name("haiku"), "Haiku");
        assert_eq!(codex_name("gpt-5.6-terra"), "GPT-5.6-Terra");
        assert_eq!(
            toml_model("# hi\nmodel = \"gpt-6-luna\" # pick\n[x]\nmodel = \"no\""),
            Some("gpt-6-luna".into())
        );
        assert_eq!(toml_model("[profiles.a]\nmodel = \"no\""), None);
        assert_eq!(model_id("opus"), "claude-opus-5-5");
        assert_eq!(model_id("haiku"), "claude-haiku-5-5");
        assert_eq!(model_id("claude-opus-5-5"), "claude-opus-5-5");
        assert_eq!(model_id(""), "");
        let scripts = vec!["rsi_bands.pine".to_string(), "ideas/ema.pine".to_string()];
        assert_eq!(
            script_link("rsi_bands.pine,", &scripts).as_deref(),
            Some("rsi_bands.pine")
        );
        assert_eq!(script_link("ema.pine", &scripts).as_deref(), Some("ideas/ema.pine"));
        assert_eq!(script_link("other.pine", &scripts), None);
        assert_eq!(script_link("rsi_bands", &scripts), None);
    }

    /// The whole loop against a stand-in CLI: a turn that uses the app's
    /// tools, asks permission, opens a symbol, streams text and reports
    /// usage; then the chat is saved.
    #[cfg(unix)]
    #[test]
    fn a_turn_with_a_fake_cli() {
        use std::os::unix::fs::PermissionsExt;
        let mut d = crate::drive::Driver::new("claude");
        let fake = d.dir.join("fake_claude.py");
        std::fs::write(&fake, include_str!("fake_claude.py")).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        // SAFETY: only this test reads OPENTERN_CLAUDE.
        unsafe {
            std::env::set_var("OPENTERN_CLAUDE", &fake);
        }
        let before = d.app.chart.symbol.id.clone();
        // A script and an image go with the message.
        let script = d.dir.join("mine.pine");
        std::fs::write(&script, "//@version=6\nindicator(\"Mine\")").unwrap();
        let image = d.dir.join("shot.png");
        std::fs::write(&image, [0x89, b'P', b'N', b'G']).unwrap();
        d.app.claude_attach(vec![script.clone(), image.clone()]);
        d.app.claude.input = "What's on my chart?".into();
        d.app.claude_send();
        let id = d.app.claude.active.expect("a chat");
        d.wait(|a| {
            a.claude
                .chat()
                .is_some_and(|c| c.items.iter().any(|i| matches!(i, Item::Ask { .. })))
        });
        let i = d
            .app
            .claude
            .chat()
            .unwrap()
            .items
            .iter()
            .position(|i| matches!(i, Item::Ask { .. }))
            .unwrap();
        d.app.claude_answer(id, i, true, false);
        d.wait(|a| a.claude.chat().is_some_and(|c| !c.busy));
        let c = d.app.claude.chat().unwrap().clone();
        let reply: String = c
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Assistant(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert!(reply.contains(&format!("The chart was {before}.")), "{reply}");
        assert!(reply.contains("Write was allow."), "{reply}");
        assert!(reply.contains("ETHUSDT"), "{reply}");
        assert!(reply.contains("Blocks text,text,text,image,text"), "{reply}");
        assert!(reply.contains("File True"), "{reply}");
        // Auto is the default mode, as in the terminal.
        assert!(reply.ends_with("Mode auto"), "{reply}");
        assert!(d.app.claude.attached.is_empty());
        assert!(d.app.chart.symbol.id.contains("ETHUSDT"), "{}", d.app.chart.symbol.id);
        assert_eq!(c.title, "What's on my chart?");
        assert_eq!(c.session.as_deref(), Some("fake-session"));
        assert_eq!(c.context, 1003);
        assert_eq!(c.usage.cache_read, 1000);
        assert!(
            matches!(&c.items[2], Item::Tool { label, result: Some(r), .. } if label == "Looked at the chart" && *r == before)
        );
        assert_eq!(d.app.claude.limits[0].used, 0.25);
        // Saved for next time; the process stays for the next message, and
        // a new chat lets it go.
        let file = d.dir.join("claude").join("chats.json");
        d.wait(|_| std::fs::read_to_string(&file).is_ok_and(|s| s.contains("fake-session")));
        assert_eq!(d.app.claude.clients.len(), 1);
        d.app.new_chat();
        d.frame(vec![]);
        assert!(d.app.claude.clients.is_empty());
    }

    /// A script's name in a reply is a link: clicking along the line
    /// finds it, and it opens that script (and nothing else does).
    #[test]
    fn script_names_are_links() {
        let ctx = egui::Context::default();
        let t = crate::theme::Theme::by_name("dark");
        let scripts = vec!["rsi_bands.pine".to_string()];
        let mut hits = Vec::new();
        let run = |events: Vec<egui::Event>, time: f64, open: &mut Option<String>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 200.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| {
                inline(
                    ui,
                    &t,
                    "I created `rsi_bands.pine` and it works.",
                    theme::regular(13.5),
                    &scripts,
                    open,
                );
            });
            out.textures_delta.clear();
        };
        let mut time = 0.0;
        for x in (0..400).step_by(4) {
            let pos = egui::pos2(x as f32 + 0.5, 10.0);
            let mut open = None;
            for (events, k) in [
                (vec![egui::Event::PointerMoved(pos)], 0),
                (
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    }],
                    1,
                ),
                (
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    }],
                    2,
                ),
            ] {
                let _ = k;
                time += 0.05;
                run(events, time, &mut open);
            }
            if let Some(f) = open {
                hits.push((x, f));
            }
        }
        assert!(!hits.is_empty(), "no link found");
        assert!(hits.iter().all(|(_, f)| f == "rsi_bands.pine"));
        // Only the name is a link, not the words around it.
        assert!(hits.len() < 40, "{} hits", hits.len());
        assert!(hits[0].0 > 30, "starts at {}", hits[0].0);
    }

    /// Live, against the real Claude Code (uses the account; run by hand):
    /// builds a strategy end to end on demo data.
    /// `cargo test -p opentern --bin opentern live_strategy -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_strategy() {
        let mut d = crate::drive::Driver::new("claude-live");
        let cli = Agent::Claude.find().expect("claude installed");
        // SAFETY: only this test reads OPENTERN_CLAUDE here.
        unsafe {
            std::env::set_var("OPENTERN_CLAUDE", &cli);
        }
        d.app.claude_load();
        d.app.claude.mode = Mode::Auto;
        d.app.claude.model = "claude-haiku-5-5".into();
        d.app.claude.input =
            "Build an RSI mean reversion strategy in Pine, put it on the chart and show me its results in a table."
                .into();
        d.app.claude_send();
        let start = std::time::Instant::now();
        loop {
            d.frame(vec![]);
            let chat = d.app.claude.chat().unwrap().clone();
            // Anything that still asks gets allowed.
            if let Some(i) = chat.items.iter().position(is_waiting) {
                d.app.claude_answer(chat.id, i, true, false);
            }
            if !chat.busy || start.elapsed().as_secs() > 300 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let chat = d.app.claude.chat().unwrap().clone();
        for it in &chat.items {
            match it {
                Item::Tool {
                    label, error, result, ..
                } => eprintln!(
                    "STEP {label}{}",
                    if *error {
                        format!("  ERROR {}", result.clone().unwrap_or_default())
                    } else {
                        String::new()
                    }
                ),
                Item::Assistant(t) => eprintln!("REPLY\n{t}"),
                Item::Note { text, .. } => eprintln!("NOTE {text}"),
                _ => {}
            }
        }
        eprintln!("MODEL {} USAGE {:?}", chat.model_name, chat.usage);
        assert!(!chat.busy, "timed out");
        assert!(
            chat.items
                .iter()
                .any(|i| matches!(i, Item::Tool { label, .. } if label == "Read the Strategy Report"))
        );
        assert!(d.app.strategies.current().is_some(), "no strategy on the chart");
    }

    /// The agent teaching with the app's hands: a real (demo) example
    /// found in the bars, marked with drawings, then walked through in a
    /// bar replay. Uses the account (Haiku):
    /// `cargo test -p opentern --bin opentern live_teach -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_teach() {
        let mut d = crate::drive::Driver::new("claude-teach");
        let cli = Agent::Claude.find().expect("claude installed");
        // SAFETY: only this test reads OPENTERN_CLAUDE here.
        unsafe {
            std::env::set_var("OPENTERN_CLAUDE", &cli);
        }
        d.app.claude_load();
        d.app.claude.mode = Mode::Auto;
        d.app.claude.model = "claude-haiku-5-5".into();
        d.app.claude.input = "Teach me the bullish engulfing pattern: find a real example on this chart, mark it \
             with a labelled rectangle and an arrow, then start a bar replay just before it and step through it."
            .into();
        d.app.claude_send();
        let start = std::time::Instant::now();
        loop {
            d.frame(vec![]);
            let chat = d.app.claude.chat().unwrap().clone();
            if let Some(i) = chat.items.iter().position(is_waiting) {
                d.app.claude_answer(chat.id, i, true, false);
            }
            if !chat.busy || start.elapsed().as_secs() > 300 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let chat = d.app.claude.chat().unwrap().clone();
        for it in &chat.items {
            match it {
                Item::Tool {
                    label, error, result, ..
                } => eprintln!(
                    "STEP {label}{}",
                    if *error {
                        format!("  ERROR {}", result.clone().unwrap_or_default())
                    } else {
                        String::new()
                    }
                ),
                Item::Assistant(t) => eprintln!("REPLY\n{t}"),
                Item::Note { text, .. } => eprintln!("NOTE {text}"),
                _ => {}
            }
        }
        assert!(!chat.busy, "timed out");
        assert!(!d.app.chart.drawings.is_empty(), "nothing drawn");
        assert!(
            chat.items
                .iter()
                .any(|i| matches!(i, Item::Tool { label, .. } if label.starts_with("Started a replay")))
        );
    }

    /// The first open shows the agent picker; picking one moves into its
    /// panel, and each agent keeps its own open chat.
    #[test]
    fn picking_and_switching_agents() {
        let mut d = crate::drive::Driver::new("claude-agents");
        d.app.layout.widget = Some(crate::shell::Widget::Claude);
        d.app.claude.cli = Some(Some(PathBuf::from("claude")));
        // Signed in (another test may point OPENTERN_CLAUDE at its fake CLI).
        d.app.claude.auth.insert(Agent::Claude, auth::Auth::In);
        d.app.claude.codex_cli = Some(Some(PathBuf::from("codex")));
        d.app.claude.auth.insert(Agent::Codex, auth::Auth::In);
        for _ in 0..40 {
            d.frame(vec![]);
        }
        assert_eq!(d.app.claude.agent, None);
        assert_eq!(d.app.claude.cards.len(), 2, "picker cards");
        let (a, r, _) = d.app.claude.cards[1];
        assert_eq!(a, Agent::Codex);
        d.click(r.center());
        assert_eq!(d.app.claude.agent, Some(Agent::Codex));
        for _ in 0..60 {
            d.frame(vec![]);
        }
        assert!(d.app.claude.anim.is_none(), "the move finishes");
        // A Codex chat, then Claude's panel, then back to the Codex chat.
        let codex_chat = d.app.new_chat();
        d.app
            .claude
            .chat_mut(codex_chat)
            .unwrap()
            .items
            .push(Item::User("hi".into()));
        assert_eq!(d.app.claude.chat().unwrap().agent, Agent::Codex);
        d.app.claude_switch(Agent::Claude, None, 0.0);
        assert_eq!(d.app.claude.active, None);
        d.app.claude_switch(Agent::Codex, None, 0.0);
        assert_eq!(d.app.claude.active, Some(codex_chat));
        // Saved and loaded with its agent.
        d.app.claude_save();
        let text = std::fs::read_to_string(d.dir.join("claude").join("chats.json")).unwrap();
        let saved: Saved = serde_json::from_str(&text).unwrap();
        assert_eq!(saved.agent, Some(Agent::Codex));
        assert!(
            saved
                .chats
                .iter()
                .any(|c| c.id == codex_chat && c.agent == Agent::Codex)
        );
    }

    /// Alt+V pastes and doesn't also type a "v".
    #[test]
    fn alt_v_types_nothing() {
        let mut d = crate::drive::Driver::new("claude-altv");
        d.app.layout.widget = Some(crate::shell::Widget::Claude);
        d.app.claude_load();
        d.app.claude.agent = Some(Agent::Claude);
        d.app.claude.cli = Some(Some(PathBuf::from("claude")));
        // Signed in (another test may point OPENTERN_CLAUDE at its fake CLI).
        d.app.claude.auth.insert(Agent::Claude, auth::Auth::In);
        for _ in 0..20 {
            d.frame(vec![]);
        }
        d.ctx.memory_mut(|m| m.request_focus(egui::Id::new("claude-input")));
        d.frame(vec![]);
        // The box has focus: plain typing lands in it.
        d.frame(vec![egui::Event::Text("x".into())]);
        assert_eq!(d.app.claude.input, "x");
        let alt = egui::Modifiers::ALT;
        d.frame(vec![
            egui::Event::ModifiersChanged(alt),
            egui::Event::Key {
                key: egui::Key::V,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: alt,
            },
            egui::Event::Text("v".into()),
        ]);
        d.frame(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        assert_eq!(d.app.claude.input, "x");
    }

    /// Live, against the real Codex CLI (uses the account; run by hand):
    /// a short turn that uses the app's tools and writes a script.
    /// `cargo test -p opentern --bin opentern live_codex -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_codex() {
        let mut d = crate::drive::Driver::new("codex-live");
        let cli = Agent::Codex.find().expect("codex installed");
        // SAFETY: only this test reads OPENTERN_CODEX here.
        unsafe {
            std::env::set_var("OPENTERN_CODEX", &cli);
        }
        d.app.claude_load();
        d.app.claude.agent = Some(Agent::Codex);
        d.app.claude.mode = Mode::Manual;
        d.app.claude.codex_model = std::env::var("CODEX_MODEL").unwrap_or_default();
        d.app.claude.input = "Call get_chart, then create hello.pine with a one-line Pine v6 indicator that \
            plots close, check it with check_script, and reply with one short sentence."
            .into();
        d.app.claude_send();
        let start = std::time::Instant::now();
        loop {
            d.frame(vec![]);
            let chat = d.app.claude.chat().unwrap().clone();
            if let Some(i) = chat.items.iter().position(is_waiting) {
                eprintln!("ASK {:?}", chat.items[i]);
                d.app.claude_answer(chat.id, i, true, false);
            }
            if !chat.busy || start.elapsed().as_secs() > 240 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let chat = d.app.claude.chat().unwrap().clone();
        for it in &chat.items {
            eprintln!("{it:?}");
        }
        eprintln!(
            "MODEL {} USAGE {:?} CONTEXT {}",
            chat.model_name, chat.usage, chat.context
        );
        eprintln!(
            "LIMITS {:?} MODELS {:?}",
            d.app.claude.codex_limits, d.app.claude.codex_models
        );
        assert!(!chat.busy, "timed out");
        assert!(chat.session.is_some());
        assert!(
            chat.items
                .iter()
                .any(|i| matches!(i, Item::Tool { label, .. } if label == "Looked at the chart"))
        );
        assert!(d.app.scripts_dir().join("hello.pine").is_file());
    }

    /// At the narrowest pane, the composer's controls still don't overlap,
    /// and a wide table is squeezed into the pane.
    #[test]
    fn narrow_pane_fits() {
        let mut d = crate::drive::Driver::new("claude-narrow");
        d.app.layout.widget = Some(crate::shell::Widget::Claude);
        d.app.claude_load();
        d.app.claude.agent = Some(Agent::Claude);
        d.app.claude.model = "claude-sonnet-5-5".into();
        // The composer only shows once Claude Code is found; CI has none.
        d.app.claude.cli = Some(Some(PathBuf::from("claude")));
        // Signed in (another test may point OPENTERN_CLAUDE at its fake CLI).
        d.app.claude.auth.insert(Agent::Claude, auth::Auth::In);
        for w in [220.0, 260.0, 300.0, 420.0] {
            d.app.layout.widget_w = w;
            for _ in 0..30 {
                d.frame(vec![]);
            }
            let f = &d.app.claude.footer;
            assert!(f.len() >= 4, "{f:?}");
            for i in 0..f.len() {
                for j in i + 1..f.len() {
                    assert!(!f[i].intersects(f[j]), "pane {w}: {:?} overlaps {:?}", f[i], f[j]);
                }
            }
        }
        // A table with long cells, in a narrow ui: it stays inside.
        // The app's context: it has the app's fonts.
        let ctx = d.ctx.clone();
        let t = crate::theme::Theme::by_name("dark");
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 600.0))),
            ..Default::default()
        };
        let mut used = egui::Rect::NOTHING;
        let mut out = ctx.run_ui(raw, |ui| {
            md_table(
                ui,
                &t,
                &[
                    "| Metric | Value | Note |",
                    "|---|---|---|",
                    "| Net profit | -2,094.26 (-20.9%) | after commission of 0.1% per trade |",
                    "| **Win rate** | `42.9%` | 12 wins and 16 losses |",
                ],
            );
            used = ui.min_rect();
        });
        out.textures_delta.clear();
        assert!(used.width() <= 240.0 + 0.5, "{used:?}");
    }
}

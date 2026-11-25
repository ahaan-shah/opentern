# Notes: the agent panel (Claude Code and Codex)

Moved out of `CLAUDE.md` (Nov 2026) to keep the briefing short. Keep it current: update it in the same commit as the code.

## Agent panel: Claude and Codex (Oct 2026)
- **Two agents** (`claude/agent.rs`, `Agent::{Claude, Codex}`): the rail button is
  the agents' face (sand disc, two dark pill eyes, drawn in `mark::face`; the
  maintainer's design). First open shows a card per agent (`claude_picker`);
  picking one moves into its panel (the card shrinks into the header and the
  logo flies there, `claude_pick_overlay`, `ANIM` 0.6 s). The header's top-left
  is the agent switcher (dropdown, either agent anytime) + `/ chat title`.
  Each chat has an `agent`; each agent keeps its open chat (`last`); opening
  another agent's chat from the list switches. Saved `agent: None` = picker.
  No suggested prompts: the empty chat shows the logo on a halo, and the
  composer's hint is "What can I help you with?". Per agent: accent (Claude terracotta, Codex
  monochrome), models, usage windows, inline-code colour.
- **Codex** (`claude/codex.rs`): one `codex app-server` per chat (JSON-RPC
  lines; checked live against codex-cli 0.163), cwd = the scripts folder,
  `thread/start` with `developerInstructions` and the app's tools as
  `dynamicTools` (`item/tool/call` → answered through `claude_mcp` like
  Claude's MCP calls), `thread/resume` for older chats, `turn/start` per
  message (Ask first = `untrusted`, Auto = `on-request`, sandbox
  `workspace-write`), approvals → the same approval cards (`acceptForSession`
  = Allow for this chat). Models come from `model/list` (account-specific;
  fetched by a short-lived app-server once a run, when the model menu opens, as
  Claude's are; the saved list shows meanwhile, and picks that are gone go back to
  the default: `set_codex_models`).
  `process::Client` is an enum over both CLIs; `Event::Models` is Codex only.
  **Terms (checked 22 Oct 2026, learn.chatgpt.com/docs/app-server):** local or
  open-source apps may keep using app-server; it was never permitted for
  commercial or hosted services. `clientInfo.name` ("opentern") identifies us
  in OpenAI's compliance logs. Opentern must stay free and local for this.
  `live_codex` (ignored, uses the account): `CODEX_MODEL=<id> cargo test -p
  opentern --bin opentern live_codex -- --ignored --nocapture`.
- **Selectable text** (Nov 2026): labels aren't selectable app-wide
  (`theme.rs`), so replies turn `selectable_labels` on in a scope and the
  user bubble is a selectable `Label` on the painted bubble; reply lines
  with script links are Labels too, traced to a word with the same cached
  galley.
- **Queued messages** (Nov 2026; the send button queues too while there's
  text in the box, and stops only when it's empty): Enter while the agent works puts the message
  in `Chat::queued` (dimmed bubbles under the spinner, × removes one); when the
  turn's `Done` arrives they all go as one message (`claude_send_queued`), as in
  Claude Code. Stop, or the CLI exiting, puts them back in the input box
  (`claude_unqueue`). Queued in the app, not written to the CLI mid-turn, so
  Claude and Codex behave the same. Not saved.
- **Enter / Shift+Enter:** `consume_key(NONE, Enter)` ignores Shift (egui's
  `matches_logically`), so the composer removes only exact plain-Enter events;
  Shift+Enter reaches the TextEdit as a new line.
- **Emoji** are pasted (no picker; the maintainer didn't want a button). egui
  draws them with its bundled monochrome Noto Emoji: no colour, and newer ones
  it lacks (🤣 🙂 🤔 🥳 🟢) show as boxes.
- Never read `ctx.data` (e.g. `code_color`) inside `ui.fonts_mut(|…|)`: the
  context is locked there and the frame deadlocks (the stall watchdog fires).
- Stills: `agent:claude|codex` picks an agent (skips the picker).

## Claude panel (Oct 2026)
- Rail button under Notifications (`Widget::Claude`). Chats with **Claude Code**, the
  CLI the user installed and signed in to (their subscription; no API key in the app).
  Found via `OPENTERN_CLAUDE`, the PATH, `~/.local/bin`, `~/.claude/local`. No CLI →
  an empty state: "Install Claude Code" links to its install page
  (`Agent::install_url`; never print other products' install commands).
  **Windows:** a `.exe` anywhere beats an npm `.cmd` shim; Rust won't pass our
  JSON/multi-line arguments through cmd.exe, so for a shim we run the binary the
  npm package carries (`agent::npm_binary`: Claude's `bin/claude.exe` after its
  postinstall, Codex's `@openai/codex-win32-<arch>/vendor/<triple>/bin/codex.exe`;
  checked against claude-code 2.1.308 and codex 0.163 package layouts, never run
  on a real PC).
- **Protocol** (checked against CLI 2.1.300; the same one Claude Code's SDKs use):
  `claude -p --input-format stream-json --output-format stream-json --verbose
  --include-partial-messages --permission-prompt-tool stdio --permission-mode
  manual|auto --mcp-config '{"mcpServers":{"opentern":{"type":"sdk",…}}}'
  --allowedTools mcp__opentern --append-system-prompt … [--model] [--resume <id>]`,
  cwd = the scripts folder. In: user messages (a hidden `<opentern_context>` text
  block first: chart, scripts and errors, editor file), `control_response`s, and
  `control_request`s `interrupt` (Stop) and `set_permission_mode` (the toggle, live).
  Out: `stream_event` text deltas, `assistant` tool uses + per-request usage,
  `result` (tokens, cost), `rate_limit_event` (the account's 5-hour/weekly
  utilization and reset times: the usage card), `control_request` `can_use_tool`
  (approval cards: Allow / Allow for this chat = `updatedPermissions` from the
  request's suggestions / Deny) and `mcp_message` (the app's tools, answered on the
  UI thread in `tools.rs`; symbol searches answer when the hub replies, seqs from
  `tools::SEARCH_BASE`). Subagent traffic (`parent_tool_use_id`) isn't shown.
- **Tools** (`mcp__opentern__*`): get_chart, get_bars (latest, or a date
  range), search_symbols, open_symbol, set_timeframe, list_scripts, check_script
  (compiles like the chart does), add_script_to_chart, remove_from_chart,
  list_indicators, add_indicator, strategy_report (the Report as JSON + latest
  trades), open_in_editor, scripting_guide (`docs/scripting.md`, embedded).
  Claude edits files with its own tools; scripts on the chart hot-reload.
- **The agent's hands** (0.2.0, the maintainer: "everything I can do with
  clicks"; `claude/control.rs`): one tool per area with an `action` (keeps the
  list, sent with every request, short): chart_view (incl. view, pan,
  zoom, price_range; each answers with `view_note`: dates, bars, price
  range in view), indicator, drawings,
  replay, snapshot, watchlist, screener, strategy_settings, strategy_lab,
  paper_trading, alerts, open_panel; seasonality and custom_index
  (`claude/markets.rs`: list/get/compare/create/edit/rebalance/delete/open;
  member lookups are a `Batch` with an `IndexJob`, saved via
  `App::upsert_index`, answered with the board once settled). Slow work answers later
  (`control::Wait`, polled in `poll_claude`): bars back to a date (pages of
  `Request::More`, 45 s cap; also open_symbol/set_timeframe wait for bars),
  screener and lab runs, the snapshot (`Snap::Agent`, PNG at 1 px per point,
  base64; Codex gets it as an `inputImage` data URL, checked against
  codex-cli 0.164's schema). Lab and screener adapters live in their own
  modules (`lab_configure`, `lab_results`, `screener_results`). Watchlist adds
  look each symbol up (`tools::Then::Watch`, a `Batch`); the hub's search
  worker answers every lookup numbered ≥ `hub::SEARCH_ALL` and coalesces
  only the search box's. Tests: `control::drive_tests` (every tool, demo
  data, offline); `live_teach` (ignored, Haiku) teaches a pattern end to end.
- One process per chat, started on the first send (`--resume` for older chats); a
  background chat's process is dropped once its turn ends. Processes die with the
  app: `on_exit` clears them, and on Linux `PR_SET_PDEATHSIG` covers crashes and
  kills (without it a process waiting on a prompt lingered ~10 s). Env vars
  a Claude Code session passes to its children (`process::SESSION_VARS`) are removed so a
  launch from inside Claude Code works; other `CLAUDE_CODE_*` are the user's, sign-in
  ones included, and must reach the CLI (Anthropic's rule).
- Chats (items, Claude Code session id, mode, model, tokens, cost) saved in
  `<workspace>/claude/chats.json`; every entry point loads it first so a send can't
  overwrite unread chats. Headless runs never start Claude Code unless
  `OPENTERN_CLAUDE` is set.
- Tests: `claude::tests::a_turn_with_a_fake_cli` drives the real app against
  `claude/fake_claude.py` (a stand-in CLI: tools, a permission prompt, open_symbol,
  usage), offline, Unix only. Stills: `OPENTERN_STILL=claude-model:haiku,claude:<message>`
  sends a message at launch (no commas in it); `widget:claude` just opens the panel.
  Screenshots: only `grim` the window after checking `hyprctl activewindow` is ours
  (an unchecked grab once caught the maintainer's terminal).
- **Look** (Claude app / opencode style): the Claude mark (`claude/mark.rs`, Simple
  Icons' CC0 glyph in `assets/claude.svg`, tinted; brand colour #D97757, used for the
  mark, the send button and the approval card's border only) on the rail, in the
  header, at the start of each reply and as the spinner. Header: mark, chat title,
  Chats (swaps the pane to the chats list: search, Today / Previous 7 days / Older,
  delete on hover) and New chat (reuses an empty open chat). Composer at the bottom:
  text, then mode pill (Ask first / Auto), model pill, usage ring (5-hour window;
  click for the plan and this chat's tokens) and the round send/stop button. Tool
  rows have per-tool icons on a thin rule (click for the result); approval cards have
  numbered choices (keys 1-3; the type-a-number interval shortcut stands down while
  one waits), edits as red/green diffs; code blocks show the language and a copy
  button. Script tools accept `file`, `path` or `file_path` (models vary).
  Second pass: tool calls and answered prompts group into one rounded "N steps"
  box (folded once a turn of 3+ finishes; open while live); user bubbles are drawn
  by hand (left-aligned text); replies use 1.55 line height; inline code is a soft
  terracotta (no background: egui paints it at full line height); script names in
  replies are links to the Script Editor (`inline` lays out its own galley only
  when a line has one, so other text stays selectable); menus open upward and fit
  the pane; scrollbars are floating and hidden until used. Models are exact ids
  shown by name. **The list comes from Claude Code** (`process::models`: a
  short-lived `claude -p` answers a `control_request` `initialize` with its
  `models`; the alias entries (opus, sonnet, fable, haiku) give each family's
  latest as `resolvedModel` + `displayName`; no model request is made). Asked
  once a run when the model menu opens, saved as `claude_models`; `MODELS` is
  the fallback until then (keep it current). Old aliases like "haiku" are
  mapped by `model_id` on load. Third pass: chats rename
  (pencil on hover in the list, or double-click the header title; Enter or click
  away keeps it, Esc cancels); composer menus and the usage card anchor to the
  composer's edge (`above`, from last frame's rect) so they never cover it, the
  model menu and usage card grow leftwards to stay in the pane; code blocks float
  their copy button (no header row unless there's a language). Still:
  `claude-menu:<mode|model|usage>` opens one of them.
- **Same Claude as the terminal:** "Default" passes no `--model` (Claude Code's own
  default, the user's settings), no effort flag, and `--append-system-prompt` only
  adds facts (scripts folder, tools, strategies → strategy_report, paper only), never
  reply-style rules (an early "keep answers short" made replies terse; removed).
  Claude Code snapshots a chat's system prompt on its first request, so prompt
  changes reach new chats only. Replies render Markdown tables (`md_table`: sized
  to content, squeezed and wrapped in a narrow pane), quotes and rules. The
  composer footer degrades in a narrow pane (mode → icon, model → first word →
  icon); `narrow_pane_fits` checks nothing overlaps at 220-420 px.
  **Mode defaults to Auto** (the maintainer's terminal is set up that way); a mode is
  kept only once picked by hand (`mode_picked`; older saves stored the old default).
- **Attachments** (`claude/attach.rs`): the paperclip (opens the file picker: rfd's async
  dialog on a thread with pollster, as in Magpie; no menu), drag and drop onto
  the window while the panel is open (overlay), Ctrl+V of copied files (egui gives
  file-manager copies as text: `file://` URIs or paths, intercepted before the text
  box), and **Ctrl+V for a clipboard image** (Alt+V until Nov 2026; Alt V is
  also the vertical line): egui-winit eats Ctrl+V's press and only forwards
  text, so an image-only clipboard sends nothing, but the V's *release* still
  arrives with no press before it; such a release with no `Paste` in the last
  second means "look for an image" (`Claude::v_down`, `pasted_at`); `arboard`
  (Wayland data-control on Linux) reads it, saved as PNG under
  `<workspace>/claude/attachments/`. Sent as content blocks before the text: images
  (≤ 4.5 MB) as image blocks (checked against the real CLI), UTF-8 files ≤ 256 KB
  inline in `<attached_file path=…>`, anything else as a path. `Item::Attached`
  shows chips above the message (click opens: scripts in the editor, else the
  system app). Still: `claude-attach:<path>`. The narrowest pane hides the usage
  ring (its numbers go in the model button's tooltip).
  `live_strategy` (ignored; uses the account, Haiku) builds a strategy end to end:
  `cargo test -p opentern --bin opentern live_strategy -- --ignored --nocapture`.
  Stills: `claude-chats`, `claude-new`.
- Also fixed with it: widgets render in a clipped child of the panel's exact size
  (long Object tree names used to widen the panel under the chart); Object tree and
  Data window rows lay out buttons/values first and truncate names.

- **Agent sign-in gate** (`claude/auth.rs`): `claude auth status --json`
  (`loggedIn`), `codex login status` ("Not logged in"); only a clear
  sign-out blocks. **Signing in is never done in the app** (the maintainer, 26
  Nov 2026; Anthropic doesn't let apps offer Claude's sign-in, and T3 Code does
  the same): the gate shows the CLI's own login command to run in a terminal,
  with a copy button at the end of its box (turns into a check:
  `widgets::copy_button`, shared with the device code) and Check again
  (`auth::login_command`: the picked account's
  variable in front; `env -u` / `$env:X=$null` when the default is picked but the
  app was started with the variable set; the CLI's full path when the bare name
  wouldn't run it; PowerShell's form on Windows, never run on a real PC). Don't
  add a login button, a code box or anything that sees a token. Rechecked when a
  chat's process exits. Tests that open the panel insert `Auth::In` (another test
  sets `OPENTERN_CLAUDE` process-wide).
- **Following Anthropic's and OpenAI's rules** (checked 26 Nov 2026 against
  code.claude.com/docs/en/legal-and-compliance, learn.chatgpt.com/docs/app-server
  and T3 Code): run the user's own unmodified CLI on their own plan, never read
  login tokens, never switch accounts automatically, never set
  `CLAUDE_CODE_ENTRYPOINT`. Claude Code gets
  `CLAUDE_AGENT_SDK_CLIENT_APP=opentern/<version>` (`process::command`, chats and
  the model list), sent as `x-client-app` (`live_client_app`, ignored, checks it
  with the real CLI, a made-up home and a local stand-in server). Codex has
  `clientInfo`. No Claude SDK, Sign in with ChatGPT or Antigravity (the
  maintainer's call). The logos stay (the maintainer's call), credited in the README, the website and
  THIRD-PARTY-LICENSES (not in Settings: the maintainer's call).
- **Model shown, never "Default"**: `config_model` reads `ANTHROPIC_MODEL` /
  `~/.claude/settings.json` (aliases via `model_id`) and Codex's
  `config.toml`; else the model a chat last reported (`defaults`, saved);
  else Codex's server default (`model/list` sorted default first).

- **Agent accounts** (`claude/accounts.rs`): an account is a CLI config
  folder: Claude Code's `CLAUDE_CONFIG_DIR`, Codex's `CODEX_HOME`. Found:
  the default, the launch env's, and `~/.claude-*` / `~/.codex-*` beside
  the default. The default means *removing* the variable (Claude Code with
  `CLAUDE_CONFIG_DIR=~/.claude` looks for `~/.claude/.claude.json` and
  fails). Picked in the header's agent menu (shown when there's more than
  one), saved as `agent_account:<agent>`; each chat keeps the account it
  started on (`Chat::account`; older chats = the launch account), so
  `--resume` keeps working. Every CLI start (chat, auth, Codex
  model list) goes through `accounts::apply`.
- Agent composer: its height is last frame's measured height (wrapped
  lines count); the text box scrolls past 8 lines. Strategy Lab list
  headers elide to their column. Windows `open_url` uses
  `rundll32 url.dll,FileProtocolHandler` (`explorer <url>` opened
  Documents for long news links).

# Opentern — briefing for Claude sessions

This file is the shared memory of every session that works on the repo: the decisions,
the rules and where to find the rest. Keep it a briefing, not a diary: feature details go
in the notes below, and this file stays short (it's loaded into every session).
If you learn something the next session needs, or a decision changes, update this file
or the right note in the same commit. `AGENTS.md` is a symlink to this file. Sibling
project and design reference: `~/projects/magpie` (read its `CLAUDE.md`; a lot is reused
from it, see `docs/design.md` §3).

**Status (Nov 2026, v0.3.6):** Phases 0–13 built and committed (one commit per phase, as the
maintainer asked), plus most loose ends. Phase 14 (ship) has started: the repo is
public on GitHub (since v0.1.2, 21 Oct 2026); releases build for Linux, macOS and Windows.
Read "Resume here" below first.

## Where the rest lives

Read the note for the area you're working on before changing it. A "§" number anywhere
refers to a section of `docs/design.md`.

| File | What's in it |
|---|---|
| `docs/design.md` | The plan: product bar, chart engine (§4), layout (§4a), customization (§4b), data (§5), scripting (§6), built-ins (§6a), backtester (§7), alerts (§8), roadmap (§9), deps, testing, licensing, sources, measured timings |
| `docs/notes/charts-ui.md` | Drawings, layouts, watchlists, chart types, multi-chart, replay, screener, widgets, UI passes 4–6, onboarding, themes, shortcuts, What's new, Windows notes |
| `docs/notes/scripts.md` | Python host, Pine engine (v1, v2, libraries), built-ins wave 2, Luau/JS/TS, feeds (`request.security` on other symbols) |
| `docs/notes/strategies.md` | Broker emulator, Strategy Report and TradingView parity fixes, paper trading, DOM, Strategy Lab |
| `docs/notes/alerts.md` | Alert engine and delivery, `--daemon`, `watcher.rs`, earnings, EXT, the shelved journal |
| `docs/notes/data.md` | Data flow, providers and their gotchas, calendars, corporate actions, commodities, indices and rates, perpetuals, delisted pairs, logos, special sessions, pre-market, robustness, the Yahoo price feed, splits |
| `docs/notes/markets.md` | Market overview, custom indices, Seasonality |
| `docs/notes/agents.md` | The agent panel: Claude Code and Codex protocols, tools, look, attachments, sign-in, accounts |
| `docs/notes/transfer-security.md` | Export and import, device handoff, untrusted-input rules, fuzzing, cargo-deny |
| `docs/notes/i18n.md` | Languages: how text is translated, catalogs, fonts, adding a text or a language |
| `docs/notes/release.md` | CI and packaging, website and screenshots, the release checklist |
| `docs/indicators.md`, `docs/ta-semantics.md`, `docs/metrics.md`, `docs/scripting.md`, `docs/ui-backlog.md`, `docs/ideas.md` | Built-in status, `ta.*` semantics, report formulas, the script API, UI backlog, future ideas |
| `CHANGELOG.md` | Release history |

## Decisions so far (from the maintainer, Oct 2026)

- Name **Opentern** (open + terminal; a tern is a seabird, a sibling to Magpie). The binary
  is `opentern`.
- **Rust + egui on glow**, like Magpie. The UX, design and self-updater all come from Magpie.
- Markets: crypto, US stocks, Indian stocks (NSE/BSE), forex/futures. Crypto goes first.
- Scope: analysis, scripting, backtests, alerts and **paper** trading. Live order routing
  comes later.
- **Strategy metrics and chart overlays must match TradingView** (names, formulas, look).
- Pine: "whatever's best, not PineTS if there's a better route". Chosen: a **native Rust
  Pine engine** (§6).
- License: MIT. It's for personal investing and trading analysis first, so it must be
  fast and accurate.
- **Anyone can download it and use it as their own:** nothing personal is baked in
  (see Privacy & portability).
- **Layout follows TradingView's arrangement** (§4a), because it works and traders
  already know it.
- **TradingView-level personal customization:** candle styles and colours, chart and
  indicator templates, defaults, favorites (§4b).
- **WASM plugins are skipped for now** (the maintainer, 10 Oct 2026). Phase 13 is
  Luau, then JS/TS. Don't add wasmtime unless they ask again.
- **The agents and Python keep full access** (the maintainer, 21 Nov 2026: "the
  agent is there to help people"; the editor is like VS Code). No Contained
  mode for the agents and no OS sandbox for Python; the README's License
  section carries the disclaimer, and the app shows a one-time notice
  (`Modal::PythonNotice`; still `python-notice`) the moment a `.py` is
  anywhere in the scripts folder: `pywatch.rs` watches it recursively with
  the `notify` crate, and the flag file `python-notice-shown` in
  `opentern_core::config_dir()` keeps it from showing again on that
  computer, in any workspace. Don't add
  either unless asked.
- **The interface is translated** (the maintainer, 25 Nov 2026): wrap every new text
  (`tr`, `trf!`; `nt!`/`msg!` when stored) and update the catalogs. English is the key;
  user's names, scripts, tickers and everything the agent reads stay as they are.
  Read `docs/notes/i18n.md` first.
- **Every standard TradingView built-in ships by default**, plus licence-checked
  open-source community scripts. They're ported from Pine to Rust with exact numerical
  parity (§6a).

## How the maintainer works (inherited from Magpie)

- **Nothing outward-facing without an explicit go.** That covers releases, making the repo
  public, publishing packages and force-pushes. Approval for one doesn't carry to the next.
- **They test before you release.** Build it, run it locally, let them try it and iterate.
- **Design taste:** clean, calm, modern, subtle. Accent colour is used sparingly. Error text
  is plain and human, not red stack traces.
- **Less is more: no gratuitous text** (the maintainer, Oct 2026). No captions explaining a
  control, no eyebrow labels over a single list ("APPLY"), no bylines, no reassurance
  lines, no sub-lines under empty states (`widgets::empty_state` takes only an icon and a
  title), no "…" on menu items. Keep real information: errors, statuses, where to get a
  key. Explanations go in tooltips if anywhere.
- **Wording:** short, plain, friendly. When they specify exact strings, use them verbatim.
- **They send follow-ups mid-task.** Fold them into the current work and say how each
  was handled.
- **Report honestly**, including your own mistakes. Say what you actually verified and
  what you didn't.
- **Never touch real data** (their workspace, API keys, paper accounts). Test with demo
  or temp data dirs.
- Never kill a running instance you didn't start. Machine-specific notes (how to launch
  apps on the maintainer's desktop, local paths) live in `CLAUDE.local.md`, which is
  git-ignored. Read it if it exists, and never move its contents into tracked files.


## Privacy & portability (hard rule)

Opentern must be something anyone can download (say, the maintainer's cousin) and use as
their own from the first launch. Nothing about the maintainer may be baked in.

- **No personal data in tracked files.** That covers code, defaults, assets, tests,
  fixtures, screenshots, docs and the website: no real API keys, account IDs, watchlists,
  trades, statements, private scripts, `/home/<user>/…` paths or machine names. The
  author line in `Cargo.toml`, the copyright notice and the GitHub repo URL are the only
  identity, and they're public anyway. The maintainer's email in commit metadata is fine
  (they said so).
- **Generic first run.** It opens with a sensible starter layout and a starter watchlist
  of liquid public symbols across markets (e.g. BTCUSDT, ETHUSDT, SPY, QQQ, AAPL, NIFTY 50,
  EURUSD). Onboarding asks which markets they follow and auto-detects the timezone. There's
  no account and no sign-in for Opentern itself. API keys are only needed for data
  providers the user chooses to add, and each provider explains in plain words where to
  get its key.
- **All user state** lives in the OS data dir (`directories` crate:
  `~/.local/share/opentern`, `%APPDATA%\opentern`, `~/Library/Application
  Support/opentern`). API keys are stored only in the OS keychain (`keyring`). There are
  no hardcoded paths. Settings → Export and import moves the workspace to another
  machine, as one file or directly over the local network.
- **No telemetry.** The only outbound traffic is the update check (GitHub releases, can be
  turned off), the data providers the user has turned on, symbol logos, and, only while
  the user is sending or receiving in Settings → Export and import, the local network
  (a UDP broadcast and one encrypted TCP connection; never the internet). About 200
  logos ship in the app (`crates/app/assets/logos/`, made by `scripts/fetch-logos.py`:
  web3icons coins and theSVG brand marks, both MIT, plus public-domain/CC0
  Wikimedia Commons logos for NIFTY 500 companies); others are fetched once per
  symbol from jsDelivr or unpkg, then logoapi.sh (US, NSE and BSE tickers) into `<workspace>/logos/` (demo mode shows only the bundled
  ones and fetches nothing), off in
  Settings → Symbol logos. Indices and commodities get drawn badges. The market
  overview, only while it's open, fetches the S&P 500 list (jsDelivr) and the NIFTY
  500 list (niftyindices.com) at most weekly, Yahoo's
  screener, spark and batch quotes, plus Binance's 24h and trading-day tickers
  and daily bars for coins. The Fed funds rate (`NYFED:EFFR`) comes from
  `markets.newyorkfed.org` (keyless; `data/src/nyfed.rs`), only while it's on a
  chart, in a watchlist or in an alert: the whole history once per run, then the
  last 10 days after 9:05 and 14:35 New York time. **Not FRED** (its terms forbid
  caching). The New York Fed's notice (`nyfed::NOTICE`) stays in English, in the
  legend tooltip and the README. **No CoinGecko** (removed Nov 2026, the
  maintainer's call: its keyless API is "for testing" and needs a visible
  credit; prices come from the exchanges anyway). The agent
  panel (`docs/notes/agents.md`) only talks through the user's own Claude Code or
  Codex CLI, and only after they send a message (or open Codex's model menu):
  nothing starts at launch.
- **Logs stay local** and are redacted: no API keys, tokens or account IDs, even in panic
  reports.
- **Private test material stays out of the repo.** The maintainer's own Pine scripts and
  TradingView exports used for conformance live in a folder outside it, pointed to by
  `OPENTERN_PRIVATE_CORPUS` and skipped when unset. Committed fixtures are synthetic data,
  or small slices of free public exchange data (e.g. Binance public klines).
- **Guardrail:** `scripts/check-leaks.sh` runs in CI and before every commit. It fails on
  absolute home paths, private-key blocks, common API-key shapes, and committed `*.db` or
  `.env` files. If it flags something, fix the leak; don't weaken the check.

## Architecture (Cargo workspace)
```
crates/core      opentern-core    Bar/Tick/Symbol/Timeframe models, SQLite store, settings, update.rs (from Magpie)
crates/data      opentern-data    provider traits + adapters, tokio network thread, bar builder/resampler, cache
crates/series    opentern-series  series runtime: bar-by-bar execution model, StudyOutput protocol, built-in indicators
crates/pine      opentern-pine    native Pine v5/v6 engine: lexer → parser → type check → bytecode VM on opentern-series
crates/script    opentern-script  language hosts: Python (subprocess), Luau (mlua), JS/TS (rquickjs); WASM skipped for now
crates/engine    opentern-engine  backtester, paper broker, optimizer, alert evaluator
crates/i18n-macros               `trf!`, the translated `format!`
crates/app       opentern         egui app: chart engine, panels, editor, strategy tester
python/opentern  pip SDK          the Python side of the script protocol (vectorized + on_bar APIs)
```
Key decisions:
- **Prices are `f64`, with `tick_size` on the Symbol for display and rounding.** This
  differs from Magpie, where money is i64. Timestamps are `i64` ms UTC, and time zones use
  `jiff`, as in Magpie.
- **Threads:** UI thread, one tokio runtime thread for all network I/O, and a rayon pool
  for scripts, backtests and optimizers. Data flows over crossbeam channels. The UI reads
  immutable snapshots (`arc-swap`) and frames never block or query the DB, as in Magpie's
  `Memo`-on-`version()` rule.
- **Repaint policy:** idle costs 0% CPU. While streaming, repaints run at ≤ 30 fps, driven
  by new ticks. Animations run at 144 fps (Magpie's `FRAME_CAP`) with vsync off.
- **Storage:** SQLite (rusqlite bundled, WAL, append-only `MIGRATIONS`) holds settings,
  watchlists, layouts, drawings, scripts, alerts, paper accounts and the bar cache
  (`bars(symbol_id, tf, ts) WITHOUT ROWID`). Ticks are kept in memory only, with an
  optional Parquet export. Only base timeframes are stored, and higher ones are resampled.
- **Secrets** (API keys) go in the OS keychain via the `keyring` crate, never in SQLite.
- **Crash shield:** each frame runs in `catch_unwind` with `panic = "unwind"`. Script
  hosts are isolated too: Python runs in a separate process, and Luau and JS run with
  instruction and memory limits. A bad script must never take the app down.

## Crates
| Crate | What it owns |
|---|---|
| `opentern-core` | `Bar`, `Timeframe`, `Symbol`, `starter_watchlist()`, SQLite workspace (`db.rs` migrations: v1 settings, v2 layouts/templates/indicator_defaults/favorites, v3 drawings/watchlists), `Store` (named layouts, templates, drawings per symbol, watchlists, `value`/`set_value` for app keys like `layout_id`), self-updater, `transfer.rs` (export files, import), `handoff.rs` (device to device) |
| `opentern-data` | `Hub`, `overview.rs` (market overview boards), providers `binance.rs`, `yahoo.rs`, `demo.rs`, **`expr.rs` (symbol maths: `BTCUSDT/ETHUSDT`, `(A + B)/2`; legs joined on time; quotes derived from legs)**, `cache.rs`, `resample.rs` |
| `opentern-series` | `ta.rs` (Pine-exact primitives; highest/lowest use a monotonic queue), `study.rs` (`Instance` with built-in, `with_factory` (Pine) and `external` (Python) modes; `Out`/`Output` with palette colours plus `raw` colours, `bg`, `bar`; `PlotStyle::Shape` + `ShapeSpec`; `InputKind::Text`), `builtins/` (62 indicators) |
| `opentern-script` | **Luau / JavaScript / TypeScript (Phase 13)**: `host.rs` (all semantics: declarations → `Decl`/`Meta`, `HostState` per-bar runtime with `ta.*` by call order, plots, `state` snapshots, strategy → `opentern_engine::Broker`), `inproc.rs` (`Lang`/`Vm` traits, `compile` = load + 60-bar dry run to find plots, `ScriptStudy`), `luau.rs` (mlua, Luau vendored), `js.rs` (rquickjs; TS via `swc_ts_fast_strip`, types blanked so lines match). Python host: SDK embedded from `python/opentern/` and unpacked to `<workspace>/python`; one worker process per script (`_runner.py`, JSON lines); reload on file change; errors and prints returned |
| `opentern-engine` | `quant/` (Phase 12: `optimize.rs` grid/random/Bayesian + heatmap, `gp.rs` Gaussian process + expected improvement, `walkforward.rs`, `montecarlo.rs`, `portfolio.rs`, `export.rs` CSV; all take a `run(values, bars) -> Broker` closure so any in-process language works; seeded and deterministic), `broker.rs` (TradingView-style emulator: next-open market fills, intrabar O→H→L→C / O→L→H→C path for stops/limits, gaps, reversals, pyramiding, brackets and tick exits, commission, slippage, qty types, process_orders_on_close, run-up/drawdown per trade, equity incl. intrabar) and `report.rs` (Strategy Report metrics) |
| `opentern-pine` | Pine engine: `lexer.rs`, `parser.rs` → `ast.rs`, `compile.rs` (names → frame slots, constant folding, declarations → `Meta`), `run.rs` (`PineStudy`, bar-by-bar interpreter), `natives/` (`mod.rs` core/math/str/color/time/syminfo/timeframe/barstate, `ta.rs` on `opentern_series::ta`), `natives/collections.rs` (array/map/matrix), `natives/draw.rs` (label/line/box/table/linefill/polyline/chart.point), `natives/security.rs` (`request.security`), `natives/strategy.rs`; `heap.rs` (objects with GC; drawing types live in `opentern_series::drawings`), `value.rs`; `tests/corpus/` + `tests/conformance.rs` |
| `opentern` (app) | `app.rs`, `shell.rs`, `chart/` (`draw.rs` + `drawing_ui.rs` = drawings and the quick measure), `drawbar.rs` (floating toolbar for the selected drawing), `watchlists.rs`, `layouts.rs`, `objects.rs` (Object tree + Data window), `scripting.rs` (scripts on the chart), `editor.rs` (Script Editor: explorer, tabs, code view), `sources.rs` (built-ins' Pine source from the corpus), `logos.rs` (symbol logos, rendered with resvg), `settings.rs` (the Settings page in the chart area), `keys.rs` (keyboard shortcuts: actions, defaults, the user's changes), `favbar.rs` (the favourite drawings toolbar), `overview.rs` (the market overview, also in the chart area), `claude/` (the agent panel: `agent.rs` Claude/Codex, `process.rs` Claude Code client, `codex.rs` Codex client, `tools.rs` the app's tools, `mod.rs` chats and UI), `pine_host.rs` (Pine scripts on the chart), `dialogs/`, `drive.rs` (headless test driver) |

## Resume here (next session)
Scripts and snapshots folders are settings (`scripts_dir`/`snapshots_dir`, cached in
`App::folders`); always go through `App::scripts_dir()`/`snapshots_dir()`. A Claude
chat resumes in the folder it started in (`Chat::folder`: Claude Code finds sessions
per folder) with `--add-dir` for a moved scripts folder; Codex resumes in the current
one; every message's context names the scripts folder. Add new
user-facing changes to a fresh `## Unreleased` section at the top of
`CHANGELOG.md` as they land; at release time rename it and follow
`docs/notes/release.md`.

**What's left, in order:**
1. **The maintainer's UI/UX feedback**, folded in iteratively. Use the Magpie-style
   widgets in `widgets.rs`, not egui's checkbox/ComboBox/selectable_value/Button.
   Past passes: `docs/ui-backlog.md`, `docs/notes/charts-ui.md`.
2. **The agent panel** (`docs/notes/agents.md`). Iterate on the maintainer's
   feedback before the next release.
3. **Phase 14:** still to do: notarizing the macOS app (`docs/notes/release.md`).

**Loose ends from earlier phases** (fit in where natural):
- Technical Ratings and Bollinger Bars (need TradingView's source or
  an export); the breadth/OI/intrabar-volume built-ins (`docs/indicators.md`).
- Keyed providers (Alpaca, OANDA, Upstox, IBKR) have never run against a real
  account. NSE 2026 holidays are in (from brokers' copies of NSE's list; check
  against the circular); add 2027's when announced.

**Testing the app for the maintainer:** run `scripts/dev-install.sh` (fast build,
installed as "Opentern Dev" in their app launcher, replacing the last one). A binary
named `opentern-dev` keeps its own workspace and never updates itself
(`opentern_core::dev_copy`). For your own checks, launch `target/fast/opentern` with a
throwaway `--data-dir` through `hyprctl` (see `CLAUDE.local.md`). Never use their real
workspace.

**Releasing:** only when asked, and only after the maintainer has tried the build
(open it for them on workspace 10 and wait for their go). Follow the checklist in
`docs/notes/release.md` exactly.

## Gotchas so far
- **Scrollbars show only while scrolling** (the maintainer, Nov 2026: a bar
  appearing because the pointer rests on a list covers its content).
  egui's floating style shows the bar whenever the pointer is over the
  area ("active" opacities); `theme::tick_scrollbars` (each frame) keeps
  those at 0 and raises them for ~1 s after a wheel/trackpad scroll.
  A panel that sets its own `spacing.scroll` must pass it through
  `theme::quieten(.., theme::scroll_shown(ctx))` (agent panel, overview).
- **Bars lay out their right-hand group first** (`right_to_left`, then a nested
  `left_to_right` with a clip rect). Check layouts at 1000×640.
- **Never `ui.separator()` or a `right_to_left` layout directly in a modal's body**: they
  take all the height offered (Chart settings and Save template blew up). Use rows or
  fixed sizes.
- **The chart takes no input while a dialog, menu or popup is open** (`interactive` flag
  in `chart::ui::show`); scrolling a dialog used to zoom the chart behind it.
- **Drags hit-test at the press origin** (`pointer.press_origin()`), not where the drag
  was recognised, or thin lines can't be grabbed.
- **Yahoo `range=max` silently returns 3-month bars**: `period1=0&period2=now` instead;
  replies with the wrong `dataGranularity` are rejected.
- **Providers are shared via `Arc`** (a session used to swap its provider out while
  streaming, so "load more" hit a placeholder).
- rustfmt reformats long lines, so scripted search-and-replace on code must match the
  formatted text (several edits silently missed; check with grep after editing).
- The demo provider slices one fixed path per series (history, paging and the live bar
  agree) with a gentle pull to its start price.
- Missing values print as "—" (fonts lack "∅"). Range-bar clock repaints per minute.
- Area-specific gotchas live in the notes (data and providers: `docs/notes/data.md`).

## Open questions for the maintainer / to verify
- **EMA seeding** (SMA of the first n values, as implemented) is the first thing to
  confirm with a TradingView export; see `docs/ta-semantics.md`.
- Default inputs/colours under "Defaults to double-check" in `docs/indicators.md`.
- Strategy Report formulas marked "to verify" in `docs/metrics.md` (Sharpe/Sortino
  period and stdev convention, buy & hold basis, account size required, margin).
- Zig Zag, Auto Fib and Pivot Points Standard follow TradingView's behaviour as
  remembered (their open questions are in `docs/indicators.md`); check them against
  exports.

## Dev loop
```bash
cargo run --profile fast -- --demo --data-dir /tmp/opentern-test   # made-up prices, no network
cargo run --profile fast -- --data-dir /tmp/opentern-test          # live Binance + Yahoo
cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --all
scripts/check-leaks.sh
cargo test -p opentern-data --test live -- --ignored --nocapture --test-threads=1  # real services (incl. formulas, crypto exchanges, Yahoo dividends)
cargo test --release -p opentern-series --test golden -- --ignored --nocapture  # 1M-bar timing
OPENTERN_PRIVATE_CORPUS=<dir> cargo test -p opentern-series --test golden  # TradingView exports
MONKEY_FRAMES=5000 cargo test -p opentern monkey                   # longer random-input soak
```
Stills also take `compare:<ticker>:scale|pane`, `favbar` or `favbar:vertical` (the favourite drawings toolbar), `measure` (a quick measure once
bars are in), `fib` or `fib:levels` (a selected Fib, its Levels panel open), `price-menu` (the "+" menu by the price scale), `note` (a sample notification), `tab:<name>`, `max` (bottom panel
maximized), `trades` (Strategy Report → List of trades), `gradient` (gradient canvas), `replay:play` (a replay under way) and
`lang:<code>` (e.g. `lang:ja`) and `settings:<general|chart|providers|shortcuts|about>` (add `scroll-end` to start at the bottom).
`crates/app/src/drive.rs` drives the real app headlessly (synthetic mouse/keys, demo
data): use it for interaction tests. `OPENTERN_STILL=indicators|symbol|chart-settings|
study:<id>|save-template` opens a dialog at launch for screenshots; `script:<file>` puts a
workspace script on the chart and `add:<id>` a built-in (comma-separate several). `OPENTERN_PYTHON` picks the interpreter.
```bash
cargo test --release -p opentern-pine --test conformance -- --ignored --nocapture --test-threads=1  # Pine timing
```

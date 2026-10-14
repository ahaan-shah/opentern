# Opentern — briefing for Claude sessions

This file is the shared memory of every session that works on the repo. Read all of it before touching anything. It holds the
research, the decisions and the A–Z roadmap. If you learn something the next session needs,
or a decision changes, update this file in the same commit. Keep it a briefing, not a diary.
`AGENTS.md` is a symlink to this file. Sibling project and design reference:
`~/projects/magpie` (read its `CLAUDE.md`; a lot is reused from it, see §3).

**Status (Oct 2026):** Phases 0–11 built and committed (one commit per phase, as the
maintainer asked). The maintainer asked for everything through Phase 13, with the backend
solid and the UI in Magpie's style, before they test and iterate; Phase 14 (ship) waits for
an explicit go. Read "Where things stand" → "Resume here" at the end first.

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
- **Every standard TradingView built-in ships by default**, plus licence-checked
  open-source community scripts. They're ported from Pine to Rust with exact numerical
  parity (§6a).

## How the maintainer works (inherited from Magpie, read this twice)

- **Nothing outward-facing without an explicit go.** That covers releases, making the repo
  public, publishing packages and force-pushes. Approval for one doesn't carry to the next.
- **They test before you release.** Build it, run it locally, let them try it and iterate.
- **Design taste:** clean, calm, modern, subtle. Accent colour is used sparingly. Error text
  is plain and human, not red stack traces.
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
  no hardcoded paths. Settings → Your data offers backup and restore of the whole
  workspace as one file, so a user can move machines.
- **No telemetry.** The only outbound traffic is the update check (GitHub releases, can be
  turned off) and the data providers the user has turned on.
- **Logs stay local** and are redacted: no API keys, tokens or account IDs, even in panic
  reports.
- **Private test material stays out of the repo.** The maintainer's own Pine scripts and
  TradingView exports used for conformance live in a folder outside it, pointed to by
  `OPENTERN_PRIVATE_CORPUS` and skipped when unset. Committed fixtures are synthetic data,
  or small slices of free public exchange data (e.g. Binance public klines).
- **Guardrail:** `scripts/check-leaks.sh` runs in CI and before every commit. It fails on
  absolute home paths, private-key blocks, common API-key shapes, and committed `*.db` or
  `.env` files. If it flags something, fix the leak; don't weaken the check.

## 1. Product & bar
Opentern is a local-first trading terminal: TradingView-grade charts, multi-language
scripting (indicators and strategies), a backtester, alerts and paper trading. There's no
account and no telemetry, and the data stays on the machine. It inherits Magpie's bar:
**blazingly fast, beautiful, light on RAM, fluid, never crashes, easy install on
Linux/macOS/Windows**. It also inherits "How the maintainer works" from Magpie's CLAUDE.md
(nothing outward-facing without a go, they test before release, plain wording, honest
reports, never touch real data). That section is above.

## 2. Architecture (Cargo workspace)
```
crates/core      opentern-core    Bar/Tick/Symbol/Timeframe models, SQLite store, settings, update.rs (from Magpie)
crates/data      opentern-data    provider traits + adapters, tokio network thread, bar builder/resampler, cache
crates/series    opentern-series  series runtime: bar-by-bar execution model, StudyOutput protocol, built-in indicators
crates/pine      opentern-pine    native Pine v5/v6 engine: lexer → parser → type check → bytecode VM on opentern-series
crates/script    opentern-script  language hosts: Python (subprocess), Luau (mlua), JS/TS later (WASM skipped for now)
crates/engine    opentern-engine  backtester, paper broker, optimizer, alert evaluator
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

## 3. What to reuse from Magpie (copy and adapt, then credit in comments)
| Magpie file | Use in Opentern |
|---|---|
| `crates/core/src/update.rs`, `crates/app/src/updater.rs` | Self-updater unchanged apart from the repo URL and `OPENTERN_UPDATE_FEED` |
| `crates/app/src/theme.rs` | Token themes, crossfade and fonts; add chart tokens (up/down candle, grid, crosshair, scale) |
| `crates/app/src/motion.rs` | Durations and tweens |
| `crates/app/src/widgets/mod.rs` | Buttons, cards, `dropdown`/`option`, inputs, date picker |
| `crates/app/src/widgets/charts.rs` | The `axis()` nice-step helper and collision-aware labels, as a base for price and time scales |
| `crates/app/src/palette.rs` | Ctrl+K command palette, which doubles as symbol search |
| `crates/app/src/dialogs.rs`, `diag.rs` | Async file dialogs, breadcrumbs, stall watchdog, panic log |
| `crates/app/src/film.rs` (stills), `monkey.rs`, `whatsnew.rs` | UI still tests, fuzz test, What's new card |
| `install.sh`, `install.ps1`, `.github/workflows/release.yml`, `packaging/`, `scripts/make-icons.py`, `scripts/third-party-licenses.py` | Release pipeline (asset names become a contract again) |
| `main.rs` renderer choice (glow, with a wgpu fallback on Windows) | Same |
| Magpie `CLAUDE.md` "Why it's built this way" gotchas | Carry over the relevant ones: vsync, Wayland hidden window, hover_wash, popup focus, scroll areas |

Later option: extract the shared pieces into an `ahaan-ui` crate both apps depend on. Not now.

## 4. Chart engine (custom egui painter, `crates/app/src/chart/`)
- **Rendering:** each layer becomes one batched `egui::Mesh` (no per-candle Shapes). When
  there are more bars than pixel columns, apply level-of-detail: an OHLC min/max per pixel
  column. The x axis is **bar-index based**, so weekends and overnight gaps vanish as they
  do on TradingView, with a separate time→index map. Target: 1M bars, smooth pan and zoom
  at 144 fps.
- **Chart types:** candles, hollow candles, Heikin Ashi, OHLC bars, line, line with
  markers, step, area, baseline, columns, high-low. Later: Renko, Kagi, Point & Figure,
  Line Break, range bars, volume candles, footprint/order flow (Sierra/ATAS style).
- **Scales:** linear, log, percent, indexed to 100, auto-fit or locked, invert, multiple
  price scales (left and right), and per-pane scales. Price labels show last price,
  pre/post market, high/low and countdown to bar close.
- **Interaction:** pan, wheel zoom, scale drag, double-click reset, magnet crosshair, and
  a data window (OHLCV and every indicator value under the cursor). Keyboard: arrows step
  bars, and typing anywhere opens symbol search (as on TradingView).
- **Panes:** stacked indicator panes with resizable dividers, collapse, maximize, and
  moving an indicator between panes.
- **Drawings (v1):** trend line, ray, extended line, horizontal and vertical lines,
  parallel channel, rectangle, ellipse, path, brush, text, arrow, Fib retracement and
  extension, Fib time zones, pitchfork, long and short position (R:R) tools, price and
  date range measure. Drawings magnet to OHLC, persist per symbol, sync across layouts,
  and include templates, lock/hide and an object tree.
- **Multi-chart layouts:** 1–8 charts with synced symbol, interval, crosshair and time.
  Plus compare and overlay symbols, session breaks, extended hours shading, dividends and
  splits and earnings markers.
- **Bar replay:** step or play from any date. It also drives backtest visualization.
- **Indicator overlays look exactly like TradingView's:**
  - Same plot styles (line, step line, histogram, columns, area, circles, cross,
    linebr), line widths, transparency, and z-order (bgcolor, fills, plots, then shapes
    and labels).
  - Label, shape and table styles and positions are the same, the default palette uses
    TradingView's `color.*` values, and the pane title row shows each indicator's
    name, inputs and current values with eye, settings and remove buttons.
  - The price scale shows a label per plot.
  - Every Pine visual element is reproduced, so a script ported from TradingView looks
    the same.
- **Ideas from other platforms:** volume profile (fixed and visible range, VPVR), anchored
  VWAP, multi-timeframe overlays (TrendSpider), AmiBroker-style exploration grid, a
  thinkorswim-style flexible grid, a Bookmap-style DOM heatmap (later), and chart trading
  with draggable SL/TP lines.

## 4a. Layout (TradingView's arrangement, Magpie's calm)

Traders already know TradingView's layout and it works, so Opentern keeps its arrangement.
Magpie's themes, fonts, spacing, motion and hover rules provide the look. Sketch:

```
┌ top bar ─────────────────────────────────────────────────────────────────────────┐
│ ☰ │ BTCUSDT ▾ │ + │ 1m 5m 15m 1h 4h D ▾ │ candles ▾ │ ƒx Indicators │ ⊞ Templates ▾ │
│   Alert │ Replay │ ↶ ↷ │ ……… │ grid ▾ │ Layout ▾ │ ⚙ │ ⤢ │ ⎙                         │
├──┬───────────────────────────────────────────────────────┬──────────────────┬───┤
│  │ legend: BTCUSDT · 1h · Binance  O… H… L… C… +1.2%     │ Watchlist        │ ≡ │
│ d│   SMA 20 close  64,210.5   [eye ⚙ ✕ …]                 │ Sym  Last  Chg%  │ ⏰ │
│ r│                                                       │ …                │ ⌗ │
│ a│                    chart pane(s)                price │──────────────────│ ▦ │
│ w│                                                 scale │ Details          │ ▤ │
│ i│───────────── indicator pane (RSI) ───────────────     │ (symbol info,    │   │
│ n│                                                       │  day range, perf)│   │
│ g│                     time scale                         │                  │   │
├──┴───────────────────────────────────────────────────────┴──────────────────┴───┤
│ 1D 5D 1M 3M 6M YTD 1Y 5Y All │ Go to date │            14:32:05 UTC+5:30 │ ADJ EXT % log auto │
├──────────────────────────────────────────────────────────────────────────────────┤
│ Screener │ Script Editor │ Strategy Report │ Paper Trading │ Alerts      [▴ ▾ ⤢ ✕] │
└──────────────────────────────────────────────────────────────────────────────────┘
```

- **Left drawing toolbar.** Grouped tools with flyouts that remember the last tool used
  (lines, Fib/Gann, patterns, forecasting/measure, shapes/brushes, text/notes, icons). At
  the bottom: measure, zoom, magnet (weak/strong), stay-in-drawing mode, lock all, hide
  all, remove. There's also a favorites floating toolbar.
- **Top bar.** Symbol search, compare/overlay, interval (favorites shown inline, the rest
  in a menu, custom intervals allowed), chart type, Indicators (`/`), indicator templates,
  alert, bar replay, undo and redo. On the right: multi-chart grid, layout name/save/load,
  chart settings, fullscreen, screenshot.
- **Right widget bar.** An icon rail toggles a resizable widget panel: Watchlist + Details,
  Alerts, Object tree + Data window, Hotlists, Calendar, News and Notifications. Widgets
  can stack in a split.
- **Bottom bar** for date-range presets, go to date, clock and timezone picker, and the
  ADJ / EXT / % / log / auto scale toggles.
- **Bottom panel** for Screener, Script Editor, Strategy Report, Paper Trading and Alerts
  log. It slides up (Magpie motion) and can be resized, maximized or collapsed.
- **Context:**
  - Right-click menus on chart, scales, legend rows and drawings.
  - A selected drawing shows a floating toolbar (colour, width, style, template, lock,
    clone, delete).
  - Selected indicators show their legend row's buttons.
- **Keyboard (TradingView's):**
  - Typing anywhere opens symbol search, and typing a number opens the interval box.
  - `Alt+T` trend line, `Alt+H` horizontal line, `Alt+F` Fib, `Alt+A` alert, `Alt+R`
    reset chart, `Alt+S` screenshot.
  - `Shift`-drag measures, `Esc` cancels, and `Ctrl+Z/Y` undo and redo.
  - Plus Magpie's `Ctrl+K` palette, which TradingView lacks.
- **Every panel** is resizable and collapsible, and its state is saved per layout. Later,
  charts can be detached into their own OS windows (egui multi-viewport) for
  multi-monitor setups.
- Phone-width layouts are out of scope. The minimum window is about 1000×640.

## 4b. Personal customization & templates (TradingView parity)

- **Chart settings dialog** with TradingView's tabs:
  - **Symbol:** chart type; up/down body, border and wick colours; colour bars based on
    previous close; hollow bodies; precision; timezone; regular or extended session.
  - **Status line:** logo, title format, OHLC values, bar change, volume, and indicator
    titles/inputs/values.
  - **Scales and lines:** price labels and lines for symbol, previous close, high/low,
    indicators and countdown; scale placement; date and time formats.
  - **Canvas:** background (solid or gradient), vertical and horizontal grid colours,
    crosshair colour/style/width, watermark, scale text size and colour, margins.
  - **Trading:** positions, orders and executions on the chart.
  - **Events:** dividends, splits, earnings, session breaks.
- **Colour picker like TradingView's:** swatch grid, custom colours, opacity slider and
  recent colours.
- **Chart templates:** save the current chart settings as a named template; apply it, set
  it as default, or apply it to all charts. By default, chart colours follow the app
  theme until customised.
- **Indicator templates:** save the current set of indicators (inputs, styles, panes and
  their order) under a name, optionally with symbol and interval. Apply one from the top
  bar's Templates menu.
- **Per-indicator defaults:** the settings dialog (Inputs / Style / Visibility) has a
  Defaults menu with Save as default, Reset settings and named presets.
- **Drawing templates** per tool (save as, apply, set default), and a separate default
  style per drawing tool.
- **Favorites:** star intervals, chart types, drawing tools and indicators, and they
  surface in the top bar and the favorites toolbar.
- **Layouts:** named saved layouts (grid, symbols, intervals, indicators, drawings,
  panel sizes) that autosave, can be copied, and load from a list.
- **Sharing:** templates and layouts can be exported and imported as JSON files.
- **Storage:** SQLite tables `chart_templates`, `indicator_templates`,
  `indicator_defaults`, `drawing_templates`, `favorites`, `layouts`. Each JSON payload
  carries a `version` field so old templates keep loading.

## 5. Data layer (`opentern-data`)
Traits: `SymbolSource` (search and metadata: tick size, session, tz, currency, type),
`HistorySource` (bars with paging), `StreamSource` (trades/quotes/klines over websockets),
and later `Broker`. A **BarBuilder** aggregates trades into any timeframe, including custom
ones like 7m, 2h, 3D or tick and range bars. A **Resampler** builds higher timeframes from
the base. The cache stores bars and only backfills the gaps.

| Market | Provider | Cost / notes |
|---|---|---|
| Crypto | Binance, Bybit, OKX, Coinbase, Kraken, written as direct REST and WS adapters (CCXT isn't Rust) | Free real-time, deep history |
| US stocks | Alpaca (IEX feed) | Free real-time WS, IEX only; consolidated SIP is paid |
| US stocks | Yahoo (unofficial) | Free history (intraday ~60 days); fragile, best effort only |
| US stocks | Polygon (rebranded Massive), Twelve Data, Finnhub, Tiingo | Free tiers are EOD or delayed; paid for real-time; Finnhub adds news |
| India | Upstox, Angel One SmartAPI, Dhan, Fyers, Zerodha Kite Connect | Need a broker account; check current data-API pricing for each; Kite data is paid |
| Forex/CFD | OANDA v20 (practice account), Dukascopy (free historical ticks) | |
| Futures/all | Interactive Brokers TWS/Gateway API | Needs IBKR account and market-data subscriptions |
| Any | CSV / Parquet import | Offline research |

Also needed: exchange calendars (sessions, holidays, half days), corporate-action
adjustment (splits and dividends) for equities, and symbol mapping (`BINANCE:BTCUSDT`,
`NSE:RELIANCE`, `NASDAQ:AAPL`).

**Legal:** never scrape TradingView data or use its proprietary charting library. Respect
each provider's ToS. Data is for local personal use only, with no redistribution.

## 6. Scripting: the heart of the app
**One runtime model and many languages.** Every language produces the same thing: a
`Study` that runs **bar by bar** with Pine semantics. Those semantics are: history
references (`close[1]`), persistent `var` state, and realtime-bar recompute with rollback
on each tick (commit on bar close). Each study returns a `StudyOutput`: plots (line,
histogram, area, columns, circles, cross), hlines, fills, bgcolor and barcolor, shapes and
chars, labels, lines, boxes, polylines, tables, alerts and strategy orders. The chart
renders `StudyOutput` the same way whatever the language. Inputs are declared by the
script and rendered as an auto-generated settings dialog (Inputs, Style and Visibility
tabs, as on TradingView).

| Language | How | Why |
|---|---|---|
| **Pine Script v5/v6** | **A native Rust engine** (`opentern-pine`). A `logos` lexer handles Pine's indentation-sensitive tokens. Then a hand-written recursive-descent parser builds the AST, and a type checker resolves the qualifier lattice (const < input < simple < series) and the types, including UDTs and generics. The result compiles to register bytecode, which runs bar by bar on `opentern-series` with a ring-buffer history per series variable (max_bars_back), `var`/`varip` slots, and realtime rollback/commit snapshots. The built-ins (`ta.*`, `math.*`, `str.*`, `array`/`map`/`matrix`) are Rust functions shared with the native indicators | **Chosen over PineTS (the maintainer: "whatever's best").** There's no JS runtime, sidecar or per-tick serialization, and no AGPL. Startup is instant and it runs at native speed, which matters for screeners and optimizers that run thousands of times. One engine owns all semantics, so Pine, Python and Luau behave identically. **Rejected:** PineTS (AGPL, needs a JS runtime, an extra process and a translation layer) and PyneCore (Python, and compiling Pine needs a paid PyneSys key). Call it "Pine Script®-compatible" because it's a TradingView trademark |
| **Python** | A separate worker process with the user's own interpreter, or an app-managed venv created with `uv`. Bars go over Arrow IPC and results come back the same way. Ships the `opentern` pip SDK: vectorized `compute(df) -> outputs` (pandas or polars) and event-driven `on_bar(ctx)` | Full ecosystem (numpy, pandas, TA-Lib, scikit, vectorbt). Process isolation means it can't crash the app, and no Python needs bundling |
| **Luau** (via `mlua`) | Embedded and sandboxed, with an instruction budget | Instant startup and fast. A great quick-scripting language for small indicators and alerts |
| **WASM plugins** (wasmtime). **Skipped for now, the maintainer's call (Oct 2026)** | A stable ABI, so Rust, C/C++, Zig, Go and AssemblyScript all compile to it | Near-native speed and safe sandboxing, and a way to share compiled indicators |
| **Rust built-ins** | ~100 indicators written natively against the same API | Fastest. Also the reference for parity tests |
| Later: **JS/TS** (`rquickjs`, types stripped with swc), and transpilers for **EasyLanguage / ThinkScript / AFL / MQL** into the Pine VM's IR | | Migration paths for users of other platforms |

**Pine build order.** Each step ships something usable:
1. Language core: literals, types, operators, `var`/`varip`, if/for/for-in/while/switch,
   functions, tuples, history `[]`, `na` handling. Also `//@version=5/6` (v6 rules: bool
   can't be na, lazy `and`/`or`, dynamic requests).
2. `indicator()`, `input.*`, `ta.*`, `math.*`, `str.*`, `color.*`, and the
   `plot`/`plotshape`/`plotchar`/`plotcandle`/`plotbar`/`hline`/`fill`/`bgcolor`/`barcolor`
   family. This covers most public indicators.
3. `strategy()` and `strategy.*`: entry, order, exit, close, cancel, OCA, pyramiding, risk
   functions and the `strategy.*` built-in variables. These feed the Rust broker emulator
   (§7).
4. `request.security` (with lookahead and gaps rules, and repainting semantics on
   realtime), `request.security_lower_tf`, `timeframe.*`, `syminfo.*`, sessions and time
   functions.
5. Drawing objects (label, line, box, table, linefill, polyline, and the `max_*_count`
   limits) plus array, map, matrix, UDTs, methods, enums and `alert`/`alertcondition`.
6. `library()` and `import` (libraries are local files), and the edge cases the
   conformance suite finds.

**Reference sources:** TradingView's Pine v6 Reference Manual and User Manual (written
semantics, especially execution model, time series, `request.*` and repainting).
TradingView exports, which are the **ground truth**. PineTS and pynescript source only as a
reading aid for tricky semantics: never copied (AGPL/LGPL), never shipped.

**Conformance suite (`crates/pine/tests/corpus/`):** about 100 scripts (built-in
TradingView indicators rewritten from the docs, plus the maintainer's own scripts). Each has golden
outputs that the maintainer exports from TradingView: the chart's "Export data" CSV for plots, and
the Strategy Report's trade list and metrics. A test runs each script on the same bars and
diffs the results within a 1e-9 relative tolerance. Errors are shown with line numbers in
plain wording.

**Editor:** an in-app pane with syntax highlighting (tree-sitter grammars), error squiggles,
autocomplete for built-ins and a console/log pane. There's also **"Open in external
editor"**: the file is watched with `notify` and hot-reloads on save, so VS Code or nvim
work. A Pine and Python LSP comes later.

## 6a. Built-in indicator library (Pine → Rust, exact)

**Goal:** the Indicators dialog matches TradingView's. It has search and tabs for
Favorites / Built-ins / Community / My scripts, with categories. **Every standard
TradingView built-in is there by default**, written in Rust on the shared series runtime,
numerically identical to TradingView, and with TradingView's default inputs, styles,
colours, levels and band fills. The checklist and status per indicator live in
`docs/indicators.md`.

**TradingView built-ins (target list, verify the full set against TradingView's Indicators
dialog):**
24-hour Volume, Accumulation/Distribution, Advance/Decline (Line, Ratio), ALMA, Aroon, ATR,
Auto Fib Extension/Retracement, Average Day Range, ADX, Awesome Oscillator, Balance of
Power, Bollinger Bands (+ %B, Width, Bars), Bull Bear Power, Chaikin Money Flow, Chaikin
Oscillator, Chande Kroll Stop, Chande Momentum Oscillator, Chop Zone, Choppiness Index,
CCI, Connors RSI, Coppock Curve, Correlation Coefficient, Cumulative Volume Delta, Cumulative
Volume Index, DEMA, Detrended Price Oscillator, DMI, Donchian Channels, Ease of Movement,
Elder Force Index, EMA, EMA Cross, Envelope, Fisher Transform, Historical Volatility, Hull
MA, Ichimoku Cloud, Keltner Channels, Klinger Oscillator, Know Sure Thing, Least Squares
MA, Linear Regression Channel, MA Cross, MA Ribbon, Majority Rule, Mass Index, McGinley
Dynamic, Median, Momentum, Money Flow Index, Moon Phases, MACD, Moving Average (SMA,
Weighted, Smoothed, Triple EMA, Multiple), Net Volume, On Balance Volume, Open Interest,
Parabolic SAR, Pivot Points (Standard, High Low), Price Oscillator, Price Volume Trend,
Rank Correlation Index, Rate of Change, Relative Strength Index, Relative Vigor Index,
Relative Volatility Index, RCI Ribbon, Seasonality, SMI Ergodic Indicator/Oscillator,
Smoothed MA, Standard Deviation, Standard Error (+ Bands), Stochastic, Stochastic Momentum
Index, Stochastic RSI, SuperTrend, Technical Ratings, Time Weighted Average Price, Trading
Sessions, Trend Strength Index, Triple EMA, TRIX, True Strength Index, Ultimate Oscillator,
Up/Down Volume, Visible Average Price, Volatility Stop, Volume, Volume Oscillator, Volume
Profile (Fixed Range, Visible Range, Session, Anchored), VWAP (+ Anchored), VWMA, Vortex
Indicator, Williams Alligator, Williams Fractals, Williams %R, Woodies CCI, Zig Zag.

**Community picks:** a curated set of popular **open-source** TradingView community
scripts, included **only when the script's licence allows it**.
- Most open-source TradingView scripts carry an MPL-2.0 header. A Rust port of one stays
  under MPL-2.0 as its own file. MPL is file-level copyleft, which works inside the MIT
  app. Credit the author and link the original in the file header, the indicator's About
  box and `THIRD-PARTY-LICENSES.txt`.
- **Excluded:** CC BY-NC-SA scripts (e.g. LuxAlgo's), "all rights reserved", protected or
  invite-only scripts, and scripts with no licence line.
- Every inclusion is logged in `docs/indicators.md` with author, URL, licence and the
  version ported.
- Candidates, each needing a licence check: Squeeze Momentum (LazyBear), WaveTrend
  (LazyBear), Williams Vix Fix (ChrisMoody), Chandelier Exit, SSL Channel, Half Trend, QQE
  MOD, UT Bot, Range Filter, Hull Suite, Volume-weighted MACD, Pivot-based S/R, Market
  Structure (BOS/CHoCH) and Order Blocks (only from MPL sources).

**How each port is made accurate:**
1. Get the exact Pine source (TradingView shows the source of its built-ins; check its
   licence header).
2. **Pine-exact `ta` primitives first.** `ta.sma`, `ema`, `rma`, `wma`, `vwma`, `alma`,
   `stdev`, `highest`/`lowest`, `crossover`, `pivothigh`, `atr`, `tr`, `change`, `rsi`,
   `stoch`, `dmi` and so on are each implemented once in `opentern-series::ta`. Their
   seeding, warm-up and `na` rules are written down in `docs/ta-semantics.md` from the
   Pine Reference Manual and checked against exports. Indicators are composed only from
   these primitives, line by line like the Pine source, so the semantics can't drift.
3. **Golden tests:** export the indicator from TradingView ("Export chart data") on a
   fixed public symbol and timeframe (e.g. BINANCE:BTCUSDT 1D, 4h; and a stock). Check
   that the same bars are `na`, and that values match within a 1e-9 relative tolerance
   after warm-up. Only small public-data slices get committed.
4. **Triple check** once the Pine engine exists: Rust built-in = Pine engine running the
   original source = TradingView export. This is a CI test for every built-in.
5. Edge cases tested: bar 0, warm-up, gaps, `na` inputs, realtime bar updates and
   rollback, and a changed timeframe.

## 7. Backtester, Strategy Tester & paper trading (`opentern-engine`)
- **Event-driven, deterministic bar-by-bar engine** that matches TradingView's broker
  emulator defaults: orders fill on the next bar's open, `process_orders_on_close`,
  intrabar path assumptions, a **bar magnifier** that uses lower-timeframe data,
  commission (percent, per contract or per order), slippage, margin, pyramiding and
  currency conversion.
- **Strategy Report: a TradingView replica, which is a hard requirement.** It uses the same
  tabs, layout and metric names, and the same formulas.
  - Tabs: **Overview** (equity curve with buy-and-hold overlay, drawdown and run-up
    histograms, headline Net PnL / Max DD / Total trades / % profitable / Profit factor),
    **Performance**, **Trades analysis**, **Risk/performance ratios**, **List of trades**
    and **Properties**. Columns are All / Long / Short where TradingView has them.
  - Metrics: all 53 TradingView metrics, implemented in `opentern-engine::report` from the
    trade list and the bar-by-bar equity series:
    - **Performance:** Initial capital, Open PnL, Net PnL, Gross profit and loss, Profit
      factor, Commission paid, Expected payoff, Buy and hold PnL and % gain, Strategy
      outperformance, Sharpe, Sortino.
    - **Trades:** Total, open, winners, losers and even trades, Percent profitable, Average
      PnL, profit and loss, Avg profit / avg loss, Largest profit and loss (also as a % of
      gross), Average bars in trades, winners and losers.
    - **Capital:** CAGR, Return on initial capital, Account size required and the return
      on it, Net profit as % of largest loss, Average and max margin used, Margin
      efficiency, Margin calls.
    - **Run-up and drawdown:** average duration, average and max (close-to-close and
      intrabar), max as % of initial capital, Return of max drawdown.
    - **Volume:** Max contracts held, Total and Largest liquidated volume.
  - Formulas come from each TradingView help article in the "Strategy report metrics"
    folder. They are written out in `docs/metrics.md` before coding, including details
    such as the Sharpe and Sortino period and risk-free rate.
  - Each metric is tested against TradingView exports of reference strategies, so numbers
    must match to the cent.
  - Exports go to CSV and XLSX, and the trade list matches TradingView's columns.
- **Strategy overlays on the chart, matching TradingView:** entry and exit arrows with
  order-id labels and quantity (e.g. "Long +1", "Close entry(s) order Long -1"), position
  avg-price line, open trade P&L, and dashed lines linking entry to exit. Toggles: trades
  on chart, signal labels, quantity.
- **Beyond TradingView:** a portfolio and multi-symbol backtest, a parallel parameter
  optimizer (grid, random, then Bayesian; results as a heatmap), walk-forward analysis,
  Monte Carlo trade resampling, and exporting results to CSV and JSON.
- **Paper broker:** multiple simulated accounts. Order types: market, limit, stop,
  stop-limit, trailing, bracket and OCO. Includes a positions and orders panel, a DOM,
  trading from the chart, and a P&L history.
- **Live broker routing** comes later behind the same `Broker` trait, starting with
  Alpaca, a crypto exchange, OANDA and an Indian broker. It needs a risk-limits module and
  a confirm-every-order mode.

## 8. Alerts, watchlists, screener & extras
- **Alerts:** on price, a drawing crossing, an indicator condition or a script's
  `alert()`/`alertcondition`. They're evaluated by the engine whether or not a chart is
  open. Delivery: desktop notification (`notify-rust`), sound, webhook, Telegram and
  email. A headless `opentern --daemon` comes later so alerts keep running with the
  window closed.
- **Watchlists** with configurable columns, sparklines, flags/colors and sections, plus a
  hotlist.
- **Screener:** built-in filters and **script-based screens** (run a study across N
  symbols on the rayon pool, AmiBroker-explore style).
- Also: a market heatmap, news (Finnhub), an economic calendar, a symbol info panel,
  fundamentals later, and an object tree.
- **UX:** Ctrl+K palette, keyboard shortcuts for everything (Magpie rule), Magpie
  Light/Dark plus chart themes, and a saved workspace and layout state.

## 9. Step-by-step roadmap
| Phase | Deliverable | Done when |
|---|---|---|
| 0. Scaffold | Workspace and crates; copy Magpie's theme, motion, diag, updater; CI (fmt, clippy, test on 3 OSes, leak check); TradingView-style layout shell (§4a): top bar, drawing toolbar, widget panel and rail, bottom bar and bottom panel | `cargo run --profile fast` opens the themed shell; checks pass |
| 1. Data core | Bar/Symbol models, SQLite cache, Binance history + WS, Yahoo history, BarBuilder/Resampler, symbol search | BTCUSDT 1m streams live into the cache |
| 2. Chart v1 | Candles, line, area; price and time scales; pan, zoom, crosshair; volume; panes; LOD; live updates | 1M bars pan smoothly; idle at 0% CPU |
| 3. Built-ins wave 1 | Pine-exact `ta` primitives and `docs/ta-semantics.md`; the ~40 most-used TradingView built-ins (§6a); Indicators dialog; settings dialog (Inputs/Style/Visibility, Defaults menu); chart settings dialog; chart and indicator templates (§4b) | Golden tests against TradingView exports pass |
| 4. Drawings & layouts | Drawing tools v1, persistence, watchlists, saved layouts | Drawings survive a restart |
| 5. Script runtime + Python | `opentern-series` StudyOutput protocol; Python worker and the pip SDK; editor pane and external-editor hot reload | A Python indicator plots on the chart and reloads on save |
| 6. Pine v1 | Pine steps 1–2 and the conformance-suite harness | Corpus indicators match TradingView's exported values |
| 7. Backtester | Engine, `docs/metrics.md`, all 53-metric report, TradingView-style Strategy Report and chart overlays, paper broker, Pine step 3 | Reference strategies match TradingView's report and trade list exactly |
| 8. Alerts | Alert engine plus notification channels | Script alerts fire a desktop notification |
| 9. Providers | Alpaca, an Indian broker (Upstox or Angel One first), OANDA, IBKR, Coinbase/Bybit/Kraken; exchange calendars; split/dividend adjustment | Each market streams live |
| 9b. Built-ins wave 2 | The rest of TradingView's built-ins plus licence-checked community picks, each triple-checked (§6a) | Every item in `docs/indicators.md` is done |
| 10. Pine v2 | Pine steps 4–6 (MTF, drawing objects, collections, UDTs, libraries) | The full corpus passes, and the maintainer's own TradingView scripts run unchanged |
| 11. Power features | Multi-chart layouts, replay, compare, more chart types, volume profile, screener | |
| 12. Quant tools | Optimizer, walk-forward, Monte Carlo, portfolio backtests | |
| 13. More languages | Luau, then JS/TS (WASM skipped for now); transpilers maybe | |
| 14. Ship | Releases via Magpie's pipeline, a website, and What's new notes | Only on an explicit go |
| Later | Live trading, `--daemon`, order-flow/footprint, DOM heatmap, LSP | |

## 10. Dependencies (expected)
eframe/egui 0.36 (glow), egui-phosphor, rusqlite (bundled), jiff, serde/serde_json,
tokio, tokio-tungstenite (rustls), reqwest or ureq (same as Magpie), crossbeam-channel,
arc-swap, rayon, keyring, notify, notify-rust, arrow (IPC), parquet/polars (optional),
mlua (luau feature), rquickjs, tree-sitter, logos (Pine lexer), thiserror, sha2/zip (the
updater). TA crates **wickra** and **mantis-ta** are only consulted or benchmarked. Built-in
indicators are hand-written so they share the series runtime.

## 11. Testing & verification conventions
- Indicator parity against TA-Lib reference CSVs. Pine conformance with golden outputs.
  Backtest determinism: the same input gives byte-identical trades.
- A recorded WS fixture replayer for provider tests, so tests run offline.
- Chart stills rendered to PNG (Magpie's `stills` pattern) for every visual change. Look
  at them.
- A monkey/fuzz test on the UI and a fuzz test on the Pine parser (`cargo fuzz`).
- Performance benches: chart frame time at 1M bars, and Pine eval over 100k bars (target
  < 100 ms for typical scripts).
- Env switches mirroring Magpie: `OPENTERN_DATA_DIR`, `OPENTERN_DEMO` (synthetic data
  with no network), `OPENTERN_HEADLESS`, `OPENTERN_UPDATE_FEED`.

## 12. Licensing
Opentern is MIT. Indicator ports of MPL-2.0 Pine scripts stay MPL-2.0 per file, with
author credit (§6a). No AGPL or LGPL code goes in: PineTS and pynescript may be read for
understanding but never copied. Pine Script® and
TradingView® are trademarks of TradingView, Inc. Never use TradingView data, code or its
charting library. Matching TradingView's look and metric formulas is fine; copying its
assets or icons is not.

## 13. Sources
LuxAlgo PineTS (AGPL-3.0 or commercial): github.com/LuxAlgo/PineTS. Evaluated and not
used (see §6).
TradingView metric definitions: tradingview.com/support/folders/43000599093-strategy-report-metrics/. PyneCore (Apache-2.0):
github.com/PyneSys/pynecore. pynescript (LGPL-3.0). NautilusTrader v2 (Rust, still at
release candidates as of Sep 2026). Polygon is now Massive. Alpaca IEX is the free
real-time feed. Rust TA crates: wickra, mantis-ta, kand.



## Where things stand

### Resume here (next session)
**Where we are (14 Oct 2026):** Phases 0–13 are built and committed, one commit per
phase. Nothing has been pushed or released. The maintainer said go for Phases 12–13
plus the loose ends, then hand them the app to test.

The maintainer's standing request: build Phases 12–13, with the backend solid and
the UI in Magpie's style, committing after each phase. Then they test everything and we
iterate on UI and bugs together. Phase 14 (ship) only on an explicit go.

**What's left, in order:**
1. ~~Phase 12, quant tools~~ (done, see below).
2. ~~Phase 13, more languages~~ (done: Luau, JS, TS; WASM skipped).
3. **The UI pass** (`docs/ui-backlog.md`), once the maintainer has tested.
4. **Phase 14** (releases, website), only on an explicit go.

**Loose ends from earlier phases** (fit in where natural):
- Done Oct 2026: `request.security` on other symbols and lower timeframes plus
  `request.security_lower_tf` (feeds, below); stock daily periods end at the
  session close; auto-scale counts script drawings; Auto Fib Extension, Median,
  Majority Rule.
- Correlation Coefficient as a Rust built-in (needs feeds for built-ins; Pine can
  do it now); Technical Ratings and Bollinger Bars (need TradingView's source or
  an export); the breadth/OI/intrabar-volume built-ins (`docs/indicators.md`).
- Keyed providers (Alpaca, OANDA, Upstox, IBKR) have never run against a real
  account. NSE 2026 holidays are in (from brokers' copies of NSE's list; check
  against the circular); add 2027's when announced.
- Done Oct 2026: Strategy settings dialog, chart trading (drag a paper order's
  tag, × cancels / closes), several paper accounts, email alerts. Still open: a
  `--daemon`, alerts on off-chart indicators.
- App icon, `THIRD-PARTY-LICENSES.txt` + `--licenses`, release workflow (Phase 14).

**Testing the app for the maintainer:** build with `cargo build --profile fast`.
Launch with a throwaway `--data-dir` through `hyprctl` (see `CLAUDE.local.md`). It
helps to drop a few demo scripts in `<data-dir>/scripts/`. Never use their real
workspace.

### Crates
| Crate | What it owns |
|---|---|
| `opentern-core` | `Bar`, `Timeframe`, `Symbol`, `starter_watchlist()`, SQLite workspace (`db.rs` migrations: v1 settings, v2 layouts/templates/indicator_defaults/favorites, v3 drawings/watchlists), `Store` (named layouts, templates, drawings per symbol, watchlists, `value`/`set_value` for app keys like `layout_id`), self-updater |
| `opentern-data` | `Hub`, providers `binance.rs`, `yahoo.rs`, `demo.rs`, **`expr.rs` (symbol maths: `BTCUSDT/ETHUSDT`, `(A + B)/2`; legs joined on time; quotes derived from legs)**, `cache.rs`, `resample.rs` |
| `opentern-series` | `ta.rs` (Pine-exact primitives; highest/lowest use a monotonic queue), `study.rs` (`Instance` with built-in, `with_factory` (Pine) and `external` (Python) modes; `Out`/`Output` with palette colours plus `raw` colours, `bg`, `bar`; `PlotStyle::Shape` + `ShapeSpec`; `InputKind::Text`), `builtins/` (62 indicators) |
| `opentern-script` | **Luau / JavaScript / TypeScript (Phase 13)**: `host.rs` (all semantics: declarations → `Decl`/`Meta`, `HostState` per-bar runtime with `ta.*` by call order, plots, `state` snapshots, strategy → `opentern_engine::Broker`), `inproc.rs` (`Lang`/`Vm` traits, `compile` = load + 60-bar dry run to find plots, `ScriptStudy`), `luau.rs` (mlua, Luau vendored), `js.rs` (rquickjs; TS via `swc_ts_fast_strip`, types blanked so lines match). Python host: SDK embedded from `python/opentern/` and unpacked to `<workspace>/python`; one worker process per script (`_runner.py`, JSON lines); reload on file change; errors and prints returned |
| `opentern-engine` | `quant/` (Phase 12: `optimize.rs` grid/random/Bayesian + heatmap, `gp.rs` Gaussian process + expected improvement, `walkforward.rs`, `montecarlo.rs`, `portfolio.rs`, `export.rs` CSV; all take a `run(values, bars) -> Broker` closure so any in-process language works; seeded and deterministic), `broker.rs` (TradingView-style emulator: next-open market fills, intrabar O→H→L→C / O→L→H→C path for stops/limits, gaps, reversals, pyramiding, brackets and tick exits, commission, slippage, qty types, process_orders_on_close, run-up/drawdown per trade, equity incl. intrabar) and `report.rs` (Strategy Report metrics) |
| `opentern-pine` | Pine engine: `lexer.rs`, `parser.rs` → `ast.rs`, `compile.rs` (names → frame slots, constant folding, declarations → `Meta`), `run.rs` (`PineStudy`, bar-by-bar interpreter), `natives/` (`mod.rs` core/math/str/color/time/syminfo/timeframe/barstate, `ta.rs` on `opentern_series::ta`), `natives/collections.rs` (array/map/matrix), `natives/draw.rs` (label/line/box/table/linefill/polyline/chart.point), `natives/security.rs` (`request.security`), `natives/strategy.rs`; `heap.rs` (objects with GC; drawing types live in `opentern_series::drawings`), `value.rs`; `tests/corpus/` + `tests/conformance.rs` |
| `opentern` (app) | `app.rs`, `shell.rs`, `chart/` (`draw.rs` + `drawing_ui.rs` = drawings), `drawbar.rs` (floating toolbar for the selected drawing), `watchlists.rs`, `layouts.rs`, `objects.rs` (Object tree + Data window), `scripting.rs` (scripts on the chart), `editor.rs` (Script Editor tab), `pine_host.rs` (Pine scripts on the chart), `dialogs/`, `drive.rs` (headless test driver) |

### Phase 4 (done): drawings, layouts, watchlists
- 27 tools in TradingView's groups (lines, Fib/Gann, patterns, forecasting/measure,
  shapes, text, icons) with flyouts that remember the last tool; anchored by time and
  price (`draw::TimeMap` interpolates between bars and extrapolates past the ends).
  Magnet, stay-in-drawing mode, lock/hide all, remove menu, Eraser, Zoom (drag a range).
- Select, move (body or handles), Delete, Esc, undo/redo (Ctrl+Z/Y, top bar buttons),
  floating toolbar (colours, width, style, text, extend, labels, lock, "use this style
  for new drawings" → templates kind `drawing`, name = `Tool::key()`).
- Drawings are saved **per symbol** (table `drawings`), not per layout.
- Named layouts (top bar menu: save, rename, copy, new, open, delete; autosaved;
  `layout_id` in settings). Several watchlists (create/rename/delete/switch, add via
  search, remove, move to top, sort by column).
- Shortcuts: Alt+T/H/V/F (trend, horizontal, vertical, Fib), Delete, Esc, Ctrl+Z/Y.

### Phase 5 (done): scripting + Python
- `python/opentern`: `indicator()`, `strategy()` (+ `strategy.entry/order/exit/close/
  close_all` with per-bar `when` series), `input.int/float/bool/string/source`,
  `plot`, `hline`, `Bars` (`.close`, `["hl2"]`, `.df()` with pandas), and `ta.*` in pure
  Python with Pine semantics (no numpy needed; numpy/pandas values accepted).
- Scripts live in `<workspace>/scripts/`; the Script Editor tab lists them, creates
  Python/Pine from templates, highlights, saves (Ctrl+S), adds to chart, opens in your
  own editor (the chart reloads on save), shows errors and prints. Errors also show in
  the study's legend row.
- Python runs re-trigger on new data at most twice a second, immediately on new
  inputs. `notify` crate not used: the worker polls the file's mtime every second.
- Verified: the SDK's RSI (pure Python) equals the Rust built-in RSI bar for bar on the
  demo chart (`drive::tests::python_script_…`).

### Phase 6 (done): Pine v1
- Pine steps 1–2: v5/v6 syntax incl. `var`/`varip`, tuples, functions, `if`/`switch`
  values, `for`/`while`/`break`/`continue`, `a = 1, b = 2`, history `[n]` on anything;
  `indicator`/`strategy` declarations, every `input.*` (colour inputs got `InputKind::Color`
  and a picker in the Inputs tab), `plot`/`plotshape`/`plotchar`/`plotarrow`/`plotcandle`/
  `plotbar` (new `PlotStyle::Candle`/`OhlcBar`/`Hidden`), `hline`, `fill`, `bgcolor`,
  `barcolor`, `alertcondition`, `alert`, `log.*`; `ta.*` (all common ones), `math.*`,
  `str.*`, `color.*`, time functions, `timeframe.*`, `syminfo.*`, `barstate.*`.
- Execution model: every variable is a frame slot; slots read with `[n]` keep a ring that
  is pushed at the end of each bar in which the slot was set (so a function's locals only
  get history on bars it ran, as in Pine); each user-function call site and each `ta.*`
  call site has its own state; v6 `and`/`or` are lazy, v5 aren't; ternaries/`if` only run
  the chosen branch. State is `Clone`: the forming bar runs on a copy (rollback).
  `Instance::latest_state()` gives the forming-bar state (errors, later drawings).
- **Conformance: 63 TradingView built-ins written in Pine (`tests/corpus/`) match the
  Rust built-ins bar for bar** on 1h and 1D synthetic data with gaps, flat bars and zero
  volume (`tests/conformance.rs`). This found two built-in bugs, now fixed: raw `/` where
  Pine gives `na` (EOM on zero volume and others now use `ta::div`), and `±inf` reaching
  plots (now `na` in `Output::write`).
- App: `pine_host.rs` compiles on save, on outside edits (mtime checked once a second)
  and on a symbol change (`syminfo` is baked into the compile). A compile error keeps
  the last good version running and shows "Line N: …" in the legend; runtime errors
  too. `Chart::set_inputs` now keeps a script's factory (`Instance::with_inputs`).
- Not yet (later phases): `strategy.*` (7), arrays/maps/matrices/UDTs/methods/drawings/
  `request.security`/libraries/enums (10). Hline prices and plot offsets that depend on
  non-trivial expressions use the input defaults. Dynamic lengths rebuild `ta.*` state.
  `varip` behaves like `var` (the forming bar is always recomputed from a copy).

### Phase 7 (done): strategies, Strategy Report, paper trading
- Pine `strategy.*` (`natives/strategy.rs`) drives `opentern_engine::Broker`; a Pine MA
  cross equals the same logic driven straight at the broker, trade for trade. Broker:
  trailing stops, OCA cancel/reduce, exit `qty_percent` of the entered size,
  `allow_entry_in`, running max drawdown/run-up, and **pyramiding fixed to TradingView's
  rule** (0 and 1 = one entry). `docs/metrics.md` records the fill model.
- `app/src/strategy.rs`: strategy runs are rebuilt only when `data_version`, `changed` or
  a Python result (`ScriptRun::out_seq`) changes; Python strategies replay their
  `intents` through a broker. Strategy Report tab: Overview (KPI cards, compact on short
  panels; equity, buy & hold and drawdown chart with hover), Performance, Trades
  analysis, Risk/performance ratios, List of trades (virtualised rows, Export → CSV in
  `<workspace>/exports/`), Properties. Short tab names under 980 px.
- Chart: `Chart::marks` (fills as arrows with "Long +1" labels, dashed entry→exit links,
  the open position's average price) and `Chart::paper` (paper position solid, orders
  dashed, with tags).
- Paper trading: `opentern_engine::paper::Account` (market/limit/stop/stop-limit/
  trailing, brackets as OCO, netting, commission), fed by the chart's last bar or quotes
  (paper symbols are added to the quote subscription), saved as JSON under the settings
  key `paper_account`. Tab: ticket, summary, Positions/Orders/History, Reset account.
  **Since Oct 2026:** several accounts (settings key `paper_accounts` = every
  account + which is in use; the old key migrates); every account's orders keep
  working; the account menu switches, makes, renames, deletes. Chart trading:
  `PaperLine::order` tags are drag handles (`Action::PaperMove` →
  `Account::modify`), the × beside a tag cancels it or closes the position.
- **Gotcha:** bottom-panel tab bodies render in `ui.new_child` with a clip rect; content
  taller than the panel used to grow it and push the tab strip under the range bar.
- **DOM widget** (Oct 2026, `app/src/dom.rs`, rail icon "Depth of market"):
  `Request::Depth` → `opentern_data::depth::book` (Binance, Bybit, OKX, Coinbase,
  Kraken public REST; verified live; demo book made up), polled 1/s while open.
  Ladder Buy | Bid | Price | Ask | Sell; a click places a paper limit (near side)
  or stop (far side); working orders show on their rows. Stocks/forex: plain
  "no free order book" message.

### Phase 8 (done): alerts
- `opentern_engine::alerts`: `Alert` (price / drawn line / study plot / script),
  `Op` (crossing, up, down, greater, less), `Freq` (only once, once per bar, once per bar
  close, every time ≤ 1/s), `Watch::feed` (pure, tested). DB v4: `alerts`, `alert_log`
  (latest 2,000).
- `app/src/alerts.rs`: evaluated every frame but only fed new (value, level, bar)
  triples; price alerts work for any symbol with a price (chart or quotes), plot alerts
  need the study on the chart; line alerts follow their drawing; script alerts
  (`alert()`, `alertcondition`) fire only for alerts newer than when they were armed
  (deduped by bar time and message). Fired → log, toast (top right, 8 s), and a
  delivery thread: desktop notification + sound (`notify-rust`), webhook (JSON or text
  body; logs only the host), Telegram (bot token in the OS keychain via `keyring`,
  service `opentern`; chat id in settings `telegram_chat_id`). Nothing is sent when
  `OPENTERN_HEADLESS` is set.
- UI: Create/Edit alert dialog (Alt+A, top-bar Alert, the widget's +), Alerts widget
  (state, pause/restart, edit, delete on hover), Alerts log tab (clear log).
- **Gotcha:** egui's `Event::Text` carries no modifiers; type-to-search must check
  `i.modifiers` or Alt+A/Alt+T also type into symbol search. Tests must send
  `Event::ModifiersChanged` to model held keys.
- Email (Oct 2026): `lettre` over SMTP with rustls (STARTTLS, or TLS on 465);
  password in the keychain (`email-smtp-password`), server/port/login/recipient
  in settings (`email_smtp_*`, `email_to`); "Send a test email" in the dialog.
  Never tested against a real mail server.
- Not yet: `opentern --daemon` (alerts while the app is closed), alerts on symbols'
  indicators off-chart, sound files.

### Phase 9 (done): providers, calendars, dividends
- New adapters in `opentern-data`, one file each, all on the `Provider` trait:
  `bybit.rs`, `okx.rs`, `coinbase.rs`, `kraken.rs` (public data; **verified live**:
  history, paging, UTC-midnight days, streaming through the hub, symbol lists),
  `alpaca.rs` (US stocks, IEX/SIP trades → bars), `oanda.rs` (forex, HTTP pricing
  stream, UTC day alignment), `upstox.rs` (NSE/BSE, v3 candles, instrument list
  download for keys, quotes polled 1/s), `ibkr.rs` (Client Portal gateway, snapshots
  polled 1/s). **The four keyed ones are written from their documented APIs and tested
  on recorded replies only; nobody has run them against a real account yet.**
- Shared: `builder.rs` (`BarBuilder`: trades/prices → bars on the resampler's grid),
  `provider::{page_back, page_forward, ws_candles, get_json_with, split_pair, …}`,
  `creds.rs` (`Credentials`, `Debug` redacts). Routing in `hub::provider_for` by
  exchange prefix: `BYBIT:`, `OKX:`, `COINBASE:`, `KRAKEN:`, `OANDA:`, `IBKR:`; US
  stocks → Alpaca and NSE/BSE → Upstox when their keys are set, else Yahoo. Changing
  credentials restarts open chart sessions. Quotes for non-Yahoo, non-Binance symbols
  come from two daily bars every 30 s. Search loads Bybit/OKX/Coinbase/Kraken lists
  (weekly), ties prefer Binance, Coinbase, Bybit, OKX, Kraken; `EXCHANGE:` narrows;
  `OANDA:X` / `IBKR:X` are offered as typed.
- **OKX gotcha:** `www.okx.com` is unreachable from some networks (it is from this
  machine); the adapter falls back to `app.okx.com` / `my.okx.com` and remembers.
- Secrets: `opentern_core::secrets` (keyring, service `opentern`); Settings → Data
  providers (☰ menu) saves keys there and non-secret options (`alpaca_sip`,
  `oanda_account`, `oanda_live`, `ibkr_url`) in settings; keys load on a background
  thread at start.
- `opentern_core::calendar`: NYSE holidays and early closes by rule, NSE 2025 holidays
  as announced (**add 2026's when the NSE publishes them**), forex Sun 17:00–Fri 17:00 NY,
  crypto always open. The legend shows a market open/closed dot with the next open.
- Corporate actions: `yahoo::Corporate` (dividends, splits, from Yahoo for stocks and
  funds whatever the price source) → `Event::Corporate` → D/S markers at the bottom of
  the price pane (hover for details) and the ADJ toggle (dividend back-adjustment;
  Yahoo's prices are already split-adjusted). `ChartState.adj` is saved.
- Not yet: Dhan/Angel One/Zerodha, Upstox's protobuf websocket, IBKR TWS socket API,
  extended-hours shading, earnings markers.

### Phase 9b (done, formula part): built-ins wave 2
- `series/src/builtins/wave2.rs`: 26 more TradingView built-ins (Chaikin Osc, Chande
  Kroll Stop, Chop Zone, Connors RSI, Klinger, MA Ribbon, Net Volume, Pivots HL, RCI,
  RCI Ribbon, RVGI, Relative Volatility Index, SMI Ergodic ×2, Standard Error (+ Bands),
  SMI, TWAP, Trend Strength Index, Volatility Stop, Williams Fractals, Woodies CCI, ADR,
  24-hour Volume, Moon Phases, Trading Sessions). 88 built-ins in all.
- 22 Pine twins added to the corpus: **85 corpus scripts match their Rust built-ins bar
  for bar** (incl. colour indices where colour carries meaning).
- Found and fixed: a plot coloured by a variable (`color = zoneColor`) didn't get the
  palette of that variable's colours (`Cx::palettes`).
- What's left of TradingView's list and why is at the end of `docs/indicators.md`.

### Phase 10 (done): Pine v2
- Collections and types: `array.*`, `map.*`, `matrix.*`, `type` (UDTs, `copy()`),
  `method`, `for [i, x] in`, `enum` + `input.enum` (a member's value is its title, which
  is what `str.tostring` and the input dropdown show). Method calls bind named arguments
  at run time against whichever method the receiver picks (`order_args` in `run.rs`).
- Drawing objects: `label`/`line`/`box`/`table`/`linefill`/`polyline` with their
  setters/getters/`copy`/`delete`/`all`, `chart.point.*` and the point overloads of
  `*.new`; `max_labels_count` etc. drop the oldest. Types live in
  `opentern_series::drawings`; any `Study` can return them via `Study::drawings()` and
  `Instance::drawings` holds the forming bar's set. The chart paints them above the
  study's plots (`app/src/chart/scripted.rs`: bubbles and marker styles, extend, arrows,
  dashed/dotted, tables at pane corners with merges, curved/closed polylines).
- `request.security` on **this chart's symbol, same or higher timeframe**
  (`natives/security.rs`): HTF bars are built from chart bars; each finished HTF bar runs
  the expression once on the call site's frame (so `ta.*` and `x[1]` step per HTF bar);
  `lookahead_off` shows the last finished HTF value (the closing chart bar shows its own
  HTF bar), the forming bar repaints; `lookahead_on` = the HTF bar so far, which equals
  TradingView for the `expr[1]` idiom; `gaps_on` = na except where a value arrives.
  Variables used inside are recomputed on the HTF from their definition (`Cx::defs`,
  inlined); ones changed with `:=`/`var` get a plain error. Not yet: other symbols, lower
  timeframes, `security_lower_tf` (each gives a clear error). Known gap: on stocks, a
  "D" bar from intraday finishes one chart bar late (periods are calendar days).
- Libraries: `library("Name")` scripts in the scripts folder are found by title
  (`opentern_pine::library_title`, passed in `Env::libraries`); `import user/Name/1 as
  x` compiles their functions, methods, types and enums under `x.`; the env key hashes
  the libraries so importers recompile when one is saved. Library-level variables aren't
  imported.
- Built-ins that draw (`series/src/builtins/drawn.rs`): Zig Zag, Auto Fib Retracement,
  Pivot Points Standard (six types), Linear Regression Channel. 92 built-ins in all.
- App: a Pine script that fails its very first compile gets a placeholder legend row
  showing the error (it used to be invisible); fixing the file swaps in the real study.
  `OPENTERN_STILL=add:<id>` adds a built-in at launch (for screenshots).
- Found and fixed: `export type`/`export enum` didn't parse; named method arguments were
  treated as positional.

### Phase 11 (done): power features
- **Chart types** (`chart/transform.rs`, `draw_series`): bars, candles, hollow,
  columns, line, line with markers, step, area, baseline (level % of pane height),
  high-low, Heikin Ashi, Renko, Line Break, Kagi, Point & figure. HA, Renko, Line
  Break, Kagi and P&F **change the bars** (studies run on them, as on TradingView):
  `Chart::display()` = raw → ADJ → transform; box size is ATR(14) by default or fixed
  (Chart settings → Symbol). Bricks formed on one bar get times 1 ms apart so the
  time↔index maps stay strictly increasing. HA ticks update incrementally; the
  others rebuild everything on each tick. Not done: range bars (need ticks).
- **Multi-chart** (`multi.rs`): 15 grids for 1–8 charts; sync symbol, interval,
  crosshair (default on), time, date range. **The active chart lives in
  `App::chart`; the others in `App::slots`, swapped into place by `with_slot`**
  (panic-safe), so every "the chart" code path works per chart. Each slot has its own
  `ChartUi`, scripts and strategy runs; inactive charts poll scripts, strategies and
  paper lines each frame. Pressing a cell activates it. Layout JSON is now
  `LayoutState { grid, sync, active, charts }` (old single-chart JSON still loads).
  Session ids come from `chart::new_session_id()` (global). Drawings saved on one
  chart reload on other charts with the same symbol. Alerts still evaluate against
  the active chart's studies only.
- **Compare/overlay** (`Chart::compares`): Compare puts the scale in percent and
  scales each symbol from the first visible bar; Overlay draws on its own fitted
  scale. As-of join on time. Legend rows with hide/remove. Saved in the layout.
- **Bar replay** (`chart/replay.rs`): Replay → click a bar → play/pause, step,
  speed (10 s … 0.03 s per bar), jump to real time. Live data goes to the hidden
  bars meanwhile; studies and strategies only see replayed bars; no history paging
  while replaying.
- **Built-ins drawn by the chart** (`series/builtins/charted.rs` metas with no
  plots, painted in `chart/profile.rs`; numbers in `series/profile.rs`): VPVR,
  Session VP, Visible Average Price, Seasonality (own pane, month axis). Drawing
  tools: Anchored VWAP, Fixed Range VP, Anchored VP. 96 built-ins in all.
- **Screener tab** (`screener.rs`): universe = a watchlist or a preset (crypto
  majors, US and India large caps); columns price, change, 1W/1M perf, volume, rel
  volume, RSI, vs SMA 50/200, ATR %, plus an optional script column (any built-in
  plot, or a Pine file compiled per symbol) computed on rayon. Filters above/below.
  Uses the new hub **`Request::Fetch`** (one-off bars, cache first, 4 worker
  threads; also meant for portfolio backtests and `request.security` on other
  symbols). Settings saved under the `screener` key.
- **Snapshot** (camera, Alt+S): download PNG to `<workspace>/snapshots/` or copy.
- **Widgets** (`side.rs`, `notes.rs`): Hotlists (Binance 24h tickers; Yahoo
  predefined screeners for US stocks), News (Yahoo search for stocks; CoinDesk's
  public RSS for crypto, since Yahoo has no crypto headlines), Calendar (NYSE/NSE
  holidays and early closes, the symbol's dividends and splits), Notifications
  (alerts, snapshots, updates; dot on the rail when unread). Hub requests
  `Hotlist` and `News` (`data/src/feeds.rs`).
- Stills: `OPENTERN_STILL` also takes `kind:<chart type>`, `grid:<Grid>`,
  `compare:<ticker>`, `tf:<interval>`, `replay`, `screen`, `widget:<name>`.

### Phase 12 (done): quant tools
- **Strategy Lab** bottom tab (`app/src/lab.rs`) on the strategy picked in the
  Strategy Report: **Optimize** (tick inputs, from/to/by; maximize net profit, PF,
  Sharpe, Sortino, % profitable, smallest DD, profit÷DD or CAGR; every combination
  up to 50,000, Random, or Bayesian; a fewest-trades floor; list or heatmap; double-
  click a row or click a cell to put those inputs on the chart), **Walk-forward**
  (rolling or anchored in-sample/out-of-sample windows, stitched OOS equity,
  efficiency = OOS profit per bar ÷ IS profit per bar), **Monte Carlo** (shuffle or
  bootstrap the closed trades; percentiles, chance of loss/ruin, fan chart),
  **Portfolio** (the strategy on a watchlist or preset, capital split equally,
  combined equity on the union of bar times, per-symbol table; correlation matrix in
  the JSON). CSV and JSON export to `<workspace>/exports/`. Runs go to a `lab`
  thread that fans out on rayon, with progress and Stop.
- Reruns use the chart study's own factory (`Instance::with_factory` +
  `strategy::broker_of`), so Python strategies (out of process) can't be rerun:
  the tab says so. Portfolio recompiles per symbol (`syminfo.*`) via `lab::Build`.
  Walk-forward OOS runs start at the IS window's first bar (warm-up); a position
  carried over counts on OOS bars. Settings saved under the `lab` key.
- **Strategy settings** dialog (Report header → "Strategy settings"): capital,
  order size and type, pyramiding, commission, slippage, margin, fill on close.
  Saved per script file as overrides (`strategy_props:<file>`), applied with
  `opentern_pine::Script::with_props` (Pine) or merged into a Python strategy's
  props; Reset returns to the script's values. `Props::to_json`/`diff_json`.
- Stills: `OPENTERN_STILL=script:x.pine,lab:optimize|heatmap|walk|monte|portfolio`
  ticks the first two inputs and runs once bars arrive.
- Measured (demo, 1h, ~1,000 bars): 165-run grid of an MA cross in 0.1 s; 45
  walk-forward windows × 165 runs in 0.6 s.

### Phase 13 (done): Luau, JavaScript, TypeScript
- `.luau`/`.lua`, `.js`, `.ts` in the scripts folder; the user API is in
  `docs/scripting.md`. One host (`opentern_script::host`) owns the meaning; a
  language only converts values and calls host functions by name, so all three
  agree bar for bar (tested: MACD in Luau = JS = TS; RSI = the Rust built-in).
- Execution model: top level once (declarations), then `on_bar(bar)` / `onBar(bar)`
  per bar; `ta.*` and `plot` keyed by call order (as Pine call sites); persistent
  values in a `state` table that the host snapshots after each closed bar (so the
  forming bar rolls back); plots found by a dry run on made-up bars at compile.
- Sandbox: Luau has no I/O by design; QuickJS gets no modules; both have a
  256 MB cap and a 1 s budget per bar (interrupt → plain error with the line).
  One VM per study (optimizer runs stay parallel); the forming bar's copy shares
  its study's VM and restores `state` by version number.
- App: `Lang::{Luau, Js, Ts}` go through the Pine host path (`pine_host.rs`
  `Compiled` enum, study ids `luau:`/`js:`/`ts:`), so recompile on save,
  errors in the legend, Strategy settings overrides, the Strategy Lab (incl.
  portfolio via `scripting::other_builder`), `alert()` alerts and the editor
  (templates, highlighting, New script menu) all work. No drawing objects or
  `request.security` in these languages yet.
- **Perf gotcha:** mlua with `send` locks on every API call, so per-field table
  sets are slow (~0.3 µs each). The bar goes in as 14 arguments to a Luau-side
  wrapper that fills one reused table and reports whether `state` is empty;
  `ta.*`/`plot` have typed fast paths (`HostState::ta_fast`, `plot_fast`).
  Measured on this throttled laptop: 100k bars with RSI + MACD + 2 plots ≈ 0.55 s
  Luau, 0.7 s JS (empty `on_bar` ≈ 2.5 µs/bar). `cargo test --release -p
  opentern-script --test timing -- --ignored --nocapture`.
- Licences: mlua/Luau, rquickjs/QuickJS (MIT), swc (Apache-2.0): fine for the
  MIT app; list them in `THIRD-PARTY-LICENSES.txt` at Phase 14.

### Feeds: other symbols and timeframes in Pine (Oct 2026)
- `opentern_pine::Env::feeds` (`Feeds` = shared map `"SYMBOL|tf"` → bars). A
  `request.security` on another symbol, or below the chart's timeframe, or
  `security_lower_tf`, reads its feed; a missing feed gives `na` and is listed
  in `PineStudy::wanted()`. `app/src/feeds.rs` fetches wanted feeds
  (`Request::Fetch`, seq ≥ `1 << 42`; bars to cover the chart's span), stores
  them in `App::feeds` (one store for all charts), reruns every chart's Pine
  studies, and refreshes feeds in use every minute. Symbols resolve like
  formulas (`expr::resolve`). Inputs can pick symbol and timeframe.
- A feed bar counts as finished when its period has ended by the chart bar's
  close; lookahead/gaps/forming-bar repaint follow the same rules as HTF bars.
- `run::period_end` + `calendar::last_close_in`: daily-and-longer periods of
  markets with set hours end at the last session close (fixes stocks' "D"
  finishing a bar late). Chart symbol's market comes from `Env` prefix + kind.

### How data flows
`App` sends `Request::Open { chart, symbol, tf }` → the hub starts a session thread: cache →
publish → network tail → publish → stream (Binance WS / Yahoo poll / demo / formula legs),
reconnecting with backoff; `More` loads older history (cache first). Events come back over
a channel and wake the UI. **Each (re)open gets a new chart id** (`App::open_chart`);
events for old ids are dropped. Studies update incrementally: `Instance::update` steps
closed bars once and recomputes the forming bar on a clone. `Chart::data_version` bumps on
every data change (scripts re-run on it).

### Gotchas so far
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
- Formulas: subtraction needs spaces (`AAPL - MSFT`) because `-` can be part of a
  ticker (`BRK-B`); bare tickers resolve via known symbols, crypto quote suffixes →
  Binance, else `YAHOO:`.

### Open questions for the maintainer / to verify
- **EMA seeding** (SMA of the first n values, as implemented) is the first thing to
  confirm with a TradingView export; see `docs/ta-semantics.md`.
- Default inputs/colours under "Defaults to double-check" in `docs/indicators.md`.
- Strategy Report formulas marked "to verify" in `docs/metrics.md` (Sharpe/Sortino
  period and stdev convention, buy & hold basis, account size required, margin).
- Zig Zag, Auto Fib and Pivot Points Standard follow TradingView's behaviour as
  remembered (their open questions are in `docs/indicators.md`); check them against
  exports.

### Dev loop
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
`crates/app/src/drive.rs` drives the real app headlessly (synthetic mouse/keys, demo
data): use it for interaction tests. `OPENTERN_STILL=indicators|symbol|chart-settings|
study:<id>|save-template` opens a dialog at launch for screenshots; `script:<file>` puts a
workspace script on the chart and `add:<id>` a built-in (comma-separate several). `OPENTERN_PYTHON` picks the interpreter.
```bash
cargo test --release -p opentern-pine --test conformance -- --ignored --nocapture --test-threads=1  # Pine timing
```

### Measured
- 62 built-ins over 1,000,000 bars: 3.7 s total (~60 ms each), release build.
- Live: Binance BTCUSDT 1m and 45m stream; Yahoo daily/15m; BTCUSDT/ETHUSDT 1m streams
  (~33.2); AAPL/SPY daily joins 8,482 bars; scroll-back loads 5,000 more Binance bars.
- Pine over 100,000 bars (release, this laptop throttled to ~1.1 GHz under its power
  profile, so ~4× slower than full clocks): Supertrend 72 ms, BB 105 ms, RSI/MACD/DMI/
  Ichimoku 155–175 ms. About 77 ns per bar fixed cost; ~40 ns per simple statement.
- 200+ tests pass (live and timing ones ignored by default); clippy `-D warnings` clean.

### Not done yet
Phases 12–14 and the loose ends listed under "Resume here". No placeholders left in
the shell.

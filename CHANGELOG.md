# Changelog

## 0.1.4 (2026-10-22)

Alt W adds the chart's symbol to your watchlist, updates show a progress bar,
and the agent panel finds Claude Code and Codex on Windows however they were
installed.

### New

- **Alt W adds the chart's symbol to the watchlist** that's open, at the end
  of its top section. Drag it from there to wherever you want it.

### Changed

- **Updates show a progress bar** with the percentage in Settings → About.
- **Deleting a watchlist takes one click.**
- **"Install Claude Code" and "Install Codex" link to their install pages**
  when the agent panel can't find them.

### Fixed

- **The agent panel works with npm installs on Windows.** Opentern runs the
  Claude Code or Codex program inside the npm package, and a native install
  wins wherever it is on the PATH.
- **The watchlist menu stays inside the watchlist panel**, and the indicator
  settings' Defaults menu no longer stretches across the window.

## 0.1.3 (2026-10-22)

Every drawing tool gets a keyboard shortcut, watchlists work like
TradingView's, and Hotlists cover Indian stocks.

### New

- **A shortcut for every drawing tool.** TradingView's (Alt T, H, V, C, F and
  Alt Shift R) plus Alt Shift and a letter for the rest, and Alt L, Alt P and
  Alt U for log, percent and auto scale. Settings → Keyboard shortcuts lists
  them all; the keyboard button beside the theme switch opens it.
- **Watchlists like TradingView's.** Sections that fold, rows you drag to
  reorder, a circle per list to switch, and naming a new list right away.
- **Indian stocks in Hotlists.** The NIFTY 50's gainers and losers.

### Changed

- **The update button shows its progress** as a ring that fills while it
  downloads, then a restart icon.
- **Dropdowns fit their choices** instead of cutting them short.
- **About** says what Opentern is.
- **A market analysis terminal.** Opentern now describes itself that way
  everywhere: it's for analysis and paper trading, and never places real orders.
- **One install line for Linux and macOS.** `install.sh` now installs the Mac app
  too.

### Fixed

- **Demo mode shows symbol logos** (the ones bundled in the app; it still never
  goes online).

## 0.1.2 (2026-10-21)

Opentern now runs on macOS and Windows, and the agent panel works with Codex
as well as Claude.

### New

- **macOS and Windows builds.** A universal app for Apple Silicon and Intel
  Macs, and an installer for x64 and ARM64 PCs. Linux gets ARM64 builds too.
- **Pick your agent.** The panel beside the chart works with Claude Code or
  OpenAI's Codex CLI, whichever you have installed. Switch between them from
  the top of the panel at any time; each keeps its own chats.
- **Starred indicator templates.** Star a template and it sits in the top bar,
  one click away.
- **License notices.** Every download includes the licenses of what Opentern
  is built from, and `opentern --licenses` prints them.

### Changed

- **A calmer interface.** Fewer captions and explanations, softer text boxes,
  focus rings and menus, and the drawing toolbar slides in and out.
- **Settings is a gear** at the right end of the top bar.

### Fixed

- **Symbol search scrolls** with the mouse wheel again.
- **Alt+V** in the agent panel pastes an image without also typing a "v".
- **Safer updates.** The libraries that unpack downloads and check secure
  connections are updated to fix known security issues.

## 0.1.1 (2026-10-21)

Strategy results now match TradingView, checked against a TradingView Strategy
Report on BINANCE:BTCUSDT 1D (2017 to 2026, 40 trades): net profit, max
drawdown, profit factor and every trade size are the same, to the cent.

### Fixed

- **Strategies act on closed bars only,** as on TradingView.
- **"% of equity" orders** leave room for the commission and round to the
  exchange's lot size.
- **Pine v6 margin:** entries the account can't pay for are skipped, and
  losing positions get margin calls.
- **Max drawdown** is measured from your highest closed-trade equity.

## 0.1.0 (2026-10-16)

The first release, for Linux x86-64: TradingView-style charts and drawings,
TradingView's built-in indicators, Pine Script®-compatible scripting (plus
Python, Luau, JavaScript and TypeScript), backtests with TradingView's Strategy
Report, a strategy lab, alerts, paper trading, a screener, and a Claude panel.

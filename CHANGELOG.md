# Changelog

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

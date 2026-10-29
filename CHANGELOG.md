# Changelog

## 0.2.3 (2026-10-30)

Build your own indices, see a symbol's seasonality, and choose which Claude
Code or Codex account the agent panel uses.

### New

- **Custom indices.** Make an index from any symbols in the new Indices tab
  (search, or add a whole watchlist). It's weighted by free-float market cap
  as NSE weights its indices, or by full market cap, or equally; starts at
  1000; and rebalances on 1 April and 1 October (or on demand) without the
  level jumping. Chart it like any symbol, with indicators and alerts.
- **An index board.** Each index shows its returns, P/E, EV/EBITDA, revenue
  growth, margins, RSI and distance from its 50- and 200-day averages, then
  every member's, as a colour-coded table or a treemap.
- **Compare indices.** Every index's numbers side by side, as a table or a
  map.
- **Seasonality.** A new bottom tab shows the chart symbol's return for
  every month of every year, with a slider to pick the years and a count of
  rises and falls per month.
- **Agent accounts.** With more than one Claude Code or Codex account on
  the computer, pick which one the agent panel uses from its menu. Each chat
  stays on the account it started with.

### Changed

- **Maximizing the bottom panel gives it the whole window** under the top
  bar, chart included.

### Fixed

- **The agent's message box stays in view** when a long message wraps; past
  eight lines it scrolls.
- **Strategy Lab column titles no longer overlap.**
- **On Windows, links open in the browser** (news links used to open the
  Documents folder).
- **Formulas accept tickers with "&"**, such as NSE:M&M.

## 0.2.2 (2026-10-28)

News covers Indian stocks and follows the coin you're viewing, the Calendar
is easier to read, and the watchlist's daily change for indices is right.

### Changed

- **News draws on many outlets at once.** Every symbol gets a search of the
  past week's news. Indian stocks and indices add the Economic Times and
  Mint. Crypto adds CoinDesk, Cointelegraph, Decrypt and The Block, keeping
  only the headlines that name the coin. Repeats are removed, the newest come
  first, and each headline shows its outlet.
- **A tidier Calendar.** Holidays show as date tiles with the weekday, how
  far off they are and a Closed or early-close tag. Dividends and splits are
  grouped by year, with amounts in the stock's currency (₹25.00, not
  25.0000).

### Fixed

- **The watchlist's daily change for NIFTY, SENSEX and BANKNIFTY matches the
  chart.** It was measured from the close two sessions back, so BANKNIFTY
  could show red on a day it rose.

## 0.2.1 (2026-10-28)

Crypto charts load again on networks that block Binance, and hotlists say
what period their change covers.

### Changed

- **Hotlists show what the change covers:** "Chg% 24h" for crypto (a rolling
  24 hours) and "Chg% today" for stocks (since the previous close).

### Fixed

- **Crypto charts and watchlist prices stay up to date on networks that
  block Binance** (such as Jio in India). Opentern now reads Binance's public
  market data from Binance's own mirror first and falls back to the main
  servers, so charts no longer sit on old cached bars with no prices in the
  watchlist. The order book and crypto hotlists use it too.
- **A blocked data service gives up sooner.** Live connections stop trying
  after 8 seconds instead of hanging for minutes before reconnecting.

## 0.2.0 (2026-10-27)

The agent panel's Claude and Codex can now use the whole app, the way you
would with clicks. Also: Windows shows Opentern's icon and symbol logos, and
a What's new card after each update.

### New

- **The agent can drive the app.** Beyond reading the chart and writing
  scripts, it can change the chart type, zoom to dates, split the window into
  several charts, compare symbols, change an indicator's inputs, draw (trend
  lines, boxes, Fibonacci, long/short positions, labels and more), run bar
  replay, manage watchlists, run the screener, change a strategy's settings,
  run the Strategy Lab (optimize, walk-forward, Monte Carlo, portfolio) and
  apply its results, paper trade, set alerts and open bottom-panel tabs.
- **Learn from real examples.** Ask the agent to teach you a pattern: it
  finds one in the chart's history (loading older bars if it has to), marks
  it, and steps through it in bar replay.
- **The agent can look at the chart.** It takes a picture of what you see to
  check its drawings or read a pattern.
- **What's new after updates.** The first launch of a new version shows what
  changed; Settings → About opens it again.
- **Labels on drawings.** Rectangles and lines can carry a short label.

### Fixed

- **Windows shows Opentern's icon** in Start, search, the taskbar and
  Settings → Apps (the program had none of its own).
- **Symbol logos show on Windows.** The bundled logos were read wrongly on
  Windows builds.

## 0.1.5 (2026-10-27)

A welcome for new installs, Opentern's own green themes, and an agent panel
that asks you to sign in before you type.

### New

- **A welcome on first launch.** Pick the markets you trade (crypto, US
  stocks, Indian stocks, forex, commodities) and each becomes a watchlist of
  its top seven. Then pick your first chart from cards with live prices and
  sparklines, and it opens into the app.
- **Sign in from the agent panel.** If Claude Code or Codex is installed but
  not signed in, the panel says so and signs you in through your browser,
  instead of failing after your first message.

### Changed

- **Tern Dark is the new dark theme**: Opentern's slate with the logo's green
  for accents and selection. It replaces Opentern Dark, and a saved Opentern
  Dark opens as Tern Dark.
- **Opentern Light is greener**: white and pale sage with the logo's deep
  green.
- **The agent panel shows the model it will use** (from Claude Code's or
  Codex's own settings) instead of "Default".
- **The agent's message box is quieter**: no suggested prompt, a softer
  cursor, and "What can I help you with?" until you type.
- **The website** takes the new colours.

### Fixed

- **Watchlist and chart prices agree.** Whichever price arrived last shows in
  both.
- **Dragging the bottom panel up no longer crushes the chart**; it stops
  where the chart still has room, and shrinks with the window.
- **Codex's model list loads on a new workspace** (it never did before a
  script was saved).
- **The website's logo is centred on phones.**

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

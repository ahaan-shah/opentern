<div align="center">

<img src="packaging/linux/opentern.svg" width="128" alt="Opentern icon">

# Opentern

**A fast, local-first market analysis terminal.**

TradingView-style charts, TradingView's built-in indicators, Pine Script®-compatible
scripting (plus Python, Luau, JavaScript and TypeScript), backtests with
TradingView's Strategy Report, alerts, paper trading and a coding agent beside
the chart. It's for analysis: it never places real orders. A native Rust app for Linux, macOS and Windows, with no account and no
telemetry. Your data stays on your computer.

[Website](https://ahaan-shah.github.io/opentern/) · [Install](#install) · [Features](#features) · [Scripting](#scripting) · [Keyboard](#keyboard) · [Your data](#your-data) · [Building](#building-from-source)

<img src="site/media/home-dark.webp" alt="Opentern's chart with Bollinger Bands, RSI and a strategy, with a row of workspaces" width="900">

</div>

## Features

**Laid out like TradingView.** Drawing tools on the left, symbol and interval in
the top bar, widgets on the right, and the Script Editor, Strategy Report and
Paper Trading along the bottom. Traders already know where everything is. It just
opens faster and asks your computer for less.

**Charts.**
- Candles, hollow candles, bars, line, line with markers, step, area, baseline,
  columns, high-low, Heikin Ashi, Renko, Line Break, Kagi and Point & Figure.
- Any interval, including custom ones like 7m, 45m or 2h, with scroll-back history.
- Log, percent and locked scales. Compare or overlay other symbols, or chart a
  formula like `BTCUSDT/ETHUSDT`.
- Up to 8 charts in one layout, with symbol, interval, crosshair and time kept in sync.
- Bar replay: pick a date and play the market forward bar by bar.
- 27 drawing tools in TradingView's groups: trend lines, Fib, pitchfork, channels,
  long/short positions, measures, anchored VWAP and volume profiles. Drawings are
  saved per symbol.
- Logos for coins and companies, as on TradingView.

**102 of TradingView's built-in indicators**, written in Rust from Pine-exact `ta.*`
primitives, so they match TradingView's numbers. They keep TradingView's default
inputs, styles, colours and levels, with settings dialogs, saved defaults and
indicator templates (star one to keep it in the top bar). They include the Volume
Profile family, Ichimoku, SuperTrend, Zig Zag, Auto Fib and Pivot Points.

<img src="site/media/report.webp" alt="The Strategy Report for a moving-average cross" width="900">

**Backtests that match TradingView.** Checked against a TradingView Strategy
Report on BTCUSDT 1D over 40 trades: net profit, max drawdown, profit factor and
every trade size agree to the cent.
- A broker emulator with TradingView's fill rules: next-open fills, the intrabar
  path for stops and limits, pyramiding, brackets, margin, commission and slippage.
- The Strategy Report has TradingView's tabs and metric names: Overview,
  Performance, Trades analysis, Risk/performance ratios, List of trades and
  Properties. The List of trades exports to CSV.
- Entry and exit arrows on the chart with "Long +1" labels, as TradingView draws them.
- The **Strategy Lab**: a grid, random or Bayesian optimizer with a heatmap,
  walk-forward analysis, Monte Carlo and portfolio backtests across a watchlist.

<img src="site/media/agent.webp" alt="Claude beside the chart, after building a strategy" width="900">

**An agent beside the chart.** If you use [Claude Code](https://claude.com/claude-code)
or OpenAI's [Codex CLI](https://github.com/openai/codex), ask it to write a
strategy, explain an indicator or read your backtest. It sees the chart, writes
scripts in your scripts folder, checks they compile, puts them on the chart and
reads the Strategy Report. Pick your agent the first time you open the panel and
switch any time; each keeps its own chats. With more than one Claude Code or
Codex account on your computer, pick which one it uses. Opentern starts the CLI already on your
computer, so it uses your own sign-in and plan. Opentern has no key of its own,
and nothing runs until you send a message.

<table>
<tr>
<td><img src="site/media/editor.webp" alt="The Script Editor"></td>
<td><img src="site/media/lab.webp" alt="The Strategy Lab's optimizer heatmap"></td>
</tr>
<tr>
<td><img src="site/media/multi.webp" alt="Four charts in one layout"></td>
<td><img src="site/media/screener.webp" alt="The screener"></td>
</tr>
</table>

<img src="site/media/indices.webp" alt="A custom Nifty Auto index: its own numbers, then every member's" width="900">

**Your own indices.** Pick any stocks, or a whole watchlist, and Opentern
builds an index the way NSE builds its own: weighted by free-float market cap
(or full market cap, or equally), starting at 1000, and rebalanced on 1 April
and 1 October without the level jumping. Chart it like any symbol, with
indicators and alerts. Its board shows the index's returns, P/E, EV/EBITDA,
revenue growth, margins, returns from a month to a year, RSI and moving averages, then every member's, as a
colour-coded table or a treemap. **Compare** puts all your indices side by side.

<img src="site/media/overview.webp" alt="The market overview: the S&P 500 as a heatmap, grouped by sector" width="900">

**Market overview.** One click beside the layout button swaps the charts for a
heatmap of a whole market: the S&P 500, the NIFTY 500 or the 150 most traded coins.
Tiles are sized by market cap (coins by 24-hour volume), grouped by sector and coloured by the day's
change, or the week's, month's, quarter's, six months', year to date or year's. Click a
sector to zoom in, click a tile to open its chart. Your watchlists and the
agent stay beside it.

**Seasonality.** See how the chart's symbol did in every month of every year,
with a slider to pick the years and a count of rises and falls per month.

<img src="site/media/seasonality.webp" alt="Twelve years of monthly returns for RELIANCE" width="900">

**Paper trading.** Several simulated accounts with market, limit, stop,
stop-limit, trailing and bracket orders. Trade from the chart by dragging an
order's tag, or from the depth-of-market ladder.

**Alerts.** Alerts can watch a price, a drawn line, an indicator plot or a
script's `alert()`. They reach you as a desktop notification with sound, a
webhook, Telegram or email, and can keep running when Opentern is closed, even
after a restart.

**Markets.**
- **Crypto:** Binance, Bybit, OKX, Coinbase and Kraken. Free, live and with deep history.
  Perpetual futures on Binance, Bybit and OKX too, with their funding rates.
- **Indices worldwide:** the S&P 500, Nasdaq, Dow, Nikkei 225, FTSE 100, DAX, CAC 40,
  Euro Stoxx 50, Hang Seng, KOSPI, ASX 200, TSX, Ibovespa and more, plus US Treasury
  yields, VIX, India VIX, the MOVE index and the dollar index (Yahoo, free).
- **The Fed funds rate:** the US effective federal funds rate (EFFR), daily since
  July 2000, with the Fed's target range as a band behind it. From the Federal
  Reserve Bank of New York, free; search "fed funds".
- **US Markets (NYSE, Nasdaq): stocks, ETFs, plus forex and commodities:** Yahoo by default, delayed.
  Add your own Alpaca, OANDA or Interactive Brokers keys for live data.
- **Indian Markets (NSE, BSE):** Yahoo by default, or Upstox with your own keys.

**Symbol search** finds a symbol however you type it: by ticker, by company or
coin name ("bitcoin"), with words in any order ("NSE VIX"), and with a typo or two.
Each result shows its price and today's change, and a star adds it to your
watchlist without leaving the search.

Exchange calendars for fifteen exchanges (the US, India, London, Frankfurt, Paris,
Zurich, Tokyo, Hong Kong, Shanghai, Seoul, Taipei, Singapore, Sydney, Toronto and
São Paulo), dividends and splits are built in. Everything is free until you
choose to add a paid provider.

**Everything else.**
- 16 themes, 8 light and 8 dark, that crossfade when you switch, and 9 interface fonts.
- In English, German, Spanish, French, Portuguese, Swedish, Russian, Hindi, Japanese and Simplified Chinese.
- A screener over any watchlist, with 1-week to 1-year performance columns and an
  optional script column.
- Watchlists with sections, laid out like TradingView's, plus hotlists (crypto, US
  and Indian stocks), news, a calendar, depth of market and an object tree in the
  right-hand widgets.
- Named layouts, chart templates and indicator templates, all of which you can
  export and import as JSON.
- Opentern updates itself: a quiet **Update available** appears on the right,
  and nothing happens until you click it.

## Install

### Linux & macOS (one line)

```bash
curl -fsSL https://raw.githubusercontent.com/ahaan-shah/opentern/main/install.sh | sh
```

What the script does on each platform:
- **Linux:** installs `opentern` into `~/.local/bin`, and adds a desktop entry and
  icon so Opentern shows up in your app launcher. x86-64 and ARM64.
- **macOS:** installs `Opentern.app` (universal, Apple Silicon + Intel) into `/Applications`.

The script checks every download against the release's `SHA256SUMS`. To pin a
version, set `OPENTERN_VERSION=0.3.8`.

The app isn't notarized yet. If macOS refuses to open it the first time,
right-click the app and choose **Open**, or run
`xattr -dr com.apple.quarantine /Applications/Opentern.app`.

### Windows (one line)

In PowerShell:

```powershell
irm https://raw.githubusercontent.com/ahaan-shah/opentern/main/install.ps1 | iex
```

This installs Opentern just for you (no admin prompt) into
`%LOCALAPPDATA%\Programs\Opentern` and adds it to the Start menu, on x64 and
ARM64 PCs. To pin a version, run `$env:OPENTERN_VERSION = "0.2.3"` first.

Prefer a regular installer? Download
[`Opentern-windows-setup.exe`](https://github.com/ahaan-shah/opentern/releases/latest/download/Opentern-windows-setup.exe)
and run it. It isn't code-signed yet, so SmartScreen may say "Windows protected
your PC". Click **More info**, then **Run anyway**.

### Other downloads

The [latest release](https://github.com/ahaan-shah/opentern/releases/latest)
also has portable archives (`.tar.gz` for Linux x86-64 and ARM64, `.zip` for
Windows x64 and ARM64) and the macOS disk image.

**Uninstall:** on Linux and macOS, run the install line with
`sh -s -- --uninstall` in place of `sh`. On Windows, use **Settings → Apps**.
Your workspace is kept either way.

## Scripting

Every language runs on the same bar-by-bar engine with Pine's semantics: history
references like `close[1]`, persistent state, and a forming bar that recalculates
on every tick. So a script plots the same way whatever it's written in.

| Language | How it runs |
|---|---|
| **Pine Script® v5/v6** | A native Rust engine. Covers `ta.*`, `strategy.*`, `request.security`, drawings, arrays/maps/matrices, types, methods and libraries |
| **Python** | Your own interpreter, in a separate process, with the `opentern` SDK (pandas welcome) |
| **Luau** | Embedded and sandboxed, with instant startup |
| **JavaScript / TypeScript** | Embedded QuickJS, sandboxed |

Scripts live in your workspace's `scripts` folder. Write them in the built-in
editor or in your own: the chart reloads when you save. A script that fails
shows a plain error with the line number in its legend row, and never takes the
app down. See [docs/scripting.md](docs/scripting.md) for the Python, Luau and
JS API.

```pine
//@version=6
strategy("MA cross", overlay=true)
f = ta.ema(close, input.int(9, "Fast"))
s = ta.ema(close, input.int(21, "Slow"))
plot(f, "Fast", color.blue)
plot(s, "Slow", color.orange)
if ta.crossover(f, s)
    strategy.entry("Long", strategy.long)
if ta.crossunder(f, s)
    strategy.close("Long")
```

## Keyboard

The shortcuts are TradingView's, and every drawing tool has one. The full list
is in Settings → Keyboard shortcuts (the keyboard button beside the theme
switch).

| Keys | Does |
|---|---|
| Type anywhere | Symbol search |
| Type a number | Change the interval (`5`, `45`, `4h`, `D`) |
| `/` | Indicators |
| `Alt T` · `Alt H` · `Alt V` · `Alt C` · `Alt F` | Trend line · horizontal line · vertical line · cross line · Fib retracement |
| `Alt Shift R` | Rectangle (the other tools are `Alt Shift` and a letter) |
| `Alt L` · `Alt P` · `Alt U` | Log · percent · auto scale |
| `Alt A` | Create an alert |
| `Alt R` | Reset the chart view |
| `Alt S` | Save a snapshot |
| `Alt M` | Market overview |
| `Ctrl Alt H` | Hide all drawings |
| `Shift F` | Full screen |
| `Ctrl Z` / `Ctrl Y` | Undo / redo |
| `←` `→` | Step one bar |
| `Delete` · `Esc` | Delete the selected drawing · cancel |
| `Ctrl V` (agent panel) | Paste an image from the clipboard |
| `Shift Enter` (agent panel) | New line in a message |

On macOS, use `Cmd` instead of `Ctrl`.

## Your data

Everything stays on your computer. There's no account, no sign-in and no
telemetry. Opentern only goes online for the market data providers you use, a
check for a new release on GitHub (you can turn it off in Settings), symbol
logos (also optional), and the agent CLI you choose, when you send it a message.
The Fed funds rate comes from the New York Fed (`markets.newyorkfed.org`), only
while it's on a chart, in a watchlist or in an alert, a few times a day.
The market overview, while it's open, also fetches the S&P 500 and NIFTY 500
member lists (weekly).
API keys are kept in your system's keychain, never in a file.

| | Linux | macOS | Windows |
|---|---|---|---|
| Workspace | `~/.local/share/opentern/` | `~/Library/Application Support/dev.opentern.opentern/` | `%APPDATA%\opentern\opentern\data\` |

The workspace holds the database (settings, layouts, drawings, alerts, paper
accounts and the bar cache), your scripts and your agent chats. To move to
another computer, copy the folder.

Want to look around first? `opentern --demo` opens a workspace of its own with
made-up prices. It doesn't go online and doesn't touch your real data.

## Building from source

```bash
git clone https://github.com/ahaan-shah/opentern.git
cd opentern
cargo run --release
```

Rust 1.89+ is required. On Linux you also need the usual GUI libraries (on
Debian or Ubuntu: `pkg-config libdbus-1-dev libxkbcommon-dev libwayland-dev
libgl1-mesa-dev libgtk-3-dev`). For day-to-day work, `cargo run --profile fast
-- --demo` builds quicker.

### Opentern Dev (Linux)

`scripts/dev-install.sh` builds your checkout and installs it as **Opentern Dev**,
next to the released app: `~/.local/bin/opentern-dev` and an entry in your app
menu. It's a separate app: its own workspace (`~/.local/share/opentern-dev`), its
own saved API keys, and it never updates itself. Run the script again to replace
it with a newer build; `scripts/dev-install.sh --remove` takes it away (its
workspace stays). The install command above always installs the released app,
with its own updates.

Opentern Dev is built from your copy of the code. Usually that's the same as the
latest release; if `main` has changes that aren't released yet, you get them early.

## License

[MIT](LICENSE). Indicators ported from MPL-2.0 Pine scripts stay MPL-2.0 file
by file, with their authors credited in [docs/indicators.md](docs/indicators.md).
The licenses of everything Opentern includes are in
[THIRD-PARTY-LICENSES.txt](THIRD-PARTY-LICENSES.txt) (`opentern --licenses`).

**Scripts and agents run on your computer.** Python scripts run in your own
Python interpreter with the same access to your files, network and computer as
any program you start; Opentern doesn't sandbox them. The agent panel starts
Claude Code or Codex, which can read and write files, run commands and run
scripts on your computer, within the permissions you give it (Ask first or
Auto). Read a script before you run it, and only give an agent the access you'd
give any program. Opentern holds back Python scripts that arrive in an imported
workspace until you run them by hand, but it can't tell whether a script is
safe. As the [license](LICENSE) says, Opentern comes with no warranty, and you
use scripts, agents and their results at your own risk. Nothing in Opentern is
financial advice.

**The Fed funds rate** is the New York Fed's data, not ours: the MIT license
covers Opentern's code only. The Effective Federal Funds Rate is subject to the
Terms of Use posted at [newyorkfed.org](https://www.newyorkfed.org/privacy/termsofuse).
Source: Federal Reserve Bank of New York. The New York Fed is not responsible
for its publication in Opentern and does not endorse it; Opentern is not
affiliated with the New York Fed.

Pine Script® and TradingView® are trademarks of TradingView, Inc. Claude and the
Claude logo are trademarks of Anthropic. OpenAI, Codex and the OpenAI logo are
trademarks of OpenAI. Opentern is an independent project, not affiliated with or
endorsed by any of them.

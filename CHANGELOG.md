# Changelog

## Unreleased

### New
- **Pre-market and after-hours prices for US stocks, as on TradingView.** While they trade, the chart's legend shows a sun (pre-market) or moon (after hours) with the price and change on hover, and the price scale gets its own label and dotted line (Chart settings → Scales and lines). The market overview says when extended hours are on, and each tile's card shows the extended-hours price. Candles and the day's change stay on the regular session.

- **Says when prices stop updating.** If a live chart's prices haven't moved for 5 minutes while its market is open, the legend says "No new prices for 7 min". The watchlist dims such a price (hover says how long), and the market overview's header shows a warning.
- **Says when Yahoo is missing prices.** A BSE stock whose daily history Yahoo lacks (most of them) says so and points to its NSE listing. An Indian chart missing a recent special session (Muhurat trading) says that session isn't on it.
- **A daily backup of your workspace.** Opentern saves your watchlists, layouts, drawings, alerts, templates and paper accounts to `backups/` once a day and keeps the last week. Restore one from Settings → Export and import → From a file.
- **The price cache's size, and a button to clear it** (Settings → General → Price cache). It asks first and says what may not come back: sources keep only recent minute and hour bars, and special sessions recorded live are never downloadable.
- **Untested data providers are marked Beta.** Alpaca, OANDA, Upstox and Interactive Brokers haven't yet been tried with a real account.

### Changed
- **The market overview keeps up while markets trade.** Prices refresh every 30 seconds while the market is open (crypto always), instead of every 3 minutes.

### Fixed
- **US stocks' 1D change in the market overview is right before the open and after the close.** It compared the extended-hours price with the close two sessions back (NVDA showed −6.18% before the open on 12 Nov when it had closed +2.87%). It now shows the regular session, like the chart and watchlist.
- **No more flat extra bar at the close on intraday stock charts.** Opened outside market hours, a 15m or 1h chart ended with a flat, volume-less bar at the closing price (Yahoo's latest-price point). The last bar now ends at the official close instead, and those bars are removed from the cache once.
- **Weekly and monthly stock charts include today.** The current week (month) ended at yesterday, and today showed as a separate bar dated today (NVDA's weekly chart had a "week" starting on Thursday). Weeks and months are now built from daily bars, as Indian stocks' already were.
- **A live chart no longer freezes after the computer sleeps or changes network.** A connection that died without closing kept the chart showing "live" on old prices for good. Quiet connections are now pinged, and one that doesn't answer is reconnected (within about a minute).
- **Watchlist stock prices keep updating while Binance is unreachable.** They used to stop until Binance answered again.
- **Live hourly bars from Upstox, IBKR and Alpaca line up with their history.** They were placed on the clock hour (09:00) while history starts at the session open (09:30, or 09:15 for NSE).
- **A damaged price cache is set aside and downloaded again** instead of being half-read.
- **The diagnostics log stays under about 2 MB.**
- **Charts build bars for sessions Yahoo skips.** During trading Yahoo's bars leave out (Muhurat trading), its latest-price point could keep the chart from building bars from the live quote.

## 0.2.11 (2026-11-10)

Diwali's Muhurat trading and special Saturday sessions now count in Indian
prices and update live while they run.

### Fixed
- **The day after a special session shows the right change.** Diwali's Muhurat trading (Sunday 8 Nov 2026) and special Saturday sessions were left out, so the watchlist and market overview measured from the session before (SBIN +2.31% instead of +2.56% on 9 Nov) and disagreed with the hotlists.
- **Special sessions update live while they run**, like a normal trading day: charts get the session's bars, weekly and monthly bars include it, and the market dot shows it as open. The Calendar lists upcoming ones with their times.

## 0.2.10 (2026-11-10)

Send messages to the agent while it works, paste images with Ctrl+V, and
chart legends whose buttons stay in reach.

### New
- **Send messages while the agent works.** Press Enter mid-task and your message waits under the reply, dimmed (× takes it back); the agent gets everything you queued as soon as it finishes, as in Claude Code. Stop puts them back in the box.

### Changed
- **Ctrl+V pastes an image in the agent panel** (Cmd+V on macOS), instead of Alt+V. Copied text and files paste as before.

### Fixed
- **Shift+Enter starts a new line** in the agent's message box; it used to send.
- **A strategy with many inputs can be hidden or removed again.** Its name on the chart used to run past the edge (with the side panel open), taking its buttons with it; a long name is now cut short with "…" (the full one shows on hover) so the buttons always fit.
- **SQLite security update.** The built-in database engine is updated for a medium-severity SQLite advisory (RUSTSEC-2026-0459).

## 0.2.9 (2026-11-09)

Custom indices always reach back to their oldest stock, and scrollbars that
stay out of the way.

### Changed
- **Scrollbars stay out of the way.** They no longer appear just because the pointer is over a list or table (on the Indices board they covered the tickers); they show while you scroll and fade out a moment later. Pointing right at the edge still shows a bar to drag.
- **Custom indices always reach back to their oldest stock.** The "Index starts" column is gone: each stock joins the day after it lists, the way NSE adds stocks, with the level carried on unbroken. Indices with a stock set to "With it" are rebuilt this way once.
## 0.2.8 (2026-11-08)

Clearer wording for when a custom index starts.

### Changed
- **Clearer wording for when an index starts.** On an index's board, the "Counts from" column is now "Index starts": **Without it** (the default) starts the index with the older stocks and adds a newer one the day after it lists, keeping their full history; **With it** makes the index wait until that stock has prices, so it's in every day of the index.

## 0.2.7 (2026-11-08)

A heatmap of the whole market, custom indices that reach back as far as
their oldest stocks, and splits that no longer read as crashes.

### New
- **Market overview**: a button beside the layout grid button (or `Alt M`) swaps the charts for a heatmap of the S&P 500, the NIFTY 500 or the 150 largest coins. Tiles are sized by market cap, grouped by sector and coloured by the change over 1D, 1W, 1M, 3M, YTD or 1Y. Hover a tile for its price, change and market cap, click a sector to zoom in, click a tile to open its chart, and press Esc to go back. Watchlists and the agent stay on the right.
- **The agent can use the market overview**: ask it to show a market, or which sectors and stocks led over a period.
- **Custom indices reach back as far as their oldest stocks.** A stock now joins an index the day after it lists, the way NSE adds stocks, with the level carried on unbroken, so one young stock no longer cuts ten years of history down to one. Indices you've made are rebuilt this way once.
- **Choose when each stock counts.** On an index's board, "Counts from" switches a stock between its listing and the index's start (the index then begins only once that stock has prices), and a tick box leaves a stock out altogether. The agent can do both.
- **See how much history each stock has.** A History column shows how far back each stock's prices go, and the index row shows the index's own.
- **Lakhs and crores, or millions and billions, per market**: Settings → General → Large numbers sets how market caps and volumes read for US stocks, Indian stocks and crypto. Indian stocks start in crores.

### Fixed
- **Splits and bonus issues no longer read as a crash.** Charts, custom indices, the screener and Seasonality kept prices saved from before a split, so a 2-for-1 split showed as a 50% fall in 1M and 3M returns. Saved prices are now checked against the source each time and downloaded again when the source has adjusted them. Prices a source hasn't adjusted yet are adjusted on the split's date, including Yahoo's hourly bars when a split and a bonus land on one day (an index holding Bajaj Finance broke in two on hourly charts). The S marker under the chart reads "Split or bonus" for Indian stocks. Saved prices are downloaded once more after updating.
- **Custom index P/E, EV/EBITDA, margins and growth are right for companies that report in another currency.** Yahoo gives Infosys's earnings in dollars and its market cap in rupees; they're now in one currency, so an index holding it no longer shows a negative or far-off P/E.
- **An index's P/E and other ratios count what the index holds.** Each company counts for the slice the index owns (its free float, or its share in an equal-weight index), as NSE works out its indices' P/E, and a one-stock index reads its stock's own P/E.
- **No index day lost or counted twice.** A stock that doesn't trade on a day (a suspension, or a gap in the data) counts at its last price instead of dropping the day, and new weights take over exactly where the old ones left off.
- **1M and 3M returns on the Indices board are calendar months,** as in the market overview.
- **Sector changes in the market overview add up.** A sector's change was its stocks' changes weighted by today's market caps, which overstated winners over longer periods; it's now how much the sector's total market cap moved.
- **New coins show up in search the day they list.** Exchange coin lists now refresh daily instead of weekly, and a search that finds no coin asks Binance about it right away.

## 0.2.6 (2026-11-05)

Move your Opentern to another computer, as a file or straight over your network.

### New
- **Export and import** in Settings: tick what to bring (watchlists, layouts, drawings, templates, favorites, alerts, custom indices, paper accounts, scripts, screener and Strategy Lab settings), then save one file you can email or copy across.
- **Send to another device**: a six-digit code connects two computers on the same network, with no account and no server, and everything sent is encrypted.
- **Merge or replace when importing**: pick what to bring in, keep what's already there or replace it. A backup is saved first, and importing that backup puts everything back. API keys and passwords never leave the device.
- **Windows and macOS permissions have a way back.** Windows Firewall and macOS's Local Network prompt only ask once. If one was turned down, the send page has a link to allow it (Windows asks for administrator permission).

### Security
- **Imported files are treated as untrusted, however they arrive.** Imported Python scripts never start by themselves, not even from a saved layout: press Run in the Script Editor and confirm first. Saved layouts, alerts and screens can't reach scripts outside the scripts folder.
- **Importing shows what a file really contains**, read from its contents: how many Python scripts, and where its alerts would send things. It can't change a mail or Telegram setup you already have, so your saved password never goes to another server.
- **Transfers stay on the local network.** Internet addresses are refused, a device pretending to be the sender gets only three tries per search, and a connection has 20 seconds to prove the code.

### Fixed
- **Luau scripts can no longer load files or raw bytecode.** `require` and `loadstring` were reachable and are gone.
- **Only web pages open from links.** A news item or other link that points at a file, a program or another app is now ignored.
- **Odd symbol names can't write outside their folders** when saving logos or snapshots.
- **Script names Windows can't save** no longer stop an import: the rest come in and the message lists which were left out.

## 0.2.5 (2026-11-02)

Alerts keep working with Opentern closed, stock charts show earnings and
extended hours, and the chart's clock picks any timezone.

### New

- **Choose where scripts and snapshots live.** Settings → General has a
  folder for each: change it, open it, or go back to the default. Scripts
  on the chart, the Script Editor and the agents follow the new scripts
  folder.
- **Pick the chart's timezone from the clock.** Click the time at the
  bottom right for Exchange, UTC, Local or any of 80 cities (search by
  name or offset). Chart settings → Symbol has the same list.
- **Alerts keep working when Opentern is closed.** Closing the window
  hands your active alerts to a small background process that watches
  them and sends them as usual (desktop, sound, webhook, Telegram,
  email); opening Opentern takes them back, with a note of any that fired
  meanwhile. Settings → General → Alerts turns it off, or on at login
  too.
- **Alerts on other symbols and indicators off the chart.** An indicator
  or script alert keeps working after you switch symbol or remove the
  indicator, and price alerts work for any symbol, not only ones in the
  watchlist.
- **Earnings on the chart.** Stocks show an E marker for each report:
  green when it beat the estimate, red when it missed, grey for the next
  one. Hover for reported and expected EPS and the surprise.
- **EXT: pre-market and after-hours bars** for US stocks on intraday
  charts, shaded as on TradingView (the EXT button in the bottom bar).

### Changed

- **Settings → General** gathers the folders, the theme and symbol logos
  (Appearance and Symbol logos were categories of their own).

### Fixed

- **Data providers settings fit a small window** instead of running off
  the right edge.

- **Solana's logo is Solana's again:** its purple-to-mint bars on black,
  instead of a teal and green disc.
- **Pasting a copied file into the agent panel works on Windows** (its
  `file:///C:/…` links were read as a computer name and ignored).

## 0.2.4 (2026-10-31)

Seasonality gets a chart of every year side by side, and the agent panel can
now work with Seasonality and custom indices.

### New

- **Seasonality chart.** A toggle at the top left of the Seasonality tab
  switches between the table and a chart with a line per year (the change
  since the year before ended), a dashed average and each year's latest
  change tagged on the right. Hover to see every year's change on that day.
- **An Average row in Seasonality**, above Rises and falls: each month's
  average return over the years shown.
- **The agent works with Seasonality and custom indices.** Claude and Codex
  can read a symbol's monthly returns, and list, read, make, edit,
  rebalance, compare, delete and show custom indices.
- **A refresh button for News.** It turns while headlines load.

### Fixed

- **Seasonality numbers no longer run into each other** when the side panel
  is open or a year's return is in the thousands of percent: columns fit
  their numbers, with fewer decimals when space is short (hover for the
  exact value).
- **News and Hotlists recover by themselves after a dropped connection.**
  They try again 15 seconds after a failure instead of waiting minutes or
  needing the chart reloaded.

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

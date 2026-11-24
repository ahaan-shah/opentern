# Notes: market overview, custom indices, Seasonality

Moved out of `CLAUDE.md` (Nov 2026) to keep the briefing short. Keep it current: update it in the same commit as the code.

## Market overview (0.2.7, Nov 2026)
- **Top-bar button** (a drawn treemap, left of the grid button; `Alt M`;
  `Arrangement::overview`, saved). Open, it takes the chart area *and* the
  bottom panel's place (no drawing toolbar, range bar or bottom panel); the
  side panel and rail stay. Settings still wins over it. The chart controls
  in the top bar are disabled meanwhile; symbol search stays, and
  `open_symbol_synced` (any symbol pick: search, watchlist, a tile) closes it.
- **Data** (`opentern_data::overview`, `Request::Overview` → `Event::Overview`,
  one thread per request): US = S&P 500 (bundled `assets/overview/sp500.csv`
  with an Exchange column, refreshed weekly from the PDDL dataset on jsDelivr;
  the live file has no exchange, so it comes from the bundled one or the
  screener), India = NIFTY 500 (bundled NSE file, refreshed weekly). Prices,
  1D and history: Yahoo spark (20 a request, 4 threads, range 2y so 1Y
  always reaches back). **1D is from the previous session's close, like
  the watchlist.** **Spark's `fulldayPrice`/`fulldayChangePercent` include
  pre-market and after-hours trades** (checked 12 Nov 2026 against the
  1-minute pre-market bars; that's why it once read NVDA +3.91% for a
  181.60 → 186.64 day). US tiles use the daily bars' last close (the
  regular session; before the open the latest bar is yesterday's, and
  mixing the two gave NVDA −6.18% for a +2.87% day); the extended price
  goes in `Member::ext` (`yahoo::Ext`, only while
  `calendar::us_extended` says pre-market or after hours) for the hover
  card and the header's ☀/☾ chip. For
  Indian stocks it *is* used: it matched NSE's own figures for all NIFTY 50
  stocks and counts sessions Yahoo's bars skip (see "Special sessions"). **Caps:
  US from Yahoo's `largest_market_cap` predefined screener** (no crumb; ~10
  pages reach every member but about 9 small ones, which go to batch quotes);
  **India from v7 batch quotes** (needs the crumb session fundamentals use).
  Caps are kept as share counts in `<cache>/overview/shares-*.json` (refreshed
  daily), so caps follow the price when Yahoo refuses later. Under half with
  caps → `Sizing::Equal` (the legend's tooltip says so); the rest get the
  smallest known cap. Crypto: Binance's 24h tickers, USDT pairs with
  volume (filtered: stablecoins, wrapped/staked, leveraged UP/DOWN;
  `excluded`), the 150 most traded, sized by 24h volume (`Sizing::Volume`;
  no market-cap source); categories from the `CATEGORIES` table, else
  "Other". Every period comes from Binance like the charts: 1D from the
  UTC day's open (`ticker/tradingDay`, 100 a request; the watchlist's daily
  change), the rest from daily klines (kept an hour) to the live price.
- **Yahoo's crumb endpoint answered 429 for a while on 8 Nov 2026** (then
  worked again). Don't try to get around a limit; the design above keeps
  working without the crumb for US and with kept share counts for India.
- Periods (`changes`): 1W = close at or before 7 days ago, 1M/3M/1Y by
  calendar months, YTD vs the last close of the previous year (UTC); too
  little history = NaN (grey tile). Colour scale per period (±3% 1D … ±30%).
- UI (`app/src/overview.rs`): sectors squarified by total size, then members
  (reuses `indices::squarify`); `plan` keeps the map filling the view as
  before; sectors too small there for their name (the maintainer: don't
  change the layout) move to a strip below it, scrolled to (floating
  scrollbar; no ScrollArea when all fit); sector titles zoom (`Overview::sector`), Esc
  unzooms then closes; tile labels by size (logo + ticker + %, ticker + %,
  one line for wide-short, logo only); hover card on the Tooltip layer;
  refresh every 3 min while open, 30 s after a failure; in between,
  while the market trades (US incl. extended hours, crypto always),
  `Request::OverviewPrices` every 30 s (`overview::prices`: 5-day spark
  for stocks, Binance trading-day tickers for coins, keyed by
  `Member::key`; `Board::reprice` keeps each period's base). A coin gets
  a key only if Binance answered for its pair (one unknown pair fails a
  whole batch). `Overview::drawn`
  records tile/sector/button rects for tests. Market and period saved under
  settings key `overview`. The legend's tooltip names the source. Stills: `overview[:market[:period[:sector]]]`,
  `overview-hover:<ticker>`.
- **Large numbers** (Settings → General): per market, millions/billions or
  lakhs/crores (`format::Numbers`, settings key `number_style`, a global
  read by `format::big`/`compact_for`; Indian stocks default to crores).
  Used for overview caps and every volume (legend, data window, measure,
  screener, volume scales). Not in Export and import (no preferences item).
- Agent tool `market_overview` (open/close/get; `claude/markets.rs`,
  `Wait::Overview`). Tests: `drive::tests::market_overview_…`,
  `control::drive_tests::the_agent_reads_and_opens_the_market_overview`,
  data unit tests, and `live::overview_boards` (ignored, real services).

## Seasonality, custom indices, agent accounts (30 Oct 2026, for 0.2.3)
- **Seasonality** bottom tab (`seasonal.rs`): the active chart's monthly
  returns (`opentern_series::profile::monthly_returns`: month close vs the
  month before's, the first month from its open; Year vs last year's
  close), newest year first, a two-handled year slider (middle grip drags
  the window; default the last 6 years), "Rises and falls" per column
  under an Average row. Columns never get narrower than their numbers
  (fewer decimals first, then a sideways scroll). Chart view (toggle top
  left, as TradingView's): a line per year from daily bars
  (`profile::seasonality`, fetched only once the view opens), dashed
  average, tags on the right pushed apart, hover line + readout. Monthly
  bars via `Request::Fetch` 1M, seqs from `seasonal::SEQ_BASE` (1 << 45);
  monthly and daily replies are told apart by their own seqs. Fonts lack
  ▲/▼: the triangles are drawn. Still: `season-chart`.
- **Custom indices** (`indices.rs`, bottom tab "Indices"; **v2, still to
  iterate with the maintainer before release**). Method = NSE's except
  caps (module doc): free-float market cap (Yahoo `floatShares`; all
  shares when a stock has no float figure; also full cap or equal), base
  1000 on the first common day, **segments**: weights hold from a time on,
  and a rebalance (1 April / 1 October when the app sees the date pass, any
  member or weighting edit, or "Rebalance now") starts a new segment
  scaled so the level at the close before is kept (the divisor). Each
  segment keeps the closes it was set at (`anchors`) and rescales its
  units when a split shows up as a changed past price. The hub charts
  segments (`expr::set_customs(id → [(from, formula)])`,
  `ExprProvider::from_segments`; live bars stream the current segment's
  legs only); custom ids are never cached. Old saves (one set of
  `units`) migrate to a first segment. Not done: caps (the maintainer
  left them out), NSE's IWF banding, historical float (past segments use
  the float known when they were made).
- Board: tiles for the index's own numbers (one row when names fit, else
  7 or 5 a row), then a table (logos, weight with a mini bar, price, 1D,
  metrics under group bands with rules between groups; numbers coloured
  by score, no cell boxes; P/E and EV/EBITDA rank lower-better,
  loss-makers worst) or a treemap. **Compare** (top of the list once
  there are two indices): every index's own numbers side by side, table
  or map (tiles by members' total market cap). Index P/E = Σcap/Σearnings,
  EV/EBITDA = ΣEV/ΣEBITDA, margins revenue-weighted. `clean`: Yahoo's 0
  for missing margins (banks) and EV/EBITDA outside (0, 200], |P/E| > 1000
  become "—". Members' data: daily bars (1500, seqs from 1 << 46) and
  `Request::Fundamentals` (Yahoo `quoteSummary` + cookie/crumb, one at a
  time; demo makes numbers up). Settings key `custom_indices`. Stills:
  `index-map`, `index-edit`, `index-chart`, `index-compare`.
- A maximized bottom panel takes the chart's and range bar's place. Settings
  hides it while open (`covered` in `shell.rs`, as the overview does; the
  layout isn't touched, so closing Settings brings it back).
- **Map of every index** (0.2.12): the treemap button beside + in the
  Indices list (`Indices::map`) shows all indices on the market
  overview's heatmap (`App::heat_map_scrolled` with a `MapSpec`: the
  overview passes its own state, the indices theirs). An index = a
  sector, sized by `Board::total_cap` (equal when one is missing),
  members by weight; the sector label shows the index's own change
  (`MapSpec::sector_change`, from `index_series`), hover cards show the
  weight. `Indices::map_board` builds it, cached on `version`. Period and
  zoom aren't saved. Still: `index-overview`.
- **Narrow panes:** the Indices tab lays out with fixed rects (header
  child, body child). A `horizontal` header wider than the pane grows
  the ui's rect, and everything after it is laid out past the panel's
  edge (that's what overlapped when the window was tiled below its
  1000 px minimum).
- Indices list: a search square beside Compare (`Indices::search`, filters
  cards by name or ticker; Esc closes); the map button is a toggle (Compare
  isn't cleared under it). Watchlists: a search button in the header
  (`watchlist_search`, temp state `("watchlist","search")`) lists matching
  lists by name in place of the circles and rows.
- **Custom indices, still open:** colour against the index's value
  instead of by rank (kept rank for now); caps if ever wanted.
- Formula lexer accepts `&` in tickers (`NSE:M&M`).

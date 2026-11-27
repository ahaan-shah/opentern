# Notes: data providers, prices and market data

Moved out of `CLAUDE.md` (Nov 2026) to keep the briefing short. Keep it current: update it in the same commit as the code.

## How data flows
`App` sends `Request::Open { chart, symbol, tf }` → the hub starts a session thread: cache →
publish → network tail → publish → stream (Binance WS / Yahoo poll / demo / formula legs),
reconnecting with backoff; `More` loads older history (cache first). Events come back over
a channel and wake the UI. **Each (re)open gets a new chart id** (`App::open_chart`);
events for old ids are dropped. Studies update incrementally: `Instance::update` steps
closed bars once and recomputes the forming bar on a clone. `Chart::data_version` bumps on
every data change (scripts re-run on it).

## Phase 9 (done): providers, calendars, dividends
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
  (daily since Nov 2026; a coin search with no match asks Binance's exchangeInfo
  for that pair at once, alongside Yahoo's search), ties prefer Binance, Coinbase, Bybit, OKX, Kraken; `EXCHANGE:` narrows,
  and `EXCHANGE:` alone lists that exchange (`hub::browse`: major coins first, up to 500);
  `OANDA:X` / `IBKR:X` are offered as typed.
- **OKX gotcha:** `www.okx.com` is unreachable from some networks (it is from this
  machine); the adapter falls back to `app.okx.com` / `my.okx.com` and remembers.
- **Yahoo hosts (Nov 2026):** `provider::get_json` sends every
  `query1`/`query2.finance.yahoo.com` URL through `yahoo_get`: the host
  that answered last gets 5 s, then the other (502s after 15 s on query1
  for NSE charts while query2 answered at once); a 404 = `provider::GONE`
  ("no data for this symbol"), and `hub::is_gone(id)` remembers it so
  index boards flag members whose cached bars still serve.
- **Error wording:** `Error::Offline` says "check your internet" only when
  nothing answered in 2 min (`provider::net_ok`, set by every reply and
  websocket message); else "Couldn't reach the data service". `Error::Down`
  names a service that's failing. The chart shows the no-signal icon only
  for `provider::NO_INTERNET`. A session's status goes `Live` as soon as a
  load succeeds.
- **Custom index charts:** `ExprProvider::latest` fetches members through
  `hub::fetch_cached` (the bar cache, in parallel); each member streams and
  retries on its own thread, a member behind counts at its last close, and
  the stream ends (with the named error) only when every member fails.
- **Symbol search:** known symbols (exchange lists, starter, the bundled
  S&P 500 and NIFTY 500 via `overview::listed_stocks`) answer at once with
  `Event::Search { done: false }`; Binance lookups and Yahoo search run on a
  thread, one typed query at a time (the latest waits), merged answer
  `done: true` (the agent waits for it; Enter waits at most
  `ENTER_WAIT`, 1 s, then takes the highlighted row). Empty query: recent picks
  per tab (`App::recent_symbols`, settings key `recent_symbols`, 40).
  Ranking (`hub::rank`): exact ticker, ticker prefix, coin names
  (`feeds::COINS`: "bitcoin"), name start, a word of the name, then
  contains, words in any order (`loose_rank`), typos last (`typo_rank`,
  1 edit, 2 from 7 letters). The dialog lifts watchlist/recent symbols
  that match from the start. Rows show price and day change (the feed's
  quote, else `Request::Peek` → `Event::Peek` for the top 14:
  Binance trading-day batch, Yahoo spark, else two daily bars), the match
  in the accent, and a star (Shift+Enter) that toggles the open
  watchlist; Tab/Shift+Tab step the kinds. Still: `search:<query>`.
  **Binance gotcha (Nov 2026):** a 400/404 from one host is final
  (`provider::get_json_refusal`): falling through to `api.binance.com`
  on Jio meant an 8 s connect timeout per unknown-coin lookup, which held
  search's Enter.
- **Binance gotcha (Oct 2026):** Jio (India) points `api.binance.com` and
  `stream.binance.com` at a dead address, so charts silently stayed on cached
  bars. Binance goes to its public market-data mirrors first
  (`data-api.binance.vision`, `data-stream.binance.vision`; same data, checked
  byte for byte), falls back to the main hosts on any failure, and remembers
  which answered (`binance::get`; depth and hotlists use it too). Connects time
  out after 8 s (`provider::agent`, `provider::websocket`). Jio also resets
  some handshakes to `api.kraken.com` (SNI); not worked around.
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
- Commodities (Oct 2026): `AssetKind::Commodity`, `opentern_core::commodities()`
  (gold, silver, platinum, palladium, copper, WTI, Brent, natural gas, RBOB,
  heating oil, wheat, corn, soybeans, coffee, sugar, cocoa, cotton, cattle) named
  like TradingView's continuous contracts (`COMEX:GC1!`), served by Yahoo's front
  month (`GC=F`; `ICEEUR:BRN1!` → `BZ=F`). Hours treated as forex's (Sun–Fri,
  New York), which ignores the daily break and the grains' shorter sessions.
  **Checked 22 Nov 2026:** Yahoo's daily closes equal the official settlements
  (gold, Brent, WTI), but intraday prices are delayed (CME: ≥ 10 min on free
  data), so the legend shows "Delayed 10 min" (`hub::delayed_minutes`). `BZ=F`
  is NYMEX's Brent Last Day Financial, a cash-settled copy of ICE Brent.
  Futures ≠ spot: futures sit above spot by the cost of carry. Spot comes
  from OANDA (`opentern_core::spot_commodities()`, `OANDA:XAUUSD` …; listed
  in search only with an OANDA key; search tags rows "futures"/"spot").
  No free keyless real-time source exists for either (CME blocks and forbids
  scraping its site). Icons: our own drawings in `assets/commodities/`
  (`scripts/make-commodity-icons.py`), keyed by `opentern_core::commodity_of`.
- Not yet: Dhan/Angel One/Zerodha, Upstox's protobuf websocket, IBKR TWS socket API,
  extended-hours shading, earnings markers.

- **Daily-or-longer bars are stamped at 00:00 UTC of their local date**
  (`resample`), so read their dates in UTC, never the symbol's zone (US
  months landed a month early in Seasonality until Nov 2026; `scale.rs`
  and `seasonal.rs` do it right). Intraday bars use the symbol's zone.
- A custom index with several segments has no native 1W/1M
  (`ExprProvider::natives`): its weeks and months are resampled from its
  own days, since weights change on a day, not at a month's start.
- **Indices and rates** (Nov 2026): `opentern_core::INDICES` is the one table
  (TradingView-style id → Yahoo ticker, name, currency, zone); `yahoo::ticker` /
  `id_for` map both ways; search knows them. `calendar::Market::Exchange(Venue)`
  reads each exchange's days from `core/src/venue_days.rs` (generated by
  `scripts/exchange-holidays.py` from `exchange_calendars`, Apache-2.0:
  regular hours, lunch, holidays with names, half days and special
  sessions; rule-based exchanges to 2031, lunar ones (HK, Shanghai, Seoul,
  Taipei, Singapore) as far as announced; past `until` = weekdays at
  regular hours). Announcements the project lacks go in the script's
  `EXTRA_HOLIDAYS`/`EXTRA_SPECIAL` (KRX Constitution Day 2026, SGX's noon
  half days). Checked Nov 2026: 2026 lists against published calendars
  (Shanghai, Taipei, Seoul, Singapore) and NSE's list = the project's XBOM.
  `Market::of` checks ids for `TVC:` ones. Icons: flags
  (`assets/flags/`, flag-icons MIT, `logos::flag_of`) for non-US/India
  indices, yields and DXY; badges for the rest.
- **The Fed funds rate** (`NYFED:EFFR`, Nov 2026, `nyfed.rs`): the New York
  Fed's markets API, keyless. `…/effr/search.json?startDate=2000-07-01&endDate=…`
  gives the whole history (~6,600 days, 1.5 MB) once per run; then
  `…/effr/last/10.json`. Every caller (chart, quotes every 30 s, alerts,
  search prices) goes through one memo (`nyfed::days`) whose `next_check` asks
  only after 9:05 and 14:35 New York on weekdays (every 15 min between them
  while the previous business day is missing; 5 min after a failure). Fields:
  `effectiveDate`, `percentRate`, `targetRateFrom`/`To` (before 16 Dec 2008
  only `From`: one target), `revisionIndicator` "r". Bars: O=H=L=C, volume 0,
  UTC midnight; weeks and months resampled in the provider; no intraday
  (`nyfed::DAILY_ONLY`). `since()` returns a week extra so revisions replace
  bars. The target band (`chart::ui::target_band`) and legend "Target …" read
  `nyfed::targets()` (never fetches). `opentern_core::daily_only(id)`: step
  line instead of candles, no market dot ("Daily" tag with the source and
  terms instead), never stale, change in bp. Search aliases:
  `opentern_core::search_aliases`. Terms checked 27 Nov 2026 (page updated
  9 Jun 2023): automated access OK if it doesn't burden the site; store and
  redistribute OK for personal or business purposes; the notice
  (`nyfed::NOTICE`, kept in English) wherever it's shown; no endorsement or
  name in advertising. Live test: `fed_funds_rate_from_the_new_york_fed`.
- **Fed funds futures** (`CBOT:ZQ1!`, Nov 2026): `opentern_core::futures()`, kind
  `Futures`, Yahoo's `ZQ=F` (Yahoo's search tags it `CBT`, `id_for` maps it back).
  Tick 0.0025 (CBOT's front month), hours and the 10-minute delay like the commodities,
  the US flag as its icon, search aliases "ZQ", "ZQ=F", "fed funds futures". Before
  this, `ZQ=F` opened only as a raw Yahoo ticker (no name, wrong tick, no delay tag).
- **Perpetuals** (`….P`, TradingView's name): `Binance::perp()` (fapi; its
  market streams are under `/market/ws/` since late 2026: the old `/ws/`
  path connects and sends nothing), `Bybit::perp()` (`category=linear`),
  OKX `BTC-USDT-SWAP` (volume from column 6: column 5 counts contracts;
  its book counts contracts too, so the DOM says so). Lists saved as
  sources `*_perp` (meta `symbol_lists_at_v2` forced a refetch). Quotes via
  per-pair streams. `Request::Funding` → legend "Funding +0.0034% 3h 11m"
  (`App::poll_funding`, once a minute). Perpetuals never get a spot
  `successor`. Not yet: open interest.
- **Hyperliquid perpetuals** (`hyperliquid.rs`, Nov 2026; verified live by
  `live::hyperliquid_perpetuals`): everything is a POST of JSON to
  `api.hyperliquid.xyz/info` (`provider::post_json`): `candleSnapshot`
  (by start/end time; asked in pages of 4000, under the 5000 a reply holds),
  `metaAndAssetCtxs` (the list, list source `hyperliquid_perp`, and funding:
  hourly, paid on the hour), `l2Book` (the DOM, 20 levels a side). Live bars
  from `wss://api.hyperliquid.xyz/ws` (`candle` subscription, `{"method":"ping"}`
  every 30 s: it drops a connection quiet for a minute). Ids are
  `HYPERLIQUID:<coin>USDC.P` (USDC, as Hyperliquid's app names them) with the coin's own case (`kPEPE` is 1000 PEPE).
  **Only the newest 5000 candles of each interval exist** (1m ≈ 3.5 days);
  paging back stops there. Tick size isn't published: five significant
  figures, at most `6 - szDecimals` decimals, whole numbers always
  (`tick_size`, from the mark price when the list is fetched).
- **Hyperliquid builder dexes (HIP-3)**: `perpDexs` names them (xyz, para, io,
  km…); each dex's `metaAndAssetCtxs` takes `"dex"` and names its markets
  `xyz:GOLD`. Ids put the dex before an underscore, `HYPERLIQUID:XYZ_GOLDUSDC.P`
  (a hyphen would read as a formula's minus), quoted in the dex's collateral
  (`collateralToken`, a `spotMeta` token index: USDC, USDH, USDE, USDT0).
  Candles, the stream and `l2Book` take `xyz:GOLD` as the coin; funding asks
  that dex. Search ranks them by the name after the underscore. The same asset
  can be on several dexes (NVDA on xyz and io). Kind is `Crypto` (they trade
  around the clock). Spot is still left out. Hyperliquid lists only tickers,
  so `hyperliquid::name_of` names the ones a ticker doesn't explain (SpaceX's
  SPCX, OpenAI, gold, crude oil) in the description; a US company in the
  results (Yahoo's "tesla" → NASDAQ:TSLA) brings its perpetuals right after
  it (`hub::with_hyperliquid`, by `hyperliquid::underlying`). Logos: the app
  draws what a market follows (`logos::stand_in`: gold's picture, EURUSD's
  flags, the S&P's badge, the company's logo tried on NASDAQ, NYSE and AMEX);
  private companies and crypto gauges keep a letter. Prices checked against
  spot on 27 Nov 2026 (gold 3,649.50 vs Binance's gold index 3,650.66; COMEX
  futures sit higher by their premium).
- **Delisted pairs:** Binance keeps listing pairs it stopped (status
  BREAK, e.g. XMRUSDT): `ticker/price` and `24hr` still return the last
  trade from years ago, klines end then, and a `tradingDay` batch with any
  unknown pair fails whole. Never guess a Binance pair: the exchange lists
  (refreshed daily, trading pairs only; Kraken filtered by `status`) are
  the truth. `hub::successor` moves a pair missing from its exchange's list
  to the same coin elsewhere (`Event::Replaced` → `App::pair_replaced`:
  watchlists, charts, price alerts, one note). The overview fetches
  Binance's list itself before the lists are in, bisects refused
  `tradingDay` batches, and ignores closes older than 4 days.
- **Logos resolve on two threads** (0.3.2): `logos` answers bundled,
  drawn (commodities, flags) and kept logos at once (`local`); only misses
  go to `NET_WORKERS` (4) `logos-net` threads. With one queue, the crypto map's 150 coins and the
  world indices' members kept bundled logos as letter circles for minutes.
- **Logo pictures are binary `.png` files** (`BUNDLED_IMAGES`, `include_bytes!`),
  never base64 in a text `.logo`: long base64 matches the leak check's API-key
  shapes. Runtime sources keep base64 only in `<workspace>/logos/`. logoapi.sh
  answers stand-ins (favicon, name tile, wordmark) with 200 + `X-Logo-Fallback`:
  treat those as no logo. Coin logos: web3icons, then cryptocurrency-icons
  (both MIT; about 240 of Binance's ~525 USDT coins). Checked and not used:
  Trust Wallet's token lists (MIT, 389 tokens, adds ~8), ErikThiart's set
  (MIT label, but "sourced from CoinMarketCap").
- Formulas: subtraction needs spaces (`AAPL - MSFT`) because `-` can be part of a
  ticker (`BRK-B`); bare tickers resolve via known symbols, crypto quote suffixes →
  Binance, else `YAHOO:`.

## Special sessions: Muhurat trading and Saturdays (0.2.11, Nov 2026)
- **Yahoo's bars skip some sessions NSE and BSE hold on holidays and
  weekends** (Muhurat trading on Sunday 8 Nov 2026 is missing from its
  daily *and* intraday bars; also 2010–2016's Muhurat sessions, 12 Nov 2023,
  the 1 Feb 2020 budget Saturday and the three 2024 disaster-recovery
  Saturdays; it has the others). Its *quotes* count them (`fulldayChange`,
  the hotlist; matched NSE for all NIFTY 50 stocks), so bar-based numbers
  disagreed with the hotlist. **Yahoo's figure was right; I first "fixed"
  the hotlist the wrong way** (reverted). Check the exchange's own data
  (by hand) before calling a reputable source wrong.
- **NSE's terms forbid fetching its files from the app** (Terms of Use,
  updated 29/10/2025, §9: no "systematic or automated data collection";
  §8: no storing without written permission, and no use "for any gaming,
  virtual trading or simulation activities", which paper trading and
  backtests are). A fill from NSE's and BSE's end-of-day files was built,
  verified to the paisa, and removed before release for this reason (the
  maintainer's call). Don't add automated NSE/BSE downloads back; a
  licensed source (a broker API the user signs up for, e.g. Upstox) is the
  way to get the missing sessions. BSE's terms weren't found. The
  overview's NIFTY 500 list fallback to `archives.nseindia.com` went with
  it; niftyindices.com (NSE Indices, own terms, not checked) and the
  bundled `nifty500.csv` remain: check their terms.
- `calendar::NSE_SPECIAL` lists every special session (date, trading
  windows, name; windows empty for old ones whose times aren't on record).
  Every date was confirmed by hand in NSE's files. `session`/`state`/
  `last_close_in` use it (breaks between windows count as closed); the
  Calendar widget shows them. **Add each new one when the exchange
  announces it** (with `NSE_HOLIDAYS`).
- **While a skipped session runs, and until the next one:**
  `yahoo::skipped_day` adds the day's bar from the quote (high, low,
  close, volume right; open = previous official close, as Yahoo gives
  none); `yahoo::OwnBar` builds intraday bars from the quote in `stream`
  while it's fresh and the exchange is in a session (or it's a weekend or
  holiday: an unlisted special session). Yahoo's own bar always wins. The
  cache keeps what it saw; Yahoo's history never gets these days, so a
  machine that wasn't running then has a gap. Indian weeks and months are
  resampled from daily bars. Indian quotes' previous close is Yahoo's
  `fulldayChange` (`quote_from`); the overview's India 1D is spark's
  `fulldayChangePercent`. `Session::upsert` inserts an older bar it lacked
  instead of dropping it. Never seen live yet: the next Muhurat session is
  the first real test.
- **Yahoo's daily history for most BSE tickers is a single bar** (SBIN.BO,
  RELIANCE.BO, INFY.BO, ITC.BO; TCS.BO fine), whatever the period asked: a
  Yahoo problem; the chart says so (see "Robustness pass").

## Pre-market and after hours (0.2.12, Nov 2026)
- As TradingView with EXT off: candles, the watchlist and every 1D stay
  on the regular session; the extended price is only shown beside it.
  `calendar::us_extended` (04:00 to the open, the close + 4 h, trading
  days only). The quote worker sends `Event::Extended` for US stocks and
  funds in `Request::Quotes` (one spark call per 20; `None` once the
  hours end); `subscribe_quotes` adds US stocks on any chart
  (`watchlists::has_extended_hours`). `Chart::extended` /
  `extended_now()`: the legend's sun/moon (price in the tooltip) and an
  amber/blue price label and dotted line (Chart settings → Scales and
  lines, `ext_price_label`/`ext_price_line`), included in auto-scale;
  hidden with EXT on (the bars show it). After-hours prices from
  `fulldayPrice` are assumed like pre-market's; not yet seen live.

## Robustness pass (0.2.12, Nov 2026)
- **Websockets go silent when a network dies without a close** (sleep,
  Wi-Fi change): reads only time out. `provider::Alive` pings after 30 s
  of silence and drops the connection when no pong comes within 20 s;
  every `read_text` takes one. Polling sources rely on HTTP timeouts.
- **Stale prices are said, never hidden:** `Chart::stale_for` (legend,
  5 min without the newest bar changing while `chart::steadily_open`),
  `App::stale_quote` (watchlist, by `quote_at` or a chart of the symbol),
  the overview header (`Overview::got`). Each requests a repaint every
  30 s, since a stalled feed sends nothing that would wake the UI.
- **`BarBuilder` continues the history's grid** (`phase` from the last
  bar): stocks' hourly history starts at the open, not on the hour.
- **Yahoo's daily (and weekly) history is one bar for most BSE listings**
  (INFY.BO, ITC.BO, HDFCBANK.BO, LT.BO; TCS.BO whole; intraday fine;
  checked 12 Nov 2026). `Chart::data_gap` says so, and says when a recent
  special session is missing. Don't build BSE days from hourly bars: the
  official close is the last 30 minutes' VWAP, so they'd be subtly wrong.
- **Cache:** checked once per run (`PRAGMA quick_check`), a damaged file
  is moved to `bars.db.broken`; Settings → General shows its size and
  clears it (`cache::clear`: VACUUM, then checkpoint). The workspace gets
  `backups/daily-<date>.opentern` once a day (`transfer::daily_backup`,
  7 kept, off the UI thread); the can't-open error points at them.
- Live-tested 12 Nov 2026: US closes matched Nasdaq for 33 sessions on 4
  symbols; Binance vs Coinbase median gap 0.13%.

## One Yahoo price feed (0.2.13, Nov 2026)
- **Quotes** (`hub::YahooQuotes` in the quote worker): every Yahoo-routed
  symbol (watchlist, paper, charted US stocks, custom indices and **all
  their members**) goes in one spark batch (20 a request), every 30 s
  while its market trades (`calendar::trading`, US incl. extended hours),
  once more after the close (`calendar::closed_between`), else every
  5 min. `Spark::quote` = regular price vs the session before (Indian:
  Yahoo's day change, snapped to the close it rounds from). The same
  spark gives the extended-hours price. **Forex (`=X`) stays on
  `Yahoo::quote`: spark rounds it to 4 decimals.** A ticker spark leaves
  out is asked on its own. Formulas/indices are recomputed once per batch.
- **`overview::sparks` caches each reply for 15 s**, so the overview's
  30 s prices, its full board (`2y` answers `5d`) and the quote worker share
  fetches (`overview::spark_requests()` counts them). The quote worker also
  applies any spark the overview fetched (`overview::cached_sparks`, newer than
  `YahooQuotes::seen`) at once and counts its next turn from it, so the
  watchlist and the map change together.
  In the app, `App::spread_quote` passes every quote (and a chart's newer
  price) to the overview's tiles (`Board::quote`) and hotlist rows, so a
  symbol shows one price everywhere. The India hotlist uses `overview::sparks`.
  The other way: `share_board` (after overview prices or a board) and
  `share_hotlist` (a new hotlist) write symbols the feed doesn't cover
  (`!App::quoted`) into `App::quotes` and on to the other view; covered
  ones take the table's price instead.
  Crypto hotlists rank by Binance's rolling 24 h change (only the price is
  shared there).
  Every charted symbol is in the quote subscription (not only US stocks).
  Bybit/OKX/Coinbase/Kraken pairs get one stream thread each in the quote
  worker (`hub::stream_quote`, 1D bars, ≤ 1 update/s; `PairStreams` stops
  them with the worker). Keyed providers still use `daily_quote` every 30 s:
  Alpaca's free plan allows one websocket (the chart's), and Upstox/IBKR
  would need batched quote calls nobody can test without an account. Charts still poll
  their own bars (spark has no volume or intraday bars).
- **Index members are live:** `App::indices_quote` puts each member's quote
  on its daily bars (`take_quote`: moves today's bar or starts it once
  `calendar::session_day` says the session opened), so the board, weights
  and the map follow the market. Members' bars and fundamentals download
  once a day (`due`: a new local day or 24 h; failures retry after
  `RETRY`, 15 min).
- **Index numbers are incremental** (Nov 2026; 63 indices of 575 stocks
  froze the app ~1 s per price). `Indices::refresh`: each member's
  `MemberData::{hist, stamp}` (from `Indices::touch`) says what changed;
  `MemberCalc` keeps its RSI/SMA state over closed days (`Steps`, the
  built-ins' own `ta::Rsi`/`ta::Sma`, so the numbers are theirs) and only
  adds today's step; `IndexCalc` keeps the index's series and redoes only
  from its newest day (`index_series_from`) unless the definition
  (`IndexDef::fingerprint`) or a member's history changed. Stale indices
  run side by side on rayon. Boards are `Arc`s (no per-frame copies).
  Rows' price and 1D come from `App::quotes` (the watchlist's); P/E and
  EV/EBITDA follow the live price from `MemberData::fund_price`
  (`at_price`). Tests: `incremental::live_prices_give_what_a_full_rebuild_gives`,
  timing `indices_timing` (ignored).
- The overview's 3-min full rebuild runs only while its market trades, plus
  once after the close.
- Live check (ignored; run while US stocks trade): `cargo test -p
  opentern-data --test live yahoo_quotes_are_batched -- --ignored --nocapture`.

## Splits in cached prices, index tick boxes (0.2.7, Nov 2026)
- **The cache kept pre-split prices** (it only ever appended), so a split
  read as a crash in 1M/3M returns. Every top-up now asks from three bars
  back (`hub::overlap_start`); if a closed cached bar's close moved > 2%
  (`history_changed`), the series is cleared and downloaded again. Both
  chart sessions and `fetch_once` (screener, indices, Seasonality, feeds).
  `yahoo::split_adjust` (applied in `Session::series` once corporate
  events are in) adjusts splits a source hasn't yet: only when the price gap
  across the date matches the ratio and the ratio is ≥ 1.2 (or ≤ 1/1.2).
  **Yahoo lists Indian bonuses as splits, and not always the whole action**
  (Bajaj Finance's June 2025 1:2 split + 4:1 bonus shows as 2:1, prices
  adjusted ×10), so never adjust by ratio alone. Markers read "Split or
  bonus" on NSE/BSE.
- **Yahoo ends intraday replies with a "now" point** (time =
  `regularMarketTime`, flat, no volume, off the grid). `fold_now_point`
  moves the last bar with it when it falls inside it, or right at its end
  outside the session (the closing print: the last bar ends at the
  official close, like the quote), and drops it anywhere else (kept, it
  also hid a skipped session from `OwnBar`, which then never built a
  bar). Yahoo's weekly and monthly bars do the same with today's whole
  day (a bar of its own, the week ending yesterday), so weeks and months
  are built from daily bars for every Yahoo symbol. Before Nov 2026 it was kept as a bar; the cache drops those once
  (meta `yahoo_now_points`).
- **Yahoo's hourly bars are adjusted by the listed split ratio only**:
  Bajaj Finance's 16 Jun 2025 hourly bars jump ×5 (daily ones are fine).
  `Yahoo::chart` asks `events=split` and runs `split_adjust`, which takes
  the listed ratio when the gap matches it, else a clean gap (a whole
  number or half, ≥ 2×, within 3%) on the split date. Cache meta
  `bars_version` 2 cleared every cached bar once.
- **Custom indices chain-link** (`indices::chain`, `IndexDef::chain` =
  `CHAIN`; older saves are rebuilt on load): the first segment holds the
  members whose prices start by t0 (the oldest first bar), and each
  other member joins in a new segment from the first
  common day after its first close (weights set at that close, level
  kept). `IndexDef::off` (tick box) leaves a member out. Any toggle =
  `IndexDef::rebuild` (segments cleared, rebuilt by `settle`), so past
  rebalances are rebuilt with today's shares. While any index has no
  segments, `poll` fetches members even with the tab shut (else its chart
  says "no longer exists"). Board: History column (`span_label`, "+"
  when `BARS` = 10,000 cut it short). Agent: `custom_index`
  `include`/`exclude`. **The per-member "Index starts" choice is gone**
  (Nov 2026, after 0.2.8; the maintainer disliked every label for it):
  every index reaches back to its oldest member, as NSE's do. Saves with
  the old `start` list are read once, cleared and rebuilt (`migrate`);
  `start` is never written. Don't bring the choice back unless asked.
- **Index math (audited Nov 2026):** new weights start on `start` and
  are set at `link`'s anchor = the last day before it the old weights
  valued (never a day lost or doubled); a member with no bar on a day
  another traded counts at its last close (`index_series` and the hub's
  `expr::combine_segs` alike; plain formulas still join strictly); index
  ratios weight each company by the slice held (`units ÷ shares`), with
  earnings = cap ÷ P/E (net income for loss-makers); 1M/3M are calendar
  months back from the last close.
- **Yahoo statement figures are in `financialCurrency`** (Infosys: USD)
  while caps are in the listing's; `fundamentals::parse` converts at the
  rate the P/E implies (no positive P/E: left out) and recomputes
  EV/EBITDA (Yahoo's mixes the two: 976 for Infosys).
- **Market overview sector change** = change of the sector's total cap:
  weights are caps at the period's start (cap ÷ (1 + change)),
  `overview::weighted`.

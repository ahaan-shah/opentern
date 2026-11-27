# Notes: strategies, Strategy Report, paper trading, quant tools

Moved out of `CLAUDE.md` (Nov 2026) to keep the briefing short. Keep it current: update it in the same commit as the code.

## Phase 7 (done): strategies, Strategy Report, paper trading
- Pine `strategy.*` (`natives/strategy.rs`) drives `opentern_engine::Broker`; a Pine MA
  cross equals the same logic driven straight at the broker, trade for trade. Broker:
  trailing stops, OCA cancel/reduce, exit `qty_percent` of the entered size,
  `allow_entry_in`, running max drawdown/run-up, and **pyramiding fixed to TradingView's
  rule** (0 and 1 = one entry). `docs/metrics.md` records the fill model.
- With `calc_on_order_fills`, Pine resumes the simulated OHLC path after
  each fill group. Fills at one tick are grouped, and a reversal exposes
  the closing and opening legs separately. The ordinary closing calculation
  runs after the path's callbacks. Rollback keeps the broker and `varip`
  state, and each bar commits history and equity once. TradingView cases
  awaiting external confirmation are listed in `crates/pine/tests/oracles/order_fills/`.
  Cost: the committed state is saved only on a bar that can run more than
  once (a fill on the path, or `process_orders_on_close`, `immediately` or
  `strategy.risk.*`, which `Program::fills_at_close` finds at compile time).
  The saved frame leaves out the histories (they change only at commit), heap
  objects are `Arc`s copied on write, and a rollback puts back only the places
  marked changed. Recalculations reuse places free when the bar began; before
  that the object table grew every bar (a label strategy took 23 s on 20,000
  bars).
- `app/src/strategy.rs`: strategy runs are rebuilt only when `data_version`, `changed` or
  a Python result (`ScriptRun::out_seq`) changes; Python strategies replay their
  `intents` through a broker. Strategy Report tab: Overview (KPI cards, compact on short
  panels; the equity chart), Performance, Trades
  analysis, Risk/performance ratios, List of trades (virtualised rows, Export → CSV in
  `<workspace>/exports/`), Properties. Short tab names under 980 px.
- The Overview's equity chart (`app/src/equity.rs`, Nov 2026, from PR #9 redone):
  `Curves` per run (P&L, buy & hold from the first bar's open as the report's
  figure, drawdown from the bar-close peak as Max equity drawdown, the worst
  drawdown's peak/low/recovery, trades by exit bar). The legend toggles each line;
  Regular (P&L from 0) / Percent, and Log (`ln(capital + P&L)`, or of 100% + %;
  off while equity reaches zero). Drag to zoom, wheel zooms, horizontal scroll pans,
  double-click or the magnifier shows all (`equity::View`, in `Strategies`, for the
  app's run, reset for another strategy). While zoomed a row gives the range's net
  profit, Max equity drawdown and trades closed in it. A click scrolls the price
  chart to that bar (`App::show_bar`). The worst drawdown, peak to recovery, is a
  deeper shade of the drawdown colour in its strip. The tooltip lists the lines, the worst
  drawdown when inside it, and up to three trades that closed on the bar. Colours
  are in Strategy settings, the same for every strategy (`report_colors`).
- Chart: `Chart::marks` (fills as arrows with "Long +1" labels, dashed entry→exit links,
  the open position's average price) and `Chart::paper` (paper position solid, orders
  dashed, with tags).
- Paper trading: `opentern_engine::paper::Account` (market/limit/stop/stop-limit/
  trailing, brackets as OCO, netting, commission), fed by the chart's last bar or quotes
  (paper symbols are added to the quote subscription), saved as JSON under the settings
  key `paper_account`. Tab: ticket, summary, Positions/Orders/History, Reset account.
  **Since Oct 2026:** several accounts (settings key `paper_accounts` = every
  account + which is in use; the old key migrates); every account's orders keep
  working; the account menu switches, makes, deletes, and opens Account
  settings (`dialogs/paper_account.rs`: name, starting capital, currency,
  commission %, slippage %, leverage; also used by New account;
  `Account::set_initial` keeps realised P&L; `reset` keeps settings; still
  `paper-account`). The engine (`paper.rs` module doc): slippage on market/
  stop/trailing fills; buying power = (equity − margin) × leverage, orders
  past it refused (triggered ones cancelled with a note in
  `Account::notices`, which the app shows); margin call below
  `MAINTENANCE` (50%) of margin in use. Account currency: positions carry
  their symbol's currency (`App::currency_of`, else `expr::guess_currency`);
  `feed_rates` puts `FX:<CCY><ACCT>` quotes into `Account::fx` (stablecoins
  = USD); P&L converts at fill time; no rate = order refused. The currency
  is locked once an account has traded. Agent: `paper_trading`
  create_account / account_settings / switch_account / reset_account /
  delete_account. Chart trading:
  `PaperLine::order` tags are drag handles (`Action::PaperMove` →
  `Account::modify`), the × beside a tag cancels it or closes the position.
- **TradingView parity fixes (21 Oct 2026, the maintainer saw different numbers):**
  the report used the state after running the script on the *forming* bar
  (TradingView runs strategies once per closed bar; now `strategy::closed_broker`
  + `last_bar_forming`, also in the Lab); cash/%-of-equity orders are sized when
  placed at that bar's close and floored to `Props::min_qty` (`min_qty_for`:
  1 for stocks/funds/indices/futures, fractional for crypto/forex, unverified);
  v6 margin (default 100, v5 0; `pine_version` goes into the props) refuses
  unaffordable entries and runs TradingView's margin-call formula; v6
  `strategy.exit` takes the nearer of `limit`/`profit`, `stop`/`loss`. The
  manual's margin example is a test. Then, from the maintainer's own report
  (BTCUSDT 1D, matched to the cent): %-of-equity sizing leaves room for the
  entry commission; crypto rounds to the exchange's lot step (`Symbol::min_qty`,
  Binance `LOT_SIZE`, sent as `Event::Lot` when a chart opens; Pine
  `syminfo.mincontract`); max drawdown runs from closed-trade equity peaks to a
  trade's worst point incl. entry commission, % of the peak. Details in
  `docs/metrics.md`. TradingView's legend shows a strategy's values at the
  last *closed* bar (it doesn't run on the forming one); ours show the forming
  bar's, so legend numbers differ slightly while a bar is open.
- **Gotcha:** bottom-panel tab bodies render in `ui.new_child` with a clip rect; content
  taller than the panel used to grow it and push the tab strip under the range bar.
- **DOM widget** (Oct 2026, `app/src/dom.rs`, rail icon "Depth of market"):
  `Request::Depth` → `opentern_data::depth::book` (Binance, Bybit, OKX, Coinbase,
  Kraken public REST; verified live; demo book made up), polled 1/s while open.
  Ladder Buy | Bid | Price | Ask | Sell; a click places a paper limit (near side)
  or stop (far side); working orders show on their rows. Stocks/forex: plain
  "no free order book" message.

## Phase 12 (done): quant tools
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

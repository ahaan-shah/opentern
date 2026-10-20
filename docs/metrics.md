# Strategy Report metrics

How `crates/engine/src/report.rs` computes each TradingView metric. "Verify" marks
formulas written from memory or a reasonable reading of TradingView's help that still
need checking against a TradingView Strategy Report export. Money is in the account
currency; percentages of initial capital unless stated.

## Fill model (broker.rs)
- The strategy runs once per **closed** bar (TradingView's default, no
  `calc_on_every_tick`). While the chart's last bar is still forming, the report uses
  the state after the last closed bar; orders placed there fill on the forming bar
  (`strategy::closed_broker`).
- Orders placed on a bar's close fill on the **next bar's open** (market), or at the
  same close with `process_orders_on_close` / `immediately`.
- Stops and limits fill along the intrabar path: open → the nearer of high/low → the
  other → close. A gap through the level fills at the open.
- Entries in the opposite direction reverse the position; `pyramiding` is the most
  same-direction entries (0 and 1 both mean one, as TradingView's manual says); exits
  close FIFO; `strategy.order` nets against the position. An entry or order that triggers
  but is turned away by pyramiding is used up.
- `strategy.exit` with both `limit` and `profit` (or `stop` and `loss`): Pine v6 uses
  the level price reaches first, v5 the absolute one (v6 migration guide).
- `strategy.exit`: `qty_percent` is of the size entered (two 50% exits close the whole
  position); an explicit `qty` wins. Targets, stops and trailing stops share the
  intrabar path; when several trigger in one segment, the one nearest the segment start
  fills first. A trailing stop activates `trail_points` ticks into profit (or at
  `trail_price`) and then trails the best price by `trail_offset` ticks — verify.
- OCA groups: `strategy.oca.cancel` cancels the group's other orders when one fills;
  `reduce` lowers their quantity by the filled amount.
- `strategy.risk.allow_entry_in`: an entry the other way closes the position instead.
  The other `strategy.risk.*` rules are accepted but not enforced yet.
- Slippage: ticks × mintick against market and stop fills. Commission per fill:
  percent of notional, cash per contract, or cash per order.
- Qty: fixed, cash / price, or percent of equity, **worked out when the order is
  placed, at that bar's close**, and rounded down to `min_qty`: the symbol's lot step
  (Binance's `LOT_SIZE` step, e.g. BTCUSDT 0.00001 — what TradingView uses), else 1 for
  stocks, funds, indices and futures and fractional for other crypto and forex.
  Percent of equity leaves room for the entry commission: qty × price + commission =
  equity × % (percent commission: equity × % ÷ (price × (1 + c%))). Verified to the cent
  against a TradingView report (BINANCE:BTCUSDT 1D, 2017–2026, 40 trades, 100% equity,
  0.1% commission, orders on close). Cash sizing and the per-contract/per-order commission
  forms are unverified. This matches
  the Pine manual's margin example: 300% of 1,000,000 at a 4.396 close = 682,438 shares,
  filled at 4.43 (a test reproduces it).
- Margin (`margin_long`/`margin_short`; default 100 in v6, 0 in v5 — v6 migration
  guide): an entry whose margin (qty × price × margin%) exceeds the available funds
  (equity − margin of the open position) isn't opened. **Margin calls** are checked at
  each point of the intrabar path, after the orders price passed on the way: when funds
  go negative, `TRUNCATE(funds / margin% / price)` × 4 is liquidated at that price
  ("Margin call"). The manual's example (111,052 shares at 3.9) is a test.

## Performance
| Metric | Formula |
|---|---|
| Net profit | Σ trade P&L after commission |
| Gross profit / loss | Σ positive / |Σ negative| trade P&L |
| Profit factor | gross profit / gross loss |
| Commission paid | Σ entry and exit commission of closed trades |
| Expected payoff (avg trade) | net profit / closed trades |
| Open P&L | open position marked at the last close |
| Buy & hold P&L | initial capital × (last close / first bar's open − 1) — verify basis |
| Strategy outperformance | net profit − buy & hold P&L |
| Sharpe | (mean monthly return − 2%/12) / σ(monthly returns), population σ — verify |
| Sortino | (mean monthly return − 2%/12) / downside deviation vs the risk-free rate — verify |

## Trades analysis (All / Long / Short)
Total, winners (P&L > 0), losers (< 0), even; percent profitable; average P&L, win, loss;
avg win / avg loss; largest win/loss and as % of gross profit/loss; average bars in all
trades, winners, losers (exit bar − entry bar); max contracts held.

## Capital efficiency
| Metric | Formula |
|---|---|
| CAGR | (final equity / initial)^(365 / days) − 1 |
| Return on initial capital | net profit / initial capital |
| Account size required | max intrabar drawdown + max margin used (capped at capital) — verify |
| Return on account size required | net profit / account size required |
| Net profit as % of largest loss | net profit / |largest loss| |
| Avg / max margin used | |position| × close × margin% on bars in the market |
| Margin efficiency | net profit / max margin used — verify |
| Margin calls | count of margin-call liquidations |

## Run-ups and drawdowns
- **Max drawdown (the headline figure, "intrabar")**: from the peak of *closed-trade*
  equity to a trade's worst point while open, its entry commission counted; % of that
  peak (not of initial capital). Verified to the cent against TradingView (45,898.94,
  46.98%). Max run-up is the mirror image (trough of closed equity to a trade's best
  point) — verify.
- Close-to-close: on the bar-close equity curve; a drawdown runs from a peak until
  equity regains it. Average drawdown and its average length in bars.
- Close-to-close percentages are of the peak (trough) they start from — verify.
- Return of max drawdown: net profit / max close-to-close drawdown.
- Liquidated volume: total and largest quantity liquidated by margin calls.

## Per trade
Run-up and drawdown: best and worst price while open (including the exit price) ×
quantity, in money.

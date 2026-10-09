# Strategy Report metrics

How `crates/engine/src/report.rs` computes each TradingView metric. "Verify" marks
formulas written from memory or a reasonable reading of TradingView's help that still
need checking against a TradingView Strategy Report export. Money is in the account
currency; percentages of initial capital unless stated.

## Fill model (broker.rs)
- Orders placed on a bar's close fill on the **next bar's open** (market), or at the
  same close with `process_orders_on_close` / `immediately`.
- Stops and limits fill along the intrabar path: open → the nearer of high/low → the
  other → close. A gap through the level fills at the open.
- Entries in the opposite direction reverse the position; `pyramiding` caps extra
  same-direction entries; exits close FIFO; `strategy.order` nets against the position.
- Slippage: ticks × mintick against market and stop fills. Commission per fill:
  percent of notional, cash per contract, or cash per order.
- Qty: fixed, cash / price, or percent of equity / price (no lot rounding — verify for
  stocks).

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
| Margin calls | 0 (margin calls aren't simulated yet) |

## Run-ups and drawdowns
- Close-to-close: on the bar-close equity curve; a drawdown runs from a peak until
  equity regains it. Average drawdown and its average length in bars.
- Intrabar: open positions marked at each bar's high and low.
- Return of max drawdown: net profit / max close-to-close drawdown.
- Liquidated volume: 0 until margin calls are simulated.

## Per trade
Run-up and drawdown: best and worst price while open (including the exit price) ×
quantity, in money.

# TradingView oracles

Run each script in Pine v6 on historical standard candles. Leave Bar Magnifier
and On history bar tick off. Use Pine Logs and the Strategy Tester trade list to
record the symbol, timeframe, bar timestamps, and each execution's position.
The Rust regressions assert the Opentern contract on synthetic OHLC.

| Status | Script | TradingView observation to capture |
| --- | --- | --- |
| BLOCKED-ORACLE | `closing_tick.pine` | Whether a fill at the close shares the ordinary close execution and when a market order from an on-close fill callback can execute. Compare equity during a next-open callback with the bar's close and fill price. |
| BLOCKED-ORACLE | `shared_tick.pine` | Whether a callback interleaves two already eligible orders at the same price. In reversal mode, count the executions and observe whether a flat intermediate position is visible. |
| BLOCKED-ORACLE | `conditional_state.pine` | Whether a conditional varip local visited only before close contributes history, how a new UDT's ordinary field rolls back, and how a var and a varip reference to the same array share mutations. |
| BLOCKED-ORACLE | `path.pine` | Whether limit crossings between vertices yield an execution before the next vertex and whether a successor misses a consumed extreme. |

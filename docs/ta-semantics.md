# Pine semantics of the `ta` primitives

How each primitive in `crates/series/src/ta.rs` behaves at the edges, written
from TradingView's Pine v6 Reference Manual. **Verified** means a TradingView
export confirmed it; until then each line is "as documented / as remembered".
`na` is `NaN`.

| Primitive | Warm-up | `na` handling | Notes | Verified |
|---|---|---|---|---|
| `math.sum(x, n)` | `na` until n values | any `na` in the window → `na` | | no |
| `ta.sma(x, n)` | `na` until n values | any `na` in window → `na` | | no |
| `ta.ema(x, n)` | first value = SMA of first n | `na` input → `na`, then re-seeds | alpha = 2/(n+1). **Seeding with the SMA is the biggest open question**: Pine's reference example seeds with the first value, TradingView's built-in is believed to seed with the SMA. Check first. | no |
| `ta.rma(x, n)` | first value = SMA of first n | as EMA | alpha = 1/n (documented SMA seed) | no |
| `ta.wma(x, n)` | n values | `na` in window → `na` | weights n…1, newest heaviest | no |
| `ta.vwma(x, n)` | n values | | sma(x·v)/sma(v) | no |
| `ta.hma(x, n)` | | | wma(2·wma(x, n/2) − wma(x, n), floor(√n)); n/2 floored | no |
| `ta.alma(x, n, off, σ)` | n values | | m = off·(n−1), s = n/σ | no |
| `ta.linreg(x, n, off)` | n values | | least squares, x = 0 oldest; value at n−1−off | no |
| `ta.stdev(x, n)` | n values | | population (biased); deviations within 1e-10 count as 0 | no |
| `ta.dev(x, n)` | n values | | mean absolute deviation from the SMA | no |
| `ta.highest/lowest(x, n)` | `na` until n values | `na` values skipped | ties: the newest wins (matters for `…bars`) | no |
| `ta.change(x, n)` | n bars | | x − x[n] | no |
| `ta.roc(x, n)` | n bars | x[n] = 0 → `na` | 100·(x − x[n])/x[n] | no |
| `ta.rsi(x, n)` | first value at bar n | | rma of gains/losses; down = 0 → 100, up = 0 → 0 (TradingView's RSI script) | no |
| `ta.tr(handle_na)` | bar 0: high − low if handle_na, else `na` | | | no |
| `ta.atr(n)` | | | rma(tr(true), n) | no |
| `ta.stoch(c, h, l, n)` | n values | range 0 → `na` | | no |
| `ta.cci(x, n)` | n values | dev 0 → `na` | (x − sma)/(0.015·dev) | no |
| `ta.mfi(x, n)` | n values | | change(x) na counts as "up" (comparisons with na are false) | no |
| `ta.dmi(di, adx)` | | | uses `ta.tr` (na on bar 0), `fixnan` on ±DI | no |
| `ta.sar(s, i, m)` | bar 0 `na` | | follows Pine's reference implementation bar for bar | no |
| `ta.supertrend(f, n)` | | | direction −1 = up; nz() on previous bands | no |
| `ta.vwap` (session) | resets each session | | bands: stdev from Σv·p² | no |
| `ta.cum(x)` | | `na` counts as 0 | | no |
| Division | | x/0 → `na` | as in Pine; the built-ins use `ta::div`, and `±inf` never reaches a plot | no |
| `ta.pivothigh/low(src, l, r)` | `l + r + 1` values | `na` neighbours don't block | left side must be strictly beaten, ties allowed on the right (**to verify**) | no |
| `ta.valuewhen` / `ta.barssince` | | `na` until the condition first holds | | no |
| `ta.change(bool)` | | | a bool: true when it differs from n bars ago | no |

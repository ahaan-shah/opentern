# Built-in indicators

Every built-in is ported from TradingView's Pine source to Rust in
`crates/series/src/builtins/`, using only the Pine-exact primitives in
`crates/series/src/ta.rs` (see [ta-semantics.md](ta-semantics.md)). This file
tracks each one. **"Verified"** means its values match a TradingView export in
the golden test. Nothing has been verified yet, because that needs exports made
on TradingView (see below).

What the automated tests already check for every built-in:
- it runs on every bar without panicking and produces values once warmed up;
- computing the forming bar on a clone (live) gives exactly the same result as
  computing that bar as history;
- fills point at real plots and levels, offsets at real inputs.

## Status

**Pine check (§6a step 4):** each built-in below with a file in `crates/pine/tests/corpus/` (63 of them) matches the same indicator written in Pine, run by `opentern-pine`, bar for bar (`cargo test -p opentern-pine --test conformance`). The TradingView-export check is still to do.

| Indicator | id | Category | Status |
|---|---|---|---|
| Accumulation/Distribution | `ad` | Volume | Ported; awaiting TradingView export |
| Arnaud Legoux Moving Average | `alma` | Moving averages | Ported; awaiting TradingView export |
| Aroon | `aroon` | Trend | Ported; awaiting TradingView export |
| Average Directional Index | `adx` | Trend | Ported; awaiting TradingView export |
| Average True Range | `atr` | Volatility | Ported; awaiting TradingView export |
| Awesome Oscillator | `ao` | Oscillators | Ported; awaiting TradingView export |
| Balance of Power | `bop` | Oscillators | Ported; awaiting TradingView export |
| Bollinger Bands | `bb` | Bands and channels | Ported; awaiting TradingView export |
| Bollinger Bands %B | `bbpb` | Bands and channels | Ported; awaiting TradingView export |
| Bollinger BandWidth | `bbw` | Bands and channels | Ported; awaiting TradingView export |
| Bull Bear Power | `bbp` | Oscillators | Ported; awaiting TradingView export |
| Chaikin Money Flow | `cmf` | Volume | Ported; awaiting TradingView export |
| Chande Momentum Oscillator | `cmo` | Oscillators | Ported; awaiting TradingView export |
| Choppiness Index | `chop` | Trend | Ported; awaiting TradingView export |
| Commodity Channel Index | `cci` | Oscillators | Ported; awaiting TradingView export |
| Coppock Curve | `coppock` | Oscillators | Ported; awaiting TradingView export |
| Detrended Price Oscillator | `dpo` | Trend | Ported; awaiting TradingView export |
| Directional Movement Index | `dmi` | Trend | Ported; awaiting TradingView export |
| Donchian Channels | `dc` | Bands and channels | Ported; awaiting TradingView export |
| Double EMA | `dema` | Moving averages | Ported; awaiting TradingView export |
| Ease of Movement | `eom` | Volume | Ported; awaiting TradingView export |
| Elder Force Index | `efi` | Oscillators | Ported; awaiting TradingView export |
| EMA Cross | `ema_cross` | Moving averages | Ported; awaiting TradingView export |
| Envelope | `env` | Bands and channels | Ported; awaiting TradingView export |
| Fisher Transform | `fisher` | Oscillators | Ported; awaiting TradingView export |
| Historical Volatility | `hv` | Volatility | Ported; awaiting TradingView export |
| Hull Moving Average | `hma` | Moving averages | Ported; awaiting TradingView export |
| Ichimoku Cloud | `ichimoku` | Trend | Ported; awaiting TradingView export |
| Keltner Channels | `kc` | Bands and channels | Ported; awaiting TradingView export |
| Know Sure Thing | `kst` | Oscillators | Ported; awaiting TradingView export |
| Least Squares Moving Average | `lsma` | Moving averages | Ported; awaiting TradingView export |
| MA Cross | `ma_cross` | Moving averages | Ported; awaiting TradingView export |
| MACD | `macd` | Oscillators | Ported; awaiting TradingView export |
| Mass Index | `mass` | Trend | Ported; awaiting TradingView export |
| McGinley Dynamic | `mcginley` | Moving averages | Ported; awaiting TradingView export |
| Momentum | `mom` | Oscillators | Ported; awaiting TradingView export |
| Money Flow Index | `mfi` | Volume | Ported; awaiting TradingView export |
| Moving Average Exponential | `ema` | Moving averages | Ported; awaiting TradingView export |
| Moving Average Simple | `sma` | Moving averages | Ported; awaiting TradingView export |
| Moving Average Weighted | `wma` | Moving averages | Ported; awaiting TradingView export |
| On Balance Volume | `obv` | Volume | Ported; awaiting TradingView export |
| Parabolic SAR | `psar` | Trend | Ported; awaiting TradingView export |
| Price Oscillator | `ppo` | Oscillators | Ported; awaiting TradingView export |
| Price Volume Trend | `pvt` | Volume | Ported; awaiting TradingView export |
| Rate Of Change | `roc` | Oscillators | Ported; awaiting TradingView export |
| Relative Strength Index | `rsi` | Oscillators | Ported; awaiting TradingView export |
| Smoothed Moving Average | `smma` | Moving averages | Ported; awaiting TradingView export |
| Standard Deviation | `stdev` | Volatility | Ported; awaiting TradingView export |
| Stochastic | `stoch` | Oscillators | Ported; awaiting TradingView export |
| Stochastic RSI | `stochrsi` | Oscillators | Ported; awaiting TradingView export |
| Supertrend | `supertrend` | Trend | Ported; awaiting TradingView export |
| Triple EMA | `tema` | Moving averages | Ported; awaiting TradingView export |
| TRIX | `trix` | Trend | Ported; awaiting TradingView export |
| True Strength Index | `tsi` | Oscillators | Ported; awaiting TradingView export |
| Ultimate Oscillator | `uo` | Oscillators | Ported; awaiting TradingView export |
| Volume | `volume` | Volume | Ported; awaiting TradingView export |
| Volume Oscillator | `vosc` | Volume | Ported; awaiting TradingView export |
| Volume Weighted Moving Average | `vwma` | Moving averages | Ported; awaiting TradingView export |
| Vortex Indicator | `vortex` | Trend | Ported; awaiting TradingView export |
| VWAP | `vwap` | Trend | Ported; awaiting TradingView export |
| Williams %R | `willr` | Oscillators | Ported; awaiting TradingView export |
| Williams Alligator | `alligator` | Trend | Ported; awaiting TradingView export |

Still to port from TradingView's list (Phase 9b): 24-hour Volume, Advance/Decline,
Auto Fib, Average Day Range, Bollinger Bars, Chaikin Oscillator, Chande Kroll Stop,
Chop Zone, Connors RSI, Correlation Coefficient, Cumulative Volume Delta/Index,
Klinger Oscillator, Linear Regression Channel, MA Ribbon, Majority Rule, Median,
Moon Phases, Net Volume, Open Interest, Pivot Points (Standard, High Low), Rank
Correlation Index, Relative Vigor Index, Relative Volatility Index, RCI Ribbon,
Seasonality, SMI Ergodic, Standard Error (+ Bands), Stochastic Momentum Index,
Technical Ratings, TWAP, Trading Sessions, Trend Strength Index, Up/Down Volume,
Visible Average Price, Volatility Stop, Volume Profile (all kinds), Anchored VWAP,
Williams Fractals, Woodies CCI, Zig Zag.

## Defaults to double-check against TradingView

Inputs, colours and levels were written from TradingView's built-in scripts as
remembered, not copied from them. When you make the exports, also check these
(they're the ones most likely to differ):
- RSI: is the "RSI-based MA" on by default (SMA 14), and its colour?
- Volume: up/down colours and transparency; whether colouring uses previous close.
- HMA: half length rounding (`length / 2` is floored here).
- DPO: `barsback = length / 2 + 1` with integer division here.
- MA Cross / EMA Cross colours; ADX colour; Bull Bear Power plot style.

## Making a golden test (TradingView export)

1. On TradingView, open a chart of a public symbol (e.g. BINANCE:BTCUSDT 1D or
   4h) and add **only** the indicator, with the inputs you want to check.
2. Chart menu → **Export chart data…** → Time format "UNIX timestamp" → Export.
3. Save it as `crates/series/tests/golden/<name>.csv`, or, if it shouldn't be
   committed, in a folder outside the repo pointed to by
   `OPENTERN_PRIVATE_CORPUS`.
4. Next to it, write `<name>.json`:
   ```json
   {"study": "rsi", "interval": "1D", "inputs": {"length": 14}, "skip": 300,
    "columns": {"RSI": "RSI", "RSI-based MA": "RSI-based MA"}}
   ```
   `columns` maps each plot title (left) to the CSV column header (right).
5. `cargo test -p opentern-series --test golden -- --nocapture` compares every
   bar (relative 1e-6, and `na` must match `na`).

The first bars of an export start mid-history (TradingView computes from
earlier data you don't have), so for recursive indicators (EMA, RSI, ATR…) the
early values legitimately differ until the warm-up washes out. Export at least a
few hundred bars more than the indicator's length, and set `"skip"` to the
number of warm-up rows to ignore.

## Community scripts (licence-checked)

None yet. Only MPL-2.0 (or more permissive) open-source TradingView scripts may
be ported; record author, URL, licence and version here, and keep the port's
file under MPL-2.0 with the credit in its header. See CLAUDE.md §6a.

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
| 24-hour Volume | `volume_24h` | Volume | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Average Day Range | `adr` | Volatility | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Chaikin Oscillator | `chaikin_osc` | Volume | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Chande Kroll Stop | `ckstop` | Trend | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Chop Zone | `chop_zone` | Trend | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Connors RSI | `crsi` | Oscillators | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Klinger Oscillator | `kvo` | Volume | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Moon Phases | `moon` | Trend | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Moving Average Ribbon | `ma_ribbon` | Moving averages | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Net Volume | `net_volume` | Volume | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Pivot Points High Low | `pivots_hl` | Trend | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Rank Correlation Index | `rci` | Oscillators | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| RCI Ribbon | `rci_ribbon` | Oscillators | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Relative Vigor Index | `rvgi` | Oscillators | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Relative Volatility Index | `rvi` | Volatility | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| SMI Ergodic Indicator | `smi_ergodic` | Oscillators | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| SMI Ergodic Oscillator | `smi_ergodic_osc` | Oscillators | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Standard Error | `stderr` | Volatility | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Standard Error Bands | `stderr_bands` | Volatility | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Stochastic Momentum Index | `smi` | Oscillators | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Time Weighted Average Price | `twap` | Trend | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Trading Sessions | `sessions` | Trend | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Trend Strength Index | `trend_strength` | Trend | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Volatility Stop | `volatility_stop` | Volatility | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Williams Fractals | `fractals` | Trend | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Woodies CCI | `woodies_cci` | Oscillators | Ported (wave 2); Pine twin matches; awaiting TradingView export |
| Zig Zag | `zigzag` | Trend | Ported (Phase 10, draws); pivot rule to verify against an export |
| Auto Fib Retracement | `auto_fib` | Trend | Ported (Phase 10, draws); uses Zig Zag's last leg; to verify |
| Auto Fib Extension | `auto_fib_ext` | Trend | Ported (Oct 2026, draws); the last three Zig Zag pivots: 0 at C, 1 at C + (B − A); to verify |
| Median | `median` | Bands | Ported (Oct 2026); Pine twin matches (colours too); written from memory of TradingView's script, to verify |
| Majority Rule | `majority_rule` | Oscillators | Ported (Oct 2026); Pine twin matches; % of the last 14 bars whose close rose, to verify |
| Pivot Points Standard | `pivots_std` | Trend | Ported (Phase 10, draws); all six types; to verify |
| Linear Regression Channel | `linreg_channel` | Trend | Ported (Phase 10, draws) line by line from the published source; to verify |
| Volume Profile Visible Range | `vpvr` | Volume | Phase 11, drawn by the chart (`app/src/chart/profile.rs`); to verify |
| Session Volume Profile | `vpsv` | Volume | Phase 11, drawn by the chart; to verify |
| Visible Average Price | `vap` | Trend | Phase 11, drawn by the chart (mean close of visible bars); to verify |
| Seasonality | `seasonality` | Trend | Phase 11, its own pane, % from the prior year's last close; to verify |
| Fixed Range / Anchored Volume Profile | drawing tools | Volume | Phase 11, drawing tools (Forecasting group); 24 rows, 70% value area |
| Anchored VWAP | drawing tool | Volume | Phase 11, drawing tool, hlc3 VWAP from the anchor bar |

Notes on wave 2: Williams Fractals uses the strict rule (equal highs/lows on either side
don't count; TradingView's script tolerates some ties). Standard Error is the standard
error of the least-squares fit over the window; the bands are that fit ± multiplier ×
error, smoothed. ADR on intraday charts uses completed days (as `request.security` with
lookahead off does on history). Moon Phases follows Meeus (minutes accurate). Trading
Sessions shades Tokyo, London and New York regular hours.

Still to port:
- Need data Opentern doesn't fetch yet: Advance/Decline Line and Ratio (market
  breadth), Cumulative Volume Index (breadth), Open Interest (futures), Correlation
  Coefficient (Pine scripts can now do it with `ta.correlation` and `request.security`
  on another symbol, which fetches on demand; a Rust built-in needs the same feeds),
  Cumulative Volume Delta and Up/Down Volume (intrabar volume).
- Not yet, to get exactly right: Technical Ratings (26 sub-ratings whose exact
  buy/sell rules need TradingView's source) and Bollinger Bars (how it colours and
  draws its bars). Both wait for the script source or an export.

## Defaults to double-check against TradingView

Inputs, colours and levels were written from TradingView's built-in scripts as
remembered, not copied from them. When you make the exports, also check these
(they're the ones most likely to differ):
- RSI: is the "RSI-based MA" on by default (SMA 14), and its colour?
- Volume: up/down colours and transparency; whether colouring uses previous close.
- HMA: half length rounding (`length / 2` is floored here).
- DPO: `barsback = length / 2 + 1` with integer division here.
- MA Cross / EMA Cross colours; ADX colour; Bull Bear Power plot style.
- Zig Zag: a pivot needs `depth / 2` bars on each side here (is "Pivot legs" the
  per-side count instead?); the label layout (price, change, volume).
- Auto Fib: which end is level 0, default extend right, which levels are on.
  Extension: is level 0 at C, and does "Reverse" flip the projection?
- Median: plots (upper/median/lower coloured by median vs its EMA, a fill), the
  default source hl2, lengths 3 and 14, multiplier 2, and the colours.
- Majority Rule: the default length (14 here) and whether a level is drawn at 50.
- Volume profiles: without lower-timeframe data each bar's volume is spread evenly
  over the rows its high–low covers (TradingView uses intrabar data); colours
  (up #2962FF / down #FBC02D at 30%, value area 70%, POC red) and Session VP's
  70% width are from memory.
- Visible Average Price: mean of visible closes here (or is it hlc3 / volume-weighted?).
- Seasonality: base is the prior year's last close; default 3 years.
- Pivot Points Standard: the Auto timeframe rule (≤15m → D, intraday → W, D → M, else
  12M), one colour for every level, periods in UTC (stocks should use the session).

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

Bundled Pine libraries (`crates/pine/libs/`, used through `import`, unchanged
apart from line endings):

| Library | Author | URL | Licence | Version |
|---|---|---|---|---|
| `TradingView/ta/9` | TradingView | https://www.tradingview.com/script/BICzyhq0-ta/ | MPL-2.0 | 9 (2024-11-26) |
| `TradingView/RelativeValue/3` (imported by ta) | TradingView | https://www.tradingview.com/script/cZnSLls2-RelativeValue/ | MPL-2.0 | 3 (2024-11-26) |

No indicator ports yet. Only MPL-2.0 (or more permissive) open-source TradingView scripts may
be ported; record author, URL, licence and version here, and keep the port's
file under MPL-2.0 with the credit in its header. See CLAUDE.md §6a.

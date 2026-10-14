# Scripting in Luau, JavaScript and TypeScript

Opentern runs indicators and strategies written in Pine Script®, Python, **Luau**,
**JavaScript** and **TypeScript**. Put a `.luau`, `.js` or `.ts` file in the scripts
folder (Script Editor → New script) and add it to the chart. It reloads whenever the
file is saved, here or in your own editor.

Luau, JavaScript and TypeScript run inside the app, sandboxed. They get no file,
network or module access, memory is capped at 256 MB, and each bar has a one-second
time budget. A script that loops forever stops with an error and a line number
instead of freezing the app. All three share one runtime, so the same script gives
the same numbers in any of them. Where they overlap with Pine, they give Pine's
numbers too, because `ta.*` is the same Rust code the built-in indicators use.

## How a script runs

1. **The top level runs once.** Declare the script there: `indicator(...)` or
   `strategy(...)`, the inputs, and any `hline(...)` levels.
2. **`on_bar(bar)` runs on every bar** (in JavaScript and TypeScript it's
   `onBar(bar)`), oldest first. On the live bar it runs again on every tick.
3. **`ta.*` calls keep their state by call order.** The third `ta` call on a bar
   continues where the third call left off on the previous bar. Call them on every
   bar, outside `if` blocks: store the result in a local and use it in the `if`.
   This is Pine's rule too.
4. **`plot(...)` works the same way.** The k-th `plot` call feeds the k-th plot.
   Plot `NaN` to show nothing on a bar.
5. **Keep values that carry over to the next bar in `state`.** The app snapshots
   `state` after every closed bar, so changes made while the live bar ticks are
   thrown away on the next tick (Pine's `var` and rollback). Other globals are not
   rolled back.

## The `bar` argument

`bar.time` (ms, UTC), `bar.open`, `bar.high`, `bar.low`, `bar.close`, `bar.volume`,
`bar.hl2`, `bar.hlc3`, `bar.ohlc4`, `bar.hlcc4`, `bar.index`, `bar.is_last`,
`bar.is_forming` and `bar.is_new_session`. `bar[src]` reads the value an
`input.source` picked.

The same table is refilled on every bar, so copy the numbers you want to keep. For
past bars use `close(n)`, `open(n)`, `high(n)`, `low(n)`, `volume(n)` and `time(n)`,
up to 5,000 bars back.

## Declarations (top level only)

| Function | Notes |
|---|---|
| `indicator(title, { overlay, shorttitle, precision })` | |
| `strategy(title, { overlay, initial_capital, default_qty_type, default_qty_value, pyramiding, commission_type, commission_value, slippage, process_orders_on_close, margin_long, margin_short })` | `default_qty_type` is `"fixed"`, `"cash"` or `"percent_of_equity"`. The Strategy Report's settings can override any of these |
| `input.int(title, default, { min, max, group })` | Returns the value set in the settings dialog |
| `input.float(title, default, { min, max, step, group })` | |
| `input.bool(title, default)` | |
| `input.string(title, default, { options = {...} })` | With `options` it shows a dropdown |
| `input.source(title, "close")` | Returns a name. Use `bar[name]` |
| `input.color(title, "#2962FF")` | |
| `hline(title, price, { color, style })` | `style` is `"solid"`, `"dashed"` or `"dotted"` |

## On every bar

| Function | Returns |
|---|---|
| `plot(value, title, { color, style, linewidth })` | `style`: `line`, `linebr`, `stepline`, `histogram`, `columns`, `area`, `circles` or `cross`. Title and style come from the first bars; `color` may change per bar |
| `bgcolor(color)`, `barcolor(color)` | |
| `alert(message)` | Pick "any alert() call" in the alert dialog |
| `log(...)` (also `print` in Luau) | Shown in the Script Editor's console |
| `na(x)`, `nz(x, replacement)` | `NaN` is "no value" |
| `close(n)`, `open(n)`, `high(n)`, `low(n)`, `volume(n)`, `time(n)` | `n` bars ago |

### `ta.*`

`sma ema rma wma vwma hma alma linreg stdev dev highest lowest change mom roc rsi cci
cum mfi vwap fixnan` take `(source, length…)`. Also:

- `tr()`, `atr(length)`, `sar(start, inc, max)`
- `stoch(source, high, low, length)`
- `crossover(a, b)`, `crossunder(a, b)`, `cross(a, b)`, which return true or false
- `prev(x, n)`: `x` from `n` bars ago, tracked per call

Several return more than one value. In Luau they come back as separate values
(`local m, s, h = ta.macd(...)`); in JavaScript and TypeScript, as an array
(`const [m, s, h] = ta.macd(...)`):

- `macd(src, fast, slow, signal)` → macd, signal, histogram
- `bb(src, length, mult)` → middle, upper, lower
- `dmi(length, smoothing)` → +DI, −DI, ADX
- `supertrend(factor, atrPeriod)` → value, direction

### `strategy.*` (strategies only)

`entry(id, "long" | "short", { qty, limit, stop, comment, oca_name, oca_type })`,
`order(...)` (same arguments),
`exit(id, from_entry, { qty, qty_percent, profit, loss, limit, stop, trail_points, trail_offset, trail_price, comment })`,
`close(id, { comment, qty, qty_percent, immediately })`, `close_all()`, `cancel(id)`
and `cancel_all()`. These read the position: `position_size()`,
`position_avg_price()`, `equity()`, `openprofit()`, `netprofit()`,
`initial_capital()`, `opentrades()`, `closedtrades()`, `wintrades()` and
`losstrades()`.

`strategy.long` and `strategy.short` are the strings `"long"` and `"short"`.
Orders fill the way TradingView's broker emulator fills them, the same as for Pine
strategies.

### Colours and symbol info

`color.red`, `color.blue` and the rest of TradingView's palette, `color.new(c,
transparency)` and `color.rgb(r, g, b, transparency)`. Colours are strings like
`"#2962FF"` or `"#2962FF80"`. `syminfo.ticker`, `tickerid`, `mintick`, `currency`,
`type` and `timezone`.

## Examples

Luau:

```lua
strategy("MA cross", { initial_capital = 10000 })
local fast = input.int("Fast", 9, { min = 1 })
local slow = input.int("Slow", 21, { min = 1 })

function on_bar(bar)
  local f = ta.sma(bar.close, fast)
  local s = ta.sma(bar.close, slow)
  local up, down = ta.crossover(f, s), ta.crossunder(f, s)
  plot(f, "Fast", { color = color.blue })
  plot(s, "Slow", { color = color.orange })
  if up then strategy.entry("Long", strategy.long) end
  if down then strategy.close("Long") end
end
```

TypeScript:

```ts
indicator("Streak");
function onBar(bar: { close: number; open: number }): void {
  state.streak = bar.close > bar.open ? (state.streak ?? 0) + 1 : 0;
  plot(state.streak, "Up bars in a row", { style: "columns" });
}
```

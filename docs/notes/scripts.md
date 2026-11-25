# Notes: Python, Pine, Luau/JS/TS and feeds

Moved out of `CLAUDE.md` (Nov 2026) to keep the briefing short. Keep it current: update it in the same commit as the code.

## Phase 5 (done): scripting + Python
- `python/opentern`: `indicator()`, `strategy()` (+ `strategy.entry/order/exit/close/
  close_all` with per-bar `when` series), `input.int/float/bool/string/source`,
  `plot`, `hline`, `Bars` (`.close`, `["hl2"]`, `.df()` with pandas), and `ta.*` in pure
  Python with Pine semantics (no numpy needed; numpy/pandas values accepted).
- Scripts live in `<workspace>/scripts/`; the Script Editor tab lists them, creates
  Python/Pine from templates, highlights, saves (Ctrl+S), adds to chart, opens in your
  own editor (the chart reloads on save), shows errors and prints. Errors also show in
  the study's legend row.
- Python runs re-trigger on new data at most twice a second, immediately on new
  inputs. The worker polls the file's mtime every second (the `notify`
  crate is only used for the Python notice, `pywatch.rs`).
- Verified: the SDK's RSI (pure Python) equals the Rust built-in RSI bar for bar on the
  demo chart (`drive::tests::python_script_…`).

## Phase 6 (done): Pine v1
- Pine steps 1–2: v5/v6 syntax incl. `var`/`varip`, tuples, functions, `if`/`switch`
  values, `for`/`while`/`break`/`continue`, `a = 1, b = 2`, history `[n]` on anything;
  `indicator`/`strategy` declarations, every `input.*` (colour inputs got `InputKind::Color`
  and a picker in the Inputs tab), `plot`/`plotshape`/`plotchar`/`plotarrow`/`plotcandle`/
  `plotbar` (new `PlotStyle::Candle`/`OhlcBar`/`Hidden`), `hline`, `fill`, `bgcolor`,
  `barcolor`, `alertcondition`, `alert`, `log.*`; `ta.*` (all common ones), `math.*`,
  `str.*`, `color.*`, time functions, `timeframe.*`, `syminfo.*`, `barstate.*`.
- Execution model: every variable is a frame slot; slots read with `[n]` keep a ring that
  is pushed at the end of each bar in which the slot was set (so a function's locals only
  get history on bars it ran, as in Pine); each user-function call site and each `ta.*`
  call site has its own state; v6 `and`/`or` are lazy, v5 aren't; ternaries/`if` only run
  the chosen branch. State is `Clone`: the forming bar runs on a copy (rollback).
  `Instance::latest_state()` gives the forming-bar state (errors, later drawings).
- **Conformance: 63 TradingView built-ins written in Pine (`tests/corpus/`) match the
  Rust built-ins bar for bar** on 1h and 1D synthetic data with gaps, flat bars and zero
  volume (`tests/conformance.rs`). This found two built-in bugs, now fixed: raw `/` where
  Pine gives `na` (EOM on zero volume and others now use `ta::div`), and `±inf` reaching
  plots (now `na` in `Output::write`).
- App: `pine_host.rs` compiles on save, on outside edits (mtime checked once a second)
  and on a symbol change (`syminfo` is baked into the compile). A compile error keeps
  the last good version running and shows "Line N: …" in the legend; runtime errors
  too. `Chart::set_inputs` now keeps a script's factory (`Instance::with_inputs`).
- Not yet (later phases): `strategy.*` (7), arrays/maps/matrices/UDTs/methods/drawings/
  `request.security`/libraries/enums (10). Hline prices and plot offsets that depend on
  non-trivial expressions use the input defaults. Dynamic lengths rebuild `ta.*` state.
  `varip` survives ticks through `Study::carry` (plain values, not objects).

## Phase 9b (done, formula part): built-ins wave 2
- `series/src/builtins/wave2.rs`: 26 more TradingView built-ins (Chaikin Osc, Chande
  Kroll Stop, Chop Zone, Connors RSI, Klinger, MA Ribbon, Net Volume, Pivots HL, RCI,
  RCI Ribbon, RVGI, Relative Volatility Index, SMI Ergodic ×2, Standard Error (+ Bands),
  SMI, TWAP, Trend Strength Index, Volatility Stop, Williams Fractals, Woodies CCI, ADR,
  24-hour Volume, Moon Phases, Trading Sessions). 88 built-ins in all.
- 22 Pine twins added to the corpus: **85 corpus scripts match their Rust built-ins bar
  for bar** (incl. colour indices where colour carries meaning).
- Found and fixed: a plot coloured by a variable (`color = zoneColor`) didn't get the
  palette of that variable's colours (`Cx::palettes`).
- What's left of TradingView's list and why is at the end of `docs/indicators.md`.

## Phase 10 (done): Pine v2
- Collections and types: `array.*`, `map.*`, `matrix.*`, `type` (UDTs, `copy()`),
  `method`, `for [i, x] in`, `enum` + `input.enum` (a member's value is its title, which
  is what `str.tostring` and the input dropdown show). Method calls bind named arguments
  at run time against whichever method the receiver picks (`order_args` in `run.rs`).
- Drawing objects: `label`/`line`/`box`/`table`/`linefill`/`polyline` with their
  setters/getters/`copy`/`delete`/`all`, `chart.point.*` and the point overloads of
  `*.new`; `max_labels_count` etc. drop the oldest. Types live in
  `opentern_series::drawings`; any `Study` can return them via `Study::drawings()` and
  `Instance::drawings` holds the forming bar's set. The chart paints them above the
  study's plots (`app/src/chart/scripted.rs`: bubbles and marker styles, extend, arrows,
  dashed/dotted, tables at pane corners with merges, curved/closed polylines).
- `request.security` on **this chart's symbol, same or higher timeframe**
  (`natives/security.rs`): HTF bars are built from chart bars; each finished HTF bar runs
  the expression once on the call site's frame (so `ta.*` and `x[1]` step per HTF bar);
  `lookahead_off` shows the last finished HTF value (the closing chart bar shows its own
  HTF bar), the forming bar repaints; `lookahead_on` = the HTF bar so far, which equals
  TradingView for the `expr[1]` idiom; `gaps_on` = na except where a value arrives.
  Variables used inside are recomputed on the HTF from their definition (`Cx::defs`,
  inlined); ones changed with `:=`/`var` get a plain error. Not yet: other symbols, lower
  timeframes, `security_lower_tf` (each gives a clear error). Known gap: on stocks, a
  "D" bar from intraday finishes one chart bar late (periods are calendar days).
- Libraries: `library("Name")` scripts in the scripts folder are found by title
  (`opentern_pine::library_title`, passed in `Env::libraries`); `import user/Name/1 as
  x` compiles their functions, methods, types and enums under `x.`; the env key hashes
  the libraries so importers recompile when one is saved. Library-level variables aren't
  imported.
- **Bundled libraries** (Oct 2026, `crates/pine/libs/`, `src/libs.rs`): TradingView's
  `ta/9` and the `RelativeValue/3` it imports (MPL-2.0, logged in `docs/indicators.md`;
  list them in `THIRD-PARTY-LICENSES.txt` at Phase 14). An import matching the whole
  path wins over the scripts folder; other versions get "Opentern includes …".
  `import TradingView/ta/9 as ta` merges with the built-in `ta.*` (user functions
  shadow natives). Tests: every export runs, and 17 match built-ins exactly (the
  "alternate" `*2` versions and `vStop` seed differently, equal once warmed up).
  Engine gaps the libraries found, now fixed: strings wrapped onto indented lines,
  user-function overloads (picked by binding plus evident argument types), methods
  called as functions `f(x, …)`, built-in series as parameter defaults (`= hl2`),
  `security_lower_tf` with a tuple expression (a tuple of arrays), and
  `timeframe.change` is false on the first bar (as TradingView's).
- Built-ins that draw (`series/src/builtins/drawn.rs`): Zig Zag, Auto Fib Retracement,
  Pivot Points Standard (six types), Linear Regression Channel. 92 built-ins in all.
- App: a Pine script that fails its very first compile gets a placeholder legend row
  showing the error (it used to be invisible); fixing the file swaps in the real study.
  `OPENTERN_STILL=add:<id>` adds a built-in at launch (for screenshots).
- Found and fixed: `export type`/`export enum` didn't parse; named method arguments were
  treated as positional.

## Phase 13 (done): Luau, JavaScript, TypeScript
- `.luau`/`.lua`, `.js`, `.ts` in the scripts folder; the user API is in
  `docs/scripting.md`. One host (`opentern_script::host`) owns the meaning; a
  language only converts values and calls host functions by name, so all three
  agree bar for bar (tested: MACD in Luau = JS = TS; RSI = the Rust built-in).
- Execution model: top level once (declarations), then `on_bar(bar)` / `onBar(bar)`
  per bar; `ta.*` and `plot` keyed by call order (as Pine call sites); persistent
  values in a `state` table that the host snapshots after each closed bar (so the
  forming bar rolls back); plots found by a dry run on made-up bars at compile.
- Sandbox: Luau has no I/O by design; QuickJS gets no modules; both have a
  256 MB cap and a 1 s budget per bar (interrupt → plain error with the line).
  One VM per study (optimizer runs stay parallel); the forming bar's copy shares
  its study's VM and restores `state` by version number.
- App: `Lang::{Luau, Js, Ts}` go through the Pine host path (`pine_host.rs`
  `Compiled` enum, study ids `luau:`/`js:`/`ts:`), so recompile on save,
  errors in the legend, Strategy settings overrides, the Strategy Lab (incl.
  portfolio via `scripting::other_builder`), `alert()` alerts and the editor
  (templates, highlighting, New script menu) all work. No drawing objects or
  `request.security` in these languages yet.
- **Perf gotcha:** mlua with `send` locks on every API call, so per-field table
  sets are slow (~0.3 µs each). The bar goes in as 14 arguments to a Luau-side
  wrapper that fills one reused table and reports whether `state` is empty;
  `ta.*`/`plot` have typed fast paths (`HostState::ta_fast`, `plot_fast`).
  Measured on this throttled laptop: 100k bars with RSI + MACD + 2 plots ≈ 0.55 s
  Luau, 0.7 s JS (empty `on_bar` ≈ 2.5 µs/bar). `cargo test --release -p
  opentern-script --test timing -- --ignored --nocapture`.
- Licences: mlua/Luau, rquickjs/QuickJS (MIT), swc (Apache-2.0): fine for the
  MIT app; list them in `THIRD-PARTY-LICENSES.txt` at Phase 14.

## Feeds: other symbols and timeframes in Pine (Oct 2026)
- `opentern_pine::Env::feeds` (`Feeds` = shared map `"SYMBOL|tf"` → bars). A
  `request.security` on another symbol, or below the chart's timeframe, or
  `security_lower_tf`, reads its feed; a missing feed gives `na` and is listed
  in `PineStudy::wanted()`. `app/src/feeds.rs` fetches wanted feeds
  (`Request::Fetch`, seq ≥ `1 << 42`; bars to cover the chart's span), stores
  them in `App::feeds` (one store for all charts), reruns every chart's Pine
  studies, and refreshes feeds in use every minute. Symbols resolve like
  formulas (`expr::resolve`). Inputs can pick symbol and timeframe.
- A feed bar counts as finished when its period has ended by the chart bar's
  close; lookahead/gaps/forming-bar repaint follow the same rules as HTF bars.
- `run::period_end` + `calendar::last_close_in`: daily-and-longer periods of
  markets with set hours end at the last session close (fixes stocks' "D"
  finishing a bar late). Chart symbol's market comes from `Env` prefix + kind.

## Pine bug hunt (Nov 2026)
- `crates/pine/src/hunt.rs` holds a test per bug, the manual's reference formulas
  against the built-ins (v5 and v6), and runs `tests/scripts/` from history and
  live (`PINE_DIR` for another folder). Add a test there for every Pine bug.
- **`request.security` recomputes what its expression depends on**: `Cx::hoist`
  compiles the variables it uses into the request's own frame (`Sec::prelude`);
  functions that read globals, or request something made from their parameters,
  are compiled again per call (`Cx::instance`). When it depends on variables changed
  with `:=`/`var`, the top-level statements that build them run in the request first
  (`slice.rs`), and the ones below it after (`Body::Late`, compiled last). Requests
  draw, order and log nothing (`G::quiet`). Each symbol + timeframe of a call site is a context
  (`natives/security.rs`) that catches up on bars it wasn't called on.
- **Times:** daily and longer bars are stamped 00:00 UTC of their local date, but
  scripts see the session's open and close (`open_in`/`close_in` in `run.rs`);
  `PineStudy::drawings` maps `xloc.bar_time` back. Never use `bar.time` for dates.
- Windows with a series length replay their inputs (`ta::replay`); built-ins with
  a `length` are marked `.window()`.
- Not simulated: `calc_on_order_fills`, `use_bar_magnifier` (both say so in the
  script's log), `request.*` other than `security` and `ticker.*` other than
  Heikin Ashi (plain errors).
- `survey` (ignored, `PINE_DIR`, `PINE_ON=us|nse`) runs a folder of other people's
  scripts: 1,231 of 1,302 from public GitHub repos ran or loaded (Nov 2026); the rest
  are mostly invalid, use Renko, or import libraries they don't come with.
- v4 and older scripts compile: bare names get their namespace (`renamed`).
- Checked against TradingView's manual (26 Nov 2026): v6's colours and white label
  text, session strings, loop rules, `str.format`/`str.tostring` defaults. The `#`
  patterns keep a leading 0, as labels on TradingView do; the manual doesn't say.
- The same hunt in the other languages (`crates/script/tests/hunt.rs`, and the tests
  in `python.rs`): `opentern_core::bartime` is shared by Pine, Luau/JS and Python
  (`scripting::script_bars`); `host.rs` windows keep their inputs for a changed length
  (`WINDOWS`). Python replies are strict JSON: `_num` turns nan and infinity into none.

# Notes: charts, drawings, layouts and UI

Moved out of `CLAUDE.md` (Nov 2026) to keep the briefing short. Keep it current: update it in the same commit as the code.

## Phase 4 (done): drawings, layouts, watchlists
- 27 tools in TradingView's groups (lines, Fib/Gann, patterns, forecasting/measure,
  shapes, text, icons) with flyouts that remember the last tool; anchored by time and
  price (`draw::TimeMap` interpolates between bars and extrapolates past the ends).
  Magnet, stay-in-drawing mode, lock/hide all, remove menu, Eraser, Zoom (drag a range).
- Select, move (body or handles), Delete, Esc, undo/redo (Ctrl+Z/Y, top bar buttons),
  floating toolbar (colours, width, style, text, extend, labels, lock, "use this style
  for new drawings" → templates kind `drawing`, name = `Tool::key()`).
- Drawings are saved **per symbol** (table `drawings`), not per layout.
- Named layouts (top bar menu: save, rename, copy, new, open, delete; autosaved;
  `layout_id` in settings). Several watchlists (create/rename/delete/switch, add via
  search, remove, move to top, sort by column).
- Shortcuts: Alt+T/H/V/F (trend, horizontal, vertical, Fib), Delete, Esc, Ctrl+Z/Y.

## Phase 11 (done): power features
- **Chart types** (`chart/transform.rs`, `draw_series`): bars, candles, hollow,
  columns, line, line with markers, step, area, baseline (level % of pane height),
  high-low, Heikin Ashi, Renko, Line Break, Kagi, Point & figure. HA, Renko, Line
  Break, Kagi and P&F **change the bars** (studies run on them, as on TradingView):
  `Chart::display()` = raw → ADJ → transform; box size is ATR(14) by default or fixed
  (Chart settings → Symbol). Bricks formed on one bar get times 1 ms apart so the
  time↔index maps stay strictly increasing. HA ticks update incrementally; the
  others rebuild everything on each tick. Not done: range bars (need ticks).
- **Multi-chart** (`multi.rs`): 15 grids for 1–8 charts; sync symbol, interval,
  crosshair (default on), time, date range. **The active chart lives in
  `App::chart`; the others in `App::slots`, swapped into place by `with_slot`**
  (panic-safe), so every "the chart" code path works per chart. Each slot has its own
  `ChartUi`, scripts and strategy runs; inactive charts poll scripts, strategies and
  paper lines each frame. Pressing a cell activates it. Layout JSON is now
  `LayoutState { grid, sync, active, charts }` (old single-chart JSON still loads).
  Session ids come from `chart::new_session_id()` (global). Drawings saved on one
  chart reload on other charts with the same symbol. Alerts still evaluate against
  the active chart's studies only.
- **Compare/overlay** (`Chart::compares`): Compare puts the scale in percent and
  scales each symbol from the first visible bar; Overlay draws on its own fitted
  scale. As-of join on time. Legend rows with hide/remove. Saved in the layout.
- **Bar replay** (`chart/replay.rs`): Replay → click a bar → play/pause, step,
  speed (10 s … 0.03 s per bar), jump to real time. Live data goes to the hidden
  bars meanwhile; studies and strategies only see replayed bars; no history paging
  while replaying.
- **Built-ins drawn by the chart** (`series/builtins/charted.rs` metas with no
  plots, painted in `chart/profile.rs`; numbers in `series/profile.rs`): VPVR,
  Session VP, Visible Average Price, Seasonality (own pane, month axis). Drawing
  tools: Anchored VWAP, Fixed Range VP, Anchored VP. 96 built-ins in all.
- Nov 2026: Moving Average Adaptive (KAMA), Hamming and Channel (`ma.rs`): 102 built-ins.
- **Screener tab** (`screener.rs`): universe = a watchlist or a preset (crypto
  majors, US and India large caps); columns price, change, 1W/1M perf, volume, rel
  volume, RSI, vs SMA 50/200, ATR %, plus an optional script column (any built-in
  plot, or a Pine file compiled per symbol) computed on rayon. Filters above/below.
  Uses the new hub **`Request::Fetch`** (one-off bars, cache first, 4 worker
  threads; also meant for portfolio backtests and `request.security` on other
  symbols). Settings saved under the `screener` key.
- **Snapshot** (camera, Alt+S): download PNG to `<workspace>/snapshots/` or copy.
- **Widgets** (`side.rs`, `notes.rs`): Hotlists (Binance 24h tickers; Yahoo
  predefined screeners for US stocks), News (`feeds::news`, all sources at once
  on threads, merged newest first, deduped by title, ≤ 2 weeks, ≤ 50: Google
  News RSS search for every symbol (name from the description, `when:7d`,
  en-IN for NSE/BSE; personal use), Yahoo search for non-crypto, CoinDesk/
  Cointelegraph/Decrypt/The Block RSS for crypto and Economic Times/Mint
  markets RSS for India, kept only when a headline names the symbol (whole
  words), except an Indian index keeps all; CryptoCompare now needs a key),
  News has a refresh button (spins while loading); News and Hotlists
  ask again 15 s after a failure (`side::RETRY`).
  Calendar (NYSE/NSE holidays and early closes as date tiles with pills, the
  symbol's dividends and splits by year), Notifications
  (alerts, snapshots, updates; dot on the rail when unread). Hub requests
  `Hotlist` and `News` (`data/src/feeds.rs`).
- Stills: `OPENTERN_STILL` also takes `kind:<chart type>`, `grid:<Grid>`,
  `compare:<ticker>`, `tf:<interval>`, `symbol:<id>` (e.g. `NSE:INFY`), `replay`, `screen`, `widget:<name>`.

## UI pass 4 (21 Oct 2026)
- Settings is a gear at the far right of the top bar (right of the camera); the
  hamburger is gone. Indicator templates can be starred (favourites kind
  `indicator_template`); starred ones show as buttons right of the templates
  button. The drawing toolbar slides like the side and bottom panels.
- Symbol search only scrolls to the selection on arrow keys (it used to pin the
  list every frame, so the wheel couldn't scroll).
- Quieter controls (`theme::visuals`): fields a shade under their surface, a soft
  focus ring (`selection.stroke` at 60%), neutral hover instead of an accent
  outline, deeper popup shadow, 32 px menu rows.

## UI pass 6 (24 Nov 2026)
- **Look:** Tern Dark is deeper (bg #0D1016, card #13171E, elevated
  #181D25, hairline border #232A34) with a brighter green (#2FD48A, dark
  text on it); Opentern Light is white on cool grey (#F3F5F7) with
  #0A9A5C. T3 Code was the structural reference only (layers, hairlines,
  roomy dialogs), not its colours (the maintainer: stay with Opentern's
  palette). `Theme::edge()` (popup/field hairline) and `Theme::field()`
  (field fill; dropdowns share it) live in theme.rs. `widgets::accent`
  reads `visuals.hyperlink_color` (full strength; the selection stroke is
  60%), and `widgets::ink_on(fill)` picks text for an accent fill.
- **Dialogs:** `dialogs::frame` draws a hairline under the title, edge to
  edge (`MARGIN` 22). `section()` is a sentence-case heading now (no
  capitals). Stacked forms: `widgets::{stacked, pair, field_label, area,
  chip, form_rule}`; the alert dialog uses them (labels above, chips for
  delivery; the webhook chip clears the URL when off).
- **Script explorer keys** (`Editor::tree_keys`): a row click gives the
  explorer the keyboard (`tree_focus`, a press elsewhere or Esc takes it
  back); `App::shortcuts` stands down while `tree_has_keys`. Ctrl+C/X put
  the path on the clipboard as text too, so Ctrl+V arrives as a paste.
- **Watchlist switcher:** a sideways-scrolling strip of names
  (`list_tab`), hidden with one list.
- **Calendar:** every `Venue` by region, folded by default (remembered
  as `+<name>` in the folded set when opened).
- **Selected/on fill** (`Theme::accent_soft`): the accent steeped into
  near-black (white in light themes): a deep, rich green, with
  `accent_ink` text and `accent_edge` borders. A grey "lift" and a plain
  green wash both read cheap to the maintainer.
- **Popups:** `Theme::popup()` (deeper than `elevated`) for menus and
  dialogs; dialogs add `dialogs::sheen` (light from the top, drawn as
  stacked rounded bands so corners stay round). Meshes can't be clipped to
  rounded rects: `color::round_off` masks their corners with the popup
  fill. Sliders go through `widgets::slider`. Colour picker: Swatches /
  Custom (SV square, hue bar, hex, RGB); still `colors[:custom]`.
- **Selection:** `selection.stroke` is both selected text's colour and a
  focused field's ring, so it's the text colour with a touch of accent;
  the agent panel sets it to the text colour exactly.

## 0.1.5 (27 Oct 2026)
- **Onboarding** (`onboarding.rs`): a new workspace only (no saved layout or
  watchlists; settings key `onboarded`, set silently for older workspaces;
  never headless). Welcome → markets (`Market::symbols()`, top 7 each) →
  first chart (cards with sparklines via `Request::Fetch`, seq ≥
  `onboarding::SEQ_BASE` = 1 << 44, checked before feeds). Enter makes one
  watchlist per market. Leaving: FOLD (onboarding still covers the app) →
  the card becomes a window onto the chart and grows to `grid_rect` (waits
  up to 1.5 s for bars) → SETTLE. Colours are the logo's greens whatever the
  theme. Stills: `onboarding`, `onboarding:1`, `onboarding:2`.

- **Themes**: Tern Dark (Opentern Dark's surfaces, accent #22B56C) is the
  default dark theme; Opentern Dark is gone (`by_name` maps it). Opentern
  Light is white and sage with #07834A. Primary buttons draw white text on
  the accent whatever `on_accent` says.
- **Prices**: a quote moves into the chart's forming bar
  (`Chart::take_quote`), and symbols on a chart show the chart's price in the
  watchlist; not in demo (each demo timeframe is its own path).
- **Bottom panel** height is clamped to the window (`panel_max_h`, chart keeps
  180 px).
- Gotchas: `ui.set_opacity` replaces the parent's opacity (a child faded
  with `set_opacity` ignores its step's fade; use `multiply_opacity`). A new
  workspace has no `scripts/` folder; create it before starting a CLI there.
  Don't animate font sizes (each size re-rasterises glyphs). Screenshot
  bursts with grim stutter the compositor; measure frame pacing instead.

## UI pass 5 (22 Oct 2026, for 0.1.3)
- Shortcuts: `Tool::shortcut()` is the one table (TradingView's Alt T/H/V/C/F,
  Alt Shift R; the rest Alt Shift + letter, a test keeps them unique);
  `Tool::for_key` dispatches; Alt L/P/U scale; Ctrl Alt H hides drawings.
  Settings → Keyboard shortcuts (`settings::shortcuts`) renders the sheet; a
  rail button beside the theme switch opens it. Alt A / Alt S ignore Shift. Alt W puts the chart symbol at the end of the open list's top section (Ctrl W closes editor tabs, Cmd W windows on macOS).
- Watchlists (`watchlists.rs`): `Item::{Section, Symbol}` (untagged JSON, so
  old symbol arrays load), `Watchlist::symbols()`; title button with a menu
  (rename, add section, new list, delete), switcher circles, drag
  to reorder, sections fold. Still `watch-sections` splits the starter list.
- Hotlists: `HotMarket::India` = NIFTY 50 via Yahoo spark (20 symbols a
  request), gainers and losers only (no volume), fetched only while open.
- Update button: a ring fills with the download, then a restart icon. Still
  `update:<pct|ready|available>`.
- `widgets::option` rows are at least as wide as their text.
- Screenshot script lesson: only ever kill the process you launched (match its
  own `--data-dir`), never "an opentern window that isn't the user's pid".

## What's new card (0.2.0)
`whatsnew.rs`, from Magpie's: `RELEASES` (newest first, 2–4 plain points,
optional PNG). Opens as `Modal::WhatsNew` on the first launch of a newer
version (settings key `seen_version`, or the updater's
`OPENTERN_UPDATED_FROM`); a workspace onboarded before 0.2.0 sees 0.2.0's;
new, demo, still and headless runs start as seen. Settings → About →
What's new reopens it. Still: `whatsnew`.

## Windows notes (0.2.0)
- `crates/app/build.rs` embeds `assets/opentern.ico` and version info with
  `winresource` (Windows only); without it Start, search and Settings → Apps
  showed no icon.
- `.gitattributes` forces LF checkouts: Windows runners check out CRLF, and
  `include_str!`'d `.logo` files then failed to parse (no symbol logos).
  `logos::render` also strips `\r` now.

# Notes: charts, drawings, layouts and UI

Moved out of `CLAUDE.md` (Nov 2026) to keep the briefing short. Keep it current: update it in the same commit as the code.

## Phase 4 (done): drawings, layouts, watchlists
- 27 tools in TradingView's groups (lines, Fib/Gann, patterns, forecasting/measure,
  shapes, text, icons) with flyouts that remember the last tool; anchored by time and
  price (`draw::TimeMap` interpolates between bars and extrapolates past the ends).
  Magnet, lock/hide all, remove menu, Eraser, Zoom (drag a range).
- Select, move (body or handles), Delete, Esc, undo/redo (Ctrl+Z/Y, top bar buttons),
  floating toolbar (colours, width, style, text, extend, labels, lock, "use this style
  for new drawings" → templates kind `drawing`, name = `Tool::key()`).
- **Fib levels** (retracement and extension) are `DrawStyle::levels` (value, colour, on);
  empty means `FibLevel::standard(tool)`, so older drawings and the standard set store
  nothing. Edited in the bar's Levels panel (`drawbar.rs`, an `Area` under the bar, not a
  popup: egui keeps one popup open and each level's colour picker is one).
- **The chart's right-click menu** (`App::chart_menu`, `Action::ChartMenu { pos, price }`
  from `chart/ui.rs`; the price only on the main pane): a small card in the dialogs'
  look (`border_stroke`, `sheen`). Reset view, Copy price, Add alert at price
  (`open_alert_at`), Remove drawings (`push_undo` first), Remove indicators (no
  undo: the first click arms it, a red "Remove N indicators"), Chart settings. It
  replaced right-click → Chart settings; the chart takes no input while it's open.
- **Position size** (`DrawStyle::sizing: Option<Sizing>`; none = `Sizing::default()`,
  10,000 account, 1% risk, any lot): `Sizing::qty` = risk ÷ |entry − stop|, rounded
  down to the lot. Shown under the risk/reward ratio (left of the position when it
  would run under the scale). The bar's calculator opens `drawbar::sizing_panel`
  in the Fib Levels panel's place (`fib-levels-open`).
- **Rectangle handles:** 8 (`handle_points`: the two drawn corners first, the other
  two, then the sides' middles top/right/bottom/left); `drag` moves a time and a
  price for a corner, one of them for a side.
- **Settings → Drawings** (`settings.rs` `drawings_page`): after drawing, keep the
  tool or back to the cursor (`App::back_to_cursor`, store key `after_drawing`,
  applied where `DrawUi::finished` is read in `shell.rs`; Brush keeps its tool), and
  each tool's default look: the same `draw_styles` the drawing bar's star saves
  (`save_drawing_default`, `reset_drawing_default`). Rows lay their controls from the
  right using last frame's width (`drawing-default-w`), as the bars do.
- **Text tool** (Nov 2026, from PR #11's ideas): `DrawStyle::font` is its size,
  `bold`, `angle` (degrees on screen). `TextBox` in `draw.rs` is the box (anchor =
  bottom-left, turned about it); handles: anchor (moves), the baseline's right end
  (`drag_text` 1: size from how far along the baseline), the knob `TURN_GAP` above
  (2: turns, snapping to 0/45/90… and `line_angles`, the trend lines' slopes, within
  3°). Text sizes for hit-tests come from `TEXT_SIZES` (a thread-local of the last
  layouts; a guess before the first paint). Every tool's handles are
  `draw::paint_handle`: a white dot in a ring of the drawing's colour (dark grey for
  very light colours), a soft shadow, bigger with a halo under the pointer; the
  text's knob is a filled disc with `ARROW_CLOCKWISE`.
- **Text on lines and boxes** (`Tool::carries_text`): `DrawStyle::text_at`
  (`TextAt { along: Start|Middle|End, side: Above|On|Below }`; none = `TextAt::usual`,
  so old drawings keep their place). `carried_text` places it along the anchors' segment
  (vertical lines bottom-up; horizontal lines stop short of their price label), turned
  to the line; On gets a backing. Rectangles: Above/On(inside)/Below × left/middle/right.
  Edited in the bar's Text panel (an `Area`, like Levels). Clicking the text hits the
  drawing (`on_carried_text`).
- **Several selected** (`DrawUi::also` besides `selected`; `select`, `all_selected`,
  `is_selected`): Ctrl/Cmd+click toggles, Ctrl+drag from an empty spot draws a box
  (`boxing`) that adds what it touches. Dragging a selected body moves all
  (`group_drag`); `App::delete_selected_drawing` removes all with one undo. The bar
  becomes `several_bar` (count, one colour, lock, delete).
- **Drawing templates** (the bar's Template menu): named `DrawStyle`s per tool in
  `templates`, kind `drawing:<Tool::key()>` (`TemplateKind::Drawing`), apart from the
  starred style (kind `drawing`). Reset to default sets `DrawStyle::for_tool` and drops
  the starred style; named ones stay. Export and import carry them with no extra code.
- **The "+" by the price scale** (`plus_rect` in `chart/ui.rs`): at the pointer's height,
  inside the main pane against the scale; it takes the press like the paper tags (no
  drawing, no pan) and is hidden while a drawing tool is picked, a drawing is being
  placed or dragged, or replay is choosing. Click → `Action::PriceMenu` →
  `App::price_menu` (an `Area`, like the compare menu): "Add alert on X at P"
  (`open_alert_at`; Alt+A still starts from the last price) or "Draw horizontal line
  at P" (`add_horizontal_line`, the starred style, selected). P is rounded to the tick.
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
- **Workspaces** (`workspaces.rs`, Nov 2026; the maintainer picked numbered pills
  in their own row above the top bar, the layout menu staying in the top bar):
  each is a whole `LayoutState` (its charts, grid, sync, sizes). Only the shown one
  is live in `App`; the others are `Ws::saved` and nothing of them loads or
  streams (`switch_workspace` saves the shown one, then `install_layout` the
  other). A named layout saves them all as `LayoutSet { version: 3, current,
  workspaces: [{ name?, layout }] }`; `LayoutSet::parse` reads older layouts as
  one workspace. A pill's name is the one given (right-click → Rename, or a
  double-click; empty goes back) or its main chart's ticker (chart 0 of the grid,
  the big one). The shown pill is accent-soft with a filled number square; the
  others muted. Drag to reorder (`drop_index`), middle-click closes, the menu has
  Rename, Duplicate, Close, Close others. Keys (`keys::Action::WORKSPACES`, all
  changeable): Ctrl+Tab / Ctrl+Shift+Tab, Alt+1…9, Ctrl+T (new, on the active
  chart's symbol, interval and indicators), Ctrl+W (never the last), Ctrl+Shift+T
  (the last 10 closed, this run only). A new workspace is a blank chart
  (`Symbol::bare("")`: `open_chart` loads nothing, no legend or watermark, a "Pick a
  symbol" empty state) with symbol search open. The row also holds the app's buttons
  (`shell::app_buttons`: overview, grid, layout menu, full screen, snapshot,
  Settings), laid out first at its right; `TOP_H` went 44 → 36. Charts keep their
  scroll and zoom: `ChartState::view` (`SavedView { bar_w, right_time }`, none time
  = following) goes into `Chart::pending_view` and is put back on the first bars
  (`replace_bars`); a symbol or interval change drops it. A blank chart shows only a "Pick a
  symbol" button (`Action::PickSymbol`; no legend, watermark, scale numbers or
  "Loading…"). New workspaces take workspace 1's main chart's `ChartStyle`. The pills
  scroll sideways when they don't fit (hidden bar, the wheel scrolls them), and
  `Workspaces::reveal` keeps the shown one (and "+") in sight after a switch.
  Rows: `BAR_H` 34, `TOP_H` 32. Speed: `many_workspaces_stay_smooth` (ignored; set
  `OPENTERN_PERF_IMPORT` to an export) imports into a scratch folder, puts a custom
  index in each of ten workspaces and checks a frame costs the same as with one
  (Nov 2026: 0.86 ms with 1, 0.73 ms with 11; a switch ~28 ms, slowest 54 ms). `Chord::usable` now allows Tab, Enter and
  the arrows with Ctrl or Alt. Still: `workspaces`. The agent acts on the shown one.
- **Multi-chart** (`multi.rs`): 15 grids for 1–8 charts; sync symbol, interval,
  crosshair (default on), time, date range. **The active chart lives in
  `App::chart`; the others in `App::slots`, swapped into place by `with_slot`**
  (panic-safe), so every "the chart" code path works per chart. Each slot has its own
  `ChartUi`, scripts and strategy runs; inactive charts poll scripts, strategies and
  paper lines each frame. Pressing a cell activates it. Layout JSON is now
  `LayoutState { grid, sync, active, cells, charts }` (old single-chart JSON still loads).
  `cells` are dragged sizes (unit rects; `None` = the grid's own; checked by
  `cells_fit`): `dividers` finds the edges, `move_divider` moves every cell edge at
  that position (so the grid stays tiled) down to `MIN_W`×`MIN_H` per chart; edges
  are interacted after the charts (they win the press) and a press on one doesn't
  activate a chart. `Multi::maximized` (Alt+Enter, not saved) paints one chart in
  the whole area and follows the Alt+arrows (`step_chart` → `neighbour`: the
  nearest chart that way, in line first, wrapping round; all four directions,
  `keys::Action::CHARTS`). Date-range sync uses the
  leader's real plot width and re-syncs when any chart's width changes
  (`last_view` holds every width).
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
  (alerts, snapshots, updates; dot on the rail when unread). Every `App::notify`
  also pops up a card (`notes::card`) at the app's top right, under the top bar
  (over the side panel), for 6 s: the dialogs' border and green sheen (`dialogs::border_stroke`,
  `sheen`), icon disc, title, text, × on hover; held while pointed at. Side panel
  lists (Alerts, Object tree, Notifications) use the watchlist's rounded hover rows,
  no rules between rows, plain-case headings (`objects::heading`, `object_row`). Hub requests
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
`whatsnew.rs`, from Magpie's: `RELEASES` (newest first; plain points, up to 4 for a
small release and more for a big one, biggest and most useful first; optional PNG). Opens as `Modal::WhatsNew` on the first launch of a newer
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

## Motion (Nov 2026)
- **Popups** (`popups.rs`): an end-of-pass hook fades and grows every Foreground/Tooltip
  area's painted shapes, and paints a closed one's last shapes while they fade. Nothing
  to do for a new menu or dialog. Layout and input don't see it; tests have it off
  unless they call `popups::enable`.
- **Hovers** ease: `motion::hover(&resp)` (or `hover_if` for rows whose buttons take the
  hover), never `if resp.hovered()` picking a colour. egui's own buttons go through
  `widgets::eased`, combo boxes through `widgets::combo`.

## Favorite drawings toolbar (Nov 2026, from pull request #4)
- `favbar.rs`: a floating bar over the charts with the starred tools (favourites kind
  `tool`, keys `Tool::key()`, in order). **Off by default**; the star button in the
  drawing toolbar shows it, and starring a tool does too. A new workspace is seeded
  once with `DEFAULT_FAVORITE_TOOLS` (five: trend line, horizontal line, rectangle,
  Fib retracement, text; `fav_tools_seeded` in settings), so an emptied list stays
  empty.
- Where it sits, its orientation and folding live in `Arrangement`
  (`fav_toolbar*`), saved with the window like the rest of the layout. The position
  is a fraction of the room it can move in, measured with the unfolded size, so it
  keeps its place as the window resizes and folding leaves the handle put. Nothing
  is written to the workspace while it's dragged.
- Shown only in a pass where `chart_grid` ran (`FavBar::charts_pass`): not over
  Settings, the overview or a maximized bottom panel; and not while a snapshot is
  pending (`App::snap`), so snapshots never include it.
- Laid out by hand (every rect computed up front, kept in `FavBar::at` for tests).
  The body (tools, divider, orientation) folds into the handle: clipped and faded
  with `motion::toggle`. Its show/hide fades on its own, so `popups.rs` leaves the
  `fav-toolbar` area alone. Buttons are `widgets::tool_button_at`, the drawing
  toolbar's look; the frame uses the popups' fill, edge and shadow.
- Reordering: a tool's button senses drags; while one is lifted, a ghost follows the
  pointer, a line marks the landing slot (`drop_slot`: how many other tools' centres
  are before the pointer), and the drop moves it once. Dragging anywhere else on the
  bar moves the bar.
- Drawing toolbar groups have no arrows: a click picks the group's tool; a click on
  it while it's picked, or a right-click, toggles the flyout. Flyout rows carry a
  star (`tool_menu_row`); the list stays open while starring.
- `widgets::filled_star` draws Phosphor's filled star (the `fill` font variant,
  `theme::filled`), which lines up with the outline `ph::STAR`.
- Drawing tools stay picked after a drawing, until another tool, the cross or Esc
  (the maintainer, Nov 2026: no "Stay in drawing mode" setting; it was removed and
  a saved `back_to_cursor` is ignored). Zoom is one go.
- Both floating bars ask `App::charts_on_screen` before drawing.
- Menu rows (`widgets::menu_row_tinted`) lay out the hint first and shorten the label
  with "…" to fit; `menu_row_width` sizes a menu to its rows (the tool flyouts do).
- The selected drawing's bar (`drawbar.rs`) shares the look (popup fill, edge and
  shadow, radius 12, 42 high, `tool_button`s at 32) and keeps clear of the
  favourites bar: `drawing_bar_anchor` tries its usual place, then below, above and
  beside `FavBar::at.bar` (last frame's), the first that fits the charts.

## Keyboard shortcuts (Nov 2026)
- `keys.rs`: `Action` (the general ones, the price scale's, every drawing tool but the
  cursor) and `Chord` (key + Ctrl/Cmd, Alt, Shift; stored as "Ctrl Alt Shift K").
  `Keymap` = defaults (TradingView's; tools' from `Tool::shortcut`) plus the user's
  changes, kept as JSON under `shortcuts` in settings, read once at launch.
- `App::shortcuts` asks the keymap first (`run_shortcut`), then the fixed keys: typing
  to search, numbers, "/", arrows, Delete, Esc, Ctrl Shift Z (redo's second key). The
  editor's and the agent panel's keys aren't in it.
- Every label of a shortcut comes from the keymap (`Keymap::label`, `tip`): tool
  flyouts, the favourites bar, undo/redo, scale buttons, hide drawings, overview.
- The sheet (`settings.rs`): a pencil (or a click on the keys) records
  (`SettingsPage::recording`; the app's own shortcuts and Esc-closes-Settings stand
  aside meanwhile): the held modifiers show as lit keycaps beside a pulsing "Press
  keys"; Esc cancels, Delete clears, a usable chord needs Ctrl/Cmd or Alt (or is
  F1–F12). Shared keys stay on both (`Keymap::clashes`): orange-edged keycaps and a
  "!" whose tooltip names the other; pressing them runs the one the user set
  (`Keymap::action`). Rows have no gaps (lines sit midway); their rule stops at a
  gutter on the right holding the pencil and, once changed, a reset.

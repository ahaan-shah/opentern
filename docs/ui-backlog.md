# UI backlog

Visual and feel issues to fix in the UI upgrade pass, after all features and data
are dialed in (the maintainer's call, Oct 2026). Goal: as beautiful and calm as
Magpie. Behaviour bugs aren't parked here; they get fixed right away.

## Done in the UI pass (14 Oct 2026)
From the maintainer, after Phase 3:
1. Indicators dialog: the search icon has its own gutter (`widgets::search_field`);
   list rows stop short of the scrollbar; categories have icons; the dialog sizes to
   the window; empty favourites and no-match get a proper empty state.
2. Save template dialog: says what it keeps (the indicators as chips), a "Name"
   field, a note when it replaces one, Cancel and an accent Save.
3. Separation: the chart, the widget panel and the open bottom panel are rounded,
   edged cards on the darker frame with 5 px gaps (`shell.rs`, `round_corners`).
4. Interval menu: 196 px wide, grouped Minutes / Hours / Days, long names ("15
   minutes"), the star beside each row (filled when pinned, shown on hover
   otherwise), custom interval field at the bottom.
5. Colour swatch: a clean rounded swatch with a faint edge; the checkerboard only
   shows for see-through colours; accent ring on hover or while open.
6. Calmer overall: Magpie-style widgets everywhere (below).

Noticed along the way, also done:
- Watchlist columns size to their widest value; long symbols elide with "…".
- Legend rows sit on a soft backing (stronger, edged, on hover).
- Top bar: layout name with a caret (a folder icon when short of room); the
  layouts, chart type, templates, main, snapshot, drawing-flyout and remove menus
  are icon rows (`widgets::menu_item`); chart types have drawn icons
  (`chart::style::paint_kind_icon`).
- Dialog tabs and small either/or choices are a segmented control
  (`widgets::segmented`): indicator settings, symbol search filters,
  compare/overlay, paper Positions/Orders/History.
- Checkboxes, radios, dropdowns, dropdown rows, text fields and dialog buttons are
  Magpie-style (`widgets::{checkbox, radio, combo, option_value, field, button}`).
  The default egui ones are no longer used in the app.
- Empty states (`widgets::empty_state`) for alerts, alerts log, notifications,
  object tree, scripts, strategy report, Strategy Lab and paper tables.
- App icon (final, 15 Oct 2026): two white line-art candlesticks, one
  hollow and one filled, on an indigo squircle, styled exactly like Magpie's
  bird (same tile, glow, rim light and soft mark shadow); 16/24 px are a
  bigger, bolder mark
  (`scripts/make-icons.py` → `packaging/linux/opentern*.svg`, `icon-1024.png`,
  `crates/app/assets/icon-512.png` (window icon), `opentern.ico`).

## Second pass (15 Oct 2026), from the maintainer's testing
1. Compare: TradingView's three choices, "Same % scale", "New price scale" (its own
   axis right of the main one: drag to zoom, double-click to fit, right-click for
   options) and "New pane" (its own candles and scale). `chart::CompareMode`; the
   legend's "…" or a right-click switches.
2. Quick measure as on TradingView: the ruler or Shift+drag / Shift+click; blue up,
   red down; change, %, ticks, bars, time, volume; both ends marked on the axes;
   the next click clears it. Not a saved drawing (`Tool::Measure`).
3. Legend code button (and "Source code" in the … menu): a script opens its file;
   a built-in opens its Pine twin from the corpus, read-only (`sources.rs`), with
   "Make a copy".
4. Script Editor as a small IDE: explorer (filter, language tags, on-chart dot,
   right-click Add/Update, Duplicate, Rename, Delete, Open in your editor;
   built-ins' source), tabs, line numbers, current and error lines marked,
   Run = Ctrl+Enter, auto-indent, Tab/Shift+Tab, Ctrl+/ comments, Ctrl+W, a
   console whose "Line N" jumps to the line, and a status bar.
5. Resize handles show a grip (the bottom panel's always; side ones on hover).
6. Scrollbars keep a 12 px gutter (`floating_allocated_width`), everywhere.
7. Symbol search: a fixed-height list (it was clipped by last frame's size);
   typing is upper-cased; Enter waits for that query's results.
8. Notifications: icon disc, title and time on one line, body aligned under the
   title, hairline rule; "Clear all" in the header.
9. Flicker: rows whose buttons appear on hover (watchlist ×, alert actions) test
   the pointer, not `hovered()` (the button took the hover and hid itself).
10. Formulas (`BTCUSDT/ETHUSDT`): resolved in the dialog at once, and the search
    thread no longer downloads five symbol lists before answering.
11. Symbol logos (watchlist, search, legend), with an off switch in the ☰ menu.

## Third pass (15 Oct 2026)
1. Compare rows' eye / × / … work (a right-click area registered after them
   ate their clicks). Every price scale has A / L / % toggles underneath; a
   compare on its own scale has its own log and percent.
2. Symbol search columns elide instead of overlapping; formulas show their
   short form. Editor tabs leave room for the lock and ×.
3. Formulas autocomplete: after an operator the list shows matches for the
   term being typed; Enter or a click inserts `EXCHANGE:TICKER`; Enter on the
   formula row (or when the last term already names a symbol) opens it.
4. Script Editor like VSCodium: explorer (folders, file icons, filter,
   outline), tabs with icons, breadcrumbs, JetBrains Mono, Catppuccin colours
   (italic comments), indent guides, current line, error squiggle, minimap
   (wide enough panels), Problems / Output, status bar, Ctrl+B for the files.
   The bottom panel can be maximized.
5. Settings is a page in the chart area (☰, or the gear for Chart): Appearance
   (theme cards), Chart, Data providers, Symbol logos (a checkbox). Logos are
   TradingView-style discs: theSVG marks in white on the brand colour, coins
   from cryptocurrency-icons. Licences for resvg, @thesvg/icons (MIT; marks
   are trademarks) and cryptocurrency-icons (CC0) go in
   `THIRD-PARTY-LICENSES.txt` at Phase 14.

## Fourth pass (15 Oct 2026)
1. List of trades: wider columns, grouped prices, every cell elided to its
   column. Strategy Report KPI cards keep text inside (the value shrinks, the
   second figure moves or goes to the tooltip); compact cards cut the label.
   Equity chart labels no longer collide. Narrow report headers use icons.
2. Logos: ~200 bundled (web3icons coins, cropped round; theSVG companies);
   indices and commodities get drawn badges ("50", "Au", a drop for oil).
3. Formula autocomplete works on the term under the cursor, not just the last.
4. Range bar: a button under the toolbar column hides/shows the drawing
   toolbar; the ranges start where the chart does.
5. Commodities in search (their own filter) and in the starter watchlist.

## Fifth pass (15 Oct 2026)
1. The Script Editor's explorer is a tree of the scripts folder: subfolders
   open and close, scripts anywhere in it open and run on the chart (study ids
   carry the path, `pine:ideas/rsi.pine`); new script / new folder go in the
   folder last picked; right-click a folder for New script here, New folder,
   Rename, Open in your file manager, Delete (empty folders only); drag a
   script onto a folder to move it. Libraries are found in subfolders too.
2. Gradient canvas background (Settings → Chart → Canvas: Solid / Gradient,
   top and bottom colours).
3. New icon (since replaced: see the icon note above).
4. Fixed: Luau / JS / TS scripts on a chart weren't restored after a restart.

## Sixth pass (15 Oct 2026)
1. Replay controls size to their contents (the bars-left count ran under ×).
2. The top bar's gear is gone: ☰ opens Settings (Chart is a category there,
   and right-clicking the chart still opens Chart settings).
3. Icon redone in Magpie's style (above).

## Open
- Panel tabs in the Strategy Report and Strategy Lab are still small buttons
  (fine, but could be segmented too).
- DragValue number fields are egui's (styled by the visuals, not custom).

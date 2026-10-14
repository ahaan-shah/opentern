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
- App icon: a tern gliding over a rising price line on a sea-blue squircle
  (`scripts/make-icons.py` → `packaging/linux/opentern*.svg`, `icon-1024.png`,
  `crates/app/assets/icon-512.png` (window icon), `opentern.ico`).

## Open
- Panel tabs in the Strategy Report and Strategy Lab are still small buttons
  (fine, but could be segmented too).
- DragValue number fields are egui's (styled by the visuals, not custom).

# UI backlog

Visual and feel issues to fix in the UI upgrade pass, after all features and data
are dialed in (the maintainer's call, Oct 2026). Goal: as beautiful and calm as
Magpie. Behaviour bugs aren't parked here; they get fixed right away.

## From the maintainer (Oct 2026, after Phase 3)
1. Indicators dialog: the list's scrollbar overlaps the category labels; the search
   icon in the hint overlaps the text cursor. (Scrolling inside it also scrolled the
   chart behind: fixed as a bug.)
2. Save template dialog "looks bad": empty, oversized card. (Its height bug is fixed;
   the look still needs the pass.)
3. Chart, axes and watchlist need clearer visual separation (panel edges, scale
   column, widget panel).
4. Interval menu (the ▾ next to the intervals) looks cheap: far too wide, plain rows,
   star column far from the labels.
5. Indicator settings: the colour swatch box looks odd (border, checkerboard, size).
6. Overall: make it beautiful like Magpie.

## Noticed along the way
- Watchlist columns collide when the panel is narrow ("BTCUSDT82901.99").
- Legend rows sit over candles with no backing; TradingView-style but busy.
- Top bar: "Untitled"/layout and chart-type buttons need proper menus with icons.
- Dialog tabs are plain buttons; should be a proper segmented control.
- Checkboxes and combo boxes are egui defaults; restyle to Magpie's widgets.
- Placeholder panels ("… is coming soon") need real empty states.
- No app icon yet.
- Auto-scale ignores script drawings: a Zig Zag label above the visible high can sit on
  the legend rows. TradingView leaves top/bottom margin for labels; do the same.

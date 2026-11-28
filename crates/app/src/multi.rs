//! Multi-chart layouts (TradingView's "Select layout"): 1–8 charts in a
//! grid, with symbol, interval, crosshair, time and date range synced
//! between them as asked.
//!
//! One chart is active: it lives in `App::chart` (with its UI state,
//! scripts and strategy runs), so everything that works on "the chart"
//! works on the active one. The others wait in `App::slots`; to paint or
//! update one, it's swapped into place for the call (`App::with_slot`).

use crate::app::App;
use crate::chart::ui::ChartUi;
use crate::chart::{Chart, ChartState};
use crate::i18n::{tr, trf};
use crate::icons::ph;
use crate::theme::Theme;
use egui::{CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};

/// How the charts are arranged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Grid {
    #[default]
    One,
    TwoCols,
    TwoRows,
    ThreeCols,
    ThreeRows,
    BigLeftTwo,
    BigTopTwo,
    Four,
    FourCols,
    BigLeftThree,
    Five,
    Six,
    SixRows,
    Seven,
    Eight,
}

impl Grid {
    pub const ALL: [Grid; 15] = [
        Grid::One,
        Grid::TwoCols,
        Grid::TwoRows,
        Grid::ThreeCols,
        Grid::ThreeRows,
        Grid::BigLeftTwo,
        Grid::BigTopTwo,
        Grid::Four,
        Grid::FourCols,
        Grid::BigLeftThree,
        Grid::Five,
        Grid::Six,
        Grid::SixRows,
        Grid::Seven,
        Grid::Eight,
    ];

    pub fn count(self) -> usize {
        self.unit_cells().len()
    }

    /// The cells as fractions of the area (x0, y0, x1, y1).
    fn unit_cells(self) -> Vec<[f32; 4]> {
        let cols = |n: usize, y0: f32, y1: f32| -> Vec<[f32; 4]> {
            (0..n)
                .map(|i| [i as f32 / n as f32, y0, (i + 1) as f32 / n as f32, y1])
                .collect()
        };
        let rows = |n: usize| -> Vec<[f32; 4]> {
            (0..n)
                .map(|i| [0.0, i as f32 / n as f32, 1.0, (i + 1) as f32 / n as f32])
                .collect()
        };
        match self {
            Grid::One => vec![[0.0, 0.0, 1.0, 1.0]],
            Grid::TwoCols => cols(2, 0.0, 1.0),
            Grid::TwoRows => rows(2),
            Grid::ThreeCols => cols(3, 0.0, 1.0),
            Grid::ThreeRows => rows(3),
            Grid::BigLeftTwo => vec![[0.0, 0.0, 0.5, 1.0], [0.5, 0.0, 1.0, 0.5], [0.5, 0.5, 1.0, 1.0]],
            Grid::BigTopTwo => vec![[0.0, 0.0, 1.0, 0.5], [0.0, 0.5, 0.5, 1.0], [0.5, 0.5, 1.0, 1.0]],
            Grid::Four => [cols(2, 0.0, 0.5), cols(2, 0.5, 1.0)].concat(),
            Grid::FourCols => cols(4, 0.0, 1.0),
            Grid::BigLeftThree => {
                let mut v = vec![[0.0, 0.0, 0.5, 1.0]];
                v.extend((0..3).map(|i| [0.5, i as f32 / 3.0, 1.0, (i + 1) as f32 / 3.0]));
                v
            }
            Grid::Five => [cols(3, 0.0, 0.5), cols(2, 0.5, 1.0)].concat(),
            Grid::Six => [cols(3, 0.0, 0.5), cols(3, 0.5, 1.0)].concat(),
            Grid::SixRows => [
                cols(2, 0.0, 1.0 / 3.0),
                cols(2, 1.0 / 3.0, 2.0 / 3.0),
                cols(2, 2.0 / 3.0, 1.0),
            ]
            .concat(),
            Grid::Seven => [cols(4, 0.0, 0.5), cols(3, 0.5, 1.0)].concat(),
            Grid::Eight => [cols(4, 0.0, 0.5), cols(4, 0.5, 1.0)].concat(),
        }
    }

    /// The cells laid out in `r`, a pixel apart.
    pub fn cells(self, r: Rect) -> Vec<Rect> {
        cells_in(&self.unit_cells(), r)
    }
}

/// Unit cells (x0, y0, x1, y1) laid out in `r`, a pixel apart.
fn cells_in(unit: &[[f32; 4]], r: Rect) -> Vec<Rect> {
    unit.iter()
        .map(|&[x0, y0, x1, y1]| {
            let px = |f: f32| (r.left() + r.width() * f).round();
            let py = |f: f32| (r.top() + r.height() * f).round();
            let inset = |f: f32, a: f32| if f > 0.0 && f < 1.0 { a } else { 0.0 };
            Rect::from_min_max(
                pos2(px(x0) + inset(x0, 0.5), py(y0) + inset(y0, 0.5)),
                pos2(px(x1) - inset(x1, 0.5), py(y1) - inset(y1, 0.5)),
            )
        })
        .collect()
}

/// The smallest a chart gets when its neighbours are dragged into it:
/// room for its price scale and a plot (across), its legend, a plot and
/// the time axis (down).
const MIN_W: f32 = 260.0;
const MIN_H: f32 = 180.0;

/// An edge between charts: on the x axis (`vertical`, a line top to
/// bottom) or the y axis, at fraction `at`. Dragging it moves every cell
/// edge there, so the grid stays tiled.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Divider {
    vertical: bool,
    at: f32,
}

/// The dividers of a tiling, each with the pieces of edge it's made of (as
/// fractions along the other axis).
fn dividers(cells: &[[f32; 4]]) -> Vec<(Divider, Vec<(f32, f32)>)> {
    let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
    let mut out: Vec<(Divider, Vec<(f32, f32)>)> = Vec::new();
    for &[x0, y0, x1, y1] in cells {
        // Each cell's right and bottom edges, when inside the area.
        for (vertical, at, span) in [(true, x1, (y0, y1)), (false, y1, (x0, x1))] {
            if at >= 1.0 - 1e-4 {
                continue;
            }
            match out.iter_mut().find(|(d, _)| d.vertical == vertical && near(d.at, at)) {
                Some((_, spans)) => spans.push(span),
                None => out.push((Divider { vertical, at }, vec![span])),
            }
        }
    }
    out
}

/// Moves divider `d` to `to` (a fraction), as far as the cells on both
/// sides keep `min` (a fraction too). False when it can't move.
fn move_divider(cells: &mut [[f32; 4]], d: Divider, to: f32, min: f32) -> bool {
    let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
    let (lo_i, hi_i) = if d.vertical { (0, 2) } else { (1, 3) };
    // The room: the cells ending at it can shrink to `min`, as can the ones
    // starting there.
    let mut lo = 0.0f32;
    let mut hi = 1.0f32;
    for c in cells.iter() {
        if near(c[hi_i], d.at) {
            lo = lo.max(c[lo_i] + min);
        }
        if near(c[lo_i], d.at) {
            hi = hi.min(c[hi_i] - min);
        }
    }
    if lo > hi {
        return false;
    }
    let to = to.clamp(lo, hi);
    if near(to, d.at) {
        return false;
    }
    for c in cells.iter_mut() {
        if near(c[hi_i], d.at) {
            c[hi_i] = to;
        }
        if near(c[lo_i], d.at) {
            c[lo_i] = to;
        }
    }
    true
}

/// The cell beside cell `from` in direction (`dx`, `dy`): of those that
/// way, the ones in line with it first (sharing some of its height for a
/// step across, its width for a step up or down), then the nearest. None
/// that way: the farthest the other way, in line (it wraps round).
fn neighbour(cells: &[[f32; 4]], from: usize, dx: i32, dy: i32) -> Option<usize> {
    const EPS: f32 = 1e-4;
    let a = *cells.get(from)?;
    let across = dx != 0;
    // Along the step's axis: start and end of a cell; across it: the span.
    let (s0, s1, o0, o1) = if across { (0, 2, 1, 3) } else { (1, 3, 0, 2) };
    let dir = if across { dx } else { dy } as f32;
    let overlap = |c: &[f32; 4]| c[o1].min(a[o1]) - c[o0].max(a[o0]) > EPS;
    let centre = |c: &[f32; 4], i0: usize, i1: usize| (c[i0] + c[i1]) / 2.0;
    let off = |c: &[f32; 4]| (centre(c, o0, o1) - centre(&a, o0, o1)).abs();
    let others = || cells.iter().enumerate().filter(move |(i, _)| *i != from);
    let ahead = others()
        .filter(|(_, c)| {
            if dir > 0.0 {
                c[s0] >= a[s1] - EPS
            } else {
                c[s1] <= a[s0] + EPS
            }
        })
        .min_by(|(_, x), (_, y)| {
            let key = |c: &[f32; 4]| {
                let gap = if dir > 0.0 { c[s0] - a[s1] } else { a[s0] - c[s1] };
                (!overlap(c), gap, off(c))
            };
            let (kx, ky) = (key(x), key(y));
            kx.0.cmp(&ky.0).then(kx.1.total_cmp(&ky.1)).then(kx.2.total_cmp(&ky.2))
        })
        .map(|(i, _)| i);
    ahead.or_else(|| {
        // Wrap: the far end the other way, in line with it.
        others()
            .filter(|(_, c)| overlap(c))
            .max_by(|(_, x), (_, y)| {
                let far = |c: &[f32; 4]| -dir * centre(c, s0, s1);
                far(x).total_cmp(&far(y)).then(off(y).total_cmp(&off(x)))
            })
            .map(|(i, _)| i)
    })
}

/// Whether saved cells still fit `n` charts: inside the area, none
/// empty, adding up to all of it.
fn cells_fit(cells: &[[f32; 4]], n: usize) -> bool {
    cells.len() == n
        && cells
            .iter()
            .all(|c| c.iter().all(|v| v.is_finite() && (-1e-4..=1.0 + 1e-4).contains(v)) && c[2] > c[0] && c[3] > c[1])
        && (cells.iter().map(|c| (c[2] - c[0]) * (c[3] - c[1])).sum::<f32>() - 1.0).abs() < 1e-3
}

/// What's kept in step between the charts of a layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sync {
    pub symbol: bool,
    pub interval: bool,
    pub crosshair: bool,
    pub time: bool,
    pub date_range: bool,
}

impl Default for Sync {
    fn default() -> Self {
        Sync {
            symbol: false,
            interval: false,
            crosshair: true,
            time: false,
            date_range: false,
        }
    }
}

/// A saved layout: the grid and every chart in it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayoutState {
    pub version: u32,
    #[serde(default)]
    pub grid: Grid,
    #[serde(default)]
    pub sync: Sync,
    #[serde(default)]
    pub active: usize,
    /// The cells as dragged (fractions of the area); none = the grid's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cells: Option<Vec<[f32; 4]>>,
    pub charts: Vec<ChartState>,
}

impl LayoutState {
    /// Reads a layout, including ones saved before there were grids (a
    /// single chart's state).
    pub fn parse(json: &str) -> Option<LayoutState> {
        if let Ok(l) = serde_json::from_str::<LayoutState>(json)
            && !l.charts.is_empty()
        {
            return Some(l);
        }
        serde_json::from_str::<ChartState>(json).ok().map(|c| LayoutState {
            version: 2,
            grid: Grid::One,
            sync: Sync::default(),
            active: 0,
            cells: None,
            charts: vec![c],
        })
    }
}

/// A chart that isn't the active one, with everything that goes with it.
pub struct Slot {
    pub chart: Chart,
    pub ui: ChartUi,
    pub scripts: Vec<crate::scripting::ScriptRun>,
    pub strategies: crate::strategy::Strategies,
    pub drawings_saved: u64,
}

impl Slot {
    pub fn new(chart: Chart) -> Slot {
        Slot {
            chart,
            ui: ChartUi::default(),
            scripts: Vec::new(),
            strategies: Default::default(),
            drawings_saved: 0,
        }
    }
}

/// Sync state carried between frames.
#[derive(Default)]
pub struct Multi {
    pub grid: Grid,
    pub sync: Sync,
    /// The cells as dragged (fractions of the area); none = the grid's own.
    pub cells: Option<Vec<[f32; 4]>>,
    /// The chart shown alone (Alt+Enter), the others kept as they are.
    pub maximized: Option<usize>,
    /// The divider being dragged.
    dragging: Option<Divider>,
    /// The time under the pointer on whichever chart it's over.
    pub cross_time: Option<i64>,
    /// What was last pushed to the others: the leader, its (right-edge
    /// time, visible span) and every chart's plot width (a resize re-syncs).
    last_view: Option<(usize, i64, i64, Vec<u32>)>,
    /// Bumped on grid and sync changes (they're saved with the layout).
    pub changed: u64,
}

impl App {
    fn swap_slot(&mut self, s: &mut Slot) {
        std::mem::swap(&mut self.chart, &mut s.chart);
        std::mem::swap(&mut self.chart_ui, &mut s.ui);
        std::mem::swap(&mut self.scripts, &mut s.scripts);
        std::mem::swap(&mut self.strategies, &mut s.strategies);
        std::mem::swap(&mut self.drawings_saved, &mut s.drawings_saved);
    }

    /// Runs `f` with chart `i` in the active chart's place (the active
    /// chart itself when `i` is active). A panic inside still puts
    /// everything back before it carries on up.
    pub fn with_slot<R>(&mut self, i: usize, f: impl FnOnce(&mut App) -> R) -> Option<R> {
        if i == self.active {
            return Some(f(self));
        }
        let mut s = self.slots.get_mut(i)?.take()?;
        self.swap_slot(&mut s);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self)));
        self.swap_slot(&mut s);
        self.slots[i] = Some(s);
        match r {
            Ok(v) => Some(v),
            Err(p) => std::panic::resume_unwind(p),
        }
    }

    /// Runs `f` on every chart of the layout, the active one included.
    pub fn each_chart(&mut self, mut f: impl FnMut(&mut App)) {
        for i in 0..self.chart_count() {
            self.with_slot(i, &mut f);
        }
    }

    pub fn chart_count(&self) -> usize {
        self.slots.len().max(1)
    }

    /// Makes chart `i` the active one.
    pub fn activate(&mut self, i: usize) {
        if i == self.active || i >= self.slots.len() {
            return;
        }
        let Some(mut s) = self.slots[i].take() else { return };
        self.chart_ui.draw.cancel();
        self.study_menu = None;
        self.swap_slot(&mut s);
        self.slots[self.active] = Some(s);
        self.active = i;
        if self.multi.maximized.is_some() {
            self.multi.maximized = Some(i);
        }
        self.multi.last_view = None;
        self.multi.changed += 1;
    }

    /// Alt+arrows: the chart beside the active one that way (`dx`, `dy`
    /// are -1, 0 or 1), the nearest in line first. Past the edge it wraps
    /// round to the far side, as a browser's tabs do.
    pub fn step_chart(&mut self, dx: i32, dy: i32) {
        let n = self.chart_count();
        if n <= 1 {
            return;
        }
        if let Some(i) = neighbour(&self.unit_cells(), self.active, dx, dy) {
            self.activate(i);
        }
    }

    /// Alt+Enter: the active chart alone in the chart area, or back to the
    /// grid.
    pub fn toggle_maximized(&mut self) {
        if self.chart_count() > 1 {
            self.multi.maximized = match self.multi.maximized {
                Some(_) => None,
                None => Some(self.active),
            };
            self.multi.last_view = None;
        }
    }

    /// The cells as fractions of the chart area: as dragged, or the grid's.
    fn unit_cells(&self) -> Vec<[f32; 4]> {
        let n = self.chart_count();
        self.multi
            .cells
            .clone()
            .filter(|c| cells_fit(c, n))
            .unwrap_or_else(|| self.multi.grid.unit_cells())
    }

    /// Bumps on any change worth saving in the layout.
    pub(crate) fn layout_version(&self) -> u64 {
        let others: u64 = self.slots.iter().flatten().map(|s| s.chart.changed).sum();
        self.chart.changed + others + self.multi.changed + self.ws.changed
    }

    pub(crate) fn layout_state(&self) -> LayoutState {
        let charts = (0..self.chart_count())
            .filter_map(|i| {
                if i == self.active {
                    Some(self.chart.state())
                } else {
                    self.slots.get(i)?.as_ref().map(|s| s.chart.state())
                }
            })
            .collect();
        LayoutState {
            version: 2,
            grid: self.multi.grid,
            sync: self.multi.sync,
            active: self.active,
            cells: self.multi.cells.clone().filter(|c| cells_fit(c, self.chart_count())),
            charts,
        }
    }

    /// Puts a whole layout on screen: the active chart in place, the rest in
    /// slots, all of them opened.
    pub(crate) fn install_layout(&mut self, l: LayoutState) {
        // Close what's there now.
        self.save_drawings_now();
        for i in 0..self.slots.len() {
            if i != self.active {
                self.with_slot(i, |a| {
                    a.save_drawings_now();
                    a.close_chart_data();
                });
            }
        }
        self.close_chart_data();
        let n = l.grid.count();
        let mut charts: Vec<ChartState> = l.charts;
        while charts.len() < n {
            let c = charts[0].clone();
            charts.push(c);
        }
        charts.truncate(n);
        let active = l.active.min(n - 1);
        self.multi.grid = l.grid;
        self.multi.sync = l.sync;
        self.multi.cells = l.cells.filter(|c| cells_fit(c, n));
        self.multi.maximized = None;
        self.multi.dragging = None;
        self.multi.last_view = None;
        self.slots = charts
            .into_iter()
            .map(|c| Some(Slot::new(Chart::from_state(0, c))))
            .collect();
        // The active one moves into place.
        if let Some(mut s) = self.slots[active].take() {
            self.swap_slot(&mut s);
            self.chart_ui = Default::default();
            self.scripts = Vec::new();
            self.strategies = Default::default();
        }
        self.active = active;
        if n == 1 {
            self.slots = vec![None];
        }
        self.each_chart(|a| {
            a.reload_drawings();
            a.open_chart_data();
        });
        self.multi.changed += 1;
    }

    /// Changes the grid: new cells copy the active chart, extra charts close.
    pub fn set_grid(&mut self, g: Grid) {
        if g == self.multi.grid && self.slots.len() == g.count() {
            return;
        }
        let n = g.count();
        if self.slots.is_empty() {
            self.slots = vec![None];
        }
        if self.active >= n {
            self.activate(0);
        }
        // Close charts that no longer fit.
        while self.slots.len() > n {
            let i = self.slots.len() - 1;
            self.with_slot(i, |a| {
                a.save_drawings_now();
                a.close_chart_data();
            });
            self.slots.pop();
        }
        while self.slots.len() < n {
            let chart = Chart::from_state(0, self.chart.state());
            self.slots.push(Some(Slot::new(chart)));
            let i = self.slots.len() - 1;
            self.with_slot(i, |a| {
                a.reload_drawings();
                a.open_chart_data();
            });
        }
        self.multi.grid = g;
        self.multi.cells = None;
        self.multi.maximized = None;
        self.multi.dragging = None;
        self.multi.last_view = None;
        self.multi.changed += 1;
    }

    /// Applies a symbol to the active chart, and to all of them when the
    /// symbol is synced.
    pub fn open_symbol_synced(&mut self, symbol: opentern_core::Symbol) {
        // Picking a symbol anywhere shows its chart.
        self.layout.overview = false;
        if self.multi.sync.symbol {
            let s = symbol.clone();
            self.each_chart(move |a| a.open_symbol(s.clone()));
        } else {
            self.open_symbol(symbol);
        }
    }

    pub fn set_interval_synced(&mut self, tf: opentern_core::Timeframe) {
        if self.multi.sync.interval {
            self.each_chart(move |a| a.set_interval(tf));
        } else {
            self.set_interval(tf);
        }
    }

    /// Lays out and paints every chart of the grid. Pressing on a chart
    /// makes it active.
    pub(crate) fn chart_grid(&mut self, ui: &mut Ui, t: &Theme) {
        let full = ui.max_rect();
        self.grid_rect = full;
        self.favbar.charts_pass = ui.ctx().cumulative_pass_nr();
        let n = self.chart_count();
        if n <= 1 {
            self.multi.cross_time = None;
            self.chart_cell(ui, t, full, 0);
            return;
        }
        let unit = self.unit_cells();
        // One chart alone (Alt+Enter): the others aren't painted, but keep
        // their place for when it goes back.
        let cells: Vec<Rect> = match self.multi.maximized.filter(|m| *m < n) {
            Some(m) => (0..n).map(|i| if i == m { full } else { Rect::NOTHING }).collect(),
            None => cells_in(&unit, full),
        };
        ui.painter().rect_filled(full, CornerRadius::ZERO, t.border);
        let pointer = ui.ctx().pointer_hover_pos();
        let pressed = ui.input(|i| i.pointer.any_pressed());
        // The edges between charts, a few pixels either side.
        let edges = if self.multi.maximized.is_some() {
            Vec::new()
        } else {
            dividers(&unit)
        };
        let edge_rect = |d: &Divider, (a, b): (f32, f32)| {
            if d.vertical {
                let x = (full.left() + full.width() * d.at).round();
                Rect::from_x_y_ranges(
                    x - 4.0..=x + 4.0,
                    full.top() + full.height() * a..=full.top() + full.height() * b,
                )
            } else {
                let y = (full.top() + full.height() * d.at).round();
                Rect::from_x_y_ranges(
                    full.left() + full.width() * a..=full.left() + full.width() * b,
                    y - 4.0..=y + 4.0,
                )
            }
        };
        let on_edge = pointer.is_some_and(|p| {
            edges
                .iter()
                .any(|(d, spans)| spans.iter().any(|s| edge_rect(d, *s).contains(p)))
        });
        // Which chart the pointer is over, for the crosshair and the press.
        let over = pointer
            .filter(|_| !on_edge && self.multi.dragging.is_none())
            .and_then(|p| cells.iter().position(|c| c.contains(p)));
        if pressed
            && let Some(i) = over
            && i != self.active
            && self.modal.is_none()
            && !egui::Popup::is_any_open(ui.ctx())
        {
            self.activate(i);
        }
        let mut cross = None;
        for (i, cell) in cells.iter().enumerate() {
            if !cell.is_positive() {
                continue;
            }
            let sync_t = if self.multi.sync.crosshair && over != Some(i) {
                self.multi.cross_time
            } else {
                None
            };
            let hover_t = self
                .with_slot(i, |a| {
                    a.chart_ui.sync_cross = sync_t;
                    a.chart_cell(ui, t, *cell, i);
                    a.chart_ui.hover_time
                })
                .flatten();
            if over == Some(i) {
                cross = hover_t;
            }
        }
        if self.multi.cross_time != cross {
            self.multi.cross_time = cross;
            ui.ctx().request_repaint();
        }
        // The active chart's outline (none when it's shown alone).
        if let Some(r) = cells.get(self.active).filter(|_| self.multi.maximized.is_none()) {
            ui.painter().rect_stroke(
                *r,
                CornerRadius::ZERO,
                Stroke::new(1.5, t.accent),
                egui::StrokeKind::Inside,
            );
        }
        // Drag an edge to resize the charts on both sides. Made after the
        // charts, so the edge wins the press.
        // As fractions of the area (a grid with no room for them can't be
        // dragged at all).
        let min_x = MIN_W / full.width().max(1.0);
        let min_y = MIN_H / full.height().max(1.0);
        for (k, (d, spans)) in edges.iter().enumerate() {
            let mut lit = self.multi.dragging == Some(*d);
            for (j, span) in spans.iter().enumerate() {
                let r = edge_rect(d, *span);
                let resp = ui.interact(r, ui.id().with(("chart-edge", k, j)), Sense::drag());
                lit |= resp.hovered();
                if resp.hovered() || resp.dragged() {
                    ui.ctx().set_cursor_icon(if d.vertical {
                        egui::CursorIcon::ResizeHorizontal
                    } else {
                        egui::CursorIcon::ResizeVertical
                    });
                }
                if resp.drag_started() {
                    self.multi.dragging = Some(*d);
                }
                if resp.dragged()
                    && let Some(p) = resp.interact_pointer_pos()
                {
                    let to = if d.vertical {
                        (p.x - full.left()) / full.width()
                    } else {
                        (p.y - full.top()) / full.height()
                    };
                    let mut cells = unit.clone();
                    if move_divider(&mut cells, *d, to, if d.vertical { min_x } else { min_y }) {
                        // It moved: keep it lit at its new place.
                        let at = dividers(&cells)
                            .into_iter()
                            .map(|(nd, _)| nd)
                            .filter(|nd| nd.vertical == d.vertical)
                            .min_by(|a, b| (a.at - to).abs().total_cmp(&(b.at - to).abs()));
                        self.multi.dragging = at;
                        self.multi.cells = Some(cells);
                        self.multi.changed += 1;
                        ui.ctx().request_repaint();
                    }
                }
                if resp.drag_stopped() {
                    self.multi.dragging = None;
                }
            }
            if lit {
                for span in spans {
                    let r = edge_rect(d, *span);
                    let c = r.center();
                    let line = Stroke::new(2.0, t.accent.gamma_multiply(0.75));
                    if d.vertical {
                        ui.painter().vline(c.x, r.y_range(), line);
                    } else {
                        ui.painter().hline(r.x_range(), c.y, line);
                    }
                }
            }
        }
        self.sync_views(over.unwrap_or(self.active));
    }

    fn chart_cell(&mut self, ui: &mut Ui, t: &Theme, rect: Rect, _i: usize) {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        self.chart_area_in(&mut child, t);
    }

    /// Time and date-range sync: the chart being worked on leads.
    fn sync_views(&mut self, lead: usize) {
        let s = self.multi.sync;
        if !(s.time || s.date_range) {
            return;
        }
        let Some((right_t, span_t)) = self
            .with_slot(lead, |a| view_times(&a.chart, a.chart_ui.main_rect.width()))
            .flatten()
        else {
            return;
        };
        // Every chart's plot width: when one changes (a divider dragged, the
        // window resized), the date range is pushed again.
        let widths: Vec<u32> = (0..self.chart_count())
            .map(|i| self.with_slot(i, |a| a.chart_ui.main_rect.width() as u32).unwrap_or(0))
            .collect();
        let now = (lead, right_t, span_t, widths);
        if self.multi.last_view.as_ref() == Some(&now) {
            return;
        }
        self.multi.last_view = Some(now);
        for i in 0..self.chart_count() {
            if i == lead {
                continue;
            }
            self.with_slot(i, |a| {
                let c = &mut a.chart;
                let Some(step) = c.tf.millis() else { return };
                if c.bars.is_empty() {
                    return;
                }
                let right = index_at(&c.bars, step, right_t);
                if s.date_range {
                    let left = index_at(&c.bars, step, right_t - span_t);
                    let bars = (right - left).max(5.0);
                    let width = a.chart_ui.main_rect.width().max(100.0);
                    c.view.bar_w = (width / bars as f32).clamp(0.4, 80.0);
                }
                c.view.right = right;
            });
        }
    }

    /// The "Select layout" menu: grid choices drawn as little diagrams and
    /// what to sync.
    pub(crate) fn grid_menu(&mut self, ui: &mut Ui, t: &Theme) {
        ui.set_min_width(250.0);
        crate::widgets::menu_heading(ui, t, tr("CHARTS IN THIS WINDOW"));
        let per_row = 5;
        for row in Grid::ALL.chunks(per_row) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for g in row {
                    let (r, resp) = ui.allocate_exact_size(vec2(42.0, 32.0), Sense::click());
                    let on = self.multi.grid == *g;
                    let hov = crate::motion::toggle(ui.ctx(), resp.id, resp.hovered(), crate::motion::MICRO);
                    let p = ui.painter();
                    if on {
                        p.rect_filled(r, CornerRadius::same(6), t.accent_soft());
                    } else {
                        p.rect_filled(r, CornerRadius::same(6), crate::motion::with_alpha(t.hover_wash(), hov));
                    }
                    let inner = r.shrink2(vec2(8.0, 7.0));
                    let col = if on { t.accent } else { t.text2 };
                    for c in g.cells(inner) {
                        p.rect_stroke(
                            c.shrink(0.5),
                            CornerRadius::same(1),
                            Stroke::new(1.0, col),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let resp = resp.on_hover_text(match g.count() {
                        1 => tr("1 chart").to_string(),
                        n => trf!("{n} charts"),
                    });
                    if resp.clicked() {
                        self.set_grid(*g);
                    }
                }
            });
        }
        if self.chart_count() > 1 {
            crate::widgets::menu_rule(ui, t);
            let alone = self.multi.maximized.is_some();
            let (icon, label) = if alone {
                (ph::ARROWS_IN, tr("Show all charts"))
            } else {
                (ph::ARROWS_OUT, tr("Maximize this chart"))
            };
            let hint = crate::keys::Chord::parse("Alt Enter")
                .map(|c| c.label())
                .unwrap_or_default();
            if crate::widgets::menu_item(ui, t, Some(icon), label, &hint, false).clicked() {
                self.toggle_maximized();
            }
            if self.multi.cells.is_some()
                && crate::widgets::menu_item(
                    ui,
                    t,
                    Some(ph::ARROW_COUNTER_CLOCKWISE),
                    tr("Reset chart sizes"),
                    "",
                    false,
                )
                .clicked()
            {
                self.multi.cells = None;
                self.multi.last_view = None;
                self.multi.changed += 1;
            }
        }
        crate::widgets::menu_rule(ui, t);
        crate::widgets::menu_heading(ui, t, tr("KEEP IN SYNC"));
        let mut s = self.multi.sync;
        crate::widgets::checkbox(ui, &mut s.symbol, tr("Symbol"));
        crate::widgets::checkbox(ui, &mut s.interval, tr("Interval"));
        crate::widgets::checkbox(ui, &mut s.crosshair, tr("Crosshair"));
        crate::widgets::checkbox(ui, &mut s.time, tr("Time"));
        crate::widgets::checkbox(ui, &mut s.date_range, tr("Date range"));
        if s != self.multi.sync {
            // Turning symbol or interval sync on brings the others in line.
            let was = self.multi.sync;
            self.multi.sync = s;
            self.multi.changed += 1;
            self.multi.last_view = None;
            if s.symbol && !was.symbol {
                let sym = self.chart.symbol.clone();
                self.open_symbol_synced(sym);
            }
            if s.interval && !was.interval {
                let tf = self.chart.tf;
                self.set_interval_synced(tf);
            }
        }
        let _ = ph::LAYOUT;
    }
}

/// The chart's right-edge time and the time span across its plot, `width`
/// pixels wide (its own, so the others show the same dates).
fn view_times(c: &Chart, width: f32) -> Option<(i64, i64)> {
    let step = c.tf.millis()?;
    if c.bars.is_empty() {
        return None;
    }
    let t_at = |i: f64| {
        let n = c.bars.len() as f64;
        let last = n - 1.0;
        if i <= 0.0 {
            c.bars[0].time + (i * step as f64) as i64
        } else if i >= last {
            c.bars[c.bars.len() - 1].time + ((i - last) * step as f64) as i64
        } else {
            let k = i.floor() as usize;
            let f = i - k as f64;
            let (a, b) = (c.bars[k].time, c.bars[k + 1].time);
            a + ((b - a) as f64 * f) as i64
        }
    };
    let width = if width > 1.0 { width as f64 } else { 800.0 };
    let bars = width / c.view.bar_w as f64;
    let right = t_at(c.view.right);
    Some((right, right - t_at(c.view.right - bars)))
}

/// The fractional bar index of time `t` (extrapolated past the ends).
pub fn index_at(bars: &[opentern_core::Bar], step: i64, t: i64) -> f64 {
    if bars.is_empty() {
        return 0.0;
    }
    let step = step.max(1) as f64;
    let last = bars.len() - 1;
    if t <= bars[0].time {
        return (t - bars[0].time) as f64 / step;
    }
    if t >= bars[last].time {
        return last as f64 + (t - bars[last].time) as f64 / step;
    }
    match bars.binary_search_by_key(&t, |b| b.time) {
        Ok(i) => i as f64,
        Err(i) => {
            let (a, b) = (bars[i - 1].time, bars[i].time);
            (i - 1) as f64 + (t - a) as f64 / (b - a).max(1) as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_step_to_the_chart_that_way() {
        // 2×2: 0 1 / 2 3.
        let c = Grid::Four.unit_cells();
        assert_eq!(neighbour(&c, 0, 1, 0), Some(1));
        assert_eq!(neighbour(&c, 0, 0, 1), Some(2));
        assert_eq!(neighbour(&c, 3, 0, -1), Some(1));
        assert_eq!(neighbour(&c, 3, -1, 0), Some(2));
        assert_eq!(neighbour(&c, 1, 1, 0), Some(0), "wraps round");
        assert_eq!(neighbour(&c, 2, 0, 1), Some(0));
        // A big chart at the left and three stacked at the right.
        let c = Grid::BigLeftThree.unit_cells();
        assert_eq!(neighbour(&c, 0, 1, 0), Some(2), "the one level with its middle");
        assert_eq!(neighbour(&c, 3, -1, 0), Some(0));
        assert_eq!(neighbour(&c, 1, 0, 1), Some(2));
        assert_eq!(neighbour(&c, 0, 0, 1), None, "nothing above or below the big one");
        // Two side by side: up and down go nowhere.
        assert_eq!(neighbour(&Grid::TwoCols.unit_cells(), 0, 0, 1), None);
    }

    #[test]
    fn dividers_move_and_keep_the_grid_tiled() {
        let mut cells = Grid::Four.unit_cells();
        let ds = dividers(&cells);
        assert_eq!(ds.len(), 2, "one across, one down: {ds:?}");
        let down = ds.iter().find(|(d, _)| d.vertical).unwrap().0;
        assert!(move_divider(&mut cells, down, 0.7, 0.1));
        assert!(cells_fit(&cells, 4));
        assert_eq!(cells[0][2], 0.7);
        assert_eq!(cells[3][0], 0.7, "both rows move together");
        let down = Divider {
            vertical: true,
            at: 0.7,
        };
        assert!(move_divider(&mut cells, down, 0.99, 0.1));
        assert!(
            (cells[1][2] - cells[1][0] - 0.1).abs() < 1e-4,
            "stops at the smallest size"
        );
        assert!(!cells_fit(&cells[..3], 4));
        let mut bad = cells.clone();
        bad[0][2] = 0.2;
        assert!(!cells_fit(&bad, 4), "a gap");
    }

    #[test]
    fn grids_have_their_counts_and_fill_the_area() {
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0));
        for g in Grid::ALL {
            let cells = g.cells(r);
            assert_eq!(cells.len(), g.count());
            let area: f32 = cells.iter().map(|c| c.area()).sum();
            assert!(area > r.area() * 0.98 && area <= r.area() + 1.0, "{g:?}");
        }
        let counts: Vec<usize> = Grid::ALL.iter().map(|g| g.count()).collect();
        for n in 1..=8 {
            assert!(counts.contains(&n), "no grid with {n} charts");
        }
    }

    #[test]
    fn old_single_chart_layouts_still_load() {
        let sym = opentern_core::starter_watchlist().remove(0);
        let c = Chart::new(1, sym, opentern_core::Timeframe::parse("1h").unwrap());
        let old = serde_json::to_string(&c.state()).unwrap();
        let l = LayoutState::parse(&old).unwrap();
        assert_eq!(l.charts.len(), 1);
        assert_eq!(l.grid, Grid::One);
    }
}

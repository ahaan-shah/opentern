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
        cells_in(r, &self.unit_cells())
    }
}

const MIN_CHART_WIDTH: f32 = 120.0;
const MIN_CHART_HEIGHT: f32 = 100.0;
const DIVIDER_HIT_SIZE: f32 = 10.0;
const EDGE_EPSILON: f32 = 0.0001;

/// Shared chart-cell geometry. Custom cell bounds are normalized to the
/// chart area; maximize only changes the rectangles returned for painting.
#[derive(Clone, Debug, Default)]
pub struct GridGeometry {
    cells: Option<Vec<[f32; 4]>>,
    pub maximized: Option<usize>,
    dragging: bool,
}

impl GridGeometry {
    fn unit_cells(&self, grid: Grid, count: usize) -> Vec<[f32; 4]> {
        self.cells
            .as_ref()
            .filter(|cells| valid_cells(cells, count))
            .cloned()
            .unwrap_or_else(|| grid.unit_cells())
    }

    pub(crate) fn rects(&self, grid: Grid, area: Rect, count: usize) -> Vec<Rect> {
        let mut rects = cells_in(area, &self.unit_cells(grid, count));
        if let Some(maximized) = self.maximized.filter(|i| *i < rects.len()) {
            for (i, rect) in rects.iter_mut().enumerate() {
                *rect = if i == maximized { area } else { Rect::NOTHING };
            }
        }
        rects
    }

    fn restore_cells(&mut self, cells: Option<Vec<[f32; 4]>>, grid: Grid) {
        self.cells = cells.filter(|cells| valid_cells(cells, grid.count()));
        self.maximized = None;
        self.dragging = false;
    }

    fn saved_cells(&self, grid: Grid, count: usize) -> Option<Vec<[f32; 4]>> {
        self.cells
            .as_ref()
            .filter(|cells| valid_cells(cells, count) && **cells != grid.unit_cells())
            .cloned()
    }

    /// Registers the chart-grid splitters and applies any drag this frame.
    /// Returns true while a splitter is hovered or handling a drag.
    fn drag_dividers(&mut self, ui: &mut Ui, t: &Theme, grid: Grid, area: Rect, count: usize) -> bool {
        if self.maximized.is_some() {
            self.dragging = false;
            return false;
        }
        let cells = self.unit_cells(grid, count);
        let dividers = dividers(&cells);
        let pointer = ui.ctx().pointer_hover_pos();
        let pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        let released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
        let down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
        let mut over = false;
        let mut on_divider = false;
        for (i, divider) in dividers.into_iter().enumerate() {
            let rect = divider.rect(area);
            let hit = pointer.is_some_and(|p| rect.contains(p));
            over |= hit;
            on_divider |= hit;
            let delta = ui
                .push_id(i, |ui| {
                    crate::widgets::drag_handle(ui, t, rect, "chart-grid-divider", divider.axis == Axis::Vertical)
                })
                .inner;
            if delta != 0.0 && self.resize(divider, delta, area, &cells) {
                ui.ctx().request_repaint();
            }
        }
        let was_dragging = self.dragging;
        if pressed && on_divider {
            self.dragging = true;
        }
        if released || !down {
            self.dragging = false;
        }
        over || self.dragging || was_dragging && (!down || released)
    }

    fn resize(&mut self, divider: Divider, delta: f32, area: Rect, preset: &[[f32; 4]]) -> bool {
        let extent = match divider.axis {
            Axis::Vertical => area.width(),
            Axis::Horizontal => area.height(),
        };
        if extent <= 0.0 || !delta.is_finite() {
            return false;
        }
        let min = match divider.axis {
            Axis::Vertical => (MIN_CHART_WIDTH + 1.0) / extent,
            Axis::Horizontal => (MIN_CHART_HEIGHT + 1.0) / extent,
        };
        let mut low = f32::NEG_INFINITY;
        let mut high = f32::INFINITY;
        let cells = self.cells.get_or_insert_with(|| preset.to_vec());
        for cell in cells.iter() {
            let (start, end) = match divider.axis {
                Axis::Vertical => (cell[0], cell[2]),
                Axis::Horizontal => (cell[1], cell[3]),
            };
            let width = end - start;
            if (end - divider.at).abs() <= EDGE_EPSILON {
                low = low.max(-(width - min));
            }
            if (start - divider.at).abs() <= EDGE_EPSILON {
                high = high.min(width - min);
            }
        }
        if low > high {
            return false;
        }
        let shift = (delta / extent).clamp(low, high);
        if shift.abs() <= f32::EPSILON {
            return false;
        }
        for cell in cells.iter_mut() {
            match divider.axis {
                Axis::Vertical => {
                    if (cell[0] - divider.at).abs() <= EDGE_EPSILON {
                        cell[0] += shift;
                    }
                    if (cell[2] - divider.at).abs() <= EDGE_EPSILON {
                        cell[2] += shift;
                    }
                }
                Axis::Horizontal => {
                    if (cell[1] - divider.at).abs() <= EDGE_EPSILON {
                        cell[1] += shift;
                    }
                    if (cell[3] - divider.at).abs() <= EDGE_EPSILON {
                        cell[3] += shift;
                    }
                }
            }
        }
        true
    }
}

fn cells_in(area: Rect, cells: &[[f32; 4]]) -> Vec<Rect> {
    cells
        .iter()
        .map(|[x0, y0, x1, y1]| {
            let px = |f: f32| (area.left() + area.width() * f).round();
            let py = |f: f32| (area.top() + area.height() * f).round();
            let inset = |f: f32, a: f32| if f > 0.0 && f < 1.0 { a } else { 0.0 };
            Rect::from_min_max(
                pos2(px(*x0) + inset(*x0, 0.5), py(*y0) + inset(*y0, 0.5)),
                pos2(px(*x1) - inset(*x1, 0.5), py(*y1) - inset(*y1, 0.5)),
            )
        })
        .collect()
}

fn valid_cells(cells: &[[f32; 4]], count: usize) -> bool {
    if cells.len() != count {
        return false;
    }
    let mut area = 0.0;
    for (i, cell) in cells.iter().enumerate() {
        let [x0, y0, x1, y1] = *cell;
        if !cell.iter().all(|v| v.is_finite())
            || x0 < 0.0
            || y0 < 0.0
            || x1 > 1.0
            || y1 > 1.0
            || x1 - x0 <= EDGE_EPSILON
            || y1 - y0 <= EDGE_EPSILON
        {
            return false;
        }
        area += (x1 - x0) * (y1 - y0);
        for other in cells.iter().skip(i + 1) {
            let overlap_x = x1.min(other[2]) - x0.max(other[0]);
            let overlap_y = y1.min(other[3]) - y0.max(other[1]);
            if overlap_x > EDGE_EPSILON && overlap_y > EDGE_EPSILON {
                return false;
            }
        }
    }
    (area - 1.0).abs() <= 0.001
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy)]
struct Divider {
    axis: Axis,
    at: f32,
    start: f32,
    end: f32,
}

impl Divider {
    fn rect(self, area: Rect) -> Rect {
        let half = DIVIDER_HIT_SIZE / 2.0;
        match self.axis {
            Axis::Vertical => Rect::from_min_max(
                pos2(
                    area.left() + area.width() * self.at - half,
                    area.top() + area.height() * self.start,
                ),
                pos2(
                    area.left() + area.width() * self.at + half,
                    area.top() + area.height() * self.end,
                ),
            ),
            Axis::Horizontal => Rect::from_min_max(
                pos2(
                    area.left() + area.width() * self.start,
                    area.top() + area.height() * self.at - half,
                ),
                pos2(
                    area.left() + area.width() * self.end,
                    area.top() + area.height() * self.at + half,
                ),
            ),
        }
    }
}

fn dividers(cells: &[[f32; 4]]) -> Vec<Divider> {
    let mut dividers = Vec::new();
    for (i, a) in cells.iter().enumerate() {
        for b in cells.iter().skip(i + 1) {
            let overlap_y = a[3].min(b[3]) - a[1].max(b[1]);
            if overlap_y > EDGE_EPSILON {
                if (a[2] - b[0]).abs() <= EDGE_EPSILON {
                    dividers.push(Divider {
                        axis: Axis::Vertical,
                        at: a[2],
                        start: a[1].max(b[1]),
                        end: a[3].min(b[3]),
                    });
                } else if (b[2] - a[0]).abs() <= EDGE_EPSILON {
                    dividers.push(Divider {
                        axis: Axis::Vertical,
                        at: b[2],
                        start: a[1].max(b[1]),
                        end: a[3].min(b[3]),
                    });
                }
            }
            let overlap_x = a[2].min(b[2]) - a[0].max(b[0]);
            if overlap_x > EDGE_EPSILON {
                if (a[3] - b[1]).abs() <= EDGE_EPSILON {
                    dividers.push(Divider {
                        axis: Axis::Horizontal,
                        at: a[3],
                        start: a[0].max(b[0]),
                        end: a[2].min(b[2]),
                    });
                } else if (b[3] - a[1]).abs() <= EDGE_EPSILON {
                    dividers.push(Divider {
                        axis: Axis::Horizontal,
                        at: b[3],
                        start: a[0].max(b[0]),
                        end: a[2].min(b[2]),
                    });
                }
            }
        }
    }
    dividers.sort_by(|a, b| {
        let axis = (a.axis == Axis::Horizontal).cmp(&(b.axis == Axis::Horizontal));
        axis.then_with(|| a.at.total_cmp(&b.at))
            .then_with(|| a.start.total_cmp(&b.start))
    });
    let mut merged: Vec<Divider> = Vec::new();
    for divider in dividers {
        if let Some(last) = merged.last_mut()
            && last.axis == divider.axis
            && (last.at - divider.at).abs() <= EDGE_EPSILON
            && divider.start <= last.end + EDGE_EPSILON
        {
            last.end = last.end.max(divider.end);
        } else {
            merged.push(divider);
        }
    }
    merged
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
    pub geometry: GridGeometry,
    /// The time under the pointer on whichever chart it's over.
    pub cross_time: Option<i64>,
    /// The chart and press origin of an Alt+primary-click in progress.
    alt_press: Option<(usize, egui::Pos2)>,
    /// The leader, visible range and plot widths last pushed to the others.
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
        if self.multi.geometry.maximized.is_some() {
            self.multi.geometry.maximized = Some(i);
        }
        self.multi.last_view = None;
        self.multi.changed += 1;
    }

    /// Bumps on any change worth saving in the layout.
    pub(crate) fn layout_version(&self) -> u64 {
        let others: u64 = self.slots.iter().flatten().map(|s| s.chart.changed).sum();
        self.chart.changed + others + self.multi.changed
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
            cells: self.multi.geometry.saved_cells(self.multi.grid, self.chart_count()),
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
        self.multi.geometry.restore_cells(l.cells, l.grid);
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
        self.multi.geometry.cells = None;
        self.multi.geometry.maximized = None;
        self.multi.geometry.dragging = false;
        self.multi.alt_press = None;
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
            self.chart_cell(ui, t, full, 0, true);
            return;
        }
        let cells = self.multi.geometry.rects(self.multi.grid, full, n);
        ui.painter().rect_filled(full, CornerRadius::ZERO, t.border);
        let pointer = ui.ctx().pointer_hover_pos();
        let pressed = ui.input(|i| i.pointer.any_pressed());
        let geometry_before = self.multi.geometry.cells.clone();
        let divider_hovered = self.multi.geometry.drag_dividers(ui, t, self.multi.grid, full, n);
        if self.multi.geometry.cells != geometry_before {
            self.multi.changed += 1;
            self.multi.last_view = None;
        }
        // Which chart the pointer is over, for the crosshair and the press.
        let over = pointer.and_then(|p| cells.iter().position(|c| c.is_positive() && c.contains(p)));
        let primary_pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        let primary_released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
        let primary_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
        let alt = ui.input(|i| i.modifiers.alt && !i.modifiers.ctrl && !i.modifiers.command && !i.modifiers.shift);
        if primary_pressed && alt && !divider_hovered && self.modal.is_none() && !egui::Popup::is_any_open(ui.ctx()) {
            self.multi.alt_press = over.zip(ui.input(|i| i.pointer.press_origin()));
        }
        let mut alt_click = false;
        if primary_released {
            if let Some((start, origin)) = self.multi.alt_press.take() {
                let release = ui.ctx().pointer_interact_pos();
                if over == Some(start)
                    && alt
                    && release.is_some_and(|p| p.distance(origin) <= 6.0)
                    && self.modal.is_none()
                    && !egui::Popup::is_any_open(ui.ctx())
                {
                    self.toggle_maximized(start);
                    alt_click = true;
                }
            }
        } else if !primary_down {
            self.multi.alt_press = None;
        }
        let alt_gesture = self.multi.alt_press.is_some() || alt_click;
        if pressed
            && let Some(i) = over
            && i != self.active
            && !divider_hovered
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
                    a.chart_cell(ui, t, *cell, i, !divider_hovered && !alt_gesture);
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
        // The active chart's outline.
        if let Some(r) = cells.get(self.active).filter(|r| r.is_positive()) {
            ui.painter().rect_stroke(
                *r,
                CornerRadius::ZERO,
                Stroke::new(1.5, t.accent),
                egui::StrokeKind::Inside,
            );
        }
        self.sync_views(over.unwrap_or(self.active));
    }

    fn chart_cell(&mut self, ui: &mut Ui, t: &Theme, rect: Rect, _i: usize, interactive: bool) {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        self.chart_area_in(&mut child, t, interactive);
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
        let widths: Vec<u32> = (0..self.chart_count())
            .filter_map(|i| self.with_slot(i, |a| a.chart_ui.main_rect.width().max(0.0).to_bits()))
            .collect();
        let view = (lead, right_t, span_t, widths);
        if self.multi.last_view.as_ref() == Some(&view) {
            return;
        }
        self.multi.last_view = Some(view);
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

    pub(crate) fn move_active(&mut self, direction: isize) {
        let count = self.chart_count();
        if count <= 1 {
            return;
        }
        let next = (self.active as isize + direction).rem_euclid(count as isize) as usize;
        self.activate(next);
    }

    pub(crate) fn toggle_maximized(&mut self, i: usize) {
        if self.chart_count() <= 1 || i >= self.chart_count() {
            return;
        }
        self.activate(i);
        self.multi.geometry.maximized = if self.multi.geometry.maximized == Some(i) {
            None
        } else {
            Some(i)
        };
        self.multi.last_view = None;
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

/// The chart's right-edge time and visible time span.
fn view_times(c: &Chart, plot_width: f32) -> Option<(i64, i64)> {
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
    let bars = plot_width.max(100.0) as f64 / c.view.bar_w as f64;
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
    fn divider_resizing_preserves_tiling_and_minimum_dimensions() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
        let preset = Grid::BigLeftTwo.unit_cells();
        let vertical = dividers(&preset)
            .into_iter()
            .find(|d| d.axis == Axis::Vertical)
            .unwrap();
        let mut geometry = GridGeometry::default();
        assert!(geometry.resize(vertical, 1000.0, area, &preset));
        let cells = geometry.unit_cells(Grid::BigLeftTwo, 3);
        assert!(valid_cells(&cells, 3));
        let rects = cells_in(area, &cells);
        assert!(rects.iter().all(|r| r.width() >= MIN_CHART_WIDTH - 1.0));

        let horizontal = dividers(&cells)
            .into_iter()
            .find(|d| d.axis == Axis::Horizontal)
            .unwrap();
        assert!(geometry.resize(horizontal, 1000.0, area, &preset));
        let rects = cells_in(area, &geometry.unit_cells(Grid::BigLeftTwo, 3));
        assert!(rects.iter().all(|r| r.height() >= MIN_CHART_HEIGHT - 1.0));
        assert!(valid_cells(&geometry.unit_cells(Grid::BigLeftTwo, 3), 3));
    }

    #[test]
    fn maximize_changes_only_visible_rectangles() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0));
        let mut geometry = GridGeometry::default();
        let original = geometry.rects(Grid::BigLeftTwo, area, 3);
        geometry.maximized = Some(2);
        let maximized = geometry.rects(Grid::BigLeftTwo, area, 3);
        assert_eq!(maximized.len(), original.len());
        assert_eq!(maximized[2], area);
        assert!(maximized[..2].iter().all(|r| !r.is_positive()));
        geometry.maximized = None;
        assert_eq!(geometry.rects(Grid::BigLeftTwo, area, 3), original);
    }

    #[test]
    fn date_range_uses_plot_width() {
        let symbol = opentern_core::starter_watchlist().remove(0);
        let mut chart = Chart::new(1, symbol, opentern_core::Timeframe::parse("1h").unwrap());
        let step = chart.tf.millis().unwrap();
        chart.bars = (0..100)
            .map(|i| opentern_core::Bar {
                time: i * step,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 1.0,
            })
            .collect();
        chart.view.right = 99.0;
        chart.view.bar_w = 10.0;
        let (_, narrow_span) = view_times(&chart, 200.0).unwrap();
        let (_, wide_span) = view_times(&chart, 600.0).unwrap();
        assert!(wide_span > narrow_span);
        assert_eq!(wide_span, narrow_span * 3);
    }

    #[test]
    fn date_range_sync_invalidates_after_plot_resize() {
        let mut d = crate::drive::Driver::new("date-range-resize");
        d.app.set_grid(Grid::BigLeftTwo);
        d.wait(|a| a.slots.iter().flatten().all(|s| !s.chart.bars.is_empty()));
        d.app.multi.sync.date_range = true;
        d.frame(vec![]);
        d.app.sync_views(0);
        let before = d.app.with_slot(1, |a| a.chart.view.bar_w).unwrap();

        let area = d.app.grid_rect;
        let preset = Grid::BigLeftTwo.unit_cells();
        let divider = dividers(&preset)
            .into_iter()
            .find(|divider| divider.axis == Axis::Vertical)
            .unwrap();
        assert!(d.app.multi.geometry.resize(divider, 80.0, area, &preset));
        d.frame(vec![]);
        d.app.sync_views(0);
        let after = d.app.with_slot(1, |a| a.chart.view.bar_w).unwrap();
        assert_ne!(after, before, "target chart range must follow the resized plot");
    }

    #[test]
    fn old_single_chart_layouts_still_load() {
        let sym = opentern_core::starter_watchlist().remove(0);
        let c = Chart::new(1, sym, opentern_core::Timeframe::parse("1h").unwrap());
        let old = serde_json::to_string(&c.state()).unwrap();
        let l = LayoutState::parse(&old).unwrap();
        assert_eq!(l.charts.len(), 1);
        assert_eq!(l.grid, Grid::One);

        let legacy_grid = serde_json::json!({
            "version": 2,
            "grid": "TwoCols",
            "charts": [c.state(), c.state()]
        });
        let mut l = LayoutState::parse(&legacy_grid.to_string()).unwrap();
        assert_eq!(l.grid, Grid::TwoCols);
        assert!(l.cells.is_none());
        l.cells = Some(vec![[0.0, 0.0, 0.4, 1.0], [0.4, 0.0, 1.0, 1.0]]);
        let saved = serde_json::to_string(&l).unwrap();
        assert_eq!(LayoutState::parse(&saved).unwrap().cells, l.cells);
    }
}

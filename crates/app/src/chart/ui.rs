//! The chart widget: lays out panes and scales, handles pan and zoom, and
//! paints series, studies, crosshair, legend and scale labels, the way
//! TradingView arranges them.

use super::paint::{self, XMap, YMap, band_quad, polyline, rect_mesh, snap, styled_hline, styled_vline};
use super::scale;
use super::style::{Palette, SeriesKind, c32};
use super::{Chart, CompareMode, StudyView};
use crate::format;
use crate::icons::ph;
use crate::motion;
use crate::theme::{self, Theme};
use egui::epaint::Mesh;
use egui::{Align2, Color32, CornerRadius, FontId, Id, Painter, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use opentern_core::Bar;
use opentern_data::Status;
use opentern_series::study::{FillBetween, Format, Placement, PlotStyle};

const TIME_AXIS_H: f32 = 28.0;
const DIVIDER: f32 = 1.0;
const LEGEND_ROW: f32 = 20.0;

/// What the chart asks the app to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    StudySettings(u64),
    RemoveStudy(u64),
    StudyMenu(u64, Pos2),
    /// The legend's code button: open the study's source in the editor.
    StudySource(u64),
    /// The options of a compared symbol (its "…" button, or a right-click
    /// on its legend row or price scale).
    CompareMenu(usize, Pos2),
    MoreHistory,
    ChartSettings,
    RemoveCompare(usize),
    /// Chart trading: a paper order's line was dragged to a new price.
    PaperMove(u64, f64),
    PaperCancel(u64),
    /// The × on the position's line.
    PaperClose,
}

/// Interaction state that isn't worth saving.
pub struct ChartUi {
    pub draw: super::drawing_ui::DrawUi,
    /// Where the main pane was last frame (the drawing toolbar sits on it).
    pub main_rect: Rect,
    /// The bar under the crosshair (for the data window).
    pub hover_index: Option<usize>,
    /// Which pane a scale drag started on.
    scale_drag: Option<usize>,
    divider_drag: Option<usize>,
    /// Another chart's crosshair time, to mirror (multi-chart sync).
    pub sync_cross: Option<i64>,
    /// The time under this chart's crosshair, if the pointer is on it.
    pub hover_time: Option<i64>,
    /// Seasonality's numbers, until the bars change.
    pub season: super::profile::SeasonCache,
    /// A paper order being dragged: (order, price under the pointer).
    pub paper_drag: Option<(u64, f64)>,
}

impl Default for ChartUi {
    fn default() -> Self {
        ChartUi {
            draw: Default::default(),
            main_rect: Rect::NOTHING,
            hover_index: None,
            scale_drag: None,
            divider_drag: None,
            sync_cross: None,
            hover_time: None,
            season: Default::default(),
            paper_drag: None,
        }
    }
}

/// One pane's place on screen and value range this frame.
struct PaneGeo {
    rect: Rect,
    y: YMap,
    /// Decimals (or volume style) for this pane's labels.
    format: Format,
    /// Base value for percent mode (main pane only).
    pct_base: Option<f64>,
}

fn value_format(chart: &Chart, f: Format) -> impl Fn(f64) -> String {
    let p = chart.precision();
    move |v: f64| match f {
        Format::Price => format::price(v, p),
        Format::Fixed(d) => format::price(v, d as usize),
        Format::Volume => format::compact(v),
    }
}

/// Values of a study plot at screen bars `first..=last`, accounting for
/// its offset. Yields (screen index, value, colour index).
/// A point's colour: from the plot's palette (editable in the Style tab),
/// or computed by a script.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Col {
    Pal(u8),
    Raw(u32),
}

fn plot_points<'a>(
    s: &'a StudyView,
    p: usize,
    first: usize,
    last: usize,
    n: usize,
) -> impl Iterator<Item = (usize, f64, Col)> + 'a {
    let off = s.meta().offset(p, &s.inst.inputs);
    let vals = &s.inst.out.plots[p];
    let cols = &s.inst.out.colors[p];
    let raw = s.inst.out.raw.get(p);
    // Screen index j shows source index j - off; future bars (offset > 0)
    // can extend past the last real bar.
    let end = if off > 0 { last + off as usize } else { last };
    (first..=end).filter_map(move |j| {
        let src = j as i64 - off;
        if src < 0 || src as usize >= n {
            return None;
        }
        let i = src as usize;
        let v = vals[i];
        let col = match raw.and_then(|r| r.get(i)).copied().unwrap_or(0) {
            0 => Col::Pal(cols[i]),
            c => Col::Raw(c),
        };
        v.is_finite().then_some((j, v, col))
    })
}

/// Background colours (`bgcolor`) of the bars `first..=last`.
fn draw_bg(p: &Painter, s: &StudyView, x: &XMap, first: usize, last: usize, rect: Rect) {
    let bg = &s.inst.out.bg;
    if bg.is_empty() {
        return;
    }
    let mut mesh = Mesh::default();
    let half = x.bar_w / 2.0;
    let end = last.min(bg.len().saturating_sub(1));
    for (j, &c) in bg.iter().enumerate().take(end + 1).skip(first) {
        if c != 0 {
            let cx = x.x(j as f64);
            let r = Rect::from_x_y_ranges((cx - half)..=(cx + half), rect.y_range());
            rect_mesh(&mut mesh, r, c32(opentern_series::Rgba(c)));
        }
    }
    p.add(mesh);
}

fn bar_color(chart: &Chart, i: usize, b: &Bar) -> bool {
    if chart.style.color_on_prev_close && i > 0 {
        b.close >= chart.bars[i - 1].close
    } else {
        b.close >= b.open
    }
}

/// `interactive` is false while a dialog or menu is open over the chart, so
/// scrolling or clicking there never reaches the chart underneath.
pub fn show(
    ui: &mut Ui,
    chart: &mut Chart,
    cui: &mut ChartUi,
    t: &Theme,
    interactive: bool,
    tools: &super::drawing_ui::Tools,
) -> Vec<Action> {
    let mut actions = Vec::new();
    chart.replay_tick(ui.ctx());
    let full = ui.max_rect();
    let pal = chart.style.palette(t);
    let painter = ui.painter_at(full);
    painter.rect_filled(full, CornerRadius::ZERO, pal.bg);
    let ppp = ui.ctx().pixels_per_point();
    let scale_font = theme::regular(chart.style.scale_font as f32 - 0.5);
    let small = theme::regular(11.5);

    // ---- layout: price scale width from the widest label it may show
    let prec = chart.precision();
    let widest = chart.bars.iter().rev().take(500).map(|b| b.high).fold(0.0f64, f64::max);
    let sample = format!("{}0", format::price(widest.max(1.0) * 10.0, prec));
    let label_w = painter.layout_no_wrap(sample, scale_font.clone(), pal.text).size().x;
    let scale_w = (label_w + 18.0).clamp(56.0, 140.0);
    // Compared symbols on a scale of their own get a column each, right of
    // the main scale (as on TradingView).
    let own: Vec<usize> = chart
        .compares
        .iter()
        .enumerate()
        .filter(|(_, c)| c.visible && c.mode == CompareMode::Scale && !c.bars.is_empty())
        .map(|(i, _)| i)
        .collect();
    let own_w: Vec<f32> = own
        .iter()
        .map(|&i| {
            let c = &chart.compares[i];
            let widest = c.bars.iter().rev().take(500).map(|b| b.high).fold(0.0f64, f64::max);
            let sample = format!("{}0", format::price(widest.max(1.0) * 10.0, c.symbol.precision()));
            let w = painter.layout_no_wrap(sample, scale_font.clone(), pal.text).size().x;
            (w + 18.0).clamp(56.0, 140.0)
        })
        .collect();
    let own_total: f32 = own_w.iter().sum();
    let plot_all = Rect::from_min_max(
        full.min,
        pos2(full.right() - scale_w - own_total, full.bottom() - TIME_AXIS_H),
    );
    let scale_col = Rect::from_min_max(
        pos2(plot_all.right(), full.top()),
        pos2(plot_all.right() + scale_w, plot_all.bottom()),
    );
    let time_axis = Rect::from_min_max(
        pos2(full.left(), plot_all.bottom()),
        pos2(plot_all.right(), full.bottom()),
    );

    if !chart.bars.is_empty() && chart.status != Status::Loading {
        chart.fit_range(plot_all.width());
    }

    // ---- horizontal map and visible range
    let n = chart.bars.len();
    let x = XMap {
        right: plot_all.right(),
        right_index: chart.view.right,
        bar_w: chart.view.bar_w,
    };
    let first_f = x.index(plot_all.left()).floor();
    let first = first_f.max(0.0) as usize;
    let last = if n == 0 {
        0
    } else {
        (x.index(plot_all.right()).ceil().max(0.0) as usize).min(n - 1)
    };
    if chart.wants_more(first_f) {
        actions.push(Action::MoreHistory);
    }

    // ---- panes
    let total_w: f32 = chart.panes.iter().map(|p| p.weight.max(0.05)).sum();
    let mut y = plot_all.top();
    let mut geos: Vec<PaneGeo> = Vec::new();
    let pane_count = chart.panes.len();
    for (k, pane) in chart.panes.iter().enumerate() {
        let h = plot_all.height() * pane.weight.max(0.05) / total_w;
        let bottom = if k + 1 == pane_count { plot_all.bottom() } else { y + h };
        let rect = Rect::from_min_max(pos2(plot_all.left(), y), pos2(plot_all.right(), bottom));
        y = bottom;
        let main = pane.id == 0;
        // Autoscale: everything visible in the pane.
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut grow = |v: f64| {
            if v.is_finite() {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        };
        if main && n > 0 {
            for (_, v) in compare_points(chart, first, last).into_iter().flat_map(|(_, pts)| pts) {
                grow(v);
            }
            for b in &chart.bars[first.min(n - 1)..=last] {
                if chart.style.kind.is_line() {
                    grow(b.close);
                } else {
                    grow(b.high);
                    grow(b.low);
                }
            }
        }
        // A compared symbol in this pane: its highs and lows.
        if n > 0 {
            for c in chart
                .compares
                .iter()
                .filter(|c| c.visible && c.mode == CompareMode::Pane && c.pane == pane.id)
            {
                for j in first.min(n - 1)..=last {
                    if let Some(b) = c.bar_at(chart.bars[j].time) {
                        grow(b.high);
                        grow(b.low);
                    }
                }
            }
        }
        let mut format = Format::Price;
        let mut first_study = true;
        // Script labels need head room beyond their price (TradingView
        // leaves it), so the top and bottom margins grow when any show.
        let mut label_lines = 0usize;
        let tm = super::draw::TimeMap {
            bars: &chart.bars,
            step: match chart.bars.as_slice() {
                [.., a, b] => (b.time - a.time).max(1),
                _ => 60_000,
            },
            x: XMap { ..x },
        };
        let in_view = |xv: i64, loc: opentern_series::drawings::XLoc| {
            let i = match loc {
                opentern_series::drawings::XLoc::BarIndex => xv as f64,
                opentern_series::drawings::XLoc::BarTime => tm.index(xv),
            };
            i >= first as f64 - 0.5 && i <= last as f64 + 0.5
        };
        for s in chart.studies.iter().filter(|s| s.pane == pane.id && s.shown(chart.tf)) {
            if let Some(d) = &s.inst.drawings {
                for l in d.labels.iter().filter(|l| in_view(l.x, l.xloc)) {
                    if l.yloc.as_ref() == "price" {
                        grow(l.y);
                    }
                    label_lines = label_lines.max(l.text.lines().count().max(1));
                }
                for l in &d.lines {
                    if in_view(l.x1, l.xloc) {
                        grow(l.y1);
                    }
                    if in_view(l.x2, l.xloc) {
                        grow(l.y2);
                    }
                }
                for b in d
                    .boxes
                    .iter()
                    .filter(|b| in_view(b.left, b.xloc) || in_view(b.right, b.xloc))
                {
                    grow(b.top);
                    grow(b.bottom);
                }
            }
            if s.meta().placement == Placement::Bottom {
                continue;
            }
            if !main && first_study {
                format = s.meta().format;
                first_study = false;
            }
            if s.meta().id == "seasonality" {
                for yr in cui.season.get(chart, s.inst.inputs.len("years")) {
                    for (_, v) in &yr.points {
                        grow(*v);
                    }
                }
            }
            for (pi, look) in s.look.plots.iter().enumerate() {
                if look.visible {
                    if s.meta().plots[pi].style == PlotStyle::Shape {
                        continue;
                    }
                    for (_, v, _) in plot_points(s, pi, first, last, n) {
                        grow(v);
                    }
                }
            }
            for h in s.look.hlines.iter().filter(|h| h.visible) {
                if !main {
                    grow(h.value);
                }
            }
        }
        let log = main && chart.log;
        let (mut alo, mut ahi) = if lo.is_finite() { (lo, hi) } else { (0.0, 1.0) };
        if log {
            alo = alo.max(1e-12);
        }
        if (ahi - alo).abs() < 1e-12 {
            let d = if alo == 0.0 { 1.0 } else { alo.abs() * 0.01 };
            alo -= d;
            ahi += d;
        }
        // Margins: 10% top, 8% bottom, as TradingView; more with labels
        // (about a label's height at this pane size).
        let (top_m, bot_m) = if label_lines > 0 {
            // A bubble: its lines, padding and pointer.
            let px = 16.0 * label_lines as f32 + 20.0;
            let extra = (px / rect.height().max(60.0)).min(0.3) as f64;
            (0.1 + extra, 0.08 + extra)
        } else {
            (0.1, 0.08)
        };
        let (tlo, thi) = if log { (alo.ln(), ahi.ln()) } else { (alo, ahi) };
        let span = thi - tlo;
        let (mlo, mhi) = (tlo - span * bot_m, thi + span * top_m);
        let (alo, ahi) = if log { (mlo.exp(), mhi.exp()) } else { (mlo, mhi) };
        let (lo, hi) = if pane.scale.auto {
            (alo, ahi)
        } else {
            (pane.scale.lo, pane.scale.hi)
        };
        let pct_base = (main && chart.percent && n > 0).then(|| chart.bars[first.min(n - 1)].close);
        geos.push(PaneGeo {
            rect,
            y: YMap {
                top: rect.top(),
                bottom: rect.bottom(),
                lo,
                hi,
                log,
            },
            format,
            pct_base,
        });
    }
    // Keep the auto range in the pane state, so a drag starts from it.
    for (pane, g) in chart.panes.iter_mut().zip(&geos) {
        if pane.scale.auto {
            pane.scale.lo = g.y.lo;
            pane.scale.hi = g.y.hi;
        }
    }
    // Compared symbols' own scales: fitted like the main one unless dragged.
    let mut owns: Vec<(usize, Rect, YMap)> = Vec::new();
    {
        let mut cx = scale_col.right();
        for (k, &ci) in own.iter().enumerate() {
            let col = Rect::from_min_max(pos2(cx, full.top()), pos2(cx + own_w[k], plot_all.bottom()));
            cx += own_w[k];
            let y = own_ymap(chart, &chart.compares[ci], first, last, geos[0].rect);
            let sc = &mut chart.compares[ci].scale;
            if sc.auto {
                sc.lo = y.lo;
                sc.hi = y.hi;
            }
            owns.push((ci, col, y));
        }
    }

    // ---- interaction
    let pointer = ui.ctx().pointer_hover_pos().filter(|_| interactive);
    let sense = |s: Sense| if interactive { s } else { Sense::hover() };
    let plot_resp = ui.interact(
        plot_all,
        Id::new(("chart-plot", chart.id)),
        sense(Sense::click_and_drag()),
    );
    // Drawings get the pointer first on the main pane.
    let main_x = XMap {
        right: plot_all.right(),
        right_index: chart.view.right,
        bar_w: chart.view.bar_w,
    };
    let on_main = plot_resp
        .interact_pointer_pos()
        .or(pointer)
        .is_some_and(|p| geos.first().is_some_and(|g| g.rect.contains(p)));
    // Chart trading: paper order tags drag their order; × cancels (or
    // closes the position). They sit above the plot, so they win the press.
    let mut paper_used = false;
    if interactive && let Some(g) = geos.first() {
        let tick = if chart.symbol.tick_size > 0.0 {
            chart.symbol.tick_size
        } else {
            0.01
        };
        for (k, l) in chart.paper.iter().enumerate() {
            let price = match (cui.paper_drag, l.order) {
                (Some((id, p)), Some(o)) if id == o => p,
                _ => l.price,
            };
            let yy = g.y.y(price);
            if yy < g.rect.top() || yy > g.rect.bottom() {
                continue;
            }
            let (_, tag, close) = paper_geom(ui.painter(), l, yy, g.rect);
            let x = ui.interact(close, Id::new(("paper-x", chart.id, k)), Sense::click());
            if x.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                paper_used = true;
            }
            if x.clicked() {
                actions.push(match l.order {
                    Some(o) => Action::PaperCancel(o),
                    None => Action::PaperClose,
                });
                paper_used = true;
                continue;
            }
            let Some(order) = l.order else { continue };
            let t = ui.interact(tag, Id::new(("paper-tag", chart.id, k)), Sense::drag());
            if t.hovered() || t.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                paper_used = true;
            }
            if t.dragged()
                && let Some(p) = t.interact_pointer_pos()
            {
                let v = (g.y.value(p.y) / tick).round() * tick;
                cui.paper_drag = Some((order, v));
            }
            if t.drag_stopped()
                && let Some((id, v)) = cui.paper_drag.take()
            {
                actions.push(Action::PaperMove(id, v));
            }
        }
    }
    let choosing = chart.replay.as_ref().is_some_and(|r| r.selecting);
    let drawing_used = !paper_used
        && interactive
        && n > 0
        && !choosing
        && (on_main || tools.tool.is_drawing())
        && super::drawing_ui::handle(
            ui,
            chart,
            &mut cui.draw,
            tools,
            &plot_resp,
            main_x,
            geos[0].y,
            geos[0].rect,
        );
    if (tools.tool.is_drawing() || tools.tool == super::draw::Tool::Measure) && interactive {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    if plot_resp.dragged() && !drawing_used && !paper_used {
        let d = plot_resp.drag_delta();
        chart.view.right -= (d.x / chart.view.bar_w) as f64;
        // Dragging vertically pans the price too, which ends autoscale.
        if d.y.abs() > 0.0
            && let Some(k) = plot_resp
                .interact_pointer_pos()
                .and_then(|p| geos.iter().position(|g| g.rect.contains(p)))
        {
            let g = &geos[k];
            let pane = &mut chart.panes[k];
            let dv = |v: f64| g.y.value(g.y.y(v) - d.y);
            let (lo, hi) = (dv(pane.scale.lo), dv(pane.scale.hi));
            pane.scale.auto = false;
            pane.scale.lo = lo;
            pane.scale.hi = hi;
        }
    }
    if plot_resp.secondary_clicked() {
        actions.push(Action::ChartSettings);
    }
    if plot_resp.hovered() && interactive {
        let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
        let mut factor = zoom;
        if scroll.y != 0.0 {
            factor *= (scroll.y * 0.003).exp();
        }
        if factor != 1.0
            && let Some(p) = pointer
        {
            // Zoom around the bar under the pointer.
            let anchor = x.index(p.x);
            let new_w = (chart.view.bar_w * factor).clamp(0.4, 80.0);
            chart.view.right = anchor + (chart.view.right - anchor) * (chart.view.bar_w / new_w) as f64;
            chart.view.bar_w = new_w;
        }
        if scroll.x != 0.0 {
            chart.view.right -= (scroll.x / chart.view.bar_w) as f64;
        }
    }
    // Price scale: drag to zoom a pane's range, double-click for auto.
    let scale_resp = ui.interact(
        scale_col,
        Id::new(("chart-scale", chart.id)),
        sense(Sense::click_and_drag()),
    );
    if scale_resp.drag_started() {
        cui.scale_drag = scale_resp
            .interact_pointer_pos()
            .and_then(|p| geos.iter().position(|g| p.y >= g.rect.top() && p.y <= g.rect.bottom()));
    }
    if scale_resp.dragged()
        && let Some(k) = cui.scale_drag
    {
        let dy = scale_resp.drag_delta().y;
        let g = &geos[k];
        let pane = &mut chart.panes[k];
        let f = (dy as f64 * 0.006).exp();
        let (tlo, thi) = if g.y.log {
            (pane.scale.lo.ln(), pane.scale.hi.ln())
        } else {
            (pane.scale.lo, pane.scale.hi)
        };
        let mid = (tlo + thi) / 2.0;
        let half = (thi - tlo) / 2.0 * f;
        let (nlo, nhi) = (mid - half, mid + half);
        pane.scale.auto = false;
        (pane.scale.lo, pane.scale.hi) = if g.y.log { (nlo.exp(), nhi.exp()) } else { (nlo, nhi) };
    }
    if scale_resp.drag_stopped() {
        cui.scale_drag = None;
    }
    if scale_resp.double_clicked()
        && let Some(k) = scale_resp
            .interact_pointer_pos()
            .and_then(|p| geos.iter().position(|g| p.y >= g.rect.top() && p.y <= g.rect.bottom()))
    {
        chart.panes[k].scale.auto = true;
    }
    let scale_resp = scale_resp.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    // Own scales: drag to zoom, double-click to fit, right-click for options.
    for (ci, col, y) in owns.iter_mut() {
        let resp = ui.interact(
            *col,
            Id::new(("own-scale", chart.id, *ci)),
            sense(Sense::click_and_drag()),
        );
        let sc = &mut chart.compares[*ci].scale;
        if resp.dragged() {
            let f = (resp.drag_delta().y as f64 * 0.006).exp();
            let mid = (sc.lo + sc.hi) / 2.0;
            let half = (sc.hi - sc.lo) / 2.0 * f;
            sc.auto = false;
            (sc.lo, sc.hi) = (mid - half, mid + half);
            (y.lo, y.hi) = (sc.lo, sc.hi);
        }
        if resp.double_clicked() {
            sc.auto = true;
        }
        if resp.secondary_clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            actions.push(Action::CompareMenu(*ci, p));
        }
        let _ = resp
            .on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text_at_pointer("Drag to scale · double-click to fit · right-click for options");
    }
    // Time axis: drag to zoom, keeping the right edge.
    let time_resp = ui.interact(
        time_axis,
        Id::new(("chart-time", chart.id)),
        sense(Sense::click_and_drag()),
    );
    if time_resp.dragged() {
        let dx = time_resp.drag_delta().x;
        chart.view.bar_w = (chart.view.bar_w * (dx * 0.006).exp()).clamp(0.4, 80.0);
    }
    if time_resp.double_clicked() {
        chart.reset_view();
    }
    let _ = time_resp.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    // Pane dividers.
    for (k, g) in geos.iter().enumerate().skip(1) {
        let yk = g.rect.top();
        let r = Rect::from_min_max(pos2(plot_all.left(), yk - 3.0), pos2(full.right(), yk + 3.0));
        let resp = ui.interact(r, Id::new(("chart-divider", chart.id, k)), Sense::drag());
        if resp.drag_started() {
            cui.divider_drag = Some(k);
        }
        if resp.dragged() && cui.divider_drag == Some(k) {
            let dy = resp.drag_delta().y;
            let per_px = total_w / plot_all.height().max(1.0);
            let (a, b) = (chart.panes[k - 1].weight, chart.panes[k].weight);
            let dw = (dy * per_px).clamp(-(a - 0.1), b - 0.1);
            chart.panes[k - 1].weight = a + dw;
            chart.panes[k].weight = b - dw;
            chart.touch();
        }
        if resp.drag_stopped() {
            cui.divider_drag = None;
        }
        let _ = resp.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    }
    // Keep the view sensible: some bars always on screen.
    let visible_bars = (plot_all.width() / chart.view.bar_w) as f64;
    let max_right = chart.last_index() + visible_bars * 0.8;
    let min_right = (5.0f64).min(chart.last_index().max(0.0));
    chart.view.right = chart.view.right.clamp(min_right, max_right.max(min_right));
    // Recompute the map after this frame's changes.
    let x = XMap {
        right: plot_all.right(),
        right_index: chart.view.right,
        bar_w: chart.view.bar_w,
    };
    for (g, pane) in geos.iter_mut().zip(&chart.panes) {
        if !pane.scale.auto {
            g.y.lo = pane.scale.lo;
            g.y.hi = pane.scale.hi;
        }
    }

    // ---- grid
    let ticks = scale::time_ticks(&chart.bars, first, last, chart.view.bar_w, 90.0, chart.tf, &chart.tz);
    if chart.style.show_grid_vert {
        for tk in &ticks {
            let xx = snap(x.x(tk.index as f64), ppp);
            painter.vline(xx, plot_all.y_range(), Stroke::new(1.0, pal.grid_v));
        }
    }
    let mut pane_ticks: Vec<Vec<(f64, String)>> = Vec::new();
    for g in &geos {
        let fmt = value_format(chart, g.format);
        let list: Vec<(f64, String)> = match g.pct_base {
            Some(base) if base > 0.0 => {
                let (plo, phi) = ((g.y.lo / base - 1.0) * 100.0, (g.y.hi / base - 1.0) * 100.0);
                let (vals, step) = scale::price_ticks(plo, phi, g.rect.height(), 48.0);
                let d = format::decimals_for(step, 0);
                vals.into_iter()
                    .map(|p| {
                        let p = if p.abs() < step * 1e-6 { 0.0 } else { p };
                        (base * (1.0 + p / 100.0), format!("{p:.d$}%"))
                    })
                    .collect()
            }
            _ => {
                let (vals, step) = if g.y.log {
                    log_ticks(g.y.lo, g.y.hi, g.rect.height())
                } else {
                    scale::price_ticks(g.y.lo, g.y.hi, g.rect.height(), 48.0)
                };
                let d = format::decimals_for(step, 0);
                vals.into_iter()
                    .map(|v| {
                        // No "-0" labels.
                        let v = if v.abs() < step * 1e-6 { 0.0 } else { v };
                        let s = match g.format {
                            Format::Volume => format::compact(v),
                            Format::Price => format::price(v, d.max(chart.precision().min(d))),
                            Format::Fixed(_) => format::price(v, d),
                        };
                        (v, if s.is_empty() { fmt(v) } else { s })
                    })
                    .collect()
            }
        };
        if chart.style.show_grid_horz {
            for (v, _) in &list {
                let yy = snap(g.y.y(*v), ppp);
                if yy > g.rect.top() + 2.0 && yy < g.rect.bottom() - 2.0 {
                    painter.hline(g.rect.x_range(), yy, Stroke::new(1.0, pal.grid_h));
                }
            }
        }
        pane_ticks.push(list);
    }

    // ---- watermark
    if chart.style.watermark
        && let Some(g) = geos.first()
    {
        painter.text(
            g.rect.center(),
            Align2::CENTER_CENTER,
            format!("{}, {}", chart.symbol.ticker(), chart.tf.label()),
            theme::display((g.rect.width() / 12.0).clamp(18.0, 56.0)),
            motion::with_alpha(t.text, if t.dark { 0.05 } else { 0.06 }),
        );
    }

    // ---- main pane: bottom studies (volume), series, overlays
    let clip_painter = |r: Rect| painter.with_clip_rect(r);
    if let Some(g) = geos.first() {
        let p = clip_painter(g.rect);
        // Volume and friends: their own scale in the bottom fifth.
        for s in chart
            .studies
            .iter()
            .filter(|s| s.pane == 0 && s.shown(chart.tf) && s.meta().placement == Placement::Bottom)
        {
            let mut vmax = 0.0f64;
            for (pi, look) in s.look.plots.iter().enumerate() {
                if look.visible {
                    for (_, v, _) in plot_points(s, pi, first, last, n) {
                        vmax = vmax.max(v);
                    }
                }
            }
            let bottom_y = YMap {
                top: g.rect.bottom() - g.rect.height() * 0.2,
                bottom: g.rect.bottom(),
                lo: 0.0,
                hi: vmax.max(1e-12),
                log: false,
            };
            draw_study(&p, &chart.bars, s, &bottom_y, &x, first, last, n, ppp, g.rect);
        }
        for s in chart.studies.iter().filter(|s| s.pane == 0 && s.shown(chart.tf)) {
            draw_bg(&p, s, &x, first, last, g.rect);
        }
        draw_series(&p, chart, &g.y, &x, first, last, ppp, g.rect);
        draw_compares(&p, chart, &g.y, &x, first, last, &owns);
        super::profile::paint_overlays(&p, chart, &g.y, &x, first, last, g.rect);
        for s in chart
            .studies
            .iter()
            .filter(|s| s.pane == 0 && s.shown(chart.tf) && s.meta().placement == Placement::Overlay)
        {
            draw_study(&p, &chart.bars, s, &g.y, &x, first, last, n, ppp, g.rect);
        }
        if let Some(m) = &chart.marks {
            draw_trades(&p, m, &chart.bars, &g.y, &x, first, last, g.rect);
        }
        if let Some(c) = &chart.corporate {
            draw_corporate(ui.ctx(), &p, c, chart, &x, first, last, g.rect);
        }
        draw_paper(&p, &chart.paper, cui.paper_drag, &g.y, g.rect);
    }
    // ---- sub panes
    for (k, g) in geos.iter().enumerate().skip(1) {
        let pane_id = chart.panes[k].id;
        let p = clip_painter(g.rect);
        for s in chart.studies.iter().filter(|s| s.pane == pane_id && s.shown(chart.tf)) {
            draw_bg(&p, s, &x, first, last, g.rect);
        }
        for c in chart
            .compares
            .iter()
            .filter(|c| c.visible && c.mode == CompareMode::Pane && c.pane == pane_id)
        {
            draw_compare_candles(&p, chart, c, &g.y, &x, first, last);
        }
        for s in chart.studies.iter().filter(|s| s.pane == pane_id && s.shown(chart.tf)) {
            if s.meta().id == "seasonality" {
                let years = cui.season.get(chart, s.inst.inputs.len("years"));
                super::profile::paint_seasonality(&p, years, &g.y, g.rect, pal.text2);
                continue;
            }
            draw_study(&p, &chart.bars, s, &g.y, &x, first, last, n, ppp, g.rect);
        }
        // Divider line above the pane.
        painter.hline(
            full.x_range(),
            snap(g.rect.top(), ppp),
            Stroke::new(DIVIDER, pal.border),
        );
    }

    // ---- drawings, on the main pane
    if let Some(g) = geos.first() {
        super::drawing_ui::paint(&painter, chart, &cui.draw, tools, x, g.y, g.rect, pal.bg);
        cui.main_rect = g.rect;
    }

    // ---- scales
    painter.vline(
        snap(plot_all.right(), ppp),
        full.y_range(),
        Stroke::new(1.0, pal.border),
    );
    painter.hline(
        full.x_range(),
        snap(plot_all.bottom(), ppp),
        Stroke::new(1.0, pal.border),
    );
    for (g, list) in geos.iter().zip(&pane_ticks) {
        for (v, label) in list {
            let yy = g.y.y(*v);
            if yy > g.rect.top() + 6.0 && yy < g.rect.bottom() - 6.0 {
                painter.text(
                    pos2(scale_col.left() + 8.0, yy),
                    Align2::LEFT_CENTER,
                    label,
                    scale_font.clone(),
                    pal.scale_text,
                );
            }
        }
    }
    if let Some(g) = geos.first() {
        for (ci, col, y) in &owns {
            let c = &chart.compares[*ci];
            painter.vline(snap(col.left(), ppp), col.y_range(), Stroke::new(1.0, pal.border));
            let (vals, step) = scale::price_ticks(y.lo, y.hi, g.rect.height(), 48.0);
            let d = format::decimals_for(step, 0);
            for v in vals {
                let yy = y.y(v);
                if yy > g.rect.top() + 6.0 && yy < g.rect.bottom() - 6.0 {
                    painter.text(
                        pos2(col.left() + 8.0, yy),
                        Align2::LEFT_CENTER,
                        format::price(v, d.max(c.symbol.precision().min(d))),
                        scale_font.clone(),
                        pal.scale_text,
                    );
                }
            }
        }
    }
    for tk in &ticks {
        let xx = x.x(tk.index as f64);
        if xx > time_axis.left() + 16.0 && xx < time_axis.right() - 16.0 {
            let font = if tk.weight >= 60 {
                theme::semibold(12.0)
            } else {
                scale_font.clone()
            };
            painter.text(
                pos2(xx, time_axis.center().y),
                Align2::CENTER_CENTER,
                &tk.label,
                font,
                pal.scale_text,
            );
        }
    }

    // ---- last-value labels on the price scale
    let tag_h = 18.0;
    for (k, g) in geos.iter().enumerate() {
        let pane_id = chart.panes[k].id;
        let mut tags: Vec<(f32, String, Color32, Option<String>)> = Vec::new();
        if k == 0
            && let Some(b) = chart.bars.last()
        {
            let up = bar_color(chart, n - 1, b);
            let col = c32(if up { chart.style.up_body } else { chart.style.down_body });
            let yy = g.y.y(b.close);
            if chart.style.last_price_line && yy >= g.rect.top() && yy <= g.rect.bottom() {
                styled_hline(
                    &clip_painter(g.rect),
                    g.rect.left(),
                    g.rect.right(),
                    snap(yy, ppp),
                    Stroke::new(1.0, col),
                    opentern_series::study::LineStyle::Dotted,
                );
            }
            if chart.style.last_price_label {
                let text = match g.pct_base {
                    Some(base) if base > 0.0 => format::signed_pct((b.close / base - 1.0) * 100.0),
                    _ => format::price(b.close, prec),
                };
                let countdown =
                    (chart.style.countdown && chart.status == Status::Live && !chart.style.kind.time_free())
                        .then(|| chart.tf.millis())
                        .flatten()
                        .map(|len| format::countdown(b.time + len - jiff::Timestamp::now().as_millisecond()));
                tags.push((yy, text, col, countdown));
            }
        }
        if chart.style.study_labels {
            for s in chart
                .studies
                .iter()
                .filter(|s| s.pane == pane_id && s.shown(chart.tf) && s.look.labels_on_scale)
            {
                if s.meta().placement == Placement::Bottom || n == 0 {
                    continue;
                }
                let fmt = value_format(chart, s.meta().format);
                for (pi, look) in s.look.plots.iter().enumerate() {
                    let off = s.meta().offset(pi, &s.inst.inputs);
                    if !look.visible || off != 0 {
                        continue;
                    }
                    let v = s.inst.out.plots[pi][n - 1];
                    if !v.is_finite() {
                        continue;
                    }
                    let ci = s.inst.out.colors[pi][n - 1] as usize;
                    let col = c32(*look.colors.get(ci).unwrap_or(&look.colors[0])).to_opaque();
                    tags.push((g.y.y(v), fmt(v), col, None));
                }
            }
        }
        for c in chart
            .compares
            .iter()
            .filter(|c| c.visible && c.mode == CompareMode::Pane && c.pane == pane_id)
        {
            if let Some(b) = c.bars.last() {
                tags.push((
                    g.y.y(b.close),
                    format::price(b.close, c.symbol.precision()),
                    c32(c.color),
                    None,
                ));
            }
        }
        tags.retain(|t| t.0 >= g.rect.top() - 1.0 && t.0 <= g.rect.bottom() + 1.0);
        let mut boxes: Vec<(f32, f32, usize)> = tags
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let h = if t.3.is_some() { tag_h * 2.0 - 2.0 } else { tag_h };
                (t.0 - tag_h / 2.0, h, i)
            })
            .collect();
        paint::spread(&mut boxes, 1.0, g.rect.top(), g.rect.bottom());
        for (top, h, i) in boxes {
            let (_, text, col, countdown) = &tags[i];
            let r = Rect::from_min_size(pos2(scale_col.left() + 1.0, top), vec2(scale_w - 2.0, h));
            let yy = top + tag_h / 2.0;
            painter.rect_filled(r, CornerRadius::same(3), *col);
            let fg = paint::text_on(*col);
            painter.text(
                pos2(r.left() + 7.0, yy),
                Align2::LEFT_CENTER,
                text,
                theme::medium(chart.style.scale_font as f32 - 0.5),
                fg,
            );
            if let Some(cd) = countdown {
                painter.text(
                    pos2(r.left() + 7.0, yy + tag_h - 3.0),
                    Align2::LEFT_CENTER,
                    cd,
                    small.clone(),
                    fg,
                );
            }
        }
    }
    if let Some(g) = geos.first() {
        for (ci, col, y) in &owns {
            let c = &chart.compares[*ci];
            let Some(b) = c.bars.last() else { continue };
            let yy = y.y(b.close);
            if yy >= g.rect.top() && yy <= g.rect.bottom() {
                let r = Rect::from_min_size(pos2(col.left() + 1.0, yy - tag_h / 2.0), vec2(col.width() - 2.0, tag_h));
                let bg = c32(c.color);
                painter.rect_filled(r, CornerRadius::same(3), bg);
                painter.text(
                    pos2(r.left() + 7.0, yy),
                    Align2::LEFT_CENTER,
                    format::price(b.close, c.symbol.precision()),
                    theme::medium(chart.style.scale_font as f32 - 0.5),
                    paint::text_on(bg),
                );
            }
        }
    }
    if chart.style.countdown && chart.status == Status::Live && chart.tf.millis().is_some() {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
    }

    // ---- the quick measure's ends, marked on both axes
    if let (Some(m), Some(g)) = (cui.draw.measure(), geos.first()) {
        let col = super::draw::measure_color(m.b.p >= m.a.p);
        let tm = super::draw::TimeMap {
            bars: &chart.bars,
            step: chart.tf.millis().unwrap_or(30 * 86_400_000),
            x: XMap { ..x },
        };
        let (ya, yb) = (g.y.y(m.a.p), g.y.y(m.b.p));
        let band = Rect::from_x_y_ranges(scale_col.x_range(), ya.min(yb)..=ya.max(yb)).intersect(g.rect);
        painter.rect_filled(band, CornerRadius::ZERO, col.gamma_multiply(0.18));
        let (xa, xb) = (tm.x(m.a.t), tm.x(m.b.t));
        let tband = Rect::from_x_y_ranges(xa.min(xb)..=xa.max(xb), time_axis.y_range()).intersect(time_axis);
        painter.rect_filled(tband, CornerRadius::ZERO, col.gamma_multiply(0.18));
        for (yy, v) in [(ya, m.a.p), (yb, m.b.p)] {
            if yy >= g.rect.top() && yy <= g.rect.bottom() {
                let r = Rect::from_min_size(
                    pos2(scale_col.left() + 1.0, yy - tag_h / 2.0),
                    vec2(scale_w - 2.0, tag_h),
                );
                painter.rect_filled(r, CornerRadius::same(3), col);
                painter.text(
                    pos2(r.left() + 7.0, yy),
                    Align2::LEFT_CENTER,
                    format::price(v, prec),
                    theme::medium(chart.style.scale_font as f32 - 0.5),
                    Color32::WHITE,
                );
            }
        }
        for (xx, tt) in [(xa, m.a.t), (xb, m.b.t)] {
            if xx >= time_axis.left() && xx <= time_axis.right() {
                let text = format::crosshair_time(tt, chart.tf, &chart.tz);
                let w = painter
                    .layout_no_wrap(text.clone(), small.clone(), Color32::WHITE)
                    .size()
                    .x
                    + 16.0;
                let r = Rect::from_center_size(pos2(xx, time_axis.center().y), vec2(w, tag_h + 2.0));
                painter.rect_filled(r, CornerRadius::same(3), col);
                painter.text(r.center(), Align2::CENTER_CENTER, text, small.clone(), Color32::WHITE);
            }
        }
    }

    // ---- crosshair
    let hover_idx = pointer.filter(|p| plot_all.contains(*p)).map(|p| x.index(p.x).round());
    if let (Some(p), Some(hi)) = (pointer, hover_idx) {
        let xx = snap(x.x(hi), ppp);
        let stroke = Stroke::new(1.0, pal.crosshair);
        styled_vline(
            &painter,
            xx,
            plot_all.top(),
            plot_all.bottom(),
            stroke,
            chart.style.crosshair_style,
        );
        if let Some(g) = geos.iter().find(|g| g.rect.contains(p)) {
            styled_hline(
                &painter,
                g.rect.left(),
                g.rect.right(),
                snap(p.y, ppp),
                stroke,
                chart.style.crosshair_style,
            );
            let v = g.y.value(p.y);
            let text = match g.pct_base {
                Some(base) if base > 0.0 => format::signed_pct((v / base - 1.0) * 100.0),
                _ => value_format(chart, g.format)(v),
            };
            let r = Rect::from_min_size(
                pos2(scale_col.left() + 1.0, p.y - tag_h / 2.0),
                vec2(scale_w - 2.0, tag_h),
            );
            paint::scale_tag(&painter, r, &text, pal.label_bg, scale_font.clone());
            if geos.first().is_some_and(|m| m.rect == g.rect) {
                for (ci, col, y) in &owns {
                    let c = &chart.compares[*ci];
                    let r = Rect::from_min_size(
                        pos2(col.left() + 1.0, p.y - tag_h / 2.0),
                        vec2(col.width() - 2.0, tag_h),
                    );
                    let text = format::price(y.value(p.y), c.symbol.precision());
                    paint::scale_tag(&painter, r, &text, pal.label_bg, scale_font.clone());
                }
            }
        }
        // Time label: extrapolated past the last bar.
        let t = if n > 0 && hi >= 0.0 {
            let i = hi as usize;
            if i < n {
                Some(chart.bars[i].time)
            } else {
                chart
                    .tf
                    .millis()
                    .map(|len| chart.bars[n - 1].time + (i - (n - 1)) as i64 * len)
            }
        } else {
            None
        };
        if let Some(t) = t {
            let text = format::crosshair_time(t, chart.tf, &chart.tz);
            let w = painter
                .layout_no_wrap(text.clone(), small.clone(), Color32::WHITE)
                .size()
                .x
                + 16.0;
            let r = Rect::from_center_size(pos2(xx, time_axis.center().y), vec2(w, tag_h + 2.0));
            paint::scale_tag(&painter, r, &text, pal.label_bg, small.clone());
        }
    }

    cui.hover_time = hover_idx.and_then(|hi| time_at_index(chart, hi));
    if hover_idx.is_none()
        && let Some(t0) = cui.sync_cross
        && n > 0
    {
        // Another chart's crosshair, mirrored at the same time.
        let step = chart.tf.millis().unwrap_or(86_400_000);
        let i = crate::multi::index_at(&chart.bars, step, t0).round();
        let xx = snap(x.x(i), ppp);
        if xx >= plot_all.left() && xx <= plot_all.right() {
            styled_vline(
                &painter,
                xx,
                plot_all.top(),
                plot_all.bottom(),
                Stroke::new(1.0, pal.crosshair),
                chart.style.crosshair_style,
            );
        }
    }
    // ---- bar replay: choosing where to start
    if choosing
        && let Some(hi) = hover_idx
        && n > 0
    {
        let i = hi.clamp(0.0, (n - 1) as f64);
        let xx = snap(x.x(i), ppp);
        let shade = Rect::from_min_max(pos2(xx, plot_all.top()), plot_all.max);
        painter.rect_filled(shade, CornerRadius::ZERO, motion::with_alpha(pal.bg, 0.6));
        painter.vline(xx, plot_all.y_range(), Stroke::new(1.5, t.accent));
        painter.text(
            pos2(xx, plot_all.top() + 14.0),
            Align2::CENTER_CENTER,
            ph::SCISSORS,
            theme::regular(16.0),
            t.accent,
        );
        if plot_resp.clicked() {
            chart.replay_from(i as usize);
        }
    }

    // ---- legend
    let legend_idx = hover_idx
        .filter(|i| *i >= 0.0 && (*i as usize) < n)
        .map(|i| i as usize)
        .or(n.checked_sub(1));
    cui.hover_index = legend_idx;
    legend(ui, chart, &geos, legend_idx, &pal, t, &mut actions, tools.logos);
    if let Some(g) = geos.first() {
        super::drawing_ui::paint_measure(&painter, chart, &cui.draw, x, g.y, g.rect);
    }

    // ---- states with nothing to draw yet
    if n == 0
        && let Some(g) = geos.first()
    {
        let msg = match &chart.status {
            Status::Error(e) => e.clone(),
            Status::Offline(e) => e.clone(),
            _ => "Loading…".to_string(),
        };
        painter.text(
            g.rect.center() + vec2(0.0, 48.0),
            Align2::CENTER_CENTER,
            msg,
            theme::regular(13.0),
            t.text3,
        );
        if chart.status == Status::Loading {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    // ---- scroll to the latest bar
    if n > 0 && !chart.following() {
        let r = Rect::from_center_size(
            pos2(plot_all.right() - 26.0, plot_all.bottom() - 26.0),
            vec2(28.0, 28.0),
        );
        let resp = ui.interact(r, Id::new(("chart-latest", chart.id)), Sense::click());
        let hov = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
        painter.rect_filled(r, CornerRadius::same(14), motion::lerp_color(t.elevated, t.hover, hov));
        painter.rect_stroke(
            r,
            CornerRadius::same(14),
            Stroke::new(1.0, t.border),
            egui::StrokeKind::Inside,
        );
        painter.text(
            r.center(),
            Align2::CENTER_CENTER,
            ph::CARET_DOUBLE_RIGHT,
            theme::regular(14.0),
            t.text2,
        );
        if resp.on_hover_text("Scroll to the most recent bar").clicked() {
            chart.view.right = chart.last_index() + super::RIGHT_MARGIN;
        }
    }
    if chart.replay.is_some()
        && let Some(g) = geos.first()
    {
        super::replay::controls(ui, chart, t, g.rect);
    }
    let _ = scale_resp;
    actions
}

/// The time at (fractional) bar index `i`, extrapolated past the last bar.
fn time_at_index(chart: &Chart, i: f64) -> Option<i64> {
    let n = chart.bars.len();
    if n == 0 || i < 0.0 {
        return None;
    }
    let k = i as usize;
    if k < n {
        Some(chart.bars[k].time)
    } else {
        chart
            .tf
            .millis()
            .map(|len| chart.bars[n - 1].time + (k - (n - 1)) as i64 * len)
    }
}

/// Compared symbols' points on screen bars `first..=last`, in the main
/// pane's units: percent comparisons scaled to start where the chart's
/// first visible close is, plain ones as prices. Overlays (own scale) are
/// left out.
fn compare_points(chart: &Chart, first: usize, last: usize) -> Vec<(usize, Vec<(usize, f64)>)> {
    let n = chart.bars.len();
    if n == 0 || first >= n {
        return Vec::new();
    }
    let base_t = chart.bars[first].time;
    let base = chart.bars[first].close;
    chart
        .compares
        .iter()
        .enumerate()
        .filter(|(_, c)| c.visible && c.mode == CompareMode::Percent)
        .map(|(k, c)| {
            let cbase = c.close_at(base_t).or_else(|| c.bars.first().map(|b| b.close));
            let pts = (first..=last.min(n - 1))
                .filter_map(|j| {
                    let v = c.close_at(chart.bars[j].time)?;
                    match (chart.percent, cbase) {
                        (true, Some(cb)) if cb != 0.0 => Some((j, base * v / cb)),
                        (true, _) => None,
                        (false, _) => Some((j, v)),
                    }
                })
                .collect();
            (k, pts)
        })
        .collect()
}

/// Compared symbols as lines: on the main scale, or on their own.
fn draw_compares(
    p: &Painter,
    chart: &Chart,
    y: &YMap,
    x: &XMap,
    first: usize,
    last: usize,
    owns: &[(usize, Rect, YMap)],
) {
    for (k, pts) in compare_points(chart, first, last) {
        let c = &chart.compares[k];
        let line: Vec<Pos2> = pts.iter().map(|(j, v)| pos2(x.x(*j as f64), y.y(*v))).collect();
        polyline(p, line, Stroke::new(2.0, c32(c.color)));
    }
    let n = chart.bars.len();
    if n == 0 || first >= n {
        return;
    }
    for (ci, _, own) in owns {
        let c = &chart.compares[*ci];
        let line: Vec<Pos2> = (first..=last.min(n - 1))
            .filter_map(|j| c.close_at(chart.bars[j].time).map(|v| pos2(x.x(j as f64), own.y(v))))
            .collect();
        polyline(p, line, Stroke::new(2.0, c32(c.color)));
    }
}

/// A compared symbol's own scale over the main pane: fitted to what's in
/// view (TradingView's margins), or where it was dragged to.
fn own_ymap(chart: &Chart, c: &super::Compare, first: usize, last: usize, rect: Rect) -> YMap {
    let n = chart.bars.len();
    let (lo, hi) = if c.scale.auto && n > 0 && first < n {
        let (lo, hi) = (first..=last.min(n - 1))
            .filter_map(|j| c.close_at(chart.bars[j].time))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), v| (l.min(v), h.max(v)));
        if lo.is_finite() {
            let span = (hi - lo).max(hi.abs() * 1e-6).max(1e-12);
            (lo - span * 0.08, hi + span * 0.1)
        } else {
            (0.0, 1.0)
        }
    } else {
        (c.scale.lo, c.scale.hi)
    };
    YMap {
        top: rect.top(),
        bottom: rect.bottom(),
        lo,
        hi,
        log: false,
    }
}

/// A compared symbol in its own pane, as candles in the chart's colours.
fn draw_compare_candles(p: &Painter, chart: &Chart, c: &super::Compare, y: &YMap, x: &XMap, first: usize, last: usize) {
    let n = chart.bars.len();
    if n == 0 || first >= n {
        return;
    }
    let bw = chart.view.bar_w;
    let body_w = (bw * 0.7).max(1.0);
    let up_c = c32(chart.style.up_body);
    let down_c = c32(chart.style.down_body);
    let mut mesh = egui::Mesh::default();
    let mut prev = None;
    for j in first..=last.min(n - 1) {
        let Some(b) = c.bar_at(chart.bars[j].time) else {
            continue;
        };
        // Each of its bars once (a daily stock bar under hourly crypto).
        if prev == Some(b.time) {
            continue;
        }
        prev = Some(b.time);
        let col = if b.close >= b.open { up_c } else { down_c };
        let cx = x.x(j as f64);
        let (yo, yc) = (y.y(b.open), y.y(b.close));
        let (yh, yl) = (y.y(b.high), y.y(b.low));
        mesh.add_colored_rect(Rect::from_min_max(pos2(cx - 0.5, yh), pos2(cx + 0.5, yl)), col);
        let (t, bt) = (yo.min(yc), yo.max(yc).max(yo.min(yc) + 1.0));
        mesh.add_colored_rect(
            Rect::from_min_max(pos2(cx - body_w / 2.0, t), pos2(cx + body_w / 2.0, bt)),
            col,
        );
    }
    p.add(egui::Shape::mesh(mesh));
}

/// Nice ticks for a log scale: a linear step chosen per decade.
fn log_ticks(lo: f64, hi: f64, height: f32) -> (Vec<f64>, f64) {
    let lo = lo.max(1e-12);
    let (vals, step) = scale::price_ticks(lo, hi, height, 48.0);
    // Linear ticks crowd at the top on a log scale; thin them by spacing.
    let y = |v: f64| (v.ln() - lo.ln()) / (hi.ln() - lo.ln()) * height as f64;
    let mut out: Vec<f64> = Vec::new();
    for v in vals {
        if out.last().is_none_or(|p| y(v) - y(*p) >= 40.0) {
            out.push(v);
        }
    }
    (out, step)
}

#[allow(clippy::too_many_arguments)]
fn draw_series(p: &Painter, chart: &Chart, y: &YMap, x: &XMap, first: usize, last: usize, ppp: f32, rect: Rect) {
    let n = chart.bars.len();
    if n == 0 || first >= n {
        return;
    }
    let st = &chart.style;
    let bw = chart.view.bar_w;
    let px = 1.0 / ppp;
    let up_of = |i: usize| bar_color(chart, i, &chart.bars[i]);
    match st.kind {
        SeriesKind::Line | SeriesKind::LineMarkers | SeriesKind::StepLine | SeriesKind::Area => {
            let mut pts: Vec<Pos2> = Vec::with_capacity((last - first + 1) * 2);
            for i in first..=last {
                let q = pos2(x.x(i as f64), y.y(chart.bars[i].close));
                if st.kind == SeriesKind::StepLine
                    && let Some(prev) = pts.last().copied()
                {
                    pts.push(pos2(q.x, prev.y));
                }
                pts.push(q);
            }
            if st.kind == SeriesKind::Area {
                let mut mesh = Mesh::default();
                paint::area_under(
                    &mut mesh,
                    &pts,
                    rect.bottom(),
                    c32(st.area_top),
                    c32(st.area_bottom),
                    rect.top(),
                );
                p.add(mesh);
            }
            let stroke = Stroke::new(st.line_width as f32, c32(st.line_color));
            if st.kind == SeriesKind::LineMarkers && bw >= 3.0 {
                let r = (st.line_width as f32 + 1.5).min(bw * 0.45);
                for q in &pts {
                    p.circle_filled(*q, r, stroke.color);
                }
            }
            polyline(p, pts, stroke);
        }
        SeriesKind::Baseline => {
            // TradingView's baseline: a level at a share of the pane's
            // height; the line and its fill are green above, red below.
            let base_y = rect.bottom() - rect.height() * st.baseline_level.min(100) as f32 / 100.0;
            let (up, down) = (c32(st.baseline_up), c32(st.baseline_down));
            let (up_f, down_f) = (up.gamma_multiply(0.28), down.gamma_multiply(0.28));
            let mut fill = Mesh::default();
            let mut above: Vec<Pos2> = Vec::new();
            let mut below: Vec<Pos2> = Vec::new();
            let mut segs: Vec<(Vec<Pos2>, bool)> = Vec::new();
            let mut cur: Vec<Pos2> = Vec::new();
            let mut cur_up = true;
            let mut prev: Option<Pos2> = None;
            for i in first..=last {
                let q = pos2(x.x(i as f64), y.y(chart.bars[i].close));
                let q_up = q.y <= base_y;
                if let Some(a) = prev {
                    let a_up = a.y <= base_y;
                    let mut pieces = vec![(a, q)];
                    if a_up != q_up && (q.y - a.y).abs() > 1e-3 {
                        let t = (base_y - a.y) / (q.y - a.y);
                        let m = pos2(a.x + (q.x - a.x) * t, base_y);
                        pieces = vec![(a, m), (m, q)];
                    }
                    for (u, v) in pieces {
                        let seg_up = (u.y + v.y) / 2.0 <= base_y;
                        band_quad(
                            &mut fill,
                            u.x,
                            u.y,
                            base_y,
                            v.x,
                            v.y,
                            base_y,
                            if seg_up { up_f } else { down_f },
                        );
                        if cur.is_empty() || seg_up != cur_up {
                            if cur.len() >= 2 {
                                segs.push((std::mem::take(&mut cur), cur_up));
                            }
                            cur = vec![u];
                            cur_up = seg_up;
                        }
                        cur.push(v);
                    }
                }
                if q_up {
                    above.push(q);
                } else {
                    below.push(q);
                }
                prev = Some(q);
            }
            if cur.len() >= 2 {
                segs.push((cur, cur_up));
            }
            p.add(fill);
            for (pts, u) in segs {
                polyline(p, pts, Stroke::new(st.line_width as f32, if u { up } else { down }));
            }
            paint::styled_hline(
                p,
                rect.left(),
                rect.right(),
                snap(base_y, ppp),
                Stroke::new(1.0, Color32::from_gray(128).gamma_multiply(0.7)),
                opentern_series::study::LineStyle::Dotted,
            );
        }
        SeriesKind::Columns => {
            let mut mesh = Mesh::default();
            let w = if bw >= 3.0 {
                (bw * 0.7).round().max(1.0)
            } else {
                bw.max(px)
            };
            for i in first..=last {
                let up = if i > 0 {
                    chart.bars[i].close >= chart.bars[i - 1].close
                } else {
                    up_of(i)
                };
                let cx = snap(x.x(i as f64), ppp);
                let top = y.y(chart.bars[i].close);
                let c = c32(if up { st.up_body } else { st.down_body }).gamma_multiply(0.85);
                rect_mesh(
                    &mut mesh,
                    Rect::from_min_max(pos2(cx - w / 2.0, top), pos2(cx + w / 2.0, rect.bottom())),
                    c,
                );
            }
            p.add(mesh);
        }
        SeriesKind::HighLow => {
            let mut mesh = Mesh::default();
            let w = if bw >= 3.0 {
                (bw * 0.7).round().max(1.0)
            } else {
                bw.max(px)
            };
            let c = c32(st.line_color);
            for i in first..=last {
                let b = &chart.bars[i];
                let cx = snap(x.x(i as f64), ppp);
                let (yh, yl) = (y.y(b.high), y.y(b.low));
                rect_mesh(
                    &mut mesh,
                    Rect::from_min_max(pos2(cx - w / 2.0, yh), pos2(cx + w / 2.0, yl.max(yh + px))),
                    c,
                );
            }
            p.add(mesh);
            // High and low values over and under the bars, when there's room.
            if bw >= 38.0 {
                let font = theme::regular(10.5);
                for i in first..=last {
                    let b = &chart.bars[i];
                    let cx = x.x(i as f64);
                    let d = chart.precision();
                    p.text(
                        pos2(cx, y.y(b.high) - 2.0),
                        Align2::CENTER_BOTTOM,
                        format::price(b.high, d),
                        font.clone(),
                        c,
                    );
                    p.text(
                        pos2(cx, y.y(b.low) + 2.0),
                        Align2::CENTER_TOP,
                        format::price(b.low, d),
                        font.clone(),
                        c,
                    );
                }
            }
        }
        SeriesKind::Kagi => {
            // Thick (yang) once a stroke rises above the last shoulder,
            // thin (yin) once it falls below the last waist.
            let (up_c, down_c) = (c32(st.up_body), c32(st.down_body));
            let mut yang = chart.bars.first().is_some_and(|b| b.close >= b.open);
            let (mut shoulder, mut waist) = (f64::NAN, f64::NAN);
            let thick = (st.line_width as f32 + 1.0).max(2.0);
            for i in 0..=last {
                let b = &chart.bars[i];
                let up = b.close >= b.open;
                // Where the stroke changes thickness, if it does.
                let flip = if up && !yang && shoulder.is_finite() && b.close > shoulder {
                    Some(shoulder)
                } else if !up && yang && waist.is_finite() && b.close < waist {
                    Some(waist)
                } else {
                    None
                };
                if i >= first {
                    let cx = x.x(i as f64);
                    let style = |yg: bool| Stroke::new(if yg { thick } else { 1.0 }, if yg { up_c } else { down_c });
                    if i > 0 {
                        let px0 = x.x(i as f64 - 1.0);
                        p.line_segment([pos2(px0, y.y(b.open)), pos2(cx, y.y(b.open))], style(yang));
                    }
                    match flip {
                        Some(lvl) => {
                            p.line_segment([pos2(cx, y.y(b.open)), pos2(cx, y.y(lvl))], style(yang));
                            p.line_segment([pos2(cx, y.y(lvl)), pos2(cx, y.y(b.close))], style(!yang));
                        }
                        None => {
                            p.line_segment([pos2(cx, y.y(b.open)), pos2(cx, y.y(b.close))], style(yang));
                        }
                    }
                }
                if flip.is_some() {
                    yang = !yang;
                }
                if up {
                    shoulder = b.close;
                } else {
                    waist = b.close;
                }
            }
        }
        SeriesKind::PointFigure => {
            let size = chart.brick;
            if size.is_nan() || size <= 0.0 {
                return;
            }
            let box_h = (y.y(0.0) - y.y(size)).abs();
            let cell = (bw * 0.8).min(box_h.max(2.0) * 1.2).max(2.0);
            let (up_c, down_c) = (c32(st.up_body), c32(st.down_body));
            let sw = if cell > 8.0 { 1.5 } else { 1.0 };
            for i in first..=last {
                let b = &chart.bars[i];
                let up = b.close >= b.open;
                let cx = x.x(i as f64);
                let (lo, hi) = ((b.low / size).round() as i64, (b.high / size).round() as i64);
                let boxes = (hi - lo).clamp(0, 2000);
                for k in 0..=boxes {
                    let v = (lo + k) as f64 * size;
                    let cy = y.y(v);
                    if cy < rect.top() - cell || cy > rect.bottom() + cell {
                        continue;
                    }
                    let h = (box_h * 0.42).min(cell * 0.45);
                    let wv = cell * 0.4;
                    if up {
                        let s = Stroke::new(sw, up_c);
                        p.line_segment([pos2(cx - wv, cy - h), pos2(cx + wv, cy + h)], s);
                        p.line_segment([pos2(cx - wv, cy + h), pos2(cx + wv, cy - h)], s);
                    } else {
                        p.add(egui::Shape::ellipse_stroke(
                            pos2(cx, cy),
                            vec2(wv, h),
                            Stroke::new(sw, down_c),
                        ));
                    }
                }
            }
        }
        SeriesKind::Candles
        | SeriesKind::HollowCandles
        | SeriesKind::Bars
        | SeriesKind::HeikinAshi
        | SeriesKind::Renko
        | SeriesKind::LineBreak => {
            let bricks = matches!(st.kind, SeriesKind::Renko | SeriesKind::LineBreak);
            let mut bodies = Mesh::default();
            let mut wicks = Mesh::default();
            let body_w = if bw >= 3.0 {
                (bw * 0.8).round().max(1.0)
            } else {
                bw.max(1.0 / ppp)
            };
            let wick_w = if bw >= 3.0 { px.max(1.0) } else { px };
            for i in first..=last {
                let b = &chart.bars[i];
                let up = bar_color(chart, i, b);
                let cx = snap(x.x(i as f64), ppp);
                let (yo, yc, yh, yl) = (y.y(b.open), y.y(b.close), y.y(b.high), y.y(b.low));
                // `barcolor` from a script on the chart wins.
                let over = chart
                    .studies
                    .iter()
                    .filter(|s| s.pane == 0 && s.shown(chart.tf))
                    .filter_map(|s| s.inst.out.bar.get(i).copied())
                    .rfind(|c| *c != 0);
                let pick = |u, d| match over {
                    Some(c) => c32(opentern_series::Rgba(c)),
                    None => c32(if up { u } else { d }),
                };
                if st.kind == SeriesKind::Bars {
                    let c = pick(st.up_body, st.down_body);
                    let w = wick_w.max(1.0);
                    rect_mesh(
                        &mut wicks,
                        Rect::from_min_max(pos2(cx - w / 2.0, yh), pos2(cx + w / 2.0, yl.max(yh + px))),
                        c,
                    );
                    let tick = (bw * 0.4).max(1.0);
                    rect_mesh(
                        &mut wicks,
                        Rect::from_min_max(pos2(cx - tick, yo - w / 2.0), pos2(cx, yo + w / 2.0)),
                        c,
                    );
                    rect_mesh(
                        &mut wicks,
                        Rect::from_min_max(pos2(cx, yc - w / 2.0), pos2(cx + tick, yc + w / 2.0)),
                        c,
                    );
                    continue;
                }
                if st.show_wick && !bricks {
                    rect_mesh(
                        &mut wicks,
                        Rect::from_min_max(pos2(cx - wick_w / 2.0, yh), pos2(cx + wick_w / 2.0, yl.max(yh + px))),
                        pick(st.up_wick, st.down_wick),
                    );
                }
                let (top, bot) = (yo.min(yc), yo.max(yc).max(yo.min(yc) + px));
                let body = Rect::from_min_max(pos2(cx - body_w / 2.0, top), pos2(cx + body_w / 2.0, bot));
                let hollow = st.kind == SeriesKind::HollowCandles && b.close > b.open;
                if st.show_body && !hollow {
                    rect_mesh(&mut bodies, body, pick(st.up_body, st.down_body));
                }
                if (st.show_borders || hollow) && bw >= 3.0 {
                    let bc = pick(st.up_border, st.down_border);
                    let fill = pick(st.up_body, st.down_body);
                    if hollow || bc != fill {
                        let w = px.max(1.0);
                        rect_mesh(
                            &mut bodies,
                            Rect::from_min_max(body.min, pos2(body.right(), body.top() + w)),
                            bc,
                        );
                        rect_mesh(
                            &mut bodies,
                            Rect::from_min_max(pos2(body.left(), body.bottom() - w), body.max),
                            bc,
                        );
                        rect_mesh(
                            &mut bodies,
                            Rect::from_min_max(body.min, pos2(body.left() + w, body.bottom())),
                            bc,
                        );
                        rect_mesh(
                            &mut bodies,
                            Rect::from_min_max(pos2(body.right() - w, body.top()), body.max),
                            bc,
                        );
                    }
                }
            }
            p.add(wicks);
            p.add(bodies);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_study(
    p: &Painter,
    bars: &[Bar],
    s: &StudyView,
    y: &YMap,
    x: &XMap,
    first: usize,
    last: usize,
    n: usize,
    ppp: f32,
    rect: Rect,
) {
    let meta = s.meta();
    // Levels and their fills first, under the plots.
    for (fi, f) in meta.fills.iter().enumerate() {
        let look = &s.look.fills[fi];
        if !look.visible {
            continue;
        }
        match f.between {
            FillBetween::HLines(a, b) => {
                let (ha, hb) = (&s.look.hlines[a], &s.look.hlines[b]);
                if !(ha.visible && hb.visible) {
                    continue;
                }
                let (ya, yb) = (y.y(ha.value), y.y(hb.value));
                let r = Rect::from_min_max(pos2(rect.left(), ya.min(yb)), pos2(rect.right(), ya.max(yb)));
                p.rect_filled(r, CornerRadius::ZERO, c32(look.colors[0]));
            }
            FillBetween::Plots(a, b) => {
                let pa: Vec<(usize, f64, Col)> = plot_points(s, a, first.saturating_sub(1), last + 1, n).collect();
                let pb: std::collections::HashMap<usize, f64> = plot_points(s, b, first.saturating_sub(1), last + 1, n)
                    .map(|(j, v, _)| (j, v))
                    .collect();
                let off_a = meta.offset(a, &s.inst.inputs);
                let mut mesh = Mesh::default();
                for w in pa.windows(2) {
                    let ((j0, a0, _), (j1, a1, _)) = (w[0], w[1]);
                    if j1 != j0 + 1 {
                        continue;
                    }
                    let (Some(&b0), Some(&b1)) = (pb.get(&j0), pb.get(&j1)) else {
                        continue;
                    };
                    let src = (j0 as i64 - off_a).max(0) as usize;
                    let ci = s.inst.out.fills[fi].get(src).copied().unwrap_or(0) as usize;
                    let raw = s
                        .inst
                        .out
                        .fill_raw
                        .get(fi)
                        .and_then(|r| r.get(src))
                        .copied()
                        .unwrap_or(0);
                    let col = if raw != 0 {
                        c32(opentern_series::Rgba(raw))
                    } else {
                        c32(*look.colors.get(ci).unwrap_or(&look.colors[0]))
                    };
                    band_quad(
                        &mut mesh,
                        x.x(j0 as f64),
                        y.y(a0),
                        y.y(b0),
                        x.x(j1 as f64),
                        y.y(a1),
                        y.y(b1),
                        col,
                    );
                }
                p.add(mesh);
            }
        }
    }
    for h in s.look.hlines.iter().filter(|h| h.visible) {
        let yy = snap(y.y(h.value), ppp);
        styled_hline(
            p,
            rect.left(),
            rect.right(),
            yy,
            Stroke::new(h.width as f32, c32(h.color)),
            h.style,
        );
    }
    for (pi, look) in s.look.plots.iter().enumerate() {
        if !look.visible {
            continue;
        }
        let color = |c: Col| match c {
            Col::Pal(ci) => c32(*look.colors.get(ci as usize).unwrap_or(&look.colors[0])),
            Col::Raw(r) => c32(opentern_series::Rgba(r)),
        };
        let width = look.width as f32;
        let pts: Vec<(usize, f64, Col)> = plot_points(s, pi, first, last, n).collect();
        match look.style {
            PlotStyle::Line | PlotStyle::LineBr | PlotStyle::StepLine => {
                let breaks = look.style == PlotStyle::LineBr;
                let step = look.style == PlotStyle::StepLine;
                let mut run: Vec<Pos2> = Vec::new();
                let mut run_col = Col::Pal(0);
                let mut prev_j: Option<usize> = None;
                let flush = |run: &mut Vec<Pos2>, ci: Col| {
                    polyline(p, std::mem::take(run), Stroke::new(width, color(ci)));
                };
                for (j, v, ci) in pts {
                    let pt = pos2(x.x(j as f64), y.y(v));
                    let gap = prev_j.is_some_and(|pj| j != pj + 1);
                    if (gap && breaks) || (ci != run_col && !run.is_empty()) {
                        let last_pt = run.last().copied();
                        flush(&mut run, run_col);
                        // Colour changes keep the line joined.
                        if !(gap && breaks)
                            && let Some(lp) = last_pt
                        {
                            run.push(lp);
                        }
                    }
                    if step && let Some(lp) = run.last().copied() {
                        run.push(pos2(pt.x, lp.y));
                    }
                    run.push(pt);
                    run_col = ci;
                    prev_j = Some(j);
                }
                flush(&mut run, run_col);
            }
            PlotStyle::Histogram | PlotStyle::Columns => {
                let base = meta.plots[pi].base;
                let yb = y.y(base).clamp(rect.top(), rect.bottom());
                let bw = x.bar_w;
                let w = if look.style == PlotStyle::Columns {
                    if bw >= 3.0 {
                        (bw * 0.8).round()
                    } else {
                        bw.max(1.0 / ppp)
                    }
                } else {
                    width.max(1.0 / ppp)
                };
                let mut mesh = Mesh::default();
                for (j, v, ci) in pts {
                    let cx = snap(x.x(j as f64), ppp);
                    let yv = y.y(v);
                    let r = Rect::from_min_max(
                        pos2(cx - w / 2.0, yv.min(yb)),
                        pos2(cx + w / 2.0, yv.max(yb).max(yv.min(yb) + 1.0 / ppp)),
                    );
                    rect_mesh(&mut mesh, r, color(ci));
                }
                p.add(mesh);
            }
            PlotStyle::Area => {
                let base = y.y(meta.plots[pi].base).clamp(rect.top(), rect.bottom());
                let line: Vec<Pos2> = pts.iter().map(|(j, v, _)| pos2(x.x(*j as f64), y.y(*v))).collect();
                let c = color(pts.first().map(|p| p.2).unwrap_or(Col::Pal(0)));
                let mut mesh = Mesh::default();
                paint::area_under(
                    &mut mesh,
                    &line,
                    base,
                    c.gamma_multiply(0.35),
                    c.gamma_multiply(0.35),
                    rect.top(),
                );
                p.add(mesh);
                polyline(p, line, Stroke::new(width, c));
            }
            PlotStyle::Circles => {
                for (j, v, ci) in pts {
                    p.circle_filled(pos2(x.x(j as f64), y.y(v)), width.max(1.0) + 1.0, color(ci));
                }
            }
            PlotStyle::Cross => {
                for (j, v, ci) in pts {
                    let c = pos2(x.x(j as f64), y.y(v));
                    let r = 2.0 + width.max(1.0);
                    let st = Stroke::new(1.5, color(ci));
                    p.line_segment([c - vec2(r, 0.0), c + vec2(r, 0.0)], st);
                    p.line_segment([c - vec2(0.0, r), c + vec2(0.0, r)], st);
                }
            }
            PlotStyle::Hidden => {}
            PlotStyle::Candle | PlotStyle::OhlcBar => {
                // Opens are this plot; highs, lows and closes the next three.
                let part = |k: usize, i: usize| {
                    s.inst
                        .out
                        .plots
                        .get(pi + k)
                        .and_then(|v| v.get(i))
                        .copied()
                        .unwrap_or(f64::NAN)
                };
                let candles = look.style == PlotStyle::Candle;
                let bw = x.bar_w;
                let body_w = if bw >= 3.0 {
                    (bw * 0.7).round().max(1.0)
                } else {
                    1.0 / ppp
                };
                let mut mesh = Mesh::default();
                for (j, o, ci) in pts {
                    let (h, l, c) = (part(1, j), part(2, j), part(3, j));
                    if !(h.is_finite() && l.is_finite() && c.is_finite()) {
                        continue;
                    }
                    let col = color(ci);
                    let cx = snap(x.x(j as f64), ppp);
                    let line_w = (1.0 / ppp).max(1.0);
                    rect_mesh(
                        &mut mesh,
                        Rect::from_min_max(pos2(cx - line_w / 2.0, y.y(h)), pos2(cx + line_w / 2.0, y.y(l))),
                        col,
                    );
                    if candles {
                        let (top, bottom) = (y.y(o.max(c)), y.y(o.min(c)));
                        rect_mesh(
                            &mut mesh,
                            Rect::from_min_max(
                                pos2(cx - body_w / 2.0, top),
                                pos2(cx + body_w / 2.0, bottom.max(top + 1.0 / ppp)),
                            ),
                            col,
                        );
                    } else {
                        let tick = (body_w / 2.0).max(2.0);
                        let (yo, yc) = (y.y(o), y.y(c));
                        rect_mesh(
                            &mut mesh,
                            Rect::from_min_max(pos2(cx - tick, yo - line_w / 2.0), pos2(cx, yo + line_w / 2.0)),
                            col,
                        );
                        rect_mesh(
                            &mut mesh,
                            Rect::from_min_max(pos2(cx, yc - line_w / 2.0), pos2(cx + tick, yc + line_w / 2.0)),
                            col,
                        );
                    }
                }
                p.add(mesh);
            }
            PlotStyle::Shape => {
                let Some(spec) = &meta.plots[pi].shape else { continue };
                for (j, v, ci) in pts {
                    let src = j as i64 - meta.offset(pi, &s.inst.inputs);
                    let Some(b) = (src >= 0).then(|| bars.get(src as usize)).flatten() else {
                        continue;
                    };
                    draw_shape(p, spec, pos2(x.x(j as f64), 0.0), v, b, y, rect, color(ci));
                }
            }
        }
    }
    if let Some(d) = &s.inst.drawings {
        super::scripted::paint(p, d, bars, x, y, rect);
    }
}

/// A strategy's trades: dashed links from entry to exit, an arrow at each
/// fill, and its order label when there's room (TradingView's look).
#[allow(clippy::too_many_arguments)]
fn draw_trades(
    p: &Painter,
    m: &super::TradeMarks,
    bars: &[Bar],
    y: &YMap,
    x: &XMap,
    first: usize,
    last: usize,
    rect: Rect,
) {
    const BUY: Color32 = Color32::from_rgb(0x29, 0x62, 0xFF);
    const SELL: Color32 = Color32::from_rgb(0xF2, 0x36, 0x45);
    for &(a, pa, b, pb, won) in &m.links {
        if b < first || a > last {
            continue;
        }
        let c = if won { BUY } else { SELL }.gamma_multiply(0.7);
        p.extend(egui::Shape::dashed_line(
            &[pos2(x.x(a as f64), y.y(pa)), pos2(x.x(b as f64), y.y(pb))],
            Stroke::new(1.0, c),
            4.0,
            3.0,
        ));
    }
    let room = x.bar_w >= 6.0;
    let font = egui::FontId::proportional(10.5);
    // Several fills on one bar stack their labels.
    let mut stack: std::collections::HashMap<(usize, bool), f32> = std::collections::HashMap::new();
    for f in m.fills.iter().filter(|f| f.bar >= first && f.bar <= last) {
        let Some(bar) = bars.get(f.bar) else { continue };
        let cx = x.x(f.bar as f64);
        let c = if f.buy { BUY } else { SELL };
        let py = y.y(f.price);
        let s = 5.0;
        // An arrow pointing at the fill price, from below (buys) or above.
        let tri = if f.buy {
            vec![pos2(cx, py), pos2(cx - s, py + s * 1.4), pos2(cx + s, py + s * 1.4)]
        } else {
            vec![pos2(cx, py), pos2(cx - s, py - s * 1.4), pos2(cx + s, py - s * 1.4)]
        };
        p.add(egui::Shape::convex_polygon(tri, c, Stroke::NONE));
        if room {
            let off = stack.entry((f.bar, f.buy)).or_insert(0.0);
            let galley = p.layout_no_wrap(f.label.clone(), font.clone(), Color32::WHITE);
            let size = galley.size() + vec2(8.0, 4.0);
            let top = if f.buy {
                y.y(bar.low) + 10.0 + *off
            } else {
                y.y(bar.high) - 10.0 - size.y - *off
            };
            *off += size.y + 2.0;
            let r = Rect::from_min_size(pos2(cx - size.x / 2.0, top), size);
            if r.intersects(rect) {
                p.rect_filled(r, CornerRadius::same(3), c);
                p.galley(r.min + vec2(4.0, 2.0), galley, Color32::WHITE);
            }
        }
    }
    if let Some((avg, long)) = m.position {
        let yy = y.y(avg);
        let c = if long { BUY } else { SELL };
        p.extend(egui::Shape::dashed_line(
            &[pos2(rect.left(), yy), pos2(rect.right(), yy)],
            Stroke::new(1.0, c),
            6.0,
            4.0,
        ));
    }
}

/// Dividends (D) and splits (S) along the bottom of the price pane, as on
/// TradingView, with the details on hover.
#[allow(clippy::too_many_arguments)]
fn draw_corporate(
    ctx: &egui::Context,
    p: &Painter,
    c: &opentern_data::yahoo::Corporate,
    chart: &Chart,
    x: &XMap,
    first: usize,
    last: usize,
    rect: Rect,
) {
    if chart.bars.is_empty() {
        return;
    }
    let tz = &chart.sym_tz;
    let day = |i: usize| opentern_data::resample::to_utc_midnight(chart.bars[i].time, tz);
    // The first bar on or after a date.
    let bar_of = |date: i64| -> Option<usize> {
        let i = chart
            .bars
            .partition_point(|b| opentern_data::resample::to_utc_midnight(b.time, tz) < date);
        (i < chart.bars.len() && day(i) - date < 7 * 86_400_000).then_some(i)
    };
    let hover = ctx.pointer_hover_pos();
    let mut tip: Option<String> = None;
    let r = 7.0;
    let y = rect.bottom() - r - 6.0;
    let fmt_day = |d: i64| {
        jiff::Timestamp::from_millisecond(d)
            .map(|t| t.to_zoned(jiff::tz::TimeZone::UTC).strftime("%d %b %Y").to_string())
            .unwrap_or_default()
    };
    let items = c
        .dividends
        .iter()
        .map(|(d, a)| {
            (
                *d,
                'D',
                format!(
                    "Dividend {} {}\nEx-date {}",
                    trim_num(*a),
                    chart.symbol.currency,
                    fmt_day(*d)
                ),
            )
        })
        .chain(c.splits.iter().map(|(d, ratio)| {
            let text = if *ratio >= 1.0 {
                format!("Split {}:1", trim_num(*ratio))
            } else {
                format!("Reverse split 1:{}", trim_num(1.0 / ratio))
            };
            (*d, 'S', format!("{text}\n{}", fmt_day(*d)))
        }));
    for (date, kind, text) in items {
        let Some(i) = bar_of(date) else { continue };
        if i < first || i > last {
            continue;
        }
        let center = pos2(x.x(i as f64), y);
        let col = if kind == 'D' {
            Color32::from_rgb(0x00, 0x89, 0x7B)
        } else {
            Color32::from_rgb(0xFF, 0x98, 0x00)
        };
        p.circle_filled(center, r, col);
        p.text(
            center,
            Align2::CENTER_CENTER,
            kind.to_string(),
            egui::FontId::proportional(9.5),
            Color32::WHITE,
        );
        if hover.is_some_and(|h| h.distance(center) <= r + 2.0) {
            tip = Some(text);
        }
    }
    if let Some(text) = tip {
        egui::Tooltip::always_open(
            ctx.clone(),
            p.layer_id(),
            egui::Id::new("corporate-tip"),
            egui::PopupAnchor::Pointer,
        )
        .show(|ui| {
            ui.label(text);
        });
    }
}

fn trim_num(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A paper line's tag at the left edge, and the × beside it.
fn paper_geom(p: &Painter, l: &super::PaperLine, yy: f32, rect: Rect) -> (std::sync::Arc<egui::Galley>, Rect, Rect) {
    let g = p.layout_no_wrap(l.label.clone(), egui::FontId::proportional(11.0), Color32::WHITE);
    let tag = Rect::from_min_size(
        pos2(rect.left() + 8.0, yy - g.size().y / 2.0 - 2.0),
        g.size() + vec2(10.0, 4.0),
    );
    let close = Rect::from_min_size(pos2(tag.right() + 2.0, tag.top()), vec2(tag.height(), tag.height()));
    (g, tag, close)
}

/// The paper account's position (solid) and orders (dashed), with a tag
/// at the left edge (drag an order's tag to move it; × cancels).
fn draw_paper(p: &Painter, lines: &[super::PaperLine], drag: Option<(u64, f64)>, y: &YMap, rect: Rect) {
    const BUY: Color32 = Color32::from_rgb(0x29, 0x62, 0xFF);
    const SELL: Color32 = Color32::from_rgb(0xF2, 0x36, 0x45);
    for l in lines {
        let price = match (drag, l.order) {
            (Some((id, v)), Some(o)) if id == o => v,
            _ => l.price,
        };
        let yy = y.y(price);
        if yy < rect.top() || yy > rect.bottom() {
            continue;
        }
        let c = if l.buy { BUY } else { SELL };
        if l.position {
            p.hline(rect.x_range(), yy, Stroke::new(1.0, c));
        } else {
            p.extend(egui::Shape::dashed_line(
                &[pos2(rect.left(), yy), pos2(rect.right(), yy)],
                Stroke::new(1.0, c),
                5.0,
                4.0,
            ));
        }
        let (g, r, close) = paper_geom(p, l, yy, rect);
        p.rect_filled(r, CornerRadius::same(3), c);
        p.galley(r.min + vec2(5.0, 2.0), g, Color32::WHITE);
        p.rect_filled(close, CornerRadius::same(3), c);
        let m = close.center();
        let s = 3.0;
        let st = Stroke::new(1.4, Color32::WHITE);
        p.line_segment([m + vec2(-s, -s), m + vec2(s, s)], st);
        p.line_segment([m + vec2(-s, s), m + vec2(s, -s)], st);
        if drag.is_some_and(|(id, _)| Some(id) == l.order) {
            // The price it will move to, at the right edge.
            let txt = p.layout_no_wrap(format!("{price}"), egui::FontId::proportional(11.0), Color32::WHITE);
            let pr = Rect::from_min_size(
                pos2(rect.right() - txt.size().x - 14.0, yy - txt.size().y / 2.0 - 2.0),
                txt.size() + vec2(10.0, 4.0),
            );
            p.rect_filled(pr, CornerRadius::same(3), c);
            p.galley(pr.min + vec2(5.0, 2.0), txt, Color32::WHITE);
        }
    }
}

/// One `plotshape`/`plotchar` marker.
#[allow(clippy::too_many_arguments)]
fn draw_shape(
    p: &Painter,
    spec: &opentern_series::study::ShapeSpec,
    at: Pos2,
    v: f64,
    b: &Bar,
    y: &YMap,
    rect: Rect,
    c: Color32,
) {
    use opentern_series::study::{Location, ShapeKind};
    let sz = spec.size;
    let gap = 4.0;
    let (cy, below) = match spec.location {
        Location::AboveBar => (y.y(b.high) - gap - sz / 2.0, false),
        Location::BelowBar => (y.y(b.low) + gap + sz / 2.0, true),
        Location::Top => (rect.top() + gap + sz / 2.0, false),
        Location::Bottom => (rect.bottom() - gap - sz / 2.0, true),
        Location::Absolute => (y.y(v), false),
    };
    let c0 = pos2(at.x, cy);
    let h = sz / 2.0;
    let tri = |up: bool| {
        let d = if up { -1.0 } else { 1.0 };
        vec![c0 + vec2(0.0, d * h), c0 + vec2(h, -d * h), c0 + vec2(-h, -d * h)]
    };
    match spec.kind {
        ShapeKind::TriangleUp | ShapeKind::ArrowUp => {
            p.add(egui::Shape::convex_polygon(tri(true), c, Stroke::NONE));
        }
        ShapeKind::TriangleDown | ShapeKind::ArrowDown => {
            p.add(egui::Shape::convex_polygon(tri(false), c, Stroke::NONE));
        }
        ShapeKind::Circle => {
            p.circle_filled(c0, h, c);
        }
        ShapeKind::Square => {
            p.rect_filled(Rect::from_center_size(c0, vec2(sz, sz)), CornerRadius::ZERO, c);
        }
        ShapeKind::Diamond => {
            p.add(egui::Shape::convex_polygon(
                vec![
                    c0 + vec2(0.0, -h),
                    c0 + vec2(h, 0.0),
                    c0 + vec2(0.0, h),
                    c0 + vec2(-h, 0.0),
                ],
                c,
                Stroke::NONE,
            ));
        }
        ShapeKind::Cross => {
            p.line_segment([c0 - vec2(h, 0.0), c0 + vec2(h, 0.0)], Stroke::new(2.0, c));
            p.line_segment([c0 - vec2(0.0, h), c0 + vec2(0.0, h)], Stroke::new(2.0, c));
        }
        ShapeKind::XCross => {
            p.line_segment([c0 - vec2(h, h), c0 + vec2(h, h)], Stroke::new(2.0, c));
            p.line_segment([c0 - vec2(h, -h), c0 + vec2(h, -h)], Stroke::new(2.0, c));
        }
        ShapeKind::Flag => {
            p.line_segment([c0 + vec2(-h * 0.6, h), c0 + vec2(-h * 0.6, -h)], Stroke::new(1.5, c));
            p.add(egui::Shape::convex_polygon(
                vec![
                    c0 + vec2(-h * 0.6, -h),
                    c0 + vec2(h, -h * 0.5),
                    c0 + vec2(-h * 0.6, 0.0),
                ],
                c,
                Stroke::NONE,
            ));
        }
        ShapeKind::LabelUp | ShapeKind::LabelDown => {
            let text = if spec.text.is_empty() { " " } else { spec.text };
            let g = p.layout_no_wrap(text.to_string(), theme::medium(11.0), paint::text_on(c));
            let r = Rect::from_center_size(c0, g.size() + vec2(10.0, 6.0));
            p.rect_filled(r, CornerRadius::same(3), c);
            p.galley(r.min + vec2(5.0, 3.0), g, paint::text_on(c));
            return;
        }
        ShapeKind::Char => {
            p.text(c0, Align2::CENTER_CENTER, spec.text, theme::medium(sz), c);
            return;
        }
    }
    if !spec.text.is_empty() {
        let ty = if below { c0.y + h + 8.0 } else { c0.y - h - 8.0 };
        p.text(
            pos2(c0.x, ty),
            Align2::CENTER_CENTER,
            spec.text,
            theme::regular(11.0),
            c,
        );
    }
}

/// The status line of each pane: symbol and OHLC on the main pane, one row
/// per study with its values, and the eye / settings / remove buttons on
/// hover.
#[allow(clippy::too_many_arguments)]
fn legend(
    ui: &mut Ui,
    chart: &mut Chart,
    geos: &[PaneGeo],
    idx: Option<usize>,
    pal: &Palette,
    t: &Theme,
    actions: &mut Vec<Action>,
    logos: Option<&crate::logos::Logos>,
) {
    // Clipped to the plot, so a long legend never runs into the scale.
    let plot = geos.iter().fold(egui::Rect::NOTHING, |r, g| r.union(g.rect));
    let painter = ui.painter().with_clip_rect(plot);
    let font = theme::regular(12.5);
    let bold = theme::semibold(13.0);
    let n = chart.bars.len();
    let st = chart.style.clone();
    let prec = chart.precision();
    for (k, g) in geos.iter().enumerate() {
        let pane_id = chart.panes[k].id;
        let mut pos = g.rect.left_top() + vec2(10.0, 8.0);
        if k == 0 && st.show_title {
            let title = format!(
                "{} · {} · {}",
                chart.symbol.ticker(),
                chart.tf.label(),
                chart.symbol.exchange()
            );
            let slot = painter.add(egui::Shape::Noop);
            let tpos = match logos {
                Some(l) => {
                    l.paint(&painter, t, &chart.symbol, pos + vec2(9.0, 8.5), 9.0);
                    pos + vec2(24.0, 0.0)
                }
                None => pos,
            };
            let r = painter.text(tpos, Align2::LEFT_TOP, &title, bold.clone(), pal.text);
            let mut xx = r.right() + 10.0;
            // Market open or closed (not for crypto, which never closes).
            let market = opentern_core::calendar::Market::of(&chart.symbol);
            if market != opentern_core::calendar::Market::Crypto {
                let now = jiff::Timestamp::now().as_millisecond();
                let state = opentern_core::calendar::state(market, now);
                let (col, tip) = match &state {
                    opentern_core::calendar::State::Open => (t.pos, "Market open".to_string()),
                    opentern_core::calendar::State::Closed(next) => {
                        let when = next
                            .and_then(|n| jiff::Timestamp::from_millisecond(n).ok())
                            .map(|ts| format!(" · opens {}", ts.to_zoned(chart.tz.clone()).strftime("%a %H:%M")))
                            .unwrap_or_default();
                        (t.text3, format!("Market closed{when}"))
                    }
                };
                let c = pos2(xx + 3.0, pos.y + 9.0);
                painter.circle_filled(c, 3.5, col);
                let hit = Rect::from_center_size(c, vec2(14.0, 14.0));
                ui.interact(hit, ui.id().with("market-state"), Sense::hover())
                    .on_hover_text(tip);
                xx += 16.0;
            }
            if let (true, Some(i)) = (st.show_ohlc, idx) {
                let b = chart.bars[i];
                let up = bar_color(chart, i, &b);
                let vc = c32(if up { st.up_body } else { st.down_body });
                for (l, v) in [("O", b.open), ("H", b.high), ("L", b.low), ("C", b.close)] {
                    let r1 = painter.text(pos2(xx, pos.y + 1.0), Align2::LEFT_TOP, l, font.clone(), pal.text2);
                    let r2 = painter.text(
                        pos2(r1.right() + 3.0, pos.y + 1.0),
                        Align2::LEFT_TOP,
                        format::price(v, prec),
                        font.clone(),
                        vc,
                    );
                    xx = r2.right() + 8.0;
                }
                if st.show_bar_change {
                    let prev = if i > 0 { chart.bars[i - 1].close } else { b.open };
                    let ch = b.close - prev;
                    let pct = if prev != 0.0 { ch / prev * 100.0 } else { 0.0 };
                    let sign = if ch >= 0.0 { "+" } else { "" };
                    let r = painter.text(
                        pos2(xx, pos.y + 1.0),
                        Align2::LEFT_TOP,
                        format!("{sign}{} ({})", format::price(ch, prec), format::signed_pct(pct)),
                        font.clone(),
                        vc,
                    );
                    xx = r.right() + 8.0;
                }
                if st.show_volume {
                    let r = painter.text(
                        pos2(xx, pos.y + 1.0),
                        Align2::LEFT_TOP,
                        format!("Vol {}", format::compact(b.volume)),
                        font.clone(),
                        pal.text2,
                    );
                    xx = r.right() + 8.0;
                }
            }
            legend_backing(&painter, slot, pos, xx - 8.0, LEGEND_ROW + 2.0, pal, t, false);
            pos.y += LEGEND_ROW + 2.0;
            let note = match &chart.status {
                Status::Offline(m) => Some(m.clone()),
                Status::Error(m) if n > 0 => Some(m.clone()),
                _ => None,
            };
            if let Some(m) = note {
                painter.text(
                    pos,
                    Align2::LEFT_TOP,
                    format!("{}  {m}", ph::WIFI_SLASH),
                    theme::regular(12.0),
                    t.warn,
                );
                pos.y += LEGEND_ROW;
            }
        }
        if k == 0 {
            for ci in 0..chart.compares.len() {
                let c = &chart.compares[ci];
                let col = c32(c.color);
                let dim = if c.visible { 1.0 } else { 0.45 };
                let row_rect = Rect::from_min_size(pos - vec2(4.0, 2.0), vec2(g.rect.width() * 0.75, LEGEND_ROW));
                let hovered = ui.rect_contains_pointer(row_rect);
                let slot = painter.add(egui::Shape::Noop);
                let tpos = match logos {
                    Some(l) => {
                        l.paint(&painter, t, &c.symbol, pos + vec2(7.0, 8.0), 7.0);
                        pos + vec2(19.0, 0.0)
                    }
                    None => pos,
                };
                let r = painter.text(
                    tpos,
                    Align2::LEFT_TOP,
                    c.symbol.ticker(),
                    font.clone(),
                    motion::with_alpha(col, dim),
                );
                let mut xx = r.right() + 8.0;
                let value = idx
                    .and_then(|i| chart.bars.get(i))
                    .and_then(|b| c.close_at(b.time))
                    .or_else(|| c.bars.last().map(|b| b.close));
                let text = match (value, &c.status) {
                    (Some(v), _) => format::price(v, c.symbol.precision()),
                    (None, Status::Error(e)) | (None, Status::Offline(e)) => e.clone(),
                    (None, _) => "Loading…".into(),
                };
                let r = painter.text(pos2(xx, pos.y), Align2::LEFT_TOP, text, font.clone(), col);
                xx = r.right() + 8.0;
                if hovered {
                    let mut btn = |glyph: &str, tip: &str, id: &str| -> bool {
                        let r = Rect::from_min_size(pos2(xx, pos.y - 2.0), vec2(20.0, 18.0));
                        xx += 22.0;
                        let resp = ui.interact(r, Id::new(("legend-cmp", chart.id, ci, id)), Sense::click());
                        if resp.hovered() {
                            painter.rect_filled(r, CornerRadius::same(4), t.hover_wash());
                        }
                        painter.text(
                            r.center(),
                            Align2::CENTER_CENTER,
                            glyph,
                            theme::regular(13.0),
                            pal.text2,
                        );
                        resp.on_hover_text(tip).clicked()
                    };
                    let visible = c.visible;
                    let eye = btn(
                        if visible { ph::EYE } else { ph::EYE_SLASH },
                        if visible { "Hide" } else { "Show" },
                        "eye",
                    );
                    let remove = btn(ph::X, "Remove", "remove");
                    let more = btn(ph::DOTS_THREE, "Scale and more", "more");
                    if eye {
                        chart.compares[ci].visible = !visible;
                        chart.touch();
                    }
                    if more {
                        actions.push(Action::CompareMenu(ci, pos2(xx, pos.y + LEGEND_ROW)));
                    }
                    if remove {
                        actions.push(Action::RemoveCompare(ci));
                    }
                }
                legend_backing(&painter, slot, pos, xx - 8.0, LEGEND_ROW, pal, t, hovered);
                let title = ui.interact(row_rect, Id::new(("legend-cmp-row", chart.id, ci)), Sense::click());
                if title.secondary_clicked() {
                    actions.push(Action::CompareMenu(ci, pos2(pos.x, pos.y + LEGEND_ROW)));
                }
                pos.y += LEGEND_ROW;
            }
        }
        let uids: Vec<u64> = chart
            .studies
            .iter()
            .filter(|s| s.pane == pane_id)
            .map(|s| s.uid)
            .collect();
        for uid in uids {
            let Some(s) = chart.studies.iter().find(|s| s.uid == uid) else {
                continue;
            };
            let title = s.title(st.show_study_inputs);
            let dim = if s.visible { 1.0 } else { 0.45 };
            let row_rect = Rect::from_min_size(pos - vec2(4.0, 2.0), vec2(g.rect.width() * 0.75, LEGEND_ROW));
            let hovered = ui.rect_contains_pointer(row_rect);
            let slot = painter.add(egui::Shape::Noop);
            let tr = if st.show_study_titles {
                painter.text(
                    pos,
                    Align2::LEFT_TOP,
                    &title,
                    font.clone(),
                    motion::with_alpha(pal.text, dim),
                )
            } else {
                Rect::from_min_size(pos, vec2(0.0, 14.0))
            };
            let mut xx = tr.right() + 8.0;
            // Buttons sit right after the title while hovered.
            if hovered {
                let mut btn = |glyph: &str, tip: &str, id: &str| -> bool {
                    let r = Rect::from_min_size(pos2(xx, pos.y - 2.0), vec2(20.0, 18.0));
                    xx += 22.0;
                    let resp = ui.interact(r, Id::new(("legend", chart.id, uid, id)), Sense::click());
                    let bg = if resp.hovered() {
                        t.hover_wash()
                    } else {
                        Color32::TRANSPARENT
                    };
                    painter.rect_filled(r, CornerRadius::same(4), bg);
                    painter.text(
                        r.center(),
                        Align2::CENTER_CENTER,
                        glyph,
                        theme::regular(13.0),
                        pal.text2,
                    );
                    resp.on_hover_text(tip).clicked()
                };
                let eye = if s.visible { ph::EYE } else { ph::EYE_SLASH };
                let toggle = btn(eye, if s.visible { "Hide" } else { "Show" }, "eye");
                let settings = btn(ph::GEAR_SIX, "Settings", "gear");
                let source = crate::app::App::has_source(s.meta().id) && btn(ph::CODE, "Source code", "code");
                let remove = btn(ph::X, "Remove", "remove");
                let more = btn(ph::DOTS_THREE, "More", "more");
                if toggle && let Some(s) = chart.study_mut(uid) {
                    s.visible = !s.visible;
                    chart.touch();
                }
                if settings {
                    actions.push(Action::StudySettings(uid));
                }
                if source {
                    actions.push(Action::StudySource(uid));
                }
                if remove {
                    actions.push(Action::RemoveStudy(uid));
                }
                if more {
                    actions.push(Action::StudyMenu(uid, pos2(xx, pos.y + LEGEND_ROW)));
                }
            }
            let Some(s) = chart.studies.iter().find(|s| s.uid == uid) else {
                continue;
            };
            if st.show_study_values
                && s.visible
                && let Some(i) = idx
            {
                let fmt = value_format(chart, s.meta().format);
                for (pi, look) in s.look.plots.iter().enumerate() {
                    if !look.visible {
                        continue;
                    }
                    let off = s.meta().offset(pi, &s.inst.inputs);
                    let src = i as i64 - off;
                    let (v, ci) = if src >= 0 && (src as usize) < n {
                        (s.inst.out.plots[pi][src as usize], s.inst.out.colors[pi][src as usize])
                    } else {
                        (f64::NAN, 0)
                    };
                    let col = c32(*look.colors.get(ci as usize).unwrap_or(&look.colors[0])).to_opaque();
                    let text = if v.is_finite() { fmt(v) } else { "—".into() };
                    let r = painter.text(pos2(xx, pos.y), Align2::LEFT_TOP, text, font.clone(), col);
                    xx = r.right() + 8.0;
                }
            }
            if let Some(err) = chart.study_errors.get(&uid) {
                let first = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(err);
                let r = painter.text(
                    pos2(xx, pos.y),
                    Align2::LEFT_TOP,
                    format!("{}  {}", ph::WARNING_CIRCLE, first.trim()),
                    font.clone(),
                    t.neg,
                );
                xx = r.right() + 8.0;
            }
            legend_backing(&painter, slot, pos, xx - 8.0, LEGEND_ROW, pal, t, hovered);
            // Double-click the title for settings, as on TradingView.
            let title_resp = ui.interact(tr, Id::new(("legend-title", chart.id, uid)), Sense::click());
            if title_resp.double_clicked() {
                actions.push(Action::StudySettings(uid));
            }
            if title_resp.secondary_clicked() {
                actions.push(Action::StudyMenu(uid, tr.left_bottom()));
            }
            pos.y += LEGEND_ROW;
        }
    }
    let _ = FontId::default();
}

/// A soft backing behind one legend row, so the text reads over candles
/// without a box around every line: the chart's background, mostly opaque,
/// a little stronger and edged while the row is hovered. Drawn into `slot`
/// (reserved before the row's text) so it sits underneath.
#[allow(clippy::too_many_arguments)]
fn legend_backing(
    painter: &egui::Painter,
    slot: egui::layers::ShapeIdx,
    pos: Pos2,
    right: f32,
    height: f32,
    pal: &Palette,
    t: &Theme,
    hovered: bool,
) {
    if right <= pos.x {
        return;
    }
    let r = Rect::from_min_max(pos2(pos.x - 6.0, pos.y - 3.0), pos2(right + 6.0, pos.y + height - 3.0));
    let fill = motion::with_alpha(pal.bg, if hovered { 0.94 } else { 0.72 });
    let stroke = if hovered {
        Stroke::new(1.0, t.border)
    } else {
        Stroke::NONE
    };
    painter.set(
        slot,
        egui::epaint::RectShape::new(r, CornerRadius::same(5), fill, stroke, egui::StrokeKind::Inside),
    );
}

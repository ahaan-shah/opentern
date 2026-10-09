//! The chart widget: lays out panes and scales, handles pan and zoom, and
//! paints series, studies, crosshair, legend and scale labels, the way
//! TradingView arranges them.

use super::paint::{self, XMap, YMap, band_quad, polyline, rect_mesh, snap, styled_hline, styled_vline};
use super::scale;
use super::style::{Palette, SeriesKind, c32};
use super::{Chart, StudyView};
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
    MoreHistory,
    ChartSettings,
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
}

impl Default for ChartUi {
    fn default() -> Self {
        ChartUi {
            draw: Default::default(),
            main_rect: Rect::NOTHING,
            hover_index: None,
            scale_drag: None,
            divider_drag: None,
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
    let plot_all = Rect::from_min_max(full.min, pos2(full.right() - scale_w, full.bottom() - TIME_AXIS_H));
    let scale_col = Rect::from_min_max(
        pos2(plot_all.right(), full.top()),
        pos2(full.right(), plot_all.bottom()),
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
            for b in &chart.bars[first.min(n - 1)..=last] {
                match chart.style.kind {
                    SeriesKind::Line | SeriesKind::Area => grow(b.close),
                    _ => {
                        grow(b.high);
                        grow(b.low);
                    }
                }
            }
        }
        let mut format = Format::Price;
        let mut first_study = true;
        for s in chart.studies.iter().filter(|s| s.pane == pane.id && s.shown(chart.tf)) {
            if s.meta().placement == Placement::Bottom {
                continue;
            }
            if !main && first_study {
                format = s.meta().format;
                first_study = false;
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
        // Margins: 10% top, 8% bottom, as TradingView.
        let (top_m, bot_m) = (0.1, 0.08);
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
    let drawing_used = interactive
        && n > 0
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
    if tools.tool.is_drawing() && interactive {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    if plot_resp.dragged() && !drawing_used {
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
            theme::display(56.0),
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
        for s in chart
            .studies
            .iter()
            .filter(|s| s.pane == 0 && s.shown(chart.tf) && s.meta().placement == Placement::Overlay)
        {
            draw_study(&p, &chart.bars, s, &g.y, &x, first, last, n, ppp, g.rect);
        }
    }
    // ---- sub panes
    for (k, g) in geos.iter().enumerate().skip(1) {
        let pane_id = chart.panes[k].id;
        let p = clip_painter(g.rect);
        for s in chart.studies.iter().filter(|s| s.pane == pane_id && s.shown(chart.tf)) {
            draw_bg(&p, s, &x, first, last, g.rect);
        }
        for s in chart.studies.iter().filter(|s| s.pane == pane_id && s.shown(chart.tf)) {
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
                let countdown = (chart.style.countdown && chart.status == Status::Live)
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
    if chart.style.countdown && chart.status == Status::Live && chart.tf.millis().is_some() {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
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

    // ---- legend
    let legend_idx = hover_idx
        .filter(|i| *i >= 0.0 && (*i as usize) < n)
        .map(|i| i as usize)
        .or(n.checked_sub(1));
    cui.hover_index = legend_idx;
    legend(ui, chart, &geos, legend_idx, &pal, t, &mut actions);

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
    let _ = scale_resp;
    actions
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
    match st.kind {
        SeriesKind::Line | SeriesKind::Area => {
            let pts: Vec<Pos2> = (first..=last)
                .map(|i| pos2(x.x(i as f64), y.y(chart.bars[i].close)))
                .collect();
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
            polyline(p, pts, Stroke::new(st.line_width as f32, c32(st.line_color)));
        }
        SeriesKind::Candles | SeriesKind::HollowCandles | SeriesKind::Bars => {
            let mut bodies = Mesh::default();
            let mut wicks = Mesh::default();
            let body_w = if bw >= 3.0 {
                (bw * 0.8).round().max(1.0)
            } else {
                bw.max(1.0 / ppp)
            };
            let px = 1.0 / ppp;
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
                if st.show_wick {
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
fn legend(
    ui: &mut Ui,
    chart: &mut Chart,
    geos: &[PaneGeo],
    idx: Option<usize>,
    pal: &Palette,
    t: &Theme,
    actions: &mut Vec<Action>,
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
            let r = painter.text(pos, Align2::LEFT_TOP, &title, bold.clone(), pal.text);
            let mut xx = r.right() + 10.0;
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
                    painter.text(
                        pos2(xx, pos.y + 1.0),
                        Align2::LEFT_TOP,
                        format!("Vol {}", format::compact(b.volume)),
                        font.clone(),
                        pal.text2,
                    );
                }
            }
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
                let remove = btn(ph::X, "Remove", "remove");
                let more = btn(ph::DOTS_THREE, "More", "more");
                if toggle && let Some(s) = chart.study_mut(uid) {
                    s.visible = !s.visible;
                    chart.touch();
                }
                if settings {
                    actions.push(Action::StudySettings(uid));
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
                painter.text(
                    pos2(xx, pos.y),
                    Align2::LEFT_TOP,
                    format!("{}  {}", ph::WARNING_CIRCLE, first.trim()),
                    font.clone(),
                    t.neg,
                );
            }
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

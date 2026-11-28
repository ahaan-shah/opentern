//! Placing, selecting and moving drawings on the chart.

use super::Chart;
use super::draw::{self, Anchor, DrawStyle, Drawing, Geo, Part, Tool};
use super::paint::{XMap, YMap};
use egui::{Pos2, Rect, Response, Ui};

/// The drawing toolbar's state, as the chart needs it this frame.
pub struct Tools<'a> {
    pub tool: Tool,
    pub magnet: bool,
    pub lock_all: bool,
    pub hide_all: bool,
    /// The style a new drawing of this tool starts with.
    pub style: &'a dyn Fn(Tool) -> DrawStyle,
    /// Symbol logos for the legend.
    pub logos: Option<&'a crate::logos::Logos>,
}

#[derive(Default)]
pub struct DrawUi {
    pub selected: Option<u64>,
    /// More drawings picked with Ctrl (Cmd on a Mac) or a box, besides
    /// `selected`: they move and are deleted together.
    pub also: Vec<u64>,
    /// Ctrl+drag on an empty spot: the box being drawn (from, to).
    boxing: Option<(Pos2, Pos2)>,
    /// The drag moves every selected drawing.
    group_drag: bool,
    pub hovered: Option<u64>,
    placing: Option<Drawing>,
    clicks: usize,
    drag: Option<(u64, Part, Anchor)>,
    zoom_from: Option<Pos2>,
    /// The quick measure on screen (the Measure tool, or Shift+drag).
    measure: Option<Measure>,
    /// The tool finished its job (the Zoom tool then puts itself down).
    pub finished: bool,
}

/// A quick measure between two points; `done` once the second is set.
#[derive(Clone, Copy, Debug)]
pub struct Measure {
    pub a: Anchor,
    pub b: Anchor,
    pub done: bool,
}

impl DrawUi {
    /// A drawing is being placed or dragged, or a measure is under way.
    pub fn busy(&self) -> bool {
        self.placing.is_some() || self.drag.is_some() || self.boxing.is_some() || self.measure.is_some_and(|m| !m.done)
    }

    pub fn measure(&self) -> Option<Measure> {
        self.measure
    }

    /// Shows a finished measure (for screenshots).
    pub fn set_measure(&mut self, a: Anchor, b: Anchor) {
        self.measure = Some(Measure { a, b, done: true });
    }

    /// Escape: stop measuring, stop placing, then deselect.
    pub fn cancel(&mut self) {
        if self.measure.take().is_some() {
            return;
        }
        if self.placing.take().is_none() {
            self.select(None);
        }
        self.zoom_from = None;
        self.boxing = None;
    }

    /// Selects just this drawing (or nothing).
    pub fn select(&mut self, id: Option<u64>) {
        self.selected = id;
        self.also.clear();
    }

    /// Every selected drawing.
    pub fn all_selected(&self) -> Vec<u64> {
        self.selected.into_iter().chain(self.also.iter().copied()).collect()
    }

    pub fn is_selected(&self, id: u64) -> bool {
        self.selected == Some(id) || self.also.contains(&id)
    }

    /// Ctrl+click: adds the drawing to the selection, or takes it out.
    fn toggle(&mut self, id: u64) {
        if self.selected == Some(id) {
            self.selected = self.also.pop();
        } else if let Some(k) = self.also.iter().position(|&x| x == id) {
            self.also.remove(k);
        } else {
            if let Some(prev) = self.selected {
                self.also.push(prev);
            }
            self.selected = Some(id);
        }
    }

    /// Adds drawings to the selection (the box).
    fn add(&mut self, ids: impl IntoIterator<Item = u64>) {
        for id in ids {
            if !self.is_selected(id) {
                if let Some(prev) = self.selected {
                    self.also.push(prev);
                }
                self.selected = Some(id);
            }
        }
    }
}

fn geo<'a>(chart: &'a Chart, x: XMap, y: YMap, rect: Rect) -> Geo<'a> {
    Geo {
        tm: draw::TimeMap {
            bars: &chart.bars,
            step: chart.tf.millis().unwrap_or(30 * 86_400_000),
            x,
        },
        y,
        rect,
        decimals: chart.precision(),
    }
}

/// Handles pointer input for drawings. Returns true when it used the drag
/// (so the chart doesn't pan).
#[allow(clippy::too_many_arguments)]
pub fn handle(
    ui: &Ui,
    chart: &mut Chart,
    du: &mut DrawUi,
    tools: &Tools,
    resp: &Response,
    x: XMap,
    y: YMap,
    rect: Rect,
) -> bool {
    du.finished = false;
    let pointer = ui.ctx().pointer_hover_pos().filter(|p| rect.contains(*p));
    let press = resp.interact_pointer_pos();
    // A drag is recognised a few pixels after the button went down; aim it
    // where the button went down (a thin line may be out of reach by then).
    let origin = ui.input(|i| i.pointer.press_origin()).or(press);
    let anchor_at = |chart: &Chart, p: Pos2| {
        let g = geo(chart, x, y, rect);
        let mut a = g.anchor(p);
        if tools.magnet {
            let bar = g.tm.bar(p.x).map(|i| &chart.bars[i]);
            a.p = draw::magnet(bar, a.p, &y, p.y);
        }
        a
    };
    let span = (y.hi - y.lo).abs();

    // ---- the quick measure: the Measure tool, or Shift with the cross.
    // Click, then click (or drag); the next click clears it.
    let tool_measure = tools.tool == Tool::Measure;
    let shift = ui.input(|i| i.modifiers.shift);
    let active = du.measure.is_some_and(|m| !m.done);
    if tool_measure || (tools.tool == Tool::Cross && (shift || active)) {
        if let (Some(m), Some(p)) = (du.measure.as_mut(), pointer)
            && !m.done
        {
            m.b = anchor_at(chart, p);
        }
        if resp.drag_started()
            && let Some(p) = origin
        {
            let a = anchor_at(chart, p);
            du.measure = Some(Measure { a, b: a, done: false });
        }
        if resp.drag_stopped()
            && let Some(m) = du.measure.as_mut()
        {
            m.done = true;
        }
        if resp.clicked()
            && let Some(p) = press
        {
            let at = anchor_at(chart, p);
            match du.measure.as_mut() {
                Some(m) if !m.done => {
                    m.b = at;
                    m.done = true;
                }
                Some(_) => {
                    du.measure = None;
                    du.finished = tool_measure;
                }
                None => {
                    du.measure = Some(Measure {
                        a: at,
                        b: at,
                        done: false,
                    })
                }
            }
        }
        return true;
    }
    // A finished measure goes on the next press.
    if du.measure.is_some() && (resp.clicked() || resp.drag_started()) {
        du.measure = None;
    }

    // ---- zoom: drag a range
    if tools.tool == Tool::Zoom {
        if resp.drag_started() {
            du.zoom_from = origin;
        }
        if resp.drag_stopped()
            && let (Some(a), Some(b)) = (du.zoom_from.take(), resp.interact_pointer_pos())
        {
            let (i0, i1) = (x.index(a.x.min(b.x)), x.index(a.x.max(b.x)));
            if i1 - i0 >= 2.0 {
                chart.view.bar_w = (rect.width() / (i1 - i0) as f32).clamp(0.4, 80.0);
                chart.view.right = i1;
            }
            du.finished = true;
        }
        return resp.dragged() || resp.drag_stopped();
    }

    // ---- placing a new drawing
    if tools.tool.is_drawing() {
        if tools.tool == Tool::Brush {
            if resp.drag_started()
                && let Some(p) = origin
            {
                let a = anchor_at(chart, p);
                let mut d = draw::start(
                    chart.next_drawing_id(),
                    Tool::Brush,
                    a,
                    (tools.style)(Tool::Brush),
                    &geo(chart, x, y, rect).tm,
                    span,
                );
                d.pts.truncate(1);
                du.placing = Some(d);
            }
            if resp.dragged()
                && let (Some(p), Some(d)) = (press, du.placing.as_mut())
            {
                let a = geo(chart, x, y, rect).anchor(p);
                let last = d.pts.last().copied();
                if last.is_none_or(|l| l.t != a.t || (l.p - a.p).abs() > span * 0.002) {
                    d.pts.push(a);
                }
            }
            if resp.drag_stopped() {
                finish(chart, du);
            }
            return true;
        }
        // Follow the pointer with the point being placed.
        if let (Some(p), Some(d)) = (pointer, du.placing.as_mut()) {
            let a = anchor_at_raw(chart, x, y, rect, p, tools.magnet);
            if let Some(last) = d.pts.last_mut() {
                *last = a;
            }
        }
        // A click (or the start/end of a drag) places a point.
        let place = resp.clicked() || resp.drag_started() || (resp.drag_stopped() && du.placing.is_some());
        let at = if resp.drag_started() { origin } else { press.or(pointer) };
        if place && let Some(p) = at {
            let a = anchor_at(chart, p);
            match du.placing.as_mut() {
                None => {
                    let d = draw::start(
                        chart.next_drawing_id(),
                        tools.tool,
                        a,
                        (tools.style)(tools.tool),
                        &geo(chart, x, y, rect).tm,
                        span,
                    );
                    du.clicks = 1;
                    if tools.tool.points() == 1 {
                        du.placing = Some(d);
                        finish(chart, du);
                    } else {
                        du.placing = Some(d);
                    }
                }
                Some(d) => {
                    // A drag that ends where it started is just a click.
                    let same = d.pts.len() >= 2 && d.pts[d.pts.len() - 2] == a;
                    if !(resp.drag_stopped() && same) {
                        if let Some(last) = d.pts.last_mut() {
                            *last = a;
                        }
                        du.clicks += 1;
                        if du.clicks >= tools.tool.points() {
                            finish(chart, du);
                        } else {
                            d.pts.push(a);
                        }
                    }
                }
            }
        }
        if tools.tool == Tool::Path && resp.double_clicked() && du.placing.is_some() {
            if let Some(d) = du.placing.as_mut() {
                // The double-click placed two extra points.
                d.pts.truncate(d.pts.len().saturating_sub(2).max(2));
            }
            finish(chart, du);
        }
        return true;
    }

    // ---- the cross: hover, select, move
    let g = geo(chart, x, y, rect);
    let hit_at = |p: Pos2| {
        if tools.hide_all {
            return None;
        }
        chart
            .drawings
            .iter()
            .rev()
            .find_map(|d| draw::hit(d, &g, p).map(|part| (d.id, part, d.locked)))
    };
    let hover = pointer.and_then(hit_at);
    du.hovered = hover.map(|h| h.0);
    if let Some((_, part, locked)) = hover {
        let icon = match (part, locked || tools.lock_all) {
            (_, true) => egui::CursorIcon::Default,
            (Part::Point(_), _) => egui::CursorIcon::Grab,
            (Part::Body, _) => egui::CursorIcon::Move,
        };
        ui.ctx().set_cursor_icon(icon);
    }
    if tools.tool == Tool::Eraser {
        if resp.clicked()
            && let Some((id, _, _)) = press.and_then(hit_at)
        {
            chart.push_undo();
            chart.drawings.retain(|d| d.id != id);
            chart.drawings_changed += 1;
        }
        return resp.dragged();
    }
    // Ctrl (Cmd on a Mac) adds to the selection: a click on a drawing, or
    // a box dragged from an empty spot.
    let ctrl = ui.input(|i| i.modifiers.command);
    if resp.clicked() {
        match press.and_then(hit_at).map(|h| h.0) {
            Some(id) if ctrl => du.toggle(id),
            hit if !ctrl => du.select(hit),
            _ => {}
        }
    }
    if ctrl
        && resp.drag_started()
        && let Some(p) = origin
        && hit_at(p).is_none()
    {
        du.boxing = Some((p, p));
    }
    if let Some((from, _)) = du.boxing {
        if let Some(p) = press {
            du.boxing = Some((from, p));
        }
        if resp.drag_stopped() {
            let area = Rect::from_two_pos(from, du.boxing.map_or(from, |b| b.1));
            let inside: Vec<u64> = chart
                .drawings
                .iter()
                .filter(|d| !d.hidden && !tools.hide_all)
                .filter(|d| {
                    d.pts.iter().any(|a| area.contains(g.pos(*a)))
                        || draw::handle_points(d, &g).into_iter().any(|q| area.contains(q))
                })
                .map(|d| d.id)
                .collect();
            du.add(inside);
            du.boxing = None;
        }
        return true;
    }
    if resp.drag_started()
        && let Some(p) = origin
        && let Some((id, part, locked)) = hit_at(p)
        && !locked
        && !tools.lock_all
    {
        let from = g.anchor(p);
        chart.push_undo();
        // Dragging one of several selected drawings by its body moves them all.
        du.group_drag = part == Part::Body && du.is_selected(id) && !du.also.is_empty();
        if !du.group_drag {
            du.select(Some(id));
        }
        du.drag = Some((id, part, from));
    }
    if let Some((id, part, from)) = du.drag {
        if resp.dragged()
            && let Some(p) = press
        {
            // Whole drawings move freely; points snap with the magnet.
            let to = anchor_at_raw(chart, x, y, rect, p, tools.magnet && part != Part::Body);
            let movers = if du.group_drag { du.all_selected() } else { vec![id] };
            let mut list = std::mem::take(&mut chart.drawings);
            {
                let g = geo(chart, x, y, rect);
                let snaps = draw::line_angles(&list, &g);
                for d in list.iter_mut().filter(|d| movers.contains(&d.id) && !d.locked) {
                    match part {
                        // The Text tool sizes and turns by its handles.
                        Part::Point(k @ 1..) if d.tool == Tool::Text => draw::drag_text(d, k, p, &g, &snaps),
                        _ => draw::drag(d, part, from, to, &g.tm),
                    }
                }
            }
            chart.drawings = list;
            chart.drawings_changed += 1;
            du.drag = Some((id, part, to));
        }
        if resp.drag_stopped() {
            du.drag = None;
            du.group_drag = false;
        }
        return true;
    }
    false
}

fn anchor_at_raw(chart: &Chart, x: XMap, y: YMap, rect: Rect, p: Pos2, magnet: bool) -> Anchor {
    let g = geo(chart, x, y, rect);
    let mut a = g.anchor(p);
    if magnet {
        let bar = g.tm.bar(p.x).map(|i| &chart.bars[i]);
        a.p = draw::magnet(bar, a.p, &y, p.y);
    }
    a
}

fn finish(chart: &mut Chart, du: &mut DrawUi) {
    if let Some(d) = du.placing.take() {
        chart.push_undo();
        du.select(Some(d.id));
        chart.drawings.push(d);
        chart.drawings_changed += 1;
        du.finished = true;
    }
    du.clicks = 0;
}

/// Paints the drawings (and the one being placed) on the main pane.
#[allow(clippy::too_many_arguments)]
pub fn paint(
    painter: &egui::Painter,
    chart: &Chart,
    du: &DrawUi,
    tools: &Tools,
    x: XMap,
    y: YMap,
    rect: Rect,
    bg: egui::Color32,
) {
    let g = geo(chart, x, y, rect);
    let p = painter.with_clip_rect(rect);
    if !tools.hide_all {
        for d in &chart.drawings {
            let sel = du.is_selected(d.id) || du.hovered == Some(d.id);
            draw::paint(&p, d, &g, sel, bg);
        }
    }
    if let Some(d) = &du.placing {
        draw::paint(&p, d, &g, true, bg);
    }
    if let Some((a, b)) = du.boxing {
        let r = Rect::from_two_pos(a, b);
        let blue = egui::Color32::from_rgb(41, 98, 255);
        p.rect_filled(r, egui::CornerRadius::ZERO, blue.gamma_multiply(0.12));
        p.rect_stroke(
            r,
            egui::CornerRadius::ZERO,
            egui::Stroke::new(1.0, blue.gamma_multiply(0.7)),
            egui::StrokeKind::Inside,
        );
    }
    if let (Some(a), Some(b)) = (du.zoom_from, painter.ctx().pointer_hover_pos()) {
        let r = Rect::from_two_pos(egui::pos2(a.x, rect.top()), egui::pos2(b.x, rect.bottom()));
        p.rect_filled(
            r,
            egui::CornerRadius::ZERO,
            egui::Color32::from_rgba_unmultiplied(41, 98, 255, 30),
        );
    }
}

/// Paints the quick measure, if any (after the legend, so it's on top).
pub fn paint_measure(painter: &egui::Painter, chart: &Chart, du: &DrawUi, x: XMap, y: YMap, rect: Rect) {
    if let Some(m) = du.measure {
        let g = geo(chart, x, y, rect);
        let p = painter.with_clip_rect(rect);
        draw::paint_measure(&p, &chart.symbol, &chart.bars, &g, m.a, m.b);
    }
}

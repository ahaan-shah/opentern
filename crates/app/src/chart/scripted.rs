//! Paints what scripts draw with Pine's drawing objects (labels, lines,
//! boxes, tables, line fills and polylines), on the study's pane, above its
//! plots, styled the way TradingView draws them.

use super::draw::TimeMap;
use super::paint::{XMap, YMap};
use super::style::c32;
use crate::theme;
use egui::epaint::{Mesh, PathShape, PathStroke};
use egui::{Align2, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use opentern_core::Bar;
use opentern_series::Rgba;
use opentern_series::drawings::{Drawings, Label, Line, PineBox, Polyline, Table, XLoc};

fn col(c: u32) -> Color32 {
    c32(Rgba(c))
}

/// Text sizes in points, as TradingView's `size.*`.
fn text_size(s: &str) -> f32 {
    match s {
        "tiny" => 8.0,
        "small" => 10.0,
        "large" => 16.0,
        "huge" => 24.0,
        "normal" | "auto" | "" => 12.0,
        n => n.parse::<f32>().map(|v| v.clamp(4.0, 64.0)).unwrap_or(12.0),
    }
}

struct Map<'a> {
    tm: TimeMap<'a>,
    y: &'a YMap,
    rect: Rect,
}

impl Map<'_> {
    fn x(&self, x: i64, loc: XLoc) -> f32 {
        match loc {
            XLoc::BarIndex => self.tm.x.x(x as f64),
            XLoc::BarTime => self.tm.x(x),
        }
    }

    fn pt(&self, x: i64, y: f64, loc: XLoc) -> Pos2 {
        pos2(self.x(x, loc), self.y.y(y))
    }

    fn bar(&self, x: i64, loc: XLoc) -> Option<&Bar> {
        let i = match loc {
            XLoc::BarIndex => x,
            XLoc::BarTime => self.tm.index(x).round() as i64,
        };
        usize::try_from(i).ok().and_then(|i| self.tm.bars.get(i))
    }
}

pub fn paint(p: &Painter, d: &Drawings, bars: &[Bar], x: &XMap, y: &YMap, rect: Rect) {
    let step = match bars {
        [.., a, b] => (b.time - a.time).max(1),
        _ => 60_000,
    };
    let m = Map {
        tm: TimeMap {
            bars,
            step,
            x: XMap { ..*x },
        },
        y,
        rect,
    };
    for f in &d.linefills {
        // The fill follows its lines' extensions, as on TradingView.
        let (a1, a2) = extend(
            m.pt(f.a.x1, f.a.y1, f.a.xloc),
            m.pt(f.a.x2, f.a.y2, f.a.xloc),
            &f.a.extend,
            rect,
        );
        let (b1, b2) = extend(
            m.pt(f.b.x1, f.b.y1, f.b.xloc),
            m.pt(f.b.x2, f.b.y2, f.b.xloc),
            &f.b.extend,
            rect,
        );
        let c = col(f.color);
        let mut mesh = Mesh::default();
        for q in [a1, a2, b2, b1] {
            mesh.colored_vertex(q, c);
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        p.add(mesh);
    }
    for b in &d.boxes {
        paint_box(p, &m, b);
    }
    for pl in &d.polylines {
        paint_polyline(p, &m, pl);
    }
    for l in &d.lines {
        paint_line(p, &m, l);
    }
    for l in &d.labels {
        paint_label(p, &m, l);
    }
    for t in &d.tables {
        paint_table(p, rect, t);
    }
}

fn stroke_path(p: &Painter, pts: &[Pos2], s: Stroke, style: &str) {
    if pts.len() < 2 || s.color.a() == 0 {
        return;
    }
    match style {
        "dashed" => p.extend(Shape::dashed_line(pts, s, 7.0, 5.0)),
        "dotted" => p.extend(Shape::dotted_line(pts, s.color, 4.0, (s.width * 0.6).max(1.0))),
        _ => {
            p.add(PathShape::line(pts.to_vec(), PathStroke::new(s.width, s.color)));
        }
    }
}

fn arrow_head(p: &Painter, tip: Pos2, from: Pos2, s: Stroke) {
    let d = (tip - from).normalized();
    if !d.x.is_finite() {
        return;
    }
    let len = 6.0 + s.width * 2.0;
    let n = vec2(-d.y, d.x);
    let base = tip - d * len;
    p.add(Shape::convex_polygon(
        vec![tip, base + n * len * 0.5, base - n * len * 0.5],
        s.color,
        Stroke::NONE,
    ));
}

/// Stretches the segment a→b to the pane's edges.
fn extend(a: Pos2, b: Pos2, mode: &str, rect: Rect) -> (Pos2, Pos2) {
    let dx = b.x - a.x;
    let at = |x: f32| {
        if dx.abs() < 1e-6 {
            None
        } else {
            Some(pos2(x, a.y + (b.y - a.y) * (x - a.x) / dx))
        }
    };
    let (l, r) = (rect.left() - 10.0, rect.right() + 10.0);
    let (left, right) = if a.x <= b.x { (a, b) } else { (b, a) };
    let vertical = |top: bool| pos2(a.x, if top { rect.top() } else { rect.bottom() });
    match mode {
        "left" => (at(l).unwrap_or(vertical(b.y > a.y)), right),
        "right" => (left, at(r).unwrap_or(vertical(b.y < a.y))),
        "both" => (at(l).unwrap_or(vertical(true)), at(r).unwrap_or(vertical(false))),
        _ => (a, b),
    }
}

fn paint_line(p: &Painter, m: &Map, l: &Line) {
    let (a, b) = (m.pt(l.x1, l.y1, l.xloc), m.pt(l.x2, l.y2, l.xloc));
    if !(a.y.is_finite() && b.y.is_finite()) {
        return;
    }
    let (s0, s1) = extend(a, b, &l.extend, m.rect);
    let s = Stroke::new(l.width as f32, col(l.color));
    let style = match &*l.style {
        "dashed" | "dotted" => &*l.style,
        _ => "solid",
    };
    stroke_path(p, &[s0, s1], s, style);
    if matches!(&*l.style, "arrow_right" | "arrow_both") {
        arrow_head(p, b, a, s);
    }
    if matches!(&*l.style, "arrow_left" | "arrow_both") {
        arrow_head(p, a, b, s);
    }
}

fn paint_box(p: &Painter, m: &Map, b: &PineBox) {
    let (mut l, mut r) = (m.x(b.left, b.xloc), m.x(b.right, b.xloc));
    let (t, btm) = (m.y.y(b.top), m.y.y(b.bottom));
    if !(t.is_finite() && btm.is_finite()) {
        return;
    }
    if matches!(&*b.extend, "left" | "both") {
        l = m.rect.left() - 10.0;
    }
    if matches!(&*b.extend, "right" | "both") {
        r = m.rect.right() + 10.0;
    }
    let rr = Rect::from_two_pos(pos2(l, t), pos2(r, btm));
    p.rect_filled(rr, CornerRadius::ZERO, col(b.bgcolor));
    if b.border_width > 0.0 && b.border_color & 0xFF != 0 {
        let s = Stroke::new(b.border_width as f32, col(b.border_color));
        let c = [
            rr.left_top(),
            rr.right_top(),
            rr.right_bottom(),
            rr.left_bottom(),
            rr.left_top(),
        ];
        stroke_path(p, &c, s, &b.border_style);
    }
    if !b.text.is_empty() {
        let size = match &*b.text_size {
            // Auto fits the text to the box.
            "auto" | "" => (rr.height() * 0.3).clamp(8.0, 14.0),
            s => text_size(s),
        };
        let g = p.layout(
            b.text.to_string(),
            theme::regular(size),
            col(b.text_color),
            (rr.width() - 8.0).max(10.0),
        );
        let ax = match &*b.text_halign {
            "left" => 0.0,
            "right" => 1.0,
            _ => 0.5,
        };
        let ay = match &*b.text_valign {
            "top" => 0.0,
            "bottom" => 1.0,
            _ => 0.5,
        };
        let inner = rr.shrink(4.0);
        let at = pos2(
            inner.left() + (inner.width() - g.size().x) * ax,
            inner.top() + (inner.height() - g.size().y) * ay,
        );
        p.with_clip_rect(rr.intersect(p.clip_rect()))
            .galley(at, g, Color32::WHITE);
    }
}

fn paint_polyline(p: &Painter, m: &Map, pl: &Polyline) {
    let mut pts: Vec<Pos2> = pl
        .points
        .iter()
        .map(|&(x, y)| m.pt(x, y, pl.xloc))
        .filter(|q| q.y.is_finite())
        .collect();
    if pts.len() < 2 {
        return;
    }
    if pl.curved {
        pts = catmull_rom(&pts, pl.closed);
    }
    if pl.closed && pl.fill_color & 0xFF != 0 {
        p.add(PathShape::convex_polygon(pts.clone(), col(pl.fill_color), Stroke::NONE));
    }
    if pl.closed {
        pts.push(pts[0]);
    }
    stroke_path(
        p,
        &pts,
        Stroke::new(pl.line_width as f32, col(pl.line_color)),
        &pl.line_style,
    );
}

/// A smooth curve through the points.
fn catmull_rom(pts: &[Pos2], closed: bool) -> Vec<Pos2> {
    let n = pts.len();
    let get = |i: isize| -> Pos2 {
        if closed {
            pts[i.rem_euclid(n as isize) as usize]
        } else {
            pts[i.clamp(0, n as isize - 1) as usize]
        }
    };
    let segs = if closed { n } else { n - 1 };
    let mut out = Vec::with_capacity(segs * 12 + 1);
    for i in 0..segs as isize {
        let (p0, p1, p2, p3) = (get(i - 1), get(i), get(i + 1), get(i + 2));
        for k in 0..12 {
            let t = k as f32 / 12.0;
            let (t2, t3) = (t * t, t * t * t);
            let f = |a: f32, b: f32, c: f32, d: f32| {
                0.5 * (2.0 * b
                    + (-a + c) * t
                    + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
                    + (-a + 3.0 * b - 3.0 * c + d) * t3)
            };
            out.push(pos2(f(p0.x, p1.x, p2.x, p3.x), f(p0.y, p1.y, p2.y, p3.y)));
        }
    }
    if !closed {
        out.push(pts[n - 1]);
    }
    out
}

fn paint_label(p: &Painter, m: &Map, l: &Label) {
    let x = m.x(l.x, l.xloc);
    let style = &*l.style;
    let size = text_size(&l.size);
    // Shape styles: a marker with the text beside it.
    let shape = matches!(
        style,
        "xcross"
            | "cross"
            | "triangleup"
            | "triangledown"
            | "flag"
            | "circle"
            | "arrowup"
            | "arrowdown"
            | "square"
            | "diamond"
    );
    let marker = size * 0.9;
    let y = match &*l.yloc {
        "abovebar" => m
            .bar(l.x, l.xloc)
            .map(|b| m.y.y(b.high) - 6.0 - if shape { marker / 2.0 } else { 0.0 }),
        "belowbar" => m
            .bar(l.x, l.xloc)
            .map(|b| m.y.y(b.low) + 6.0 + if shape { marker / 2.0 } else { 0.0 }),
        _ => Some(m.y.y(l.y)),
    };
    let Some(y) = y.filter(|y| y.is_finite()) else { return };
    let at = pos2(x, y);
    let (bg, fg) = (col(l.color), col(l.textcolor));
    if shape || style == "none" || style == "text_outline" {
        if shape {
            paint_marker(p, style, at, marker, bg);
        }
        if !l.text.is_empty() {
            // Text sits away from the bar for markers above or below it.
            let below = l.yloc.as_ref() == "belowbar" || matches!(style, "triangleup" | "arrowup");
            let g = p.layout(l.text.to_string(), theme::medium(size), fg, 400.0);
            let off = if shape { marker / 2.0 + 3.0 } else { 0.0 };
            let pos = if !shape {
                at - g.size() / 2.0
            } else if below {
                pos2(x - g.size().x / 2.0, y + off)
            } else {
                pos2(x - g.size().x / 2.0, y - off - g.size().y)
            };
            p.galley(pos, g, fg);
        }
        return;
    }
    // A bubble with a pointer toward the point.
    let g = p.layout(l.text.to_string(), theme::medium(size), fg, 400.0);
    let pad = vec2(size * 0.5, size * 0.3);
    let body = if l.text.is_empty() {
        vec2(size, size)
    } else {
        g.size() + pad * 2.0
    };
    let tip = 6.0_f32;
    let (anchor, dir) = match style {
        "label_up" => (Align2::CENTER_TOP, vec2(0.0, 1.0)),
        "label_left" => (Align2::LEFT_CENTER, vec2(1.0, 0.0)),
        "label_right" => (Align2::RIGHT_CENTER, vec2(-1.0, 0.0)),
        "label_center" => (Align2::CENTER_CENTER, vec2(0.0, 0.0)),
        "label_upper_left" => (Align2::LEFT_TOP, vec2(0.0, 0.0)),
        "label_upper_right" => (Align2::RIGHT_TOP, vec2(0.0, 0.0)),
        "label_lower_left" => (Align2::LEFT_BOTTOM, vec2(0.0, 0.0)),
        "label_lower_right" => (Align2::RIGHT_BOTTOM, vec2(0.0, 0.0)),
        _ => (Align2::CENTER_BOTTOM, vec2(0.0, -1.0)),
    };
    let r = anchor.anchor_size(at + dir * tip, body);
    p.rect_filled(r, CornerRadius::same(3), bg);
    if dir != vec2(0.0, 0.0) {
        let n = vec2(-dir.y, dir.x) * 5.0;
        let base = at + dir * (tip + 1.0);
        p.add(Shape::convex_polygon(vec![at, base + n, base - n], bg, Stroke::NONE));
    }
    let ax = match &*l.textalign {
        "left" => r.left() + pad.x,
        "right" => r.right() - pad.x - g.size().x,
        _ => r.center().x - g.size().x / 2.0,
    };
    p.galley(pos2(ax, r.center().y - g.size().y / 2.0), g, fg);
}

fn paint_marker(p: &Painter, style: &str, at: Pos2, s: f32, c: Color32) {
    let h = s / 2.0;
    let poly = |pts: Vec<Pos2>| {
        p.add(Shape::convex_polygon(pts, c, Stroke::NONE));
    };
    match style {
        "triangleup" => poly(vec![at + vec2(0.0, -h), at + vec2(h, h), at + vec2(-h, h)]),
        "triangledown" => poly(vec![at + vec2(0.0, h), at + vec2(-h, -h), at + vec2(h, -h)]),
        "arrowup" => {
            poly(vec![at + vec2(0.0, -h), at + vec2(h, 0.0), at + vec2(-h, 0.0)]);
            p.rect_filled(
                Rect::from_center_size(at + vec2(0.0, h * 0.5), vec2(h * 0.7, h)),
                0.0,
                c,
            );
        }
        "arrowdown" => {
            poly(vec![at + vec2(0.0, h), at + vec2(-h, 0.0), at + vec2(h, 0.0)]);
            p.rect_filled(
                Rect::from_center_size(at - vec2(0.0, h * 0.5), vec2(h * 0.7, h)),
                0.0,
                c,
            );
        }
        "circle" => {
            p.circle_filled(at, h, c);
        }
        "square" => {
            p.rect_filled(Rect::from_center_size(at, vec2(s, s)), 0.0, c);
        }
        "diamond" => poly(vec![
            at + vec2(0.0, -h),
            at + vec2(h, 0.0),
            at + vec2(0.0, h),
            at + vec2(-h, 0.0),
        ]),
        "flag" => {
            p.line_segment([at + vec2(-h, h), at + vec2(-h, -h)], Stroke::new(1.5, c));
            poly(vec![at + vec2(-h, -h), at + vec2(h, -h * 0.5), at + vec2(-h, 0.0)]);
        }
        "cross" => {
            let st = Stroke::new(2.0, c);
            p.line_segment([at - vec2(h, 0.0), at + vec2(h, 0.0)], st);
            p.line_segment([at - vec2(0.0, h), at + vec2(0.0, h)], st);
        }
        _ => {
            let st = Stroke::new(2.0, c);
            p.line_segment([at - vec2(h, h), at + vec2(h, h)], st);
            p.line_segment([at + vec2(-h, h), at + vec2(h, -h)], st);
        }
    }
}

#[allow(clippy::needless_range_loop)]
fn paint_table(p: &Painter, pane: Rect, t: &Table) {
    let (cols, rows) = (t.columns, t.rows);
    if cols == 0 || rows == 0 {
        return;
    }
    // Cells covered by a merge (other than its top-left) aren't drawn.
    let mut owner: Vec<Option<(usize, usize)>> = vec![None; cols * rows];
    for &(c0, r0, c1, r1) in &t.merged {
        for r in r0..=r1.min(rows - 1) {
            for c in c0..=c1.min(cols - 1) {
                owner[r * cols + c] = Some((c1.min(cols - 1), r1.min(rows - 1)));
            }
        }
        if c0 < cols && r0 < rows {
            owner[r0 * cols + c0] = None;
        }
    }
    let skip = |c: usize, r: usize| {
        t.merged
            .iter()
            .any(|&(c0, r0, c1, r1)| (c0..=c1).contains(&c) && (r0..=r1).contains(&r) && (c, r) != (c0, r0))
    };
    let pad = vec2(6.0, 3.0);
    let galleys: Vec<_> = t
        .cells
        .iter()
        .map(|c| {
            (!c.text.is_empty()).then(|| {
                p.layout(
                    c.text.to_string(),
                    theme::regular(text_size(&c.text_size)),
                    col(c.text_color),
                    600.0,
                )
            })
        })
        .collect();
    // Column widths and row heights: fixed ones are % of the pane.
    let mut w = vec![0.0_f32; cols];
    let mut h = vec![0.0_f32; rows];
    for r in 0..rows {
        for c in 0..cols {
            let k = r * cols + c;
            if skip(c, r) {
                continue;
            }
            let cell = &t.cells[k];
            let size = galleys[k]
                .as_ref()
                .map(|g| g.size() + pad * 2.0)
                .unwrap_or(vec2(0.0, 0.0));
            let cw = if cell.width > 0.0 {
                pane.width() * cell.width as f32 / 100.0
            } else {
                size.x
            };
            let ch = if cell.height > 0.0 {
                pane.height() * cell.height as f32 / 100.0
            } else {
                size.y
            };
            if owner[k].is_none() {
                w[c] = w[c].max(cw);
                h[r] = h[r].max(ch);
            }
        }
    }
    let (tw, th) = (w.iter().sum::<f32>(), h.iter().sum::<f32>());
    if tw <= 0.0 || th <= 0.0 {
        return;
    }
    let margin = 10.0;
    let (vert, horz) = t.position.split_once('_').unwrap_or(("top", "right"));
    let x0 = match horz {
        "left" => pane.left() + margin,
        "center" => pane.center().x - tw / 2.0,
        _ => pane.right() - margin - tw,
    };
    let y0 = match vert {
        "top" => pane.top() + margin,
        "middle" => pane.center().y - th / 2.0,
        _ => pane.bottom() - margin - th,
    };
    let frame = Rect::from_min_size(pos2(x0, y0), vec2(tw, th));
    p.rect_filled(frame, CornerRadius::ZERO, col(t.bgcolor));
    let xs: Vec<f32> = std::iter::once(x0)
        .chain(w.iter().scan(x0, |a, v| {
            *a += v;
            Some(*a)
        }))
        .collect();
    let ys: Vec<f32> = std::iter::once(y0)
        .chain(h.iter().scan(y0, |a, v| {
            *a += v;
            Some(*a)
        }))
        .collect();
    let border = (t.border_width > 0.0 && t.border_color & 0xFF != 0)
        .then(|| Stroke::new(t.border_width as f32, col(t.border_color)));
    for r in 0..rows {
        for c in 0..cols {
            if skip(c, r) {
                continue;
            }
            let k = r * cols + c;
            let (c1, r1) = t
                .merged
                .iter()
                .find(|m| (m.0, m.1) == (c, r))
                .map(|m| (m.2.min(cols - 1), m.3.min(rows - 1)))
                .unwrap_or((c, r));
            let cr = Rect::from_min_max(pos2(xs[c], ys[r]), pos2(xs[c1 + 1], ys[r1 + 1]));
            let cell = &t.cells[k];
            p.rect_filled(cr, CornerRadius::ZERO, col(cell.bgcolor));
            if let Some(b) = border {
                p.rect_stroke(cr, CornerRadius::ZERO, b, StrokeKind::Inside);
            }
            if let Some(g) = galleys[k].clone() {
                let inner = cr.shrink2(pad);
                let gx = match &*cell.text_halign {
                    "left" => inner.left(),
                    "right" => inner.right() - g.size().x,
                    _ => inner.center().x - g.size().x / 2.0,
                };
                let gy = match &*cell.text_valign {
                    "top" => inner.top(),
                    "bottom" => inner.bottom() - g.size().y,
                    _ => inner.center().y - g.size().y / 2.0,
                };
                p.with_clip_rect(cr.intersect(p.clip_rect()))
                    .galley(pos2(gx, gy), g, Color32::WHITE);
            }
        }
    }
    if t.frame_width > 0.0 && t.frame_color & 0xFF != 0 {
        p.rect_stroke(
            frame,
            CornerRadius::ZERO,
            Stroke::new(t.frame_width as f32, col(t.frame_color)),
            StrokeKind::Inside,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_extend_to_the_pane_edges() {
        let r = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0));
        let (a, b) = extend(pos2(40.0, 50.0), pos2(60.0, 40.0), "right", r);
        assert_eq!(a, pos2(40.0, 50.0));
        assert!((b.x - 110.0).abs() < 1e-3 && (b.y - 15.0).abs() < 1e-3);
        let (a, _) = extend(pos2(40.0, 50.0), pos2(60.0, 40.0), "both", r);
        assert!((a.x + 10.0).abs() < 1e-3 && (a.y - 75.0).abs() < 1e-3);
        assert_eq!(text_size("huge"), 24.0);
        assert_eq!(text_size("14"), 14.0);
    }

    #[test]
    fn curves_pass_through_their_points() {
        let pts = [pos2(0.0, 0.0), pos2(10.0, 10.0), pos2(20.0, 0.0)];
        let c = catmull_rom(&pts, false);
        assert_eq!(c[0], pts[0]);
        assert_eq!(c[12], pts[1]);
        assert_eq!(*c.last().unwrap(), pts[2]);
    }
}

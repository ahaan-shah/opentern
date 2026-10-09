//! Drawing helpers: coordinate maps and batched shapes. Everything that's
//! drawn once per bar goes into one `Mesh` per layer, so a screen full of
//! candles is a handful of draw calls, not thousands.

use egui::epaint::{Mesh, PathShape, PathStroke};
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};
use opentern_series::study::LineStyle;

/// Maps a pane's values to y and back (linear or log).
#[derive(Clone, Copy, Debug)]
pub struct YMap {
    pub top: f32,
    pub bottom: f32,
    pub lo: f64,
    pub hi: f64,
    pub log: bool,
}

impl YMap {
    fn t(&self, v: f64) -> f64 {
        if self.log { v.max(1e-12).ln() } else { v }
    }

    fn inv(&self, v: f64) -> f64 {
        if self.log { v.exp() } else { v }
    }

    pub fn y(&self, v: f64) -> f32 {
        let (lo, hi) = (self.t(self.lo), self.t(self.hi));
        let span = (hi - lo).max(1e-300);
        let f = (self.t(v) - lo) / span;
        let y = self.bottom as f64 - f * (self.bottom - self.top) as f64;
        // Keep far-off values drawable without overflowing.
        y.clamp(-1e6, 1e6) as f32
    }

    pub fn value(&self, y: f32) -> f64 {
        let (lo, hi) = (self.t(self.lo), self.t(self.hi));
        let f = (self.bottom - y) as f64 / (self.bottom - self.top).max(1.0) as f64;
        self.inv(lo + f * (hi - lo))
    }
}

/// Maps bar indices to x and back.
#[derive(Clone, Copy, Debug)]
pub struct XMap {
    pub right: f32,
    pub right_index: f64,
    pub bar_w: f32,
}

impl XMap {
    pub fn x(&self, i: f64) -> f32 {
        self.right - ((self.right_index - i) * self.bar_w as f64) as f32
    }

    pub fn index(&self, x: f32) -> f64 {
        self.right_index - ((self.right - x) / self.bar_w) as f64
    }
}

/// Snaps to the pixel grid so 1-pixel lines stay crisp.
pub fn snap(v: f32, ppp: f32) -> f32 {
    (v * ppp).round() / ppp
}

pub fn rect_mesh(mesh: &mut Mesh, r: Rect, c: Color32) {
    if c.a() > 0 && r.is_positive() {
        mesh.add_colored_rect(r, c);
    }
}

/// A filled quad between two vertical spans (x0: a0..b0, x1: a1..b1),
/// split where the edges cross so it never folds over itself.
#[allow(clippy::too_many_arguments)]
pub fn band_quad(mesh: &mut Mesh, x0: f32, a0: f32, b0: f32, x1: f32, a1: f32, b1: f32, c: Color32) {
    let d0 = a0 - b0;
    let d1 = a1 - b1;
    if d0 * d1 < 0.0 {
        let t = d0 / (d0 - d1);
        let xm = x0 + (x1 - x0) * t;
        let ym = a0 + (a1 - a0) * t;
        tri(mesh, [pos2(x0, a0), pos2(xm, ym), pos2(x0, b0)], c);
        tri(mesh, [pos2(xm, ym), pos2(x1, a1), pos2(x1, b1)], c);
    } else {
        tri(mesh, [pos2(x0, a0), pos2(x1, a1), pos2(x1, b1)], c);
        tri(mesh, [pos2(x0, a0), pos2(x1, b1), pos2(x0, b0)], c);
    }
}

fn tri(mesh: &mut Mesh, p: [Pos2; 3], c: Color32) {
    let i = mesh.vertices.len() as u32;
    for q in p {
        mesh.colored_vertex(q, c);
    }
    mesh.add_triangle(i, i + 1, i + 2);
}

/// A vertical gradient quad strip under a line, from `top_c` at the line
/// to `bottom_c` at `base`.
pub fn area_under(mesh: &mut Mesh, pts: &[Pos2], base: f32, top_c: Color32, bottom_c: Color32, top: f32) {
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let i = mesh.vertices.len() as u32;
        // Colour by height so the gradient is continuous along the line.
        let col = |y: f32| {
            let f = ((y - top) / (base - top).max(1.0)).clamp(0.0, 1.0);
            crate::motion::lerp_color(top_c, bottom_c, f)
        };
        mesh.colored_vertex(a, col(a.y));
        mesh.colored_vertex(b, col(b.y));
        mesh.colored_vertex(pos2(b.x, base), bottom_c);
        mesh.colored_vertex(pos2(a.x, base), bottom_c);
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    }
}

pub fn polyline(painter: &Painter, pts: Vec<Pos2>, stroke: Stroke) {
    if pts.len() >= 2 {
        painter.add(Shape::Path(PathShape::line(
            pts,
            PathStroke::new(stroke.width, stroke.color),
        )));
    } else if let Some(p) = pts.first() {
        painter.circle_filled(*p, stroke.width.max(1.0) * 0.8, stroke.color);
    }
}

/// A horizontal line in a TradingView line style.
pub fn styled_hline(painter: &Painter, x0: f32, x1: f32, y: f32, stroke: Stroke, style: LineStyle) {
    match style {
        LineStyle::Solid => {
            painter.hline(x0..=x1, y, stroke);
        }
        LineStyle::Dashed => {
            painter.extend(Shape::dashed_line(&[pos2(x0, y), pos2(x1, y)], stroke, 6.0, 4.0));
        }
        LineStyle::Dotted => {
            painter.extend(Shape::dotted_line(
                &[pos2(x0, y), pos2(x1, y)],
                stroke.color,
                4.0,
                stroke.width * 0.6,
            ));
        }
    }
}

pub fn styled_vline(painter: &Painter, x: f32, y0: f32, y1: f32, stroke: Stroke, style: LineStyle) {
    match style {
        LineStyle::Solid => {
            painter.vline(x, y0..=y1, stroke);
        }
        LineStyle::Dashed => {
            painter.extend(Shape::dashed_line(&[pos2(x, y0), pos2(x, y1)], stroke, 6.0, 4.0));
        }
        LineStyle::Dotted => {
            painter.extend(Shape::dotted_line(
                &[pos2(x, y0), pos2(x, y1)],
                stroke.color,
                4.0,
                stroke.width * 0.6,
            ));
        }
    }
}

/// Text that reads on `bg`: white on dark colours, near-black on light.
pub fn text_on(bg: Color32) -> Color32 {
    let l = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if l > 160.0 {
        Color32::from_rgb(0x13, 0x17, 0x22)
    } else {
        Color32::WHITE
    }
}

/// A label tag on a scale: a filled rounded box with centred text.
pub fn scale_tag(painter: &Painter, rect: Rect, text: &str, bg: Color32, font: egui::FontId) {
    painter.rect_filled(rect, egui::CornerRadius::same(3), bg);
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, font, text_on(bg));
}

/// Moves labels apart so none overlap, keeping each as close to its value
/// as it can, within `top..bottom`. Each label is (top y, height, id); the
/// tops are adjusted in place.
pub fn spread(tags: &mut [(f32, f32, usize)], gap: f32, top: f32, bottom: f32) {
    tags.sort_by(|a, b| a.0.total_cmp(&b.0));
    for i in 1..tags.len() {
        let min = tags[i - 1].0 + tags[i - 1].1 + gap;
        if tags[i].0 < min {
            tags[i].0 = min;
        }
    }
    // If the bottom overflowed, push back up.
    if let Some(last) = tags.last_mut()
        && last.0 + last.1 > bottom
    {
        last.0 = bottom - last.1;
        for i in (0..tags.len() - 1).rev() {
            let max = tags[i + 1].0 - gap - tags[i].1;
            if tags[i].0 > max {
                tags[i].0 = max;
            }
        }
    }
    for t in tags.iter_mut() {
        t.0 = t.0.max(top);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_round_trip() {
        let m = YMap {
            top: 0.0,
            bottom: 100.0,
            lo: 10.0,
            hi: 20.0,
            log: false,
        };
        assert_eq!(m.y(10.0), 100.0);
        assert_eq!(m.y(20.0), 0.0);
        assert!((m.value(50.0) - 15.0).abs() < 1e-9);
        let l = YMap { log: true, ..m };
        // Screen positions are f32, so round trips are good to ~1e-5.
        assert!((l.value(l.y(13.0)) - 13.0).abs() < 1e-4);
        let x = XMap {
            right: 1000.0,
            right_index: 100.0,
            bar_w: 10.0,
        };
        assert_eq!(x.x(100.0), 1000.0);
        assert_eq!(x.x(90.0), 900.0);
        assert!((x.index(950.0) - 95.0).abs() < 1e-9);
    }

    #[test]
    fn labels_spread_apart() {
        // A tall label (with a countdown) followed by two short ones.
        let mut tags = vec![(50.0, 34.0, 0), (52.0, 18.0, 1), (51.0, 18.0, 2)];
        spread(&mut tags, 1.0, 0.0, 400.0);
        for w in tags.windows(2) {
            assert!(w[1].0 >= w[0].0 + w[0].1 + 1.0, "{tags:?}");
        }
        // Squeezed at the bottom, they stay inside.
        let mut tags = vec![(390.0, 18.0, 0), (395.0, 18.0, 1)];
        spread(&mut tags, 1.0, 0.0, 400.0);
        assert!(tags.iter().all(|t| t.0 + t.1 <= 400.0));
    }
}

//! What the chart draws for the screen-dependent built-ins (Volume Profile
//! Visible Range and Session, Visible Average Price, Seasonality) and the
//! volume-profile drawing tools. The numbers come from
//! `opentern_series::profile`.

use super::paint::{XMap, YMap, polyline, rect_mesh};
use super::style::c32;
use super::{Chart, StudyView};
use crate::format;
use crate::theme;
use egui::epaint::Mesh;
use egui::{Align2, Color32, Painter, Rect, Stroke, pos2};
use opentern_series::Rgba;
use opentern_series::builtins::charted::{VP_DOWN, VP_POC, VP_UP, VP_VA_DOWN, VP_VA_UP};
use opentern_series::builtins::colour_of;
use opentern_series::profile::{Profile, SeasonYear, volume_profile};

/// How a profile is drawn.
pub struct ProfileLook {
    pub up: Color32,
    pub down: Color32,
    pub va_up: Color32,
    pub va_down: Color32,
    pub poc: Color32,
    pub split: bool,
    pub show_poc: bool,
    pub va_lines: bool,
}

impl Default for ProfileLook {
    fn default() -> Self {
        ProfileLook {
            up: c32(VP_UP),
            down: c32(VP_DOWN),
            va_up: c32(VP_VA_UP),
            va_down: c32(VP_VA_DOWN),
            poc: c32(VP_POC),
            split: true,
            show_poc: true,
            va_lines: false,
        }
    }
}

impl ProfileLook {
    fn from_study(s: &StudyView) -> ProfileLook {
        let i = &s.inst.inputs;
        let c = |id: &str, d: Rgba| c32(Rgba(colour_of(i, id, d)));
        ProfileLook {
            up: c("up", VP_UP),
            down: c("down", VP_DOWN),
            va_up: c("va_up", VP_VA_UP),
            va_down: c("va_down", VP_VA_DOWN),
            poc: c("poc_color", VP_POC),
            split: i.text("volume") != "Total",
            show_poc: i.bool("poc"),
            va_lines: i.bool("va_lines"),
        }
    }
}

/// Paints a profile's rows from `edge` (growing right, or left when
/// `leftward`), the longest `width` px; the POC line runs `poc_span`.
#[allow(clippy::too_many_arguments)]
pub fn paint_profile(
    p: &Painter,
    prof: &Profile,
    look: &ProfileLook,
    y: &YMap,
    edge: f32,
    width: f32,
    leftward: bool,
    poc_span: (f32, f32),
) {
    let max = prof.max();
    if max.is_nan() || max <= 0.0 || width <= 0.0 {
        return;
    }
    let mut mesh = Mesh::default();
    for i in 0..prof.rows() {
        let (y0, y1) = (
            y.y(prof.lo + (i + 1) as f64 * prof.step),
            y.y(prof.lo + i as f64 * prof.step),
        );
        // A pixel between rows when they're tall enough to show it.
        let (top, bot) = if y1 - y0 > 3.0 { (y0 + 0.5, y1 - 0.5) } else { (y0, y1) };
        let in_va = i >= prof.val && i <= prof.vah;
        let (cu, cd) = if in_va {
            (look.va_up, look.va_down)
        } else {
            (look.up, look.down)
        };
        let total = prof.total(i);
        let len = (total / max) as f32 * width;
        let up_len = if look.split && total > 0.0 {
            (prof.up[i] / total) as f32 * len
        } else {
            len
        };
        let seg = |a: f32, b: f32| {
            if leftward {
                Rect::from_min_max(pos2(edge - b, top), pos2(edge - a, bot))
            } else {
                Rect::from_min_max(pos2(edge + a, top), pos2(edge + b, bot))
            }
        };
        rect_mesh(&mut mesh, seg(0.0, up_len), cu);
        if look.split && len > up_len {
            rect_mesh(&mut mesh, seg(up_len, len), cd);
        }
    }
    p.add(mesh);
    if look.show_poc {
        let yy = y.y(prof.mid(prof.poc));
        p.hline(poc_span.0..=poc_span.1, yy, Stroke::new(1.0, look.poc));
    }
    if look.va_lines {
        let s = Stroke::new(1.0, look.va_up.to_opaque());
        for yy in [
            y.y(prof.lo + (prof.vah + 1) as f64 * prof.step),
            y.y(prof.lo + prof.val as f64 * prof.step),
        ] {
            p.hline(poc_span.0..=poc_span.1, yy, s);
        }
    }
}

/// The main pane's chart-drawn built-ins.
#[allow(clippy::too_many_arguments)]
pub fn paint_overlays(p: &Painter, chart: &Chart, y: &YMap, x: &XMap, first: usize, last: usize, rect: Rect) {
    let n = chart.bars.len();
    if n == 0 || first >= n {
        return;
    }
    let last = last.min(n - 1);
    for s in chart.studies.iter().filter(|s| s.pane == 0 && s.shown(chart.tf)) {
        let ins = &s.inst.inputs;
        match s.meta().id {
            "vpvr" => {
                let Some(prof) = volume_profile(&chart.bars[first..=last], ins.len("rows"), ins.float("va") / 100.0)
                else {
                    continue;
                };
                let width = rect.width() * (ins.float("width") / 100.0) as f32;
                let left = ins.text("placement") == "Left";
                let edge = if left { rect.left() } else { rect.right() };
                paint_profile(
                    p,
                    &prof,
                    &ProfileLook::from_study(s),
                    y,
                    edge,
                    width,
                    !left,
                    (rect.left(), rect.right()),
                );
            }
            "vpsv" => {
                let look = ProfileLook::from_study(s);
                // Back to the start of the first visible session.
                let mut start = first;
                while start > 0 && !chart.sessions.get(start).copied().unwrap_or(true) {
                    start -= 1;
                }
                let mut a = start;
                while a <= last {
                    let mut b = a + 1;
                    while b < n && !chart.sessions.get(b).copied().unwrap_or(true) {
                        b += 1;
                    }
                    let Some(prof) = volume_profile(&chart.bars[a..b], ins.len("rows"), ins.float("va") / 100.0) else {
                        a = b;
                        continue;
                    };
                    let half = x.bar_w / 2.0;
                    let (x0, x1) = (x.x(a as f64) - half, x.x((b - 1) as f64) + half);
                    let width = (x1 - x0) * (ins.float("width") / 100.0) as f32;
                    paint_profile(p, &prof, &look, y, x0, width, false, (x0, x1));
                    a = b;
                }
            }
            "vap" => {
                let bars = &chart.bars[first..=last];
                let avg = bars.iter().map(|b| b.close).sum::<f64>() / bars.len() as f64;
                let col = c32(Rgba(colour_of(ins, "color", Rgba::hex(0xFF9800))));
                let yy = y.y(avg);
                p.hline(
                    rect.x_range(),
                    yy,
                    Stroke::new(ins.int("width").clamp(1, 4) as f32, col),
                );
                p.text(
                    pos2(rect.right() - 6.0, yy - 3.0),
                    Align2::RIGHT_BOTTOM,
                    format!("VAP {}", format::price(avg, chart.precision())),
                    theme::medium(11.5),
                    col,
                );
            }
            _ => {}
        }
    }
}

/// Seasonality's colours, the newest year first (TradingView-like).
const SEASON_COLORS: [u32; 10] = [
    0x2962FF, 0xFF9800, 0x089981, 0xF23645, 0x9C27B0, 0x00BCD4, 0x795548, 0xE91E63, 0x607D8B, 0x4CAF50,
];

/// The years and their values, kept until the bars change.
#[derive(Default)]
pub struct SeasonCache {
    key: Option<(u64, usize, usize)>,
    pub years: Vec<SeasonYear>,
}

impl SeasonCache {
    pub fn get(&mut self, chart: &Chart, years: usize) -> &[SeasonYear] {
        let key = (chart.data_version, chart.bars.len(), years);
        if self.key != Some(key) {
            self.key = Some(key);
            self.years = opentern_series::profile::seasonality(&chart.bars, &chart.sym_tz, years);
        }
        &self.years
    }
}

/// Seasonality in its pane: a line per year across the pane (January at
/// the left, December at the right), with the months marked.
pub fn paint_seasonality(p: &Painter, years: &[SeasonYear], y: &YMap, rect: Rect, text: Color32) {
    let plot = rect.shrink2(egui::vec2(8.0, 0.0));
    let n = years.len();
    for (k, yr) in years.iter().enumerate() {
        let newest = k + 1 == n;
        let col = c32(Rgba::hex(SEASON_COLORS[(n - 1 - k) % SEASON_COLORS.len()]));
        let pts: Vec<_> = yr
            .points
            .iter()
            .map(|(f, v)| pos2(plot.left() + *f as f32 * plot.width(), y.y(*v)))
            .collect();
        if let Some(end) = pts.last().copied() {
            let inside = end.x + 36.0 < rect.right();
            p.text(
                end + egui::vec2(if inside { 4.0 } else { -4.0 }, if inside { 0.0 } else { -9.0 }),
                if inside {
                    Align2::LEFT_CENTER
                } else {
                    Align2::RIGHT_CENTER
                },
                yr.year.to_string(),
                theme::medium(11.0),
                col,
            );
        }
        polyline(p, pts, Stroke::new(if newest { 2.0 } else { 1.25 }, col));
    }
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    for (m, name) in MONTHS.iter().enumerate() {
        let xx = plot.left() + plot.width() * (m as f32 + 0.5) / 12.0;
        p.text(
            pos2(xx, rect.bottom() - 4.0),
            Align2::CENTER_BOTTOM,
            *name,
            theme::regular(10.5),
            text,
        );
    }
}

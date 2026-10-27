//! Drawings: the model, geometry, painting and hit-testing of TradingView's
//! drawing tools. Points are anchored by time and price, so a trend line
//! drawn on 1h stays put on 4h or 1D.

use super::paint::{XMap, YMap, styled_hline};
use super::style::c32;
use crate::format;
use crate::icons::ph;
use crate::theme;
use egui::epaint::{PathShape, PathStroke};
use egui::{Align2, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};
use opentern_core::Bar;
use opentern_series::Rgba;
use opentern_series::study::LineStyle;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tool {
    Cross,
    Eraser,
    /// Drag a range to zoom into it.
    Zoom,
    /// A quick measure (also Shift+drag): change, bars, time and volume
    /// between two points. Not saved; the next click clears it.
    Measure,
    TrendLine,
    Ray,
    ExtendedLine,
    Arrow,
    HorizontalLine,
    HorizontalRay,
    VerticalLine,
    CrossLine,
    ParallelChannel,
    FibRetracement,
    FibExtension,
    Pitchfork,
    Path,
    Triangle,
    LongPosition,
    ShortPosition,
    PriceRange,
    DateRange,
    DatePriceRange,
    Rectangle,
    Ellipse,
    Brush,
    Text,
    ArrowUp,
    ArrowDown,
    AnchoredVwap,
    FixedRangeVolumeProfile,
    AnchoredVolumeProfile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Group {
    Cursors,
    Lines,
    Fib,
    Patterns,
    Forecast,
    Shapes,
    Text,
    Icons,
}

impl Group {
    pub const ALL: [Group; 8] = [
        Group::Cursors,
        Group::Lines,
        Group::Fib,
        Group::Patterns,
        Group::Forecast,
        Group::Shapes,
        Group::Text,
        Group::Icons,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Group::Cursors => "Cursors",
            Group::Lines => "Trend line tools",
            Group::Fib => "Gann and Fibonacci tools",
            Group::Patterns => "Patterns",
            Group::Forecast => "Forecasting and measurement tools",
            Group::Shapes => "Geometric shapes",
            Group::Text => "Annotation tools",
            Group::Icons => "Icons",
        }
    }

    pub fn tools(self) -> &'static [Tool] {
        use Tool::*;
        match self {
            Group::Cursors => &[Cross, Eraser],
            Group::Lines => &[
                TrendLine,
                Ray,
                ExtendedLine,
                Arrow,
                HorizontalLine,
                HorizontalRay,
                VerticalLine,
                CrossLine,
                ParallelChannel,
            ],
            Group::Fib => &[FibRetracement, FibExtension, Pitchfork],
            Group::Patterns => &[Path, Triangle],
            Group::Forecast => &[
                LongPosition,
                ShortPosition,
                AnchoredVwap,
                FixedRangeVolumeProfile,
                AnchoredVolumeProfile,
                PriceRange,
                DateRange,
                DatePriceRange,
            ],
            Group::Shapes => &[Rectangle, Ellipse, Brush],
            Group::Text => &[Text],
            Group::Icons => &[ArrowUp, ArrowDown],
        }
    }
}

impl Tool {
    pub fn group(self) -> Group {
        Group::ALL
            .into_iter()
            .find(|g| g.tools().contains(&self))
            .unwrap_or(Group::Cursors)
    }

    /// Every tool, in the toolbar's order (the groups, then the bottom ones).
    pub const ALL: [Tool; 32] = {
        use Tool::*;
        [
            Cross,
            Eraser,
            TrendLine,
            Ray,
            ExtendedLine,
            Arrow,
            HorizontalLine,
            HorizontalRay,
            VerticalLine,
            CrossLine,
            ParallelChannel,
            FibRetracement,
            FibExtension,
            Pitchfork,
            Path,
            Triangle,
            LongPosition,
            ShortPosition,
            AnchoredVwap,
            FixedRangeVolumeProfile,
            AnchoredVolumeProfile,
            PriceRange,
            DateRange,
            DatePriceRange,
            Rectangle,
            Ellipse,
            Brush,
            Text,
            ArrowUp,
            ArrowDown,
            Measure,
            Zoom,
        ]
    };

    /// The tool's shortcut: TradingView's default where it has one (Alt T,
    /// H, V, C, F and Alt Shift R), else Alt Shift and a letter, clear of
    /// TradingView's other Alt keys (G, I, L, P, R, S, W, Z on their own).
    pub fn shortcut(self) -> &'static str {
        use Tool::*;
        match self {
            Cross => "Esc",
            TrendLine => "Alt T",
            HorizontalLine => "Alt H",
            VerticalLine => "Alt V",
            CrossLine => "Alt C",
            FibRetracement => "Alt F",
            Rectangle => "Alt Shift R",
            Eraser => "Alt Shift E",
            Zoom => "Alt Shift Z",
            Measure => "Alt Shift M",
            Ray => "Alt Shift Y",
            ExtendedLine => "Alt Shift X",
            Arrow => "Alt Shift W",
            HorizontalRay => "Alt Shift H",
            ParallelChannel => "Alt Shift C",
            FibExtension => "Alt Shift F",
            Pitchfork => "Alt Shift K",
            Path => "Alt Shift P",
            Triangle => "Alt Shift I",
            LongPosition => "Alt Shift L",
            ShortPosition => "Alt Shift S",
            PriceRange => "Alt Shift Q",
            DateRange => "Alt Shift D",
            DatePriceRange => "Alt Shift J",
            Ellipse => "Alt Shift O",
            Brush => "Alt Shift B",
            Text => "Alt Shift T",
            ArrowUp => "Alt Shift U",
            ArrowDown => "Alt Shift N",
            AnchoredVwap => "Alt Shift V",
            FixedRangeVolumeProfile => "Alt Shift G",
            AnchoredVolumeProfile => "Alt Shift A",
        }
    }

    /// The tool a key press picks, if any (Alt or Alt Shift plus a letter).
    pub fn for_key(key: egui::Key, shift: bool) -> Option<Tool> {
        let want = format!("Alt {}{}", if shift { "Shift " } else { "" }, key.name());
        Tool::ALL.into_iter().find(|t| t.shortcut() == want)
    }

    pub fn name(self) -> &'static str {
        use Tool::*;
        match self {
            Cross => "Cross",
            Eraser => "Eraser",
            Zoom => "Zoom in",
            TrendLine => "Trend line",
            Ray => "Ray",
            ExtendedLine => "Extended line",
            Arrow => "Arrow",
            HorizontalLine => "Horizontal line",
            HorizontalRay => "Horizontal ray",
            VerticalLine => "Vertical line",
            CrossLine => "Cross line",
            ParallelChannel => "Parallel channel",
            FibRetracement => "Fib retracement",
            FibExtension => "Trend-based fib extension",
            Pitchfork => "Pitchfork",
            Path => "Path",
            Triangle => "Triangle",
            LongPosition => "Long position",
            ShortPosition => "Short position",
            Measure => "Measure",
            PriceRange => "Price range",
            DateRange => "Date range",
            DatePriceRange => "Date and price range",
            Rectangle => "Rectangle",
            Ellipse => "Ellipse",
            Brush => "Brush",
            Text => "Text",
            ArrowUp => "Arrow mark up",
            ArrowDown => "Arrow mark down",
            AnchoredVwap => "Anchored VWAP",
            FixedRangeVolumeProfile => "Fixed range volume profile",
            AnchoredVolumeProfile => "Anchored volume profile",
        }
    }

    pub fn icon(self) -> &'static str {
        use Tool::*;
        match self {
            Cross => ph::CROSSHAIR_SIMPLE,
            Eraser => ph::ERASER,
            Zoom => ph::MAGNIFYING_GLASS_PLUS,
            Measure => ph::RULER,
            TrendLine => ph::LINE_SEGMENT,
            Ray => ph::ARROW_UP_RIGHT,
            ExtendedLine => ph::ARROWS_HORIZONTAL,
            Arrow => ph::ARROW_UP_RIGHT,
            HorizontalLine => ph::MINUS,
            HorizontalRay => ph::ARROW_RIGHT,
            VerticalLine => ph::LINE_VERTICAL,
            CrossLine => ph::PLUS,
            ParallelChannel => ph::PARALLELOGRAM,
            FibRetracement => ph::ROWS,
            FibExtension => ph::LIST_DASHES,
            Pitchfork => ph::GIT_FORK,
            Path => ph::LINE_SEGMENTS,
            Triangle => ph::TRIANGLE,
            LongPosition => ph::TREND_UP,
            ShortPosition => ph::TREND_DOWN,
            PriceRange => ph::ARROWS_VERTICAL,
            DateRange => ph::ARROWS_HORIZONTAL,
            DatePriceRange => ph::RULER,
            Rectangle => ph::RECTANGLE,
            Ellipse => ph::CIRCLE,
            Brush => ph::PAINT_BRUSH,
            Text => ph::TEXT_T,
            ArrowUp => ph::ARROW_FAT_UP,
            ArrowDown => ph::ARROW_FAT_DOWN,
            AnchoredVwap => ph::ANCHOR_SIMPLE,
            FixedRangeVolumeProfile => ph::CHART_BAR_HORIZONTAL,
            AnchoredVolumeProfile => ph::ALIGN_LEFT,
        }
    }

    /// Clicks needed to place it (0 = drawn by dragging, `usize::MAX` = until
    /// double-click).
    pub fn points(self) -> usize {
        use Tool::*;
        match self {
            Cross | Eraser | Zoom | Measure => 0,
            HorizontalLine
            | HorizontalRay
            | VerticalLine
            | CrossLine
            | Text
            | ArrowUp
            | ArrowDown
            | LongPosition
            | ShortPosition
            | AnchoredVwap
            | AnchoredVolumeProfile => 1,
            ParallelChannel | FibExtension | Pitchfork | Triangle => 3,
            Path => usize::MAX,
            Brush => 0,
            _ => 2,
        }
    }

    pub fn is_drawing(self) -> bool {
        !matches!(self, Tool::Cross | Tool::Eraser | Tool::Zoom | Tool::Measure)
    }

    pub fn key(self) -> String {
        format!("{self:?}")
    }
}

/// A point on the chart.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    /// Bar open time (ms), possibly past the last bar.
    pub t: i64,
    pub p: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrawStyle {
    pub color: Rgba,
    pub width: u8,
    pub line: LineStyle,
    pub fill: Rgba,
    pub extend_left: bool,
    pub extend_right: bool,
    pub font: u8,
    pub labels: bool,
}

impl Default for DrawStyle {
    fn default() -> Self {
        DrawStyle {
            color: Rgba::hex(0x2962FF),
            width: 2,
            line: LineStyle::Solid,
            fill: Rgba::hex(0x2962FF).transp(80),
            extend_left: false,
            extend_right: false,
            font: 14,
            labels: true,
        }
    }
}

impl DrawStyle {
    /// TradingView's defaults per tool.
    pub fn for_tool(tool: Tool) -> DrawStyle {
        use Tool::*;
        let d = DrawStyle::default();
        match tool {
            HorizontalLine | HorizontalRay | VerticalLine | CrossLine => DrawStyle { width: 1, ..d },
            Rectangle | Ellipse | Triangle => DrawStyle {
                color: Rgba::hex(0x9C27B0),
                fill: Rgba::hex(0x9C27B0).transp(80),
                ..d
            },
            FibRetracement | FibExtension => DrawStyle {
                color: Rgba::hex(0x787B86),
                width: 1,
                ..d
            },
            Pitchfork => DrawStyle {
                color: Rgba::hex(0xF23645),
                ..d
            },
            Brush => DrawStyle {
                color: Rgba::hex(0x00BCD4),
                ..d
            },
            Text => DrawStyle {
                color: Rgba::hex(0x2962FF),
                ..d
            },
            ArrowUp => DrawStyle {
                color: Rgba::hex(0x089981),
                ..d
            },
            ArrowDown => DrawStyle {
                color: Rgba::hex(0xF23645),
                ..d
            },
            AnchoredVwap => DrawStyle {
                color: Rgba::hex(0x1E88E5),
                width: 2,
                ..d
            },
            FixedRangeVolumeProfile | AnchoredVolumeProfile => DrawStyle {
                color: Rgba::hex(0x2962FF),
                width: 1,
                ..d
            },
            PriceRange | DateRange | DatePriceRange => DrawStyle {
                color: Rgba::hex(0x2962FF),
                fill: Rgba::hex(0x2962FF).transp(80),
                width: 1,
                ..d
            },
            _ => d,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Drawing {
    pub id: u64,
    pub tool: Tool,
    pub pts: Vec<Anchor>,
    pub style: DrawStyle,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub hidden: bool,
}

/// What part of a drawing the pointer is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part {
    Point(usize),
    Body,
}

/// TradingView's Fibonacci levels and colours.
pub const FIB_LEVELS: [(f64, u32); 11] = [
    (0.0, 0x787B86),
    (0.236, 0xF23645),
    (0.382, 0xFF9800),
    (0.5, 0x4CAF50),
    (0.618, 0x089981),
    (0.786, 0x00BCD4),
    (1.0, 0x787B86),
    (1.618, 0x2962FF),
    (2.618, 0xF23645),
    (3.618, 0x9C27B0),
    (4.236, 0xE91E63),
];

/// Maps times to x and back, using the bars (fractional between bars,
/// extrapolated past either end with the interval's length).
pub struct TimeMap<'a> {
    pub bars: &'a [Bar],
    pub step: i64,
    pub x: XMap,
}

impl TimeMap<'_> {
    pub fn index(&self, t: i64) -> f64 {
        let b = self.bars;
        if b.is_empty() {
            return 0.0;
        }
        let step = self.step.max(1) as f64;
        if t <= b[0].time {
            return (t - b[0].time) as f64 / step;
        }
        let last = b.len() - 1;
        if t >= b[last].time {
            return last as f64 + (t - b[last].time) as f64 / step;
        }
        match b.binary_search_by_key(&t, |x| x.time) {
            Ok(i) => i as f64,
            Err(i) => {
                let (a, c) = (b[i - 1].time, b[i].time);
                (i - 1) as f64 + (t - a) as f64 / (c - a).max(1) as f64
            }
        }
    }

    pub fn x(&self, t: i64) -> f32 {
        self.x.x(self.index(t))
    }

    /// The time of the bar nearest to `x` (extrapolated past the ends).
    pub fn time(&self, x: f32) -> i64 {
        let i = self.x.index(x).round();
        let b = self.bars;
        if b.is_empty() {
            return 0;
        }
        let last = b.len() as f64 - 1.0;
        if i < 0.0 {
            b[0].time + (i * self.step as f64) as i64
        } else if i > last {
            b[b.len() - 1].time + ((i - last) * self.step as f64) as i64
        } else {
            b[i as usize].time
        }
    }

    /// The bar index nearest to `x`, if it's a real bar.
    pub fn bar(&self, x: f32) -> Option<usize> {
        let i = self.x.index(x).round();
        (i >= 0.0 && (i as usize) < self.bars.len()).then_some(i as usize)
    }
}

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 < 1e-6 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// The visible segment of a line through a→b, extended to the rect edges
/// as asked.
fn extend(a: Pos2, b: Pos2, left: bool, right: bool, r: Rect) -> (Pos2, Pos2) {
    let d = b - a;
    if d.x.abs() < 1e-3 {
        // Vertical: extend to top/bottom.
        let (lo, hi) = if a.y < b.y { (a, b) } else { (b, a) };
        let lo = if left || right { pos2(lo.x, r.top()) } else { lo };
        let hi = if left || right { pos2(hi.x, r.bottom()) } else { hi };
        return (lo, hi);
    }
    let slope = d.y / d.x;
    let (l, rr) = if a.x <= b.x { (a, b) } else { (b, a) };
    let at = |x: f32| pos2(x, l.y + (x - l.x) * slope);
    let p0 = if left { at(r.left() - 10.0) } else { l };
    let p1 = if right { at(r.right() + 10.0) } else { rr };
    (p0, p1)
}

/// Screen geometry helpers for one frame.
pub struct Geo<'a> {
    pub tm: TimeMap<'a>,
    pub y: YMap,
    pub rect: Rect,
    pub decimals: usize,
}

impl Geo<'_> {
    pub fn pos(&self, a: Anchor) -> Pos2 {
        pos2(self.tm.x(a.t), self.y.y(a.p))
    }

    pub fn anchor(&self, p: Pos2) -> Anchor {
        Anchor {
            t: self.tm.time(p.x),
            p: self.y.value(p.y),
        }
    }
}

fn stroke(d: &Drawing, sel: bool) -> Stroke {
    Stroke::new(d.style.width as f32 + if sel { 0.5 } else { 0.0 }, c32(d.style.color))
}

fn line(p: &Painter, a: Pos2, b: Pos2, s: Stroke, style: LineStyle) {
    match style {
        LineStyle::Solid => {
            p.line_segment([a, b], s);
        }
        LineStyle::Dashed => {
            p.extend(Shape::dashed_line(&[a, b], s, 7.0, 5.0));
        }
        LineStyle::Dotted => {
            p.extend(Shape::dotted_line(&[a, b], s.color, 4.0, s.width * 0.6));
        }
    }
}

fn label(p: &Painter, at: Pos2, align: Align2, text: &str, fg: Color32, bg: Color32) {
    let g = p.layout_no_wrap(text.to_string(), theme::medium(12.0), fg);
    let r = align.anchor_size(at, g.size() + vec2(12.0, 6.0));
    p.rect_filled(r, CornerRadius::same(4), bg);
    p.galley(r.min + vec2(6.0, 3.0), g, fg);
}

/// Long/short position geometry: entry, target and stop prices and the
/// x span.
fn position_parts(d: &Drawing, g: &Geo) -> Option<(f32, f32, f64, f64, f64)> {
    let (e, t, s) = (d.pts.first()?, d.pts.get(1)?, d.pts.get(2)?);
    let x0 = g.tm.x(e.t);
    let x1 = g.tm.x(t.t).max(x0 + 4.0);
    Some((x0, x1, e.p, t.p, s.p))
}

/// Paints a drawing. `sel` draws its handles.
pub fn paint(p: &Painter, d: &Drawing, g: &Geo, sel: bool, bg: Color32) {
    use Tool::*;
    if d.hidden {
        return;
    }
    let s = stroke(d, sel);
    let pts: Vec<Pos2> = d.pts.iter().map(|a| g.pos(*a)).collect();
    let r = g.rect;
    let fill = c32(d.style.fill);
    match d.tool {
        TrendLine | Ray | ExtendedLine | Arrow if pts.len() >= 2 => {
            let (l, rt) = match d.tool {
                Ray => (false, true),
                ExtendedLine => (true, true),
                _ => (d.style.extend_left, d.style.extend_right),
            };
            let (a, b) = if d.tool == Ray {
                // A ray goes from the first point through the second.
                let (_, far) = extend(pts[0], pts[1], false, true, r);
                let far = if pts[1].x >= pts[0].x {
                    far
                } else {
                    extend(pts[0], pts[1], true, false, r).0
                };
                (pts[0], far)
            } else {
                extend(pts[0], pts[1], l, rt, r)
            };
            line(p, a, b, s, d.style.line);
            if d.tool == Arrow {
                let dir = (pts[1] - pts[0]).normalized();
                let n = vec2(-dir.y, dir.x);
                let size = 8.0 + d.style.width as f32 * 2.0;
                let tip = pts[1];
                p.add(Shape::convex_polygon(
                    vec![
                        tip,
                        tip - dir * size + n * size * 0.5,
                        tip - dir * size - n * size * 0.5,
                    ],
                    s.color,
                    Stroke::NONE,
                ));
            }
        }
        HorizontalLine | HorizontalRay if !pts.is_empty() => {
            let x0 = if d.tool == HorizontalRay { pts[0].x } else { r.left() };
            styled_hline(p, x0, r.right(), pts[0].y, s, d.style.line);
            if d.style.labels {
                let text = format::price(d.pts[0].p, g.decimals);
                label(
                    p,
                    pos2(r.right() - 4.0, pts[0].y - 2.0),
                    Align2::RIGHT_BOTTOM,
                    &text,
                    Color32::WHITE,
                    s.color,
                );
            }
        }
        VerticalLine if !pts.is_empty() => {
            line(p, pos2(pts[0].x, r.top()), pos2(pts[0].x, r.bottom()), s, d.style.line);
        }
        CrossLine if !pts.is_empty() => {
            line(p, pos2(pts[0].x, r.top()), pos2(pts[0].x, r.bottom()), s, d.style.line);
            line(p, pos2(r.left(), pts[0].y), pos2(r.right(), pts[0].y), s, d.style.line);
        }
        ParallelChannel if pts.len() >= 2 => {
            let off = if pts.len() >= 3 { pts[2].y - pts[0].y } else { 0.0 };
            let (a, b) = (pts[0], pts[1]);
            let (c, e) = (a + vec2(0.0, off), b + vec2(0.0, off));
            p.add(Shape::convex_polygon(vec![a, b, e, c], fill, Stroke::NONE));
            line(p, a, b, s, d.style.line);
            line(p, c, e, s, d.style.line);
            let mid = Stroke::new(1.0, s.color.gamma_multiply(0.6));
            p.extend(Shape::dashed_line(
                &[a + vec2(0.0, off / 2.0), b + vec2(0.0, off / 2.0)],
                mid,
                5.0,
                4.0,
            ));
        }
        FibRetracement | FibExtension if pts.len() >= 2 => {
            // Retracement: 1 at the start, 0 at the end. Extension: levels
            // from the third point by the first leg's height.
            let (x0, x1) = (pts[0].x.min(pts[pts.len() - 1].x), pts[0].x.max(pts[pts.len() - 1].x));
            let (base, span) = if d.tool == FibExtension && d.pts.len() >= 3 {
                (d.pts[2].p, d.pts[1].p - d.pts[0].p)
            } else {
                (d.pts[1].p, d.pts[0].p - d.pts[1].p)
            };
            let x1 = if d.style.extend_right { r.right() } else { x1 };
            if d.tool == FibExtension && pts.len() >= 2 {
                let tl = Stroke::new(1.0, s.color.gamma_multiply(0.7));
                p.extend(Shape::dashed_line(&pts, tl, 5.0, 4.0));
            }
            let mut prev: Option<(f32, u32)> = None;
            for (lvl, col) in FIB_LEVELS {
                if d.tool == FibRetracement && lvl > 1.0 {
                    continue;
                }
                let price = base + span * lvl;
                let y = g.y.y(price);
                let c = c32(Rgba::hex(col));
                if let Some((py, pc)) = prev {
                    let band = Rect::from_x_y_ranges(x0..=x1, py.min(y)..=py.max(y));
                    p.rect_filled(band, CornerRadius::ZERO, c32(Rgba::hex(pc).transp(88)));
                }
                p.hline(x0..=x1, y, Stroke::new(1.0, c));
                if d.style.labels {
                    p.text(
                        pos2(x0 - 4.0, y),
                        Align2::RIGHT_CENTER,
                        format!("{lvl} ({})", format::price(price, g.decimals)),
                        theme::regular(11.0),
                        c,
                    );
                }
                prev = Some((y, col));
            }
            if d.tool == FibRetracement {
                p.extend(Shape::dashed_line(
                    &[pts[0], pts[1]],
                    Stroke::new(1.0, s.color.gamma_multiply(0.7)),
                    5.0,
                    4.0,
                ));
            }
        }
        Pitchfork if pts.len() >= 2 => {
            let (a, b) = (pts[0], pts[1]);
            let c = pts.get(2).copied().unwrap_or(b);
            let mid = pos2((b.x + c.x) / 2.0, (b.y + c.y) / 2.0);
            let dir = mid - a;
            let far = |q: Pos2| {
                if dir.x.abs() < 1e-3 {
                    q
                } else {
                    let k = (r.right() + 10.0 - q.x) / dir.x;
                    q + dir * k.max(0.0)
                }
            };
            line(p, a, far(mid), s, d.style.line);
            line(p, b, far(b), s, d.style.line);
            line(p, c, far(c), s, d.style.line);
            line(p, b, c, Stroke::new(1.0, s.color.gamma_multiply(0.7)), LineStyle::Solid);
        }
        Path | Brush if pts.len() >= 2 => {
            p.add(Shape::Path(PathShape::line(
                pts.clone(),
                PathStroke::new(s.width, s.color),
            )));
        }
        Triangle if pts.len() >= 2 => {
            let poly = pts.clone();
            if poly.len() >= 3 {
                p.add(Shape::convex_polygon(poly.clone(), fill, Stroke::NONE));
            }
            p.add(Shape::Path(PathShape::closed_line(
                poly,
                PathStroke::new(s.width, s.color),
            )));
        }
        Rectangle | PriceRange | DateRange | DatePriceRange if pts.len() >= 2 => {
            let rect = Rect::from_two_pos(pts[0], pts[1]);
            p.rect_filled(rect, CornerRadius::ZERO, fill);
            if d.tool == Rectangle {
                p.rect_stroke(rect, CornerRadius::ZERO, s, egui::StrokeKind::Middle);
                // A label above the box (the agent marks patterns this way).
                if !d.text.is_empty() {
                    p.text(
                        rect.left_top() + vec2(0.0, -4.0),
                        Align2::LEFT_BOTTOM,
                        &d.text,
                        theme::medium(d.style.font as f32 * 0.9),
                        s.color,
                    );
                }
            } else {
                measure(p, d, g, pts[0], pts[1], s);
            }
        }
        Ellipse if pts.len() >= 2 => {
            let rect = Rect::from_two_pos(pts[0], pts[1]);
            let c = rect.center();
            let (rx, ry) = (rect.width() / 2.0, rect.height() / 2.0);
            let ring: Vec<Pos2> = (0..64)
                .map(|i| {
                    let a = i as f32 / 64.0 * std::f32::consts::TAU;
                    c + vec2(a.cos() * rx, a.sin() * ry)
                })
                .collect();
            p.add(Shape::convex_polygon(ring.clone(), fill, Stroke::NONE));
            p.add(Shape::Path(PathShape::closed_line(
                ring,
                PathStroke::new(s.width, s.color),
            )));
        }
        LongPosition | ShortPosition => {
            if let Some((x0, x1, e, t, st)) = position_parts(d, g) {
                let (ye, yt, ys) = (g.y.y(e), g.y.y(t), g.y.y(st));
                let green = Color32::from_rgba_unmultiplied(8, 153, 129, 60);
                let red = Color32::from_rgba_unmultiplied(242, 54, 69, 60);
                p.rect_filled(
                    Rect::from_x_y_ranges(x0..=x1, ye.min(yt)..=ye.max(yt)),
                    CornerRadius::ZERO,
                    green,
                );
                p.rect_filled(
                    Rect::from_x_y_ranges(x0..=x1, ye.min(ys)..=ye.max(ys)),
                    CornerRadius::ZERO,
                    red,
                );
                p.hline(x0..=x1, ye, Stroke::new(1.0, Color32::from_gray(140)));
                let pct = |a: f64| (a / e - 1.0) * 100.0;
                let rr = if (e - st).abs() > 0.0 {
                    ((t - e) / (e - st)).abs()
                } else {
                    0.0
                };
                let tc = Color32::from_rgb(8, 153, 129);
                let sc = Color32::from_rgb(242, 54, 69);
                let cx = (x0 + x1) / 2.0;
                let (up_t, up_s) = (yt < ye, ys < ye);
                let place = |up: bool| if up { Align2::CENTER_BOTTOM } else { Align2::CENTER_TOP };
                label(
                    p,
                    pos2(cx, yt + if up_t { -4.0 } else { 4.0 }),
                    place(up_t),
                    &format!(
                        "Target: {} ({})",
                        format::price(t, g.decimals),
                        format::signed_pct(pct(t))
                    ),
                    Color32::WHITE,
                    tc,
                );
                label(
                    p,
                    pos2(cx, ys + if up_s { -4.0 } else { 4.0 }),
                    place(up_s),
                    &format!(
                        "Stop: {} ({})",
                        format::price(st, g.decimals),
                        format::signed_pct(pct(st))
                    ),
                    Color32::WHITE,
                    sc,
                );
                label(
                    p,
                    pos2(x1 + 6.0, ye),
                    Align2::LEFT_CENTER,
                    &format!("Risk/reward ratio: {rr:.2}"),
                    Color32::WHITE,
                    Color32::from_gray(90),
                );
            }
        }
        AnchoredVwap if !d.pts.is_empty() => {
            let line = vwap_points(d, g);
            if line.len() >= 2 {
                p.add(Shape::Path(PathShape::line(
                    line.clone(),
                    PathStroke::new(s.width, s.color),
                )));
            }
            if let (true, Some(end)) = (d.style.labels, line.last()) {
                label(
                    p,
                    *end + vec2(6.0, 0.0),
                    Align2::LEFT_CENTER,
                    "VWAP",
                    Color32::WHITE,
                    s.color,
                );
            }
            p.circle_filled(line.first().copied().unwrap_or(pts[0]), 3.0, s.color);
        }
        FixedRangeVolumeProfile | AnchoredVolumeProfile if !d.pts.is_empty() => {
            if let Some((prof, x0, x1)) = range_profile(d, g) {
                let look = super::profile::ProfileLook::default();
                let width = ((x1 - x0) * 0.3).max(40.0);
                super::profile::paint_profile(p, &prof, &look, &g.y, x0, width, false, (x0, x1));
                let (top, bot) = (g.y.y(prof.lo + prof.step * prof.rows() as f64), g.y.y(prof.lo));
                let frame = Rect::from_x_y_ranges(x0..=x1, top..=bot);
                p.rect_stroke(
                    frame,
                    CornerRadius::ZERO,
                    Stroke::new(1.0, s.color.gamma_multiply(0.5)),
                    egui::StrokeKind::Middle,
                );
            }
        }
        Text if !pts.is_empty() => {
            let text = if d.text.is_empty() { "Text" } else { d.text.as_str() };
            p.text(
                pts[0],
                Align2::LEFT_BOTTOM,
                text,
                theme::medium(d.style.font as f32),
                s.color,
            );
        }
        ArrowUp | ArrowDown if !pts.is_empty() => {
            let up = d.tool == ArrowUp;
            let tip = pts[0];
            let sz = 10.0;
            let dir = if up { -1.0 } else { 1.0 };
            let base = tip + vec2(0.0, -dir * sz);
            p.add(Shape::convex_polygon(
                vec![
                    tip,
                    base + vec2(sz * 0.7, 0.0),
                    base + vec2(sz * 0.25, 0.0),
                    base + vec2(sz * 0.25, -dir * sz),
                    base + vec2(-sz * 0.25, -dir * sz),
                    base + vec2(-sz * 0.25, 0.0),
                    base + vec2(-sz * 0.7, 0.0),
                ]
                .into_iter()
                .collect::<Vec<_>>(),
                s.color,
                Stroke::NONE,
            ));
            if !d.text.is_empty() {
                p.text(
                    tip + vec2(0.0, dir * -26.0),
                    Align2::CENTER_CENTER,
                    &d.text,
                    theme::medium(12.0),
                    s.color,
                );
            }
        }
        _ => {
            for w in pts.windows(2) {
                line(p, w[0], w[1], s, d.style.line);
            }
        }
    }
    // Other tools' labels: above a horizontal line at the right, else
    // above the first point.
    if !d.text.is_empty()
        && !matches!(d.tool, Text | ArrowUp | ArrowDown | Rectangle)
        && let Some(&first) = pts.first()
    {
        let (at, align) = match d.tool {
            HorizontalLine | HorizontalRay => (pos2(r.right() - 6.0, first.y - 3.0), Align2::RIGHT_BOTTOM),
            _ => (first + vec2(0.0, -4.0), Align2::LEFT_BOTTOM),
        };
        p.text(at, align, &d.text, theme::medium(d.style.font as f32 * 0.9), s.color);
    }
    if sel {
        for (i, q) in handle_points(d, g).into_iter().enumerate() {
            let _ = i;
            p.circle_filled(q, 5.0, bg);
            p.circle_stroke(q, 5.0, Stroke::new(1.5, c32(d.style.color)));
        }
    }
}

/// The bars a volume-profile tool covers: between its two points, or
/// from its point to the last bar.
fn tool_range(d: &Drawing, g: &Geo) -> Option<(usize, usize)> {
    let n = g.tm.bars.len();
    if n == 0 {
        return None;
    }
    let idx = |t: i64| g.tm.index(t).round().clamp(0.0, (n - 1) as f64) as usize;
    let a = idx(d.pts.first()?.t);
    let b = match (d.tool, d.pts.get(1)) {
        (Tool::FixedRangeVolumeProfile, Some(q)) => idx(q.t),
        _ => n - 1,
    };
    Some((a.min(b), a.max(b)))
}

fn range_profile(d: &Drawing, g: &Geo) -> Option<(opentern_series::profile::Profile, f32, f32)> {
    let (a, b) = tool_range(d, g)?;
    let prof = opentern_series::profile::volume_profile(&g.tm.bars[a..=b], 24, 0.7)?;
    let half = g.tm.x.bar_w / 2.0;
    Some((prof, g.tm.x.x(a as f64) - half, g.tm.x.x(b as f64) + half))
}

/// The anchored VWAP's line on screen.
fn vwap_points(d: &Drawing, g: &Geo) -> Vec<Pos2> {
    let Some((a, _)) = tool_range(d, g) else {
        return Vec::new();
    };
    opentern_series::profile::anchored_vwap(g.tm.bars, a)
        .into_iter()
        .enumerate()
        .map(|(k, v)| pos2(g.tm.x.x((a + k) as f64), g.y.y(v)))
        .collect()
}

/// The measuring tools' labels: price change, bars, time.
fn measure(p: &Painter, d: &Drawing, g: &Geo, a: Pos2, b: Pos2, s: Stroke) {
    let (pa, pb) = (d.pts[0].p, d.pts[1].p);
    let dp = pb - pa;
    let pct = if pa != 0.0 { dp / pa * 100.0 } else { 0.0 };
    let bars = (g.tm.index(d.pts[1].t) - g.tm.index(d.pts[0].t)).round() as i64;
    let span = (d.pts[1].t - d.pts[0].t).abs();
    let days = span / 86_400_000;
    let hours = span / 3_600_000 % 24;
    let mins = span / 60_000 % 60;
    let time = if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    };
    let text = match d.tool {
        Tool::PriceRange => format!("{} ({})", format::price(dp, g.decimals), format::signed_pct(pct)),
        Tool::DateRange => format!("{bars} bars, {time}"),
        _ => format!(
            "{} ({})\n{bars} bars, {time}",
            format::price(dp, g.decimals),
            format::signed_pct(pct)
        ),
    };
    let col = if dp >= 0.0 {
        Color32::from_rgb(41, 98, 255)
    } else {
        Color32::from_rgb(242, 54, 69)
    };
    let r = Rect::from_two_pos(a, b);
    p.arrow(pos2(r.center().x, a.y), vec2(0.0, b.y - a.y), Stroke::new(1.0, s.color));
    p.arrow(pos2(a.x, r.center().y), vec2(b.x - a.x, 0.0), Stroke::new(1.0, s.color));
    let at = pos2(r.center().x, r.bottom() + 6.0);
    let g2 = p.layout(text, theme::medium(12.0), Color32::WHITE, 300.0);
    let lr = Align2::CENTER_TOP.anchor_size(at, g2.size() + vec2(14.0, 8.0));
    p.rect_filled(lr, CornerRadius::same(5), col);
    p.galley(lr.min + vec2(7.0, 4.0), g2, Color32::WHITE);
}

/// The quick measure, as on TradingView: a blue (up) or red (down) box
/// with arrows, and a label with the change, its percent and ticks, the
/// bars and time between, and the volume traded.
pub fn paint_measure(p: &Painter, bars: &[opentern_core::Bar], tick: f64, g: &Geo, a: Anchor, b: Anchor) {
    let (pa, pb) = (pos2(g.tm.x(a.t), g.y.y(a.p)), pos2(g.tm.x(b.t), g.y.y(b.p)));
    let up = b.p >= a.p;
    let col = measure_color(up);
    let r = Rect::from_two_pos(pa, pb);
    p.rect_filled(r, CornerRadius::ZERO, col.gamma_multiply(0.2));
    let s = Stroke::new(1.0, col);
    // Arrows with small fixed heads (egui's scale with the length).
    let arrow = |from: Pos2, to: Pos2| {
        let d = to - from;
        if d.length() < 8.0 {
            return;
        }
        let u = d.normalized();
        let n = vec2(-u.y, u.x);
        p.line_segment([from, to], s);
        p.line_segment([to, to - u * 6.0 + n * 4.0], s);
        p.line_segment([to, to - u * 6.0 - n * 4.0], s);
    };
    arrow(pos2(r.center().x, pa.y), pos2(r.center().x, pb.y));
    arrow(pos2(pa.x, r.center().y), pos2(pb.x, r.center().y));
    let dp = b.p - a.p;
    let pct = if a.p != 0.0 { dp / a.p * 100.0 } else { 0.0 };
    let ticks = if tick > 0.0 { (dp / tick).round() } else { 0.0 };
    let (i0, i1) = (g.tm.index(a.t), g.tm.index(b.t));
    let nb = (i1 - i0).round() as i64;
    let span = (b.t - a.t).abs();
    let (days, hours, mins) = (span / 86_400_000, span / 3_600_000 % 24, span / 60_000 % 60);
    let time = match (days, hours, mins) {
        (d, 0, _) if d > 0 => format!("{d}d"),
        (d, h, _) if d > 0 => format!("{d}d {h}h"),
        (_, h, 0) if h > 0 => format!("{h}h"),
        (_, h, m) if h > 0 => format!("{h}h {m}m"),
        (_, _, m) => format!("{m}m"),
    };
    let (lo, hi) = (
        i0.min(i1).round().max(0.0) as usize,
        i1.max(i0).round().max(0.0) as usize,
    );
    let vol: f64 = bars
        .iter()
        .skip(lo)
        .take((hi + 1).saturating_sub(lo))
        .map(|b| b.volume)
        .sum();
    let sign = if dp >= 0.0 { "" } else { "−" };
    let mut text = format!(
        "{sign}{} ({}) {sign}{}\n{nb} bars, {time}",
        format::grouped(dp.abs(), g.decimals),
        format::signed_pct(pct),
        format::grouped(ticks.abs(), 0),
    );
    if vol > 0.0 {
        text.push_str(&format!("\nVol {}", format::compact(vol)));
    }
    let mut job = egui::text::LayoutJob::simple(text, theme::medium(12.0), Color32::WHITE, f32::INFINITY);
    job.halign = egui::Align::Center;
    let galley = p.layout_job(job);
    let size = galley.size() + vec2(20.0, 12.0);
    // Above an up move, below a down one, as TradingView; flipped or
    // pulled in when that would leave the pane.
    let above = Align2::CENTER_BOTTOM.anchor_size(pos2(r.center().x, r.top() - 8.0), size);
    let below = Align2::CENTER_TOP.anchor_size(pos2(r.center().x, r.bottom() + 8.0), size);
    let mut at = match (up, above.top() >= g.rect.top(), below.bottom() <= g.rect.bottom()) {
        (true, true, _) | (false, true, false) => above,
        (false, _, true) | (true, false, true) => below,
        _ => Align2::CENTER_TOP.anchor_size(pos2(r.center().x, g.rect.top() + 6.0), size),
    };
    let dx = (g.rect.left() + 4.0 - at.left()).max(0.0) - (at.right() - g.rect.right() + 4.0).max(0.0);
    at = at.translate(vec2(dx, 0.0));
    p.rect_filled(at, CornerRadius::same(4), col);
    p.galley(pos2(at.center().x, at.top() + 6.0), galley, Color32::WHITE);
}

/// TradingView's measure colours.
pub fn measure_color(up: bool) -> Color32 {
    if up {
        Color32::from_rgb(41, 98, 255)
    } else {
        Color32::from_rgb(242, 54, 69)
    }
}

/// Where a drawing's draggable handles are on screen.
pub fn handle_points(d: &Drawing, g: &Geo) -> Vec<Pos2> {
    match d.tool {
        Tool::AnchoredVwap => vwap_points(d, g).into_iter().take(1).collect(),
        Tool::Brush => d.pts.first().map(|a| vec![g.pos(*a)]).unwrap_or_default(),
        Tool::HorizontalLine => d
            .pts
            .first()
            .map(|a| vec![pos2(g.rect.center().x, g.y.y(a.p))])
            .unwrap_or_default(),
        Tool::VerticalLine => d
            .pts
            .first()
            .map(|a| vec![pos2(g.tm.x(a.t), g.rect.center().y)])
            .unwrap_or_default(),
        Tool::ParallelChannel if d.pts.len() >= 3 => {
            let (a, b) = (g.pos(d.pts[0]), g.pos(d.pts[1]));
            let off = g.pos(d.pts[2]).y - a.y;
            vec![a, b, pos2((a.x + b.x) / 2.0, (a.y + b.y) / 2.0 + off)]
        }
        Tool::LongPosition | Tool::ShortPosition if d.pts.len() >= 3 => {
            let x0 = g.tm.x(d.pts[0].t);
            let x1 = g.tm.x(d.pts[1].t);
            vec![
                pos2(x0, g.y.y(d.pts[0].p)),
                pos2(x0, g.y.y(d.pts[1].p)),
                pos2(x0, g.y.y(d.pts[2].p)),
                pos2(x1, g.y.y(d.pts[0].p)),
            ]
        }
        _ => d.pts.iter().map(|a| g.pos(*a)).collect(),
    }
}

/// What part of `d`, if any, is under `p`.
pub fn hit(d: &Drawing, g: &Geo, p: Pos2) -> Option<Part> {
    use Tool::*;
    if d.hidden {
        return None;
    }
    const NEAR: f32 = 7.0;
    for (i, h) in handle_points(d, g).into_iter().enumerate() {
        if h.distance(p) <= NEAR + 1.0 {
            return Some(Part::Point(i));
        }
    }
    let pts: Vec<Pos2> = d.pts.iter().map(|a| g.pos(*a)).collect();
    let r = g.rect;
    let near_seg = |a: Pos2, b: Pos2| dist_to_segment(p, a, b) <= NEAR;
    let body = match d.tool {
        TrendLine | Arrow if pts.len() >= 2 => {
            let (a, b) = extend(pts[0], pts[1], d.style.extend_left, d.style.extend_right, r);
            near_seg(a, b)
        }
        Ray if pts.len() >= 2 => {
            let (_, far) = extend(pts[0], pts[1], false, true, r);
            near_seg(pts[0], far)
        }
        ExtendedLine if pts.len() >= 2 => {
            let (a, b) = extend(pts[0], pts[1], true, true, r);
            near_seg(a, b)
        }
        HorizontalLine if !pts.is_empty() => (p.y - pts[0].y).abs() <= NEAR,
        HorizontalRay if !pts.is_empty() => (p.y - pts[0].y).abs() <= NEAR && p.x >= pts[0].x - NEAR,
        VerticalLine if !pts.is_empty() => (p.x - pts[0].x).abs() <= NEAR,
        CrossLine if !pts.is_empty() => (p.x - pts[0].x).abs() <= NEAR || (p.y - pts[0].y).abs() <= NEAR,
        ParallelChannel if pts.len() >= 3 => {
            let off = pts[2].y - pts[0].y;
            let poly = [pts[0], pts[1], pts[1] + vec2(0.0, off), pts[0] + vec2(0.0, off)];
            point_in_poly(p, &poly)
        }
        FibRetracement | FibExtension | Rectangle | Ellipse | PriceRange | DateRange | DatePriceRange
            if pts.len() >= 2 =>
        {
            let (a, b) = if d.tool == FibExtension && pts.len() >= 3 {
                (pts[0], pts[2])
            } else {
                (pts[0], pts[1])
            };
            let mut rect = Rect::from_two_pos(a, b);
            if matches!(d.tool, FibRetracement | FibExtension) {
                rect = rect.expand2(vec2(0.0, 4.0));
                if d.tool == FibExtension && d.pts.len() >= 3 {
                    let span = d.pts[1].p - d.pts[0].p;
                    let ys = FIB_LEVELS.iter().map(|(l, _)| g.y.y(d.pts[2].p + span * l));
                    let (lo, hi) = ys.fold((f32::MAX, f32::MIN), |(l, h), y| (l.min(y), h.max(y)));
                    rect = Rect::from_x_y_ranges(rect.x_range(), lo..=hi);
                }
            }
            rect.contains(p)
        }
        Pitchfork if pts.len() >= 3 => point_in_poly(p, &[pts[0], pts[1], pts[2]]) || near_seg(pts[1], pts[2]),
        Path | Brush => pts.windows(2).any(|w| near_seg(w[0], w[1])),
        Triangle if pts.len() >= 3 => point_in_poly(p, &pts),
        LongPosition | ShortPosition => position_parts(d, g).is_some_and(|(x0, x1, e, t, s)| {
            let ys = [g.y.y(e), g.y.y(t), g.y.y(s)];
            let (lo, hi) = (
                ys.iter().cloned().fold(f32::MAX, f32::min),
                ys.iter().cloned().fold(f32::MIN, f32::max),
            );
            p.x >= x0 && p.x <= x1 && p.y >= lo && p.y <= hi
        }),
        Text if !pts.is_empty() => {
            let w = (d.text.chars().count().max(4) as f32) * d.style.font as f32 * 0.6;
            Rect::from_min_size(
                pts[0] - vec2(0.0, d.style.font as f32 + 4.0),
                vec2(w, d.style.font as f32 + 6.0),
            )
            .contains(p)
        }
        ArrowUp | ArrowDown if !pts.is_empty() => pts[0].distance(p) <= 16.0,
        AnchoredVwap => vwap_points(d, g).windows(2).any(|w| near_seg(w[0], w[1])),
        FixedRangeVolumeProfile | AnchoredVolumeProfile => range_profile(d, g).is_some_and(|(prof, x0, x1)| {
            let (top, bot) = (g.y.y(prof.lo + prof.step * prof.rows() as f64), g.y.y(prof.lo));
            p.x >= x0 && p.x <= x1 && p.y >= top && p.y <= bot
        }),
        _ => false,
    };
    body.then_some(Part::Body)
}

fn point_in_poly(p: Pos2, poly: &[Pos2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + n - 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}

/// Moves part of a drawing to a new anchor (or the whole drawing by the
/// difference between `from` and `to`).
pub fn drag(d: &mut Drawing, part: Part, from: Anchor, to: Anchor, tm: &TimeMap) {
    match part {
        Part::Body => {
            let di = tm.index(to.t) - tm.index(from.t);
            let dp = to.p - from.p;
            for a in &mut d.pts {
                let i = tm.index(a.t) + di;
                a.t = time_at(tm, i);
                a.p += dp;
            }
        }
        Part::Point(k) => match d.tool {
            Tool::LongPosition | Tool::ShortPosition if d.pts.len() >= 3 => match k {
                0 => {
                    // The entry moves the whole thing in time, and its price.
                    let di = tm.index(to.t) - tm.index(d.pts[0].t);
                    let t1 = time_at(tm, tm.index(d.pts[1].t) + di);
                    d.pts[0] = to;
                    d.pts[1].t = t1;
                    d.pts[2].t = t1;
                }
                1 => d.pts[1].p = to.p,
                2 => d.pts[2].p = to.p,
                _ => {
                    d.pts[1].t = to.t;
                    d.pts[2].t = to.t;
                }
            },
            Tool::HorizontalLine => {
                if let Some(a) = d.pts.first_mut() {
                    a.p = to.p;
                }
            }
            Tool::VerticalLine => {
                if let Some(a) = d.pts.first_mut() {
                    a.t = to.t;
                }
            }
            Tool::ParallelChannel if k == 2 && d.pts.len() >= 3 => {
                // The middle handle of the second line sets the channel width.
                let mid_p = (d.pts[0].p + d.pts[1].p) / 2.0;
                let off = to.p - mid_p;
                d.pts[2] = Anchor {
                    t: d.pts[0].t,
                    p: d.pts[0].p + off,
                };
            }
            _ => {
                if let Some(a) = d.pts.get_mut(k) {
                    *a = to;
                }
            }
        },
    }
}

fn time_at(tm: &TimeMap, i: f64) -> i64 {
    let b = tm.bars;
    if b.is_empty() {
        return 0;
    }
    let last = b.len() as f64 - 1.0;
    let i = i.round();
    if i < 0.0 {
        b[0].time + (i * tm.step as f64) as i64
    } else if i > last {
        b[b.len() - 1].time + ((i - last) * tm.step as f64) as i64
    } else {
        b[i as usize].time
    }
}

/// A new drawing's first points, from where it was started.
pub fn start(id: u64, tool: Tool, a: Anchor, style: DrawStyle, tm: &TimeMap, visible_span: f64) -> Drawing {
    let pts = match tool {
        Tool::LongPosition | Tool::ShortPosition => {
            let sign = if tool == Tool::LongPosition { 1.0 } else { -1.0 };
            let t1 = time_at(tm, tm.index(a.t) + 20.0);
            let move_ = (visible_span * 0.08).max(a.p.abs() * 0.002);
            vec![
                a,
                Anchor {
                    t: t1,
                    p: a.p + sign * move_ * 2.0,
                },
                Anchor {
                    t: t1,
                    p: a.p - sign * move_,
                },
            ]
        }
        t if t.points() == 1 => vec![a],
        _ => vec![a, a],
    };
    Drawing {
        id,
        tool,
        pts,
        style,
        text: String::new(),
        locked: false,
        hidden: false,
    }
}

/// Snaps a price to the nearest open/high/low/close of the bar under the
/// pointer, when it's within reach (the magnet).
pub fn magnet(bar: Option<&Bar>, price: f64, y: &YMap, py: f32) -> f64 {
    let Some(b) = bar else { return price };
    let mut best = (f32::MAX, price);
    for v in [b.open, b.high, b.low, b.close] {
        let d = (y.y(v) - py).abs();
        if d < best.0 {
            best = (d, v);
        }
    }
    if best.0 <= 30.0 { best.1 } else { price }
}

#[cfg(test)]
mod tests {

    /// Every tool has a shortcut, none twice; TradingView's are kept.
    #[test]
    fn every_tool_has_its_own_shortcut() {
        let mut seen = std::collections::HashSet::new();
        for t in Tool::ALL {
            assert!(!t.shortcut().is_empty(), "{t:?}");
            assert!(seen.insert(t.shortcut()), "{t:?} shares {}", t.shortcut());
        }
        for g in Group::ALL {
            for t in g.tools() {
                assert!(Tool::ALL.contains(t), "{t:?} missing from Tool::ALL");
            }
        }
        assert_eq!(Tool::for_key(egui::Key::T, false), Some(Tool::TrendLine));
        assert_eq!(Tool::for_key(egui::Key::R, true), Some(Tool::Rectangle));
        assert_eq!(Tool::for_key(egui::Key::R, false), None, "Alt R resets the chart");
    }

    use super::*;

    fn bars() -> Vec<Bar> {
        (0..10)
            .map(|i| Bar {
                time: i * 3_600_000,
                open: 1.0,
                high: 2.0,
                low: 0.5,
                close: 1.5,
                volume: 1.0,
            })
            .collect()
    }

    fn geo(b: &[Bar]) -> Geo<'_> {
        Geo {
            tm: TimeMap {
                bars: b,
                step: 3_600_000,
                x: XMap {
                    right: 1000.0,
                    right_index: 10.0,
                    bar_w: 100.0,
                },
            },
            y: YMap {
                top: 0.0,
                bottom: 100.0,
                lo: 0.0,
                hi: 10.0,
                log: false,
            },
            rect: Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 100.0)),
            decimals: 2,
        }
    }

    #[test]
    fn time_maps_between_and_beyond_bars() {
        let b = bars();
        let g = geo(&b);
        assert_eq!(g.tm.index(3 * 3_600_000), 3.0);
        assert_eq!(g.tm.index(3 * 3_600_000 + 1_800_000), 3.5);
        assert_eq!(g.tm.index(12 * 3_600_000), 12.0);
        assert_eq!(g.tm.index(-2 * 3_600_000), -2.0);
        assert_eq!(g.tm.time(g.tm.x(5 * 3_600_000)), 5 * 3_600_000);
        assert_eq!(g.tm.time(g.tm.x(14 * 3_600_000)), 14 * 3_600_000);
    }

    #[test]
    fn hits_lines_and_handles_and_moves() {
        let b = bars();
        let g = geo(&b);
        let a0 = Anchor {
            t: 2 * 3_600_000,
            p: 2.0,
        };
        let a1 = Anchor {
            t: 6 * 3_600_000,
            p: 6.0,
        };
        let mut d = Drawing {
            id: 1,
            tool: Tool::TrendLine,
            pts: vec![a0, a1],
            style: DrawStyle::default(),
            text: String::new(),
            locked: false,
            hidden: false,
        };
        let p0 = g.pos(a0);
        assert_eq!(hit(&d, &g, p0), Some(Part::Point(0)));
        let mid = g.pos(Anchor {
            t: 4 * 3_600_000,
            p: 4.0,
        });
        assert_eq!(hit(&d, &g, mid), Some(Part::Body));
        assert_eq!(hit(&d, &g, mid + vec2(0.0, 30.0)), None);
        // Moving the body shifts both points by one bar and one unit.
        drag(
            &mut d,
            Part::Body,
            Anchor {
                t: 4 * 3_600_000,
                p: 4.0,
            },
            Anchor {
                t: 5 * 3_600_000,
                p: 5.0,
            },
            &g.tm,
        );
        assert_eq!(
            d.pts[0],
            Anchor {
                t: 3 * 3_600_000,
                p: 3.0
            }
        );
        assert_eq!(
            d.pts[1],
            Anchor {
                t: 7 * 3_600_000,
                p: 7.0
            }
        );
        d.hidden = true;
        assert_eq!(hit(&d, &g, g.pos(d.pts[0])), None);
    }

    #[test]
    fn every_tool_is_in_one_group_and_long_position_has_three_points() {
        for g in Group::ALL {
            for t in g.tools() {
                assert_eq!(t.group(), g);
            }
        }
        let b = bars();
        let g = geo(&b);
        let d = start(
            1,
            Tool::LongPosition,
            Anchor { t: 0, p: 5.0 },
            DrawStyle::default(),
            &g.tm,
            10.0,
        );
        assert_eq!(d.pts.len(), 3);
        assert!(d.pts[1].p > 5.0 && d.pts[2].p < 5.0);
    }

    #[test]
    fn magnet_snaps_when_close() {
        let b = bars();
        let g = geo(&b);
        // high = 2.0 sits at y = 80; a pointer at 78 snaps to it.
        assert_eq!(magnet(b.first(), 2.2, &g.y, 78.0), 2.0);
        assert_eq!(magnet(b.first(), 9.0, &g.y, 10.0), 9.0);
    }
}

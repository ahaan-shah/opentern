//! What a study draws besides plots: labels, lines, boxes, tables, fills
//! between lines and polylines (Pine's drawing objects, and the built-ins
//! that draw, like Zig Zag). Coordinates are bars (index or time) and
//! prices; the chart maps them to its pane.

use std::sync::Arc;

/// Where a drawing's x coordinates are measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XLoc {
    BarIndex,
    BarTime,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub x: i64,
    pub y: f64,
    pub xloc: XLoc,
    /// "price", "abovebar", "belowbar".
    pub yloc: Arc<str>,
    pub text: Arc<str>,
    /// `label.style_*` without the prefix ("label_down", "none", …).
    pub style: Arc<str>,
    pub color: u32,
    pub textcolor: u32,
    /// "auto", "tiny", "small", "normal", "large", "huge", or points.
    pub size: Arc<str>,
    pub textalign: Arc<str>,
    pub tooltip: Arc<str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub x1: i64,
    pub y1: f64,
    pub x2: i64,
    pub y2: f64,
    pub xloc: XLoc,
    /// "none", "left", "right", "both".
    pub extend: Arc<str>,
    pub color: u32,
    /// "solid", "dotted", "dashed", "arrow_left", "arrow_right", "arrow_both".
    pub style: Arc<str>,
    pub width: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PineBox {
    pub left: i64,
    pub top: f64,
    pub right: i64,
    pub bottom: f64,
    pub xloc: XLoc,
    pub border_color: u32,
    pub border_width: f64,
    pub border_style: Arc<str>,
    pub extend: Arc<str>,
    pub bgcolor: u32,
    pub text: Arc<str>,
    pub text_size: Arc<str>,
    pub text_color: u32,
    pub text_halign: Arc<str>,
    pub text_valign: Arc<str>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct TableCell {
    pub text: Arc<str>,
    pub width: f64,
    pub height: f64,
    pub text_color: u32,
    pub text_halign: Arc<str>,
    pub text_valign: Arc<str>,
    pub text_size: Arc<str>,
    pub bgcolor: u32,
    pub tooltip: Arc<str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    /// "top_left", "middle_center", "bottom_right", …
    pub position: Arc<str>,
    pub columns: usize,
    pub rows: usize,
    pub bgcolor: u32,
    pub frame_color: u32,
    pub frame_width: f64,
    pub border_color: u32,
    pub border_width: f64,
    /// Row-major, `columns * rows`.
    pub cells: Vec<TableCell>,
    /// Merged ranges: (col0, row0, col1, row1).
    pub merged: Vec<(usize, usize, usize, usize)>,
}

/// A fill between two lines, resolved to their coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Linefill {
    pub a: Line,
    pub b: Line,
    pub color: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Polyline {
    pub points: Vec<(i64, f64)>,
    pub xloc: XLoc,
    pub curved: bool,
    pub closed: bool,
    pub line_color: u32,
    pub fill_color: u32,
    pub line_style: Arc<str>,
    pub line_width: f64,
}

/// Everything a script has drawn, oldest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drawings {
    pub labels: Vec<Label>,
    pub lines: Vec<Line>,
    pub boxes: Vec<PineBox>,
    pub tables: Vec<Table>,
    pub linefills: Vec<Linefill>,
    pub polylines: Vec<Polyline>,
}

impl Drawings {
    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
            && self.lines.is_empty()
            && self.boxes.is_empty()
            && self.tables.is_empty()
            && self.linefills.is_empty()
            && self.polylines.is_empty()
    }
}

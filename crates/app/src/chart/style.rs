//! What the chart looks like: series type and colours, canvas, scales and
//! status line (TradingView's Chart settings), and per-indicator style
//! overrides (an indicator's Style tab). All serialisable, so they can be
//! saved in layouts, chart templates and indicator defaults.

use crate::theme::Theme;
use egui::Color32;
use opentern_series::Rgba;
use opentern_series::study::{LineStyle, Meta, PlotStyle};
use serde::{Deserialize, Serialize};

pub fn c32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), c.a())
}

pub fn rgba(c: Color32) -> Rgba {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Rgba(u32::from_be_bytes([r, g, b, a]))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeriesKind {
    Candles,
    HollowCandles,
    Bars,
    HeikinAshi,
    Line,
    LineMarkers,
    StepLine,
    Area,
    Baseline,
    Columns,
    HighLow,
    Renko,
    LineBreak,
    Kagi,
    PointFigure,
}

/// How a chart type changes the bars (see `transform.rs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transform {
    None,
    HeikinAshi,
    /// Rebuilt from all bars on every change (bricks, lines, columns).
    Renko(f64),
    LineBreak(usize),
    Kagi(f64),
    PointFigure(f64, usize),
}

impl SeriesKind {
    pub const ALL: [SeriesKind; 15] = [
        SeriesKind::Bars,
        SeriesKind::Candles,
        SeriesKind::HollowCandles,
        SeriesKind::Columns,
        SeriesKind::Line,
        SeriesKind::LineMarkers,
        SeriesKind::StepLine,
        SeriesKind::Area,
        SeriesKind::Baseline,
        SeriesKind::HighLow,
        SeriesKind::HeikinAshi,
        SeriesKind::Renko,
        SeriesKind::LineBreak,
        SeriesKind::Kagi,
        SeriesKind::PointFigure,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SeriesKind::Candles => "Candles",
            SeriesKind::HollowCandles => "Hollow candles",
            SeriesKind::Bars => "Bars",
            SeriesKind::HeikinAshi => "Heikin Ashi",
            SeriesKind::Line => "Line",
            SeriesKind::LineMarkers => "Line with markers",
            SeriesKind::StepLine => "Step line",
            SeriesKind::Area => "Area",
            SeriesKind::Baseline => "Baseline",
            SeriesKind::Columns => "Columns",
            SeriesKind::HighLow => "High-low",
            SeriesKind::Renko => "Renko",
            SeriesKind::LineBreak => "Line break",
            SeriesKind::Kagi => "Kagi",
            SeriesKind::PointFigure => "Point & figure",
        }
    }

    pub fn icon(self) -> &'static str {
        use crate::icons::ph;
        match self {
            SeriesKind::Candles | SeriesKind::HeikinAshi => ph::CHART_BAR,
            SeriesKind::HollowCandles => ph::CHART_BAR_HORIZONTAL,
            SeriesKind::Bars => ph::CHART_BAR,
            SeriesKind::Line | SeriesKind::LineMarkers => ph::CHART_LINE,
            SeriesKind::StepLine => ph::STEPS,
            SeriesKind::Area | SeriesKind::Baseline => ph::CHART_LINE_UP,
            SeriesKind::Columns | SeriesKind::HighLow => ph::CHART_BAR,
            SeriesKind::Renko | SeriesKind::LineBreak => ph::SQUARES_FOUR,
            SeriesKind::Kagi => ph::WAVE_SQUARE,
            SeriesKind::PointFigure => ph::X,
        }
    }

    /// Drawn from closes only (autoscale fits closes).
    pub fn is_line(self) -> bool {
        matches!(
            self,
            SeriesKind::Line
                | SeriesKind::LineMarkers
                | SeriesKind::StepLine
                | SeriesKind::Area
                | SeriesKind::Baseline
                | SeriesKind::Columns
        )
    }

    /// The bars aren't the symbol's time bars (no countdown, no paging by
    /// time).
    pub fn time_free(self) -> bool {
        matches!(
            self,
            SeriesKind::Renko | SeriesKind::LineBreak | SeriesKind::Kagi | SeriesKind::PointFigure
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeZoneChoice {
    /// The exchange's own timezone (TradingView's default).
    Exchange,
    Utc,
    Local,
}

/// Chart settings. Colours that are `None` follow the app theme, so a
/// chart nobody has customised looks right in light and dark.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChartStyle {
    pub kind: SeriesKind,
    // Symbol
    pub up_body: Rgba,
    pub down_body: Rgba,
    pub up_border: Rgba,
    pub down_border: Rgba,
    pub up_wick: Rgba,
    pub down_wick: Rgba,
    pub show_body: bool,
    pub show_borders: bool,
    pub show_wick: bool,
    pub color_on_prev_close: bool,
    pub line_color: Rgba,
    pub line_width: u8,
    pub area_top: Rgba,
    pub area_bottom: Rgba,
    /// Baseline: the level, as a percent of the pane height from the
    /// bottom, and the colours above and below it.
    pub baseline_level: u8,
    pub baseline_up: Rgba,
    pub baseline_down: Rgba,
    /// Renko, Kagi and Point & figure: a fixed box size, or ATR when `None`.
    pub box_size: Option<f64>,
    pub atr_length: u16,
    pub pnf_reversal: u8,
    pub line_break: u8,
    pub precision: Option<u8>,
    pub timezone: TimeZoneChoice,
    // Status line
    pub show_title: bool,
    pub show_ohlc: bool,
    pub show_bar_change: bool,
    pub show_volume: bool,
    pub show_study_titles: bool,
    pub show_study_inputs: bool,
    pub show_study_values: bool,
    // Scales and lines
    pub last_price_line: bool,
    pub last_price_label: bool,
    pub countdown: bool,
    pub study_labels: bool,
    pub high_low_labels: bool,
    // Canvas
    pub background: Option<Rgba>,
    pub grid_vert: Option<Rgba>,
    pub grid_horz: Option<Rgba>,
    pub show_grid_vert: bool,
    pub show_grid_horz: bool,
    pub crosshair: Option<Rgba>,
    pub crosshair_style: LineStyle,
    pub scale_text: Option<Rgba>,
    pub scale_font: u8,
    pub watermark: bool,
}

impl Default for ChartStyle {
    fn default() -> Self {
        let up = Rgba::hex(0x089981);
        let down = Rgba::hex(0xF23645);
        ChartStyle {
            kind: SeriesKind::Candles,
            up_body: up,
            down_body: down,
            up_border: up,
            down_border: down,
            up_wick: up,
            down_wick: down,
            show_body: true,
            show_borders: true,
            show_wick: true,
            color_on_prev_close: false,
            line_color: Rgba::hex(0x2962FF),
            line_width: 2,
            area_top: Rgba::hex(0x2962FF).transp(72),
            area_bottom: Rgba::hex(0x2962FF).transp(100),
            baseline_level: 50,
            baseline_up: Rgba::hex(0x089981),
            baseline_down: Rgba::hex(0xF23645),
            box_size: None,
            atr_length: 14,
            pnf_reversal: 3,
            line_break: 3,
            precision: None,
            timezone: TimeZoneChoice::Exchange,
            show_title: true,
            show_ohlc: true,
            show_bar_change: true,
            show_volume: false,
            show_study_titles: true,
            show_study_inputs: true,
            show_study_values: true,
            last_price_line: true,
            last_price_label: true,
            countdown: true,
            study_labels: true,
            high_low_labels: false,
            background: None,
            grid_vert: None,
            grid_horz: None,
            show_grid_vert: true,
            show_grid_horz: true,
            crosshair: None,
            crosshair_style: LineStyle::Dashed,
            scale_text: None,
            scale_font: 12,
            watermark: true,
        }
    }
}

/// The colours actually used this frame, with theme fallbacks resolved.
pub struct Palette {
    pub bg: Color32,
    pub grid_v: Color32,
    pub grid_h: Color32,
    pub crosshair: Color32,
    pub scale_text: Color32,
    pub text: Color32,
    pub text2: Color32,
    pub border: Color32,
    pub label_bg: Color32,
}

impl ChartStyle {
    /// How this chart type changes the bars; box sizes from `raw` (ATR by
    /// default, as TradingView).
    pub fn transform(&self, raw: &[opentern_core::Bar]) -> Transform {
        let boxed = || {
            self.box_size
                .filter(|v| *v > 0.0)
                .unwrap_or_else(|| super::transform::atr(raw, self.atr_length as usize))
        };
        match self.kind {
            SeriesKind::HeikinAshi => Transform::HeikinAshi,
            SeriesKind::Renko => Transform::Renko(boxed()),
            SeriesKind::LineBreak => Transform::LineBreak(self.line_break.max(1) as usize),
            SeriesKind::Kagi => Transform::Kagi(boxed()),
            SeriesKind::PointFigure => Transform::PointFigure(boxed(), self.pnf_reversal.max(1) as usize),
            _ => Transform::None,
        }
    }

    /// What decides the transform (a change means rebuilding the bars).
    pub fn transform_key(&self) -> String {
        match self.kind {
            SeriesKind::HeikinAshi => "ha".into(),
            SeriesKind::Renko | SeriesKind::Kagi => {
                format!("{:?}{:?}{}", self.kind, self.box_size, self.atr_length)
            }
            SeriesKind::PointFigure => {
                format!("pnf{:?}{}{}", self.box_size, self.atr_length, self.pnf_reversal)
            }
            SeriesKind::LineBreak => format!("lb{}", self.line_break),
            _ => String::new(),
        }
    }

    pub fn palette(&self, t: &Theme) -> Palette {
        let grid = crate::motion::lerp_color(t.bg, t.border, if t.dark { 0.55 } else { 0.6 });
        Palette {
            bg: self.background.map(c32).unwrap_or(t.bg),
            grid_v: self.grid_vert.map(c32).unwrap_or(grid),
            grid_h: self.grid_horz.map(c32).unwrap_or(grid),
            crosshair: self.crosshair.map(c32).unwrap_or(t.text3),
            scale_text: self.scale_text.map(c32).unwrap_or(t.text2),
            text: t.text,
            text2: t.text2,
            border: t.border,
            label_bg: if t.dark {
                Color32::from_rgb(0x36, 0x3A, 0x45)
            } else {
                Color32::from_rgb(0x13, 0x17, 0x22)
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlotLook {
    pub visible: bool,
    pub colors: Vec<Rgba>,
    pub width: u8,
    pub style: PlotStyle,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HLineLook {
    pub visible: bool,
    pub value: f64,
    pub color: Rgba,
    pub style: LineStyle,
    pub width: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FillLook {
    pub visible: bool,
    pub colors: Vec<Rgba>,
}

/// One indicator's Style tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StudyLook {
    pub plots: Vec<PlotLook>,
    pub hlines: Vec<HLineLook>,
    pub fills: Vec<FillLook>,
    #[serde(default = "yes")]
    pub labels_on_scale: bool,
    #[serde(default)]
    pub precision: Option<u8>,
    /// Shown on seconds, minutes, hours, days, weeks, months intervals
    /// (the Visibility tab).
    #[serde(default = "all_units")]
    pub intervals: [bool; 6],
}

fn yes() -> bool {
    true
}

fn all_units() -> [bool; 6] {
    [true; 6]
}

pub const UNIT_NAMES: [&str; 6] = ["Seconds", "Minutes", "Hours", "Days", "Weeks", "Months"];

pub fn unit_index(u: opentern_core::Unit) -> usize {
    use opentern_core::Unit::*;
    match u {
        Second => 0,
        Minute => 1,
        Hour => 2,
        Day => 3,
        Week => 4,
        Month => 5,
    }
}

impl StudyLook {
    pub fn defaults(meta: &Meta) -> StudyLook {
        StudyLook {
            plots: meta
                .plots
                .iter()
                .map(|p| PlotLook {
                    visible: p.visible,
                    colors: p.colors.iter().map(|c| c.1).collect(),
                    width: p.width,
                    style: p.style,
                })
                .collect(),
            hlines: meta
                .hlines
                .iter()
                .map(|h| HLineLook {
                    visible: h.visible,
                    value: h.value,
                    color: h.color,
                    style: h.style,
                    width: h.width,
                })
                .collect(),
            fills: meta
                .fills
                .iter()
                .map(|f| FillLook {
                    visible: f.visible,
                    colors: f.colors.iter().map(|c| c.1).collect(),
                })
                .collect(),
            labels_on_scale: true,
            precision: None,
            intervals: all_units(),
        }
    }

    /// Fits a saved look to the study's current shape (a plot added in a
    /// newer version keeps its default; extra saved entries are dropped).
    pub fn fitted(&self, meta: &Meta) -> StudyLook {
        let mut d = StudyLook::defaults(meta);
        for (a, b) in d.plots.iter_mut().zip(&self.plots) {
            if a.colors.len() == b.colors.len() {
                *a = b.clone();
            }
        }
        for (a, b) in d.hlines.iter_mut().zip(&self.hlines) {
            *a = b.clone();
        }
        for (a, b) in d.fills.iter_mut().zip(&self.fills) {
            if a.colors.len() == b.colors.len() {
                *a = b.clone();
            }
        }
        d.labels_on_scale = self.labels_on_scale;
        d.precision = self.precision;
        d.intervals = self.intervals;
        d
    }
}

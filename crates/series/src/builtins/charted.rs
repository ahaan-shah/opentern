//! Built-ins the chart draws itself, because what they show depends on
//! the screen (the visible bars) or isn't a per-bar series: Volume Profile
//! Visible Range and Session, Visible Average Price and Seasonality. Their
//! studies compute nothing per bar; the chart reads their inputs and uses
//! `crate::profile`.

use super::drawn::colour;
use super::{choice, flag, float, int};
use crate::color::Rgba;
use crate::study::*;

/// The ids the chart paints itself.
pub const CHART_DRAWN: [&str; 4] = ["vpvr", "vpsv", "vap", "seasonality"];

fn meta(
    id: &'static str,
    name: &'static str,
    short: &'static str,
    category: Category,
    placement: Placement,
    inputs: Vec<InputSpec>,
) -> Meta {
    Meta {
        id,
        name,
        short,
        category,
        placement,
        format: if placement == Placement::Pane {
            Format::Fixed(2)
        } else {
            Format::Price
        },
        inputs,
        plots: vec![],
        hlines: vec![],
        fills: vec![],
        build: |_| Box::new(NullStudy),
    }
}

/// TradingView's volume profile colours.
pub const VP_UP: Rgba = Rgba(0x2962FF4D);
pub const VP_DOWN: Rgba = Rgba(0xFBC02D4D);
pub const VP_VA_UP: Rgba = Rgba(0x2962FFB3);
pub const VP_VA_DOWN: Rgba = Rgba(0xFBC02DB3);
pub const VP_POC: Rgba = Rgba(0xFF0000FF);

fn profile_inputs(width: f64) -> Vec<InputSpec> {
    vec![
        int("rows", "Number of rows", 24, 1, 1000),
        float("va", "Value area volume %", 70.0, 0.0, 100.0, 1.0),
        choice("volume", "Volume", "Up/Down", &["Up/Down", "Total"]),
        float("width", "Width %", width, 1.0, 100.0, 1.0),
        choice("placement", "Placement", "Right", &["Right", "Left"]),
        flag("poc", "Show POC", true),
        flag("va_lines", "Show value area high and low", false),
        colour("up", "Up volume", VP_UP).group("Style"),
        colour("down", "Down volume", VP_DOWN).group("Style"),
        colour("va_up", "Value area up", VP_VA_UP).group("Style"),
        colour("va_down", "Value area down", VP_VA_DOWN).group("Style"),
        colour("poc_color", "POC", VP_POC).group("Style"),
    ]
}

pub(super) fn metas() -> Vec<Meta> {
    use Category::*;
    use Placement::*;
    let mut session = profile_inputs(70.0);
    // Session profiles sit at the start of their session.
    session.retain(|i| i.id != "placement");
    vec![
        meta(
            "vpvr",
            "Volume Profile Visible Range",
            "VPVR",
            Volume,
            Overlay,
            profile_inputs(30.0),
        ),
        meta("vpsv", "Session Volume Profile", "SVP", Volume, Overlay, session),
        meta(
            "vap",
            "Visible Average Price",
            "VAP",
            Trend,
            Overlay,
            vec![
                colour("color", "Line", Rgba::hex(0xFF9800)),
                int("width", "Line width", 1, 1, 4),
            ],
        ),
        meta(
            "seasonality",
            "Seasonality",
            "Seasonality",
            Trend,
            Pane,
            vec![int("years", "Number of years", 3, 1, 10)],
        ),
    ]
}

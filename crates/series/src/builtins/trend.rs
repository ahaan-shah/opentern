//! Trend indicators.

use super::*;
use crate::color::tv;
use crate::ta;

#[derive(Clone)]
struct Donch {
    hh: ta::Extreme,
    ll: ta::Extreme,
}

impl Donch {
    fn new(n: usize) -> Donch {
        Donch {
            hh: ta::Extreme::highest(n),
            ll: ta::Extreme::lowest(n),
        }
    }

    fn update(&mut self, high: f64, low: f64) -> f64 {
        let l = self.ll.update(low);
        let h = self.hh.update(high);
        (l + h) / 2.0
    }
}

#[derive(Clone)]
struct Ichimoku {
    conv: Donch,
    base: Donch,
    span_b: Donch,
}

impl Study for Ichimoku {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let (h, l) = (c.bar.high, c.bar.low);
        let conv = self.conv.update(h, l);
        let base = self.base.update(h, l);
        let a = (conv + base) / 2.0;
        let b = self.span_b.update(h, l);
        o.values[0] = conv;
        o.values[1] = base;
        o.values[2] = c.bar.close;
        o.values[3] = a;
        o.values[4] = b;
        o.fills[0] = if a > b { 0 } else { 1 };
    }
}

#[derive(Clone)]
struct Psar(ta::Sar);

impl Study for Psar {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        o.values[0] = self.0.update(c.bar.high, c.bar.low, c.bar.close);
    }
}

#[derive(Clone)]
struct SuperTrend(ta::Supertrend);

impl Study for SuperTrend {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let (st, dir) = self.0.update(c.bar.high, c.bar.low, c.bar.close);
        if c.index == 0 {
            return;
        }
        if dir < 0.0 {
            o.values[0] = st;
        } else {
            o.values[1] = st;
        }
        o.values[2] = (c.bar.open + c.bar.close) / 2.0;
    }
}

const ANCHORS: &[&str] = &["Session", "Week", "Month", "Quarter", "Year"];
const VWAP_MODES: &[&str] = &["Standard Deviation", "Percentage"];

#[derive(Clone)]
struct Vwap {
    src: Source,
    anchor: u8,
    hide_daily: bool,
    percent: bool,
    bands: [(bool, f64); 3],
    vwap: ta::Vwap,
    period: i64,
}

/// A number that changes when a new anchor period starts.
fn period_key(t: i64, anchor: u8) -> i64 {
    let Ok(ts) = jiff::Timestamp::from_millisecond(t) else {
        return 0;
    };
    let d = ts.to_zoned(jiff::tz::TimeZone::UTC).date();
    let (y, m) = (d.year() as i64, d.month() as i64);
    match anchor {
        1 => {
            let days = t.div_euclid(86_400_000);
            (days + 3).div_euclid(7)
        }
        2 => y * 12 + m,
        3 => y * 4 + (m - 1) / 3,
        _ => y,
    }
}

impl Study for Vwap {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let reset = if self.anchor == 0 {
            c.new_session
        } else {
            let k = period_key(c.bar.time, self.anchor);
            let r = k != self.period;
            self.period = k;
            r
        };
        let (vwap, dev) = self.vwap.update(c.src(self.src), c.bar.volume, reset || c.index == 0);
        if self.hide_daily && !c.tf.is_intraday() {
            return;
        }
        o.values[0] = vwap;
        for (k, (on, mult)) in self.bands.iter().enumerate() {
            if *on {
                let w = if self.percent { vwap * 0.01 * mult } else { dev * mult };
                o.values[1 + 2 * k] = vwap + w;
                o.values[2 + 2 * k] = vwap - w;
            }
        }
    }
}

#[derive(Clone)]
struct Alligator {
    jaw: ta::Rma,
    teeth: ta::Rma,
    lips: ta::Rma,
}

impl Study for Alligator {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let x = Source::Hl2.of(c.bar);
        o.values[0] = self.jaw.update(x);
        o.values[1] = self.teeth.update(x);
        o.values[2] = self.lips.update(x);
    }
}

#[derive(Clone)]
struct DmiStudy {
    dmi: ta::Dmi,
    adx_only: bool,
}

impl Study for DmiStudy {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let (plus, minus, adx) = self.dmi.update(c.bar.high, c.bar.low, c.bar.close);
        if self.adx_only {
            o.values[0] = adx;
        } else {
            o.values[0] = adx;
            o.values[1] = plus;
            o.values[2] = minus;
        }
    }
}

#[derive(Clone)]
struct Aroon {
    hh: ta::Extreme,
    ll: ta::Extreme,
    len: f64,
}

impl Study for Aroon {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        // ta.highestbars(high, length + 1) is minus the bars back.
        let (_, hb) = self.hh.update_with_offset(c.bar.high);
        let (_, lb) = self.ll.update_with_offset(c.bar.low);
        o.values[0] = 100.0 * (-hb + self.len) / self.len;
        o.values[1] = 100.0 * (-lb + self.len) / self.len;
    }
}

#[derive(Clone)]
struct Vortex {
    prev_high: f64,
    prev_low: f64,
    vmp: ta::Sum,
    vmm: ta::Sum,
    tr: ta::Atr,
    str_: ta::Sum,
}

impl Study for Vortex {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let b = c.bar;
        let vmp = self.vmp.update((b.high - self.prev_low).abs());
        let vmm = self.vmm.update((b.low - self.prev_high).abs());
        let s = self.str_.update(self.tr.update(b.high, b.low, b.close));
        self.prev_high = b.high;
        self.prev_low = b.low;
        o.values[0] = vmp / s;
        o.values[1] = vmm / s;
    }
}

#[derive(Clone)]
struct Chop {
    atr: ta::Atr,
    sum: ta::Sum,
    hh: ta::Extreme,
    ll: ta::Extreme,
    len: f64,
}

impl Study for Chop {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let b = c.bar;
        let s = self.sum.update(self.atr.update(b.high, b.low, b.close));
        let range = self.hh.update(b.high) - self.ll.update(b.low);
        o.values[0] = 100.0 * (s / range).log10() / self.len.log10();
    }
}

#[derive(Clone)]
struct Mass {
    e1: ta::Ema,
    e2: ta::Ema,
    sum: ta::Sum,
}

impl Study for Mass {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let span = c.bar.high - c.bar.low;
        let a = self.e1.update(span);
        let b = self.e2.update(a);
        o.values[0] = self.sum.update(a / b);
    }
}

#[derive(Clone)]
struct Dpo {
    sma: ta::Sma,
    close_back: ta::Prev,
    ma_back: ta::Prev,
    centered: bool,
}

impl Study for Dpo {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let ma = self.sma.update(c.bar.close);
        let cb = self.close_back.update(c.bar.close);
        let mb = self.ma_back.update(ma);
        o.values[0] = if self.centered { cb - ma } else { c.bar.close - mb };
    }
}

#[derive(Clone)]
struct Trix {
    e1: ta::Ema,
    e2: ta::Ema,
    e3: ta::Ema,
    ch: ta::Change,
}

impl Study for Trix {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let a = self.e1.update(c.bar.close.ln());
        let b = self.e2.update(a);
        let x = self.e3.update(b);
        o.values[0] = 10000.0 * self.ch.update(x);
    }
}

pub(super) fn metas() -> Vec<Meta> {
    let dmi_inputs = || {
        vec![
            int("adx_smoothing", "ADX Smoothing", 14, 1, 5000),
            int("di_length", "DI Length", 14, 1, 5000),
        ]
    };
    vec![
        Meta {
            id: "ichimoku",
            name: "Ichimoku Cloud",
            short: "Ichimoku",
            category: Category::Trend,
            placement: Placement::Overlay,
            format: Format::Price,
            inputs: vec![
                int("conversion", "Conversion Line Length", 9, 1, 5000),
                int("base", "Base Line Length", 26, 1, 5000),
                int("span_b", "Leading Span B Length", 52, 1, 5000),
                int("displacement", "Lagging Span", 26, 1, 5000),
            ],
            plots: vec![
                line("Conversion Line", tv::BLUE),
                line("Base Line", tv::DARK_RED),
                line("Lagging Span", tv::LEAF).offset_by("displacement", -1, 1),
                line("Leading Span A", Rgba::hex(0xA5D6A7)).offset_by("displacement", 1, -1),
                line("Leading Span B", Rgba::hex(0xEF9A9A)).offset_by("displacement", 1, -1),
            ],
            hlines: vec![],
            fills: vec![
                fill_p("Plots Background", 3, 4, Rgba::hex(0x43A047).transp(90)).colors(&[
                    ("Up", Rgba::hex(0x43A047).transp(90)),
                    ("Down", Rgba::hex(0xF44336).transp(90)),
                ]),
            ],
            build: |i| {
                Box::new(Ichimoku {
                    conv: Donch::new(i.len("conversion")),
                    base: Donch::new(i.len("base")),
                    span_b: Donch::new(i.len("span_b")),
                })
            },
        },
        Meta {
            id: "psar",
            name: "Parabolic SAR",
            short: "SAR",
            category: Category::Trend,
            placement: Placement::Overlay,
            format: Format::Price,
            inputs: vec![
                float("start", "Start", 0.02, 0.0, 10.0, 0.01),
                float("increment", "Increment", 0.02, 0.0, 10.0, 0.01),
                float("maximum", "Max value", 0.2, 0.0, 10.0, 0.01),
            ],
            plots: vec![line("ParabolicSAR", tv::BLUE).style(PlotStyle::Cross)],
            hlines: vec![],
            fills: vec![],
            build: |i| {
                Box::new(Psar(ta::Sar::new(
                    i.float("start"),
                    i.float("increment"),
                    i.float("maximum"),
                )))
            },
        },
        Meta {
            id: "supertrend",
            name: "Supertrend",
            short: "Supertrend",
            category: Category::Trend,
            placement: Placement::Overlay,
            format: Format::Price,
            inputs: vec![
                int("atr_period", "ATR Length", 10, 1, 5000),
                float("factor", "Factor", 3.0, 0.01, 100.0, 0.01),
            ],
            plots: vec![
                line("Up Trend", tv::GREEN).style(PlotStyle::LineBr),
                line("Down Trend", tv::RED).style(PlotStyle::LineBr),
                line("Body Middle", tv::GRAY).hidden(),
            ],
            hlines: vec![],
            fills: vec![
                fill_p("Uptrend Highlighter", 2, 0, tv::GREEN.transp(90)),
                fill_p("Downtrend Highlighter", 2, 1, tv::RED.transp(90)),
            ],
            build: |i| Box::new(SuperTrend(ta::Supertrend::new(i.float("factor"), i.len("atr_period")))),
        },
        Meta {
            id: "vwap",
            name: "VWAP",
            short: "VWAP",
            category: Category::Trend,
            placement: Placement::Overlay,
            format: Format::Price,
            inputs: vec![
                flag("hide_daily", "Hide VWAP on 1D or Above", true).group("VWAP Settings"),
                choice("anchor", "Anchor Period", "Session", ANCHORS).group("VWAP Settings"),
                source("hlc3").group("VWAP Settings"),
                offset_input().group("VWAP Settings"),
                choice("mode", "Bands Calculation Mode", "Standard Deviation", VWAP_MODES).group("Bands Settings"),
                flag("band1", "Bands Multiplier #1", true).group("Bands Settings"),
                float("mult1", "Multiplier #1", 1.0, 0.0, 100.0, 0.5).group("Bands Settings"),
                flag("band2", "Bands Multiplier #2", false).group("Bands Settings"),
                float("mult2", "Multiplier #2", 2.0, 0.0, 100.0, 0.5).group("Bands Settings"),
                flag("band3", "Bands Multiplier #3", false).group("Bands Settings"),
                float("mult3", "Multiplier #3", 3.0, 0.0, 100.0, 0.5).group("Bands Settings"),
            ],
            plots: vec![
                line("VWAP", tv::BLUE).offset_by("offset", 1, 0),
                line("Upper Band #1", Rgba::hex(0x4CAF50)).offset_by("offset", 1, 0),
                line("Lower Band #1", Rgba::hex(0x4CAF50)).offset_by("offset", 1, 0),
                line("Upper Band #2", Rgba::hex(0x808000)).offset_by("offset", 1, 0),
                line("Lower Band #2", Rgba::hex(0x808000)).offset_by("offset", 1, 0),
                line("Upper Band #3", Rgba::hex(0x00897B)).offset_by("offset", 1, 0),
                line("Lower Band #3", Rgba::hex(0x00897B)).offset_by("offset", 1, 0),
            ],
            hlines: vec![],
            fills: vec![
                fill_p("Bands Fill #1", 1, 2, Rgba::hex(0x4CAF50).transp(95)),
                fill_p("Bands Fill #2", 3, 4, Rgba::hex(0x808000).transp(95)),
                fill_p("Bands Fill #3", 5, 6, Rgba::hex(0x00897B).transp(95)),
            ],
            build: |i| {
                Box::new(Vwap {
                    src: i.src("source"),
                    anchor: ANCHORS.iter().position(|a| *a == i.text("anchor")).unwrap_or(0) as u8,
                    hide_daily: i.bool("hide_daily"),
                    percent: i.text("mode") == "Percentage",
                    bands: [
                        (i.bool("band1"), i.float("mult1")),
                        (i.bool("band2"), i.float("mult2")),
                        (i.bool("band3"), i.float("mult3")),
                    ],
                    vwap: ta::Vwap::default(),
                    period: i64::MIN,
                })
            },
        },
        Meta {
            id: "alligator",
            name: "Williams Alligator",
            short: "Alligator",
            category: Category::Trend,
            placement: Placement::Overlay,
            format: Format::Price,
            inputs: vec![
                int("jaw", "Jaw Length", 13, 1, 5000),
                int("teeth", "Teeth Length", 8, 1, 5000),
                int("lips", "Lips Length", 5, 1, 5000),
                int("jaw_offset", "Jaw Offset", 8, -500, 500),
                int("teeth_offset", "Teeth Offset", 5, -500, 500),
                int("lips_offset", "Lips Offset", 3, -500, 500),
            ],
            plots: vec![
                line("Jaw", tv::BLUE).offset_by("jaw_offset", 1, 0),
                line("Teeth", tv::PINK).offset_by("teeth_offset", 1, 0),
                line("Lips", Rgba::hex(0x66BB6A)).offset_by("lips_offset", 1, 0),
            ],
            hlines: vec![],
            fills: vec![],
            build: |i| {
                Box::new(Alligator {
                    jaw: ta::rma(i.len("jaw")),
                    teeth: ta::rma(i.len("teeth")),
                    lips: ta::rma(i.len("lips")),
                })
            },
        },
        Meta {
            id: "adx",
            name: "Average Directional Index",
            short: "ADX",
            category: Category::Trend,
            placement: Placement::Pane,
            format: Format::Fixed(2),
            inputs: dmi_inputs(),
            plots: vec![line("ADX", tv::MAGENTA)],
            hlines: vec![],
            fills: vec![],
            build: |i| {
                Box::new(DmiStudy {
                    dmi: ta::Dmi::new(i.len("di_length"), i.len("adx_smoothing")),
                    adx_only: true,
                })
            },
        },
        Meta {
            id: "dmi",
            name: "Directional Movement Index",
            short: "DMI",
            category: Category::Trend,
            placement: Placement::Pane,
            format: Format::Fixed(2),
            inputs: dmi_inputs(),
            plots: vec![line("ADX", tv::MAGENTA), line("+DI", tv::BLUE), line("-DI", tv::ORANGE)],
            hlines: vec![],
            fills: vec![],
            build: |i| {
                Box::new(DmiStudy {
                    dmi: ta::Dmi::new(i.len("di_length"), i.len("adx_smoothing")),
                    adx_only: false,
                })
            },
        },
        Meta {
            id: "aroon",
            name: "Aroon",
            short: "Aroon",
            category: Category::Trend,
            placement: Placement::Pane,
            format: Format::Fixed(2),
            inputs: vec![length(14)],
            plots: vec![line("Aroon Up", tv::AMBER), line("Aroon Down", tv::BLUE)],
            hlines: vec![],
            fills: vec![],
            build: |i| {
                let n = i.len("length");
                Box::new(Aroon {
                    hh: ta::Extreme::highest(n + 1),
                    ll: ta::Extreme::lowest(n + 1),
                    len: n as f64,
                })
            },
        },
        Meta {
            id: "vortex",
            name: "Vortex Indicator",
            short: "VI",
            category: Category::Trend,
            placement: Placement::Pane,
            format: Format::Fixed(4),
            inputs: vec![int("period", "Length", 14, 2, 5000)],
            plots: vec![line("VI +", tv::BLUE), line("VI -", tv::PINK)],
            hlines: vec![],
            fills: vec![],
            build: |i| {
                let n = i.len("period");
                Box::new(Vortex {
                    prev_high: f64::NAN,
                    prev_low: f64::NAN,
                    vmp: ta::Sum::new(n),
                    vmm: ta::Sum::new(n),
                    tr: ta::Atr::new(1),
                    str_: ta::Sum::new(n),
                })
            },
        },
        Meta {
            id: "chop",
            name: "Choppiness Index",
            short: "CHOP",
            category: Category::Trend,
            placement: Placement::Pane,
            format: Format::Fixed(2),
            inputs: vec![length(14), offset_input()],
            plots: vec![line("CHOP", tv::BLUE).offset_by("offset", 1, 0)],
            hlines: vec![hline("Upper Band", 61.8, LEVEL), hline("Lower Band", 38.2, LEVEL)],
            fills: vec![fill_h("Background", 0, 1, tv::BAND.transp(90))],
            build: |i| {
                let n = i.len("length");
                Box::new(Chop {
                    atr: ta::Atr::new(1),
                    sum: ta::Sum::new(n),
                    hh: ta::Extreme::highest(n),
                    ll: ta::Extreme::lowest(n),
                    len: n as f64,
                })
            },
        },
        Meta {
            id: "mass",
            name: "Mass Index",
            short: "Mass Index",
            category: Category::Trend,
            placement: Placement::Pane,
            format: Format::Fixed(4),
            inputs: vec![length(10)],
            plots: vec![line("Mass Index", tv::BLUE)],
            hlines: vec![],
            fills: vec![],
            build: |i| {
                Box::new(Mass {
                    e1: ta::ema(9),
                    e2: ta::ema(9),
                    sum: ta::Sum::new(i.len("length")),
                })
            },
        },
        Meta {
            id: "dpo",
            name: "Detrended Price Oscillator",
            short: "DPO",
            category: Category::Trend,
            placement: Placement::Pane,
            format: Format::Price,
            inputs: vec![
                int("period", "Length", 21, 1, 5000),
                flag("centered", "Centered", false),
            ],
            plots: vec![line("Detrended Price Oscillator", tv::LEAF)],
            hlines: vec![zero_line()],
            fills: vec![],
            build: |i| {
                let n = i.len("period");
                let back = n / 2 + 1;
                Box::new(Dpo {
                    sma: ta::Sma::new(n),
                    close_back: ta::Prev::new(back),
                    ma_back: ta::Prev::new(back),
                    centered: i.bool("centered"),
                })
            },
        },
        Meta {
            id: "trix",
            name: "TRIX",
            short: "TRIX",
            category: Category::Trend,
            placement: Placement::Pane,
            format: Format::Fixed(4),
            inputs: vec![length(18)],
            plots: vec![line("TRIX", tv::RED)],
            hlines: vec![zero_line()],
            fills: vec![],
            build: |i| {
                let n = i.len("length");
                Box::new(Trix {
                    e1: ta::ema(n),
                    e2: ta::ema(n),
                    e3: ta::ema(n),
                    ch: ta::Change::new(1),
                })
            },
        },
    ]
}

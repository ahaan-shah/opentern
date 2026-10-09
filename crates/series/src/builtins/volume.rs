//! Volume indicators.

use super::*;
use crate::color::tv;
use crate::ta;

#[derive(Clone)]
struct Volume {
    prev_close: bool,
    last_close: f64,
    ma: ta::Sma,
}

impl Study for Volume {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let b = c.bar;
        let down = if self.prev_close {
            self.last_close > b.close
        } else {
            b.open > b.close
        };
        self.last_close = b.close;
        o.values[0] = b.volume;
        o.colors[0] = down as u8;
        o.values[1] = self.ma.update(b.volume);
    }
}

#[derive(Clone)]
struct Obv {
    src: Source,
    ch: ta::Change,
    cum: ta::Cum,
}

impl Study for Obv {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let ch = self.ch.update(c.src(self.src));
        let sign = if ch.is_nan() {
            f64::NAN
        } else {
            ch.signum() * (ch != 0.0) as u8 as f64
        };
        o.values[0] = self.cum.update(sign * c.bar.volume);
    }
}

#[derive(Clone)]
struct Mfi(ta::Mfi);

impl Study for Mfi {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        o.values[0] = self.0.update(Source::Hlc3.of(c.bar), c.bar.volume);
    }
}

/// Close location value × volume, the heart of A/D and CMF.
fn clv_volume(b: &opentern_core::Bar) -> f64 {
    if (b.close == b.high && b.close == b.low) || b.high == b.low {
        0.0
    } else {
        ((2.0 * b.close - b.low - b.high) / (b.high - b.low)) * b.volume
    }
}

#[derive(Clone)]
struct Cmf {
    ad: ta::Sum,
    vol: ta::Sum,
}

impl Study for Cmf {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let a = self.ad.update(clv_volume(c.bar));
        let v = self.vol.update(c.bar.volume);
        o.values[0] = a / v;
    }
}

#[derive(Clone, Default)]
struct Ad(ta::Cum);

impl Study for Ad {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        o.values[0] = self.0.update(clv_volume(c.bar));
    }
}

#[derive(Clone)]
struct Eom {
    ch: ta::Change,
    sma: ta::Sma,
    div: f64,
}

impl Study for Eom {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let b = c.bar;
        let ch = self.ch.update(Source::Hl2.of(b));
        o.values[0] = self.sma.update(self.div * ch * (b.high - b.low) / b.volume);
    }
}

#[derive(Clone)]
struct Pvt {
    prev: f64,
    cum: ta::Cum,
}

impl Study for Pvt {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let x = c.bar.close;
        let v = (x - self.prev) / self.prev * c.bar.volume;
        self.prev = x;
        o.values[0] = self.cum.update(v);
    }
}

#[derive(Clone)]
struct VolOsc {
    short: ta::Ema,
    long: ta::Ema,
}

impl Study for VolOsc {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let s = self.short.update(c.bar.volume);
        let l = self.long.update(c.bar.volume);
        o.values[0] = 100.0 * (s - l) / l;
    }
}

fn vol_pane(
    id: &'static str,
    name: &'static str,
    short: &'static str,
    inputs: Vec<InputSpec>,
    plots: Vec<PlotSpec>,
    build: fn(&Inputs) -> Box<dyn Study>,
) -> Meta {
    Meta {
        id,
        name,
        short,
        category: Category::Volume,
        placement: Placement::Pane,
        format: Format::Volume,
        inputs,
        plots,
        hlines: vec![],
        fills: vec![],
        build,
    }
}

pub(super) fn metas() -> Vec<Meta> {
    let mut mfi = vol_pane(
        "mfi",
        "Money Flow Index",
        "MFI",
        vec![length(14)],
        vec![line("MF", tv::PURPLE)],
        |i| Box::new(Mfi(ta::Mfi::new(i.len("length")))),
    );
    mfi.format = Format::Fixed(2);
    (mfi.hlines, mfi.fills) = bands(80.0, 50.0, 20.0, tv::PURPLE.transp(90));
    let mut cmf = vol_pane(
        "cmf",
        "Chaikin Money Flow",
        "CMF",
        vec![length(20)],
        vec![line("MF", tv::LEAF)],
        |i| {
            Box::new(Cmf {
                ad: ta::Sum::new(i.len("length")),
                vol: ta::Sum::new(i.len("length")),
            })
        },
    );
    cmf.format = Format::Fixed(3);
    cmf.hlines = vec![hline("Zero", 0.0, LEVEL)];
    let mut vosc = vol_pane(
        "vosc",
        "Volume Oscillator",
        "Volume Osc",
        vec![
            int("short", "Short Length", 5, 1, 5000),
            int("long", "Long Length", 10, 1, 5000),
        ],
        vec![line("Volume Osc", tv::BLUE)],
        |i| {
            Box::new(VolOsc {
                short: ta::ema(i.len("short")),
                long: ta::ema(i.len("long")),
            })
        },
    );
    vosc.format = Format::Fixed(2);
    vosc.hlines = vec![zero_line()];
    let mut eom = vol_pane(
        "eom",
        "Ease of Movement",
        "EOM",
        vec![length(14), int("divisor", "Divisor", 10000, 1, 1_000_000_000)],
        vec![line("EOM", tv::LEAF)],
        |i| {
            Box::new(Eom {
                ch: ta::Change::new(1),
                sma: ta::Sma::new(i.len("length")),
                div: i.int("divisor") as f64,
            })
        },
    );
    eom.format = Format::Fixed(4);
    vec![
        Meta {
            id: "volume",
            name: "Volume",
            short: "Vol",
            category: Category::Volume,
            placement: Placement::Bottom,
            format: Format::Volume,
            inputs: vec![
                flag("prev_close", "Color based on previous close", false),
                int("ma_length", "MA Length", 20, 1, 5000),
            ],
            plots: vec![
                line("Volume", tv::GREEN.transp(50))
                    .style(PlotStyle::Columns)
                    .colors(&[("Growing", tv::GREEN.transp(50)), ("Falling", tv::RED.transp(50))]),
                line("Volume MA", tv::BLUE).hidden(),
            ],
            hlines: vec![],
            fills: vec![],
            build: |i| {
                Box::new(Volume {
                    prev_close: i.bool("prev_close"),
                    last_close: f64::NAN,
                    ma: ta::Sma::new(i.len("ma_length")),
                })
            },
        },
        vol_pane(
            "obv",
            "On Balance Volume",
            "OBV",
            vec![source("close")],
            vec![line("OnBalanceVolume", tv::BLUE)],
            |i| {
                Box::new(Obv {
                    src: i.src("source"),
                    ch: ta::Change::new(1),
                    cum: ta::Cum::default(),
                })
            },
        ),
        mfi,
        cmf,
        vol_pane(
            "ad",
            "Accumulation/Distribution",
            "Accum/Dist",
            vec![],
            vec![line("Accumulation/Distribution", Rgba::hex(0x999915))],
            |_| Box::new(Ad::default()),
        ),
        eom,
        vol_pane(
            "pvt",
            "Price Volume Trend",
            "PVT",
            vec![],
            vec![line("PVT", tv::BLUE)],
            |_| {
                Box::new(Pvt {
                    prev: f64::NAN,
                    cum: ta::Cum::default(),
                })
            },
        ),
        vosc,
    ]
}

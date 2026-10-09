//! Bands and channels.

use super::*;
use crate::color::tv;
use crate::ta;

const BASIS_MA: &[&str] = &["SMA", "EMA", "SMMA (RMA)", "WMA", "VWMA"];

#[derive(Clone)]
struct Bb {
    src: Source,
    basis: ta::Ma,
    dev: ta::Stdev,
    mult: f64,
    mode: u8,
}

impl Study for Bb {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let x = c.src(self.src);
        let basis = self.basis.update(x, c.bar.volume);
        let dev = self.mult * self.dev.update(x);
        let (upper, lower) = (basis + dev, basis - dev);
        match self.mode {
            0 => {
                o.values[0] = basis;
                o.values[1] = upper;
                o.values[2] = lower;
            }
            // %B
            1 => o.values[0] = (x - lower) / (upper - lower),
            // BandWidth, in percent
            _ => o.values[0] = (upper - lower) / basis * 100.0,
        }
    }
}

fn bb(i: &Inputs, mode: u8) -> Box<dyn Study> {
    let n = i.len("length");
    let ma = if mode == 0 { i.text("ma") } else { "SMA" };
    Box::new(Bb {
        src: i.src("source"),
        basis: ta::Ma::by_name(ma, n),
        dev: ta::Stdev::new(n),
        mult: i.float("mult"),
        mode,
    })
}

#[derive(Clone)]
struct Keltner {
    src: Source,
    sma: ta::Sma,
    ema: ta::Ema,
    exp: bool,
    style: u8,
    tr: ta::Tr,
    atr: ta::Atr,
    range: ta::Rma,
    mult: f64,
}

impl Study for Keltner {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let b = c.bar;
        let x = c.src(self.src);
        let s = self.sma.update(x);
        let e = self.ema.update(x);
        let ma = if self.exp { e } else { s };
        let tr = self.tr.update(b.high, b.low, b.close);
        let atr = self.atr.update(b.high, b.low, b.close);
        let range = self.range.update(b.high - b.low);
        let r = match self.style {
            0 => atr,
            1 => tr,
            _ => range,
        };
        o.values[0] = ma + r * self.mult;
        o.values[1] = ma;
        o.values[2] = ma - r * self.mult;
    }
}

const KC_STYLES: &[&str] = &["Average True Range", "True Range", "Range"];

#[derive(Clone)]
struct Donchian {
    hh: ta::Extreme,
    ll: ta::Extreme,
}

impl Study for Donchian {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let lower = self.ll.update(c.bar.low);
        let upper = self.hh.update(c.bar.high);
        o.values[0] = (upper + lower) / 2.0;
        o.values[1] = upper;
        o.values[2] = lower;
    }
}

#[derive(Clone)]
struct Envelope {
    src: Source,
    sma: ta::Sma,
    ema: ta::Ema,
    exp: bool,
    k: f64,
}

impl Study for Envelope {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let x = c.src(self.src);
        let s = self.sma.update(x);
        let e = self.ema.update(x);
        let basis = if self.exp { e } else { s };
        o.values[0] = basis;
        o.values[1] = basis * (1.0 + self.k);
        o.values[2] = basis * (1.0 - self.k);
    }
}

pub(super) fn metas() -> Vec<Meta> {
    vec![
        Meta {
            id: "bb",
            name: "Bollinger Bands",
            short: "BB",
            category: Category::Bands,
            placement: Placement::Overlay,
            format: Format::Price,
            inputs: vec![
                length(20),
                choice("ma", "Basis MA Type", "SMA", BASIS_MA),
                source("close"),
                float("mult", "StdDev", 2.0, 0.001, 50.0, 0.1),
                offset_input(),
            ],
            plots: vec![
                line("Basis", tv::BLUE).offset_by("offset", 1, 0),
                line("Upper", tv::RED).offset_by("offset", 1, 0),
                line("Lower", tv::GREEN).offset_by("offset", 1, 0),
            ],
            hlines: vec![],
            fills: vec![fill_p("Background", 1, 2, tv::BAND)],
            build: |i| bb(i, 0),
        },
        Meta {
            id: "bbpb",
            name: "Bollinger Bands %B",
            short: "BB %B",
            category: Category::Bands,
            placement: Placement::Pane,
            format: Format::Fixed(2),
            inputs: vec![
                length(20),
                source("close"),
                float("mult", "StdDev", 2.0, 0.001, 50.0, 0.1),
            ],
            plots: vec![line("Bollinger Bands %B", tv::TEAL)],
            hlines: vec![
                hline("Overbought", 1.0, LEVEL),
                hline("Middle Band", 0.5, MID),
                hline("Oversold", 0.0, LEVEL),
            ],
            fills: vec![fill_h("Background", 0, 2, Rgba::hex(0x26A69A).transp(90))],
            build: |i| bb(i, 1),
        },
        Meta {
            id: "bbw",
            name: "Bollinger BandWidth",
            short: "BBW",
            category: Category::Bands,
            placement: Placement::Pane,
            format: Format::Fixed(2),
            inputs: vec![
                length(20),
                source("close"),
                float("mult", "StdDev", 2.0, 0.001, 50.0, 0.1),
            ],
            plots: vec![line("Bollinger BandWidth", tv::BLUE)],
            hlines: vec![],
            fills: vec![],
            build: |i| bb(i, 2),
        },
        Meta {
            id: "kc",
            name: "Keltner Channels",
            short: "KC",
            category: Category::Bands,
            placement: Placement::Overlay,
            format: Format::Price,
            inputs: vec![
                length(20),
                float("mult", "Multiplier", 2.0, 0.01, 50.0, 0.1),
                source("close"),
                flag("exp", "Use Exponential MA", true),
                choice("style", "Bands Style", "Average True Range", KC_STYLES),
                int("atr_length", "ATR Length", 10, 1, 5000),
            ],
            plots: vec![
                line("Upper", tv::BLUE),
                line("Basis", tv::BLUE),
                line("Lower", tv::BLUE),
            ],
            hlines: vec![],
            fills: vec![fill_p("Background", 0, 2, tv::BAND)],
            build: |i| {
                let n = i.len("length");
                Box::new(Keltner {
                    src: i.src("source"),
                    sma: ta::Sma::new(n),
                    ema: ta::ema(n),
                    exp: i.bool("exp"),
                    style: KC_STYLES.iter().position(|s| *s == i.text("style")).unwrap_or(0) as u8,
                    tr: ta::Tr::new(true),
                    atr: ta::Atr::new(i.len("atr_length")),
                    range: ta::rma(n),
                    mult: i.float("mult"),
                })
            },
        },
        Meta {
            id: "dc",
            name: "Donchian Channels",
            short: "DC",
            category: Category::Bands,
            placement: Placement::Overlay,
            format: Format::Price,
            inputs: vec![length(20), offset_input()],
            plots: vec![
                line("Basis", tv::ORANGE).offset_by("offset", 1, 0),
                line("Upper", tv::BLUE).offset_by("offset", 1, 0),
                line("Lower", tv::BLUE).offset_by("offset", 1, 0),
            ],
            hlines: vec![],
            fills: vec![fill_p("Background", 1, 2, tv::BAND)],
            build: |i| {
                Box::new(Donchian {
                    hh: ta::Extreme::highest(i.len("length")),
                    ll: ta::Extreme::lowest(i.len("length")),
                })
            },
        },
        Meta {
            id: "env",
            name: "Envelope",
            short: "Env",
            category: Category::Bands,
            placement: Placement::Overlay,
            format: Format::Price,
            inputs: vec![
                length(20),
                float("percent", "Percent", 10.0, 0.0, 100.0, 0.1),
                source("close"),
                flag("exp", "Exponential", false),
            ],
            plots: vec![
                line("Basis", tv::ORANGE),
                line("Upper", tv::BLUE),
                line("Lower", tv::BLUE),
            ],
            hlines: vec![],
            fills: vec![fill_p("Background", 1, 2, tv::BAND)],
            build: |i| {
                let n = i.len("length");
                Box::new(Envelope {
                    src: i.src("source"),
                    sma: ta::Sma::new(n),
                    ema: ta::ema(n),
                    exp: i.bool("exp"),
                    k: i.float("percent") / 100.0,
                })
            },
        },
    ]
}

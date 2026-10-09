//! Moving averages.

use super::*;
use crate::color::tv;
use crate::ta;

#[derive(Clone)]
enum Calc {
    Sma(ta::Sma),
    Ema(ta::Ema),
    Wma(ta::Wma),
    Vwma(ta::Vwma),
    Hma(ta::Hma),
    Alma(ta::Alma),
    Dema(ta::Ema, ta::Ema),
    Tema(ta::Ema, ta::Ema, ta::Ema),
    Smma(ta::Rma),
    Lsma(ta::Linreg),
    McGinley { ema: ta::Ema, prev: f64, len: f64 },
}

impl Calc {
    fn update(&mut self, x: f64, volume: f64) -> f64 {
        match self {
            Calc::Sma(m) => m.update(x),
            Calc::Ema(m) | Calc::Smma(m) => m.update(x),
            Calc::Wma(m) => m.update(x),
            Calc::Vwma(m) => m.update(x, volume),
            Calc::Hma(m) => m.update(x),
            Calc::Alma(m) => m.update(x),
            Calc::Dema(a, b) => {
                let e1 = a.update(x);
                let e2 = b.update(e1);
                2.0 * e1 - e2
            }
            Calc::Tema(a, b, c) => {
                let e1 = a.update(x);
                let e2 = b.update(e1);
                let e3 = c.update(e2);
                3.0 * (e1 - e2) + e3
            }
            Calc::Lsma(m) => m.update(x),
            Calc::McGinley { ema, prev, len } => {
                // The EMA seeds the first value, as in TradingView's script.
                let e = ema.update(x);
                *prev = if prev.is_nan() {
                    e
                } else {
                    *prev + (x - *prev) / (*len * (x / *prev).powi(4))
                };
                *prev
            }
        }
    }
}

#[derive(Clone)]
struct OneMa {
    src: Source,
    calc: Calc,
}

impl Study for OneMa {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        o.values[0] = self.calc.update(c.src(self.src), c.bar.volume);
    }
}

fn one(
    id: &'static str,
    name: &'static str,
    short: &'static str,
    len: i64,
    color: Rgba,
    build: fn(&Inputs) -> Box<dyn Study>,
) -> Meta {
    Meta {
        id,
        name,
        short,
        category: Category::MovingAverages,
        placement: Placement::Overlay,
        format: Format::Price,
        inputs: vec![length(len), source("close"), offset_input()],
        plots: vec![line("MA", color).offset_by("offset", 1, 0)],
        hlines: vec![],
        fills: vec![],
        build,
    }
}

fn boxed(i: &Inputs, calc: Calc) -> Box<dyn Study> {
    Box::new(OneMa {
        src: i.src("source"),
        calc,
    })
}

#[derive(Clone)]
struct CrossMa {
    short: Calc,
    long: Calc,
    cross: ta::Cross,
}

impl Study for CrossMa {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let x = c.bar.close;
        let s = self.short.update(x, c.bar.volume);
        let l = self.long.update(x, c.bar.volume);
        let (over, under) = self.cross.update(s, l);
        o.values[0] = s;
        o.values[1] = l;
        o.values[2] = if over || under { s } else { f64::NAN };
    }
}

fn cross_meta(id: &'static str, name: &'static str, short: &'static str, s: i64, l: i64, ema: bool) -> Meta {
    Meta {
        id,
        name,
        short,
        category: Category::MovingAverages,
        placement: Placement::Overlay,
        format: Format::Price,
        inputs: vec![
            int("short", "Short MA length", s, 1, 5000),
            int("long", "Long MA length", l, 1, 5000),
        ],
        plots: vec![
            line("Short MA", tv::ORANGE),
            line("Long MA", tv::LEAF),
            line("Cross", tv::BLUE).style(PlotStyle::Cross).width(4),
        ],
        hlines: vec![],
        fills: vec![],
        build: if ema {
            |i| {
                Box::new(CrossMa {
                    short: Calc::Ema(ta::ema(i.len("short"))),
                    long: Calc::Ema(ta::ema(i.len("long"))),
                    cross: ta::Cross::default(),
                })
            }
        } else {
            |i| {
                Box::new(CrossMa {
                    short: Calc::Sma(ta::Sma::new(i.len("short"))),
                    long: Calc::Sma(ta::Sma::new(i.len("long"))),
                    cross: ta::Cross::default(),
                })
            }
        },
    }
}

pub(super) fn metas() -> Vec<Meta> {
    let mut alma = one("alma", "Arnaud Legoux Moving Average", "ALMA", 9, tv::BLUE, |i| {
        boxed(
            i,
            Calc::Alma(ta::Alma::new(
                i.len("length"),
                i.float("alma_offset"),
                i.float("sigma"),
                i.bool("floor"),
            )),
        )
    });
    alma.inputs.splice(
        1..1,
        [
            float("alma_offset", "Offset", 0.85, 0.0, 1.0, 0.01),
            float("sigma", "Sigma", 6.0, 0.1, 100.0, 0.1),
            flag("floor", "Floor", false),
        ],
    );
    let mut lsma = one("lsma", "Least Squares Moving Average", "LSMA", 25, tv::BLUE, |i| {
        boxed(i, Calc::Lsma(ta::Linreg::new(i.len("length"), i.int("offset"))))
    });
    // LSMA's offset is the regression offset, not a plot shift.
    lsma.plots[0].offset = Offset::Fixed(0);
    vec![
        one("sma", "Moving Average Simple", "SMA", 9, tv::BLUE, |i| {
            boxed(i, Calc::Sma(ta::Sma::new(i.len("length"))))
        }),
        one("ema", "Moving Average Exponential", "EMA", 9, tv::BLUE, |i| {
            boxed(i, Calc::Ema(ta::ema(i.len("length"))))
        }),
        one("wma", "Moving Average Weighted", "WMA", 9, tv::BLUE, |i| {
            boxed(i, Calc::Wma(ta::Wma::new(i.len("length"))))
        }),
        one("vwma", "Volume Weighted Moving Average", "VWMA", 20, tv::BLUE, |i| {
            boxed(i, Calc::Vwma(ta::Vwma::new(i.len("length"))))
        }),
        one("hma", "Hull Moving Average", "HMA", 9, tv::BLUE, |i| {
            boxed(i, Calc::Hma(ta::Hma::new(i.len("length"))))
        }),
        alma,
        one("dema", "Double EMA", "DEMA", 9, tv::LEAF, |i| {
            let n = i.len("length");
            boxed(i, Calc::Dema(ta::ema(n), ta::ema(n)))
        }),
        one("tema", "Triple EMA", "TEMA", 9, tv::BLUE, |i| {
            let n = i.len("length");
            boxed(i, Calc::Tema(ta::ema(n), ta::ema(n), ta::ema(n)))
        }),
        one("smma", "Smoothed Moving Average", "SMMA", 7, Rgba::hex(0x673AB7), |i| {
            boxed(i, Calc::Smma(ta::rma(i.len("length"))))
        }),
        lsma,
        one("mcginley", "McGinley Dynamic", "McGinley Dynamic", 14, tv::BLUE, |i| {
            let n = i.len("length");
            boxed(
                i,
                Calc::McGinley {
                    ema: ta::ema(n),
                    prev: f64::NAN,
                    len: n as f64,
                },
            )
        }),
        cross_meta("ma_cross", "MA Cross", "MA Cross", 9, 21, false),
        cross_meta("ema_cross", "EMA Cross", "EMA Cross", 9, 26, true),
    ]
}

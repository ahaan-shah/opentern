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
    McGinley {
        ema: ta::Ema,
        prev: f64,
        len: f64,
    },
    /// Kaufman's adaptive average: the last `len + 1` values, the fast and
    /// slow smoothing constants, and the average so far.
    Kama {
        buf: std::collections::VecDeque<f64>,
        len: usize,
        fast: f64,
        slow: f64,
        prev: f64,
    },
    /// Hamming-window weighted average of the last `weights.len()` values.
    Hamming {
        buf: std::collections::VecDeque<f64>,
        weights: Vec<f64>,
    },
}

/// Hamming window weights for `n` values (oldest first).
fn hamming_weights(n: usize) -> Vec<f64> {
    if n <= 1 {
        return vec![1.0];
    }
    (0..n)
        .map(|i| 0.54 - 0.46 * (2.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos())
        .collect()
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
            Calc::Kama {
                buf,
                len,
                fast,
                slow,
                prev,
            } => {
                if !x.is_finite() {
                    return *prev;
                }
                buf.push_back(x);
                if buf.len() > *len + 1 {
                    buf.pop_front();
                }
                if buf.len() < *len + 1 {
                    return f64::NAN;
                }
                // Efficiency ratio: net move over the window ÷ the path taken.
                let signal = (x - buf[0]).abs();
                let noise: f64 = buf.iter().zip(buf.iter().skip(1)).map(|(a, b)| (b - a).abs()).sum();
                let er = if noise > 0.0 { signal / noise } else { 0.0 };
                let sc = (er * (*fast - *slow) + *slow).powi(2);
                // Seeded with the price where the window first fills.
                *prev = if prev.is_nan() { x } else { *prev + sc * (x - *prev) };
                *prev
            }
            Calc::Hamming { buf, weights } => {
                if !x.is_finite() {
                    return f64::NAN;
                }
                buf.push_back(x);
                if buf.len() > weights.len() {
                    buf.pop_front();
                }
                if buf.len() < weights.len() {
                    return f64::NAN;
                }
                let total: f64 = weights.iter().sum();
                buf.iter().zip(weights.iter()).map(|(v, w)| v * w).sum::<f64>() / total
            }
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

/// Moving Average Channel: an average of the highs above and of the lows
/// below, each with its own length and shift, the band between shaded.
#[derive(Clone)]
struct MaChannel {
    upper: ta::Sma,
    lower: ta::Sma,
}

impl Study for MaChannel {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        o.values[0] = self.upper.update(c.bar.high);
        o.values[1] = self.lower.update(c.bar.low);
    }
}

fn ma_channel() -> Meta {
    Meta {
        id: "ma_channel",
        name: "Moving Average Channel",
        short: "MAC",
        category: Category::Bands,
        placement: Placement::Overlay,
        format: Format::Price,
        inputs: vec![
            int("upper_len", "Upper length", 20, 1, 5000),
            int("lower_len", "Lower length", 20, 1, 5000),
            int("upper_offset", "Upper offset", 0, -500, 500),
            int("lower_offset", "Lower offset", 0, -500, 500),
        ],
        plots: vec![
            line("Upper", tv::BLUE).offset_by("upper_offset", 1, 0),
            line("Lower", tv::ORANGE).offset_by("lower_offset", 1, 0),
        ],
        hlines: vec![],
        fills: vec![fill_p("Background", 0, 1, tv::BAND)],
        build: |i| {
            Box::new(MaChannel {
                upper: ta::Sma::new(i.len("upper_len")),
                lower: ta::Sma::new(i.len("lower_len")),
            })
        },
    }
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
    // Kaufman's Adaptive Moving Average: fast in a trend, flat in chop.
    let mut kama = one("kama", "Moving Average Adaptive", "KAMA", 10, tv::BLUE, |i| {
        let sc = |n: usize| 2.0 / (n as f64 + 1.0);
        boxed(
            i,
            Calc::Kama {
                buf: Default::default(),
                len: i.len("length"),
                fast: sc(i.len("fast")),
                slow: sc(i.len("slow")),
                prev: f64::NAN,
            },
        )
    });
    kama.inputs.splice(
        1..1,
        [
            int("fast", "Fast length", 2, 1, 500),
            int("slow", "Slow length", 30, 1, 500),
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
        kama,
        one("hamming", "Moving Average Hamming", "Hamming MA", 10, tv::BLUE, |i| {
            let n = i.len("length");
            boxed(
                i,
                Calc::Hamming {
                    buf: Default::default(),
                    weights: hamming_weights(n),
                },
            )
        }),
        ma_channel(),
        cross_meta("ma_cross", "MA Cross", "MA Cross", 9, 21, false),
        cross_meta("ema_cross", "EMA Cross", "EMA Cross", 9, 26, true),
    ]
}

#[cfg(test)]
mod new_ma_tests {
    use super::*;

    #[test]
    fn kama_follows_a_trend_and_holds_in_chop() {
        let mut k = Calc::Kama {
            buf: Default::default(),
            len: 10,
            fast: 2.0 / 3.0,
            slow: 2.0 / 31.0,
            prev: f64::NAN,
        };
        // Warm-up: na until the window fills, then the price itself.
        for i in 0..10 {
            assert!(k.update(100.0 + i as f64, 0.0).is_nan());
        }
        assert_eq!(k.update(110.0, 0.0), 110.0);
        // A clean trend (efficiency 1) moves it at the fast rate.
        let before = k.update(111.0, 0.0);
        assert!((before - (110.0 + (2.0_f64 / 3.0).powi(2))).abs() < 1e-9, "{before}");
        // Back and forth (efficiency ~0) barely moves it.
        let mut last = before;
        for i in 0..40 {
            last = k.update(if i % 2 == 0 { 120.0 } else { 100.0 }, 0.0);
        }
        assert!((last - before).abs() < 8.0, "{last}");
    }

    #[test]
    fn hamming_is_a_symmetric_weighted_average() {
        let w = hamming_weights(10);
        assert!((w[0] - 0.08).abs() < 1e-9 && (w[9] - 0.08).abs() < 1e-9);
        assert!((w[2] - w[7]).abs() < 1e-12);
        let mut h = Calc::Hamming {
            buf: Default::default(),
            weights: w,
        };
        for _ in 0..9 {
            assert!(h.update(5.0, 0.0).is_nan());
        }
        assert!((h.update(5.0, 0.0) - 5.0).abs() < 1e-12);
    }

    #[test]
    fn hundred_built_ins() {
        let n = crate::builtins::all().len();
        println!("{n} built-ins");
        let mut by: std::collections::BTreeMap<String, usize> = Default::default();
        for m in crate::builtins::all() {
            *by.entry(format!("{:?}", m.category)).or_default() += 1;
        }
        println!("{by:?}");
        assert!(n >= 100, "{n}");
        for id in ["kama", "hamming", "ma_channel"] {
            assert!(crate::builtins::by_id(id).is_some(), "{id}");
        }
    }
}

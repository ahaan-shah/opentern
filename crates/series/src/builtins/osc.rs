//! Oscillators and volatility measures.

use super::*;
use crate::color::tv;
use crate::ta;

const RSI_MA: &[&str] = &["None", "SMA", "EMA", "SMMA (RMA)", "WMA", "VWMA"];

#[derive(Clone)]
struct Rsi {
    src: Source,
    rsi: ta::Rsi,
    ma: Option<ta::Ma>,
}

impl Study for Rsi {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let r = self.rsi.update(c.src(self.src));
        o.values[0] = r;
        if let Some(ma) = &mut self.ma {
            o.values[1] = ma.update(r, c.bar.volume);
        }
    }
}

#[derive(Clone)]
struct Macd {
    src: Source,
    fast: ta::Ma,
    slow: ta::Ma,
    signal: ta::Ma,
    prev_hist: f64,
}

impl Study for Macd {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let x = c.src(self.src);
        let macd = self.fast.update(x, c.bar.volume) - self.slow.update(x, c.bar.volume);
        let signal = self.signal.update(macd, c.bar.volume);
        let hist = macd - signal;
        let growing = self.prev_hist < hist;
        o.values[0] = hist;
        o.colors[0] = match (hist >= 0.0, growing) {
            (true, true) => 0,
            (true, false) => 1,
            (false, true) => 2,
            (false, false) => 3,
        };
        o.values[1] = macd;
        o.values[2] = signal;
        self.prev_hist = hist;
    }
}

const OSC_MA: &[&str] = &["EMA", "SMA"];

#[derive(Clone)]
struct Stochastic {
    stoch: ta::Stoch,
    k: ta::Sma,
    d: ta::Sma,
    rsi: Option<(Source, ta::Rsi)>,
}

impl Study for Stochastic {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let raw = match &mut self.rsi {
            Some((src, rsi)) => {
                let r = rsi.update(src.of(c.bar));
                self.stoch.update(r, r, r)
            }
            None => self.stoch.update(c.bar.close, c.bar.high, c.bar.low),
        };
        let k = self.k.update(raw);
        o.values[0] = k;
        o.values[1] = self.d.update(k);
    }
}

#[derive(Clone)]
struct Cci {
    src: Source,
    cci: ta::Cci,
}

impl Study for Cci {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        o.values[0] = self.cci.update(c.src(self.src));
    }
}

#[derive(Clone)]
struct Mom {
    src: Source,
    ch: ta::Change,
}

impl Study for Mom {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        o.values[0] = self.ch.update(c.src(self.src));
    }
}

#[derive(Clone)]
struct Roc {
    src: Source,
    roc: ta::Roc,
}

impl Study for Roc {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        o.values[0] = self.roc.update(c.src(self.src));
    }
}

#[derive(Clone)]
struct WillR {
    src: Source,
    hh: ta::Extreme,
    ll: ta::Extreme,
}

impl Study for WillR {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let max = self.hh.update(c.bar.high);
        let min = self.ll.update(c.bar.low);
        o.values[0] = 100.0 * (c.src(self.src) - max) / (max - min);
    }
}

#[derive(Clone)]
struct Ao {
    fast: ta::Sma,
    slow: ta::Sma,
    prev: f64,
}

impl Study for Ao {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let x = Source::Hl2.of(c.bar);
        let ao = self.fast.update(x) - self.slow.update(x);
        let diff = ao - self.prev;
        self.prev = ao;
        o.values[0] = ao;
        o.colors[0] = if diff <= 0.0 { 1 } else { 0 };
    }
}

#[derive(Clone)]
struct Uo {
    prev_close: f64,
    sums: [(ta::Sum, ta::Sum); 3],
}

impl Study for Uo {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let b = c.bar;
        let high = ta::max(b.high, self.prev_close);
        let low = ta::min(b.low, self.prev_close);
        self.prev_close = b.close;
        let bp = b.close - low;
        let tr = high - low;
        let avg: Vec<f64> = self
            .sums
            .iter_mut()
            .map(|(s_bp, s_tr)| s_bp.update(bp) / s_tr.update(tr))
            .collect();
        o.values[0] = 100.0 * (4.0 * avg[0] + 2.0 * avg[1] + avg[2]) / 7.0;
    }
}

#[derive(Clone)]
struct Tsi {
    ch: ta::Change,
    pc_long: ta::Ema,
    pc_short: ta::Ema,
    abs_long: ta::Ema,
    abs_short: ta::Ema,
    signal: ta::Ema,
}

impl Study for Tsi {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let pc = self.ch.update(c.bar.close);
        let a = self.pc_short.update(self.pc_long.update(pc));
        let b = self.abs_short.update(self.abs_long.update(pc.abs()));
        let tsi = 100.0 * (a / b);
        o.values[0] = tsi;
        o.values[1] = self.signal.update(tsi);
    }
}

#[derive(Clone)]
struct Cmo {
    src: Source,
    ch: ta::Change,
    up: ta::Sum,
    down: ta::Sum,
}

impl Study for Cmo {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let m = self.ch.update(c.src(self.src));
        let m1 = if m >= 0.0 { m } else { 0.0 };
        let m2 = if m >= 0.0 { 0.0 } else { -m };
        let s1 = self.up.update(m1);
        let s2 = self.down.update(m2);
        o.values[0] = 100.0 * (s1 - s2) / (s1 + s2);
    }
}

#[derive(Clone)]
struct Fisher {
    hh: ta::Extreme,
    ll: ta::Extreme,
    value: f64,
    fish: f64,
}

impl Study for Fisher {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let x = Source::Hl2.of(c.bar);
        let high = self.hh.update(x);
        let low = self.ll.update(x);
        let round = |v: f64| {
            if v > 0.99 {
                0.999
            } else if v < -0.99 {
                -0.999
            } else {
                v
            }
        };
        let value = round(0.66 * ((x - low) / (high - low) - 0.5) + 0.67 * ta::nz(self.value, 0.0));
        let prev_fish = self.fish;
        let fish = 0.5 * ((1.0 + value) / (1.0 - value)).ln() + 0.5 * ta::nz(prev_fish, 0.0);
        self.value = value;
        self.fish = fish;
        o.values[0] = fish;
        o.values[1] = prev_fish;
    }
}

#[derive(Clone)]
struct Ppo {
    src: Source,
    short: (ta::Sma, ta::Ema),
    long: (ta::Sma, ta::Ema),
    exp: bool,
}

impl Study for Ppo {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let x = c.src(self.src);
        let (ss, se) = (self.short.0.update(x), self.short.1.update(x));
        let (ls, le) = (self.long.0.update(x), self.long.1.update(x));
        let (s, l) = if self.exp { (se, le) } else { (ss, ls) };
        o.values[0] = (s - l) / l * 100.0;
    }
}

#[derive(Clone)]
struct Kst {
    parts: Vec<(ta::Roc, ta::Sma)>,
    signal: ta::Sma,
}

impl Study for Kst {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let mut kst = 0.0;
        for (k, (roc, sma)) in self.parts.iter_mut().enumerate() {
            kst += (k as f64 + 1.0) * sma.update(roc.update(c.bar.close));
        }
        o.values[0] = kst;
        o.values[1] = self.signal.update(kst);
    }
}

#[derive(Clone)]
struct Coppock {
    src: Source,
    long: ta::Roc,
    short: ta::Roc,
    wma: ta::Wma,
}

impl Study for Coppock {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let x = c.src(self.src);
        let s = self.long.update(x) + self.short.update(x);
        o.values[0] = self.wma.update(s);
    }
}

#[derive(Clone)]
struct Bbp {
    ema: ta::Ema,
}

impl Study for Bbp {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let e = self.ema.update(c.bar.close);
        o.values[0] = (c.bar.high - e) + (c.bar.low - e);
    }
}

#[derive(Clone)]
struct Efi {
    ch: ta::Change,
    ema: ta::Ema,
}

impl Study for Efi {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        o.values[0] = self.ema.update(self.ch.update(c.bar.close) * c.bar.volume);
    }
}

#[derive(Clone)]
struct Bop;

impl Study for Bop {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let b = c.bar;
        o.values[0] = (b.close - b.open) / (b.high - b.low);
    }
}

const ATR_MA: &[&str] = &["RMA", "SMA", "EMA", "WMA"];

#[derive(Clone)]
struct AtrStudy {
    tr: ta::Tr,
    ma: ta::Ma,
}

impl Study for AtrStudy {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let tr = self.tr.update(c.bar.high, c.bar.low, c.bar.close);
        o.values[0] = self.ma.update(tr, c.bar.volume);
    }
}

#[derive(Clone)]
struct Hv {
    prev: f64,
    dev: ta::Stdev,
}

impl Study for Hv {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let r = (c.bar.close / self.prev).ln();
        self.prev = c.bar.close;
        let per = if c.tf.is_intraday() || (c.tf.unit == opentern_core::Unit::Day && c.tf.count == 1) {
            1.0
        } else {
            7.0
        };
        o.values[0] = 100.0 * self.dev.update(r) * (365.0f64 / per).sqrt();
    }
}

#[derive(Clone)]
struct StdDev {
    src: Source,
    dev: ta::Stdev,
}

impl Study for StdDev {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        o.values[0] = self.dev.update(c.src(self.src));
    }
}

fn pane(
    id: &'static str,
    name: &'static str,
    short: &'static str,
    category: Category,
    inputs: Vec<InputSpec>,
    plots: Vec<PlotSpec>,
    build: fn(&Inputs) -> Box<dyn Study>,
) -> Meta {
    Meta {
        id,
        name,
        short,
        category,
        placement: Placement::Pane,
        format: Format::Fixed(2),
        inputs,
        plots,
        hlines: vec![],
        fills: vec![],
        build,
    }
}

fn with_levels(mut m: Meta, levels: (Vec<HLineSpec>, Vec<FillSpec>)) -> Meta {
    m.hlines = levels.0;
    m.fills = levels.1;
    m
}

fn with_zero(mut m: Meta) -> Meta {
    m.hlines = vec![zero_line()];
    m
}

pub(super) fn metas() -> Vec<Meta> {
    use Category::{Oscillators as O, Volatility as V};
    let purple_band = tv::PURPLE.transp(90);
    let blue_band = Rgba::hex(0x2196F3).transp(90);
    let mut macd = pane(
        "macd",
        "MACD",
        "MACD",
        O,
        vec![
            int("fast", "Fast Length", 12, 1, 5000),
            int("slow", "Slow Length", 26, 1, 5000),
            source("close"),
            int("signal", "Signal Smoothing", 9, 1, 50),
            choice("osc_ma", "Oscillator MA Type", "EMA", OSC_MA),
            choice("signal_ma", "Signal Line MA Type", "EMA", OSC_MA),
        ],
        vec![
            line("Histogram", Rgba::hex(0x26A69A))
                .style(PlotStyle::Columns)
                .colors(&[
                    ("Grow above", Rgba::hex(0x26A69A)),
                    ("Fall above", Rgba::hex(0xB2DFDB)),
                    ("Grow below", Rgba::hex(0xFFCDD2)),
                    ("Fall below", Rgba::hex(0xFF5252)),
                ]),
            line("MACD", tv::BLUE),
            line("Signal line", tv::ORANGE),
        ],
        |i| {
            Box::new(Macd {
                src: i.src("source"),
                fast: ta::Ma::by_name(i.text("osc_ma"), i.len("fast")),
                slow: ta::Ma::by_name(i.text("osc_ma"), i.len("slow")),
                signal: ta::Ma::by_name(i.text("signal_ma"), i.len("signal")),
                prev_hist: f64::NAN,
            })
        },
    );
    macd.format = Format::Price;
    macd.hlines = vec![zero_line()];

    let mut ao = pane("ao", "Awesome Oscillator", "AO", O, vec![], vec![], |_| {
        Box::new(Ao {
            fast: ta::Sma::new(5),
            slow: ta::Sma::new(34),
            prev: f64::NAN,
        })
    });
    ao.format = Format::Price;
    ao.plots = vec![
        line("AO", tv::GREEN)
            .style(PlotStyle::Columns)
            .colors(&[("Growing", tv::GREEN), ("Falling", tv::RED)]),
    ];

    vec![
        with_levels(
            pane(
                "rsi",
                "Relative Strength Index",
                "RSI",
                O,
                vec![
                    int("length", "RSI Length", 14, 1, 5000).group("RSI Settings"),
                    source("close").group("RSI Settings"),
                    choice("ma", "Type", "SMA", RSI_MA).group("Smoothing"),
                    int("ma_length", "Length", 14, 1, 5000).group("Smoothing"),
                ],
                vec![line("RSI", tv::PURPLE), line("RSI-based MA", tv::YELLOW)],
                |i| {
                    let ma = i.text("ma");
                    Box::new(Rsi {
                        src: i.src("source"),
                        rsi: ta::Rsi::new(i.len("length")),
                        ma: (ma != "None").then(|| ta::Ma::by_name(ma, i.len("ma_length"))),
                    })
                },
            ),
            bands(70.0, 50.0, 30.0, purple_band),
        ),
        macd,
        with_levels(
            pane(
                "stoch",
                "Stochastic",
                "Stoch",
                O,
                vec![
                    int("k", "%K Length", 14, 1, 5000),
                    int("k_smooth", "%K Smoothing", 1, 1, 5000),
                    int("d", "%D Smoothing", 3, 1, 5000),
                ],
                vec![line("%K", tv::BLUE), line("%D", tv::ORANGE)],
                |i| {
                    Box::new(Stochastic {
                        stoch: ta::Stoch::new(i.len("k")),
                        k: ta::Sma::new(i.len("k_smooth")),
                        d: ta::Sma::new(i.len("d")),
                        rsi: None,
                    })
                },
            ),
            bands(80.0, 50.0, 20.0, blue_band),
        ),
        with_levels(
            pane(
                "stochrsi",
                "Stochastic RSI",
                "Stoch RSI",
                O,
                vec![
                    int("k", "K", 3, 1, 5000),
                    int("d", "D", 3, 1, 5000),
                    int("rsi_length", "RSI Length", 14, 1, 5000),
                    int("stoch_length", "Stochastic Length", 14, 1, 5000),
                    source("close"),
                ],
                vec![line("K", tv::BLUE), line("D", tv::ORANGE)],
                |i| {
                    Box::new(Stochastic {
                        stoch: ta::Stoch::new(i.len("stoch_length")),
                        k: ta::Sma::new(i.len("k")),
                        d: ta::Sma::new(i.len("d")),
                        rsi: Some((i.src("source"), ta::Rsi::new(i.len("rsi_length")))),
                    })
                },
            ),
            bands(80.0, 50.0, 20.0, blue_band),
        ),
        with_levels(
            pane(
                "cci",
                "Commodity Channel Index",
                "CCI",
                O,
                vec![length(20), source("hlc3")],
                vec![line("CCI", tv::BLUE)],
                |i| {
                    Box::new(Cci {
                        src: i.src("source"),
                        cci: ta::Cci::new(i.len("length")),
                    })
                },
            ),
            bands(100.0, 0.0, -100.0, blue_band),
        ),
        with_zero({
            let mut m = pane(
                "mom",
                "Momentum",
                "Mom",
                O,
                vec![length(10), source("close")],
                vec![line("Momentum", tv::BLUE)],
                |i| {
                    Box::new(Mom {
                        src: i.src("source"),
                        ch: ta::Change::new(i.len("length")),
                    })
                },
            );
            m.format = Format::Price;
            m
        }),
        with_zero(pane(
            "roc",
            "Rate Of Change",
            "ROC",
            O,
            vec![length(9), source("close")],
            vec![line("ROC", tv::BLUE)],
            |i| {
                Box::new(Roc {
                    src: i.src("source"),
                    roc: ta::Roc::new(i.len("length")),
                })
            },
        )),
        with_levels(
            pane(
                "willr",
                "Williams %R",
                "%R",
                O,
                vec![length(14), source("close")],
                vec![line("%R", tv::PURPLE)],
                |i| {
                    let n = i.len("length");
                    Box::new(WillR {
                        src: i.src("source"),
                        hh: ta::Extreme::highest(n),
                        ll: ta::Extreme::lowest(n),
                    })
                },
            ),
            bands(-20.0, -50.0, -80.0, purple_band),
        ),
        ao,
        pane(
            "uo",
            "Ultimate Oscillator",
            "UO",
            O,
            vec![
                int("fast", "Fast Length", 7, 1, 5000),
                int("middle", "Middle Length", 14, 1, 5000),
                int("slow", "Slow Length", 28, 1, 5000),
            ],
            vec![line("UO", tv::RED)],
            |i| {
                let s = |id| (ta::Sum::new(i.len(id)), ta::Sum::new(i.len(id)));
                Box::new(Uo {
                    prev_close: f64::NAN,
                    sums: [s("fast"), s("middle"), s("slow")],
                })
            },
        ),
        with_zero(pane(
            "tsi",
            "True Strength Index",
            "TSI",
            O,
            vec![
                int("long", "Long Length", 25, 1, 5000),
                int("short", "Short Length", 13, 1, 5000),
                int("signal", "Signal Length", 13, 1, 5000),
            ],
            vec![line("True Strength Index", tv::BLUE), line("Signal", tv::PINK)],
            |i| {
                let (l, s) = (i.len("long"), i.len("short"));
                Box::new(Tsi {
                    ch: ta::Change::new(1),
                    pc_long: ta::ema(l),
                    pc_short: ta::ema(s),
                    abs_long: ta::ema(l),
                    abs_short: ta::ema(s),
                    signal: ta::ema(i.len("signal")),
                })
            },
        )),
        with_zero(pane(
            "cmo",
            "Chande Momentum Oscillator",
            "ChandeMO",
            O,
            vec![length(9), source("close")],
            vec![line("Chande MO", tv::BLUE)],
            |i| {
                let n = i.len("length");
                Box::new(Cmo {
                    src: i.src("source"),
                    ch: ta::Change::new(1),
                    up: ta::Sum::new(n),
                    down: ta::Sum::new(n),
                })
            },
        )),
        {
            let mut m = pane(
                "fisher",
                "Fisher Transform",
                "Fisher",
                O,
                vec![length(9)],
                vec![line("Fisher", tv::BLUE), line("Trigger", tv::ORANGE)],
                |i| {
                    let n = i.len("length");
                    Box::new(Fisher {
                        hh: ta::Extreme::highest(n),
                        ll: ta::Extreme::lowest(n),
                        value: f64::NAN,
                        fish: f64::NAN,
                    })
                },
            );
            m.hlines = vec![
                hline("1.5", 1.5, tv::PINK),
                hline("0.75", 0.75, LEVEL).style(LineStyle::Dotted),
                hline("0", 0.0, tv::PINK),
                hline("-0.75", -0.75, LEVEL).style(LineStyle::Dotted),
                hline("-1.5", -1.5, tv::PINK),
            ];
            m
        },
        with_zero(pane(
            "ppo",
            "Price Oscillator",
            "PPO",
            O,
            vec![
                int("short", "Short Length", 10, 1, 5000),
                int("long", "Long Length", 21, 1, 5000),
                source("close"),
                flag("exp", "Use Exponential MA", false),
            ],
            vec![line("Price Oscillator", tv::BLUE)],
            |i| {
                let (s, l) = (i.len("short"), i.len("long"));
                Box::new(Ppo {
                    src: i.src("source"),
                    short: (ta::Sma::new(s), ta::ema(s)),
                    long: (ta::Sma::new(l), ta::ema(l)),
                    exp: i.bool("exp"),
                })
            },
        )),
        with_zero({
            let mut m = pane(
                "kst",
                "Know Sure Thing",
                "KST",
                O,
                vec![
                    int("roc1", "ROC Length #1", 10, 1, 5000),
                    int("roc2", "ROC Length #2", 15, 1, 5000),
                    int("roc3", "ROC Length #3", 20, 1, 5000),
                    int("roc4", "ROC Length #4", 30, 1, 5000),
                    int("sma1", "SMA Length #1", 10, 1, 5000),
                    int("sma2", "SMA Length #2", 10, 1, 5000),
                    int("sma3", "SMA Length #3", 10, 1, 5000),
                    int("sma4", "SMA Length #4", 15, 1, 5000),
                    int("signal", "Signal Line Length", 9, 1, 5000),
                ],
                vec![line("KST", tv::GREEN), line("Signal", tv::RED)],
                |i| {
                    let part = |r: &str, s: &str| (ta::Roc::new(i.len(r)), ta::Sma::new(i.len(s)));
                    Box::new(Kst {
                        parts: vec![
                            part("roc1", "sma1"),
                            part("roc2", "sma2"),
                            part("roc3", "sma3"),
                            part("roc4", "sma4"),
                        ],
                        signal: ta::Sma::new(i.len("signal")),
                    })
                },
            );
            m.format = Format::Fixed(4);
            m
        }),
        pane(
            "coppock",
            "Coppock Curve",
            "Coppock Curve",
            O,
            vec![
                int("wma", "WMA Length", 10, 1, 5000),
                int("long_roc", "Long RoC Length", 14, 1, 5000),
                int("short_roc", "Short RoC Length", 11, 1, 5000),
                source("close"),
            ],
            vec![line("Coppock Curve", tv::BLUE)],
            |i| {
                Box::new(Coppock {
                    src: i.src("source"),
                    long: ta::Roc::new(i.len("long_roc")),
                    short: ta::Roc::new(i.len("short_roc")),
                    wma: ta::Wma::new(i.len("wma")),
                })
            },
        ),
        {
            let mut m = pane(
                "bbp",
                "Bull Bear Power",
                "BBP",
                O,
                vec![length(13)],
                vec![line("BBPower", tv::BLUE).style(PlotStyle::Columns)],
                |i| {
                    Box::new(Bbp {
                        ema: ta::ema(i.len("length")),
                    })
                },
            );
            m.format = Format::Price;
            m
        },
        with_zero({
            let mut m = pane(
                "efi",
                "Elder Force Index",
                "EFI",
                O,
                vec![length(13)],
                vec![line("Elder Force Index", tv::RED)],
                |i| {
                    Box::new(Efi {
                        ch: ta::Change::new(1),
                        ema: ta::ema(i.len("length")),
                    })
                },
            );
            m.format = Format::Volume;
            m
        }),
        pane(
            "bop",
            "Balance of Power",
            "BOP",
            O,
            vec![],
            vec![line("BOP", tv::RED)],
            |_| Box::new(Bop),
        ),
        {
            let mut m = pane(
                "atr",
                "Average True Range",
                "ATR",
                V,
                vec![length(14), choice("smoothing", "Smoothing", "RMA", ATR_MA)],
                vec![line("ATR", tv::DARK_RED)],
                |i| {
                    let kind = match i.text("smoothing") {
                        "RMA" => "SMMA (RMA)",
                        other => other,
                    };
                    Box::new(AtrStudy {
                        tr: ta::Tr::new(true),
                        ma: ta::Ma::by_name(kind, i.len("length")),
                    })
                },
            );
            m.format = Format::Price;
            m
        },
        pane(
            "hv",
            "Historical Volatility",
            "HV",
            V,
            vec![length(10)],
            vec![line("HV", tv::BLUE)],
            |i| {
                Box::new(Hv {
                    prev: f64::NAN,
                    dev: ta::Stdev::new(i.len("length")),
                })
            },
        ),
        {
            let mut m = pane(
                "stdev",
                "Standard Deviation",
                "StDev",
                V,
                vec![length(20), source("close")],
                vec![line("StDev", tv::BLUE)],
                |i| {
                    Box::new(StdDev {
                        src: i.src("source"),
                        dev: ta::Stdev::new(i.len("length")),
                    })
                },
            );
            m.format = Format::Price;
            m
        },
    ]
}

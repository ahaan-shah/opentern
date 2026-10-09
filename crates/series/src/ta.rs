//! Technical analysis primitives with Pine Script's semantics, one value per
//! bar. Each is a small state machine: call `update` exactly once per bar,
//! oldest bar first. `na` is `f64::NAN`; it propagates the way Pine's does.
//!
//! The exact rules (seeding, warm-up, `na` handling) are written down in
//! `docs/ta-semantics.md`. Indicators are built only from these, line by
//! line like their Pine source, so the semantics can't drift.
//!
//! Every type is `Clone`: the runtime clones a study's state to compute the
//! forming bar, then throws the clone away (Pine's realtime rollback).

use std::collections::VecDeque;

pub const NA: f64 = f64::NAN;

pub fn na(x: f64) -> bool {
    x.is_nan()
}

/// Pine's `nz`: `na` becomes `replacement`.
pub fn nz(x: f64, replacement: f64) -> f64 {
    if x.is_nan() { replacement } else { x }
}

/// Pine's `math.max`/`math.min`: `na` if either side is `na`.
pub fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { NA } else { a.max(b) }
}

pub fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { NA } else { a.min(b) }
}

/// Division where dividing by zero gives `na`, as in Pine.
pub fn div(a: f64, b: f64) -> f64 {
    if b == 0.0 { NA } else { a / b }
}

/// The last `cap` values of a series, newest at index 0 (Pine's `x[i]`).
#[derive(Clone, Debug)]
pub struct Hist {
    buf: VecDeque<f64>,
    cap: usize,
    /// Values seen so far (can exceed `cap`).
    seen: usize,
}

impl Hist {
    pub fn new(cap: usize) -> Hist {
        let cap = cap.max(1);
        Hist {
            buf: VecDeque::with_capacity(cap),
            cap,
            seen: 0,
        }
    }

    pub fn push(&mut self, x: f64) {
        if self.buf.len() == self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(x);
        self.seen += 1;
    }

    /// `x[i]`; `na` before the series has that many values.
    pub fn get(&self, i: usize) -> f64 {
        if i >= self.buf.len() {
            return NA;
        }
        self.buf[self.buf.len() - 1 - i]
    }

    /// The window `x[0], x[1], …, x[n-1]` if there have been `n` values.
    pub fn window(&self, n: usize) -> Option<impl Iterator<Item = f64> + '_> {
        (self.buf.len() >= n && n > 0).then(|| self.buf.iter().rev().take(n).copied())
    }

    pub fn seen(&self) -> usize {
        self.seen
    }
}

/// `x[n]`: the value `n` bars ago.
#[derive(Clone, Debug)]
pub struct Prev {
    h: Hist,
    n: usize,
}

impl Prev {
    pub fn new(n: usize) -> Prev {
        Prev { h: Hist::new(n + 1), n }
    }

    /// Pushes `x` and returns `x[n]`.
    pub fn update(&mut self, x: f64) -> f64 {
        self.h.push(x);
        self.h.get(self.n)
    }
}

/// `math.sum(x, len)`: `na` until `len` values, and if any is `na`.
#[derive(Clone, Debug)]
pub struct Sum {
    h: Hist,
    len: usize,
}

impl Sum {
    pub fn new(len: usize) -> Sum {
        let len = len.max(1);
        Sum { h: Hist::new(len), len }
    }

    pub fn update(&mut self, x: f64) -> f64 {
        self.h.push(x);
        match self.h.window(self.len) {
            Some(w) => w.sum(),
            None => NA,
        }
    }
}

/// `ta.sma`.
#[derive(Clone, Debug)]
pub struct Sma {
    sum: Sum,
    len: usize,
}

impl Sma {
    pub fn new(len: usize) -> Sma {
        let len = len.max(1);
        Sma {
            sum: Sum::new(len),
            len,
        }
    }

    pub fn update(&mut self, x: f64) -> f64 {
        self.sum.update(x) / self.len as f64
    }
}

/// Exponential smoothing seeded with the SMA of the first `len` values:
/// `ta.ema` (alpha = 2 / (len + 1)) and `ta.rma` (alpha = 1 / len).
#[derive(Clone, Debug)]
pub struct Smooth {
    alpha: f64,
    seed: Sma,
    prev: f64,
}

impl Smooth {
    pub fn update(&mut self, x: f64) -> f64 {
        // The seed SMA sees every bar, as Pine evaluates it every bar.
        let s = self.seed.update(x);
        self.prev = if self.prev.is_nan() {
            s
        } else {
            self.alpha * x + (1.0 - self.alpha) * self.prev
        };
        self.prev
    }
}

/// `ta.ema`.
pub fn ema(len: usize) -> Smooth {
    let len = len.max(1);
    Smooth {
        alpha: 2.0 / (len as f64 + 1.0),
        seed: Sma::new(len),
        prev: NA,
    }
}

/// `ta.rma` (Wilder's smoothing, also "SMMA").
pub fn rma(len: usize) -> Smooth {
    let len = len.max(1);
    Smooth {
        alpha: 1.0 / len as f64,
        seed: Sma::new(len),
        prev: NA,
    }
}

pub type Ema = Smooth;
pub type Rma = Smooth;

/// `ta.wma`: linearly weighted, newest weighted `len`.
#[derive(Clone, Debug)]
pub struct Wma {
    h: Hist,
    len: usize,
}

impl Wma {
    pub fn new(len: usize) -> Wma {
        let len = len.max(1);
        Wma { h: Hist::new(len), len }
    }

    pub fn update(&mut self, x: f64) -> f64 {
        self.h.push(x);
        let Some(w) = self.h.window(self.len) else { return NA };
        let n = self.len as f64;
        let norm = n * (n + 1.0) / 2.0;
        w.enumerate().map(|(i, v)| v * (n - i as f64)).sum::<f64>() / norm
    }
}

/// `ta.vwma`: `sma(x * volume) / sma(volume)`.
#[derive(Clone, Debug)]
pub struct Vwma {
    xv: Sma,
    v: Sma,
}

impl Vwma {
    pub fn new(len: usize) -> Vwma {
        Vwma {
            xv: Sma::new(len),
            v: Sma::new(len),
        }
    }

    pub fn update(&mut self, x: f64, volume: f64) -> f64 {
        let a = self.xv.update(x * volume);
        let b = self.v.update(volume);
        div(a, b)
    }
}

/// `ta.hma`: `wma(2 * wma(x, len / 2) - wma(x, len), floor(sqrt(len)))`.
#[derive(Clone, Debug)]
pub struct Hma {
    half: Wma,
    full: Wma,
    out: Wma,
}

impl Hma {
    pub fn new(len: usize) -> Hma {
        let len = len.max(2);
        Hma {
            half: Wma::new(len / 2),
            full: Wma::new(len),
            out: Wma::new((len as f64).sqrt().floor() as usize),
        }
    }

    pub fn update(&mut self, x: f64) -> f64 {
        let a = self.half.update(x);
        let b = self.full.update(x);
        self.out.update(2.0 * a - b)
    }
}

/// `ta.alma` (Arnaud Legoux): Gaussian weights centred at `offset`.
#[derive(Clone, Debug)]
pub struct Alma {
    h: Hist,
    weights: Vec<f64>,
    norm: f64,
}

impl Alma {
    pub fn new(len: usize, offset: f64, sigma: f64, floor: bool) -> Alma {
        let len = len.max(1);
        let mut m = offset * (len as f64 - 1.0);
        if floor {
            m = m.floor();
        }
        let s = len as f64 / sigma;
        // weights[i] applies to x[len - 1 - i] (oldest first).
        let weights: Vec<f64> = (0..len)
            .map(|i| (-(i as f64 - m).powi(2) / (2.0 * s * s)).exp())
            .collect();
        let norm = weights.iter().sum();
        Alma {
            h: Hist::new(len),
            weights,
            norm,
        }
    }

    pub fn update(&mut self, x: f64) -> f64 {
        self.h.push(x);
        let len = self.weights.len();
        if self.h.window(len).is_none() {
            return NA;
        }
        let sum: f64 = (0..len).map(|i| self.h.get(len - 1 - i) * self.weights[i]).sum();
        sum / self.norm
    }
}

/// `ta.linreg(x, len, offset)`: the least-squares line through the last
/// `len` values, evaluated `offset` bars back from the newest.
#[derive(Clone, Debug)]
pub struct Linreg {
    h: Hist,
    len: usize,
    offset: i64,
}

impl Linreg {
    pub fn new(len: usize, offset: i64) -> Linreg {
        let len = len.max(1);
        Linreg {
            h: Hist::new(len),
            len,
            offset,
        }
    }

    pub fn update(&mut self, x: f64) -> f64 {
        self.h.push(x);
        if self.h.window(self.len).is_none() {
            return NA;
        }
        let n = self.len as f64;
        let (mut sx, mut sy, mut sxy, mut sxx) = (0.0, 0.0, 0.0, 0.0);
        for i in 0..self.len {
            // x = 0 is the oldest value.
            let xi = i as f64;
            let yi = self.h.get(self.len - 1 - i);
            sx += xi;
            sy += yi;
            sxy += xi * yi;
            sxx += xi * xi;
        }
        let slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
        let slope = if slope.is_finite() { slope } else { 0.0 };
        let intercept = (sy - slope * sx) / n;
        intercept + slope * (n - 1.0 - self.offset as f64)
    }
}

/// `ta.stdev` (population, as Pine's default `biased = true`). Deviations
/// within 1e-10 of zero count as zero, as in Pine's reference code.
#[derive(Clone, Debug)]
pub struct Stdev {
    h: Hist,
    mean: Sma,
    len: usize,
    biased: bool,
}

impl Stdev {
    pub fn new(len: usize) -> Stdev {
        Stdev::with(len, true)
    }

    pub fn with(len: usize, biased: bool) -> Stdev {
        let len = len.max(1);
        Stdev {
            h: Hist::new(len),
            mean: Sma::new(len),
            len,
            biased,
        }
    }

    pub fn variance(&mut self, x: f64) -> f64 {
        self.h.push(x);
        let avg = self.mean.update(x);
        let Some(w) = self.h.window(self.len) else { return NA };
        let ss: f64 = w
            .map(|v| {
                let d = v - avg;
                if d.abs() <= 1e-10 { 0.0 } else { d * d }
            })
            .sum();
        let n = if self.biased {
            self.len
        } else {
            self.len.saturating_sub(1).max(1)
        };
        ss / n as f64
    }

    pub fn update(&mut self, x: f64) -> f64 {
        self.variance(x).sqrt()
    }
}

/// `ta.dev`: mean absolute deviation from the SMA.
#[derive(Clone, Debug)]
pub struct Dev {
    h: Hist,
    mean: Sma,
    len: usize,
}

impl Dev {
    pub fn new(len: usize) -> Dev {
        let len = len.max(1);
        Dev {
            h: Hist::new(len),
            mean: Sma::new(len),
            len,
        }
    }

    pub fn update(&mut self, x: f64) -> f64 {
        self.h.push(x);
        let avg = self.mean.update(x);
        match self.h.window(self.len) {
            Some(w) => w.map(|v| (v - avg).abs()).sum::<f64>() / self.len as f64,
            None => NA,
        }
    }
}

/// `ta.highest` / `ta.lowest` and their `…bars` offsets. `na` until `len`
/// values have been seen; `na` values inside the window are skipped.
///
/// A monotonic queue keeps the candidates, so each bar costs O(1)
/// amortised however long the window (Ichimoku's 52, Donchian's 200).
#[derive(Clone, Debug)]
pub struct Extreme {
    /// (bar number, value), best first; later bars win ties.
    q: VecDeque<(usize, f64)>,
    len: usize,
    seen: usize,
    highest: bool,
}

impl Extreme {
    pub fn highest(len: usize) -> Extreme {
        Extreme {
            q: VecDeque::new(),
            len: len.max(1),
            seen: 0,
            highest: true,
        }
    }

    pub fn lowest(len: usize) -> Extreme {
        Extreme {
            highest: false,
            ..Extreme::highest(len)
        }
    }

    /// Pushes `x` and returns `(value, bars back)` (bars back is 0 or more;
    /// `ta.highestbars` is its negative). The newest wins a tie.
    pub fn update_with_offset(&mut self, x: f64) -> (f64, f64) {
        let i = self.seen;
        self.seen += 1;
        if !x.is_nan() {
            while let Some(&(_, v)) = self.q.back() {
                let worse = if self.highest { v <= x } else { v >= x };
                if !worse {
                    break;
                }
                self.q.pop_back();
            }
            self.q.push_back((i, x));
        }
        while let Some(&(j, _)) = self.q.front() {
            if j + self.len <= i {
                self.q.pop_front();
            } else {
                break;
            }
        }
        if self.seen < self.len {
            return (NA, NA);
        }
        match self.q.front() {
            Some(&(j, v)) => (v, (i - j) as f64),
            None => (NA, NA),
        }
    }

    pub fn update(&mut self, x: f64) -> f64 {
        self.update_with_offset(x).0
    }
}

/// `ta.change(x, len)`: `x - x[len]`.
#[derive(Clone, Debug)]
pub struct Change {
    prev: Prev,
}

impl Change {
    pub fn new(len: usize) -> Change {
        Change { prev: Prev::new(len) }
    }

    pub fn update(&mut self, x: f64) -> f64 {
        x - self.prev.update(x)
    }
}

/// `ta.roc(x, len)`: `100 * (x - x[len]) / x[len]`.
#[derive(Clone, Debug)]
pub struct Roc {
    prev: Prev,
}

impl Roc {
    pub fn new(len: usize) -> Roc {
        Roc { prev: Prev::new(len) }
    }

    pub fn update(&mut self, x: f64) -> f64 {
        let p = self.prev.update(x);
        100.0 * div(x - p, p)
    }
}

/// `ta.rsi`, with the zero guards of TradingView's RSI indicator
/// (`down == 0 ? 100 : up == 0 ? 0 : …`).
#[derive(Clone, Debug)]
pub struct Rsi {
    prev: Prev,
    up: Rma,
    down: Rma,
}

impl Rsi {
    pub fn new(len: usize) -> Rsi {
        Rsi {
            prev: Prev::new(1),
            up: rma(len),
            down: rma(len),
        }
    }

    pub fn update(&mut self, x: f64) -> f64 {
        let p = self.prev.update(x);
        let ch = x - p;
        let up = self.up.update(max(ch, 0.0));
        let down = self.down.update(max(-ch, 0.0));
        if down == 0.0 {
            100.0
        } else if up == 0.0 {
            0.0
        } else {
            100.0 - 100.0 / (1.0 + up / down)
        }
    }
}

/// `ta.tr(handle_na)`: true range. On the first bar it's `high - low` when
/// `handle_na`, else `na` (the plain `ta.tr` variable).
#[derive(Clone, Debug)]
pub struct Tr {
    prev_close: f64,
    handle_na: bool,
}

impl Tr {
    pub fn new(handle_na: bool) -> Tr {
        Tr {
            prev_close: NA,
            handle_na,
        }
    }

    pub fn update(&mut self, high: f64, low: f64, close: f64) -> f64 {
        let pc = self.prev_close;
        self.prev_close = close;
        if pc.is_nan() {
            if self.handle_na { high - low } else { NA }
        } else {
            (high - low).max((high - pc).abs()).max((low - pc).abs())
        }
    }
}

/// `ta.atr`: `rma(tr(true), len)`.
#[derive(Clone, Debug)]
pub struct Atr {
    tr: Tr,
    rma: Rma,
}

impl Atr {
    pub fn new(len: usize) -> Atr {
        Atr {
            tr: Tr::new(true),
            rma: rma(len),
        }
    }

    pub fn update(&mut self, high: f64, low: f64, close: f64) -> f64 {
        let tr = self.tr.update(high, low, close);
        self.rma.update(tr)
    }
}

/// `ta.stoch(src, high, low, len)`.
#[derive(Clone, Debug)]
pub struct Stoch {
    hh: Extreme,
    ll: Extreme,
}

impl Stoch {
    pub fn new(len: usize) -> Stoch {
        Stoch {
            hh: Extreme::highest(len),
            ll: Extreme::lowest(len),
        }
    }

    pub fn update(&mut self, src: f64, high: f64, low: f64) -> f64 {
        let hh = self.hh.update(high);
        let ll = self.ll.update(low);
        100.0 * div(src - ll, hh - ll)
    }
}

/// `ta.cci(src, len)`: `(src - sma) / (0.015 * dev)`.
#[derive(Clone, Debug)]
pub struct Cci {
    sma: Sma,
    dev: Dev,
}

impl Cci {
    pub fn new(len: usize) -> Cci {
        Cci {
            sma: Sma::new(len),
            dev: Dev::new(len),
        }
    }

    pub fn update(&mut self, x: f64) -> f64 {
        let ma = self.sma.update(x);
        let dev = self.dev.update(x);
        div(x - ma, 0.015 * dev)
    }
}

/// `ta.cum`: running total, `na` counted as 0.
#[derive(Clone, Debug, Default)]
pub struct Cum {
    total: f64,
}

impl Cum {
    pub fn update(&mut self, x: f64) -> f64 {
        self.total += nz(x, 0.0);
        self.total
    }
}

/// `fixnan`: replaces `na` with the last non-`na` value.
#[derive(Clone, Debug)]
pub struct FixNan {
    last: f64,
}

impl Default for FixNan {
    fn default() -> Self {
        FixNan { last: NA }
    }
}

impl FixNan {
    pub fn update(&mut self, x: f64) -> f64 {
        if !x.is_nan() {
            self.last = x;
        }
        self.last
    }
}

/// `ta.mfi(src, len)`.
#[derive(Clone, Debug)]
pub struct Mfi {
    change: Change,
    upper: Sum,
    lower: Sum,
}

impl Mfi {
    pub fn new(len: usize) -> Mfi {
        Mfi {
            change: Change::new(1),
            upper: Sum::new(len),
            lower: Sum::new(len),
        }
    }

    pub fn update(&mut self, src: f64, volume: f64) -> f64 {
        let ch = self.change.update(src);
        // Comparisons with na are false in Pine, as with NaN here.
        let up = self.upper.update(volume * if ch <= 0.0 { 0.0 } else { src });
        let down = self.lower.update(volume * if ch >= 0.0 { 0.0 } else { src });
        100.0 - 100.0 / (1.0 + up / down)
    }
}

/// `ta.dmi(di_len, adx_smoothing)` → (+DI, −DI, ADX).
#[derive(Clone, Debug)]
pub struct Dmi {
    up: Change,
    down: Change,
    tr: Tr,
    trur: Rma,
    plus_rma: Rma,
    minus_rma: Rma,
    plus_fix: FixNan,
    minus_fix: FixNan,
    adx: Rma,
}

impl Dmi {
    pub fn new(di_len: usize, adx_smoothing: usize) -> Dmi {
        Dmi {
            up: Change::new(1),
            down: Change::new(1),
            tr: Tr::new(false),
            trur: rma(di_len),
            plus_rma: rma(di_len),
            minus_rma: rma(di_len),
            plus_fix: FixNan::default(),
            minus_fix: FixNan::default(),
            adx: rma(adx_smoothing),
        }
    }

    pub fn update(&mut self, high: f64, low: f64, close: f64) -> (f64, f64, f64) {
        let up = self.up.update(high);
        let down = -self.down.update(low);
        let plus_dm = if up.is_nan() {
            NA
        } else if up > down && up > 0.0 {
            up
        } else {
            0.0
        };
        let minus_dm = if down.is_nan() {
            NA
        } else if down > up && down > 0.0 {
            down
        } else {
            0.0
        };
        let tr = self.tr.update(high, low, close);
        let trur = self.trur.update(tr);
        let plus = self.plus_fix.update(100.0 * self.plus_rma.update(plus_dm) / trur);
        let minus = self.minus_fix.update(100.0 * self.minus_rma.update(minus_dm) / trur);
        let sum = plus + minus;
        let adx = 100.0
            * self
                .adx
                .update((plus - minus).abs() / if sum == 0.0 { 1.0 } else { sum });
        (plus, minus, adx)
    }
}

/// `ta.sar(start, inc, max)`: Parabolic SAR, following Pine's reference
/// implementation bar for bar.
#[derive(Clone, Debug)]
pub struct Sar {
    start: f64,
    inc: f64,
    max: f64,
    result: f64,
    max_min: f64,
    accel: f64,
    is_below: bool,
    bar: usize,
    high: Hist,
    low: Hist,
    close: Hist,
}

impl Sar {
    pub fn new(start: f64, inc: f64, max: f64) -> Sar {
        Sar {
            start,
            inc,
            max,
            result: NA,
            max_min: NA,
            accel: NA,
            is_below: false,
            bar: 0,
            high: Hist::new(3),
            low: Hist::new(3),
            close: Hist::new(2),
        }
    }

    pub fn update(&mut self, high: f64, low: f64, close: f64) -> f64 {
        self.high.push(high);
        self.low.push(low);
        self.close.push(close);
        let bar = self.bar;
        self.bar += 1;
        let mut first_trend_bar = false;
        if bar == 1 {
            if close > self.close.get(1) {
                self.is_below = true;
                self.max_min = high;
                self.result = self.low.get(1);
            } else {
                self.is_below = false;
                self.max_min = low;
                self.result = self.high.get(1);
            }
            first_trend_bar = true;
            self.accel = self.start;
        }
        self.result += self.accel * (self.max_min - self.result);
        if self.is_below {
            if self.result > low {
                first_trend_bar = true;
                self.is_below = false;
                self.result = high.max(self.max_min);
                self.max_min = low;
                self.accel = self.start;
            }
        } else if self.result < high {
            first_trend_bar = true;
            self.is_below = true;
            self.result = low.min(self.max_min);
            self.max_min = high;
            self.accel = self.start;
        }
        if !first_trend_bar {
            if self.is_below {
                if high > self.max_min {
                    self.max_min = high;
                    self.accel = (self.accel + self.inc).min(self.max);
                }
            } else if low < self.max_min {
                self.max_min = low;
                self.accel = (self.accel + self.inc).min(self.max);
            }
        }
        if self.is_below {
            self.result = self.result.min(self.low.get(1));
            if bar > 1 {
                self.result = self.result.min(self.low.get(2));
            }
        } else {
            self.result = self.result.max(self.high.get(1));
            if bar > 1 {
                self.result = self.result.max(self.high.get(2));
            }
        }
        self.result
    }
}

/// `ta.supertrend(factor, atr_period)` → (supertrend, direction), direction
/// -1 for an uptrend and 1 for a downtrend, as in Pine.
#[derive(Clone, Debug)]
pub struct Supertrend {
    factor: f64,
    atr: Atr,
    prev_atr: f64,
    prev_upper: f64,
    prev_lower: f64,
    prev_close: f64,
    prev_st: f64,
}

impl Supertrend {
    pub fn new(factor: f64, atr_period: usize) -> Supertrend {
        Supertrend {
            factor,
            atr: Atr::new(atr_period),
            prev_atr: NA,
            prev_upper: NA,
            prev_lower: NA,
            prev_close: NA,
            prev_st: NA,
        }
    }

    pub fn update(&mut self, high: f64, low: f64, close: f64) -> (f64, f64) {
        let src = (high + low) / 2.0;
        let atr = self.atr.update(high, low, close);
        let mut upper = src + self.factor * atr;
        let mut lower = src - self.factor * atr;
        let prev_lower = nz(self.prev_lower, 0.0);
        let prev_upper = nz(self.prev_upper, 0.0);
        let pc = self.prev_close;
        lower = if lower > prev_lower || pc < prev_lower {
            lower
        } else {
            prev_lower
        };
        upper = if upper < prev_upper || pc > prev_upper {
            upper
        } else {
            prev_upper
        };
        let direction = if self.prev_atr.is_nan() {
            1.0
        } else if self.prev_st == prev_upper {
            if close > upper { -1.0 } else { 1.0 }
        } else if close < lower {
            1.0
        } else {
            -1.0
        };
        let st = if direction == -1.0 { lower } else { upper };
        self.prev_atr = atr;
        self.prev_upper = upper;
        self.prev_lower = lower;
        self.prev_close = close;
        self.prev_st = st;
        (st, direction)
    }
}

/// Session VWAP with standard-deviation bands (`ta.vwap(src, anchor, 1)`).
#[derive(Clone, Debug, Default)]
pub struct Vwap {
    sum_pv: f64,
    sum_v: f64,
    sum_p2v: f64,
}

impl Vwap {
    /// Returns (vwap, stdev). `reset` starts a new anchor period.
    pub fn update(&mut self, src: f64, volume: f64, reset: bool) -> (f64, f64) {
        if reset {
            *self = Vwap::default();
        }
        if !src.is_nan() && !volume.is_nan() {
            self.sum_pv += src * volume;
            self.sum_v += volume;
            self.sum_p2v += src * src * volume;
        }
        let vwap = div(self.sum_pv, self.sum_v);
        let var = div(self.sum_p2v, self.sum_v) - vwap * vwap;
        (vwap, var.max(0.0).sqrt())
    }
}

/// `ta.crossover(a, b)` / `ta.crossunder(a, b)`.
#[derive(Clone, Debug)]
pub struct Cross {
    pa: f64,
    pb: f64,
}

impl Default for Cross {
    fn default() -> Self {
        Cross { pa: NA, pb: NA }
    }
}

impl Cross {
    /// Returns (crossed over, crossed under).
    pub fn update(&mut self, a: f64, b: f64) -> (bool, bool) {
        let over = a > b && self.pa <= self.pb;
        let under = a < b && self.pa >= self.pb;
        self.pa = a;
        self.pb = b;
        (over, under)
    }
}

/// The moving averages TradingView's indicators let you choose between.
#[derive(Clone, Debug)]
pub enum Ma {
    Sma(Sma),
    Ema(Ema),
    Rma(Rma),
    Wma(Wma),
    Vwma(Vwma),
}

impl Ma {
    /// From a menu label: "SMA", "EMA", "SMMA (RMA)", "WMA", "VWMA".
    pub fn by_name(name: &str, len: usize) -> Ma {
        match name {
            "EMA" => Ma::Ema(ema(len)),
            "SMMA (RMA)" | "RMA" => Ma::Rma(rma(len)),
            "WMA" => Ma::Wma(Wma::new(len)),
            "VWMA" => Ma::Vwma(Vwma::new(len)),
            _ => Ma::Sma(Sma::new(len)),
        }
    }

    pub fn update(&mut self, x: f64, volume: f64) -> f64 {
        match self {
            Ma::Sma(m) => m.update(x),
            Ma::Ema(m) | Ma::Rma(m) => m.update(x),
            Ma::Wma(m) => m.update(x),
            Ma::Vwma(m) => m.update(x, volume),
        }
    }
}

pub const MA_TYPES: &[&str] = &["SMA", "EMA", "SMMA (RMA)", "WMA", "VWMA"];

#[cfg(test)]
mod tests {
    //! Each primitive is checked against a plain, whole-array translation of
    //! Pine's reference code (written separately from the streaming version,
    //! so a shared mistake is unlikely), plus hand-worked values.

    use super::*;

    fn close() -> Vec<f64> {
        // A deterministic, wiggly series with a flat stretch.
        (0..300)
            .map(|i| {
                let x = i as f64;
                if (120..130).contains(&i) {
                    100.0
                } else {
                    100.0 + (x * 0.37).sin() * 5.0 + (x * 0.05).cos() * 12.0 + x * 0.03
                }
            })
            .collect()
    }

    fn approx(a: &[f64], b: &[f64]) {
        assert_eq!(a.len(), b.len());
        for (i, (x, y)) in a.iter().zip(b).enumerate() {
            if x.is_nan() || y.is_nan() {
                assert!(x.is_nan() && y.is_nan(), "bar {i}: {x} vs {y}");
            } else {
                let tol = 1e-9 * x.abs().max(y.abs()).max(1.0);
                assert!((x - y).abs() <= tol, "bar {i}: {x} vs {y}");
            }
        }
    }

    fn run(xs: &[f64], mut f: impl FnMut(f64) -> f64) -> Vec<f64> {
        xs.iter().map(|&x| f(x)).collect()
    }

    // ---- whole-array references, following Pine's documented code

    fn ref_sma(x: &[f64], n: usize) -> Vec<f64> {
        (0..x.len())
            .map(|i| {
                if i + 1 < n {
                    NA
                } else {
                    x[i + 1 - n..=i].iter().sum::<f64>() / n as f64
                }
            })
            .collect()
    }

    fn ref_smooth(x: &[f64], n: usize, alpha: f64) -> Vec<f64> {
        let seed = ref_sma(x, n);
        let mut out = vec![NA; x.len()];
        for i in 0..x.len() {
            let prev = if i == 0 { NA } else { out[i - 1] };
            out[i] = if prev.is_nan() {
                seed[i]
            } else {
                alpha * x[i] + (1.0 - alpha) * prev
            };
        }
        out
    }

    #[test]
    fn sma_ema_rma_match_reference() {
        let x = close();
        approx(
            &run(&x, {
                let mut m = Sma::new(14);
                move |v| m.update(v)
            }),
            &ref_sma(&x, 14),
        );
        approx(
            &run(&x, {
                let mut m = ema(9);
                move |v| m.update(v)
            }),
            &ref_smooth(&x, 9, 0.2),
        );
        approx(
            &run(&x, {
                let mut m = rma(14);
                move |v| m.update(v)
            }),
            &ref_smooth(&x, 14, 1.0 / 14.0),
        );
    }

    #[test]
    fn sma_hand_worked() {
        let mut m = Sma::new(3);
        let out: Vec<f64> = [1.0, 2.0, 3.0, 4.0].iter().map(|&v| m.update(v)).collect();
        assert!(out[0].is_nan() && out[1].is_nan());
        assert_eq!(&out[2..], &[2.0, 3.0]);
    }

    #[test]
    fn ema_seeds_with_sma() {
        let mut m = ema(3);
        let out: Vec<f64> = [2.0, 4.0, 6.0, 8.0].iter().map(|&v| m.update(v)).collect();
        assert!(out[1].is_nan());
        assert_eq!(out[2], 4.0);
        assert_eq!(out[3], 0.5 * 8.0 + 0.5 * 4.0);
    }

    #[test]
    fn wma_hand_worked() {
        let mut m = Wma::new(3);
        let out: Vec<f64> = [1.0, 2.0, 3.0].iter().map(|&v| m.update(v)).collect();
        // (3*3 + 2*2 + 1*1) / 6
        assert_eq!(out[2], 14.0 / 6.0);
    }

    #[test]
    fn rsi_matches_reference_and_bounds() {
        let x = close();
        let out = run(&x, {
            let mut m = Rsi::new(14);
            move |v| m.update(v)
        });
        // Reference: rma of gains and losses from bar 1 on.
        let u: Vec<f64> = (0..x.len())
            .map(|i| if i == 0 { NA } else { (x[i] - x[i - 1]).max(0.0) })
            .collect();
        let d: Vec<f64> = (0..x.len())
            .map(|i| if i == 0 { NA } else { (x[i - 1] - x[i]).max(0.0) })
            .collect();
        let (ru, rd) = (ref_smooth_na(&u, 14), ref_smooth_na(&d, 14));
        let expect: Vec<f64> = ru
            .iter()
            .zip(&rd)
            .map(|(&a, &b)| {
                if b == 0.0 {
                    100.0
                } else if a == 0.0 {
                    0.0
                } else {
                    100.0 - 100.0 / (1.0 + a / b)
                }
            })
            .collect();
        approx(&out, &expect);
        assert!(out[13].is_nan() && !out[14].is_nan());
        assert!(out.iter().filter(|v| !v.is_nan()).all(|v| (0.0..=100.0).contains(v)));
    }

    /// Like `ref_smooth`, for a series whose first value is na.
    fn ref_smooth_na(x: &[f64], n: usize) -> Vec<f64> {
        let seed: Vec<f64> = (0..x.len())
            .map(|i| {
                if i + 1 < n {
                    return NA;
                }
                let w = &x[i + 1 - n..=i];
                if w.iter().any(|v| v.is_nan()) {
                    NA
                } else {
                    w.iter().sum::<f64>() / n as f64
                }
            })
            .collect();
        let alpha = 1.0 / n as f64;
        let mut out = vec![NA; x.len()];
        for i in 0..x.len() {
            let prev = if i == 0 { NA } else { out[i - 1] };
            out[i] = if prev.is_nan() {
                seed[i]
            } else {
                alpha * x[i] + (1.0 - alpha) * prev
            };
        }
        out
    }

    #[test]
    fn stdev_and_dev() {
        let mut s = Stdev::new(4);
        let mut d = Dev::new(4);
        let xs = [2.0, 4.0, 4.0, 6.0];
        let mut last = (NA, NA);
        for x in xs {
            last = (s.update(x), d.update(x));
        }
        // mean 4; squared deviations 4,0,0,4 → var 2 (population)
        assert!((last.0 - 2f64.sqrt()).abs() < 1e-12);
        // absolute deviations 2,0,0,2 → 1
        assert_eq!(last.1, 1.0);
        // A flat window is exactly zero, not a tiny float.
        let mut s = Stdev::new(3);
        for _ in 0..3 {
            s.update(0.1 + 0.2);
        }
        assert_eq!(s.update(0.1 + 0.2), 0.0);
    }

    #[test]
    fn highest_lowest_and_offsets() {
        let mut h = Extreme::highest(3);
        let mut l = Extreme::lowest(3);
        let xs = [5.0, 1.0, 9.0, 3.0, 2.0];
        let hs: Vec<(f64, f64)> = xs.iter().map(|&x| h.update_with_offset(x)).collect();
        let ls: Vec<f64> = xs.iter().map(|&x| l.update(x)).collect();
        assert!(hs[1].0.is_nan());
        assert_eq!(hs[2], (9.0, 0.0));
        assert_eq!(hs[4], (9.0, 2.0));
        assert_eq!(ls[2..], [1.0, 1.0, 2.0]);
    }

    #[test]
    fn extreme_matches_a_plain_window_scan() {
        // Includes ties, na values and a flat stretch.
        let xs: Vec<f64> = (0..500)
            .map(|i| {
                if i % 37 == 0 {
                    NA
                } else if (200..220).contains(&i) {
                    5.0
                } else {
                    ((i * 7919) % 101) as f64
                }
            })
            .collect();
        for len in [1, 3, 14, 52] {
            let mut hi = Extreme::highest(len);
            let mut lo = Extreme::lowest(len);
            for (i, &x) in xs.iter().enumerate() {
                let got_h = hi.update_with_offset(x);
                let got_l = lo.update_with_offset(x);
                if i + 1 < len {
                    assert!(got_h.0.is_nan() && got_l.0.is_nan());
                    continue;
                }
                let scan = |better: fn(f64, f64) -> bool| {
                    let mut best = (NA, NA);
                    for k in 0..len {
                        let v = xs[i - k];
                        if !v.is_nan() && (best.0.is_nan() || better(v, best.0)) {
                            best = (v, k as f64);
                        }
                    }
                    best
                };
                let want_h = scan(|a, b| a > b);
                let want_l = scan(|a, b| a < b);
                let same = |a: (f64, f64), b: (f64, f64)| (a.0.is_nan() && b.0.is_nan()) || a == b;
                assert!(same(got_h, want_h), "len {len} bar {i}: {got_h:?} vs {want_h:?}");
                assert!(same(got_l, want_l), "len {len} bar {i}: {got_l:?} vs {want_l:?}");
            }
        }
    }

    #[test]
    fn linreg_on_a_line_is_the_line() {
        let mut lr = Linreg::new(5, 0);
        let mut last = NA;
        for i in 0..10 {
            last = lr.update(3.0 + 2.0 * i as f64);
        }
        assert!((last - (3.0 + 2.0 * 9.0)).abs() < 1e-9);
    }

    #[test]
    fn true_range_and_atr() {
        let mut tr = Tr::new(false);
        assert!(tr.update(10.0, 8.0, 9.0).is_nan());
        assert_eq!(tr.update(12.0, 11.0, 11.5), 3.0);
        let mut tr = Tr::new(true);
        assert_eq!(tr.update(10.0, 8.0, 9.0), 2.0);
        let mut atr = Atr::new(2);
        atr.update(10.0, 8.0, 9.0);
        // tr = 2 then 3 → sma seed 2.5
        assert_eq!(atr.update(12.0, 11.0, 11.5), 2.5);
    }

    #[test]
    fn crossover() {
        let mut c = Cross::default();
        assert_eq!(c.update(1.0, 2.0), (false, false));
        assert_eq!(c.update(3.0, 2.0), (true, false));
        assert_eq!(c.update(1.0, 2.0), (false, true));
    }

    #[test]
    fn sar_follows_trend() {
        let mut sar = Sar::new(0.02, 0.02, 0.2);
        let mut out = vec![];
        for i in 0..30 {
            let x = 100.0 + i as f64;
            out.push((sar.update(x + 1.0, x - 1.0, x), x));
        }
        assert!(out[0].0.is_nan());
        // In a steady uptrend the SAR stays below the lows.
        assert!(out[5..].iter().all(|(s, x)| *s < x - 1.0));
    }

    #[test]
    fn vwap_resets_each_session() {
        let mut v = Vwap::default();
        assert_eq!(v.update(10.0, 1.0, true).0, 10.0);
        assert_eq!(v.update(20.0, 3.0, false).0, 17.5);
        assert_eq!(v.update(30.0, 1.0, true).0, 30.0);
    }
}

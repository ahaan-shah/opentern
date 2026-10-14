//! TradingView built-ins, second wave: ported from their Pine source line by
//! line onto the `ta` primitives, like the first wave. Each one has a Pine
//! twin in `crates/pine/tests/corpus/` that must agree bar for bar.

use super::*;
use crate::color::tv;
use crate::ta::{self, Hist};

#[allow(clippy::too_many_arguments)]
fn meta(
    id: &'static str,
    name: &'static str,
    short: &'static str,
    category: Category,
    placement: Placement,
    inputs: Vec<InputSpec>,
    plots: Vec<PlotSpec>,
    build: fn(&Inputs) -> Box<dyn Study>,
) -> Meta {
    Meta {
        id,
        name,
        short,
        category,
        placement,
        format: if placement == Placement::Overlay {
            Format::Price
        } else {
            Format::Fixed(2)
        },
        inputs,
        plots,
        hlines: vec![],
        fills: vec![],
        build,
    }
}

// ------------------------------------------------------------ Chaikin Oscillator

#[derive(Clone)]
struct ChaikinOsc {
    ad: ta::Cum,
    fast: ta::Ema,
    slow: ta::Ema,
}

impl Study for ChaikinOsc {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let b = c.bar;
        let mfv = if (b.close == b.high && b.close == b.low) || b.high == b.low {
            0.0
        } else {
            (2.0 * b.close - b.low - b.high) / (b.high - b.low) * b.volume
        };
        let ad = self.ad.update(mfv);
        o.values[0] = self.fast.update(ad) - self.slow.update(ad);
    }
}

// ------------------------------------------------------------ Chande Kroll Stop

#[derive(Clone)]
struct ChandeKroll {
    hh: ta::Extreme,
    ll: ta::Extreme,
    atr: ta::Atr,
    x: f64,
    stop_short: ta::Extreme,
    stop_long: ta::Extreme,
}

impl Study for ChandeKroll {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let b = c.bar;
        let atr = self.atr.update(b.high, b.low, b.close);
        let first_high = self.hh.update(b.high) - self.x * atr;
        let first_low = self.ll.update(b.low) + self.x * atr;
        o.values[1] = self.stop_short.update(first_high);
        o.values[0] = self.stop_long.update(first_low);
    }
}

// ------------------------------------------------------------ Chop Zone

#[derive(Clone)]
struct ChopZone {
    hh: ta::Extreme,
    ll: ta::Extreme,
    ema: ta::Ema,
    prev: f64,
}

impl Study for ChopZone {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let b = c.bar;
        let avg = (b.high + b.low + b.close) / 3.0;
        let hh = self.hh.update(b.high);
        let ll = self.ll.update(b.low);
        let span = ta::div(25.0, hh - ll) * ll;
        let ema = self.ema.update(b.close);
        let y2 = ta::div(self.prev - ema, avg) * span;
        self.prev = ema;
        let cl = (1.0 + y2 * y2).sqrt();
        let angle1 = (180.0 * (1.0 / cl).acos() / std::f64::consts::PI + 0.5).floor();
        let angle = if y2 > 0.0 { -angle1 } else { angle1 };
        o.values[0] = 1.0;
        o.colors[0] = if angle.is_nan() {
            8
        } else if angle >= 5.0 {
            0
        } else if angle >= 3.57 {
            1
        } else if angle >= 2.14 {
            2
        } else if angle >= 0.71 {
            3
        } else if angle <= -5.0 {
            4
        } else if angle <= -3.57 {
            5
        } else if angle <= -2.14 {
            6
        } else if angle <= -0.71 {
            7
        } else {
            8
        };
    }
}

// ------------------------------------------------------------ Connors RSI

#[derive(Clone)]
struct ConnorsRsi {
    rsi: ta::Rsi,
    prev: f64,
    streak: f64,
    streak_rsi: ta::Rsi,
    roc_prev: ta::Prev,
    pr: Hist,
    lenroc: usize,
}

impl Study for ConnorsRsi {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let x = c.bar.close;
        let r = self.rsi.update(x);
        // Up/down streak: +n for n rising closes in a row, −n falling.
        self.streak = if x == self.prev {
            0.0
        } else if x > self.prev {
            if self.streak <= 0.0 { 1.0 } else { self.streak + 1.0 }
        } else if x < self.prev {
            if self.streak >= 0.0 { -1.0 } else { self.streak - 1.0 }
        } else {
            // First bar: no previous close.
            if self.streak >= 0.0 { -1.0 } else { self.streak - 1.0 }
        };
        let p = self.prev;
        self.prev = x;
        let ud = self.streak_rsi.update(self.streak);
        let roc_p = self.roc_prev.update(x);
        let roc = 100.0 * ta::div(x - roc_p, roc_p);
        self.pr.push(roc);
        let pr = if self.pr.seen() > self.lenroc && !roc.is_nan() {
            let le = (1..=self.lenroc).filter(|&i| self.pr.get(i) <= roc).count();
            100.0 * le as f64 / self.lenroc as f64
        } else {
            f64::NAN
        };
        let _ = p;
        o.values[0] = (r + ud + pr) / 3.0;
    }
}

// ------------------------------------------------------------ Klinger Oscillator

#[derive(Clone)]
struct Klinger {
    prev: f64,
    fast: ta::Ema,
    slow: ta::Ema,
    signal: ta::Ema,
}

impl Study for Klinger {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let b = c.bar;
        let hlc3 = (b.high + b.low + b.close) / 3.0;
        let ch = hlc3 - self.prev;
        self.prev = hlc3;
        // `ta.change(hlc3) >= 0` is false on the first bar (na).
        let sv = if ch >= 0.0 { b.volume } else { -b.volume };
        let kvo = self.fast.update(sv) - self.slow.update(sv);
        o.values[0] = kvo;
        o.values[1] = self.signal.update(kvo);
    }
}

// ------------------------------------------------------------ MA Ribbon

#[derive(Clone)]
struct Ribbon {
    src: Source,
    mas: Vec<ta::Ma>,
}

impl Study for Ribbon {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let x = c.src(self.src);
        for (k, m) in self.mas.iter_mut().enumerate() {
            o.values[k] = m.update(x, c.bar.volume);
        }
    }
}

// ------------------------------------------------------------ Median

/// Median of the last `n` values, ATR bands around it, and the median's
/// EMA (which colours it).
#[derive(Clone)]
struct Median {
    src: Source,
    win: Hist,
    n: usize,
    atr: ta::Atr,
    mult: f64,
    ema: ta::Smooth,
}

/// `ta.percentile_nearest_rank(x, n, 50)` over a window (`na`s skipped).
fn median_of(win: &Hist, n: usize) -> f64 {
    let Some(w) = win.window(n) else { return f64::NAN };
    let mut v: Vec<f64> = w.filter(|x| !x.is_nan()).collect();
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let rank = (0.5 * v.len() as f64).ceil() as usize;
    v[rank.saturating_sub(1).min(v.len() - 1)]
}

impl Study for Median {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        self.win.push(c.src(self.src));
        let median = median_of(&self.win, self.n);
        let atr = self.mult * self.atr.update(c.bar.high, c.bar.low, c.bar.close);
        let ema = self.ema.update(median);
        o.values[0] = median + atr;
        o.values[1] = median;
        o.values[2] = median - atr;
        o.values[3] = ema;
        let up = if median > ema { 0 } else { 1 };
        o.colors[0] = up;
        o.colors[1] = up;
        o.colors[2] = up;
        o.fills[0] = up;
    }
}

// ------------------------------------------------------------ Majority Rule

/// The share of the last `n` bars whose source rose, in percent.
#[derive(Clone)]
struct MajorityRule {
    src: Source,
    prev: f64,
    sum: ta::Sum,
    n: usize,
}

impl Study for MajorityRule {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let x = c.src(self.src);
        let up = if x > self.prev { 1.0 } else { 0.0 };
        self.prev = x;
        o.values[0] = 100.0 * self.sum.update(up) / self.n as f64;
    }
}

// ------------------------------------------------------------ Net Volume

#[derive(Clone)]
struct NetVolume {
    prev: f64,
}

impl Study for NetVolume {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let ch = c.bar.close - self.prev;
        self.prev = c.bar.close;
        o.values[0] = if ch > 0.0 {
            c.bar.volume
        } else if ch < 0.0 {
            -c.bar.volume
        } else {
            0.0
        };
    }
}

// ------------------------------------------------------------ Pivot Points High Low

#[derive(Clone)]
struct PivotsHl {
    left: usize,
    right: usize,
    highs: Hist,
    lows: Hist,
}

/// `ta.pivothigh` (or low): the value `right` bars ago if it beats the
/// `left` bars before it strictly and the `right` bars after it or ties.
fn pivot(h: &Hist, left: usize, right: usize, high: bool) -> f64 {
    let n = left + right + 1;
    if h.seen() < n {
        return f64::NAN;
    }
    let c = h.get(right);
    if c.is_nan() {
        return f64::NAN;
    }
    let beats = |x: f64, strict: bool| {
        x.is_nan()
            || match (high, strict) {
                (true, true) => c > x,
                (true, false) => c >= x,
                (false, true) => c < x,
                (false, false) => c <= x,
            }
    };
    let ok = (right + 1..n).all(|i| beats(h.get(i), true)) && (0..right).all(|i| beats(h.get(i), false));
    if ok { c } else { f64::NAN }
}

impl Study for PivotsHl {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        self.highs.push(c.bar.high);
        self.lows.push(c.bar.low);
        o.values[0] = pivot(&self.highs, self.left, self.right, true);
        o.values[1] = pivot(&self.lows, self.left, self.right, false);
    }
}

// ------------------------------------------------------------ Rank Correlation Index

/// Spearman's rank correlation of the last `n` values against time, ×100
/// (newest bar = rank 1 in time; ties get the average rank).
fn rci(h: &Hist, n: usize) -> f64 {
    let Some(w) = h.window(n) else { return f64::NAN };
    let v: Vec<f64> = w.collect();
    if v.iter().any(|x| x.is_nan()) || n < 2 {
        return f64::NAN;
    }
    let mut sum = 0.0;
    for (i, &x) in v.iter().enumerate() {
        // Price rank: 1 for the highest; ties averaged.
        let higher = v.iter().filter(|&&y| y > x).count() as f64;
        let equal = v.iter().filter(|&&y| y == x).count() as f64;
        let rank = higher + (equal + 1.0) / 2.0;
        let time_rank = (i + 1) as f64;
        sum += (time_rank - rank).powi(2);
    }
    let n = n as f64;
    (1.0 - 6.0 * sum / (n * (n * n - 1.0))) * 100.0
}

#[derive(Clone)]
struct Rci {
    src: Source,
    h: Hist,
    lens: Vec<usize>,
}

impl Study for Rci {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        self.h.push(c.src(self.src));
        for (k, n) in self.lens.iter().enumerate() {
            o.values[k] = rci(&self.h, *n);
        }
    }
}

// ------------------------------------------------------------ Relative Vigor Index

#[derive(Clone)]
struct Rvgi {
    num: Hist,
    den: Hist,
    s_num: ta::Sum,
    s_den: ta::Sum,
    rvi: Hist,
}

fn swma(h: &Hist) -> f64 {
    match h.window(4) {
        Some(w) => {
            let x: Vec<f64> = w.collect();
            x[3] / 6.0 + x[2] * 2.0 / 6.0 + x[1] * 2.0 / 6.0 + x[0] / 6.0
        }
        None => f64::NAN,
    }
}

impl Study for Rvgi {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let b = c.bar;
        self.num.push(b.close - b.open);
        self.den.push(b.high - b.low);
        let rvi = ta::div(self.s_num.update(swma(&self.num)), self.s_den.update(swma(&self.den)));
        self.rvi.push(rvi);
        o.values[0] = rvi;
        o.values[1] = swma(&self.rvi);
    }
}

// ------------------------------------------------------------ Relative Volatility Index

#[derive(Clone)]
struct RelVol {
    src: Source,
    sd: ta::Stdev,
    prev: f64,
    up: ta::Ema,
    down: ta::Ema,
}

impl Study for RelVol {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let x = c.src(self.src);
        let sd = self.sd.update(x);
        let ch = x - self.prev;
        self.prev = x;
        let u = self.up.update(if ch <= 0.0 { 0.0 } else { sd });
        let d = self.down.update(if ch > 0.0 { 0.0 } else { sd });
        o.values[0] = ta::div(u, u + d) * 100.0;
    }
}

// ------------------------------------------------------------ SMI Ergodic

#[derive(Clone)]
struct Ergodic {
    prev: ta::Prev,
    a1: ta::Ema,
    a2: ta::Ema,
    b1: ta::Ema,
    b2: ta::Ema,
    sig: ta::Ema,
    osc: bool,
}

impl Study for Ergodic {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let x = c.bar.close;
        let pc = x - self.prev.update(x);
        let erg = ta::div(
            self.a2.update(self.a1.update(pc)),
            self.b2.update(self.b1.update(pc.abs())),
        );
        let sig = self.sig.update(erg);
        if self.osc {
            o.values[0] = erg - sig;
        } else {
            o.values[0] = erg;
            o.values[1] = sig;
        }
    }
}

// ------------------------------------------------------------ Standard Error (+ Bands)

/// Least-squares line over the window (x = 0 for the oldest): the line's
/// value at the newest bar and the standard error of the fit.
fn regression(h: &Hist, n: usize) -> (f64, f64) {
    let Some(w) = h.window(n) else {
        return (f64::NAN, f64::NAN);
    };
    let mut y: Vec<f64> = w.collect();
    y.reverse();
    if y.iter().any(|v| v.is_nan()) || n < 3 {
        return (f64::NAN, f64::NAN);
    }
    let nf = n as f64;
    let mx = (nf - 1.0) / 2.0;
    let my = y.iter().sum::<f64>() / nf;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (i, v) in y.iter().enumerate() {
        let dx = i as f64 - mx;
        sxy += dx * (v - my);
        sxx += dx * dx;
    }
    let slope = sxy / sxx;
    let icpt = my - slope * mx;
    let sse: f64 = y
        .iter()
        .enumerate()
        .map(|(i, v)| (v - (icpt + slope * i as f64)).powi(2))
        .sum();
    (icpt + slope * (nf - 1.0), (sse / (nf - 2.0)).sqrt())
}

#[derive(Clone)]
struct StdErr {
    src: Source,
    h: Hist,
    n: usize,
    bands: Option<(f64, ta::Sma, ta::Sma, ta::Sma)>,
}

impl Study for StdErr {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        self.h.push(c.src(self.src));
        let (line, se) = regression(&self.h, self.n);
        match &mut self.bands {
            None => o.values[0] = se,
            Some((mult, m, u, l)) => {
                o.values[0] = m.update(line);
                o.values[1] = u.update(line + *mult * se);
                o.values[2] = l.update(line - *mult * se);
            }
        }
    }
}

// ------------------------------------------------------------ Stochastic Momentum Index

#[derive(Clone)]
struct Smi {
    hh: ta::Extreme,
    ll: ta::Extreme,
    r1: ta::Ema,
    r2: ta::Ema,
    d1: ta::Ema,
    d2: ta::Ema,
    ema: ta::Ema,
}

impl Study for Smi {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let b = c.bar;
        let hh = self.hh.update(b.high);
        let ll = self.ll.update(b.low);
        let diff = hh - ll;
        let rdiff = b.close - (hh + ll) / 2.0;
        let avgrel = self.r2.update(self.r1.update(rdiff));
        let avgdiff = self.d2.update(self.d1.update(diff));
        let smi = if avgdiff != 0.0 {
            avgrel / (avgdiff / 2.0) * 100.0
        } else {
            0.0
        };
        // An na avgdiff also lands here in Pine (na != 0 is true → na).
        let smi = if avgdiff.is_nan() { f64::NAN } else { smi };
        o.values[0] = smi;
        o.values[1] = self.ema.update(smi);
    }
}

// ------------------------------------------------------------ TWAP

#[derive(Clone)]
struct Twap {
    src: Source,
    sum: f64,
    n: f64,
}

impl Study for Twap {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        if c.new_session || c.index == 0 {
            self.sum = 0.0;
            self.n = 0.0;
        }
        self.sum += c.src(self.src);
        self.n += 1.0;
        o.values[0] = self.sum / self.n;
    }
}

// ------------------------------------------------------------ Trend Strength Index

#[derive(Clone)]
struct TrendStrength {
    n: usize,
    y: Hist,
    x: Hist,
}

impl Study for TrendStrength {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        self.y.push(c.bar.close);
        self.x.push(c.index as f64);
        let (Some(a), Some(b)) = (self.y.window(self.n), self.x.window(self.n)) else {
            return;
        };
        let a: Vec<f64> = a.collect();
        let b: Vec<f64> = b.collect();
        let nf = self.n as f64;
        let (ma, mb) = (a.iter().sum::<f64>() / nf, b.iter().sum::<f64>() / nf);
        let cov: f64 = a.iter().zip(&b).map(|(p, q)| (p - ma) * (q - mb)).sum();
        let va: f64 = a.iter().map(|p| (p - ma).powi(2)).sum();
        let vb: f64 = b.iter().map(|q| (q - mb).powi(2)).sum();
        o.values[0] = ta::div(cov, (va * vb).sqrt());
    }
}

// ------------------------------------------------------------ Volatility Stop

#[derive(Clone)]
struct VolStop {
    src: Source,
    atr: ta::Atr,
    tr: ta::Tr,
    factor: f64,
    max: f64,
    min: f64,
    uptrend: bool,
    prev_up: Option<bool>,
    stop: f64,
    started: bool,
}

impl Study for VolStop {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let b = c.bar;
        let src = c.src(self.src);
        if !self.started {
            self.started = true;
            self.max = src;
            self.min = src;
            self.uptrend = true;
            self.stop = f64::NAN;
        }
        let tr = self.tr.update(b.high, b.low, b.close);
        let atr_m = ta::nz(self.atr.update(b.high, b.low, b.close) * self.factor, tr);
        self.max = ta::max(self.max, src);
        self.min = ta::min(self.min, src);
        let cand = if self.uptrend {
            ta::max(self.stop, self.max - atr_m)
        } else {
            ta::min(self.stop, self.min + atr_m)
        };
        self.stop = ta::nz(cand, src);
        self.uptrend = src - self.stop >= 0.0;
        if Some(self.uptrend) != Some(self.prev_up.unwrap_or(true)) {
            self.max = src;
            self.min = src;
            self.stop = if self.uptrend {
                self.max - atr_m
            } else {
                self.min + atr_m
            };
        }
        self.prev_up = Some(self.uptrend);
        o.values[0] = self.stop;
        o.colors[0] = if self.uptrend { 0 } else { 1 };
    }
}

// ------------------------------------------------------------ Williams Fractals

#[derive(Clone)]
struct Fractals {
    n: usize,
    highs: Hist,
    lows: Hist,
}

impl Study for Fractals {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        self.highs.push(c.bar.high);
        self.lows.push(c.bar.low);
        let n = self.n;
        if self.highs.seen() < 2 * n + 1 {
            return;
        }
        let (h, l) = (self.highs.get(n), self.lows.get(n));
        let up = (1..=n).all(|i| h > self.highs.get(n - i) && h > self.highs.get(n + i));
        let down = (1..=n).all(|i| l < self.lows.get(n - i) && l < self.lows.get(n + i));
        o.values[0] = if down { 1.0 } else { f64::NAN };
        o.values[1] = if up { 1.0 } else { f64::NAN };
    }
}

// ------------------------------------------------------------ Woodies CCI

#[derive(Clone)]
struct Woodies {
    cci: ta::Cci,
    turbo: ta::Cci,
}

impl Study for Woodies {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let x = (c.bar.high + c.bar.low + c.bar.close) / 3.0;
        let cci = self.cci.update(x);
        o.values[0] = cci;
        o.colors[0] = if cci >= 0.0 { 0 } else { 1 };
        o.values[1] = self.turbo.update(x);
        o.values[2] = cci;
    }
}

// ------------------------------------------------------------ Average Day Range

#[derive(Clone)]
struct Adr {
    n: usize,
    /// Completed days' ranges.
    ranges: ta::Sma,
    day: Option<(f64, f64)>,
    value: f64,
}

impl Study for Adr {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let b = c.bar;
        if !c.tf.is_intraday() {
            // On daily and longer bars it's simply the SMA of the ranges.
            o.values[0] = self.ranges.update(b.high - b.low);
            return;
        }
        // Intraday: the previous completed days, as request.security with
        // lookahead off gives on history.
        if c.new_session || c.index == 0 {
            if let Some((h, l)) = self.day.take() {
                self.value = self.ranges.update(h - l);
            }
            self.day = Some((b.high, b.low));
        } else if let Some((h, l)) = &mut self.day {
            *h = h.max(b.high);
            *l = l.min(b.low);
        }
        let _ = self.n;
        o.values[0] = self.value;
    }
}

// ------------------------------------------------------------ 24-hour Volume

#[derive(Clone)]
struct Volume24 {
    window: std::collections::VecDeque<(i64, f64)>,
    sum: f64,
}

impl Study for Volume24 {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let t = c.bar.time;
        self.window.push_back((t, c.bar.volume));
        self.sum += c.bar.volume;
        while let Some(&(t0, v)) = self.window.front() {
            if t - t0 >= 86_400_000 {
                self.window.pop_front();
                self.sum -= v;
            } else {
                break;
            }
        }
        o.values[0] = self.sum;
    }
}

// ------------------------------------------------------------ Moon Phases

/// Times (ms) of new (k integer) or full (k + 0.5) moons, after Meeus,
/// Astronomical Algorithms ch. 49 (main periodic terms; accurate to a few
/// minutes).
pub(crate) fn moon_phase_time(k: f64) -> i64 {
    let t = k / 1236.85;
    let jde =
        2451550.09766 + 29.530588861 * k + 0.00015437 * t * t - 0.00000015 * t * t * t + 0.00000000073 * t * t * t * t;
    let e = 1.0 - 0.002516 * t - 0.0000074 * t * t;
    let rad = |d: f64| d.to_radians();
    let m = rad(2.5534 + 29.10535670 * k - 0.0000014 * t * t);
    let mp = rad(201.5643 + 385.81693528 * k + 0.0107582 * t * t);
    let f = rad(160.7108 + 390.67050284 * k - 0.0016118 * t * t);
    let om = rad(124.7746 - 1.56375588 * k + 0.0020672 * t * t);
    let full = (k.rem_euclid(1.0) - 0.5).abs() < 0.01;
    let corr = if full {
        -0.40614 * mp.sin()
            + 0.17302 * e * m.sin()
            + 0.01614 * (2.0 * mp).sin()
            + 0.01043 * (2.0 * f).sin()
            + 0.00734 * e * (mp - m).sin()
            - 0.00515 * e * (mp + m).sin()
            + 0.00209 * e * e * (2.0 * m).sin()
            - 0.00111 * (mp - 2.0 * f).sin()
            - 0.00057 * (mp + 2.0 * f).sin()
            + 0.00056 * e * (2.0 * mp + m).sin()
            - 0.00042 * (3.0 * mp).sin()
            - 0.00017 * om.sin()
    } else {
        -0.40720 * mp.sin()
            + 0.17241 * e * m.sin()
            + 0.01608 * (2.0 * mp).sin()
            + 0.01039 * (2.0 * f).sin()
            + 0.00739 * e * (mp - m).sin()
            - 0.00514 * e * (mp + m).sin()
            + 0.00208 * e * e * (2.0 * m).sin()
            - 0.00111 * (mp - 2.0 * f).sin()
            - 0.00057 * (mp + 2.0 * f).sin()
            + 0.00056 * e * (2.0 * mp + m).sin()
            - 0.00042 * (3.0 * mp).sin()
            - 0.00017 * om.sin()
    };
    let jd = jde + corr;
    // JD → Unix ms (ignoring ΔT, about a minute).
    ((jd - 2440587.5) * 86_400_000.0) as i64
}

#[derive(Clone)]
struct Moon {
    next: Option<(i64, bool)>,
}

impl Moon {
    /// The first new or full moon at or after `t`.
    fn first_after(t: i64) -> (i64, bool) {
        let days = (t as f64 / 86_400_000.0 - 10_957.0) / 29.530588861;
        let mut k = (days * 2.0).floor() / 2.0 - 1.0;
        loop {
            let at = moon_phase_time(k);
            if at >= t {
                return (at, (k.rem_euclid(1.0) - 0.5).abs() < 0.01);
            }
            k += 0.5;
        }
    }
}

impl Study for Moon {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        let start = c.bar.time;
        let end = start + c.tf.millis().unwrap_or(30 * 86_400_000);
        let (at, full) = match self.next {
            Some(n) if n.0 >= start => n,
            _ => Moon::first_after(start),
        };
        if at < end {
            o.values[if full { 1 } else { 0 }] = 1.0;
            self.next = Some(Moon::first_after(at + 1));
        } else {
            self.next = Some((at, full));
        }
    }
}

// ------------------------------------------------------------ Trading Sessions

#[derive(Clone)]
struct Sessions {
    zones: Vec<(jiff::tz::TimeZone, i32, i32, u32)>,
}

impl Study for Sessions {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        if !c.tf.is_intraday() {
            return;
        }
        let Ok(ts) = jiff::Timestamp::from_millisecond(c.bar.time) else {
            return;
        };
        for (tz, from, to, color) in &self.zones {
            let z = ts.to_zoned(tz.clone());
            if matches!(
                z.weekday(),
                jiff::civil::Weekday::Saturday | jiff::civil::Weekday::Sunday
            ) {
                continue;
            }
            let m = z.hour() as i32 * 60 + z.minute() as i32;
            if m >= *from && m < *to {
                o.bg = *color;
            }
        }
    }
}

pub(super) fn metas() -> Vec<Meta> {
    use Category::{MovingAverages as Ma, Oscillators as Osc, Trend, Volatility as Vol, Volume as Vlm};
    use Placement::{Overlay, Pane};
    let blue_band = Rgba::hex(0x2196F3).transp(90);
    let mut v = vec![
        meta(
            "chaikin_osc",
            "Chaikin Oscillator",
            "Chaikin Osc",
            Vlm,
            Pane,
            vec![
                int("short", "Fast Length", 3, 1, 5000),
                int("long", "Slow Length", 10, 1, 5000),
            ],
            vec![line("Chaikin Oscillator", Rgba::hex(0xEC407A))],
            |i| {
                Box::new(ChaikinOsc {
                    ad: ta::Cum::default(),
                    fast: ta::ema(i.len("short")),
                    slow: ta::ema(i.len("long")),
                })
            },
        ),
        meta(
            "ckstop",
            "Chande Kroll Stop",
            "Chande Kroll Stop",
            Trend,
            Overlay,
            vec![
                int("p", "ATR Length", 10, 1, 5000),
                float("x", "ATR Coefficient", 1.0, 0.01, 100.0, 0.1),
                int("q", "Stop Length", 9, 1, 5000),
            ],
            vec![line("Stop Long", tv::BLUE), line("Stop Short", tv::ORANGE)],
            |i| {
                let p = i.len("p");
                Box::new(ChandeKroll {
                    hh: ta::Extreme::highest(p),
                    ll: ta::Extreme::lowest(p),
                    atr: ta::Atr::new(p),
                    x: i.float("x"),
                    stop_short: ta::Extreme::highest(i.len("q")),
                    stop_long: ta::Extreme::lowest(i.len("q")),
                })
            },
        ),
        meta(
            "chop_zone",
            "Chop Zone",
            "Chop Zone",
            Trend,
            Pane,
            vec![],
            vec![
                line("Chop Zone", Rgba::hex(0x26C6DA))
                    .style(PlotStyle::Columns)
                    .colors(&[
                        ("Turquoise", Rgba::hex(0x26C6DA)),
                        ("Dark green", Rgba::hex(0x43A047)),
                        ("Pale green", Rgba::hex(0xA5D6A7)),
                        ("Lime", Rgba::hex(0x009688)),
                        ("Dark red", Rgba::hex(0xD50000)),
                        ("Red", Rgba::hex(0xE91E63)),
                        ("Orange", Rgba::hex(0xFF6D00)),
                        ("Light orange", Rgba::hex(0xFFB74D)),
                        ("Yellow", Rgba::hex(0xFDD835)),
                    ]),
            ],
            |_| {
                Box::new(ChopZone {
                    hh: ta::Extreme::highest(30),
                    ll: ta::Extreme::lowest(30),
                    ema: ta::ema(34),
                    prev: f64::NAN,
                })
            },
        ),
        meta(
            "crsi",
            "Connors RSI",
            "CRSI",
            Osc,
            Pane,
            vec![
                int("lenrsi", "RSI Length", 3, 1, 5000),
                int("lenupdown", "UpDown Length", 2, 1, 5000),
                int("lenroc", "ROC Length", 100, 1, 5000),
            ],
            vec![line("CRSI", tv::BLUE)],
            |i| {
                Box::new(ConnorsRsi {
                    rsi: ta::Rsi::new(i.len("lenrsi")),
                    prev: f64::NAN,
                    streak: 0.0,
                    streak_rsi: ta::Rsi::new(i.len("lenupdown")),
                    roc_prev: ta::Prev::new(1),
                    pr: Hist::new(i.len("lenroc") + 1),
                    lenroc: i.len("lenroc"),
                })
            },
        ),
        meta(
            "kvo",
            "Klinger Oscillator",
            "KVO",
            Vlm,
            Pane,
            vec![
                int("fast", "Fast Length", 34, 1, 5000),
                int("slow", "Slow Length", 55, 1, 5000),
                int("signal", "Signal Length", 13, 1, 5000),
            ],
            vec![line("Klinger Oscillator", tv::BLUE), line("Signal", tv::LEAF)],
            |i| {
                Box::new(Klinger {
                    prev: f64::NAN,
                    fast: ta::ema(i.len("fast")),
                    slow: ta::ema(i.len("slow")),
                    signal: ta::ema(i.len("signal")),
                })
            },
        ),
        meta(
            "ma_ribbon",
            "Moving Average Ribbon",
            "MA Ribbon",
            Ma,
            Overlay,
            vec![
                choice("type", "MA Type", "SMA", ta::MA_TYPES),
                source("close"),
                int("len1", "MA #1 Length", 20, 1, 5000),
                int("len2", "MA #2 Length", 50, 1, 5000),
                int("len3", "MA #3 Length", 100, 1, 5000),
                int("len4", "MA #4 Length", 200, 1, 5000),
            ],
            vec![
                line("MA #1", Rgba::hex(0xF6C309)),
                line("MA #2", Rgba::hex(0xFB9800)),
                line("MA #3", Rgba::hex(0xFB6500)),
                line("MA #4", Rgba::hex(0xF60C0C)),
            ],
            |i| {
                Box::new(Ribbon {
                    src: i.src("source"),
                    mas: ["len1", "len2", "len3", "len4"]
                        .iter()
                        .map(|k| ta::Ma::by_name(i.text("type"), i.len(k)))
                        .collect(),
                })
            },
        ),
        meta(
            "net_volume",
            "Net Volume",
            "Net Volume",
            Vlm,
            Pane,
            vec![],
            vec![line("Net Volume", tv::BLUE)],
            |_| Box::new(NetVolume { prev: f64::NAN }),
        ),
        {
            let mut m = meta(
                "pivots_hl",
                "Pivot Points High Low",
                "Pivots HL",
                Trend,
                Overlay,
                vec![
                    int("left", "Left Bars", 10, 1, 500),
                    int("right", "Right Bars", 10, 1, 500),
                ],
                vec![
                    PlotSpec {
                        shape: Some(ShapeSpec {
                            kind: ShapeKind::LabelDown,
                            location: Location::AboveBar,
                            text: "H",
                            size: 13.0,
                        }),
                        ..line("Pivot High", tv::BLUE)
                            .style(PlotStyle::Shape)
                            .offset_by("right", -1, 0)
                    },
                    PlotSpec {
                        shape: Some(ShapeSpec {
                            kind: ShapeKind::LabelUp,
                            location: Location::BelowBar,
                            text: "L",
                            size: 13.0,
                        }),
                        ..line("Pivot Low", tv::RED)
                            .style(PlotStyle::Shape)
                            .offset_by("right", -1, 0)
                    },
                ],
                |i| {
                    let (l, r) = (i.len("left"), i.len("right"));
                    Box::new(PivotsHl {
                        left: l,
                        right: r,
                        highs: Hist::new(l + r + 1),
                        lows: Hist::new(l + r + 1),
                    })
                },
            );
            m.format = Format::Price;
            m
        },
        meta(
            "rci",
            "Rank Correlation Index",
            "RCI",
            Osc,
            Pane,
            vec![length(10), source("close")],
            vec![line("RCI", tv::BLUE)],
            |i| {
                let n = i.len("length");
                Box::new(Rci {
                    src: i.src("source"),
                    h: Hist::new(n),
                    lens: vec![n],
                })
            },
        ),
        meta(
            "rci_ribbon",
            "RCI Ribbon",
            "RCI Ribbon",
            Osc,
            Pane,
            vec![
                source("close"),
                int("short", "Short RCI Length", 10, 2, 5000),
                int("middle", "Middle RCI Length", 30, 2, 5000),
                int("long", "Long RCI Length", 50, 2, 5000),
            ],
            vec![
                line("Short RCI", Rgba::hex(0xF44336)),
                line("Middle RCI", tv::BLUE),
                line("Long RCI", Rgba::hex(0x43A047)),
            ],
            |i| {
                let lens = vec![i.len("short"), i.len("middle"), i.len("long")];
                Box::new(Rci {
                    src: i.src("source"),
                    h: Hist::new(*lens.iter().max().unwrap_or(&50)),
                    lens,
                })
            },
        ),
        meta(
            "rvgi",
            "Relative Vigor Index",
            "RVGI",
            Osc,
            Pane,
            vec![length(10)],
            vec![line("RVGI", Rgba::hex(0x008000)), line("Signal", Rgba::hex(0xFF0000))],
            |i| {
                let n = i.len("length");
                Box::new(Rvgi {
                    num: Hist::new(4),
                    den: Hist::new(4),
                    s_num: ta::Sum::new(n),
                    s_den: ta::Sum::new(n),
                    rvi: Hist::new(4),
                })
            },
        ),
        meta(
            "rvi",
            "Relative Volatility Index",
            "RVI",
            Vol,
            Pane,
            vec![length(10), source("close"), int("ema", "Smoothing Length", 14, 1, 5000)],
            vec![line("RVI", tv::PURPLE)],
            |i| {
                Box::new(RelVol {
                    src: i.src("source"),
                    sd: ta::Stdev::new(i.len("length")),
                    prev: f64::NAN,
                    up: ta::ema(i.len("ema")),
                    down: ta::ema(i.len("ema")),
                })
            },
        ),
        meta(
            "smi_ergodic",
            "SMI Ergodic Indicator",
            "SMII",
            Osc,
            Pane,
            vec![
                int("long", "Long Length", 20, 1, 5000),
                int("short", "Short Length", 5, 1, 5000),
                int("signal", "Signal Line Length", 5, 1, 5000),
            ],
            vec![line("SMI", tv::BLUE), line("Signal", tv::ORANGE)],
            |i| ergodic(i, false),
        ),
        meta(
            "smi_ergodic_osc",
            "SMI Ergodic Oscillator",
            "SMIO",
            Osc,
            Pane,
            vec![
                int("long", "Long Length", 20, 1, 5000),
                int("short", "Short Length", 5, 1, 5000),
                int("signal", "Signal Line Length", 5, 1, 5000),
            ],
            vec![line("SMI Ergodic Oscillator", Rgba::hex(0xFF5252)).style(PlotStyle::Histogram)],
            |i| ergodic(i, true),
        ),
        meta(
            "stderr",
            "Standard Error",
            "StdErr",
            Vol,
            Pane,
            vec![length(21), source("close")],
            vec![line("Standard Error", tv::BLUE)],
            |i| {
                let n = i.len("length").max(3);
                Box::new(StdErr {
                    src: i.src("source"),
                    h: Hist::new(n),
                    n,
                    bands: None,
                })
            },
        ),
        meta(
            "stderr_bands",
            "Standard Error Bands",
            "SEB",
            Vol,
            Overlay,
            vec![
                length(21),
                source("close"),
                float("mult", "Multiplier", 2.0, 0.01, 100.0, 0.1),
                int("smooth", "Smoothing", 3, 1, 500),
            ],
            vec![line("Middle", tv::BLUE), line("Upper", tv::RED), line("Lower", tv::RED)],
            |i| {
                let n = i.len("length").max(3);
                let s = i.len("smooth");
                Box::new(StdErr {
                    src: i.src("source"),
                    h: Hist::new(n),
                    n,
                    bands: Some((i.float("mult"), ta::Sma::new(s), ta::Sma::new(s), ta::Sma::new(s))),
                })
            },
        ),
        meta(
            "smi",
            "Stochastic Momentum Index",
            "SMI",
            Osc,
            Pane,
            vec![
                int("k", "%K Length", 10, 1, 5000),
                int("d", "%D Length", 3, 1, 5000),
                int("ema", "EMA Length", 10, 1, 5000),
            ],
            vec![line("SMI", tv::BLUE), line("SMI-based EMA", tv::ORANGE)],
            |i| {
                let (k, d) = (i.len("k"), i.len("d"));
                Box::new(Smi {
                    hh: ta::Extreme::highest(k),
                    ll: ta::Extreme::lowest(k),
                    r1: ta::ema(d),
                    r2: ta::ema(d),
                    d1: ta::ema(d),
                    d2: ta::ema(d),
                    ema: ta::ema(i.len("ema")),
                })
            },
        ),
        meta(
            "twap",
            "Time Weighted Average Price",
            "TWAP",
            Trend,
            Overlay,
            vec![source("ohlc4")],
            vec![line("TWAP", tv::BLUE)],
            |i| {
                Box::new(Twap {
                    src: i.src("source"),
                    sum: 0.0,
                    n: 0.0,
                })
            },
        ),
        meta(
            "trend_strength",
            "Trend Strength Index",
            "TSI",
            Trend,
            Pane,
            vec![length(14)],
            vec![line("Trend Strength Index", tv::BLUE)],
            |i| {
                let n = i.len("length").max(2);
                Box::new(TrendStrength {
                    n,
                    y: Hist::new(n),
                    x: Hist::new(n),
                })
            },
        ),
        meta(
            "volatility_stop",
            "Volatility Stop",
            "VStop",
            Vol,
            Overlay,
            vec![
                length(20),
                source("close"),
                float("factor", "Multiplier", 2.0, 0.01, 100.0, 0.1),
            ],
            vec![
                line("Volatility Stop", tv::TEAL)
                    .style(PlotStyle::Cross)
                    .colors(&[("Uptrend", tv::TEAL), ("Downtrend", tv::RED)]),
            ],
            |i| {
                Box::new(VolStop {
                    src: i.src("source"),
                    atr: ta::Atr::new(i.len("length")),
                    tr: ta::Tr::new(false),
                    factor: i.float("factor"),
                    max: f64::NAN,
                    min: f64::NAN,
                    uptrend: true,
                    prev_up: None,
                    stop: f64::NAN,
                    started: false,
                })
            },
        ),
        meta(
            "fractals",
            "Williams Fractals",
            "Fractals",
            Trend,
            Overlay,
            vec![int("n", "Periods", 2, 2, 100)],
            vec![
                PlotSpec {
                    shape: Some(ShapeSpec {
                        kind: ShapeKind::TriangleDown,
                        location: Location::BelowBar,
                        text: "",
                        size: 8.0,
                    }),
                    ..line("Down", tv::RED).style(PlotStyle::Shape).offset_by("n", -1, 0)
                },
                PlotSpec {
                    shape: Some(ShapeSpec {
                        kind: ShapeKind::TriangleUp,
                        location: Location::AboveBar,
                        text: "",
                        size: 8.0,
                    }),
                    ..line("Up", tv::TEAL).style(PlotStyle::Shape).offset_by("n", -1, 0)
                },
            ],
            |i| {
                let n = i.len("n");
                Box::new(Fractals {
                    n,
                    highs: Hist::new(2 * n + 1),
                    lows: Hist::new(2 * n + 1),
                })
            },
        ),
        meta(
            "woodies_cci",
            "Woodies CCI",
            "Woodies CCI",
            Osc,
            Pane,
            vec![
                int("cci", "CCI Length", 14, 1, 5000),
                int("turbo", "CCI Turbo Length", 6, 1, 5000),
            ],
            vec![
                line("CCI Histogram", Rgba::hex(0x26A69A))
                    .style(PlotStyle::Histogram)
                    .colors(&[("Positive", Rgba::hex(0x26A69A)), ("Negative", Rgba::hex(0xEF5350))]),
                line("CCI Turbo", tv::ORANGE),
                line("CCI", tv::BLUE),
            ],
            |i| {
                Box::new(Woodies {
                    cci: ta::Cci::new(i.len("cci")),
                    turbo: ta::Cci::new(i.len("turbo")),
                })
            },
        ),
        meta(
            "adr",
            "Average Day Range",
            "ADR",
            Vol,
            Pane,
            vec![length(14)],
            vec![line("ADR", tv::BLUE)],
            |i| {
                Box::new(Adr {
                    n: i.len("length"),
                    ranges: ta::Sma::new(i.len("length")),
                    day: None,
                    value: f64::NAN,
                })
            },
        ),
        {
            let mut m = meta(
                "volume_24h",
                "24-hour Volume",
                "24h Vol",
                Vlm,
                Pane,
                vec![],
                vec![line("24h Volume", tv::BLUE).style(PlotStyle::Columns)],
                |_| {
                    Box::new(Volume24 {
                        window: Default::default(),
                        sum: 0.0,
                    })
                },
            );
            m.format = Format::Volume;
            m
        },
        meta(
            "moon",
            "Moon Phases",
            "Moon",
            Trend,
            Overlay,
            vec![],
            vec![
                PlotSpec {
                    shape: Some(ShapeSpec {
                        kind: ShapeKind::Circle,
                        location: Location::BelowBar,
                        text: "",
                        size: 9.0,
                    }),
                    ..line("New Moon", Rgba::hex(0x363A45)).style(PlotStyle::Shape)
                },
                PlotSpec {
                    shape: Some(ShapeSpec {
                        kind: ShapeKind::Circle,
                        location: Location::AboveBar,
                        text: "",
                        size: 9.0,
                    }),
                    ..line("Full Moon", Rgba::hex(0xFFEB3B)).style(PlotStyle::Shape)
                },
            ],
            |_| Box::new(Moon { next: None }),
        ),
        meta(
            "sessions",
            "Trading Sessions",
            "Sessions",
            Trend,
            Overlay,
            vec![],
            vec![],
            |_| {
                let tz = |n: &str| jiff::tz::TimeZone::get(n).unwrap_or(jiff::tz::TimeZone::UTC);
                Box::new(Sessions {
                    zones: vec![
                        (tz("Asia/Tokyo"), 9 * 60, 15 * 60, Rgba::hex(0xF23645).transp(92).0),
                        (
                            tz("Europe/London"),
                            8 * 60,
                            16 * 60 + 30,
                            Rgba::hex(0x2962FF).transp(92).0,
                        ),
                        (
                            tz("America/New_York"),
                            9 * 60 + 30,
                            16 * 60,
                            Rgba::hex(0x089981).transp(92).0,
                        ),
                    ],
                })
            },
        ),
        {
            let up_down = [("Up", Rgba::hex(0x089981)), ("Down", Rgba::hex(0xF23645))];
            meta(
                "median",
                "Median",
                "Median",
                Category::Bands,
                Overlay,
                vec![
                    source("hl2"),
                    int("length", "Median Length", 3, 1, 5000),
                    int("atr_length", "ATR Length", 14, 1, 5000),
                    float("mult", "ATR Multiplier", 2.0, 0.0, 1000.0, 0.1),
                ],
                vec![
                    line("Upper", tv::GREEN).colors(&up_down),
                    line("Median", tv::GREEN).colors(&up_down).width(2),
                    line("Lower", tv::GREEN).colors(&up_down),
                    line("Median EMA", tv::BLUE).hidden(),
                ],
                |i| {
                    let n = i.len("length");
                    Box::new(Median {
                        src: i.src("source"),
                        win: Hist::new(n),
                        n,
                        atr: ta::Atr::new(i.len("atr_length")),
                        mult: i.float("mult"),
                        ema: ta::ema(n),
                    })
                },
            )
        },
        meta(
            "majority_rule",
            "Majority Rule",
            "MR",
            Osc,
            Pane,
            vec![source("close"), length(14)],
            vec![line("MR", tv::BLUE)],
            |i| {
                Box::new(MajorityRule {
                    src: i.src("source"),
                    prev: f64::NAN,
                    sum: ta::Sum::new(i.len("length")),
                    n: i.len("length"),
                })
            },
        ),
    ];
    // Levels and fills.
    for m in &mut v {
        match m.id {
            "crsi" => {
                (m.hlines, m.fills) = bands(70.0, 50.0, 30.0, Rgba::hex(0x7E57C2).transp(90));
            }
            "rvi" => {
                (m.hlines, m.fills) = bands(80.0, 50.0, 20.0, Rgba::hex(0x7E57C2).transp(90));
            }
            "smi" => {
                m.hlines = vec![hline("Upper Band", 40.0, LEVEL), hline("Lower Band", -40.0, LEVEL)];
                m.fills = vec![fill_h("Background", 0, 1, blue_band)];
            }
            "rci" | "rci_ribbon" => {
                (m.hlines, m.fills) = bands(80.0, 0.0, -80.0, blue_band);
            }
            "woodies_cci" => {
                m.hlines = vec![
                    hline("Zero", 0.0, MID),
                    hline("100", 100.0, LEVEL),
                    hline("-100", -100.0, LEVEL),
                    hline("200", 200.0, LEVEL),
                    hline("-200", -200.0, LEVEL),
                ];
            }
            "kvo" | "smi_ergodic" | "smi_ergodic_osc" | "chaikin_osc" | "net_volume" | "trend_strength" => {
                m.hlines = vec![zero_line()];
            }
            "stderr_bands" => {
                m.fills = vec![fill_p("Background", 1, 2, blue_band)];
            }
            "median" => {
                m.fills = vec![fill_p("Background", 0, 2, Rgba::hex(0x089981).transp(90)).colors(&[
                    ("Up", Rgba::hex(0x089981).transp(90)),
                    ("Down", Rgba::hex(0xF23645).transp(90)),
                ])];
            }
            "majority_rule" => {
                m.hlines = vec![hline("Middle", 50.0, MID)];
            }
            _ => {}
        }
    }
    v
}

fn ergodic(i: &Inputs, osc: bool) -> Box<dyn Study> {
    let (l, s) = (i.len("long"), i.len("short"));
    Box::new(Ergodic {
        prev: ta::Prev::new(1),
        a1: ta::ema(l),
        a2: ta::ema(s),
        b1: ta::ema(l),
        b2: ta::ema(s),
        sig: ta::ema(i.len("signal")),
        osc,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moon_phases_land_on_known_dates() {
        // Full moon 2024-01-25 17:54 UTC; new moon 2024-01-11 11:57 UTC.
        let (t, full) = Moon::first_after(1_705_968_000_000); // 2024-01-23
        assert!(full);
        assert!((t - 1_706_205_240_000).abs() < 15 * 60_000, "{t}");
        let (t, full) = Moon::first_after(1_704_758_400_000); // 2024-01-09
        assert!(!full);
        assert!((t - 1_704_974_220_000).abs() < 15 * 60_000, "{t}");
    }

    #[test]
    fn rci_of_rising_prices_is_100() {
        let mut h = Hist::new(5);
        for x in [1.0, 2.0, 3.0, 4.0, 5.0] {
            h.push(x);
        }
        assert_eq!(rci(&h, 5), 100.0);
        let mut h = Hist::new(5);
        for x in [5.0, 4.0, 3.0, 2.0, 1.0] {
            h.push(x);
        }
        assert_eq!(rci(&h, 5), -100.0);
    }

    #[test]
    fn regression_fits_a_line_exactly() {
        let mut h = Hist::new(10);
        for i in 0..10 {
            h.push(3.0 + 2.0 * i as f64);
        }
        let (line, se) = regression(&h, 10);
        assert!((line - 21.0).abs() < 1e-9 && se.abs() < 1e-9);
    }
}

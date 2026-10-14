//! TradingView built-ins that draw rather than plot: Zig Zag, Auto Fib
//! Retracement, Pivot Points Standard and Linear Regression Channel. They
//! keep their state bar by bar like the others and hand the chart labels,
//! lines and boxes through `Study::drawings`.
//!
//! Linear Regression Channel follows TradingView's published source line by
//! line. Zig Zag (and Auto Fib, which reads its last leg) follows the
//! behaviour of TradingView's ZigZag library: a pivot needs `depth / 2`
//! bars on each side, a pivot in the same direction as the last one
//! replaces it when it goes further, and a turn needs the price to move at
//! least the deviation. Their exact pivots are on the "to verify against an
//! export" list in `docs/indicators.md`.

use super::*;
use crate::color::tv;
use crate::drawings::{Drawings, Label, Line, Linefill, PineBox, XLoc};
use crate::ta::Hist;
use opentern_core::Bar;
use std::sync::Arc;

fn s(x: &str) -> Arc<str> {
    Arc::from(x)
}

pub(crate) fn colour(id: &'static str, title: &'static str, c: Rgba) -> InputSpec {
    InputSpec {
        id,
        title,
        kind: InputKind::Color,
        default: Value::Text(format!("#{:08X}", c.0)),
        group: None,
    }
}

/// A colour input's value (`#RRGGBB` or `#RRGGBBAA`).
pub fn colour_of(ins: &Inputs, id: &str, d: Rgba) -> u32 {
    let t = ins.text(id).trim_start_matches('#');
    match t.len() {
        6 => u32::from_str_radix(t, 16).map(|v| (v << 8) | 0xFF).unwrap_or(d.0),
        8 => u32::from_str_radix(t, 16).unwrap_or(d.0),
        _ => d.0,
    }
}

fn with_alpha(c: u32, transp: u32) -> u32 {
    Rgba(c).transp(transp).0
}

/// Prices as TradingView prints them on drawings: grouped, trimmed.
pub(crate) fn price_text(v: f64) -> String {
    if !v.is_finite() {
        return "—".into();
    }
    let dec = if v.abs() >= 1000.0 {
        2
    } else if v.abs() >= 1.0 {
        4
    } else {
        6
    };
    let t = format!("{v:.dec$}");
    let t = if t.contains('.') {
        t.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        t
    };
    let (int, frac) = t.split_once('.').map(|(a, b)| (a, Some(b))).unwrap_or((&t, None));
    let (sign, digits) = int.strip_prefix('-').map(|d| ("-", d)).unwrap_or(("", int));
    let mut g = String::new();
    for (k, ch) in digits.chars().enumerate() {
        if k > 0 && (digits.len() - k) % 3 == 0 {
            g.push(',');
        }
        g.push(ch);
    }
    match frac {
        Some(f) => format!("{sign}{g}.{f}"),
        None => format!("{sign}{g}"),
    }
}

fn volume_text(v: f64) -> String {
    let a = v.abs();
    let (d, u) = if a >= 1e9 {
        (1e9, "B")
    } else if a >= 1e6 {
        (1e6, "M")
    } else if a >= 1e3 {
        (1e3, "K")
    } else {
        (1.0, "")
    };
    let t = format!("{:.2}", v / d);
    format!("{}{u}", t.trim_end_matches('0').trim_end_matches('.'))
}

fn label(x: i64, y: f64, text: String, style: &str, color: u32, textcolor: u32) -> Label {
    Label {
        x,
        y,
        xloc: XLoc::BarIndex,
        yloc: s("price"),
        text: Arc::from(text),
        style: s(style),
        color,
        textcolor,
        size: s("normal"),
        textalign: s("center"),
        tooltip: s(""),
    }
}

fn line(x1: i64, y1: f64, x2: i64, y2: f64, color: u32, width: f64, extend: &str) -> Line {
    Line {
        x1,
        y1,
        x2,
        y2,
        xloc: XLoc::BarIndex,
        extend: s(extend),
        color,
        style: s("solid"),
        width,
    }
}

// ------------------------------------------------------------ zig zag engine

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pivot {
    pub x: i64,
    pub price: f64,
    pub high: bool,
    /// Volume from the previous pivot to this one.
    pub vol: f64,
}

#[derive(Clone)]
pub(crate) struct ZigZag {
    dev: f64,
    legs: usize,
    highs: Hist,
    lows: Hist,
    vols: Hist,
    pub pivots: Vec<Pivot>,
    /// Volume since the last pivot's bar.
    since: f64,
}

impl ZigZag {
    pub fn new(dev: f64, depth: usize) -> ZigZag {
        let legs = (depth / 2).max(1);
        ZigZag {
            dev,
            legs,
            highs: Hist::new(2 * legs + 1),
            lows: Hist::new(2 * legs + 1),
            vols: Hist::new(2 * legs + 1),
            pivots: Vec::new(),
            since: 0.0,
        }
    }

    /// A pivot `legs` bars back: beyond everything on its right, and at
    /// least as far as everything on its left.
    fn found(h: &Hist, legs: usize, high: bool) -> Option<f64> {
        let p = h.get(legs);
        if p.is_nan() || h.get(2 * legs).is_nan() {
            return None;
        }
        let beats = |v: f64, strict: bool| {
            if high {
                if strict { v > p } else { v >= p }
            } else if strict {
                v < p
            } else {
                v <= p
            }
        };
        let right = (0..legs).any(|i| beats(h.get(i), true));
        let left = (legs + 1..=2 * legs).any(|i| beats(h.get(i), false));
        (!right && !left).then_some(p)
    }

    pub fn update(&mut self, index: usize, b: &Bar) {
        self.highs.push(b.high);
        self.lows.push(b.low);
        self.vols.push(b.volume);
        self.since += b.volume;
        let x = index as i64 - self.legs as i64;
        // Volume after the pivot bar belongs to the next leg.
        let after: f64 = (0..self.legs).map(|i| self.vols.get(i)).filter(|v| !v.is_nan()).sum();
        for high in [true, false] {
            let h = if high { &self.highs } else { &self.lows };
            let Some(p) = Self::found(h, self.legs, high) else {
                continue;
            };
            self.pivot(x, p, high, after);
        }
    }

    fn pivot(&mut self, x: i64, price: f64, high: bool, after: f64) {
        let vol = (self.since - after).max(0.0);
        match self.pivots.last_mut() {
            None => {
                self.pivots.push(Pivot {
                    x,
                    price,
                    high,
                    vol: 0.0,
                });
                self.since = after;
            }
            Some(last) if last.high == high => {
                if (high && price > last.price) || (!high && price < last.price) {
                    last.x = x;
                    last.price = price;
                    last.vol += vol;
                    self.since = after;
                }
            }
            Some(last) => {
                let dev = 100.0 * (price - last.price) / last.price.abs();
                if dev.abs() >= self.dev {
                    self.pivots.push(Pivot { x, price, high, vol });
                    self.since = after;
                }
            }
        }
    }
}

// ------------------------------------------------------------ Zig Zag

#[derive(Clone)]
struct ZigZagStudy {
    zz: ZigZag,
    color: u32,
    extend: bool,
    show_price: bool,
    show_vol: bool,
    show_change: bool,
    percent: bool,
    last: Option<(i64, f64)>,
}

impl Study for ZigZagStudy {
    fn step(&mut self, c: &Ctx, _: &mut Out) {
        self.zz.update(c.index, c.bar);
        self.last = Some((c.index as i64, c.bar.close));
    }

    fn drawings(&self) -> Option<Drawings> {
        let mut d = Drawings::default();
        let p = &self.zz.pivots;
        let from = p.len().saturating_sub(500);
        for k in from..p.len() {
            let cur = &p[k];
            if k > 0 {
                let prev = &p[k - 1];
                d.lines
                    .push(line(prev.x, prev.price, cur.x, cur.price, self.color, 2.0, "none"));
            }
            let mut parts = Vec::new();
            if self.show_price {
                parts.push(price_text(cur.price));
            }
            if self.show_change && k > 0 {
                let diff = cur.price - p[k - 1].price;
                parts.push(if self.percent {
                    format!("({:+.2}%)", 100.0 * diff / p[k - 1].price.abs())
                } else {
                    format!("({}{})", if diff >= 0.0 { "+" } else { "" }, price_text(diff))
                });
            }
            if self.show_vol && k > 0 {
                parts.push(volume_text(cur.vol));
            }
            if !parts.is_empty() {
                let text = if self.show_price && parts.len() > 1 {
                    let first = parts.remove(0);
                    let rest = parts.join("\n");
                    match rest.split_once('\n') {
                        Some((chg, vol)) if self.show_change && k > 0 => format!("{first} {chg}\n{vol}"),
                        _ if self.show_change && k > 0 => format!("{first} {rest}"),
                        _ => format!("{first}\n{rest}"),
                    }
                } else {
                    parts.join("\n")
                };
                let style = if cur.high { "label_down" } else { "label_up" };
                d.labels
                    .push(label(cur.x, cur.price, text, style, 0xFFFF_FF00, self.color));
            }
        }
        if self.extend
            && let (Some(lp), Some((x, close))) = (p.last(), self.last)
            && x > lp.x
        {
            d.lines.push(line(lp.x, lp.price, x, close, self.color, 2.0, "none"));
        }
        Some(d)
    }
}

// ------------------------------------------------------------ Auto Fib Retracement

const FIB_LEVELS: [(&str, f64, u32, bool); 11] = [
    ("l0", 0.0, 0x787B86, true),
    ("l0236", 0.236, 0xF44336, true),
    ("l0382", 0.382, 0x81C784, true),
    ("l05", 0.5, 0x4CAF50, true),
    ("l0618", 0.618, 0x009688, true),
    ("l0786", 0.786, 0x64B5F6, true),
    ("l1", 1.0, 0x787B86, true),
    ("l1618", 1.618, 0x2962FF, true),
    ("l2618", 2.618, 0xF44336, true),
    ("l3618", 3.618, 0x9C27B0, true),
    ("l4236", 4.236, 0xE91E63, true),
];

#[derive(Clone)]
struct AutoFib {
    zz: ZigZag,
    reverse: bool,
    extend_left: bool,
    extend_right: bool,
    prices: bool,
    levels: bool,
    percent: bool,
    left: bool,
    bg: bool,
    shown: Vec<bool>,
    last: i64,
    /// Auto Fib Extension: levels project the leg before last (A→B) from
    /// the last pivot (C), instead of retracing the last leg.
    extension: bool,
}

impl Study for AutoFib {
    fn step(&mut self, c: &Ctx, _: &mut Out) {
        self.zz.update(c.index, c.bar);
        self.last = c.index as i64;
    }

    fn drawings(&self) -> Option<Drawings> {
        let p = &self.zz.pivots;
        let (a, b, c) = match (p.len(), self.extension) {
            (n, false) if n >= 2 => (&p[n - 2], &p[n - 1], None),
            (n, true) if n >= 3 => (&p[n - 3], &p[n - 2], Some(&p[n - 1])),
            _ => return None,
        };
        // Retracement: 1 at the leg's start, 0 at its end (reversed: the
        // other way). Extension: 0 at C, 1 at C + (B − A).
        let (end, height) = match c {
            Some(c) => {
                let leg = b.price - a.price;
                if self.reverse {
                    (c.price + leg, -leg)
                } else {
                    (c.price, leg)
                }
            }
            None => {
                let (start, end) = if self.reverse {
                    (b.price, a.price)
                } else {
                    (a.price, b.price)
                };
                (end, start - end)
            }
        };
        let a = match c {
            Some(c) => c,
            None => a,
        };
        let extend = match (self.extend_left, self.extend_right) {
            (true, true) => "both",
            (true, false) => "left",
            (false, true) => "right",
            _ => "none",
        };
        let (x1, x2) = (a.x, self.last.max(a.x + 1));
        let mut d = Drawings::default();
        let mut prev: Option<(f64, u32)> = None;
        for (k, (_, lvl, col, _)) in FIB_LEVELS.iter().enumerate() {
            if !self.shown[k] {
                continue;
            }
            let y = end + height * lvl;
            let c = Rgba::hex(*col).0;
            d.lines.push(line(x1, y, x2, y, c, 1.0, extend));
            if self.bg
                && let Some((py, _)) = prev
            {
                let (l, r) = match extend {
                    "left" | "both" => (x1 - 100_000, x2),
                    _ => (x1, x2),
                };
                let r = if matches!(extend, "right" | "both") {
                    r + 100_000
                } else {
                    r
                };
                d.boxes.push(PineBox {
                    left: l,
                    top: py.max(y),
                    right: r,
                    bottom: py.min(y),
                    xloc: XLoc::BarIndex,
                    border_color: 0,
                    border_width: 0.0,
                    border_style: s("solid"),
                    extend: s("none"),
                    bgcolor: with_alpha(c, 85),
                    text: s(""),
                    text_size: s("auto"),
                    text_color: 0,
                    text_halign: s("center"),
                    text_valign: s("center"),
                });
            }
            prev = Some((y, c));
            let mut text = String::new();
            if self.levels {
                text = if self.percent {
                    format!("{}%", price_text(lvl * 100.0))
                } else {
                    price_text(*lvl)
                };
            }
            if self.prices {
                let pr = format!("({})", price_text(y));
                text = if text.is_empty() { pr } else { format!("{text} {pr}") };
            }
            if !text.is_empty() {
                let (x, style) = if self.left {
                    (x1, "label_right")
                } else {
                    (x2, "label_left")
                };
                d.labels.push(label(x, y, text, style, 0xFFFF_FF00, c));
            }
        }
        // The leg(s) measured, dashed.
        let pts: Vec<&Pivot> = match self.extension {
            true => p[p.len() - 3..].iter().collect(),
            false => p[p.len() - 2..].iter().collect(),
        };
        for w in pts.windows(2) {
            let mut leg = line(w[0].x, w[0].price, w[1].x, w[1].price, tv::GRAY.0, 1.0, "none");
            leg.style = s("dashed");
            d.lines.push(leg);
        }
        Some(d)
    }
}

// ------------------------------------------------------------ Pivot Points Standard

/// Named price levels ("P", "R1", …).
type Levels = Vec<(&'static str, f64)>;

const PIVOT_TYPES: &[&str] = &["Traditional", "Fibonacci", "Woodie", "Classic", "DM", "Camarilla"];
const PIVOT_TFS: &[&str] = &["Auto", "Daily", "Weekly", "Monthly", "Quarterly", "Yearly"];

/// The levels of one period from the previous period's prices.
pub(crate) fn pivot_levels(kind: &str, o: f64, h: f64, l: f64, c: f64, cur_open: f64) -> Levels {
    let r = h - l;
    match kind {
        "Fibonacci" => {
            let p = (h + l + c) / 3.0;
            vec![
                ("P", p),
                ("R1", p + 0.382 * r),
                ("S1", p - 0.382 * r),
                ("R2", p + 0.618 * r),
                ("S2", p - 0.618 * r),
                ("R3", p + r),
                ("S3", p - r),
            ]
        }
        "Woodie" => {
            let p = (h + l + 2.0 * cur_open) / 4.0;
            let r3 = h + 2.0 * (p - l);
            let s3 = l - 2.0 * (h - p);
            vec![
                ("P", p),
                ("R1", 2.0 * p - l),
                ("S1", 2.0 * p - h),
                ("R2", p + r),
                ("S2", p - r),
                ("R3", r3),
                ("S3", s3),
                ("R4", r3 + r),
                ("S4", s3 - r),
            ]
        }
        "Classic" => {
            let p = (h + l + c) / 3.0;
            vec![
                ("P", p),
                ("R1", 2.0 * p - l),
                ("S1", 2.0 * p - h),
                ("R2", p + r),
                ("S2", p - r),
                ("R3", p + 2.0 * r),
                ("S3", p - 2.0 * r),
                ("R4", p + 3.0 * r),
                ("S4", p - 3.0 * r),
            ]
        }
        "DM" => {
            let x = if c < o {
                h + 2.0 * l + c
            } else if c > o {
                2.0 * h + l + c
            } else {
                h + l + 2.0 * c
            };
            vec![("P", x / 4.0), ("R1", x / 2.0 - l), ("S1", x / 2.0 - h)]
        }
        "Camarilla" => {
            let p = (h + l + c) / 3.0;
            let k = 1.1 * r;
            let (r3, r4) = (c + k / 4.0, c + k / 2.0);
            let (s3, s4) = (c - k / 4.0, c - k / 2.0);
            vec![
                ("P", p),
                ("R1", c + k / 12.0),
                ("S1", c - k / 12.0),
                ("R2", c + k / 6.0),
                ("S2", c - k / 6.0),
                ("R3", r3),
                ("S3", s3),
                ("R4", r4),
                ("S4", s4),
                ("R5", r4 + 1.168 * (r4 - r3)),
                ("S5", s4 - 1.168 * (s3 - s4)),
            ]
        }
        // Traditional
        _ => {
            let p = (h + l + c) / 3.0;
            vec![
                ("P", p),
                ("R1", 2.0 * p - l),
                ("S1", 2.0 * p - h),
                ("R2", p + r),
                ("S2", p - r),
                ("R3", h + 2.0 * (p - l)),
                ("S3", l - 2.0 * (h - p)),
                ("R4", p * 3.0 + (h - 3.0 * l)),
                ("S4", p * 3.0 - (3.0 * h - l)),
                ("R5", p * 4.0 + (h - 4.0 * l)),
                ("S5", p * 4.0 - (4.0 * h - l)),
            ]
        }
    }
}

/// The start of the period holding `t` (UTC), and the next one's.
fn period(unit: &str, t: i64) -> (i64, i64) {
    use jiff::{Timestamp, ToSpan, tz::TimeZone};
    let Ok(ts) = Timestamp::from_millisecond(t) else {
        return (t, t + 1);
    };
    let z = ts.to_zoned(TimeZone::UTC);
    let d = z.date();
    let (start, step) = match unit {
        "Weekly" => {
            let back = d.weekday().to_monday_zero_offset() as i64;
            (d.checked_sub(back.days()).unwrap_or(d), 7.days())
        }
        "Monthly" => (d.first_of_month(), 1.month()),
        "Quarterly" => {
            let m = ((d.month() - 1) / 3) * 3 + 1;
            (jiff::civil::date(d.year(), m, 1), 3.months())
        }
        "Yearly" => (jiff::civil::date(d.year(), 1, 1), 1.year()),
        _ => (d, 1.day()),
    };
    let ms = |x: jiff::civil::Date| {
        x.to_zoned(TimeZone::UTC)
            .map(|z| z.timestamp().as_millisecond())
            .unwrap_or(t)
    };
    (ms(start), ms(start.checked_add(step).unwrap_or(start)))
}

#[derive(Clone, Debug)]
struct Period {
    start: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
}

#[derive(Clone)]
struct PivotsStd {
    kind: &'static str,
    unit: String,
    back: usize,
    labels: bool,
    prices: bool,
    left: bool,
    width: f64,
    color: u32,
    cur: Option<Period>,
    /// Periods with their levels (from the period before).
    drawn: Vec<(i64, i64, Levels)>,
}

impl Study for PivotsStd {
    fn step(&mut self, c: &Ctx, _: &mut Out) {
        if self.unit == "Auto" {
            self.unit = match (c.tf.unit, c.tf.millis()) {
                (_, Some(ms)) if ms <= 15 * 60_000 => "Daily",
                (_, Some(ms)) if ms < 86_400_000 => "Weekly",
                (opentern_core::Unit::Day, _) => "Monthly",
                _ => "Yearly",
            }
            .into();
        }
        let b = c.bar;
        let (start, end) = period(&self.unit, b.time);
        match &mut self.cur {
            Some(p) if p.start == start => {
                p.high = p.high.max(b.high);
                p.low = p.low.min(b.low);
                p.close = b.close;
            }
            prev => {
                if let Some(p) = prev.take() {
                    let lv = pivot_levels(self.kind, p.open, p.high, p.low, p.close, b.open);
                    self.drawn.push((start, end, lv));
                    if self.drawn.len() > self.back {
                        self.drawn.remove(0);
                    }
                }
                *prev = Some(Period {
                    start,
                    open: b.open,
                    high: b.high,
                    low: b.low,
                    close: b.close,
                });
            }
        }
    }

    fn drawings(&self) -> Option<Drawings> {
        let mut d = Drawings::default();
        for (start, end, lv) in &self.drawn {
            for (name, y) in lv {
                let mut l = line(*start, *y, *end, *y, self.color, self.width, "none");
                l.xloc = XLoc::BarTime;
                d.lines.push(l);
                if self.labels || self.prices {
                    let text = match (self.labels, self.prices) {
                        (true, true) => format!("{name} ({})", price_text(*y)),
                        (true, false) => name.to_string(),
                        _ => format!("({})", price_text(*y)),
                    };
                    let (x, style) = if self.left {
                        (*start, "label_right")
                    } else {
                        (*end, "label_left")
                    };
                    let mut lb = label(x, *y, text, style, 0xFFFF_FF00, self.color);
                    lb.xloc = XLoc::BarTime;
                    d.labels.push(lb);
                }
            }
        }
        Some(d)
    }
}

// ------------------------------------------------------------ Linear Regression Channel

#[derive(Clone)]
struct LinReg {
    len: usize,
    src: Source,
    up: Option<f64>,
    dn: Option<f64>,
    pearson: bool,
    extend: &'static str,
    upper_col: u32,
    lower_col: u32,
    srcs: Hist,
    highs: Hist,
    lows: Hist,
    out: Option<Drawings>,
}

impl LinReg {
    fn compute(&self, last: i64) -> Option<Drawings> {
        let n = self.len;
        if n <= 1 || self.srcs.get(n - 1).is_nan() {
            return None;
        }
        // calcSlope
        let (mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0);
        for i in 0..n {
            let v = self.srcs.get(i);
            let per = i as f64 + 1.0;
            sx += per;
            sy += v;
            sxx += per * per;
            sxy += v * per;
        }
        let nf = n as f64;
        let slope = (nf * sxy - sx * sy) / (nf * sxx - sx * sx);
        let average = sy / nf;
        let intercept = average - slope * sx / nf + slope;
        let start = intercept + slope * (nf - 1.0);
        let end = intercept;
        // calcDev
        let (mut up, mut dn, mut acc, mut dsxx, mut dsyy, mut dsxy) = (0.0f64, 0.0f64, 0.0, 0.0, 0.0, 0.0);
        let periods = n - 1;
        let day = intercept + slope * periods as f64 / 2.0;
        let mut val = intercept;
        for j in 0..=periods {
            up = up.max(self.highs.get(j) - val);
            dn = dn.max(val - self.lows.get(j));
            let price = self.srcs.get(j);
            let dxt = price - average;
            let dyt = val - day;
            acc += (price - val) * (price - val);
            dsxx += dxt * dxt;
            dsyy += dyt * dyt;
            dsxy += dxt * dyt;
            val += slope;
        }
        let stdev = (acc / if periods == 0 { 1.0 } else { periods as f64 }).sqrt();
        let r = if dsxx == 0.0 || dsyy == 0.0 {
            0.0
        } else {
            dsxy / (dsxx * dsyy).sqrt()
        };
        let up_off = self.up.map(|m| m * stdev).unwrap_or(up);
        let dn_off = self.dn.map(|m| m * stdev).unwrap_or(dn);
        let x1 = last - n as i64 + 1;
        let opaque = |c: u32| c | 0xFF;
        let base = line(x1, start, last, end, opaque(self.lower_col), 1.0, self.extend);
        let upper = line(
            x1,
            start + up_off,
            last,
            end + up_off,
            opaque(self.upper_col),
            1.0,
            self.extend,
        );
        let lower = line(
            x1,
            start - dn_off,
            last,
            end - dn_off,
            opaque(self.lower_col),
            1.0,
            self.extend,
        );
        let mut d = Drawings::default();
        d.linefills.push(Linefill {
            a: upper.clone(),
            b: base.clone(),
            color: self.upper_col,
        });
        d.linefills.push(Linefill {
            a: base.clone(),
            b: lower.clone(),
            color: self.lower_col,
        });
        if self.pearson {
            let text = format!("{r:.16}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string();
            let mut lb = label(
                x1,
                start - dn_off,
                text,
                "label_up",
                0xFFFF_FF00,
                opaque(self.upper_col),
            );
            lb.size = s("normal");
            d.labels.push(lb);
        }
        d.lines.extend([base, upper, lower]);
        Some(d)
    }
}

impl Study for LinReg {
    fn step(&mut self, c: &Ctx, _: &mut Out) {
        self.srcs.push(c.src(self.src));
        self.highs.push(c.bar.high);
        self.lows.push(c.bar.low);
        // Like the original, only the last bar draws.
        self.out = if c.index + 1 == c.count {
            self.compute(c.index as i64)
        } else {
            None
        };
    }

    fn drawings(&self) -> Option<Drawings> {
        self.out.clone()
    }
}

// ------------------------------------------------------------ declarations

fn meta(
    id: &'static str,
    name: &'static str,
    short: &'static str,
    category: Category,
    inputs: Vec<InputSpec>,
    build: fn(&Inputs) -> Box<dyn Study>,
) -> Meta {
    Meta {
        id,
        name,
        short,
        category,
        placement: Placement::Overlay,
        format: Format::Price,
        inputs,
        plots: vec![],
        hlines: vec![],
        fills: vec![],
        build,
    }
}

pub(super) fn metas() -> Vec<Meta> {
    let mut fib_inputs = vec![
        float("dev", "Deviation", 3.0, 0.00001, 100.0, 0.5),
        int("depth", "Depth", 10, 2, 500),
        flag("reverse", "Reverse", false),
        flag("extend_left", "Extend left", false),
        flag("extend_right", "Extend right", true),
        flag("prices", "Prices", true),
        flag("levels", "Levels", true),
        choice("levels_format", "Levels format", "Values", &["Values", "Percent"]),
        choice("labels_pos", "Labels position", "Left", &["Left", "Right"]),
        flag("background", "Background", true),
    ];
    let fib_titles: [&'static str; 11] = [
        "0", "0.236", "0.382", "0.5", "0.618", "0.786", "1", "1.618", "2.618", "3.618", "4.236",
    ];
    for (k, (id, _, _, on)) in FIB_LEVELS.iter().enumerate() {
        fib_inputs.push(flag(id, fib_titles[k], *on).group("Levels"));
    }
    let fib_inputs_ext = fib_inputs.clone();
    vec![
        meta(
            "zigzag",
            "Zig Zag",
            "ZigZag",
            Category::Trend,
            vec![
                float("dev", "Price deviation for reversals (%)", 5.0, 0.00001, 100.0, 0.5),
                int("depth", "Pivot legs", 10, 2, 500),
                colour("color", "Line color", tv::BLUE),
                flag("extend", "Extend to last bar", true),
                flag("show_price", "Display reversal price", true),
                flag("show_vol", "Display cumulative volume", true),
                flag("show_change", "Display reversal price change", true),
                choice("diff", "Difference", "Absolute", &["Absolute", "Percent"]),
            ],
            |i| {
                Box::new(ZigZagStudy {
                    zz: ZigZag::new(i.float("dev"), i.len("depth")),
                    color: colour_of(i, "color", tv::BLUE),
                    extend: i.bool("extend"),
                    show_price: i.bool("show_price"),
                    show_vol: i.bool("show_vol"),
                    show_change: i.bool("show_change"),
                    percent: i.text("diff") == "Percent",
                    last: None,
                })
            },
        ),
        meta(
            "auto_fib",
            "Auto Fib Retracement",
            "Auto Fib",
            Category::Trend,
            fib_inputs,
            |i| {
                Box::new(AutoFib {
                    zz: ZigZag::new(i.float("dev"), i.len("depth")),
                    reverse: i.bool("reverse"),
                    extend_left: i.bool("extend_left"),
                    extend_right: i.bool("extend_right"),
                    prices: i.bool("prices"),
                    levels: i.bool("levels"),
                    percent: i.text("levels_format") == "Percent",
                    left: i.text("labels_pos") == "Left",
                    bg: i.bool("background"),
                    shown: FIB_LEVELS.iter().map(|l| i.bool(l.0)).collect(),
                    last: 0,
                    extension: false,
                })
            },
        ),
        meta(
            "auto_fib_ext",
            "Auto Fib Extension",
            "Auto Fib Ext",
            Category::Trend,
            fib_inputs_ext,
            |i| {
                Box::new(AutoFib {
                    zz: ZigZag::new(i.float("dev"), i.len("depth")),
                    reverse: i.bool("reverse"),
                    extend_left: i.bool("extend_left"),
                    extend_right: i.bool("extend_right"),
                    prices: i.bool("prices"),
                    levels: i.bool("levels"),
                    percent: i.text("levels_format") == "Percent",
                    left: i.text("labels_pos") == "Left",
                    bg: i.bool("background"),
                    shown: FIB_LEVELS.iter().map(|l| i.bool(l.0)).collect(),
                    last: 0,
                    extension: true,
                })
            },
        ),
        meta(
            "pivots_std",
            "Pivot Points Standard",
            "Pivots",
            Category::Trend,
            vec![
                choice("type", "Type", "Traditional", PIVOT_TYPES),
                choice("tf", "Pivots timeframe", "Auto", PIVOT_TFS),
                int("back", "Number of pivots back", 15, 1, 5000),
                flag("labels", "Show labels", true),
                flag("prices", "Show prices", true),
                choice("labels_pos", "Labels position", "Left", &["Left", "Right"]),
                int("width", "Line width", 1, 1, 10),
                colour("color", "Color", tv::AMBER),
            ],
            |i| {
                Box::new(PivotsStd {
                    kind: PIVOT_TYPES
                        .iter()
                        .find(|k| **k == i.text("type"))
                        .copied()
                        .unwrap_or("Traditional"),
                    unit: i.text("tf").to_string(),
                    back: i.len("back"),
                    labels: i.bool("labels"),
                    prices: i.bool("prices"),
                    left: i.text("labels_pos") == "Left",
                    width: i.int("width") as f64,
                    color: colour_of(i, "color", tv::AMBER),
                    cur: None,
                    drawn: Vec::new(),
                })
            },
        ),
        meta(
            "linreg_channel",
            "Linear Regression Channel",
            "LinReg",
            Category::Trend,
            vec![
                int("length", "Length", 100, 1, 5000),
                source("close"),
                flag("use_up", "Upper deviation", true).group("Channel settings"),
                float("up", "Upper multiplier", 2.0, 0.0, 100.0, 0.1).group("Channel settings"),
                flag("use_dn", "Lower deviation", true).group("Channel settings"),
                float("dn", "Lower multiplier", 2.0, 0.0, 100.0, 0.1).group("Channel settings"),
                flag("pearson", "Show Pearson's R", true).group("Display settings"),
                flag("extend_left", "Extend lines left", false).group("Display settings"),
                flag("extend_right", "Extend lines right", true).group("Display settings"),
                colour("upper", "Upper color", Rgba::hex(0x2196F3).transp(85)),
                colour("lower", "Lower color", Rgba::hex(0xFF5252).transp(85)),
            ],
            |i| {
                let n = i.len("length").min(5000);
                Box::new(LinReg {
                    len: n,
                    src: Source::by_name(i.text("source")),
                    up: i.bool("use_up").then(|| i.float("up")),
                    dn: i.bool("use_dn").then(|| i.float("dn")),
                    pearson: i.bool("pearson"),
                    extend: match (i.bool("extend_left"), i.bool("extend_right")) {
                        (true, true) => "both",
                        (true, false) => "left",
                        (false, true) => "right",
                        _ => "none",
                    },
                    upper_col: colour_of(i, "upper", Rgba::hex(0x2196F3).transp(85)),
                    lower_col: colour_of(i, "lower", Rgba::hex(0xFF5252).transp(85)),
                    srcs: Hist::new(n),
                    highs: Hist::new(n),
                    lows: Hist::new(n),
                    out: None,
                })
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentern_core::{Timeframe, Unit};

    fn bars(path: &[f64]) -> Vec<Bar> {
        path.iter()
            .enumerate()
            .map(|(i, &c)| Bar {
                time: 1_700_000_000_000 + i as i64 * 3_600_000,
                open: c,
                high: c + 0.5,
                low: c - 0.5,
                close: c,
                volume: 10.0,
            })
            .collect()
    }

    fn drawn(id: &str, b: &[Bar], tf: Timeframe) -> Drawings {
        let m = by_id(id).unwrap();
        let mut inst = Instance::new(m, &m.default_inputs());
        inst.update(b, &[], tf);
        inst.drawings.map(|d| (*d).clone()).unwrap_or_default()
    }

    #[test]
    fn zigzag_turns_only_past_the_deviation() {
        // Up 100 → 120, down to 105, a 1% wiggle, up to 130.
        let mut p = Vec::new();
        p.extend((0..=20).map(|i| 100.0 + i as f64));
        p.extend((1..=15).map(|i| 120.0 - i as f64));
        p.extend([106.0, 105.5, 106.0, 105.5]);
        p.extend((1..=25).map(|i| 105.0 + i as f64));
        p.extend((1..=10).map(|i| 130.0 - i as f64));
        let d = drawn("zigzag", &bars(&p), Timeframe::new(1, Unit::Hour));
        let tops: Vec<f64> = d.labels.iter().map(|l| l.y).collect();
        // Bar 0 has nothing on its left, so it can't be a pivot; the 1%
        // wiggle at the bottom isn't a turn.
        assert_eq!(tops, vec![120.5, 104.5, 130.5], "{:?}", d.labels);
        assert_eq!(d.labels[0].style.as_ref(), "label_down");
        assert_eq!(d.labels[1].text.as_ref(), "104.5 (-16)\n150");
        // Two legs plus the extension to the last bar.
        assert_eq!(d.lines.len(), 3);
    }

    #[test]
    fn auto_fib_measures_the_last_leg() {
        let mut p: Vec<f64> = (0..=20).map(|i| 100.0 + i as f64).collect();
        p.extend((1..=15).map(|i| 120.0 - i as f64));
        p.extend((1..=8).map(|i| 105.0 + i as f64 * 0.2));
        let d = drawn("auto_fib", &bars(&p), Timeframe::new(1, Unit::Hour));
        // Leg 120.5 → 104.5: 0 at the end, 1 at the start.
        let y = |lvl: f64| d.lines.iter().any(|l| (l.y1 - (104.5 + 16.0 * lvl)).abs() < 1e-9);
        assert!(y(0.0) && y(0.5) && y(1.0) && y(0.618));
        assert!(
            d.labels.iter().any(|l| l.text.as_ref() == "0.618 (114.388)"),
            "{:?}",
            d.labels.iter().map(|l| &l.text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn auto_fib_extension_projects_the_leg_from_the_last_pivot() {
        // Down to 100, up to 120, down to 108, up a bit: pivots A (low),
        // B (high), C (low).
        let mut p: Vec<f64> = (0..10).map(|i| 110.0 - i as f64).collect();
        p.extend((0..=20).map(|i| 100.0 + i as f64));
        p.extend((1..=12).map(|i| 120.0 - i as f64));
        p.extend((1..=8).map(|i| 108.0 + i as f64 * 0.3));
        let d = drawn("auto_fib_ext", &bars(&p), Timeframe::new(1, Unit::Hour));
        let piv = |d: &Drawings| d.lines.iter().filter(|l| l.style.as_ref() == "dashed").count();
        assert_eq!(piv(&d), 2, "two legs drawn");
        let legs: Vec<(f64, f64)> = d
            .lines
            .iter()
            .filter(|l| l.style.as_ref() == "dashed")
            .map(|l| (l.y1, l.y2))
            .collect();
        let (a, b, c) = (legs[0].0, legs[0].1, legs[1].1);
        let y = |lvl: f64| d.lines.iter().any(|l| (l.y1 - (c + (b - a) * lvl)).abs() < 1e-9);
        assert!(y(0.0) && y(0.618) && y(1.0) && y(1.618), "{a} {b} {c}");
    }

    #[test]
    fn pivot_levels_by_kind() {
        let t = pivot_levels("Traditional", 10.0, 12.0, 8.0, 11.0, 11.0);
        let get = |v: &[(&str, f64)], k: &str| v.iter().find(|x| x.0 == k).unwrap().1;
        let p = (12.0 + 8.0 + 11.0) / 3.0;
        assert_eq!(get(&t, "P"), p);
        assert_eq!(get(&t, "R1"), 2.0 * p - 8.0);
        assert_eq!(get(&t, "S3"), 8.0 - 2.0 * (12.0 - p));
        let c = pivot_levels("Camarilla", 10.0, 12.0, 8.0, 11.0, 11.0);
        assert!((get(&c, "R4") - (11.0 + 1.1 * 4.0 / 2.0)).abs() < 1e-12);
        let dm = pivot_levels("DM", 10.0, 12.0, 8.0, 11.0, 11.0);
        assert_eq!(get(&dm, "P"), (2.0 * 12.0 + 8.0 + 11.0) / 4.0);
        // Daily pivots on hourly bars: one set per finished day.
        let b = bars(&(0..24 * 4).map(|i| 100.0 + (i % 24) as f64).collect::<Vec<_>>());
        let m = by_id("pivots_std").unwrap();
        let mut ins = m.default_inputs();
        ins.set("tf", Value::Text("Daily".into()));
        let mut inst = Instance::new(m, &ins);
        inst.update(&b, &[], Timeframe::new(1, Unit::Hour));
        let d = inst.drawings.map(|d| (*d).clone()).unwrap_or_default();
        let days = b
            .iter()
            .map(|x| x.time.div_euclid(86_400_000))
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        assert_eq!(d.lines.len(), (days - 1) * 11);
    }

    #[test]
    fn linreg_channel_fits_a_straight_line() {
        let p: Vec<f64> = (0..150).map(|i| 50.0 + 0.5 * i as f64).collect();
        let d = drawn("linreg_channel", &bars(&p), Timeframe::new(1, Unit::Hour));
        let base = &d.lines[0];
        assert_eq!((base.x1, base.x2), (50, 149));
        assert!((base.y1 - p[50]).abs() < 1e-9 && (base.y2 - p[149]).abs() < 1e-9);
        // A perfect line: no deviation, Pearson's R of 1.
        assert!((d.lines[1].y1 - base.y1).abs() < 1e-9);
        assert_eq!(d.labels[0].text.as_ref(), "1");
        assert_eq!(price_text(1234567.891), "1,234,567.89");
        assert_eq!(price_text(-0.5), "-0.5");
    }
}

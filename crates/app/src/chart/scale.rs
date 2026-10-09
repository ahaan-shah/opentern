//! Scales: nice price steps, and time labels chosen the way TradingView
//! does it, by giving each bar a weight (a new year outranks a new month,
//! which outranks a new day, then hours, then minutes) and placing the
//! heaviest labels first, keeping them apart.

use jiff::tz::TimeZone;
use opentern_core::{Bar, Timeframe};

/// A step of 1, 2, 2.5 or 5 × 10^k that splits `range` into about `n` parts.
pub fn nice_step(range: f64, n: f64) -> f64 {
    if !(range.is_finite() && range > 0.0) {
        return 1.0;
    }
    let raw = range / n.max(1.0);
    let mag = 10f64.powf(raw.log10().floor());
    let f = raw / mag;
    let nice = if f <= 1.0 {
        1.0
    } else if f <= 2.0 {
        2.0
    } else if f <= 2.5 {
        2.5
    } else if f <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * mag
}

/// Tick values within `lo..=hi`, about one per `spacing` pixels.
pub fn price_ticks(lo: f64, hi: f64, height: f32, spacing: f32) -> (Vec<f64>, f64) {
    let n = (height / spacing).max(2.0) as f64;
    let step = nice_step(hi - lo, n);
    let mut v = (lo / step).ceil() * step;
    let mut out = Vec::new();
    // Bounded, so a degenerate range can never spin.
    while v <= hi && out.len() < 200 {
        out.push(v);
        v += step;
    }
    (out, step)
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimeTick {
    pub index: usize,
    pub label: String,
    /// Heavier ticks (years, months) are drawn bolder.
    pub weight: u8,
}

struct Fields {
    year: i16,
    month: i8,
    day: i8,
    hour: i8,
    minute: i8,
    second: i8,
}

fn fields(t: i64, tz: &TimeZone) -> Option<Fields> {
    let z = jiff::Timestamp::from_millisecond(t).ok()?.to_zoned(tz.clone());
    Some(Fields {
        year: z.year(),
        month: z.month(),
        day: z.day(),
        hour: z.hour(),
        minute: z.minute(),
        second: z.second(),
    })
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Weight and label of the boundary between `prev` and `cur`.
fn mark(prev: &Fields, cur: &Fields) -> (u8, String) {
    if cur.year != prev.year {
        return (70, cur.year.to_string());
    }
    if cur.month != prev.month {
        return (60, MONTHS[(cur.month - 1) as usize].to_string());
    }
    if cur.day != prev.day {
        return (50, cur.day.to_string());
    }
    let hm = format!("{:02}:{:02}", cur.hour, cur.minute);
    if cur.hour != prev.hour {
        let w = if cur.hour % 12 == 0 {
            34
        } else if cur.hour % 6 == 0 {
            33
        } else if cur.hour % 3 == 0 {
            32
        } else {
            30
        };
        return (w, hm);
    }
    if cur.minute != prev.minute {
        let w = if cur.minute % 30 == 0 {
            24
        } else if cur.minute % 15 == 0 {
            23
        } else if cur.minute % 5 == 0 {
            22
        } else {
            20
        };
        return (w, hm);
    }
    if cur.second != prev.second {
        return (10, format!("{hm}:{:02}", cur.second));
    }
    (0, String::new())
}

/// Labels for the bars `first..=last`, at least `spacing` pixels apart
/// (`bar_w` pixels per bar).
pub fn time_ticks(
    bars: &[Bar],
    first: usize,
    last: usize,
    bar_w: f32,
    spacing: f32,
    tf: Timeframe,
    tz: &TimeZone,
) -> Vec<TimeTick> {
    if bars.is_empty() || first > last {
        return Vec::new();
    }
    let last = last.min(bars.len() - 1);
    // Daily and longer bars are stamped at UTC midnight of their date.
    let tz = if tf.is_intraday() { tz.clone() } else { TimeZone::UTC };
    // Look at no more than ~2000 candidates: thin out very dense views.
    let stride = ((last - first + 1) / 2000).max(1);
    let mut cands: Vec<(u8, usize, String)> = Vec::new();
    let mut prev = fields(bars[first.saturating_sub(1)].time, &tz);
    let mut i = first;
    while i <= last {
        let cur = fields(bars[i].time, &tz);
        if let (Some(p), Some(c)) = (&prev, &cur) {
            let (w, label) = if i == 0 { (0, String::new()) } else { mark(p, c) };
            if w > 0 {
                cands.push((w, i, label));
            }
        }
        // With a stride, compare against the bar just before the next one
        // looked at, so boundaries aren't missed.
        let next = i + stride;
        prev = if stride > 1 && next <= last {
            fields(bars[next - 1].time, &tz)
        } else {
            cur
        };
        i = next;
    }
    cands.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let min_bars = (spacing / bar_w.max(0.01)).ceil() as usize;
    let mut taken: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    for (w, idx, label) in cands {
        let pos = taken.binary_search(&idx).unwrap_or_else(|p| p);
        let left_ok = pos == 0 || idx - taken[pos - 1] >= min_bars;
        let right_ok = pos == taken.len() || taken[pos] - idx >= min_bars;
        if left_ok && right_ok {
            taken.insert(pos, idx);
            out.push(TimeTick {
                index: idx,
                label,
                weight: w,
            });
        }
    }
    out.sort_by_key(|t| t.index);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_steps() {
        assert_eq!(nice_step(100.0, 10.0), 10.0);
        assert_eq!(nice_step(1.0, 4.0), 0.25);
        assert_eq!(nice_step(0.003, 6.0), 0.0005);
        assert_eq!(nice_step(0.0, 4.0), 1.0);
        let (ticks, step) = price_ticks(101.3, 108.9, 400.0, 50.0);
        assert_eq!(step, 1.0);
        assert_eq!(ticks.first(), Some(&102.0));
        assert_eq!(ticks.last(), Some(&108.0));
        // A degenerate range can't blow up.
        assert!(price_ticks(1.0, 1e12, 400.0, 1.0).0.len() <= 200);
    }

    fn hourly(n: usize, start: i64) -> Vec<Bar> {
        (0..n)
            .map(|i| Bar {
                time: start + i as i64 * 3_600_000,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 1.0,
            })
            .collect()
    }

    #[test]
    fn days_and_months_win_over_hours() {
        // 2026-09-29 00:00 UTC, hourly, across the turn of the month.
        let bars = hourly(24 * 4, 1_790_640_000_000);
        let tf = Timeframe::parse("1h").unwrap();
        let ticks = time_ticks(&bars, 0, bars.len() - 1, 6.0, 80.0, tf, &TimeZone::UTC);
        let labels: Vec<_> = ticks.iter().map(|t| t.label.as_str()).collect();
        assert!(labels.contains(&"Oct"), "{labels:?}");
        assert!(labels.contains(&"30"), "{labels:?}");
        // Labels never crowd: at 6 px per bar, 80 px is 14 bars.
        for w in ticks.windows(2) {
            assert!(w[1].index - w[0].index >= 14);
        }
        // Zoomed in, hours appear between days.
        let ticks = time_ticks(&bars, 0, 30, 30.0, 80.0, tf, &TimeZone::UTC);
        assert!(ticks.iter().any(|t| t.label.contains(':')));
    }
}

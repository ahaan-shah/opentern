//! Volume profiles (TradingView's Visible Range, Session, Fixed Range and
//! Anchored volume profiles), Seasonality and VWAP from an anchor: the
//! numbers only, shared by the chart's built-ins and drawing tools.
//!
//! Without lower-timeframe data, each bar's volume is spread evenly over
//! the price rows its high–low range covers (TradingView uses intrabar
//! data where it has it, so rows can differ a little; see
//! `docs/indicators.md`).

use opentern_core::Bar;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    /// Bottom of the lowest row.
    pub lo: f64,
    /// Row height.
    pub step: f64,
    /// Volume per row on up bars (close ≥ open) and down bars.
    pub up: Vec<f64>,
    pub down: Vec<f64>,
    /// Row with the most volume (point of control).
    pub poc: usize,
    /// Value area: rows `val..=vah` hold `value_area` of the volume.
    pub val: usize,
    pub vah: usize,
}

impl Profile {
    pub fn rows(&self) -> usize {
        self.up.len()
    }

    pub fn total(&self, i: usize) -> f64 {
        self.up[i] + self.down[i]
    }

    pub fn max(&self) -> f64 {
        (0..self.rows()).map(|i| self.total(i)).fold(0.0, f64::max)
    }

    /// The price at the middle of row `i`.
    pub fn mid(&self, i: usize) -> f64 {
        self.lo + (i as f64 + 0.5) * self.step
    }
}

/// The profile of `bars` with `rows` price rows; `value_area` is a share
/// (0.7 for TradingView's 70%).
pub fn volume_profile(bars: &[Bar], rows: usize, value_area: f64) -> Option<Profile> {
    let rows = rows.clamp(1, 1000);
    let lo = bars
        .iter()
        .map(|b| b.low)
        .filter(|v| v.is_finite())
        .fold(f64::INFINITY, f64::min);
    let hi = bars
        .iter()
        .map(|b| b.high)
        .filter(|v| v.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);
    if !lo.is_finite() || !hi.is_finite() {
        return None;
    }
    let span = if hi > lo { hi - lo } else { lo.abs().max(1.0) * 1e-6 };
    let step = span / rows as f64;
    let mut up = vec![0.0; rows];
    let mut down = vec![0.0; rows];
    for b in bars {
        if b.volume.is_nan() || b.volume <= 0.0 || !b.low.is_finite() || !b.high.is_finite() {
            continue;
        }
        let row = |p: f64| (((p - lo) / step).floor() as i64).clamp(0, rows as i64 - 1) as usize;
        let (r0, r1) = (row(b.low), row(b.high));
        let side = if b.close >= b.open { &mut up } else { &mut down };
        let range = b.high - b.low;
        if range <= 0.0 || r0 == r1 {
            side[r0] += b.volume;
            continue;
        }
        // Each row gets the share of the bar's range that falls in it.
        for (r, slot) in side.iter_mut().enumerate().take(r1 + 1).skip(r0) {
            let (a, z) = (lo + r as f64 * step, lo + (r + 1) as f64 * step);
            let overlap = (z.min(b.high) - a.max(b.low)).max(0.0);
            *slot += b.volume * overlap / range;
        }
    }
    let total = |i: usize| up[i] + down[i];
    let poc = (0..rows)
        .max_by(|a, b| total(*a).partial_cmp(&total(*b)).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(0);
    // Value area: grow from the POC, adding whichever neighbour has more
    // volume, until it holds `value_area` of the total (the usual method).
    let sum: f64 = (0..rows).map(total).sum();
    let target = sum * value_area.clamp(0.0, 1.0);
    let (mut val, mut vah) = (poc, poc);
    let mut acc = total(poc);
    while acc < target && (val > 0 || vah + 1 < rows) {
        let below = if val > 0 { total(val - 1) } else { -1.0 };
        let above = if vah + 1 < rows { total(vah + 1) } else { -1.0 };
        if above >= below {
            vah += 1;
            acc += above;
        } else {
            val -= 1;
            acc += below;
        }
    }
    Some(Profile {
        lo,
        step,
        up,
        down,
        poc,
        val,
        vah,
    })
}

/// VWAP of hlc3 from bar `from` on: one value per bar from `from`.
pub fn anchored_vwap(bars: &[Bar], from: usize) -> Vec<f64> {
    let mut out = Vec::with_capacity(bars.len().saturating_sub(from));
    let (mut pv, mut v) = (0.0, 0.0);
    for b in bars.iter().skip(from) {
        let hlc3 = (b.high + b.low + b.close) / 3.0;
        pv += hlc3 * b.volume;
        v += b.volume;
        out.push(if v > 0.0 { pv / v } else { hlc3 });
    }
    out
}

/// One year of Seasonality: the percent change from the year's first
/// close, by fraction of the year (0 = 1 January, 1 = 31 December).
#[derive(Clone, Debug, PartialEq)]
pub struct SeasonYear {
    pub year: i16,
    pub points: Vec<(f64, f64)>,
}

/// The last `years` calendar years (the current one included), from daily
/// or longer bars (intraday bars use each day's last close).
pub fn seasonality(bars: &[Bar], tz: &jiff::tz::TimeZone, years: usize) -> Vec<SeasonYear> {
    let mut out: Vec<SeasonYear> = Vec::new();
    // The close before the year began is the base, as TradingView does
    // when it has it; else the year's first close.
    let mut prev_close: Option<f64> = None;
    let mut base: Option<f64> = None;
    let mut last_day: Option<jiff::civil::Date> = None;
    for b in bars {
        let Ok(ts) = jiff::Timestamp::from_millisecond(b.time) else {
            continue;
        };
        let z = ts.to_zoned(tz.clone());
        let d = z.date();
        let y = d.year();
        if out.last().is_none_or(|s| s.year != y) {
            base = prev_close.or(Some(b.open));
            out.push(SeasonYear {
                year: y,
                points: Vec::new(),
            });
        }
        let len = if d.in_leap_year() { 366.0 } else { 365.0 };
        let frac = (d.day_of_year() as f64 - 1.0) / (len - 1.0);
        let pct = match base {
            Some(b0) if b0 != 0.0 => (b.close / b0 - 1.0) * 100.0,
            _ => 0.0,
        };
        let s = out.last_mut().expect("pushed");
        if last_day == Some(d) {
            if let Some(p) = s.points.last_mut() {
                *p = (frac, pct);
            }
        } else {
            s.points.push((frac, pct));
        }
        last_day = Some(d);
        prev_close = Some(b.close);
    }
    let skip = out.len().saturating_sub(years.max(1));
    out.drain(..skip);
    out
}

/// One year of monthly returns: each month's close against the month
/// before's (in percent; NaN where there's no month), and the year's close
/// against the previous year's.
#[derive(Clone, Debug, PartialEq)]
pub struct MonthRow {
    pub year: i16,
    pub months: [f64; 12],
    pub year_pct: f64,
}

/// Monthly returns from any bars (daily, monthly, …), grouped by calendar
/// month in `tz`, newest year first. A month's base is the previous
/// month's last close; the very first month, which has none, uses its
/// first open.
pub fn monthly_returns(bars: &[Bar], tz: &jiff::tz::TimeZone) -> Vec<MonthRow> {
    // (year, month) → (first open, last close), in time order.
    let mut months: Vec<((i16, i8), f64, f64)> = Vec::new();
    for b in bars {
        let Ok(ts) = jiff::Timestamp::from_millisecond(b.time) else {
            continue;
        };
        let d = ts.to_zoned(tz.clone()).date();
        let key = (d.year(), d.month());
        match months.last_mut() {
            Some((k, _, c)) if *k == key => *c = b.close,
            _ => months.push((key, b.open, b.close)),
        }
    }
    let mut rows: Vec<MonthRow> = Vec::new();
    let mut prev: Option<f64> = None;
    let mut year_base: Option<f64> = None;
    for &((y, m), open, close) in &months {
        if rows.last().is_none_or(|r| r.year != y) {
            year_base = prev.or(Some(open));
            rows.push(MonthRow {
                year: y,
                months: [f64::NAN; 12],
                year_pct: f64::NAN,
            });
        }
        let base = prev.unwrap_or(open);
        let r = rows.last_mut().expect("pushed");
        if base != 0.0 {
            r.months[(m - 1) as usize] = (close / base - 1.0) * 100.0;
        }
        if let Some(b0) = year_base.filter(|b| *b != 0.0) {
            r.year_pct = (close / b0 - 1.0) * 100.0;
        }
        prev = Some(close);
    }
    rows.reverse();
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(t: i64, o: f64, h: f64, l: f64, c: f64, v: f64) -> Bar {
        Bar {
            time: t,
            open: o,
            high: h,
            low: l,
            close: c,
            volume: v,
        }
    }

    #[test]
    fn volume_spreads_over_rows_and_sums_up() {
        let bars = vec![
            bar(0, 10.0, 20.0, 10.0, 15.0, 100.0),
            bar(1, 15.0, 15.0, 12.0, 12.0, 30.0),
        ];
        let p = volume_profile(&bars, 10, 0.7).unwrap();
        let total: f64 = (0..p.rows()).map(|i| p.total(i)).sum();
        assert!((total - 130.0).abs() < 1e-9);
        // The up bar covers every row equally; the down bar rows 2..=4.
        assert!((p.up[0] - 10.0).abs() < 1e-9);
        assert!(p.down[3] > 0.0 && p.down[6] == 0.0);
        assert!(p.poc >= 2 && p.poc <= 4);
        assert!(p.val <= p.poc && p.vah >= p.poc);
        let va: f64 = (p.val..=p.vah).map(|i| p.total(i)).sum();
        assert!(va >= 0.7 * 130.0);
    }

    #[test]
    fn monthly_returns_chain_month_closes() {
        let day = 86_400_000i64;
        // 2024-01-01 is 19723 days after the epoch.
        let t0 = 19_723 * day;
        let bars = vec![
            bar(t0, 100.0, 0.0, 0.0, 110.0, 0.0),            // Jan 1
            bar(t0 + 20 * day, 110.0, 0.0, 0.0, 120.0, 0.0), // Jan 21
            bar(t0 + 40 * day, 120.0, 0.0, 0.0, 90.0, 0.0),  // Feb 10
            bar(t0 + 370 * day, 90.0, 0.0, 0.0, 99.0, 0.0),  // 2025
        ];
        let rows = monthly_returns(&bars, &jiff::tz::TimeZone::UTC);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].year, 2025);
        assert!((rows[0].months[0] - 10.0).abs() < 1e-9);
        assert!((rows[0].year_pct - 10.0).abs() < 1e-9);
        let y24 = &rows[1];
        assert!(
            (y24.months[0] - 20.0).abs() < 1e-9,
            "the first month runs from its open"
        );
        assert!((y24.months[1] + 25.0).abs() < 1e-9);
        assert!(y24.months[2].is_nan());
        assert!((y24.year_pct + 10.0).abs() < 1e-9);
    }

    #[test]
    fn vwap_from_an_anchor() {
        let bars = vec![
            bar(0, 1.0, 3.0, 0.0, 3.0, 1.0),
            bar(1, 3.0, 6.0, 3.0, 6.0, 1.0),
            bar(2, 6.0, 9.0, 6.0, 6.0, 2.0),
        ];
        let v = anchored_vwap(&bars, 1);
        assert_eq!(v.len(), 2);
        assert!((v[0] - 5.0).abs() < 1e-9);
        assert!((v[1] - (5.0 + 2.0 * 7.0) / 3.0).abs() < 1e-9);
    }

    #[test]
    fn seasonality_starts_each_year_at_zero_from_the_prior_close() {
        let day = 86_400_000i64;
        // 2023-12-31 then three days of 2024.
        let t0 = jiff::civil::date(2023, 12, 31)
            .to_zoned(jiff::tz::TimeZone::UTC)
            .unwrap()
            .timestamp()
            .as_millisecond();
        let bars: Vec<Bar> = [100.0, 110.0, 99.0, 121.0]
            .iter()
            .enumerate()
            .map(|(i, c)| bar(t0 + i as i64 * day, *c, *c, *c, *c, 1.0))
            .collect();
        let s = seasonality(&bars, &jiff::tz::TimeZone::UTC, 2);
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].year, 2024);
        assert!((s[1].points[0].1 - 10.0).abs() < 1e-9);
        assert!((s[1].points[2].1 - 21.0).abs() < 1e-9);
        assert_eq!(s[1].points[0].0, 0.0);
    }
}

//! Building one interval from another: 7m from 1m, 45m from 15m, 2D from
//! 1D. Intraday buckets restart at the first bar of each local day (00:00
//! for crypto, the session open for stocks), as TradingView does. Daily and
//! longer bars are stamped at 00:00 UTC of their exchange-local date, so
//! their calendar maths happens in UTC.

use jiff::civil::Date;
use jiff::tz::TimeZone;
use opentern_core::{Bar, Timeframe, Unit};

const DAY: i64 = 86_400_000;

/// Folds `b` into `a` (a is earlier).
pub fn merge(a: &Bar, b: &Bar) -> Bar {
    Bar {
        time: a.time,
        open: a.open,
        high: a.high.max(b.high),
        low: a.low.min(b.low),
        close: b.close,
        volume: a.volume + b.volume,
    }
}

/// The local date of a timestamp in `tz`, as days since 1970-01-01.
fn local_day(t: i64, tz: &TimeZone) -> i64 {
    let Ok(ts) = jiff::Timestamp::from_millisecond(t) else {
        return t.div_euclid(DAY);
    };
    let d = ts.to_zoned(tz.clone()).date();
    days_from_date(d)
}

fn days_from_date(d: Date) -> i64 {
    let epoch = Date::constant(1970, 1, 1);
    epoch.until(d).map(|s| s.get_days() as i64).unwrap_or(0)
}

fn date_from_days(days: i64) -> Date {
    Date::constant(1970, 1, 1)
        .checked_add(jiff::Span::new().days(days))
        .unwrap_or(Date::constant(1970, 1, 1))
}

/// Bucket start for a daily-or-longer target, given a bar stamped at UTC
/// midnight.
fn calendar_bucket(t: i64, tf: Timeframe) -> i64 {
    let day = t.div_euclid(DAY);
    let n = tf.count.max(1) as i64;
    match tf.unit {
        Unit::Day => day.div_euclid(n) * n * DAY,
        Unit::Week => {
            // 1970-01-01 was a Thursday; weeks start on Monday.
            let monday = day - (day + 3).rem_euclid(7);
            let week = (monday + 3).div_euclid(7);
            let start = week.div_euclid(n) * n;
            (start * 7 - 3) * DAY
        }
        Unit::Month => {
            let d = date_from_days(day);
            let m = d.year() as i64 * 12 + (d.month() as i64 - 1);
            let m = m.div_euclid(n) * n;
            let first = Date::constant((m / 12) as i16, (m % 12 + 1) as i8, 1);
            days_from_date(first) * DAY
        }
        _ => t,
    }
}

/// Resamples `base` (oldest first) into `tf`. `tz` is the exchange
/// timezone, used for the start of each intraday day.
pub fn resample(base: &[Bar], tf: Timeframe, tz: &TimeZone) -> Vec<Bar> {
    let mut out: Vec<Bar> = Vec::new();
    let len = tf.millis();
    let mut day = i64::MIN;
    let mut anchor = 0;
    for b in base {
        let key = if tf.is_intraday() {
            let d = local_day(b.time, tz);
            if d != day {
                day = d;
                anchor = b.time;
            }
            let len = len.unwrap_or(60_000);
            anchor + (b.time - anchor).div_euclid(len) * len
        } else {
            calendar_bucket(b.time, tf)
        };
        match out.last_mut() {
            Some(last) if last.time == key => *last = merge(last, b),
            _ => out.push(Bar { time: key, ..*b }),
        }
    }
    out
}

/// Recomputes just the last target bar from the tail of `base`: enough to
/// follow a live stream without resampling everything on each tick.
pub fn resample_last(base: &[Bar], tf: Timeframe, tz: &TimeZone) -> Option<Bar> {
    let last = base.last()?;
    // For intraday targets, the bucket depends on the first bar of the
    // local day; for longer ones a bucket never spans more than a few
    // hundred base bars.
    let start = if tf.is_intraday() {
        let d = local_day(last.time, tz);
        let mut i = base.len() - 1;
        while i > 0 && local_day(base[i - 1].time, tz) == d {
            i -= 1;
        }
        i
    } else {
        let key = calendar_bucket(last.time, tf);
        let mut i = base.len() - 1;
        while i > 0 && calendar_bucket(base[i - 1].time, tf) == key {
            i -= 1;
        }
        i
    };
    resample(&base[start..], tf, tz).pop()
}

/// The interval to fetch from a provider to build `tf`: `tf` itself if the
/// provider has it, else the longest native interval that divides it.
pub fn choose_base(tf: Timeframe, natives: &[Timeframe]) -> Option<Timeframe> {
    if natives.contains(&tf) {
        return Some(tf);
    }
    let fits = |b: &Timeframe| match (tf.unit, b.unit) {
        (Unit::Month, Unit::Month) => tf.count.is_multiple_of(b.count),
        (Unit::Month, _) => false,
        (Unit::Week, Unit::Week) => tf.count.is_multiple_of(b.count),
        (Unit::Week | Unit::Day, Unit::Day) => {
            let days = if tf.unit == Unit::Week { tf.count * 7 } else { tf.count };
            days % b.count == 0
        }
        (Unit::Day, _) | (Unit::Week, _) => false,
        _ => match (tf.millis(), b.millis()) {
            (Some(t), Some(m)) => b.is_intraday() && t % m == 0,
            _ => false,
        },
    };
    natives
        .iter()
        .filter(|b| fits(b))
        .max_by_key(|b| b.millis().unwrap_or(i64::MAX))
        .copied()
}

/// Stamps a daily-or-longer bar at 00:00 UTC of its local date in `tz`.
pub fn to_utc_midnight(t: i64, tz: &TimeZone) -> i64 {
    local_day(t, tz) * DAY
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(time: i64, o: f64, h: f64, l: f64, c: f64, v: f64) -> Bar {
        Bar {
            time,
            open: o,
            high: h,
            low: l,
            close: c,
            volume: v,
        }
    }

    fn tf(s: &str) -> Timeframe {
        Timeframe::parse(s).unwrap()
    }

    #[test]
    fn minutes_into_seven_minute_bars_restarting_each_day() {
        // 1440 one-minute bars from midnight UTC, then the next day's first.
        let mut base: Vec<Bar> = (0..1441).map(|i| bar(i * 60_000, 1.0, 2.0, 0.5, 1.5, 1.0)).collect();
        base[3].high = 9.0;
        let out = resample(&base, tf("7m"), &TimeZone::UTC);
        assert_eq!(out[0].time, 0);
        assert_eq!(out[0].high, 9.0);
        assert_eq!(out[0].volume, 7.0);
        assert_eq!(out[1].time, 7 * 60_000);
        // 1440 / 7 = 205 full bars plus a 5-minute one, then the new day.
        assert_eq!(out[205].volume, 5.0);
        assert_eq!(out[206].time, DAY);
        assert_eq!(out.len(), 207);
    }

    #[test]
    fn stock_hours_start_at_the_session_open() {
        // 30-minute bars from 09:30 New York (14:30 UTC in winter).
        let tz = TimeZone::get("America/New_York").unwrap();
        let open = 1_704_810_600_000; // 2024-01-09 14:30 UTC
        let base: Vec<Bar> = (0..13)
            .map(|i| bar(open + i * 1_800_000, 1.0, 1.0, 1.0, 1.0, 1.0))
            .collect();
        let out = resample(&base, tf("1h"), &tz);
        assert_eq!(out[0].time, open);
        assert_eq!(out[1].time, open + 3_600_000);
        assert_eq!(out.len(), 7);
        assert_eq!(out[6].volume, 1.0);
    }

    #[test]
    fn weeks_start_on_monday_and_months_on_the_first() {
        // 2024-01-01 was a Monday.
        let jan1 = days_from_date(Date::constant(2024, 1, 1)) * DAY;
        let days: Vec<Bar> = (0..40).map(|i| bar(jan1 + i * DAY, 1.0, 1.0, 1.0, 1.0, 1.0)).collect();
        let weeks = resample(&days, tf("1W"), &TimeZone::UTC);
        assert_eq!(weeks[0].time, jan1);
        assert_eq!(weeks[1].time, jan1 + 7 * DAY);
        assert_eq!(weeks[0].volume, 7.0);
        let months = resample(&days, tf("1M"), &TimeZone::UTC);
        assert_eq!(months.len(), 2);
        assert_eq!(months[0].volume, 31.0);
        assert_eq!(months[1].time, jan1 + 31 * DAY);
    }

    #[test]
    fn last_bar_matches_full_resample() {
        let base: Vec<Bar> = (0..3000)
            .map(|i| bar(i * 60_000, i as f64, i as f64 + 1.0, i as f64 - 1.0, i as f64, 1.0))
            .collect();
        for t in ["7m", "45m", "4h"] {
            let full = resample(&base, tf(t), &TimeZone::UTC);
            assert_eq!(resample_last(&base, tf(t), &TimeZone::UTC).as_ref(), full.last(), "{t}");
        }
    }

    #[test]
    fn picks_the_longest_dividing_native_interval() {
        let natives: Vec<Timeframe> = ["1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "1D", "1W", "1M"]
            .into_iter()
            .map(tf)
            .collect();
        assert_eq!(choose_base(tf("45m"), &natives), Some(tf("15m")));
        assert_eq!(choose_base(tf("7m"), &natives), Some(tf("1m")));
        assert_eq!(choose_base(tf("3h"), &natives), Some(tf("1h")));
        assert_eq!(choose_base(tf("2D"), &natives), Some(tf("1D")));
        assert_eq!(choose_base(tf("2W"), &natives), Some(tf("1W")));
        assert_eq!(choose_base(tf("3M"), &natives), Some(tf("1M")));
        assert_eq!(choose_base(tf("4h"), &natives), Some(tf("4h")));
    }
}

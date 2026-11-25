//! When bars really open and close. Daily and longer bars are stamped at
//! 00:00 UTC of their local date; scripts see their session's open and
//! close, as on TradingView.

use crate::calendar::{self, Market};
use crate::{Timeframe, Unit};

pub const DAY: i64 = 86_400_000;

pub fn date_of(day: i64) -> jiff::civil::Date {
    jiff::civil::date(1970, 1, 1)
        .checked_add(jiff::Span::new().days(day.clamp(-700_000, 2_900_000)))
        .unwrap_or(jiff::civil::date(1970, 1, 1))
}

pub fn days_to(d: jiff::civil::Date) -> i64 {
    jiff::civil::date(1970, 1, 1)
        .until(d)
        .map_or(0, |s| s.get_days() as i64)
}

/// Local midnight of a day, as UTC ms.
pub fn midnight(day: i64, tz: &jiff::tz::TimeZone) -> i64 {
    date_of(day)
        .to_zoned(tz.clone())
        .map_or(day * DAY, |z| z.timestamp().as_millisecond())
}

/// The local date of a `tf` bar stamped `t`, as days since 1970.
pub fn day_of(tf: Timeframe, t: i64, tz: &jiff::tz::TimeZone) -> i64 {
    if !tf.is_intraday() {
        return t.div_euclid(DAY);
    }
    local_day(t, tz)
}

pub fn local_day(t: i64, tz: &jiff::tz::TimeZone) -> i64 {
    match jiff::Timestamp::from_millisecond(t) {
        Ok(ts) => days_to(ts.to_zoned(tz.clone()).date()),
        Err(_) => t.div_euclid(DAY),
    }
}

/// The local days a daily-or-longer bar stamped `t` covers.
fn days_in(tf: Timeframe, t: i64) -> (i64, i64) {
    let first = t.div_euclid(DAY);
    let n = tf.count.max(1) as i64;
    let last = match tf.unit {
        Unit::Week => first + 7 * n,
        Unit::Month => days_to(
            date_of(first)
                .checked_add(jiff::Span::new().months(n))
                .unwrap_or(date_of(first)),
        ),
        _ => first + n,
    };
    (first, last)
}

/// When a `tf` bar stamped `t` opens.
pub fn open_in(tf: Timeframe, t: i64, tz: &jiff::tz::TimeZone, market: Market) -> i64 {
    if tf.is_intraday() || t.rem_euclid(DAY) != 0 {
        return t;
    }
    let (first, last) = days_in(tf, t);
    for d in first..last.min(first + 12) {
        if let Some((open, _)) = session_on(market, d) {
            return open;
        }
    }
    midnight(first, tz)
}

/// When a `tf` bar stamped `t` closes: no later than the session it's in,
/// and for daily and longer bars, with their last session (a stock's "D"
/// bar ends at 16:00, not midnight).
pub fn close_in(tf: Timeframe, t: i64, tz: &jiff::tz::TimeZone, market: Market) -> i64 {
    if let Some(ms) = tf.millis().filter(|_| tf.is_intraday()) {
        let end = t + ms;
        return match session_on(market, local_day_in(market, t)) {
            Some((_, close)) if close > t && close < end => close,
            _ => end,
        };
    }
    let (first, last) = days_in(tf, t);
    let (start, end) = (midnight(first, tz), midnight(last, tz));
    remember(&CLOSES, (market, start, end), || {
        calendar::last_close_in(market, start, end)
    })
    .unwrap_or(end)
}

type Remembered<K, T> = std::cell::RefCell<std::collections::HashMap<K, T>>;
type Memo<K, T> = std::thread::LocalKey<Remembered<K, T>>;

thread_local! {
    static SESSIONS: Remembered<(Market, i64), Option<(i64, i64)>> = Default::default();
    static CLOSES: Remembered<(Market, i64, i64), Option<i64>> = Default::default();
}

/// Calendar lookups build their year's holidays each time, and scripts ask
/// on every bar.
fn remember<K: std::hash::Hash + Eq, T: Copy>(memo: &'static Memo<K, T>, k: K, f: impl FnOnce() -> T) -> T {
    memo.with(|m| {
        if let Some(v) = m.borrow().get(&k) {
            return *v;
        }
        let v = f();
        let mut m = m.borrow_mut();
        if m.len() > 20_000 {
            m.clear();
        }
        m.insert(k, v);
        v
    })
}

/// A local day's session, if the market has set hours and trades.
pub fn session_on(market: Market, day: i64) -> Option<(i64, i64)> {
    if !matches!(market, Market::Us | Market::India | Market::Exchange(_)) {
        return None;
    }
    remember(&SESSIONS, (market, day), || {
        calendar::session_times(market, date_of(day))
    })
}

pub fn local_day_in(market: Market, t: i64) -> i64 {
    match market {
        Market::Us | Market::India | Market::Exchange(_) => local_day(t, &market.tz()),
        _ => t.div_euclid(DAY),
    }
}

/// The market from an exchange prefix and a `syminfo.type`-style kind.
pub fn market_of(prefix: &str, kind: &str) -> Market {
    use crate::AssetKind;
    let kind = match kind {
        "crypto" => AssetKind::Crypto,
        "forex" => AssetKind::Forex,
        "fund" => AssetKind::Fund,
        "index" => AssetKind::Index,
        "futures" => AssetKind::Futures,
        _ => AssetKind::Stock,
    };
    Market::from_parts(prefix, kind)
}

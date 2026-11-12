//! When markets trade: regular sessions, weekends, holidays and early
//! closes. US holidays follow the NYSE's published rules (so they're right
//! for any year); Indian holidays are lunar and announced each year, so
//! they're listed (update `NSE_HOLIDAYS` when the exchange publishes the
//! next year's). Sessions outside the usual days or hours (Diwali's
//! Muhurat trading, special Saturday sessions) are in `NSE_SPECIAL`: add
//! each as the exchange announces it: Yahoo's bars skip some, and the app
//! builds their bars from its quote while they run (`opentern_data::yahoo`).
//! Forex trades Sunday 17:00 to Friday 17:00 New York time; crypto never
//! closes.

use crate::{AssetKind, Symbol};
use jiff::civil::{Date, Time, Weekday, date};
use jiff::tz::TimeZone;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Market {
    Crypto,
    Forex,
    /// NYSE, NASDAQ, AMEX: 09:30–16:00 New York.
    Us,
    /// NSE, BSE: 09:15–15:30 India.
    India,
    /// Anything else: assumed open on weekdays.
    Other,
}

impl Market {
    pub fn of(sym: &Symbol) -> Market {
        Market::from_parts(sym.exchange(), sym.kind)
    }

    /// From an exchange prefix ("NASDAQ") and the asset kind.
    pub fn from_parts(exchange: &str, kind: AssetKind) -> Market {
        match exchange {
            "NASDAQ" | "NYSE" | "AMEX" => Market::Us,
            "NSE" | "BSE" => Market::India,
            "FX" | "OANDA" => Market::Forex,
            // Commodity futures trade nearly round the clock, Sunday evening
            // to Friday evening New York, like forex.
            "COMEX" | "NYMEX" | "CBOT" | "CME" | "ICEUS" | "ICEEUR" => Market::Forex,
            "BINANCE" | "BYBIT" | "OKX" | "COINBASE" | "KRAKEN" => Market::Crypto,
            _ => match kind {
                AssetKind::Crypto => Market::Crypto,
                AssetKind::Forex => Market::Forex,
                _ => Market::Other,
            },
        }
    }

    pub fn tz(self) -> TimeZone {
        let name = match self {
            Market::Us | Market::Forex => "America/New_York",
            Market::India => "Asia/Kolkata",
            Market::Crypto | Market::Other => "UTC",
        };
        TimeZone::get(name).unwrap_or(TimeZone::UTC)
    }

    /// Regular hours (local), if the market has fixed ones.
    fn hours(self) -> Option<(Time, Time)> {
        match self {
            Market::Us => Some((Time::constant(9, 30, 0, 0), Time::constant(16, 0, 0, 0))),
            Market::India => Some((Time::constant(9, 15, 0, 0), Time::constant(15, 30, 0, 0))),
            _ => None,
        }
    }
}

/// Easter Sunday (Gregorian), by the anonymous computus.
fn easter(y: i16) -> Date {
    let y = y as i32;
    let (a, b, c) = (y % 19, y / 100, y % 100);
    let (d, e) = (b / 4, b % 4);
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let (i, k) = (c / 4, c % 4);
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = (h + l - 7 * m + 114) % 31 + 1;
    date(y as i16, month as i8, day as i8)
}

/// The `n`th `wd` of a month (n = -1 for the last).
fn nth(y: i16, m: i8, wd: Weekday, n: i32) -> Date {
    if n > 0 {
        let first = date(y, m, 1);
        let shift = (wd.to_monday_zero_offset() as i32 - first.weekday().to_monday_zero_offset() as i32).rem_euclid(7);
        date(y, m, (1 + shift + 7 * (n - 1)) as i8)
    } else {
        let last = date(y, m, 1).last_of_month();
        let back = (last.weekday().to_monday_zero_offset() as i32 - wd.to_monday_zero_offset() as i32).rem_euclid(7);
        date(y, m, (last.day() as i32 - back) as i8)
    }
}

/// A fixed-date holiday moved off a weekend: Saturday → Friday, Sunday →
/// Monday.
fn observed(d: Date) -> Date {
    match d.weekday() {
        Weekday::Saturday => d.yesterday().unwrap_or(d),
        Weekday::Sunday => d.tomorrow().unwrap_or(d),
        _ => d,
    }
}

/// NYSE holidays of a year, by rule.
pub fn us_holidays(y: i16) -> Vec<(Date, &'static str)> {
    let mut v = Vec::new();
    // New Year's Day: a Saturday holiday isn't moved back into the old year.
    let ny = date(y, 1, 1);
    match ny.weekday() {
        Weekday::Saturday => {}
        _ => v.push((observed(ny), "New Year's Day")),
    }
    v.push((nth(y, 1, Weekday::Monday, 3), "Martin Luther King Jr. Day"));
    v.push((nth(y, 2, Weekday::Monday, 3), "Washington's Birthday"));
    if let Ok(gf) = easter(y).checked_sub(jiff::Span::new().days(2)) {
        v.push((gf, "Good Friday"));
    }
    v.push((nth(y, 5, Weekday::Monday, -1), "Memorial Day"));
    if y >= 2022 {
        v.push((observed(date(y, 6, 19)), "Juneteenth"));
    }
    v.push((observed(date(y, 7, 4)), "Independence Day"));
    v.push((nth(y, 9, Weekday::Monday, 1), "Labor Day"));
    v.push((nth(y, 11, Weekday::Thursday, 4), "Thanksgiving Day"));
    v.push((observed(date(y, 12, 25)), "Christmas Day"));
    v
}

/// NYSE early closes (13:00 New York) of a year.
fn us_early_closes(y: i16) -> Vec<Date> {
    let mut v = Vec::new();
    let weekday = |d: Date| {
        matches!(
            d.weekday(),
            Weekday::Monday | Weekday::Tuesday | Weekday::Wednesday | Weekday::Thursday
        )
    };
    let jul3 = date(y, 7, 3);
    if weekday(jul3) {
        v.push(jul3);
    }
    if let Ok(friday) = nth(y, 11, Weekday::Thursday, 4).tomorrow() {
        v.push(friday);
    }
    let dec24 = date(y, 12, 24);
    if weekday(dec24) {
        v.push(dec24);
    }
    v
}

/// NSE/BSE trading holidays as announced (weekday ones; weekends are
/// closed anyway). To verify against the exchange's circular each year.
const NSE_HOLIDAYS: &[(i16, i8, i8, &str)] = &[
    (2025, 2, 26, "Mahashivratri"),
    (2025, 3, 14, "Holi"),
    (2025, 3, 31, "Id-Ul-Fitr"),
    (2025, 4, 10, "Shri Mahavir Jayanti"),
    (2025, 4, 14, "Dr. Baba Saheb Ambedkar Jayanti"),
    (2025, 4, 18, "Good Friday"),
    (2025, 5, 1, "Maharashtra Day"),
    (2025, 8, 15, "Independence Day"),
    (2025, 8, 27, "Ganesh Chaturthi"),
    (2025, 10, 2, "Mahatma Gandhi Jayanti / Dussehra"),
    (2025, 10, 21, "Diwali Laxmi Pujan"),
    (2025, 10, 22, "Diwali Balipratipada"),
    (2025, 11, 5, "Prakash Gurpurb Sri Guru Nanak Dev"),
    (2025, 12, 25, "Christmas"),
    // 2026, from NSE's December 2025 list (as reported by brokers; the
    // 15 January closure for the Maharashtra municipal elections came in a
    // later circular).
    (2026, 1, 15, "Municipal Corporation Elections (Maharashtra)"),
    (2026, 1, 26, "Republic Day"),
    (2026, 3, 3, "Holi"),
    (2026, 3, 26, "Shri Ram Navami"),
    (2026, 3, 31, "Shri Mahavir Jayanti"),
    (2026, 4, 3, "Good Friday"),
    (2026, 4, 14, "Dr. Baba Saheb Ambedkar Jayanti"),
    (2026, 5, 1, "Maharashtra Day"),
    (2026, 5, 28, "Bakri Id"),
    (2026, 6, 26, "Muharram"),
    (2026, 9, 14, "Ganesh Chaturthi"),
    (2026, 10, 2, "Mahatma Gandhi Jayanti"),
    (2026, 10, 20, "Dussehra"),
    (2026, 11, 10, "Diwali Balipratipada"),
    (2026, 11, 24, "Prakash Gurpurb Sri Guru Nanak Dev"),
    (2026, 12, 25, "Christmas"),
];

/// Hours and minutes, local time.
type Hm = (i8, i8);

/// A session NSE and BSE hold outside their usual days or hours.
struct Extra {
    date: (i16, i8, i8),
    /// Trading windows (local open, close) of the normal market, after the
    /// pre-open; a closing auction may run a little later. Empty when the
    /// times aren't on record (old sessions: only the day's prices are
    /// used, and the regular hours stand in).
    windows: &'static [(Hm, Hm)],
    name: &'static str,
}

const MUHURAT: &str = "Muhurat trading";
const SATURDAY: &str = "Special session";

/// Diwali's Muhurat trading and special Saturday sessions (budget days,
/// disaster-recovery drills). Yahoo's daily bars lack many of them
/// (checked 10 Nov 2026: 2010–2016, 2020-02-01, 2023-11-12, the three 2024
/// drills, 2026-11-08). Every date was confirmed by hand in NSE's own
/// end-of-day file (10 Nov 2026); times from the exchanges' circulars and
/// their reporting. Add each new one when the exchange announces it.
const NSE_SPECIAL: &[Extra] = &[
    Extra {
        date: (2010, 11, 5),
        windows: &[],
        name: MUHURAT,
    },
    Extra {
        date: (2011, 10, 26),
        windows: &[],
        name: MUHURAT,
    },
    Extra {
        date: (2012, 11, 13),
        windows: &[],
        name: MUHURAT,
    },
    Extra {
        date: (2013, 11, 3),
        windows: &[],
        name: MUHURAT,
    },
    Extra {
        date: (2014, 10, 23),
        windows: &[],
        name: MUHURAT,
    },
    Extra {
        date: (2015, 2, 28),
        windows: &[((9, 15), (15, 30))],
        name: "Union Budget session",
    },
    Extra {
        date: (2015, 11, 11),
        windows: &[],
        name: MUHURAT,
    },
    Extra {
        date: (2016, 10, 30),
        windows: &[],
        name: MUHURAT,
    },
    Extra {
        date: (2017, 10, 19),
        windows: &[],
        name: MUHURAT,
    },
    Extra {
        date: (2018, 11, 7),
        windows: &[],
        name: MUHURAT,
    },
    Extra {
        date: (2019, 10, 27),
        windows: &[],
        name: MUHURAT,
    },
    Extra {
        date: (2020, 2, 1),
        windows: &[((9, 15), (15, 30))],
        name: "Union Budget session",
    },
    Extra {
        date: (2020, 11, 14),
        windows: &[],
        name: MUHURAT,
    },
    Extra {
        date: (2021, 11, 4),
        windows: &[],
        name: MUHURAT,
    },
    Extra {
        date: (2022, 10, 24),
        windows: &[((18, 15), (19, 15))],
        name: MUHURAT,
    },
    Extra {
        date: (2023, 11, 12),
        windows: &[((18, 15), (19, 15))],
        name: MUHURAT,
    },
    Extra {
        date: (2024, 1, 20),
        windows: &[((9, 15), (10, 0)), ((11, 30), (12, 30))],
        name: SATURDAY,
    },
    Extra {
        date: (2024, 3, 2),
        windows: &[((9, 15), (10, 0)), ((11, 30), (12, 30))],
        name: SATURDAY,
    },
    Extra {
        date: (2024, 5, 18),
        windows: &[((9, 15), (10, 0)), ((11, 45), (12, 40))],
        name: SATURDAY,
    },
    Extra {
        date: (2024, 11, 1),
        windows: &[((18, 0), (19, 0))],
        name: MUHURAT,
    },
    Extra {
        date: (2025, 2, 1),
        windows: &[((9, 15), (15, 30))],
        name: "Union Budget session",
    },
    Extra {
        date: (2025, 10, 21),
        windows: &[((13, 45), (14, 45))],
        name: MUHURAT,
    },
    Extra {
        date: (2026, 11, 8),
        windows: &[((14, 20), (15, 30))],
        name: MUHURAT,
    },
];

fn extra(m: Market, d: Date) -> Option<&'static Extra> {
    if m != Market::India {
        return None;
    }
    NSE_SPECIAL.iter().find(|e| date(e.date.0, e.date.1, e.date.2) == d)
}

fn hm((h, m): Hm) -> Time {
    Time::constant(h, m, 0, 0)
}

/// The special session on local date `d`: first open, last close (the
/// regular hours when its times aren't on record) and its name.
pub fn special(m: Market, d: Date) -> Option<(Time, Time, &'static str)> {
    let e = extra(m, d)?;
    let (open, close) = match (e.windows.first(), e.windows.last()) {
        (Some(first), Some(last)) => (hm(first.0), hm(last.1)),
        _ => m.hours()?,
    };
    Some((open, close, e.name))
}

/// Every special session: local date, first open and last close (UTC ms).
pub fn special_sessions(m: Market) -> Vec<(Date, i64, i64)> {
    if m != Market::India {
        return Vec::new();
    }
    NSE_SPECIAL
        .iter()
        .filter_map(|e| {
            let d = date(e.date.0, e.date.1, e.date.2);
            let (o, c) = session(m, d)?;
            Some((d, o, c))
        })
        .collect()
}

/// A special session's trading windows on `d` (UTC ms), when it has more
/// than one (a break between them).
fn windows(m: Market, d: Date) -> Vec<(i64, i64)> {
    let Some(e) = extra(m, d) else { return Vec::new() };
    let tz = m.tz();
    let at = |t: Time| {
        d.to_datetime(t)
            .to_zoned(tz.clone())
            .ok()
            .map(|z| z.timestamp().as_millisecond())
    };
    e.windows
        .iter()
        .filter_map(|(o, c)| Some((at(hm(*o))?, at(hm(*c))?)))
        .collect()
}

/// Why the market is closed on a local date, if it's a holiday.
pub fn holiday(m: Market, d: Date) -> Option<&'static str> {
    match m {
        Market::Us => us_holidays(d.year()).into_iter().find(|(h, _)| *h == d).map(|(_, n)| n),
        Market::India => NSE_HOLIDAYS
            .iter()
            .find(|(y, mo, da, _)| date(*y, *mo, *da) == d)
            .map(|(_, _, _, n)| *n),
        _ => None,
    }
}

/// A day the market doesn't keep its usual hours.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Special {
    /// Closed all day for this holiday.
    Holiday(String),
    /// Closes early at this exchange-local time.
    EarlyClose(Time),
    /// A special session (Muhurat trading, a Saturday session): its name,
    /// open and close, local time.
    Session(String, Time, Time),
}

/// Holidays and early closes from `from` for `days` days, for the
/// Calendar widget.
pub fn upcoming(m: Market, from: Date, days: i64) -> Vec<(Date, Special)> {
    let mut out = Vec::new();
    let mut d = from;
    for _ in 0..days.max(0) {
        if let Some((open, close, name)) = special(m, d) {
            out.push((d, Special::Session(name.to_string(), open, close)));
        } else if let Some(name) = holiday(m, d) {
            out.push((d, Special::Holiday(name.to_string())));
        } else if m == Market::Us && us_early_closes(d.year()).contains(&d) {
            out.push((d, Special::EarlyClose(Time::constant(13, 0, 0, 0))));
        }
        match d.tomorrow() {
            Ok(n) => d = n,
            Err(_) => break,
        }
    }
    out
}

/// The market's state at time `t` (ms).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Open,
    /// Closed, opening at this time (ms), if known.
    Closed(Option<i64>),
}

/// Today's (local date `d`) regular session as UTC ms, if it trades.
fn session(m: Market, d: Date) -> Option<(i64, i64)> {
    let (mut open, mut close) = m.hours()?;
    if let Some((o, c, _)) = special(m, d) {
        (open, close) = (o, c);
    } else if matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday) || holiday(m, d).is_some() {
        return None;
    }
    if m == Market::Us && us_early_closes(d.year()).contains(&d) {
        close = Time::constant(13, 0, 0, 0);
    }
    let tz = m.tz();
    let at = |t: Time| {
        d.to_datetime(t)
            .to_zoned(tz.clone())
            .ok()
            .map(|z| z.timestamp().as_millisecond())
    };
    Some((at(open)?, at(close)?))
}

/// When a daily-or-longer period really ends: the close of its last
/// trading session, for markets with fixed hours (a stock's "D" bar ends
/// at 16:00, not midnight). `start..end` is the calendar period; `None`
/// when the market has no fixed hours or nothing trades in the period.
pub fn last_close_in(m: Market, start: i64, end: i64) -> Option<i64> {
    m.hours()?;
    let tz = m.tz();
    let day = |t: i64| {
        jiff::Timestamp::from_millisecond(t)
            .ok()
            .map(|ts| ts.to_zoned(tz.clone()).date())
    };
    let (first, mut d) = (day(start)?, day(end - 1)?);
    for _ in 0..400 {
        if let Some((_, close)) = session(m, d)
            && close > start
            && close <= end
        {
            return Some(close);
        }
        if d <= first {
            return None;
        }
        d = d.yesterday().ok()?;
    }
    None
}

pub fn state(m: Market, t: i64) -> State {
    let Ok(ts) = jiff::Timestamp::from_millisecond(t) else {
        return State::Open;
    };
    let tz = m.tz();
    let z = ts.to_zoned(tz.clone());
    match m {
        Market::Crypto => State::Open,
        Market::Other => {
            if matches!(z.weekday(), Weekday::Saturday | Weekday::Sunday) {
                State::Closed(None)
            } else {
                State::Open
            }
        }
        Market::Forex => {
            // Sunday 17:00 → Friday 17:00 New York.
            let mins = z.hour() as i32 * 60 + z.minute() as i32;
            let closed = match z.weekday() {
                Weekday::Saturday => true,
                Weekday::Sunday => mins < 17 * 60,
                Weekday::Friday => mins >= 17 * 60,
                _ => false,
            };
            if !closed {
                return State::Open;
            }
            let mut d = z.date();
            while d.weekday() != Weekday::Sunday {
                d = d.tomorrow().unwrap_or(d);
            }
            let open = d
                .to_datetime(Time::constant(17, 0, 0, 0))
                .to_zoned(tz)
                .ok()
                .map(|z| z.timestamp().as_millisecond());
            State::Closed(open)
        }
        Market::Us | Market::India => {
            let d = z.date();
            if let Some((o, c)) = session(m, d)
                && t >= o
                && t < c
            {
                // A break between a special session's windows.
                let w = windows(m, d);
                if w.len() > 1 && !w.iter().any(|(a, b)| t >= *a && t < *b) {
                    return State::Closed(w.iter().map(|(a, _)| *a).find(|a| *a > t));
                }
                return State::Open;
            }
            // The next session that hasn't ended.
            let mut day = d;
            for _ in 0..14 {
                if let Some((o, _)) = session(m, day)
                    && o > t
                {
                    return State::Closed(Some(o));
                }
                day = day.tomorrow().unwrap_or(day);
            }
            State::Closed(None)
        }
    }
}

/// US stocks' extended hours: pre-market and after-hours trading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extended {
    /// 04:00 New York until the open.
    Pre,
    /// The close until four hours after it (20:00, or 17:00 after an
    /// early close).
    Post,
}

impl Extended {
    pub fn label(self) -> &'static str {
        match self {
            Extended::Pre => "Pre-market",
            Extended::Post => "After hours",
        }
    }
}

/// Whether US pre-market or after-hours trading is on at `t` (ms). Only on
/// days with a regular session.
/// US stocks and funds: they trade before the open and after the close.
pub fn has_extended_hours(s: &crate::Symbol) -> bool {
    Market::of(s) == Market::Us && matches!(s.kind, crate::AssetKind::Stock | crate::AssetKind::Fund)
}

/// Whether `t` is the moment the market's session ends that day.
pub fn closes_at(m: Market, t: i64) -> bool {
    let Ok(ts) = jiff::Timestamp::from_millisecond(t) else {
        return false;
    };
    session(m, ts.to_zoned(m.tz()).date()).is_some_and(|(_, close)| close == t)
}

pub fn us_extended(t: i64) -> Option<Extended> {
    let ts = jiff::Timestamp::from_millisecond(t).ok()?;
    let d = ts.to_zoned(Market::Us.tz()).date();
    let (open, close) = session(Market::Us, d)?;
    let pre = d
        .to_datetime(Time::constant(4, 0, 0, 0))
        .to_zoned(Market::Us.tz())
        .ok()?
        .timestamp()
        .as_millisecond();
    if t >= pre && t < open {
        Some(Extended::Pre)
    } else if t >= close && t < close + 4 * 3_600_000 {
        Some(Extended::Post)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn us_extended_hours() {
        // Thursday 12 Nov 2026 (New York is UTC-5).
        assert_eq!(us_extended(ms("2026-11-12T08:59:00Z")), None);
        assert_eq!(us_extended(ms("2026-11-12T09:00:00Z")), Some(Extended::Pre));
        assert_eq!(us_extended(ms("2026-11-12T14:29:00Z")), Some(Extended::Pre));
        assert_eq!(us_extended(ms("2026-11-12T14:30:00Z")), None);
        assert_eq!(us_extended(ms("2026-11-12T21:00:00Z")), Some(Extended::Post));
        assert_eq!(us_extended(ms("2026-11-13T01:00:00Z")), None);
        // Day after Thanksgiving: closes 13:00, after hours to 17:00.
        assert_eq!(us_extended(ms("2026-11-27T21:59:00Z")), Some(Extended::Post));
        assert_eq!(us_extended(ms("2026-11-27T22:00:00Z")), None);
        // Saturday and Thanksgiving itself.
        assert_eq!(us_extended(ms("2026-11-14T13:00:00Z")), None);
        assert_eq!(us_extended(ms("2026-11-26T13:00:00Z")), None);
    }

    fn ms(s: &str) -> i64 {
        s.parse::<jiff::Timestamp>().unwrap().as_millisecond()
    }

    #[test]
    fn periods_end_at_the_last_session_close() {
        let day = |d: &str| ms(&format!("{d}T04:00:00Z"));
        // A Monday: 16:00 New York (20:00 UTC in October).
        assert_eq!(
            last_close_in(Market::Us, day("2026-10-12"), day("2026-10-13")),
            Some(ms("2026-10-12T20:00:00Z"))
        );
        // The week of 12 Oct 2026 ends Friday's close.
        assert_eq!(
            last_close_in(Market::Us, day("2026-10-12"), day("2026-10-19")),
            Some(ms("2026-10-16T20:00:00Z"))
        );
        // Day after Thanksgiving closes at 13:00.
        assert_eq!(
            last_close_in(Market::Us, day("2026-11-27"), day("2026-11-28")),
            Some(ms("2026-11-27T18:00:00Z"))
        );
        // Weekends and crypto: nothing to say.
        assert_eq!(last_close_in(Market::Us, day("2026-10-17"), day("2026-10-18")), None);
        assert_eq!(
            last_close_in(Market::Crypto, day("2026-10-12"), day("2026-10-13")),
            None
        );
    }

    #[test]
    fn nyse_holidays_by_rule() {
        let names = |y| {
            us_holidays(y)
                .into_iter()
                .map(|(d, _)| d.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(2025),
            vec![
                "2025-01-01",
                "2025-01-20",
                "2025-02-17",
                "2025-04-18",
                "2025-05-26",
                "2025-06-19",
                "2025-07-04",
                "2025-09-01",
                "2025-11-27",
                "2025-12-25"
            ]
        );
        // 2026: July 4 is a Saturday, observed Friday the 3rd; Good Friday April 3.
        let h26 = names(2026);
        assert!(h26.contains(&"2026-07-03".to_string()) && h26.contains(&"2026-04-03".to_string()));
        // 2022: New Year's Day fell on a Saturday: no holiday moved into 2021.
        assert!(!names(2022).iter().any(|d| d.starts_with("2021")));
        assert_eq!(easter(2024), date(2024, 3, 31));
        assert_eq!(
            us_early_closes(2025),
            vec![date(2025, 7, 3), date(2025, 11, 28), date(2025, 12, 24)]
        );
    }

    #[test]
    fn special_sessions_open_and_close() {
        // Muhurat trading, Sunday 8 Nov 2026, 14:20–15:30 IST.
        assert_eq!(state(Market::India, ms("2026-11-08T09:00:00Z")), State::Open);
        assert_eq!(
            state(Market::India, ms("2026-11-08T08:00:00Z")),
            State::Closed(Some(ms("2026-11-08T08:50:00Z")))
        );
        assert!(matches!(
            state(Market::India, ms("2026-11-08T10:05:00Z")),
            State::Closed(_)
        ));
        // A drill Saturday's break between its two windows.
        assert_eq!(
            state(Market::India, ms("2024-01-20T05:00:00Z")),
            State::Closed(Some(ms("2024-01-20T06:00:00Z")))
        );
        assert_eq!(state(Market::India, ms("2024-01-20T06:30:00Z")), State::Open);
        // The daily bar of a Sunday session ends at its close.
        assert_eq!(
            last_close_in(Market::India, ms("2026-11-08T00:00:00Z"), ms("2026-11-09T00:00:00Z")),
            Some(ms("2026-11-08T10:00:00Z"))
        );
        let up = upcoming(Market::India, date(2026, 11, 1), 10);
        assert!(up.contains(&(
            date(2026, 11, 8),
            Special::Session(MUHURAT.into(), hm((14, 20)), hm((15, 30)))
        )));
        assert!(special(Market::Us, date(2026, 11, 8)).is_none());
    }

    /// New York time on a date as UTC ms.
    fn ny(d: Date, h: i8, m: i8) -> i64 {
        d.at(h, m, 0, 0)
            .to_zoned(Market::Us.tz())
            .unwrap()
            .timestamp()
            .as_millisecond()
    }

    #[test]
    fn every_us_session_opens_at_0930_and_closes_at_1600_new_york_time() {
        // Every weekday 2025-2027, across each clock change: the open and
        // close are New York times whatever the UTC offset.
        let early: Vec<Date> = (2025..=2027).flat_map(us_early_closes).collect();
        let mut d = date(2025, 1, 1);
        let mut sessions = 0;
        while d < date(2028, 1, 1) {
            let weekend = matches!(
                d.weekday(),
                jiff::civil::Weekday::Saturday | jiff::civil::Weekday::Sunday
            );
            if !weekend && holiday(Market::Us, d).is_none() {
                let (open, close) = (
                    ny(d, 9, 30),
                    if early.contains(&d) { ny(d, 13, 0) } else { ny(d, 16, 0) },
                );
                assert!(
                    matches!(state(Market::Us, open - 60_000), State::Closed(_)),
                    "{d} before the open"
                );
                assert_eq!(state(Market::Us, open), State::Open, "{d} at the open");
                assert_eq!(state(Market::Us, close - 60_000), State::Open, "{d} before the close");
                assert!(matches!(state(Market::Us, close), State::Closed(_)), "{d} at the close");
                assert!(closes_at(Market::Us, close), "{d} close");
                let midnight = ny(d, 0, 0);
                assert_eq!(
                    last_close_in(Market::Us, midnight, midnight + 86_400_000),
                    Some(close),
                    "{d}"
                );
                assert_eq!(us_extended(open - 60_000), Some(Extended::Pre), "{d} pre-market");
                assert_eq!(us_extended(close), Some(Extended::Post), "{d} after hours");
                sessions += 1;
            }
            d = d.tomorrow().unwrap();
        }
        // 252 or so a year.
        assert!((740..=770).contains(&sessions), "{sessions} sessions");
    }

    #[test]
    fn the_weeks_the_clocks_change() {
        // US clocks went forward Sunday 8 Mar 2026 and back Sunday 1 Nov 2026.
        // Friday before (EST): 14:30-21:00 UTC; Monday after (EDT): 13:30-20:00.
        assert_eq!(state(Market::Us, ms("2026-03-06T14:30:00Z")), State::Open);
        assert_eq!(
            state(Market::Us, ms("2026-03-06T21:00:00Z")),
            State::Closed(Some(ms("2026-03-09T13:30:00Z")))
        );
        assert_eq!(state(Market::Us, ms("2026-03-09T19:59:00Z")), State::Open);
        assert!(matches!(
            state(Market::Us, ms("2026-03-09T20:00:00Z")),
            State::Closed(_)
        ));
        // And back: Friday 30 Oct at EDT, Monday 2 Nov at EST.
        assert_eq!(
            state(Market::Us, ms("2026-10-30T20:00:00Z")),
            State::Closed(Some(ms("2026-11-02T14:30:00Z")))
        );
        assert!(matches!(
            state(Market::Us, ms("2026-11-02T14:00:00Z")),
            State::Closed(_)
        ));
        assert_eq!(us_extended(ms("2026-11-02T09:00:00Z")), Some(Extended::Pre));
        assert_eq!(us_extended(ms("2026-11-02T08:30:00Z")), None);
        // Forex opens Sunday 17:00 New York: 21:00 UTC on the 8 March change day.
        assert_eq!(
            state(Market::Forex, ms("2026-03-08T20:00:00Z")),
            State::Closed(Some(ms("2026-03-08T21:00:00Z")))
        );
        assert_eq!(
            state(Market::Forex, ms("2026-11-01T21:30:00Z")),
            State::Closed(Some(ms("2026-11-01T22:00:00Z")))
        );
        // India has no clock changes: 09:15-15:30 IST = 03:45-10:00 UTC all year.
        for d in ["2026-03-09", "2026-11-02", "2026-07-01"] {
            assert_eq!(state(Market::India, ms(&format!("{d}T03:45:00Z"))), State::Open, "{d}");
            assert!(matches!(
                state(Market::India, ms(&format!("{d}T10:00:00Z"))),
                State::Closed(_)
            ));
        }
    }

    #[test]
    fn states() {
        // Thursday 2025-07-03, 12:30 New York: open (early close at 13:00).
        assert_eq!(state(Market::Us, ms("2025-07-03T16:30:00Z")), State::Open);
        // 13:30: closed, opens Monday the 7th (the 4th is a holiday).
        assert_eq!(
            state(Market::Us, ms("2025-07-03T17:30:00Z")),
            State::Closed(Some(ms("2025-07-07T13:30:00Z")))
        );
        // India: 10:00 IST on a Monday is open; Holi (14 March 2025) isn't.
        assert_eq!(state(Market::India, ms("2025-03-10T04:30:00Z")), State::Open);
        assert!(matches!(
            state(Market::India, ms("2025-03-14T04:30:00Z")),
            State::Closed(_)
        ));
        // Forex: Saturday is closed, reopening Sunday 17:00 New York.
        assert_eq!(
            state(Market::Forex, ms("2025-07-05T12:00:00Z")),
            State::Closed(Some(ms("2025-07-06T21:00:00Z")))
        );
        assert_eq!(state(Market::Forex, ms("2025-07-07T12:00:00Z")), State::Open);
        assert_eq!(state(Market::Crypto, ms("2025-07-05T12:00:00Z")), State::Open);
    }
}

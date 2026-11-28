//! The effective federal funds rate (EFFR), from the Federal Reserve Bank
//! of New York's markets API (`markets.newyorkfed.org`): free, no key.
//!
//! One value per business day, published about 9:00 a.m. New York time for
//! the previous business day and revised about 2:30 p.m. when it moves by
//! more than a basis point. Every request goes through one memo per run, so
//! however many charts, watchlists and alerts ask, the New York Fed hears
//! from us a few times a day: the whole history once, then the last few
//! days after each publication time.
//!
//! Their terms (newyorkfed.org/privacy/termsofuse, updated 9 Jun 2023)
//! allow automated access that doesn't burden the site, keeping the data,
//! and using it for personal or business purposes, with a notice and a
//! disclaimer wherever it's shown (`NOTICE`). FRED is not used: its terms
//! forbid caching its data.

use crate::provider::{Error, Live, Provider, Result, get_json, now_ms};
use jiff::Timestamp;
use jiff::civil::{Date, Time, Weekday};
use jiff::tz::TimeZone;
use opentern_core::{Bar, Symbol, Timeframe};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Our id for the rate.
pub const ID: &str = opentern_core::FED_FUNDS;

const API: &str = "https://markets.newyorkfed.org/api/rates/unsecured/effr";
/// The first day the API has.
const FIRST_DAY: &str = "2000-07-01";
const DAY: i64 = 86_400_000;

/// The credit and disclaimer their terms ask for, wherever the rate shows.
pub const NOTICE: &str = "Source: Federal Reserve Bank of New York. The Effective Federal Funds Rate is \
     subject to the Terms of Use posted at newyorkfed.org. The New York Fed is not responsible for its \
     publication here and does not endorse Opentern, which is not affiliated with it.";

/// What a chart says when a reply has no rates in it, or can't be read.
pub const NO_RATES: &str = "The New York Fed sent no rates.";
pub const DIDNT_LOAD: &str = "The New York Fed's rates didn't load.";

/// One published day.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Day {
    /// UTC midnight of the business day the rate is for (as daily bars).
    pub time: i64,
    /// Percent.
    pub rate: f64,
    /// The FOMC's target range (from, to), percent. Before 16 Dec 2008 the
    /// Fed set one target, so both ends are the same.
    pub target: Option<(f64, f64)>,
}

/// A reply's days, oldest first. Anything malformed is skipped.
pub fn parse(v: &serde_json::Value) -> Vec<Day> {
    let mut days: Vec<Day> = v
        .get("refRates")
        .and_then(|r| r.as_array())
        .map(|list| {
            list.iter()
                .filter(|r| r.get("type").and_then(|t| t.as_str()).is_none_or(|t| t == "EFFR"))
                .filter_map(|r| {
                    let date: Date = r.get("effectiveDate")?.as_str()?.parse().ok()?;
                    let time = date.to_zoned(TimeZone::UTC).ok()?.timestamp().as_millisecond();
                    let rate = r.get("percentRate")?.as_f64().filter(|x| sane(*x))?;
                    let num = |k: &str| r.get(k).and_then(|x| x.as_f64()).filter(|x| sane(*x));
                    let target = match (num("targetRateFrom"), num("targetRateTo")) {
                        (Some(a), Some(b)) => Some((a.min(b), a.max(b))),
                        (Some(a), None) | (None, Some(a)) => Some((a, a)),
                        (None, None) => None,
                    };
                    Some(Day { time, rate, target })
                })
                .collect()
        })
        .unwrap_or_default();
    days.sort_by_key(|d| d.time);
    // A day listed twice: the later entry (a revision) wins.
    days.reverse();
    days.dedup_by_key(|d| d.time);
    days.reverse();
    days
}

/// A plausible percentage (the rate has been negative nowhere near this).
fn sane(x: f64) -> bool {
    x.is_finite() && (-50.0..=100.0).contains(&x)
}

/// Daily bars: open, high, low and close all the day's rate.
pub fn bars(days: &[Day]) -> Vec<Bar> {
    days.iter()
        .map(|d| Bar {
            time: d.time,
            open: d.rate,
            high: d.rate,
            low: d.rate,
            close: d.rate,
            volume: 0.0,
        })
        .collect()
}

/// Puts `fresh` into `days` (both oldest first): new days are added and
/// days already there take the fresh value (revisions).
fn merge(days: &mut Vec<Day>, fresh: &[Day]) {
    for f in fresh {
        match days.binary_search_by_key(&f.time, |d| d.time) {
            Ok(i) => days[i] = *f,
            Err(i) => days.insert(i, *f),
        }
    }
}

fn ny() -> TimeZone {
    TimeZone::get("America/New_York").unwrap_or(TimeZone::UTC)
}

fn weekday(d: Date) -> bool {
    !matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday)
}

/// When to ask again after asking at `now` (ms), given the newest day we
/// have: the next publication (9:05 New York) or revision (14:35) time on a
/// weekday. Between the two, if the previous business day still hasn't
/// come out (a late publication, or a Fed holiday), every 15 minutes.
pub fn next_check(now: i64, newest: Option<i64>) -> i64 {
    let tz = ny();
    let Ok(ts) = Timestamp::from_millisecond(now) else {
        return now + 3_600_000;
    };
    let z = ts.to_zoned(tz.clone());
    let at = |d: Date, h: i8, m: i8| {
        d.to_datetime(Time::constant(h, m, 0, 0))
            .to_zoned(tz.clone())
            .map(|z| z.timestamp().as_millisecond())
            .unwrap_or(now + 3_600_000)
    };
    let today = z.date();
    let (publish, revise) = (at(today, 9, 5), at(today, 14, 35));
    if weekday(today) && now >= publish && now < revise {
        let mut prev = today.yesterday().unwrap_or(today);
        while !weekday(prev) {
            prev = prev.yesterday().unwrap_or(prev);
        }
        let want = prev
            .to_zoned(TimeZone::UTC)
            .map(|z| z.timestamp().as_millisecond())
            .unwrap_or(0);
        if newest.is_none_or(|n| n < want) {
            return (now + 15 * 60_000).min(revise);
        }
    }
    let mut d = today;
    for _ in 0..8 {
        if weekday(d) {
            for t in [at(d, 9, 5), at(d, 14, 35)] {
                if t > now {
                    return t;
                }
            }
        }
        d = d.tomorrow().unwrap_or(d);
    }
    now + DAY
}

/// What one run of the app knows.
#[derive(Default)]
struct Memo {
    days: Arc<Vec<Day>>,
    /// The whole history has been fetched.
    full: bool,
    /// When to ask the New York Fed again (ms).
    next: i64,
    /// The last failure, served until `next` when there's nothing else.
    error: Option<String>,
}

static MEMO: OnceLock<Mutex<Memo>> = OnceLock::new();

fn memo() -> &'static Mutex<Memo> {
    MEMO.get_or_init(Default::default)
}

/// Every published day, oldest first: from the memo, asking the New York
/// Fed only when a new value can be out.
pub fn days() -> Result<Arc<Vec<Day>>> {
    let mut m = memo().lock().unwrap_or_else(|e| e.into_inner());
    let now = now_ms();
    if now < m.next {
        return match (&m.error, m.days.is_empty()) {
            (Some(e), true) => Err(Error::Bad(e.clone())),
            _ => Ok(m.days.clone()),
        };
    }
    let agent = crate::provider::agent();
    let url = if m.full {
        format!("{API}/last/10.json")
    } else {
        let today = Timestamp::from_millisecond(now)
            .map(|t| t.to_zoned(ny()).date())
            .unwrap_or(Date::constant(2026, 1, 1));
        format!("{API}/search.json?startDate={FIRST_DAY}&endDate={today}")
    };
    match get_json(&agent, &url) {
        Ok(v) => {
            let fresh = parse(&v);
            if fresh.is_empty() {
                m.error = Some(NO_RATES.into());
                m.next = now + 10 * 60_000;
            } else {
                let mut days = (*m.days).clone();
                merge(&mut days, &fresh);
                m.days = Arc::new(days);
                m.full = true;
                m.error = None;
                m.next = next_check(now, m.days.last().map(|d| d.time));
            }
        }
        Err(e) => {
            m.error = Some(match e {
                Error::Bad(_) => DIDNT_LOAD.into(),
                e => e.to_string(),
            });
            // Not every 30 seconds while it's down.
            m.next = now + 5 * 60_000;
        }
    }
    match (&m.error, m.days.is_empty()) {
        (Some(e), true) => Err(Error::Bad(e.clone())),
        _ => Ok(m.days.clone()),
    }
}

/// The target range on each day the memo has, for the chart's band: empty
/// until the rate has loaded once this run (it never fetches).
pub fn targets() -> Arc<Vec<Day>> {
    if crate::hub::DEMO.load(std::sync::atomic::Ordering::Relaxed) {
        static DEMO_DAYS: OnceLock<Arc<Vec<Day>>> = OnceLock::new();
        return DEMO_DAYS.get_or_init(|| Arc::new(demo_days())).clone();
    }
    memo().lock().map(|m| m.days.clone()).unwrap_or_default()
}

/// A made-up history for demo mode: steps a quarter point at a time, with
/// the rate a little inside its range. No network.
pub fn demo_days() -> Vec<Day> {
    let now = now_ms();
    let end = now - now.rem_euclid(DAY);
    let mut days = Vec::new();
    let mut lo = 4.0_f64;
    let mut t = end - 1500 * DAY;
    let mut k = 0u32;
    while t <= end - DAY {
        let wd = (t / DAY + 3).rem_euclid(7); // 0 = Monday
        if wd < 5 {
            k += 1;
            // A change every ~70 business days: cuts, then a pause, then hikes.
            if k.is_multiple_of(70) {
                lo += match (k / 70) % 6 {
                    0..=2 => -0.25,
                    3 => 0.0,
                    _ => 0.25,
                };
                lo = lo.max(0.0);
            }
            // Mostly flat, as the real rate is, with the odd basis point.
            let wobble = if k.is_multiple_of(23) { 0.09 } else { 0.08 };
            days.push(Day {
                time: t,
                rate: ((lo + wobble) * 100.0).round() / 100.0,
                target: Some((lo, lo + 0.25)),
            });
        }
        t += DAY;
    }
    days
}

/// The rate as a provider. Daily bars, weeks and months built from them
/// (open = the first day's rate, close = the last's); nothing intraday.
pub struct NyFed {
    demo: bool,
}

impl NyFed {
    pub fn new(demo: bool) -> NyFed {
        NyFed { demo }
    }

    fn all(&self, tf: Timeframe) -> Result<Vec<Bar>> {
        let daily = if self.demo { bars(&demo_days()) } else { bars(&days()?) };
        Ok(match Timeframe::parse("1D") {
            Some(d) if tf == d => daily,
            _ => crate::resample::resample(&daily, tf, &TimeZone::UTC),
        })
    }
}

/// What a chart says when asked for an intraday interval.
pub const DAILY_ONLY: &str = "The Fed funds rate is published once a day. Pick 1D or longer.";

impl Provider for NyFed {
    fn natives(&self) -> Vec<Timeframe> {
        ["1D", "1W", "1M"].iter().filter_map(|t| Timeframe::parse(t)).collect()
    }

    fn latest(&self, _sym: &Symbol, tf: Timeframe, n: usize) -> Result<Vec<Bar>> {
        let all = self.all(tf)?;
        Ok(all[all.len().saturating_sub(n)..].to_vec())
    }

    fn since(&self, _sym: &Symbol, tf: Timeframe, start: i64) -> Result<Vec<Bar>> {
        // A week back as well: a revised day is replaced in the chart.
        Ok(self
            .all(tf)?
            .into_iter()
            .filter(|b| b.time >= start - 7 * DAY)
            .collect())
    }

    fn before(&self, _sym: &Symbol, tf: Timeframe, end: i64, n: usize) -> Result<Vec<Bar>> {
        let all: Vec<Bar> = self.all(tf)?.into_iter().filter(|b| b.time < end).collect();
        Ok(all[all.len().saturating_sub(n)..].to_vec())
    }

    fn stream(&self, sym: &Symbol, tf: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()> {
        // The memo decides when the network is asked; looking at it is free.
        loop {
            if let Some(b) = self.latest(sym, tf, 1)?.pop()
                && !sink(Live::Bar(b))
            {
                return Ok(());
            }
            for _ in 0..120 {
                std::thread::sleep(Duration::from_millis(500));
                if !sink(Live::Idle) {
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../tests/fixtures/nyfed-effr.json");

    fn ms(s: &str) -> i64 {
        s.parse::<Timestamp>().unwrap().as_millisecond()
    }

    #[test]
    fn parses_a_recorded_reply() {
        let days = parse(&serde_json::from_str(SAMPLE).unwrap());
        assert_eq!(days.len(), 10);
        assert!(days.windows(2).all(|w| w[0].time < w[1].time), "oldest first");
        let last = days.last().unwrap();
        assert_eq!(last.time, ms("2026-11-24T00:00:00Z"));
        assert_eq!(last.rate, 3.88);
        assert_eq!(last.target, Some((3.75, 4.0)));
        // Before the range (16 Dec 2008): one target.
        let first = days[0];
        assert_eq!(first.time, ms("2000-07-03T00:00:00Z"));
        assert_eq!((first.rate, first.target), (7.03, Some((6.5, 6.5))));
        let dec15 = days.iter().find(|d| d.time == ms("2008-12-15T00:00:00Z")).unwrap();
        let dec16 = days.iter().find(|d| d.time == ms("2008-12-16T00:00:00Z")).unwrap();
        assert_eq!(dec15.target, Some((1.0, 1.0)));
        assert_eq!(dec16.target, Some((0.0, 0.25)));
        let b = bars(&days);
        assert!(
            b.iter()
                .all(|b| b.open == b.close && b.high == b.close && b.low == b.close)
        );
    }

    #[test]
    fn junk_is_skipped_and_revisions_win() {
        let v = serde_json::json!({"refRates": [
            {"effectiveDate": "2026-11-20", "type": "EFFR", "percentRate": 3.88},
            {"effectiveDate": "2026-11-20", "type": "EFFR", "percentRate": 3.90, "revisionIndicator": "r"},
            {"effectiveDate": "not a date", "type": "EFFR", "percentRate": 3.0},
            {"effectiveDate": "2026-11-19", "type": "EFFR", "percentRate": "3.88"},
            {"effectiveDate": "2026-11-18", "type": "OBFR", "percentRate": 3.87},
            {"effectiveDate": "2026-11-17", "type": "EFFR", "percentRate": 1e9},
        ]});
        let days = parse(&v);
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].rate, 3.90);
        assert!(parse(&serde_json::json!({"oops": 1})).is_empty());

        let mut have = parse(&serde_json::from_str(SAMPLE).unwrap());
        let n = have.len();
        let revised = Day {
            rate: 3.89,
            ..*have.last().unwrap()
        };
        let next = Day {
            time: revised.time + DAY,
            ..revised
        };
        merge(&mut have, &[revised, next]);
        assert_eq!(have.len(), n + 1);
        assert_eq!(have[n - 1].rate, 3.89);
    }

    #[test]
    fn asks_after_publication_and_revision_times_only() {
        let ny = |s: &str| {
            s.parse::<jiff::civil::DateTime>()
                .unwrap()
                .to_zoned(super::ny())
                .unwrap()
                .timestamp()
                .as_millisecond()
        };
        let day = |s: &str| ms(&format!("{s}T00:00:00Z"));
        // Wednesday 25 Nov 2026 at 08:00: the 9:05 publication is next.
        assert_eq!(
            next_check(ny("2026-11-25T08:00"), Some(day("2026-11-23"))),
            ny("2026-11-25T09:05")
        );
        // 10:00, Tuesday's rate in: wait for the revision time.
        assert_eq!(
            next_check(ny("2026-11-25T10:00"), Some(day("2026-11-24"))),
            ny("2026-11-25T14:35")
        );
        // 10:00, Tuesday's rate not out yet: every 15 minutes.
        assert_eq!(
            next_check(ny("2026-11-25T10:00"), Some(day("2026-11-23"))),
            ny("2026-11-25T10:15")
        );
        // 14:30 and still missing: no later than the revision time.
        assert_eq!(
            next_check(ny("2026-11-25T14:30"), Some(day("2026-11-23"))),
            ny("2026-11-25T14:35")
        );
        // Friday evening: Monday morning.
        assert_eq!(
            next_check(ny("2026-11-27T18:00"), Some(day("2026-11-26"))),
            ny("2026-11-30T09:05")
        );
        // Monday 10:00 wants Friday's rate.
        assert_eq!(
            next_check(ny("2026-11-30T10:00"), Some(day("2026-11-27"))),
            ny("2026-11-30T14:35")
        );
    }

    #[test]
    fn weeks_and_months_from_days_nothing_intraday() {
        let p = NyFed::new(true);
        let sym = opentern_core::fed_funds();
        let day = Timeframe::parse("1D").unwrap();
        let d = p.latest(&sym, day, 5000).unwrap();
        assert!(d.len() > 1000);
        assert!(d.iter().all(|b| {
            let wd = (b.time / DAY + 3).rem_euclid(7);
            wd < 5
        }));
        let w = p.latest(&sym, Timeframe::parse("1W").unwrap(), 10).unwrap();
        assert_eq!(w.len(), 10);
        let last_week = w.last().unwrap();
        assert_eq!(last_week.close, d.last().unwrap().close);
        let m = p.latest(&sym, Timeframe::parse("1M").unwrap(), 3).unwrap();
        assert_eq!(m.len(), 3);
        assert_eq!(
            crate::resample::choose_base(Timeframe::parse("1h").unwrap(), &p.natives()),
            None
        );
        // Demo steps stay inside their range.
        for x in demo_days() {
            let (a, b) = x.target.unwrap();
            assert!(x.rate >= a && x.rate <= b, "{x:?}");
        }
    }
}

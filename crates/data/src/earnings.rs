//! Earnings dates, with reported and expected EPS, for the chart's "E"
//! markers. Best effort from three unofficial sources, merged by date:
//! Yahoo's earnings calendar (history, though it stops updating at times),
//! Nasdaq's last four quarters (US stocks), and Yahoo's next date.

use crate::fundamentals::{SUMMARY, new_session, session, text};
use crate::provider::{Error, Result};
use crate::resample::to_utc_midnight;
use opentern_core::{AssetKind, Symbol};

/// One earnings report, dated like daily bars (00:00 UTC of the
/// exchange-local date).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Earning {
    pub time: i64,
    /// Reported EPS (`None` for an upcoming report).
    pub eps: Option<f64>,
    /// The consensus estimate.
    pub estimate: Option<f64>,
    /// Surprise, in percent of the estimate.
    pub surprise: Option<f64>,
}

const VISUALIZATION: &str = "https://query1.finance.yahoo.com/v1/finance/visualization";
const NASDAQ: &str = "https://api.nasdaq.com/api/company";

/// Whether a symbol has earnings (stocks on the markets we know).
pub fn applies(sym: &Symbol) -> bool {
    sym.kind == AssetKind::Stock && matches!(sym.exchange(), "NASDAQ" | "NYSE" | "AMEX" | "NSE" | "BSE" | "YAHOO")
}

/// Every earnings date found for `sym`, oldest first. Sources that fail
/// are skipped; an error only when none answered.
pub fn fetch(sym: &Symbol) -> Result<Vec<Earning>> {
    let tz = jiff::tz::TimeZone::get(&sym.timezone).unwrap_or(jiff::tz::TimeZone::UTC);
    let agent = crate::provider::agent();
    let ticker = crate::yahoo::ticker(&sym.id);
    let mut all = Vec::new();
    let mut answered = false;
    let mut last_err = None;
    for part in [
        history(&agent, &ticker, &tz),
        next(&agent, &ticker, &tz),
        if matches!(sym.exchange(), "NASDAQ" | "NYSE" | "AMEX") {
            nasdaq(&agent, sym.ticker(), &tz)
        } else {
            Ok(Vec::new())
        },
    ] {
        match part {
            Ok(v) => {
                answered = true;
                all.extend(v);
            }
            Err(e) => last_err = Some(e),
        }
    }
    if !answered && let Some(e) = last_err {
        return Err(e);
    }
    Ok(merge(all))
}

/// Same report from two sources (dates a few days apart at most): one
/// entry, with whichever numbers are known.
pub(crate) fn merge(mut all: Vec<Earning>) -> Vec<Earning> {
    all.sort_by_key(|e| e.time);
    let mut out: Vec<Earning> = Vec::new();
    for e in all {
        match out.last_mut() {
            Some(prev) if e.time - prev.time <= 5 * 86_400_000 => {
                // A reported figure fixes the date.
                if prev.eps.is_none() && e.eps.is_some() {
                    prev.time = e.time;
                }
                prev.eps = prev.eps.or(e.eps);
                prev.estimate = prev.estimate.or(e.estimate);
                prev.surprise = prev.surprise.or(e.surprise);
            }
            _ => out.push(e),
        }
    }
    for e in &mut out {
        if e.surprise.is_none()
            && let (Some(a), Some(est)) = (e.eps, e.estimate)
            && est != 0.0
        {
            e.surprise = Some((a - est) / est.abs() * 100.0);
        }
    }
    out
}

/// A Yahoo request with the session cookie and crumb (a new session once
/// if the held one went stale).
fn with_crumb(agent: &ureq::Agent, call: impl Fn(&str, &str) -> Result<(u16, String)>) -> Result<serde_json::Value> {
    for attempt in 0..2 {
        let held = session().lock().ok().and_then(|s| s.clone());
        let (cookie, crumb) = match held {
            Some(s) if attempt == 0 => s,
            _ => {
                let s = new_session(agent)?;
                if let Ok(mut g) = session().lock() {
                    *g = Some(s.clone());
                }
                s
            }
        };
        let (status, body) = call(&cookie, &crumb)?;
        match status {
            401 | 403 if attempt == 0 => continue,
            429 => return Err(Error::Bad("Yahoo is limiting requests; try again in a minute.".into())),
            _ => {}
        }
        return serde_json::from_str(&body)
            .map_err(|_| Error::Bad(format!("Yahoo answered with an error ({status}).")));
    }
    Err(Error::Bad("Yahoo didn't start a session.".into()))
}

fn day_of(secs: i64, tz: &jiff::tz::TimeZone) -> i64 {
    to_utc_midnight(secs * 1000, tz)
}

/// Yahoo's earnings calendar for the ticker (newest 100 reports).
fn history(agent: &ureq::Agent, ticker: &str, tz: &jiff::tz::TimeZone) -> Result<Vec<Earning>> {
    let body = serde_json::json!({
        "sortType": "DESC",
        "entityIdType": "earnings",
        "sortField": "startdatetime",
        "includeFields": ["ticker", "startdatetime", "epsestimate", "epsactual", "epssurprisepct", "eventtype"],
        "query": {"operator": "eq", "operands": ["ticker", ticker]},
        "offset": 0,
        "size": 100
    })
    .to_string();
    let v = with_crumb(agent, |cookie, crumb| {
        let url = format!(
            "{VISUALIZATION}?crumb={}&lang=en-US&region=US",
            crate::yahoo::urlencode(crumb)
        );
        let mut resp = agent
            .post(&url)
            .header("Cookie", cookie)
            .header("Content-Type", "application/json")
            .config()
            .http_status_as_error(false)
            .build()
            .send(&body)
            .map_err(|_| Error::Offline)?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        Ok((status, text))
    })?;
    Ok(parse_history(&v, tz))
}

pub(crate) fn parse_history(v: &serde_json::Value, tz: &jiff::tz::TimeZone) -> Vec<Earning> {
    let empty = Vec::new();
    let rows = v["finance"]["result"][0]["documents"][0]["rows"]
        .as_array()
        .unwrap_or(&empty);
    rows.iter()
        .filter_map(|r| {
            // Event type 2 is an earnings report (others: meetings, calls).
            let kind = r[5].as_str().unwrap_or("2");
            if kind != "2" {
                return None;
            }
            let t: jiff::Timestamp = r[1].as_str()?.parse().ok()?;
            Some(Earning {
                time: day_of(t.as_second(), tz),
                estimate: r[2].as_f64(),
                eps: r[3].as_f64(),
                surprise: r[4].as_f64(),
            })
        })
        .collect()
}

/// The next report's date and expected EPS.
fn next(agent: &ureq::Agent, ticker: &str, tz: &jiff::tz::TimeZone) -> Result<Vec<Earning>> {
    let v = with_crumb(agent, |cookie, crumb| {
        let url = format!(
            "{SUMMARY}/{}?modules=calendarEvents&crumb={}",
            crate::yahoo::urlencode(ticker),
            crate::yahoo::urlencode(crumb)
        );
        let (status, body, _) = text(agent, &url, Some(cookie))?;
        Ok((status, body))
    })?;
    Ok(parse_next(&v, tz, crate::provider::now_ms()))
}

pub(crate) fn parse_next(v: &serde_json::Value, tz: &jiff::tz::TimeZone, now: i64) -> Vec<Earning> {
    let e = &v["quoteSummary"]["result"][0]["calendarEvents"]["earnings"];
    let Some(t) = e["earningsDate"][0]["raw"].as_i64() else {
        return Vec::new();
    };
    let time = day_of(t, tz);
    // Yahoo keeps last quarter's date until the new one is set.
    if time + 86_400_000 < now {
        return Vec::new();
    }
    vec![Earning {
        time,
        estimate: e["earningsAverage"]["raw"].as_f64(),
        ..Earning::default()
    }]
}

/// Nasdaq's last four quarters (report date, EPS, consensus).
fn nasdaq(agent: &ureq::Agent, ticker: &str, tz: &jiff::tz::TimeZone) -> Result<Vec<Earning>> {
    let url = format!("{NASDAQ}/{}/earnings-surprise", crate::yahoo::urlencode(ticker));
    let mut resp = agent
        .get(&url)
        .header("Accept", "application/json")
        .call()
        .map_err(|_| Error::Offline)?;
    let body = resp.body_mut().read_to_string().map_err(|_| Error::Offline)?;
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|_| Error::Bad("Nasdaq sent no earnings.".into()))?;
    Ok(parse_nasdaq(&v, tz))
}

pub(crate) fn parse_nasdaq(v: &serde_json::Value, tz: &jiff::tz::TimeZone) -> Vec<Earning> {
    let num = |x: &serde_json::Value| x.as_f64().or_else(|| x.as_str()?.trim().parse().ok());
    let empty = Vec::new();
    v["data"]["earningsSurpriseTable"]["rows"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter_map(|r| {
            // "7/30/2026"
            let mut p = r["dateReported"].as_str()?.split('/');
            let (m, d, y) = (
                p.next()?.parse().ok()?,
                p.next()?.parse().ok()?,
                p.next()?.parse().ok()?,
            );
            let date = jiff::civil::Date::new(y, m, d).ok()?;
            // Dated like daily bars: midnight UTC of that local date.
            let noon = date
                .to_datetime(jiff::civil::Time::constant(12, 0, 0, 0))
                .to_zoned(tz.clone())
                .ok()?;
            Some(Earning {
                time: day_of(noon.timestamp().as_second(), tz),
                eps: num(&r["eps"]),
                estimate: num(&r["consensusForecast"]),
                surprise: num(&r["percentageSurprise"]),
            })
        })
        .collect()
}

/// Quarterly reports for demo mode: steady, plausible, and one coming up.
pub fn made_up(sym: &Symbol, now: i64) -> Vec<Earning> {
    let day = 86_400_000;
    let mut seed: u64 = sym
        .id
        .bytes()
        .fold(1469598103934665603, |h, b| (h ^ b as u64).wrapping_mul(1099511628211));
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 10_000) as f64 / 10_000.0
    };
    let first = now - now.rem_euclid(day) + 23 * day;
    let mut eps = 1.0 + rnd() * 2.0;
    let mut out: Vec<Earning> = (0..24)
        .map(|k| {
            let time = first - k * 91 * day;
            let estimate = (eps * 100.0).round() / 100.0;
            let actual = ((eps * (0.94 + rnd() * 0.14)) * 100.0).round() / 100.0;
            eps *= 0.97 + rnd() * 0.03;
            if k == 0 {
                Earning {
                    time,
                    estimate: Some(estimate),
                    ..Earning::default()
                }
            } else {
                Earning {
                    time,
                    eps: Some(actual),
                    estimate: Some(estimate),
                    surprise: None,
                }
            }
        })
        .collect();
    out.reverse();
    merge(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ny() -> jiff::tz::TimeZone {
        jiff::tz::TimeZone::get("America/New_York").unwrap()
    }

    #[test]
    fn sources_parse_and_merge_by_date() {
        let hist = serde_json::json!({"finance": {"result": [{"documents": [{"rows": [
            ["AAPL", "2025-05-01T20:30:00.000Z", 1.63, 1.65, 1.41, "2"],
            ["AAPL", "2025-02-27T17:00:00.000Z", null, null, null, "11"],
            ["AAPL", "2025-01-30T21:31:00.000Z", 2.35, 2.4, 2.15, "2"]
        ]}]}]}});
        let h = parse_history(&hist, &ny());
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].eps, Some(1.65));
        // After the close in New York is still that day there.
        let may1 = jiff::civil::date(2025, 5, 1).to_zoned(jiff::tz::TimeZone::UTC).unwrap();
        assert_eq!(h[0].time, may1.timestamp().as_millisecond());

        let nas = serde_json::json!({"data": {"earningsSurpriseTable": {"rows": [
            {"dateReported": "5/1/2025", "eps": 1.65, "consensusForecast": "1.62", "percentageSurprise": "1.85"},
            {"dateReported": "7/30/2026", "eps": 1.91, "consensusForecast": "1.88", "percentageSurprise": "1.6"}
        ]}}});
        let n = parse_nasdaq(&nas, &ny());
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].time, h[0].time);

        let next = serde_json::json!({"quoteSummary": {"result": [{"calendarEvents": {"earnings": {
            "earningsDate": [{"raw": 1793653200}], "earningsAverage": {"raw": 1.98}
        }}}]}});
        let up = parse_next(&next, &ny(), 0);
        assert_eq!(up.len(), 1);
        assert_eq!(up[0].eps, None);
        assert!(parse_next(&next, &ny(), i64::MAX / 2).is_empty());

        let all = merge(h.into_iter().chain(n).chain(up).collect());
        assert_eq!(all.len(), 4, "{all:?}");
        assert!(all.windows(2).all(|w| w[0].time < w[1].time));
        assert_eq!(all[1].estimate, Some(1.63));
        assert_eq!(all.last().unwrap().estimate, Some(1.98));
    }

    #[test]
    fn demo_reports_are_quarterly_with_one_ahead() {
        let s = opentern_core::starter_watchlist()
            .into_iter()
            .find(|s| s.id == "NASDAQ:AAPL")
            .unwrap();
        let now = 1_790_000_000_000;
        let e = made_up(&s, now);
        assert_eq!(e.len(), 24);
        assert_eq!(e.iter().filter(|x| x.time > now).count(), 1);
        assert!(e.last().unwrap().eps.is_none());
        assert!(e[0].surprise.is_some());
    }
}

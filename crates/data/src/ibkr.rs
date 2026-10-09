//! Interactive Brokers through the Client Portal gateway (the small Java
//! service IBKR provides; run it and log in, then point Opentern at it).
//! History from `marketdata/history`, live prices by polling snapshots once
//! a second. Needs market-data subscriptions on the IBKR account. Symbols
//! are `IBKR:AAPL` (stocks, smart-routed).

use crate::builder::BarBuilder;
use crate::provider::{self, Error, Live, Provider, Result, num};
use opentern_core::{Bar, Symbol, Timeframe, Unit};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub struct Ibkr {
    agent: ureq::Agent,
    base: String,
}

pub fn bar_size(tf: Timeframe) -> Option<String> {
    Some(match (tf.unit, tf.count) {
        (Unit::Minute, n @ (1 | 2 | 3 | 5 | 10 | 15 | 30)) => format!("{n}min"),
        (Unit::Hour, n @ (1 | 2 | 3 | 4 | 8)) => format!("{n}h"),
        (Unit::Day, 1) => "1d".into(),
        (Unit::Week, 1) => "1w".into(),
        (Unit::Month, 1) => "1m".into(),
        _ => return None,
    })
}

/// A history period covering about `n` bars (the gateway caps a request
/// at 1,000 points).
fn period(tf: Timeframe, n: usize) -> String {
    let n = n.clamp(1, 1000) as i64;
    let ms = tf.millis().unwrap_or(30 * 86_400_000);
    // Stocks trade ~6.5h a day, 5 days a week: stretch intraday spans.
    let span_days = if tf.is_intraday() {
        (n * ms / 86_400_000).max(1) * 4
    } else {
        (n * ms / 86_400_000).max(1) * 7 / 5
    };
    if span_days <= 30 {
        format!("{span_days}d")
    } else if span_days <= 365 {
        format!("{}m", span_days / 30 + 1)
    } else {
        format!("{}y", (span_days / 365 + 1).min(15))
    }
}

pub(crate) fn parse_history(v: &serde_json::Value, daily: bool, tz: &jiff::tz::TimeZone) -> Result<Vec<Bar>> {
    if let Some(e) = v["error"].as_str() {
        return Err(Error::Bad(format!("IBKR: {e}")));
    }
    let empty = Vec::new();
    let mut bars: Vec<Bar> = v["data"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter_map(|b| {
            let t = b["t"].as_i64()?;
            Some(Bar {
                time: if daily {
                    crate::resample::to_utc_midnight(t, tz)
                } else {
                    t
                },
                open: b["o"].as_f64()?,
                high: b["h"].as_f64()?,
                low: b["l"].as_f64()?,
                close: b["c"].as_f64()?,
                volume: b["v"].as_f64().unwrap_or(0.0),
            })
        })
        .collect();
    bars.sort_by_key(|b| b.time);
    Ok(bars)
}

/// The first stock contract id in a secdef search reply.
pub(crate) fn parse_conid(v: &serde_json::Value) -> Option<String> {
    v.as_array()?.iter().find_map(|c| match &c["conid"] {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

/// A snapshot row → (last price, day volume). Field 31 is the last price
/// (sometimes prefixed with "C" for a prior close), 87 the volume.
pub(crate) fn parse_snapshot(v: &serde_json::Value) -> Option<(f64, f64)> {
    let row = v.as_array()?.first()?;
    let last = row["31"].as_str()?.trim_start_matches(['C', 'H']);
    let price: f64 = last.parse().ok()?;
    let vol = row["87_raw"].as_f64().or_else(|| num(&row["87"])).unwrap_or(0.0);
    Some((price, vol))
}

fn conids() -> &'static Mutex<HashMap<String, String>> {
    static C: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

impl Ibkr {
    pub fn new(base: &str) -> Ibkr {
        let base = base.trim_end_matches('/').to_string();
        // The gateway uses a self-signed certificate; only trust that on
        // this machine.
        let local = base.contains("://localhost") || base.contains("://127.0.0.1");
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .tls_config(ureq::tls::TlsConfig::builder().disable_verification(local).build())
            .build()
            .into();
        Ibkr { agent, base }
    }

    fn get(&self, path: &str) -> Result<serde_json::Value> {
        provider::get_json(&self.agent, &format!("{}/v1/api{path}", self.base)).map_err(|e| match e {
            Error::Offline => Error::Bad(
                "Can't reach the IBKR gateway. Start the Client Portal gateway and log in, then try again.".into(),
            ),
            e => e,
        })
    }

    fn conid(&self, sym: &Symbol) -> Result<String> {
        if let Some(c) = conids().lock().unwrap_or_else(|e| e.into_inner()).get(&sym.id) {
            return Ok(c.clone());
        }
        let v = self.get(&format!("/iserver/secdef/search?symbol={}&secType=STK", sym.ticker()))?;
        let c = parse_conid(&v).ok_or_else(|| Error::Bad(format!("IBKR doesn't know {}.", sym.ticker())))?;
        conids()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(sym.id.clone(), c.clone());
        Ok(c)
    }

    fn history(&self, sym: &Symbol, tf: Timeframe, n: usize, end: Option<i64>) -> Result<Vec<Bar>> {
        let size = bar_size(tf).ok_or_else(|| Error::Bad(format!("IBKR has no {} bars", tf.label())))?;
        let conid = self.conid(sym)?;
        let mut q = format!(
            "/iserver/marketdata/history?conid={conid}&period={}&bar={size}&outsideRth=false",
            period(tf, n)
        );
        if let Some(e) = end {
            let t = jiff::Timestamp::from_millisecond(e)
                .map(|t| t.strftime("%Y%m%d-%H:%M:%S").to_string())
                .unwrap_or_default();
            q += &format!("&startTime={t}");
        }
        let tz = jiff::tz::TimeZone::get(&sym.timezone).unwrap_or(jiff::tz::TimeZone::UTC);
        parse_history(&self.get(&q)?, !tf.is_intraday(), &tz)
    }
}

impl Provider for Ibkr {
    fn natives(&self) -> Vec<Timeframe> {
        ["1m", "2m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "1D", "1W", "1M"]
            .iter()
            .filter_map(|s| Timeframe::parse(s))
            .collect()
    }

    fn latest(&self, sym: &Symbol, tf: Timeframe, n: usize) -> Result<Vec<Bar>> {
        let mut bars = self.history(sym, tf, n, None)?;
        if bars.len() > n {
            bars.drain(..bars.len() - n);
        }
        Ok(bars)
    }

    fn since(&self, sym: &Symbol, tf: Timeframe, start: i64) -> Result<Vec<Bar>> {
        let ms = tf.millis().unwrap_or(86_400_000);
        let n = ((provider::now_ms() - start) / ms + 2).clamp(1, 1000) as usize;
        let mut bars = self.history(sym, tf, n, None)?;
        bars.retain(|b| b.time >= start);
        Ok(bars)
    }

    fn before(&self, sym: &Symbol, tf: Timeframe, end: i64, n: usize) -> Result<Vec<Bar>> {
        let mut bars = self.history(sym, tf, n, Some(end - 1))?;
        bars.retain(|b| b.time < end);
        if bars.len() > n {
            bars.drain(..bars.len() - n);
        }
        Ok(bars)
    }

    fn stream(&self, sym: &Symbol, tf: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()> {
        let conid = self.conid(sym)?;
        let last = self.latest(sym, tf, 1)?.pop();
        let tz = jiff::tz::TimeZone::get(&sym.timezone).unwrap_or(jiff::tz::TimeZone::UTC);
        let mut builder = BarBuilder::new(tf, tz, last);
        let path = format!("/iserver/marketdata/snapshot?conids={conid}&fields=31,87");
        let mut prev_volume: Option<f64> = None;
        let mut next = Instant::now();
        loop {
            if Instant::now() >= next {
                next = Instant::now() + Duration::from_secs(1);
                // The first snapshots after subscribing can be empty.
                if let Some((price, volume)) = parse_snapshot(&self.get(&path)?) {
                    let dv = prev_volume.map(|p| (volume - p).max(0.0)).unwrap_or(0.0);
                    prev_volume = Some(volume);
                    if !sink(Live::Bar(builder.trade(provider::now_ms(), price, dv))) {
                        return Ok(());
                    }
                    continue;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
            if !sink(Live::Idle) {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_history_conids_and_snapshots() {
        let ny = jiff::tz::TimeZone::get("America/New_York").unwrap();
        let v = serde_json::json!({"symbol":"AAPL","data":[{"o":185.0,"c":186.0,"h":187.0,"l":184.0,"v":1000.0,"t":1704205800000i64}]});
        let b = parse_history(&v, false, &ny).unwrap();
        assert_eq!((b[0].time, b[0].close), (1_704_205_800_000, 186.0));
        assert!(parse_history(&serde_json::json!({"error":"Chart data unavailable"}), false, &ny).is_err());
        assert_eq!(
            parse_conid(&serde_json::json!([{"conid":265598,"companyName":"APPLE INC"}])).as_deref(),
            Some("265598")
        );
        assert_eq!(
            parse_conid(&serde_json::json!([{"conid":"8314"}])).as_deref(),
            Some("8314")
        );
        let s = serde_json::json!([{"31":"C185.50","87":"1.2M","87_raw":1200000.0,"conid":265598}]);
        assert_eq!(parse_snapshot(&s), Some((185.5, 1_200_000.0)));
        assert_eq!(period(Timeframe::parse("1D").unwrap(), 500), "2y");
        assert_eq!(bar_size(Timeframe::parse("1h").unwrap()).as_deref(), Some("1h"));
    }
}

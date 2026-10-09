//! Upstox (India, NSE and BSE): historical and intraday candles from the v3
//! API, and live prices by polling quotes once a second (built into bars;
//! volume from the day's running total). Instruments are addressed by
//! Upstox keys (`NSE_EQ|INE002A01018`), looked up in Upstox's published
//! instrument list, downloaded once per run. Needs an access token (Upstox
//! tokens last a day).

use crate::builder::BarBuilder;
use crate::provider::{self, Error, Live, Provider, Result, num};
use crate::resample::to_utc_midnight;
use opentern_core::{Bar, Symbol, Timeframe, Unit};
use std::collections::HashMap;
use std::io::Read;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const API: &str = "https://api.upstox.com";
const MASTER: &str = "https://assets.upstox.com/market-quote/instruments/exchange";

pub struct Upstox {
    agent: ureq::Agent,
    token: String,
}

fn ist() -> jiff::tz::TimeZone {
    jiff::tz::TimeZone::get("Asia/Kolkata").unwrap_or(jiff::tz::TimeZone::UTC)
}

/// (unit, interval) in the v3 API.
pub fn unit(tf: Timeframe) -> Option<(&'static str, u32)> {
    Some(match (tf.unit, tf.count) {
        (Unit::Minute, n @ 1..=300) => ("minutes", n),
        (Unit::Hour, n @ 1..=5) => ("hours", n),
        (Unit::Day, 1) => ("days", 1),
        (Unit::Week, 1) => ("weeks", 1),
        (Unit::Month, 1) => ("months", 1),
        _ => return None,
    })
}

/// How many days one request may span for an interval (Upstox's limits).
fn window_days(tf: Timeframe) -> i64 {
    match (tf.unit, tf.count) {
        (Unit::Minute, n) if n <= 15 => 28,
        (Unit::Minute, _) | (Unit::Hour, _) => 88,
        (Unit::Day, _) => 3600,
        _ => 36500,
    }
}

/// Keys for the indices Opentern knows by name.
fn index_key(id: &str) -> Option<&'static str> {
    Some(match id {
        "NSE:NIFTY" => "NSE_INDEX|Nifty 50",
        "NSE:BANKNIFTY" => "NSE_INDEX|Nifty Bank",
        "BSE:SENSEX" => "BSE_INDEX|SENSEX",
        _ => return None,
    })
}

/// Trading symbol → instrument key, from Upstox's instrument list (an array
/// of objects with `segment`, `trading_symbol` and `instrument_key`).
pub(crate) fn parse_master(list: &serde_json::Value, out: &mut HashMap<String, String>) {
    for i in list.as_array().into_iter().flatten() {
        let (Some(seg), Some(sym), Some(key)) = (
            i["segment"].as_str(),
            i["trading_symbol"].as_str(),
            i["instrument_key"].as_str(),
        ) else {
            continue;
        };
        let ex = match seg {
            "NSE_EQ" => "NSE",
            "BSE_EQ" => "BSE",
            _ => continue,
        };
        out.insert(format!("{ex}:{sym}"), key.to_string());
    }
}

fn keys() -> &'static Mutex<HashMap<String, String>> {
    static KEYS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    KEYS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A candles reply → bars, oldest first. Rows are [time, o, h, l, c, v, oi].
pub(crate) fn parse_candles(v: &serde_json::Value, daily: bool) -> Result<Vec<Bar>> {
    if v["status"] != "success" {
        let msg = v["errors"][0]["message"].as_str().unwrap_or("unexpected reply");
        return Err(Error::Bad(format!("Upstox: {msg}")));
    }
    let tz = ist();
    let empty = Vec::new();
    let mut bars: Vec<Bar> = v["data"]["candles"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter_map(|r| {
            let t = provider::parse_time(r.get(0)?.as_str()?)?;
            Some(Bar {
                time: if daily { to_utc_midnight(t, &tz) } else { t },
                open: num(r.get(1)?)?,
                high: num(r.get(2)?)?,
                low: num(r.get(3)?)?,
                close: num(r.get(4)?)?,
                volume: num(r.get(5)?).unwrap_or(0.0),
            })
        })
        .collect();
    bars.sort_by_key(|b| b.time);
    bars.dedup_by_key(|b| b.time);
    Ok(bars)
}

/// A quotes reply → (last price, day volume).
pub(crate) fn parse_quote(v: &serde_json::Value) -> Option<(f64, f64)> {
    let q = v["data"].as_object()?.values().next()?;
    Some((num(&q["last_price"])?, num(&q["volume"]).unwrap_or(0.0)))
}

fn encode(key: &str) -> String {
    key.replace('|', "%7C").replace(' ', "%20")
}

fn date(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|t| t.to_zoned(ist()).date().to_string())
        .unwrap_or_default()
}

impl Upstox {
    pub fn new(token: &str) -> Upstox {
        Upstox {
            agent: provider::agent(),
            token: token.into(),
        }
    }

    fn get(&self, url: &str) -> Result<serde_json::Value> {
        let auth = format!("Bearer {}", self.token);
        provider::get_json_with(
            &self.agent,
            url,
            &[("Authorization", &auth), ("Accept", "application/json")],
        )
    }

    /// The Upstox key for a symbol (downloading the instrument list once).
    fn key(&self, sym: &Symbol) -> Result<String> {
        if let Some(k) = index_key(&sym.id) {
            return Ok(k.into());
        }
        let mut map = keys().lock().unwrap_or_else(|e| e.into_inner());
        if map.is_empty() {
            for ex in ["NSE", "BSE"] {
                let mut resp = self
                    .agent
                    .get(&format!("{MASTER}/{ex}.json.gz"))
                    .call()
                    .map_err(|_| Error::Offline)?;
                let mut gz = Vec::new();
                resp.body_mut()
                    .with_config()
                    .limit(256 * 1024 * 1024)
                    .reader()
                    .read_to_end(&mut gz)
                    .map_err(|_| Error::Offline)?;
                let mut text = String::new();
                flate2::read::GzDecoder::new(&gz[..])
                    .read_to_string(&mut text)
                    .map_err(|_| Error::Bad("Upstox: couldn't read the instrument list.".into()))?;
                let v: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|_| Error::Bad("Upstox: couldn't read the instrument list.".into()))?;
                parse_master(&v, &mut map);
            }
        }
        map.get(&sym.id)
            .cloned()
            .ok_or_else(|| Error::Bad(format!("Upstox doesn't list {}.", sym.id)))
    }

    fn history(&self, key: &str, tf: Timeframe, from: i64, to: i64) -> Result<Vec<Bar>> {
        let (u, n) = unit(tf).ok_or_else(|| Error::Bad(format!("Upstox has no {} bars", tf.label())))?;
        let url = format!(
            "{API}/v3/historical-candle/{}/{u}/{n}/{}/{}",
            encode(key),
            date(to),
            date(from)
        );
        parse_candles(&self.get(&url)?, !tf.is_intraday())
    }

    fn intraday(&self, key: &str, tf: Timeframe) -> Result<Vec<Bar>> {
        let (u, n) = unit(tf).ok_or_else(|| Error::Bad(format!("Upstox has no {} bars", tf.label())))?;
        if !matches!(u, "minutes" | "hours" | "days") {
            return Ok(Vec::new());
        }
        let url = format!("{API}/v3/historical-candle/intraday/{}/{u}/{n}", encode(key));
        parse_candles(&self.get(&url)?, !tf.is_intraday())
    }

    /// Up to `n` bars older than `end`, a window at a time.
    fn back(&self, key: &str, tf: Timeframe, end: i64, n: usize) -> Result<Vec<Bar>> {
        let span = window_days(tf) * 86_400_000;
        let mut out: Vec<Bar> = Vec::new();
        let mut to = end - 1;
        let mut empty = 0;
        while out.len() < n && empty < 4 && to > 946_684_800_000 {
            let from = to - span;
            let page: Vec<Bar> = self
                .history(key, tf, from, to)?
                .into_iter()
                .filter(|b| b.time < end)
                .collect();
            if page.is_empty() {
                empty += 1;
            } else {
                empty = 0;
                let first = page[0].time;
                out.retain(|b| b.time < first || !page.iter().any(|p| p.time == b.time));
                out.splice(0..0, page);
            }
            to = from - 1;
        }
        out.sort_by_key(|b| b.time);
        out.dedup_by_key(|b| b.time);
        if out.len() > n {
            out.drain(..out.len() - n);
        }
        Ok(out)
    }
}

impl Provider for Upstox {
    fn natives(&self) -> Vec<Timeframe> {
        ["1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "1D", "1W", "1M"]
            .iter()
            .filter_map(|s| Timeframe::parse(s))
            .collect()
    }

    fn latest(&self, sym: &Symbol, tf: Timeframe, n: usize) -> Result<Vec<Bar>> {
        let key = self.key(sym)?;
        let mut today = self.intraday(&key, tf).unwrap_or_default();
        let start = today.first().map(|b| b.time).unwrap_or(provider::now_ms() + 1);
        let mut older = self.back(&key, tf, start, n.saturating_sub(today.len()).max(1))?;
        older.append(&mut today);
        Ok(older)
    }

    fn since(&self, sym: &Symbol, tf: Timeframe, start: i64) -> Result<Vec<Bar>> {
        let key = self.key(sym)?;
        let span = window_days(tf) * 86_400_000;
        let mut out = provider::page_forward(|a, b| self.history(&key, tf, a, b), start, provider::now_ms(), span)?;
        for b in self.intraday(&key, tf).unwrap_or_default() {
            if out.last().is_none_or(|l| b.time > l.time) {
                out.push(b);
            }
        }
        out.retain(|b| b.time >= start);
        Ok(out)
    }

    fn before(&self, sym: &Symbol, tf: Timeframe, end: i64, n: usize) -> Result<Vec<Bar>> {
        let key = self.key(sym)?;
        self.back(&key, tf, end, n)
    }

    fn stream(&self, sym: &Symbol, tf: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()> {
        let key = self.key(sym)?;
        let last = self.latest(sym, tf, 1)?.pop();
        let mut builder = BarBuilder::new(tf, ist(), last);
        let url = format!("{API}/v2/market-quote/quotes?instrument_key={}", encode(&key));
        let mut prev_volume: Option<f64> = None;
        let mut next = Instant::now();
        loop {
            if Instant::now() >= next {
                next = Instant::now() + Duration::from_secs(1);
                let (price, volume) = parse_quote(&self.get(&url)?)
                    .ok_or_else(|| Error::Bad("Upstox: no quote for this symbol.".into()))?;
                let dv = prev_volume.map(|p| (volume - p).max(0.0)).unwrap_or(0.0);
                prev_volume = Some(volume);
                if !sink(Live::Bar(builder.trade(provider::now_ms(), price, dv))) {
                    return Ok(());
                }
            } else {
                std::thread::sleep(Duration::from_millis(100));
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

    #[test]
    fn parses_candles_master_and_quotes() {
        let v = serde_json::json!({"status":"success","data":{"candles":[
            ["2025-01-03T00:00:00+05:30", 1250.0, 1260.0, 1240.0, 1255.5, 1000, 0],
            ["2025-01-02T00:00:00+05:30", 1230.0, 1252.0, 1225.0, 1249.0, 900, 0]]}});
        let b = parse_candles(&v, true).unwrap();
        // Daily bars stamped at 00:00 UTC of their Indian date.
        assert_eq!(b[0].time, 1_735_776_000_000);
        assert_eq!((b[1].close, b[1].volume), (1255.5, 1000.0));
        let intraday = parse_candles(
            &serde_json::json!({"status":"success","data":{"candles":[["2025-01-02T09:15:00+05:30",1,2,0.5,1.5,10,0]]}}),
            false,
        )
        .unwrap();
        assert_eq!(intraday[0].time, 1_735_789_500_000);
        assert!(
            parse_candles(
                &serde_json::json!({"status":"error","errors":[{"message":"Invalid token"}]}),
                false
            )
            .is_err()
        );
        let mut m = HashMap::new();
        parse_master(
            &serde_json::json!([{"segment":"NSE_EQ","trading_symbol":"RELIANCE","instrument_key":"NSE_EQ|INE002A01018"},
                {"segment":"NSE_FO","trading_symbol":"X","instrument_key":"NSE_FO|1"}]),
            &mut m,
        );
        assert_eq!(m.get("NSE:RELIANCE").map(|s| s.as_str()), Some("NSE_EQ|INE002A01018"));
        assert_eq!(m.len(), 1);
        let q =
            serde_json::json!({"status":"success","data":{"NSE_EQ:RELIANCE":{"last_price":1255.5,"volume":123456}}});
        assert_eq!(parse_quote(&q), Some((1255.5, 123456.0)));
        assert_eq!(encode("NSE_INDEX|Nifty 50"), "NSE_INDEX%7CNifty%2050");
    }
}

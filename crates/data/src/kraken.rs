//! Kraken spot: REST OHLC (Kraken keeps only the latest 720 bars of each
//! interval, so scrolling back stops there) and the v2 websocket `ohlc`
//! channel for live bars. No key needed.

use crate::provider::{self, Error, Live, Provider, Result, num};
use opentern_core::{AssetKind, Bar, Symbol, Timeframe, Unit};
use std::time::Duration;

const REST: &str = "https://api.kraken.com";
const WS_HOST: &str = "ws.kraken.com";

pub struct Kraken {
    agent: ureq::Agent,
}

impl Default for Kraken {
    fn default() -> Self {
        Kraken {
            agent: provider::agent(),
        }
    }
}

fn minutes(tf: Timeframe) -> Option<u32> {
    Some(match (tf.unit, tf.count) {
        (Unit::Minute, n @ (1 | 5 | 15 | 30)) => n,
        (Unit::Hour, 1) => 60,
        (Unit::Hour, 4) => 240,
        (Unit::Day, 1) => 1440,
        (Unit::Week, 1) => 10080,
        _ => return None,
    })
}

/// "KRAKEN:BTCUSD" → ("XBTUSD" for REST, "BTC/USD" for the websocket).
pub fn pairs(sym: &Symbol) -> (String, String) {
    let t = sym.ticker();
    let (b, q) = provider::split_pair(t).unwrap_or((t, ""));
    let rest_base = if b == "BTC" { "XBT" } else { b };
    (format!("{rest_base}{q}"), format!("{b}/{q}"))
}

pub(crate) fn parse_ohlc(v: &serde_json::Value) -> Result<Vec<Bar>> {
    if let Some(e) = v["error"].as_array().and_then(|e| e.first()) {
        return Err(Error::Bad(format!("Kraken: {}", e.as_str().unwrap_or("error"))));
    }
    let result = v["result"]
        .as_object()
        .ok_or_else(|| Error::Bad("Kraken: unexpected reply".into()))?;
    let rows = result
        .iter()
        .find(|(k, _)| *k != "last")
        .and_then(|(_, r)| r.as_array().cloned())
        .unwrap_or_default();
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some(Bar {
                time: r.get(0)?.as_i64()? * 1000,
                open: num(r.get(1)?)?,
                high: num(r.get(2)?)?,
                low: num(r.get(3)?)?,
                close: num(r.get(4)?)?,
                volume: num(r.get(6)?)?,
            })
        })
        .collect())
}

pub(crate) fn parse_ws(v: &serde_json::Value) -> Vec<Bar> {
    if v["channel"] != "ohlc" {
        return Vec::new();
    }
    v["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|k| {
                    Some(Bar {
                        time: provider::parse_time(k["interval_begin"].as_str()?)?,
                        open: num(&k["open"])?,
                        high: num(&k["high"])?,
                        low: num(&k["low"])?,
                        close: num(&k["close"])?,
                        volume: num(&k["volume"])?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Kraken {
    fn ohlc(&self, sym: &Symbol, tf: Timeframe, since: Option<i64>) -> Result<Vec<Bar>> {
        let m = minutes(tf).ok_or_else(|| Error::Bad(format!("Kraken has no {} bars", tf.label())))?;
        let mut url = format!("{REST}/0/public/OHLC?pair={}&interval={m}", pairs(sym).0);
        if let Some(s) = since {
            url += &format!("&since={}", s / 1000);
        }
        parse_ohlc(&provider::get_json(&self.agent, &url)?)
    }

    pub fn symbols(&self) -> Result<Vec<Symbol>> {
        let v = provider::get_json(&self.agent, &format!("{REST}/0/public/AssetPairs"))?;
        let Some(map) = v["result"].as_object() else {
            return Ok(Vec::new());
        };
        Ok(map
            .values()
            .filter_map(|p| {
                let ws = p["wsname"].as_str()?;
                let (b, q) = ws.split_once('/')?;
                let b = if b == "XBT" { "BTC" } else { b };
                let decimals = p["pair_decimals"].as_i64().unwrap_or(2) as i32;
                Some(Symbol {
                    id: format!("KRAKEN:{b}{q}"),
                    description: format!("{b} / {q}"),
                    kind: AssetKind::Crypto,
                    currency: q.into(),
                    tick_size: 10f64.powi(-decimals),
                    min_qty: 0.0,
                    timezone: "UTC".into(),
                })
            })
            .collect())
    }
}

impl Provider for Kraken {
    fn natives(&self) -> Vec<Timeframe> {
        ["1m", "5m", "15m", "30m", "1h", "4h", "1D", "1W"]
            .iter()
            .filter_map(|s| Timeframe::parse(s))
            .collect()
    }

    fn latest(&self, sym: &Symbol, tf: Timeframe, n: usize) -> Result<Vec<Bar>> {
        let mut bars = self.ohlc(sym, tf, None)?;
        if bars.len() > n {
            bars.drain(..bars.len() - n);
        }
        Ok(bars)
    }

    fn since(&self, sym: &Symbol, tf: Timeframe, start: i64) -> Result<Vec<Bar>> {
        let mut bars = self.ohlc(sym, tf, Some(start - 1))?;
        bars.retain(|b| b.time >= start);
        Ok(bars)
    }

    fn before(&self, _: &Symbol, _: Timeframe, _: i64, _: usize) -> Result<Vec<Bar>> {
        // Kraken serves no further back than its last 720 bars.
        Ok(Vec::new())
    }

    fn stream(&self, sym: &Symbol, tf: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()> {
        let m = minutes(tf).ok_or_else(|| Error::Bad(format!("Kraken has no {} bars", tf.label())))?;
        let sub = serde_json::json!({"method": "subscribe", "params": {"channel": "ohlc", "symbol": [pairs(sym).1], "interval": m}})
            .to_string();
        provider::ws_candles(
            WS_HOST,
            443,
            "/v2",
            &sub,
            Some((r#"{"method":"ping"}"#, Duration::from_secs(30))),
            parse_ws,
            sink,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rest_and_ws() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"error":[],"result":{"XXBTZUSD":[[1700000000,"1","2","0.5","1.5","1.2","7.5",3]],"last":1700000000}}"#,
        )
        .unwrap();
        let b = parse_ohlc(&v).unwrap();
        assert_eq!((b[0].time, b[0].close, b[0].volume), (1_700_000_000_000, 1.5, 7.5));
        assert!(parse_ohlc(&serde_json::json!({"error":["EQuery:Unknown asset pair"]})).is_err());
        let ws = serde_json::json!({"channel":"ohlc","type":"update","data":[{"symbol":"BTC/USD","open":1.0,"high":2.0,
            "low":0.5,"close":1.5,"trades":3,"volume":7.5,"vwap":1.2,"interval_begin":"2023-11-14T22:13:00.000000000Z","interval":1}]});
        assert_eq!(parse_ws(&ws)[0].time, 1_699_999_980_000);
        assert_eq!(
            pairs(&Symbol::bare("KRAKEN:BTCUSD")),
            ("XBTUSD".into(), "BTC/USD".into())
        );
    }
}

//! Bybit spot (v5 API): REST klines for history, the public kline stream
//! for live bars, instruments for the symbol list. No key needed.

use crate::provider::{self, Error, Live, Provider, Result, num};
use opentern_core::{AssetKind, Bar, Symbol, Timeframe, Unit};
use std::time::Duration;

const REST: &str = "https://api.bybit.com";
const WS_HOST: &str = "stream.bybit.com";
const PAGE: usize = 1000;

pub struct Bybit {
    agent: ureq::Agent,
}

impl Default for Bybit {
    fn default() -> Self {
        Bybit {
            agent: provider::agent(),
        }
    }
}

pub fn interval(tf: Timeframe) -> Option<&'static str> {
    Some(match (tf.unit, tf.count) {
        (Unit::Minute, 1) => "1",
        (Unit::Minute, 3) => "3",
        (Unit::Minute, 5) => "5",
        (Unit::Minute, 15) => "15",
        (Unit::Minute, 30) => "30",
        (Unit::Hour, 1) => "60",
        (Unit::Hour, 2) => "120",
        (Unit::Hour, 4) => "240",
        (Unit::Hour, 6) => "360",
        (Unit::Hour, 12) => "720",
        (Unit::Day, 1) => "D",
        (Unit::Week, 1) => "W",
        (Unit::Month, 1) => "M",
        _ => return None,
    })
}

/// A kline reply → bars, oldest first (Bybit sends newest first).
pub(crate) fn parse_klines(v: &serde_json::Value) -> Result<Vec<Bar>> {
    if v["retCode"].as_i64() != Some(0) {
        return Err(Error::Bad(format!(
            "Bybit: {}",
            v["retMsg"].as_str().unwrap_or("unexpected reply")
        )));
    }
    let rows = v["result"]["list"].as_array().cloned().unwrap_or_default();
    let mut bars: Vec<Bar> = rows
        .iter()
        .filter_map(|r| {
            Some(Bar {
                time: num(r.get(0)?)? as i64,
                open: num(r.get(1)?)?,
                high: num(r.get(2)?)?,
                low: num(r.get(3)?)?,
                close: num(r.get(4)?)?,
                volume: num(r.get(5)?)?,
            })
        })
        .collect();
    bars.reverse();
    Ok(bars)
}

pub(crate) fn parse_ws(v: &serde_json::Value) -> Vec<Bar> {
    if !v["topic"].as_str().is_some_and(|t| t.starts_with("kline.")) {
        return Vec::new();
    }
    v["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|k| {
                    Some(Bar {
                        time: k["start"].as_i64()?,
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

impl Bybit {
    fn klines(&self, sym: &Symbol, tf: Timeframe, query: &str) -> Result<Vec<Bar>> {
        let iv = interval(tf).ok_or_else(|| Error::Bad(format!("Bybit has no {} bars", tf.label())))?;
        let url = format!(
            "{REST}/v5/market/kline?category=spot&symbol={}&interval={iv}&limit={PAGE}{query}",
            sym.ticker()
        );
        parse_klines(&provider::get_json(&self.agent, &url)?)
    }

    pub fn symbols(&self) -> Result<Vec<Symbol>> {
        let v = provider::get_json(&self.agent, &format!("{REST}/v5/market/instruments-info?category=spot"))?;
        let list = v["result"]["list"].as_array().cloned().unwrap_or_default();
        Ok(list
            .iter()
            .filter(|s| s["status"] == "Trading")
            .filter_map(|s| {
                let t = s["symbol"].as_str()?;
                let (base, quote) = (s["baseCoin"].as_str()?, s["quoteCoin"].as_str()?);
                Some(Symbol {
                    id: format!("BYBIT:{t}"),
                    description: format!("{base} / {quote}"),
                    kind: AssetKind::Crypto,
                    currency: quote.into(),
                    tick_size: num(&s["priceFilter"]["tickSize"]).filter(|x| *x > 0.0).unwrap_or(0.01),
                    min_qty: 0.0,
                    timezone: "UTC".into(),
                })
            })
            .collect())
    }
}

impl Provider for Bybit {
    fn natives(&self) -> Vec<Timeframe> {
        [
            "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "12h", "1D", "1W", "1M",
        ]
        .iter()
        .filter_map(|s| Timeframe::parse(s))
        .collect()
    }

    fn latest(&self, sym: &Symbol, tf: Timeframe, n: usize) -> Result<Vec<Bar>> {
        let mut bars = self.klines(sym, tf, "")?;
        if bars.len() < n
            && let Some(first) = bars.first().map(|b| b.time)
        {
            let older = self.before(sym, tf, first, n - bars.len())?;
            bars.splice(0..0, older);
        }
        Ok(bars)
    }

    fn since(&self, sym: &Symbol, tf: Timeframe, start: i64) -> Result<Vec<Bar>> {
        let span = tf.millis().unwrap_or(86_400_000 * 31) * PAGE as i64;
        provider::page_forward(
            |from, to| self.klines(sym, tf, &format!("&start={from}&end={to}")),
            start,
            provider::now_ms(),
            span,
        )
    }

    fn before(&self, sym: &Symbol, tf: Timeframe, end: i64, n: usize) -> Result<Vec<Bar>> {
        provider::page_back(|end| self.klines(sym, tf, &format!("&end={end}")), end, n)
    }

    fn stream(&self, sym: &Symbol, tf: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()> {
        let iv = interval(tf).ok_or_else(|| Error::Bad(format!("Bybit has no {} bars", tf.label())))?;
        let sub = serde_json::json!({"op": "subscribe", "args": [format!("kline.{iv}.{}", sym.ticker())]}).to_string();
        provider::ws_candles(
            WS_HOST,
            443,
            "/v5/public/spot",
            &sub,
            Some((r#"{"op":"ping"}"#, Duration::from_secs(20))),
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
            r#"{"retCode":0,"retMsg":"OK","result":{"category":"spot","symbol":"BTCUSDT","list":[
                ["1700003600000","2","4","1","3","20","0"],["1700000000000","1","2","0.5","2","10","0"]]}}"#,
        )
        .unwrap();
        let b = parse_klines(&v).unwrap();
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].time, b[0].close, b[1].high), (1_700_000_000_000, 2.0, 4.0));
        let err: serde_json::Value = serde_json::from_str(r#"{"retCode":10001,"retMsg":"params error"}"#).unwrap();
        assert!(parse_klines(&err).is_err());
        let ws: serde_json::Value = serde_json::from_str(
            r#"{"topic":"kline.60.BTCUSDT","data":[{"start":1700000000000,"end":1700003599999,"interval":"60",
                "open":"1","close":"2","high":"3","low":"0.5","volume":"7","turnover":"0","confirm":false,"timestamp":1}],
                "ts":1,"type":"snapshot"}"#,
        )
        .unwrap();
        let k = parse_ws(&ws);
        assert_eq!((k[0].open, k[0].high, k[0].volume), (1.0, 3.0, 7.0));
        assert!(parse_ws(&serde_json::json!({"op":"pong"})).is_empty());
    }
}

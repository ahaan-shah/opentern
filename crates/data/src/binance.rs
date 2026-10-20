//! Binance spot: REST klines for history, websocket klines for live bars,
//! exchangeInfo for the symbol list. Public endpoints, no key needed.

use crate::provider::{self, Error, Live, Provider, Result, num};
use opentern_core::{AssetKind, Bar, Symbol, Timeframe};

const REST: &str = "https://api.binance.com";
const WS_HOST: &str = "stream.binance.com";
const WS_PORT: u16 = 9443;
const PAGE: usize = 1000;

pub struct Binance {
    agent: ureq::Agent,
}

impl Default for Binance {
    fn default() -> Self {
        Binance {
            agent: provider::agent(),
        }
    }
}

/// "BINANCE:BTCUSDT" → "BTCUSDT".
fn pair(sym: &Symbol) -> &str {
    sym.ticker()
}

/// Binance's name for an interval, if it has one.
pub fn interval(tf: Timeframe) -> Option<String> {
    use opentern_core::Unit::*;
    let s = match (tf.unit, tf.count) {
        (Second, 1) => "1s",
        (Minute, n @ (1 | 3 | 5 | 15 | 30)) => return Some(format!("{n}m")),
        (Hour, n @ (1 | 2 | 4 | 6 | 8 | 12)) => return Some(format!("{n}h")),
        (Day, 1) => "1d",
        (Day, 3) => "3d",
        (Week, 1) => "1w",
        (Month, 1) => "1M",
        _ => return None,
    };
    Some(s.to_string())
}

pub(crate) fn parse_klines(v: &serde_json::Value) -> Result<Vec<Bar>> {
    let rows = v
        .as_array()
        .ok_or_else(|| Error::Bad(format!("Binance: {}", v["msg"].as_str().unwrap_or("unexpected reply"))))?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some(Bar {
                time: r.get(0)?.as_i64()?,
                open: num(r.get(1)?)?,
                high: num(r.get(2)?)?,
                low: num(r.get(3)?)?,
                close: num(r.get(4)?)?,
                volume: num(r.get(5)?)?,
            })
        })
        .collect())
}

/// A kline websocket message → the bar it carries.
pub(crate) fn parse_ws_kline(v: &serde_json::Value) -> Option<Bar> {
    let k = &v["k"];
    Some(Bar {
        time: k["t"].as_i64()?,
        open: num(&k["o"])?,
        high: num(&k["h"])?,
        low: num(&k["l"])?,
        close: num(&k["c"])?,
        volume: num(&k["v"])?,
    })
}

impl Binance {
    fn klines(&self, sym: &Symbol, tf: Timeframe, query: &str) -> Result<Vec<Bar>> {
        let iv = interval(tf).ok_or_else(|| Error::Bad(format!("Binance has no {} bars", tf.label())))?;
        let url = format!(
            "{REST}/api/v3/klines?symbol={}&interval={iv}&limit={PAGE}{query}",
            pair(sym)
        );
        parse_klines(&provider::get_json(&self.agent, &url)?)
    }

    /// One symbol's lot step (`LOT_SIZE` → `stepSize`), which TradingView
    /// uses as `syminfo.mincontract`: strategy sizes round down to it.
    pub fn min_qty(&self, ticker: &str) -> Result<f64> {
        let v = provider::get_json(&self.agent, &format!("{REST}/api/v3/exchangeInfo?symbol={ticker}"))?;
        v["symbols"]
            .get(0)
            .and_then(lot_step)
            .ok_or_else(|| Error::Bad("Binance: no lot size".into()))
    }

    /// Every symbol currently trading on Binance spot.
    pub fn symbols(&self) -> Result<Vec<Symbol>> {
        let v = provider::get_json(&self.agent, &format!("{REST}/api/v3/exchangeInfo?symbolStatus=TRADING"))?;
        let list = v["symbols"]
            .as_array()
            .ok_or_else(|| Error::Bad("Binance: unexpected symbol list".into()))?;
        Ok(list
            .iter()
            .filter_map(|s| {
                let ticker = s["symbol"].as_str()?;
                let base = s["baseAsset"].as_str()?;
                let quote = s["quoteAsset"].as_str()?;
                let tick = s["filters"]
                    .as_array()?
                    .iter()
                    .find(|f| f["filterType"] == "PRICE_FILTER")
                    .and_then(|f| num(&f["tickSize"]))
                    .filter(|t| *t > 0.0)
                    .unwrap_or(0.01);
                Some(Symbol {
                    id: format!("BINANCE:{ticker}"),
                    description: format!("{base} / {quote}"),
                    kind: AssetKind::Crypto,
                    currency: quote.to_string(),
                    tick_size: tick,
                    min_qty: lot_step(s).unwrap_or(0.0),
                    timezone: "UTC".into(),
                })
            })
            .collect())
    }
}

/// A symbol's `LOT_SIZE` step from exchangeInfo.
fn lot_step(s: &serde_json::Value) -> Option<f64> {
    s["filters"]
        .as_array()?
        .iter()
        .find(|f| f["filterType"] == "LOT_SIZE")
        .and_then(|f| num(&f["stepSize"]))
        .filter(|t| *t > 0.0)
}

impl Provider for Binance {
    fn natives(&self) -> Vec<Timeframe> {
        [
            "1s", "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "8h", "12h", "1D", "3D", "1W", "1M",
        ]
        .iter()
        .filter_map(|s| Timeframe::parse(s))
        .collect()
    }

    fn latest(&self, sym: &Symbol, tf: Timeframe, n: usize) -> Result<Vec<Bar>> {
        let mut bars = self.klines(sym, tf, "")?;
        while bars.len() < n {
            let Some(first) = bars.first() else { break };
            let older = self.before(sym, tf, first.time, n - bars.len())?;
            if older.is_empty() {
                break;
            }
            bars.splice(0..0, older);
        }
        Ok(bars)
    }

    fn since(&self, sym: &Symbol, tf: Timeframe, start: i64) -> Result<Vec<Bar>> {
        let mut out = Vec::new();
        let mut from = start;
        loop {
            let page = self.klines(sym, tf, &format!("&startTime={from}"))?;
            let n = page.len();
            if let Some(last) = page.last() {
                from = last.time + 1;
            }
            out.extend(page);
            if n < PAGE {
                return Ok(out);
            }
        }
    }

    fn before(&self, sym: &Symbol, tf: Timeframe, end: i64, n: usize) -> Result<Vec<Bar>> {
        let mut out: Vec<Bar> = Vec::new();
        let mut end = end - 1;
        while out.len() < n {
            let page = self.klines(sym, tf, &format!("&endTime={end}"))?;
            let Some(first) = page.first() else { break };
            end = first.time - 1;
            let full = page.len() == PAGE;
            out.splice(0..0, page);
            if !full {
                break;
            }
        }
        if out.len() > n {
            out.drain(..out.len() - n);
        }
        Ok(out)
    }

    fn stream(&self, sym: &Symbol, tf: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()> {
        let iv = interval(tf).ok_or_else(|| Error::Bad(format!("Binance has no {} bars", tf.label())))?;
        let path = format!("/ws/{}@kline_{iv}", pair(sym).to_lowercase());
        let (mut ws, _tcp) = provider::websocket(WS_HOST, WS_PORT, &path)?;
        loop {
            let item = match provider::read_text(&mut ws)? {
                Some(text) => serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .as_ref()
                    .and_then(parse_ws_kline)
                    .map(Live::Bar)
                    .unwrap_or(Live::Idle),
                None => Live::Idle,
            };
            if !sink(item) {
                let _ = ws.close(None);
                return Ok(());
            }
        }
    }
}

/// Live last price and change since the daily open for many symbols, via
/// one combined stream of daily klines.
pub(crate) fn stream_quotes(pairs: &[String], sink: &mut dyn FnMut(Option<(String, f64, f64)>) -> bool) -> Result<()> {
    let streams: Vec<String> = pairs.iter().map(|p| format!("{}@kline_1d", p.to_lowercase())).collect();
    let path = format!("/stream?streams={}", streams.join("/"));
    let (mut ws, _tcp) = provider::websocket(WS_HOST, WS_PORT, &path)?;
    loop {
        let item = provider::read_text(&mut ws)?.and_then(|text| {
            let v: serde_json::Value = serde_json::from_str(&text).ok()?;
            let k = &v["data"]["k"];
            Some((k["s"].as_str()?.to_string(), num(&k["c"])?, num(&k["o"])?))
        });
        if !sink(item) {
            let _ = ws.close(None);
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rest_klines() {
        let v: serde_json::Value = serde_json::from_str(
            r#"[[1700000000000,"36500.10","36600.00","36400.00","36550.50","12.5",1700000059999,"0",10,"0","0","0"]]"#,
        )
        .unwrap();
        let bars = parse_klines(&v).unwrap();
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].time, 1_700_000_000_000);
        assert_eq!(bars[0].open, 36500.10);
        assert_eq!(bars[0].volume, 12.5);
        let err: serde_json::Value = serde_json::from_str(r#"{"code":-1121,"msg":"Invalid symbol."}"#).unwrap();
        assert!(parse_klines(&err).is_err());
    }

    #[test]
    fn parses_ws_kline() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"e":"kline","E":1,"s":"BTCUSDT","k":{"t":1700000000000,"T":1700000059999,"s":"BTCUSDT","i":"1m","o":"1.0","c":"2.0","h":"3.0","l":"0.5","v":"10","x":false}}"#,
        )
        .unwrap();
        let b = parse_ws_kline(&v).unwrap();
        assert_eq!((b.open, b.high, b.low, b.close, b.volume), (1.0, 3.0, 0.5, 2.0, 10.0));
    }

    #[test]
    fn interval_names() {
        let n = |s| interval(Timeframe::parse(s).unwrap());
        assert_eq!(n("1h").as_deref(), Some("1h"));
        assert_eq!(n("1D").as_deref(), Some("1d"));
        assert_eq!(n("1M").as_deref(), Some("1M"));
        assert_eq!(n("45m"), None);
    }
}

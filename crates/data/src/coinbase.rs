//! Coinbase Exchange: REST candles for history (granularities 1m, 5m, 15m,
//! 1h, 6h, 1D; 300 per request) and the public `matches` feed for live
//! trades, built into bars. No key needed.

use crate::builder::BarBuilder;
use crate::provider::{self, Error, Live, Provider, Result, num};
use opentern_core::{AssetKind, Bar, Symbol, Timeframe, Unit};

const REST: &str = "https://api.exchange.coinbase.com";
const WS_HOST: &str = "ws-feed.exchange.coinbase.com";
const PAGE: i64 = 300;

pub struct Coinbase {
    agent: ureq::Agent,
}

impl Default for Coinbase {
    fn default() -> Self {
        Coinbase {
            agent: provider::agent(),
        }
    }
}

fn granularity(tf: Timeframe) -> Option<i64> {
    Some(match (tf.unit, tf.count) {
        (Unit::Minute, 1) => 60,
        (Unit::Minute, 5) => 300,
        (Unit::Minute, 15) => 900,
        (Unit::Hour, 1) => 3600,
        (Unit::Hour, 6) => 21600,
        (Unit::Day, 1) => 86400,
        _ => return None,
    })
}

/// "COINBASE:BTCUSD" → "BTC-USD".
pub fn product(sym: &Symbol) -> String {
    let t = sym.ticker();
    if t.contains('-') {
        return t.to_string();
    }
    match provider::split_pair(t) {
        Some((b, q)) => format!("{b}-{q}"),
        None => t.to_string(),
    }
}

/// A candles reply → bars, oldest first. Rows are [time, low, high, open,
/// close, volume] with time in seconds, newest first.
pub(crate) fn parse_candles(v: &serde_json::Value) -> Result<Vec<Bar>> {
    let Some(rows) = v.as_array() else {
        return Err(Error::Bad(format!(
            "Coinbase: {}",
            v["message"].as_str().unwrap_or("unexpected reply")
        )));
    };
    let mut bars: Vec<Bar> = rows
        .iter()
        .filter_map(|r| {
            Some(Bar {
                time: r.get(0)?.as_i64()? * 1000,
                low: num(r.get(1)?)?,
                high: num(r.get(2)?)?,
                open: num(r.get(3)?)?,
                close: num(r.get(4)?)?,
                volume: num(r.get(5)?)?,
            })
        })
        .collect();
    bars.sort_by_key(|b| b.time);
    Ok(bars)
}

/// A `match` message → (time, price, size).
pub(crate) fn parse_match(v: &serde_json::Value) -> Option<(i64, f64, f64)> {
    if !matches!(v["type"].as_str(), Some("match" | "last_match")) {
        return None;
    }
    Some((
        provider::parse_time(v["time"].as_str()?)?,
        num(&v["price"])?,
        num(&v["size"])?,
    ))
}

impl Coinbase {
    fn window(&self, sym: &Symbol, tf: Timeframe, from: i64, to: i64) -> Result<Vec<Bar>> {
        let g = granularity(tf).ok_or_else(|| Error::Bad(format!("Coinbase has no {} bars", tf.label())))?;
        let url = format!(
            "{REST}/products/{}/candles?granularity={g}&start={}&end={}",
            product(sym),
            provider::rfc3339(from),
            provider::rfc3339(to)
        );
        parse_candles(&provider::get_json(&self.agent, &url)?)
    }

    pub fn symbols(&self) -> Result<Vec<Symbol>> {
        let v = provider::get_json(&self.agent, &format!("{REST}/products"))?;
        Ok(v.as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter(|p| p["status"] == "online" && p["trading_disabled"] != true)
            .filter_map(|p| {
                let (base, quote) = (p["base_currency"].as_str()?, p["quote_currency"].as_str()?);
                Some(Symbol {
                    id: format!("COINBASE:{base}{quote}"),
                    description: format!("{base} / {quote}"),
                    kind: AssetKind::Crypto,
                    currency: quote.into(),
                    tick_size: num(&p["quote_increment"]).filter(|x| *x > 0.0).unwrap_or(0.01),
                    min_qty: 0.0,
                    timezone: "UTC".into(),
                })
            })
            .collect())
    }
}

impl Provider for Coinbase {
    fn natives(&self) -> Vec<Timeframe> {
        ["1m", "5m", "15m", "1h", "6h", "1D"]
            .iter()
            .filter_map(|s| Timeframe::parse(s))
            .collect()
    }

    fn latest(&self, sym: &Symbol, tf: Timeframe, n: usize) -> Result<Vec<Bar>> {
        self.before(sym, tf, provider::now_ms() + 1, n)
    }

    fn since(&self, sym: &Symbol, tf: Timeframe, start: i64) -> Result<Vec<Bar>> {
        let span = tf.millis().unwrap_or(86_400_000) * PAGE;
        provider::page_forward(|a, b| self.window(sym, tf, a, b), start, provider::now_ms(), span)
    }

    fn before(&self, sym: &Symbol, tf: Timeframe, end: i64, n: usize) -> Result<Vec<Bar>> {
        let ms = tf.millis().unwrap_or(86_400_000);
        provider::page_back(|e| self.window(sym, tf, e - ms * (PAGE - 1), e), end, n)
    }

    fn stream(&self, sym: &Symbol, tf: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()> {
        // Continue the forming bar from history, so its open stays right.
        let last = self.latest(sym, tf, 1)?.pop();
        let mut builder = BarBuilder::new(tf, jiff::tz::TimeZone::UTC, last);
        let (mut ws, _tcp) = provider::websocket(WS_HOST, 443, "/")?;
        let sub = serde_json::json!({"type": "subscribe", "product_ids": [product(sym)], "channels": ["matches"]});
        provider::send_text(&mut ws, &sub.to_string())?;
        loop {
            let item = provider::read_text(&mut ws)?
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .and_then(|v| parse_match(&v))
                .map(|(t, p, s)| Live::Bar(builder.trade(t, p, s)))
                .unwrap_or(Live::Idle);
            if !sink(item) {
                let _ = ws.close(None);
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_candles_and_matches() {
        let v: serde_json::Value =
            serde_json::from_str("[[1700003600, 1, 4, 2, 3, 20], [1700000000, 0.5, 2, 1, 2, 10]]").unwrap();
        let b = parse_candles(&v).unwrap();
        assert_eq!((b[0].time, b[0].open, b[0].low), (1_700_000_000_000, 1.0, 0.5));
        assert!(parse_candles(&serde_json::json!({"message": "NotFound"})).is_err());
        let m =
            serde_json::json!({"type":"match","price":"64210.5","size":"0.01","time":"2023-11-14T22:13:20.123456Z"});
        assert_eq!(parse_match(&m), Some((1_700_000_000_123, 64210.5, 0.01)));
        assert_eq!(product(&Symbol::bare("COINBASE:ETHUSD")), "ETH-USD");
    }
}

//! OANDA v20 for forex and CFDs: REST candles (mid prices) and the HTTP
//! pricing stream, built into bars (volume is the tick count, as OANDA's
//! own). Days are aligned to 00:00 UTC. Needs an API token and account id;
//! practice accounts use the fxpractice hosts.

use crate::builder::BarBuilder;
use crate::provider::{self, Error, Live, Provider, Result, num};
use opentern_core::{Bar, Symbol, Timeframe, Unit};
use std::io::BufRead;
use std::time::Duration;

pub struct Oanda {
    agent: ureq::Agent,
    token: String,
    account: String,
    live: bool,
}

const COUNT: usize = 5000;

pub fn granularity(tf: Timeframe) -> Option<&'static str> {
    Some(match (tf.unit, tf.count) {
        (Unit::Second, 5) => "S5",
        (Unit::Second, 10) => "S10",
        (Unit::Second, 15) => "S15",
        (Unit::Second, 30) => "S30",
        (Unit::Minute, 1) => "M1",
        (Unit::Minute, 2) => "M2",
        (Unit::Minute, 4) => "M4",
        (Unit::Minute, 5) => "M5",
        (Unit::Minute, 10) => "M10",
        (Unit::Minute, 15) => "M15",
        (Unit::Minute, 30) => "M30",
        (Unit::Hour, 1) => "H1",
        (Unit::Hour, 2) => "H2",
        (Unit::Hour, 3) => "H3",
        (Unit::Hour, 4) => "H4",
        (Unit::Hour, 6) => "H6",
        (Unit::Hour, 8) => "H8",
        (Unit::Hour, 12) => "H12",
        (Unit::Day, 1) => "D",
        (Unit::Week, 1) => "W",
        (Unit::Month, 1) => "M",
        _ => return None,
    })
}

/// "OANDA:EURUSD" → "EUR_USD"; "OANDA:XAUUSD" → "XAU_USD".
pub fn instrument(sym: &Symbol) -> String {
    let t = sym.ticker();
    if t.contains('_') {
        return t.to_string();
    }
    if t.len() == 6 {
        return format!("{}_{}", &t[..3], &t[3..]);
    }
    match provider::split_pair(t) {
        Some((b, q)) => format!("{b}_{q}"),
        None => t.to_string(),
    }
}

pub(crate) fn parse_candles(v: &serde_json::Value) -> Result<Vec<Bar>> {
    if let Some(m) = v["errorMessage"].as_str() {
        return Err(Error::Bad(format!("OANDA: {m}")));
    }
    let empty = Vec::new();
    Ok(v["candles"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter_map(|c| {
            let m = &c["mid"];
            Some(Bar {
                time: provider::parse_time(c["time"].as_str()?)?,
                open: num(&m["o"])?,
                high: num(&m["h"])?,
                low: num(&m["l"])?,
                close: num(&m["c"])?,
                volume: c["volume"].as_f64().unwrap_or(0.0),
            })
        })
        .collect())
}

/// A pricing-stream line → (time, mid price).
pub(crate) fn parse_price(v: &serde_json::Value) -> Option<(i64, f64)> {
    if v["type"] != "PRICE" {
        return None;
    }
    let bid = num(&v["bids"][0]["price"])?;
    let ask = num(&v["asks"][0]["price"])?;
    Some((provider::parse_time(v["time"].as_str()?)?, (bid + ask) / 2.0))
}

impl Oanda {
    pub fn new(token: &str, account: &str, live: bool) -> Oanda {
        Oanda {
            agent: provider::agent(),
            token: token.into(),
            account: account.into(),
            live,
        }
    }

    fn rest(&self) -> &'static str {
        if self.live {
            "https://api-fxtrade.oanda.com"
        } else {
            "https://api-fxpractice.oanda.com"
        }
    }

    fn candles(&self, sym: &Symbol, tf: Timeframe, query: &str) -> Result<Vec<Bar>> {
        let g = granularity(tf).ok_or_else(|| Error::Bad(format!("OANDA has no {} bars", tf.label())))?;
        let url = format!(
            "{}/v3/instruments/{}/candles?granularity={g}&price=M&dailyAlignment=0&alignmentTimezone=UTC&weeklyAlignment=Monday{query}",
            self.rest(),
            instrument(sym)
        );
        let auth = format!("Bearer {}", self.token);
        parse_candles(&provider::get_json_with(
            &self.agent,
            &url,
            &[("Authorization", &auth)],
        )?)
    }
}

impl Provider for Oanda {
    fn natives(&self) -> Vec<Timeframe> {
        [
            "5s", "15s", "30s", "1m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "8h", "12h", "1D", "1W", "1M",
        ]
        .iter()
        .filter_map(|s| Timeframe::parse(s))
        .collect()
    }

    fn latest(&self, sym: &Symbol, tf: Timeframe, n: usize) -> Result<Vec<Bar>> {
        self.candles(sym, tf, &format!("&count={}", n.clamp(1, COUNT)))
    }

    fn since(&self, sym: &Symbol, tf: Timeframe, start: i64) -> Result<Vec<Bar>> {
        let mut out: Vec<Bar> = Vec::new();
        let mut from = start;
        loop {
            let page = self.candles(sym, tf, &format!("&from={}&count={COUNT}", provider::rfc3339(from)))?;
            let n = page.len();
            for b in page {
                if out.last().is_none_or(|l| b.time > l.time) {
                    out.push(b);
                }
            }
            match out.last() {
                Some(l) if n >= COUNT => from = l.time + 1,
                _ => return Ok(out),
            }
        }
    }

    fn before(&self, sym: &Symbol, tf: Timeframe, end: i64, n: usize) -> Result<Vec<Bar>> {
        provider::page_back(
            |e| {
                self.candles(
                    sym,
                    tf,
                    &format!("&to={}&count={}", provider::rfc3339(e + 1000), n.clamp(1, COUNT)),
                )
            },
            end,
            n,
        )
    }

    fn stream(&self, sym: &Symbol, tf: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()> {
        let last = self.latest(sym, tf, 1)?.pop();
        let mut builder = BarBuilder::new(tf, jiff::tz::TimeZone::UTC, last);
        let host = if self.live {
            "https://stream-fxtrade.oanda.com"
        } else {
            "https://stream-fxpractice.oanda.com"
        };
        let url = format!(
            "{host}/v3/accounts/{}/pricing/stream?instruments={}",
            self.account,
            instrument(sym)
        );
        // A stream: no overall timeout, but give up if it goes quiet (OANDA
        // sends a heartbeat every 5 seconds).
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(None)
            .timeout_recv_body(Some(Duration::from_secs(30)))
            .build()
            .into();
        let auth = format!("Bearer {}", self.token);
        let resp = agent
            .get(&url)
            .header("Authorization", &auth)
            .call()
            .map_err(|e| match e {
                ureq::Error::StatusCode(401 | 403) => {
                    Error::Bad("OANDA refused the token. Check it in Settings → Data providers.".into())
                }
                _ => Error::Offline,
            })?;
        let reader = std::io::BufReader::new(resp.into_body().into_reader());
        for line in reader.lines() {
            let line = line.map_err(|_| Error::Offline)?;
            let item = serde_json::from_str::<serde_json::Value>(&line)
                .ok()
                .and_then(|v| parse_price(&v))
                .map(|(t, p)| Live::Bar(builder.trade(t, p, 1.0)))
                .unwrap_or(Live::Idle);
            if !sink(item) {
                return Ok(());
            }
        }
        Err(Error::Offline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_candles_and_prices() {
        let v = serde_json::json!({"instrument":"EUR_USD","granularity":"H1","candles":[
            {"complete":true,"volume":120,"time":"2024-01-02T14:00:00.000000000Z","mid":{"o":"1.09400","h":"1.09500","l":"1.09300","c":"1.09450"}}]});
        let b = parse_candles(&v).unwrap();
        assert_eq!((b[0].time, b[0].close, b[0].volume), (1_704_204_000_000, 1.0945, 120.0));
        assert!(parse_candles(&serde_json::json!({"errorMessage":"Insufficient authorization"})).is_err());
        let p = serde_json::json!({"type":"PRICE","time":"2024-01-02T14:00:01.000000000Z","bids":[{"price":"1.09440"}],"asks":[{"price":"1.09460"}]});
        assert_eq!(parse_price(&p), Some((1_704_204_001_000, 1.0945)));
        assert_eq!(parse_price(&serde_json::json!({"type":"HEARTBEAT"})), None);
        assert_eq!(instrument(&Symbol::bare("OANDA:EURUSD")), "EUR_USD");
        assert_eq!(instrument(&Symbol::bare("OANDA:XAUUSD")), "XAU_USD");
    }
}

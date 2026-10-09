//! Alpaca market data for US stocks: REST bars (split-adjusted) and the
//! live trade stream, built into bars. The free plan's feed is IEX (a
//! slice of the market's volume); accounts with SIP get the full tape.
//! Needs an Alpaca key id and secret.

use crate::builder::BarBuilder;
use crate::provider::{self, Error, Live, Provider, Result};
use crate::resample::to_utc_midnight;
use opentern_core::{Bar, Symbol, Timeframe, Unit};

const REST: &str = "https://data.alpaca.markets";
const WS_HOST: &str = "stream.data.alpaca.markets";
/// History goes back to 2016 on Alpaca.
const EPOCH: &str = "2016-01-01T00:00:00Z";

pub struct Alpaca {
    agent: ureq::Agent,
    key: String,
    secret: String,
    sip: bool,
}

impl Alpaca {
    pub fn new(key: &str, secret: &str, sip: bool) -> Alpaca {
        Alpaca {
            agent: provider::agent(),
            key: key.into(),
            secret: secret.into(),
            sip,
        }
    }

    fn feed(&self) -> &'static str {
        if self.sip { "sip" } else { "iex" }
    }

    fn get(&self, url: &str) -> Result<serde_json::Value> {
        provider::get_json_with(
            &self.agent,
            url,
            &[("APCA-API-KEY-ID", &self.key), ("APCA-API-SECRET-KEY", &self.secret)],
        )
    }

    /// One request; returns bars (oldest first) and the next page token.
    fn bars(&self, sym: &Symbol, tf: Timeframe, query: &str) -> Result<(Vec<Bar>, Option<String>)> {
        let iv = timeframe(tf).ok_or_else(|| Error::Bad(format!("Alpaca has no {} bars", tf.label())))?;
        let url = format!(
            "{REST}/v2/stocks/{}/bars?timeframe={iv}&adjustment=split&feed={}&limit=10000{query}",
            sym.ticker(),
            self.feed()
        );
        let v = self.get(&url)?;
        let tz = jiff::tz::TimeZone::get(&sym.timezone).unwrap_or(jiff::tz::TimeZone::UTC);
        let mut bars = parse_bars(&v, !tf.is_intraday(), &tz)?;
        bars.sort_by_key(|b| b.time);
        Ok((bars, v["next_page_token"].as_str().map(|s| s.to_string())))
    }
}

pub fn timeframe(tf: Timeframe) -> Option<String> {
    Some(match (tf.unit, tf.count) {
        (Unit::Minute, n @ 1..=59) => format!("{n}Min"),
        (Unit::Hour, n @ 1..=23) => format!("{n}Hour"),
        (Unit::Day, 1) => "1Day".into(),
        (Unit::Week, 1) => "1Week".into(),
        (Unit::Month, n @ (1 | 2 | 3 | 4 | 6 | 12)) => format!("{n}Month"),
        _ => return None,
    })
}

/// A bars reply → bars. Daily and longer bars are stamped at 00:00 UTC of
/// their exchange-local date, as everywhere else.
pub(crate) fn parse_bars(v: &serde_json::Value, daily: bool, tz: &jiff::tz::TimeZone) -> Result<Vec<Bar>> {
    if let Some(m) = v["message"].as_str() {
        return Err(Error::Bad(format!("Alpaca: {m}")));
    }
    let empty = Vec::new();
    Ok(v["bars"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter_map(|b| {
            let t = provider::parse_time(b["t"].as_str()?)?;
            Some(Bar {
                time: if daily { to_utc_midnight(t, tz) } else { t },
                open: b["o"].as_f64()?,
                high: b["h"].as_f64()?,
                low: b["l"].as_f64()?,
                close: b["c"].as_f64()?,
                volume: b["v"].as_f64()?,
            })
        })
        .collect())
}

/// Trades in a stream message: (time, price, size).
pub(crate) fn parse_trades(v: &serde_json::Value) -> Vec<(i64, f64, f64)> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter(|m| m["T"] == "t")
                .filter_map(|m| {
                    Some((
                        provider::parse_time(m["t"].as_str()?)?,
                        m["p"].as_f64()?,
                        m["s"].as_f64()?,
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Provider for Alpaca {
    fn natives(&self) -> Vec<Timeframe> {
        ["1m", "5m", "15m", "30m", "1h", "4h", "1D", "1W", "1M"]
            .iter()
            .filter_map(|s| Timeframe::parse(s))
            .collect()
    }

    fn latest(&self, sym: &Symbol, tf: Timeframe, n: usize) -> Result<Vec<Bar>> {
        let (bars, _) = self.bars(sym, tf, &format!("&start={EPOCH}&sort=desc&limit={}", n.min(10_000)))?;
        Ok(bars)
    }

    fn since(&self, sym: &Symbol, tf: Timeframe, start: i64) -> Result<Vec<Bar>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut q = format!("&start={}&sort=asc", provider::rfc3339(start));
            if let Some(t) = &token {
                q += &format!("&page_token={t}");
            }
            let (page, next) = self.bars(sym, tf, &q)?;
            out.extend(page);
            match next {
                Some(t) if out.len() < 200_000 => token = Some(t),
                _ => return Ok(out),
            }
        }
    }

    fn before(&self, sym: &Symbol, tf: Timeframe, end: i64, n: usize) -> Result<Vec<Bar>> {
        let q = format!(
            "&start={EPOCH}&end={}&sort=desc&limit={}",
            provider::rfc3339(end - 1000),
            n.min(10_000)
        );
        let (mut bars, _) = self.bars(sym, tf, &q)?;
        bars.retain(|b| b.time < end);
        Ok(bars)
    }

    fn stream(&self, sym: &Symbol, tf: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()> {
        let last = self.latest(sym, tf, 1)?.pop();
        let tz = jiff::tz::TimeZone::get(&sym.timezone).unwrap_or(jiff::tz::TimeZone::UTC);
        let mut builder = BarBuilder::new(tf, tz, last);
        let (mut ws, _tcp) = provider::websocket(WS_HOST, 443, &format!("/v2/{}", self.feed()))?;
        let auth = serde_json::json!({"action": "auth", "key": self.key, "secret": self.secret});
        provider::send_text(&mut ws, &auth.to_string())?;
        let sub = serde_json::json!({"action": "subscribe", "trades": [sym.ticker()]});
        provider::send_text(&mut ws, &sub.to_string())?;
        loop {
            let msg = provider::read_text(&mut ws)?;
            let v = msg.and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
            if let Some(v) = &v
                && let Some(err) = v.as_array().and_then(|a| a.iter().find(|m| m["T"] == "error"))
            {
                return Err(Error::Bad(format!(
                    "Alpaca: {}",
                    err["msg"].as_str().unwrap_or("stream error")
                )));
            }
            let trades = v.as_ref().map(parse_trades).unwrap_or_default();
            let mut go = true;
            if trades.is_empty() {
                go = sink(Live::Idle);
            }
            for (t, p, s) in trades {
                go &= sink(Live::Bar(builder.trade(t, p, s)));
            }
            if !go {
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
    fn parses_bars_trades_and_errors() {
        let ny = jiff::tz::TimeZone::get("America/New_York").unwrap();
        let v = serde_json::json!({"bars":[{"t":"2024-01-02T05:00:00Z","o":187.15,"h":188.44,"l":183.89,"c":185.64,"v":82488674,"n":1,"vw":1}],
            "symbol":"AAPL","next_page_token":null});
        let b = parse_bars(&v, true, &ny).unwrap();
        // Stamped at 00:00 UTC of 2024-01-02.
        assert_eq!(b[0].time, 1_704_153_600_000);
        assert_eq!(b[0].close, 185.64);
        let intraday = parse_bars(
            &serde_json::json!({"bars":[{"t":"2024-01-02T14:30:00Z","o":1.0,"h":2.0,"l":0.5,"c":1.5,"v":10.0}]}),
            false,
            &ny,
        )
        .unwrap();
        assert_eq!(intraday[0].time, 1_704_205_800_000);
        assert!(parse_bars(&serde_json::json!({"message":"forbidden"}), false, &ny).is_err());
        let t = serde_json::json!([{"T":"t","S":"AAPL","p":185.5,"s":100,"t":"2024-01-02T14:30:01.5Z"},{"T":"success","msg":"authenticated"}]);
        assert_eq!(parse_trades(&t), vec![(1_704_205_801_500, 185.5, 100.0)]);
        assert_eq!(timeframe(Timeframe::parse("15m").unwrap()).as_deref(), Some("15Min"));
    }
}

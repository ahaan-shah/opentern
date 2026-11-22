//! OKX spot and USDT perpetual swaps (v5 API): candles and history-candles
//! for history, the public candle channel for live bars. Daily and longer
//! bars use OKX's UTC-aligned intervals (`1Dutc`), as TradingView does. No
//! key needed. Perpetuals are `OKX:BTCUSDT.P` (OKX's `BTC-USDT-SWAP`).

use crate::provider::{self, Error, Live, Provider, Result, num};
use opentern_core::{AssetKind, Bar, Symbol, Timeframe, Unit};
use std::time::Duration;

/// OKX's REST hosts: www is blocked on some networks; the others serve the
/// same API. The first that answers is remembered.
const HOSTS: [&str; 3] = ["https://www.okx.com", "https://app.okx.com", "https://my.okx.com"];
static HOST: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
const WS_HOST: &str = "ws.okx.com";
const PAGE: usize = 100;

pub struct Okx {
    agent: ureq::Agent,
}

impl Default for Okx {
    fn default() -> Self {
        Okx {
            agent: provider::agent(),
        }
    }
}

/// Whether a symbol is a perpetual swap (`OKX:BTCUSDT.P`).
fn is_swap(sym: &Symbol) -> bool {
    sym.ticker().ends_with(".P")
}

pub fn interval(tf: Timeframe) -> Option<&'static str> {
    Some(match (tf.unit, tf.count) {
        (Unit::Minute, 1) => "1m",
        (Unit::Minute, 3) => "3m",
        (Unit::Minute, 5) => "5m",
        (Unit::Minute, 15) => "15m",
        (Unit::Minute, 30) => "30m",
        (Unit::Hour, 1) => "1H",
        (Unit::Hour, 2) => "2H",
        (Unit::Hour, 4) => "4H",
        (Unit::Hour, 6) => "6Hutc",
        (Unit::Hour, 12) => "12Hutc",
        (Unit::Day, 1) => "1Dutc",
        (Unit::Week, 1) => "1Wutc",
        (Unit::Month, 1) => "1Mutc",
        _ => return None,
    })
}

/// "OKX:BTCUSDT" → "BTC-USDT"; "OKX:BTCUSDT.P" → "BTC-USDT-SWAP".
pub fn inst_id(sym: &Symbol) -> String {
    let t = sym.ticker();
    let (t, swap) = match t.strip_suffix(".P") {
        Some(b) => (b, "-SWAP"),
        None => (t, ""),
    };
    if t.contains('-') {
        return format!("{t}{swap}");
    }
    match provider::split_pair(t) {
        Some((b, q)) => format!("{b}-{q}{swap}"),
        None => format!("{t}{swap}"),
    }
}

/// A candle row; `vol` is the volume column: 5 for spot (the coin), 6 for
/// swaps (5 counts contracts there; 6 is the coin).
fn row_at(r: &serde_json::Value, vol: usize) -> Option<Bar> {
    Some(Bar {
        time: num(r.get(0)?)? as i64,
        open: num(r.get(1)?)?,
        high: num(r.get(2)?)?,
        low: num(r.get(3)?)?,
        close: num(r.get(4)?)?,
        volume: num(r.get(vol)?)?,
    })
}

fn row(r: &serde_json::Value) -> Option<Bar> {
    row_at(r, 5)
}

/// Swaps' candles: their volume in the coin.
pub(crate) fn parse_ws_swap(v: &serde_json::Value) -> Vec<Bar> {
    if !v["arg"]["channel"].as_str().is_some_and(|c| c.starts_with("candle")) {
        return Vec::new();
    }
    v["data"]
        .as_array()
        .map(|a| a.iter().filter_map(|r| row_at(r, 6)).collect())
        .unwrap_or_default()
}

/// A candles reply → bars, oldest first; `vol` as in `row_at`.
pub(crate) fn parse_candles_at(v: &serde_json::Value, vol: usize) -> Result<Vec<Bar>> {
    if v["code"].as_str() != Some("0") {
        return Err(Error::Bad(format!(
            "OKX: {}",
            v["msg"].as_str().unwrap_or("unexpected reply")
        )));
    }
    let mut bars: Vec<Bar> = v["data"]
        .as_array()
        .map(|a| a.iter().filter_map(|r| row_at(r, vol)).collect())
        .unwrap_or_default();
    bars.reverse();
    Ok(bars)
}

pub(crate) fn parse_ws(v: &serde_json::Value) -> Vec<Bar> {
    if !v["arg"]["channel"].as_str().is_some_and(|c| c.starts_with("candle")) {
        return Vec::new();
    }
    v["data"]
        .as_array()
        .map(|a| a.iter().filter_map(row).collect())
        .unwrap_or_default()
}

impl Okx {
    /// GET `path` from whichever OKX host answers.
    pub(crate) fn get(&self, path: &str) -> Result<serde_json::Value> {
        use std::sync::atomic::Ordering;
        let start = HOST.load(Ordering::Relaxed);
        let mut last = Error::Offline;
        for k in 0..HOSTS.len() {
            let i = (start + k) % HOSTS.len();
            match provider::get_json(&self.agent, &format!("{}{path}", HOSTS[i])) {
                Err(Error::Offline) => last = Error::Offline,
                r => {
                    HOST.store(i, Ordering::Relaxed);
                    return r;
                }
            }
        }
        Err(last)
    }

    /// Up to 100 bars ending at `end` (inclusive), oldest first.
    fn page(&self, sym: &Symbol, tf: Timeframe, end: Option<i64>) -> Result<Vec<Bar>> {
        let bar = interval(tf).ok_or_else(|| Error::Bad(format!("OKX has no {} bars", tf.label())))?;
        let id = inst_id(sym);
        let path = match end {
            None => format!("/api/v5/market/candles?instId={id}&bar={bar}&limit=300"),
            Some(e) => format!(
                "/api/v5/market/history-candles?instId={id}&bar={bar}&limit={PAGE}&after={}",
                e + 1
            ),
        };
        parse_candles_at(&self.get(&path)?, if is_swap(sym) { 6 } else { 5 })
    }

    /// Every USDT perpetual swap trading now (`OKX:BTCUSDT.P`).
    pub fn perps(&self) -> Result<Vec<Symbol>> {
        let v = self.get("/api/v5/public/instruments?instType=SWAP")?;
        Ok(v["data"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter(|s| s["state"] == "live")
            .filter_map(|s| {
                // USDT- and USDC-margined only (coin-margined ones are
                // priced in USD contracts of the coin).
                let (base, quote) = s["uly"].as_str()?.split_once('-')?;
                if !matches!(quote, "USDT" | "USDC") {
                    return None;
                }
                Some(Symbol {
                    id: format!("OKX:{base}{quote}.P"),
                    description: format!("{base} / {quote} Perpetual"),
                    kind: AssetKind::Crypto,
                    currency: quote.into(),
                    tick_size: num(&s["tickSz"]).filter(|x| *x > 0.0).unwrap_or(0.01),
                    min_qty: 0.0,
                    timezone: "UTC".into(),
                })
            })
            .collect())
    }

    /// A perpetual's funding rate (a fraction, per period) and when it's
    /// next paid (ms).
    pub fn funding(&self, sym: &Symbol) -> Result<(f64, i64)> {
        let v = self.get(&format!("/api/v5/public/funding-rate?instId={}", inst_id(sym)))?;
        let d = &v["data"][0];
        let rate = num(&d["fundingRate"]).ok_or_else(|| Error::Bad("OKX: no funding rate".into()))?;
        let next = num(&d["fundingTime"]).unwrap_or(0.0) as i64;
        Ok((rate, next))
    }

    pub fn symbols(&self) -> Result<Vec<Symbol>> {
        let v = self.get("/api/v5/public/instruments?instType=SPOT")?;
        Ok(v["data"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter(|s| s["state"] == "live")
            .filter_map(|s| {
                let (base, quote) = (s["baseCcy"].as_str()?, s["quoteCcy"].as_str()?);
                Some(Symbol {
                    id: format!("OKX:{base}{quote}"),
                    description: format!("{base} / {quote}"),
                    kind: AssetKind::Crypto,
                    currency: quote.into(),
                    tick_size: num(&s["tickSz"]).filter(|x| *x > 0.0).unwrap_or(0.01),
                    min_qty: 0.0,
                    timezone: "UTC".into(),
                })
            })
            .collect())
    }
}

impl Provider for Okx {
    fn natives(&self) -> Vec<Timeframe> {
        [
            "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "12h", "1D", "1W", "1M",
        ]
        .iter()
        .filter_map(|s| Timeframe::parse(s))
        .collect()
    }

    fn latest(&self, sym: &Symbol, tf: Timeframe, n: usize) -> Result<Vec<Bar>> {
        let mut bars = self.page(sym, tf, None)?;
        if bars.len() < n
            && let Some(first) = bars.first().map(|b| b.time)
        {
            let older = self.before(sym, tf, first, n - bars.len())?;
            bars.splice(0..0, older);
        }
        Ok(bars)
    }

    fn since(&self, sym: &Symbol, tf: Timeframe, start: i64) -> Result<Vec<Bar>> {
        // Page back from now until the start is covered.
        let mut out = self.page(sym, tf, None)?;
        while out.first().is_some_and(|b| b.time > start) {
            let first = out[0].time;
            let older = self.page(sym, tf, Some(first - 1))?;
            if older.is_empty() {
                break;
            }
            out.splice(0..0, older);
        }
        out.retain(|b| b.time >= start);
        Ok(out)
    }

    fn before(&self, sym: &Symbol, tf: Timeframe, end: i64, n: usize) -> Result<Vec<Bar>> {
        provider::page_back(|end| self.page(sym, tf, Some(end)), end, n)
    }

    fn stream(&self, sym: &Symbol, tf: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()> {
        let bar = interval(tf).ok_or_else(|| Error::Bad(format!("OKX has no {} bars", tf.label())))?;
        let sub = serde_json::json!({"op": "subscribe", "args": [{"channel": format!("candle{bar}"), "instId": inst_id(sym)}]})
            .to_string();
        provider::ws_candles(
            WS_HOST,
            8443,
            "/ws/v5/business",
            &sub,
            Some(("ping", Duration::from_secs(20))),
            if is_swap(sym) { parse_ws_swap } else { parse_ws },
            sink,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_candles_and_ids() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"code":"0","msg":"","data":[["1700003600000","2","4","1","3","20","0","0","1"],
                ["1700000000000","1","2","0.5","2","10","0","0","1"]]}"#,
        )
        .unwrap();
        let b = parse_candles_at(&v, 5).unwrap();
        assert_eq!((b[0].time, b[1].close), (1_700_000_000_000, 3.0));
        assert!(
            parse_candles_at(
                &serde_json::json!({"code":"51001","msg":"Instrument ID does not exist"}),
                5
            )
            .is_err()
        );
        let ws = serde_json::json!({"arg":{"channel":"candle1H","instId":"BTC-USDT"},"data":[["1700000000000","1","2","0.5","1.5","9","0","0","0"]]});
        assert_eq!(parse_ws(&ws)[0].volume, 9.0);
        assert_eq!(inst_id(&Symbol::bare("OKX:ETHUSDT")), "ETH-USDT");
        assert_eq!(inst_id(&Symbol::bare("OKX:ETHUSDT.P")), "ETH-USDT-SWAP");
        // Swaps: the coin's volume (column 6), not contracts (5).
        let swap = serde_json::json!({"arg":{"channel":"candle1H","instId":"BTC-USDT-SWAP"},"data":[["1700000000000","1","2","0.5","1.5","900","9","0","0"]]});
        assert_eq!(parse_ws_swap(&swap)[0].volume, 9.0);
        assert_eq!(interval(Timeframe::parse("1D").unwrap()), Some("1Dutc"));
    }
}

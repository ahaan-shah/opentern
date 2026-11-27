//! Hyperliquid perpetuals: candle snapshots from the `/info` endpoint (a
//! POST with a JSON body) for history, the public `candle` stream for live
//! bars, `metaAndAssetCtxs` for the list and funding. No key needed.
//! Perpetuals are `HYPERLIQUID:BTCUSDC.P` (margined and settled in USDC,
//! as Hyperliquid names them); the coin keeps its own case
//! (`kPEPE`, a thousand PEPE). Markets of the builder-deployed dexes
//! (HIP-3: gold, oil, stocks, indices…) carry the dex before an underscore:
//! `HYPERLIQUID:XYZ_GOLDUSDC.P` is Hyperliquid's `xyz:GOLD`, quoted in its
//! dex's collateral. Hyperliquid keeps only the newest 5000 candles of each
//! interval, so history ends there (about 3.5 days of 1m).

use crate::provider::{self, Error, Live, Provider, Result, num};
use opentern_core::{AssetKind, Bar, Symbol, Timeframe, Unit};
use std::time::Duration;

const INFO: &str = "https://api.hyperliquid.xyz/info";
const WS_HOST: &str = "api.hyperliquid.xyz";
/// Bars asked for at once: under the 5000 a reply holds.
const PAGE: i64 = 4000;
/// Funding is paid every hour.
const HOUR: i64 = 3_600_000;

pub struct Hyperliquid {
    agent: ureq::Agent,
}

impl Default for Hyperliquid {
    fn default() -> Self {
        Hyperliquid {
            agent: provider::agent(),
        }
    }
}

/// The tokens a dex can use as collateral, which its ids end in.
const QUOTES: [&str; 4] = ["USDC", "USDT0", "USDH", "USDE"];

/// "HYPERLIQUID:BTCUSDC.P" → "BTC"; "HYPERLIQUID:XYZ_GOLDUSDC.P" → "xyz:GOLD".
pub fn coin(sym: &Symbol) -> String {
    let t = sym.ticker();
    let t = t.strip_suffix(".P").unwrap_or(t);
    let t = QUOTES
        .iter()
        .find_map(|q| t.strip_suffix(q).filter(|c| !c.is_empty()))
        .unwrap_or(t);
    match t.split_once('_') {
        Some((dex, c)) if !dex.is_empty() && !c.is_empty() => format!("{}:{c}", dex.to_ascii_lowercase()),
        _ => t.to_string(),
    }
}

/// The dex a coin trades on: "" for Hyperliquid's own.
fn dex_of(coin: &str) -> &str {
    coin.split_once(':').map_or("", |(d, _)| d)
}

/// An `/info` query, on a builder-deployed dex when `dex` isn't empty.
fn on_dex(mut body: serde_json::Value, dex: &str) -> serde_json::Value {
    if !dex.is_empty() {
        body["dex"] = dex.into();
    }
    body
}

pub fn interval(tf: Timeframe) -> Option<&'static str> {
    Some(match (tf.unit, tf.count) {
        (Unit::Minute, 1) => "1m",
        (Unit::Minute, 3) => "3m",
        (Unit::Minute, 5) => "5m",
        (Unit::Minute, 15) => "15m",
        (Unit::Minute, 30) => "30m",
        (Unit::Hour, 1) => "1h",
        (Unit::Hour, 2) => "2h",
        (Unit::Hour, 4) => "4h",
        (Unit::Hour, 8) => "8h",
        (Unit::Hour, 12) => "12h",
        (Unit::Day, 1) => "1d",
        (Unit::Day, 3) => "3d",
        (Unit::Week, 1) => "1w",
        (Unit::Month, 1) => "1M",
        _ => return None,
    })
}

/// One candle, from a snapshot or the stream:
/// `{"t":open ms,"T":close ms,"o":"1","h":…,"l":…,"c":…,"v":base volume,…}`.
fn candle(k: &serde_json::Value) -> Option<Bar> {
    Some(Bar {
        time: k["t"].as_i64()?,
        open: num(&k["o"])?,
        high: num(&k["h"])?,
        low: num(&k["l"])?,
        close: num(&k["c"])?,
        volume: num(&k["v"])?,
    })
}

/// A candle snapshot → bars, oldest first. An unknown coin gets an error
/// object or `null` instead of a list.
pub(crate) fn parse_candles(v: &serde_json::Value) -> Result<Vec<Bar>> {
    let Some(rows) = v.as_array() else {
        return Err(Error::Bad(format!(
            "Hyperliquid: {}",
            v["error"].as_str().unwrap_or("no candles for this market")
        )));
    };
    let mut bars: Vec<Bar> = rows.iter().filter_map(candle).collect();
    bars.sort_by_key(|b| b.time);
    bars.dedup_by_key(|b| b.time);
    Ok(bars)
}

pub(crate) fn parse_ws(v: &serde_json::Value) -> Vec<Bar> {
    if v["channel"] != "candle" {
        return Vec::new();
    }
    // One candle per message; a list is taken too.
    match &v["data"] {
        serde_json::Value::Array(a) => a.iter().filter_map(candle).collect(),
        k => candle(k).into_iter().collect(),
    }
}

/// Names for builder-dex markets whose ticker doesn't say what they are
/// (Hyperliquid lists only tickers), so a search by name finds them.
/// Listed companies are found through the stock search instead
/// (`underlying`).
fn name_of(c: &str) -> Option<&'static str> {
    Some(match c {
        "SPCX" => "SpaceX",
        "OAI" => "OpenAI",
        "ANTH" => "Anthropic",
        "SMSN" => "Samsung Electronics",
        "SKHX" | "SKHY" => "SK Hynix",
        "SOFTBANK" => "SoftBank",
        "HYUNDAI" => "Hyundai",
        "KIOXIA" => "Kioxia",
        "XIAOMI" => "Xiaomi",
        "TCNT" => "Tencent",
        "MINIMAX" => "MiniMax",
        "ZHIPU" => "Zhipu AI",
        "SHEIN" => "Shein",
        "UNITREE" => "Unitree Robotics",
        "CAMBRICON" => "Cambricon",
        "CXMT" => "CXMT",
        "JIO" => "Jio Platforms",
        "GIGADEV" => "GigaDevice",
        "GOLD" => "Gold",
        "SILVER" => "Silver",
        "COPPER" => "Copper",
        "PLATINUM" => "Platinum",
        "PALLADIUM" => "Palladium",
        "CL" => "WTI Crude Oil",
        "BRENTOIL" => "Brent Crude Oil",
        "NATGAS" => "Natural Gas",
        "HO" => "Heating Oil",
        "SP500" => "S&P 500",
        "JP225" => "Nikkei 225",
        "KR200" => "KOSPI 200",
        "NIFTY" => "Nifty 50",
        "EUR" => "Euro",
        "GBP" => "British Pound",
        "JPY" => "Japanese Yen",
        "BTCD" => "Bitcoin Dominance",
        _ => return None,
    })
}

/// A builder-dex market's underlying ticker: "HYPERLIQUID:XYZ_TSLAUSDC.P"
/// → "TSLA". None for Hyperliquid's own coins and other exchanges.
pub fn underlying(sym: &Symbol) -> Option<&str> {
    if sym.exchange() != "HYPERLIQUID" {
        return None;
    }
    let t = sym.ticker();
    let t = t.strip_suffix(".P").unwrap_or(t);
    let t = QUOTES
        .iter()
        .find_map(|q| t.strip_suffix(q).filter(|c| !c.is_empty()))
        .unwrap_or(t);
    t.split_once('_').map(|(_, c)| c).filter(|c| !c.is_empty())
}

/// `metaAndAssetCtxs` → the perpetuals trading now, quoted in `quote` (the
/// dex's collateral). A builder dex's names are "xyz:GOLD".
pub(crate) fn parse_perps(v: &serde_json::Value, quote: &str) -> Vec<Symbol> {
    let universe = v[0]["universe"].as_array().cloned().unwrap_or_default();
    let ctxs = v[1].as_array().cloned().unwrap_or_default();
    universe
        .iter()
        .enumerate()
        .filter(|(_, u)| u["isDelisted"].as_bool() != Some(true))
        .filter_map(|(i, u)| {
            let name = u["name"].as_str().filter(|n| !n.is_empty() && !n.contains('_'))?;
            let (ticker, description) = match name.split_once(':') {
                Some((dex, c)) if !dex.is_empty() && !c.is_empty() => {
                    let dex = dex.to_ascii_uppercase();
                    let label = name_of(c).unwrap_or(c);
                    (format!("{dex}_{c}"), format!("{label} / {quote} Perpetual ({dex})"))
                }
                Some(_) => return None,
                None => (name.to_string(), format!("{name} / {quote} Perpetual")),
            };
            let sz = u["szDecimals"].as_i64().unwrap_or(0);
            let px = ctxs.get(i).and_then(|c| num(&c["markPx"])).unwrap_or(0.0);
            Some(Symbol {
                id: format!("HYPERLIQUID:{ticker}{quote}.P"),
                description,
                kind: AssetKind::Crypto,
                currency: quote.into(),
                tick_size: opentern_core::hyperliquid_tick(px, sz),
                min_qty: 10f64.powi(-(sz.clamp(0, 12) as i32)),
                timezone: "UTC".into(),
            })
        })
        .collect()
}

/// A coin's funding rate (a fraction, per hour) from `metaAndAssetCtxs`.
pub(crate) fn parse_funding(v: &serde_json::Value, coin: &str) -> Option<f64> {
    let i = v[0]["universe"]
        .as_array()?
        .iter()
        .position(|u| u["name"].as_str() == Some(coin))?;
    num(&v[1][i]["funding"])
}

impl Hyperliquid {
    fn info(&self, body: serde_json::Value) -> Result<serde_json::Value> {
        provider::post_json(&self.agent, INFO, &body)
    }

    fn candles(&self, sym: &Symbol, tf: Timeframe, start: i64, end: i64) -> Result<Vec<Bar>> {
        let iv = interval(tf).ok_or_else(|| Error::Bad(format!("Hyperliquid has no {} bars", tf.label())))?;
        parse_candles(&self.info(serde_json::json!({
            "type": "candleSnapshot",
            "req": {"coin": coin(sym), "interval": iv, "startTime": start.max(0), "endTime": end},
        }))?)
    }

    /// Every perpetual trading now: Hyperliquid's own
    /// (`HYPERLIQUID:BTCUSDC.P`), then each builder-deployed dex's
    /// (`HYPERLIQUID:XYZ_GOLDUSDC.P`). A dex that doesn't answer is left out.
    pub fn perps(&self) -> Result<Vec<Symbol>> {
        let mut list = parse_perps(&self.info(serde_json::json!({"type": "metaAndAssetCtxs"}))?, "USDC");
        if list.is_empty() {
            return Err(Error::Bad("Hyperliquid: no markets listed".into()));
        }
        let dexes = self.info(serde_json::json!({"type": "perpDexs"})).unwrap_or_default();
        // Collateral token names, asked for when a dex isn't on USDC.
        let mut tokens: Option<serde_json::Value> = None;
        for dex in dexes
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| d["name"].as_str())
        {
            let Ok(v) = self.info(on_dex(serde_json::json!({"type": "metaAndAssetCtxs"}), dex)) else {
                continue;
            };
            let quote = match v[0]["collateralToken"].as_i64().unwrap_or(0) {
                0 => "USDC".to_string(),
                i => {
                    if tokens.is_none() {
                        tokens = self.info(serde_json::json!({"type": "spotMeta"})).ok();
                    }
                    let name = tokens.as_ref().and_then(|t| {
                        t["tokens"]
                            .as_array()?
                            .iter()
                            .find(|x| x["index"].as_i64() == Some(i))?["name"]
                            .as_str()
                            .map(str::to_string)
                    });
                    match name.filter(|n| QUOTES.contains(&n.as_str())) {
                        Some(n) => n,
                        None => continue,
                    }
                }
            };
            list.extend(parse_perps(&v, &quote));
        }
        Ok(list)
    }

    /// A perpetual's funding rate (a fraction, per hour) and when it's next
    /// paid (ms): on the hour.
    pub fn funding(&self, sym: &Symbol) -> Result<(f64, i64)> {
        let coin = coin(sym);
        let v = self.info(on_dex(serde_json::json!({"type": "metaAndAssetCtxs"}), dex_of(&coin)))?;
        let rate = parse_funding(&v, &coin).ok_or_else(|| Error::Bad("Hyperliquid: no funding rate".into()))?;
        Ok((rate, (provider::now_ms() / HOUR + 1) * HOUR))
    }

    /// The order book: `{"levels":[[bids…],[asks…]]}`, each `{"px","sz","n"}`.
    pub fn book(&self, sym: &Symbol) -> Result<serde_json::Value> {
        self.info(serde_json::json!({"type": "l2Book", "coin": coin(sym)}))
    }
}

/// One bar's span, months taken as 31 days.
fn span(tf: Timeframe) -> i64 {
    tf.millis().unwrap_or(86_400_000 * 31)
}

impl Provider for Hyperliquid {
    fn natives(&self) -> Vec<Timeframe> {
        [
            "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "8h", "12h", "1D", "3D", "1W", "1M",
        ]
        .iter()
        .filter_map(|s| Timeframe::parse(s))
        .filter(|tf| interval(*tf).is_some())
        .collect()
    }

    fn latest(&self, sym: &Symbol, tf: Timeframe, n: usize) -> Result<Vec<Bar>> {
        let now = provider::now_ms();
        let n = (n as i64).clamp(1, PAGE);
        let mut bars = self.candles(sym, tf, now - span(tf) * n, now)?;
        if bars.len() > n as usize {
            bars.drain(..bars.len() - n as usize);
        }
        Ok(bars)
    }

    fn since(&self, sym: &Symbol, tf: Timeframe, start: i64) -> Result<Vec<Bar>> {
        provider::page_forward(
            |from, to| self.candles(sym, tf, from, to),
            start,
            provider::now_ms(),
            span(tf) * PAGE,
        )
    }

    fn before(&self, sym: &Symbol, tf: Timeframe, end: i64, n: usize) -> Result<Vec<Bar>> {
        let page = (n as i64).clamp(1, PAGE);
        provider::page_back(|end| self.candles(sym, tf, end - span(tf) * page, end), end, n)
    }

    fn stream(&self, sym: &Symbol, tf: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()> {
        let iv = interval(tf).ok_or_else(|| Error::Bad(format!("Hyperliquid has no {} bars", tf.label())))?;
        let sub = serde_json::json!({
            "method": "subscribe",
            "subscription": {"type": "candle", "coin": coin(sym), "interval": iv},
        })
        .to_string();
        // The server drops a connection that's quiet for a minute.
        provider::ws_candles(
            WS_HOST,
            443,
            "/ws",
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

    fn sym(id: &str) -> Symbol {
        Symbol {
            id: id.into(),
            description: String::new(),
            kind: AssetKind::Crypto,
            currency: "USDC".into(),
            tick_size: 1.0,
            min_qty: 0.0,
            timezone: "UTC".into(),
        }
    }

    #[test]
    fn coins_from_ids() {
        assert_eq!(coin(&sym("HYPERLIQUID:BTCUSDC.P")), "BTC");
        assert_eq!(coin(&sym("HYPERLIQUID:kPEPEUSDC.P")), "kPEPE");
        assert_eq!(coin(&sym("HYPERLIQUID:HYPEUSDC.P")), "HYPE");
        assert_eq!(coin(&sym("HYPERLIQUID:XYZ_GOLDUSDC.P")), "xyz:GOLD");
        assert_eq!(coin(&sym("HYPERLIQUID:XYZ_SP500USDC.P")), "xyz:SP500");
        assert_eq!(coin(&sym("HYPERLIQUID:KM_US500USDH.P")), "km:US500");
        assert_eq!(dex_of("xyz:GOLD"), "xyz");
        assert_eq!(dex_of("BTC"), "");
    }

    /// A builder dex's markets: the dex before an underscore, its
    /// collateral as the quote, searchable by the asset's name.
    #[test]
    fn lists_builder_dex_markets() {
        let v: serde_json::Value = serde_json::from_str(
            r#"[{"universe":[{"name":"xyz:GOLD","szDecimals":4},{"name":"xyz:SP500","szDecimals":3},
                             {"name":"xyz:OLD","szDecimals":0,"isDelisted":true}],"collateralToken":0},
                [{"funding":"0.00001","markPx":"3634.9"},{"funding":"0","markPx":"6850.5"},{"funding":"0","markPx":"1"}]]"#,
        )
        .unwrap();
        let list = parse_perps(&v, "USDC");
        let ids: Vec<&str> = list.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["HYPERLIQUID:XYZ_GOLDUSDC.P", "HYPERLIQUID:XYZ_SP500USDC.P"]);
        // Named where the ticker doesn't say: SpaceX's SPCX, gold.
        assert_eq!(list[0].description, "Gold / USDC Perpetual (XYZ)");
        assert_eq!(name_of("SPCX"), Some("SpaceX"));
        assert_eq!(underlying(&list[0]), Some("GOLD"));
        assert_eq!(underlying(&Symbol::bare("HYPERLIQUID:BTCUSDC.P")), None);
        assert_eq!(underlying(&Symbol::bare("NASDAQ:TSLA")), None);
        assert_eq!(coin(&list[1]), "xyz:SP500");
        assert_eq!(parse_funding(&v, "xyz:GOLD"), Some(0.00001));
        assert_eq!(on_dex(serde_json::json!({"type": "x"}), "xyz")["dex"], "xyz");
        assert!(on_dex(serde_json::json!({"type": "x"}), "").get("dex").is_none());
    }

    #[test]
    fn parses_snapshots_and_the_stream() {
        let v: serde_json::Value = serde_json::from_str(
            r#"[{"T":1700003599999,"c":"3","h":"4","i":"1h","l":"1","n":12,"o":"2","s":"BTC","t":1700000000000,"v":"20"},
                {"T":1699999999999,"c":"2","h":"2","i":"1h","l":"0.5","n":3,"o":"1","s":"BTC","t":1699996400000,"v":"10"}]"#,
        )
        .unwrap();
        let b = parse_candles(&v).unwrap();
        assert_eq!(b.len(), 2);
        assert_eq!(
            (b[0].time, b[0].close, b[1].high, b[1].volume),
            (1_699_996_400_000, 2.0, 4.0, 20.0)
        );
        assert!(parse_candles(&serde_json::Value::Null).is_err());
        let ws: serde_json::Value = serde_json::from_str(
            r#"{"channel":"candle","data":{"t":1700000000000,"T":1700000059999,"s":"BTC","i":"1m",
                "o":"1","c":"2","h":"3","l":"0.5","v":"7","n":4}}"#,
        )
        .unwrap();
        let k = parse_ws(&ws);
        assert_eq!((k[0].open, k[0].high, k[0].volume), (1.0, 3.0, 7.0));
        assert!(parse_ws(&serde_json::json!({"channel":"pong"})).is_empty());
        assert!(parse_ws(&serde_json::json!({"channel":"subscriptionResponse","data":{}})).is_empty());
    }

    #[test]
    fn lists_perps_with_their_steps_and_funding() {
        let v: serde_json::Value = serde_json::from_str(
            r#"[{"universe":[{"name":"BTC","szDecimals":5,"maxLeverage":40},
                             {"name":"OLD","szDecimals":0,"maxLeverage":3,"isDelisted":true},
                             {"name":"kPEPE","szDecimals":0,"maxLeverage":10},
                             {"name":"ETH","szDecimals":4,"maxLeverage":25}]},
                [{"funding":"0.0000125","markPx":"97012.0"},{"funding":"0","markPx":"1"},
                 {"funding":"-0.00002","markPx":"0.012345"},{"funding":"0.00001","markPx":"3456.7"}]]"#,
        )
        .unwrap();
        let list = parse_perps(&v, "USDC");
        let ids: Vec<&str> = list.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "HYPERLIQUID:BTCUSDC.P",
                "HYPERLIQUID:kPEPEUSDC.P",
                "HYPERLIQUID:ETHUSDC.P"
            ]
        );
        // BTC: whole dollars; kPEPE: five figures (0.000001); ETH: 0.1.
        assert_eq!(list[0].tick_size, 1.0);
        assert!((list[1].tick_size - 0.000001).abs() < 1e-12);
        assert!((list[2].tick_size - 0.1).abs() < 1e-12);
        assert!((list[0].min_qty - 0.00001).abs() < 1e-12);
        assert_eq!(parse_funding(&v, "BTC"), Some(0.0000125));
        assert_eq!(parse_funding(&v, "kPEPE"), Some(-0.00002));
        assert_eq!(parse_funding(&v, "NOPE"), None);
    }

    #[test]
    fn steps_follow_the_price() {
        // Five significant figures, capped by 6 - szDecimals decimals.
        let tick = opentern_core::hyperliquid_tick;
        assert!((tick(1.2345, 2) - 0.0001).abs() < 1e-12);
        assert!((tick(0.5, 0) - 0.00001).abs() < 1e-12);
        assert!((tick(0.5, 3) - 0.001).abs() < 1e-12);
        assert_eq!(tick(250_000.0, 5), 1.0);
    }
}

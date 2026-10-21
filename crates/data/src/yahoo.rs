//! Yahoo Finance's public chart API, for stocks, ETFs, indices and forex.
//! Unofficial and best effort: history is limited for short intervals
//! (1m: 7 days, up to 30m: 60 days, 1h: 2 years) and quotes may be delayed.
//! "Live" is a poll every 30 seconds.

use crate::provider::{self, Error, Live, Provider, Result};
use crate::resample::to_utc_midnight;
use opentern_core::{AssetKind, Bar, Symbol, Timeframe};
use std::time::{Duration, Instant};

const CHART: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const SEARCH: &str = "https://query2.finance.yahoo.com/v1/finance/search";
const POLL: Duration = Duration::from_secs(30);

pub struct Yahoo {
    agent: ureq::Agent,
}

impl Default for Yahoo {
    fn default() -> Self {
        Yahoo {
            agent: provider::agent(),
        }
    }
}

/// Our symbol id → Yahoo's ticker.
pub fn ticker(id: &str) -> String {
    let (ex, t) = id.split_once(':').unwrap_or(("", id));
    match (ex, t) {
        ("NSE", "NIFTY") => "^NSEI".into(),
        ("NSE", "BANKNIFTY") => "^NSEBANK".into(),
        ("BSE", "SENSEX") => "^BSESN".into(),
        ("NSE", t) => format!("{t}.NS"),
        ("BSE", t) => format!("{t}.BO"),
        ("FX", t) => format!("{t}=X"),
        // Commodities: TradingView's continuous contract → Yahoo's front month.
        ("COMEX" | "NYMEX" | "CBOT" | "CME" | "ICEUS" | "ICEEUR", t) if t.ends_with("1!") => match &t[..t.len() - 2] {
            "BRN" => "BZ=F".into(),
            root => format!("{root}=F"),
        },
        (_, t) => t.into(),
    }
}

/// A Yahoo exchange code and ticker → our symbol id.
fn id_for(exchange: &str, symbol: &str) -> String {
    let base = |s: &str, suffix: &str| s.strip_suffix(suffix).unwrap_or(s).to_string();
    match exchange {
        "NMS" | "NGM" | "NCM" | "NAS" => format!("NASDAQ:{symbol}"),
        "NYQ" | "NYS" => format!("NYSE:{symbol}"),
        "PCX" | "ASE" | "BTS" => format!("AMEX:{symbol}"),
        "NSI" if symbol == "^NSEI" => "NSE:NIFTY".into(),
        "NSI" if symbol == "^NSEBANK" => "NSE:BANKNIFTY".into(),
        "NSI" => format!("NSE:{}", base(symbol, ".NS")),
        "BSE" if symbol == "^BSESN" => "BSE:SENSEX".into(),
        "BSE" => format!("BSE:{}", base(symbol, ".BO")),
        "CCY" => format!("FX:{}", base(symbol, "=X")),
        // A front-month future we list as a commodity (GC=F → COMEX:GC1!).
        _ if symbol.ends_with("=F")
            && let Some(c) = opentern_core::commodities()
                .into_iter()
                .find(|c| ticker(&c.id) == symbol) =>
        {
            c.id
        }
        _ => format!("YAHOO:{symbol}"),
    }
}

fn kind(quote_type: &str) -> AssetKind {
    match quote_type {
        "ETF" | "MUTUALFUND" => AssetKind::Fund,
        "INDEX" => AssetKind::Index,
        "CURRENCY" => AssetKind::Forex,
        "CRYPTOCURRENCY" => AssetKind::Crypto,
        "FUTURE" => AssetKind::Futures,
        _ => AssetKind::Stock,
    }
}

/// Yahoo's interval name and how far back it serves that interval.
fn interval(tf: Timeframe) -> Option<(&'static str, &'static str)> {
    use opentern_core::Unit::*;
    Some(match (tf.unit, tf.count) {
        (Minute, 1) => ("1m", "7d"),
        (Minute, 2) => ("2m", "60d"),
        (Minute, 5) => ("5m", "60d"),
        (Minute, 15) => ("15m", "60d"),
        (Minute, 30) => ("30m", "60d"),
        (Minute, 90) => ("90m", "60d"),
        (Hour, 1) => ("60m", "730d"),
        (Day, 1) => ("1d", "max"),
        (Week, 1) => ("1wk", "max"),
        (Month, 1) => ("1mo", "max"),
        _ => return None,
    })
}

/// Parses a chart reply into bars (skipping gaps Yahoo fills with nulls)
/// and the symbol details in its `meta`.
pub(crate) fn parse_chart(v: &serde_json::Value, daily: bool) -> Result<(Vec<Bar>, serde_json::Value)> {
    let r = &v["chart"]["result"][0];
    if r.is_null() {
        let msg = v["chart"]["error"]["description"]
            .as_str()
            .unwrap_or("No data for this symbol.");
        return Err(Error::Bad(format!("Yahoo: {msg}")));
    }
    let meta = r["meta"].clone();
    let tz = meta["exchangeTimezoneName"]
        .as_str()
        .and_then(|n| jiff::tz::TimeZone::get(n).ok())
        .unwrap_or(jiff::tz::TimeZone::UTC);
    let empty = Vec::new();
    let times = r["timestamp"].as_array().unwrap_or(&empty);
    let q = &r["indicators"]["quote"][0];
    let col = |k: &str, i: usize| q[k][i].as_f64();
    let mut bars = Vec::with_capacity(times.len());
    for (i, t) in times.iter().enumerate() {
        let Some(t) = t.as_i64() else { continue };
        let (Some(open), Some(high), Some(low), Some(close)) =
            (col("open", i), col("high", i), col("low", i), col("close", i))
        else {
            continue;
        };
        let ms = t * 1000;
        bars.push(Bar {
            time: if daily { to_utc_midnight(ms, &tz) } else { ms },
            open,
            high,
            low,
            close,
            volume: col("volume", i).unwrap_or(0.0),
        });
    }
    // Yahoo can repeat the last bar (once as a bar, once as "now").
    bars.dedup_by_key(|b| b.time);
    Ok((bars, meta))
}

/// Dividends and splits, dated like daily bars (00:00 UTC of the
/// exchange-local date).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Corporate {
    /// (ex-date, amount per share).
    pub dividends: Vec<(i64, f64)>,
    /// (date, ratio new shares per old: 4 for a 4-for-1 split).
    pub splits: Vec<(i64, f64)>,
}

pub(crate) fn parse_events(v: &serde_json::Value) -> Corporate {
    let r = &v["chart"]["result"][0];
    let tz = r["meta"]["exchangeTimezoneName"]
        .as_str()
        .and_then(|n| jiff::tz::TimeZone::get(n).ok())
        .unwrap_or(jiff::tz::TimeZone::UTC);
    let day = |e: &serde_json::Value| e["date"].as_i64().map(|t| to_utc_midnight(t * 1000, &tz));
    let mut c = Corporate::default();
    if let Some(m) = r["events"]["dividends"].as_object() {
        c.dividends = m
            .values()
            .filter_map(|e| Some((day(e)?, e["amount"].as_f64()?)))
            .collect();
    }
    if let Some(m) = r["events"]["splits"].as_object() {
        c.splits = m
            .values()
            .filter_map(|e| {
                let (n, d) = (e["numerator"].as_f64()?, e["denominator"].as_f64()?);
                (d > 0.0).then_some((day(e)?, n / d))
            })
            .collect();
    }
    c.dividends.sort_by_key(|x| x.0);
    c.splits.sort_by_key(|x| x.0);
    c
}

/// Back-adjusts prices for dividends, as TradingView's ADJ: each bar
/// before an ex-date is scaled by (1 − dividend / previous close). Yahoo's
/// prices are already split-adjusted. `tz` dates intraday bars.
pub fn dividend_adjust(bars: &[Bar], dividends: &[(i64, f64)], tz: &jiff::tz::TimeZone) -> Vec<Bar> {
    let day = |t: i64| to_utc_midnight(t, tz);
    let days: Vec<i64> = bars.iter().map(|b| day(b.time)).collect();
    let mut factor = vec![1.0f64; bars.len()];
    for &(ex, amount) in dividends {
        // The last bar before the ex-date carries the reference close.
        let Some(k) = days.iter().rposition(|d| *d < ex) else {
            continue;
        };
        if k + 1 >= bars.len() {
            continue;
        }
        let close = bars[k].close;
        if close <= 0.0 || amount <= 0.0 || amount >= close {
            continue;
        }
        let f = 1.0 - amount / close;
        for x in factor.iter_mut().take(k + 1) {
            *x *= f;
        }
    }
    bars.iter()
        .zip(factor)
        .map(|(b, f)| Bar {
            open: b.open * f,
            high: b.high * f,
            low: b.low * f,
            close: b.close * f,
            ..*b
        })
        .collect()
}

impl Yahoo {
    /// The symbol's whole dividend and split history.
    pub fn corporate(&self, sym: &Symbol) -> Result<Corporate> {
        let url = format!(
            "{CHART}/{}?interval=1d&period1=0&period2={}&events=div%7Csplit",
            urlencode(&ticker(&sym.id)),
            provider::now_ms() / 1000
        );
        Ok(parse_events(&provider::get_json(&self.agent, &url)?))
    }

    fn chart(&self, sym: &Symbol, tf: Timeframe, range: &str) -> Result<(Vec<Bar>, serde_json::Value)> {
        let (iv, _) = interval(tf).ok_or_else(|| Error::Bad(format!("Yahoo has no {} bars", tf.label())))?;
        // `range=max` quietly switches to 3-month bars, so ask for the whole
        // history by dates instead.
        let span = if range == "max" {
            format!("period1=0&period2={}", provider::now_ms() / 1000)
        } else {
            format!("range={range}")
        };
        let url = format!(
            "{CHART}/{}?interval={iv}&{span}&includePrePost=false",
            urlencode(&ticker(&sym.id))
        );
        let (bars, meta) = parse_chart(&provider::get_json(&self.agent, &url)?, !tf.is_intraday())?;
        // Never pass off bars of another length as the ones asked for.
        if let Some(g) = meta["dataGranularity"].as_str()
            && g != iv
            && !(g == "1h" && iv == "60m")
        {
            return Err(Error::Bad(format!("Yahoo sent {g} bars instead of {iv}.")));
        }
        Ok((bars, meta))
    }

    /// Searches Yahoo for stocks, funds, indices and currencies.
    pub fn search(&self, query: &str) -> Result<Vec<Symbol>> {
        let url = format!("{SEARCH}?q={}&quotesCount=20&newsCount=0", urlencode(query));
        let v = provider::get_json(&self.agent, &url)?;
        let empty = Vec::new();
        Ok(v["quotes"]
            .as_array()
            .unwrap_or(&empty)
            .iter()
            .filter_map(|q| {
                let symbol = q["symbol"].as_str()?;
                let exchange = q["exchange"].as_str().unwrap_or("");
                let qt = q["quoteType"].as_str().unwrap_or("EQUITY");
                if qt == "CRYPTOCURRENCY" || qt == "OPTION" {
                    return None;
                }
                let id = id_for(exchange, symbol);
                let tz = match id.split(':').next() {
                    Some("NSE" | "BSE") => "Asia/Kolkata",
                    Some("NASDAQ" | "NYSE" | "AMEX") => "America/New_York",
                    _ => "UTC",
                };
                Some(Symbol {
                    description: q["longname"]
                        .as_str()
                        .or(q["shortname"].as_str())
                        .unwrap_or(symbol)
                        .to_string(),
                    kind: kind(qt),
                    currency: String::new(),
                    tick_size: 0.01,
                    min_qty: 0.0,
                    timezone: tz.into(),
                    id,
                })
            })
            .collect())
    }

    /// Last price and previous close.
    pub fn quote(&self, sym: &Symbol) -> Result<(f64, f64)> {
        let url = format!("{CHART}/{}?interval=1d&range=1d", urlencode(&ticker(&sym.id)));
        let (_, meta) = parse_chart(&provider::get_json(&self.agent, &url)?, true)?;
        let last = meta["regularMarketPrice"].as_f64();
        let prev = meta["chartPreviousClose"].as_f64().or(meta["previousClose"].as_f64());
        match (last, prev) {
            (Some(l), Some(p)) => Ok((l, p)),
            _ => Err(Error::Bad("Yahoo: no quote".into())),
        }
    }
}

impl Provider for Yahoo {
    fn natives(&self) -> Vec<Timeframe> {
        ["1m", "2m", "5m", "15m", "30m", "90m", "1h", "1D", "1W", "1M"]
            .iter()
            .filter_map(|s| Timeframe::parse(s))
            .collect()
    }

    /// Everything Yahoo has for the interval: it can't page further back,
    /// so there's no point holding any of it back.
    fn latest(&self, sym: &Symbol, tf: Timeframe, _n: usize) -> Result<Vec<Bar>> {
        let (_, range) = interval(tf).ok_or_else(|| Error::Bad(format!("Yahoo has no {} bars", tf.label())))?;
        Ok(self.chart(sym, tf, range)?.0)
    }

    fn since(&self, sym: &Symbol, tf: Timeframe, start: i64) -> Result<Vec<Bar>> {
        let age = provider::now_ms() - start;
        let range = match age / 86_400_000 {
            0..=4 if tf.is_intraday() => "5d",
            0..=25 => "1mo",
            _ => interval(tf).map(|(_, r)| r).unwrap_or("max"),
        };
        let (bars, _) = self.chart(sym, tf, range)?;
        Ok(bars.into_iter().filter(|b| b.time >= start).collect())
    }

    /// Yahoo only serves its fixed range per interval, all at once.
    fn before(&self, _: &Symbol, _: Timeframe, _: i64, _: usize) -> Result<Vec<Bar>> {
        Ok(Vec::new())
    }

    fn stream(&self, sym: &Symbol, tf: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()> {
        let mut last_poll = Instant::now();
        let mut since = provider::now_ms() - tf.millis().unwrap_or(86_400_000) * 2;
        loop {
            if last_poll.elapsed() >= POLL {
                last_poll = Instant::now();
                for b in self.since(sym, tf, since)? {
                    since = b.time;
                    if !sink(Live::Bar(b)) {
                        return Ok(());
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(250));
            if !sink(Live::Idle) {
                return Ok(());
            }
        }
    }
}

pub(crate) fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dividends_splits_and_adjustment() {
        let v = serde_json::json!({"chart":{"result":[{"meta":{"exchangeTimezoneName":"America/New_York"},
            "events":{"dividends":{"1":{"amount":0.25,"date":1704292200}},
                      "splits":{"2":{"date":1598880600,"numerator":4,"denominator":1,"splitRatio":"4:1"}}}}]}});
        let c = parse_events(&v);
        // 2024-01-03 14:30 UTC → dated 2024-01-03 00:00 UTC.
        assert_eq!(c.dividends, vec![(1_704_240_000_000, 0.25)]);
        assert_eq!(c.splits[0].1, 4.0);
        let day = 86_400_000;
        let bars: Vec<Bar> = (0..4)
            .map(|i| Bar {
                time: 1_704_067_200_000 + i * day,
                open: 10.0,
                high: 10.0,
                low: 10.0,
                close: 10.0,
                volume: 1.0,
            })
            .collect();
        // Ex-date 2024-01-03 (index 2): bars 0 and 1 scale by 1 − 0.25/10.
        let adj = dividend_adjust(&bars, &c.dividends, &jiff::tz::TimeZone::UTC);
        assert_eq!((adj[0].close, adj[1].close, adj[2].close), (9.75, 9.75, 10.0));
        assert_eq!(adj[0].volume, 1.0);
    }

    #[test]
    fn maps_ids_both_ways() {
        assert_eq!(ticker("NASDAQ:AAPL"), "AAPL");
        assert_eq!(ticker("NSE:RELIANCE"), "RELIANCE.NS");
        assert_eq!(ticker("NSE:NIFTY"), "^NSEI");
        assert_eq!(ticker("COMEX:GC1!"), "GC=F");
        assert_eq!(ticker("ICEEUR:BRN1!"), "BZ=F");
        assert_eq!(id_for("CMX", "GC=F"), "COMEX:GC1!");
        assert_eq!(ticker("FX:EURUSD"), "EURUSD=X");
        assert_eq!(id_for("NSI", "RELIANCE.NS"), "NSE:RELIANCE");
        assert_eq!(id_for("NSI", "^NSEI"), "NSE:NIFTY");
        assert_eq!(id_for("NMS", "AAPL"), "NASDAQ:AAPL");
        assert_eq!(id_for("CCY", "EURUSD=X"), "FX:EURUSD");
        assert_eq!(id_for("FGI", "X"), "YAHOO:X");
        assert_eq!(ticker("YAHOO:^GSPC"), "^GSPC");
        assert_eq!(urlencode("^GSPC"), "%5EGSPC");
    }

    #[test]
    fn parses_chart_skipping_nulls_and_dating_daily_bars() {
        // Two daily bars stamped 09:30 New York; the middle one is a gap.
        let v: serde_json::Value = serde_json::from_str(
            r#"{"chart":{"result":[{"meta":{"exchangeTimezoneName":"America/New_York","regularMarketPrice":3.0,"chartPreviousClose":2.0},
            "timestamp":[1704810600,1704897000,1704983400],
            "indicators":{"quote":[{"open":[1.0,null,2.0],"high":[1.5,null,2.5],"low":[0.5,null,1.5],"close":[1.2,null,2.2],"volume":[100,null,200]}]}}],"error":null}}"#,
        )
        .unwrap();
        let (bars, meta) = parse_chart(&v, true).unwrap();
        assert_eq!(bars.len(), 2);
        // 2024-01-09 00:00 UTC
        assert_eq!(bars[0].time, 1_704_758_400_000);
        assert_eq!(bars[1].close, 2.2);
        assert_eq!(meta["regularMarketPrice"].as_f64(), Some(3.0));
        let none: serde_json::Value =
            serde_json::from_str(r#"{"chart":{"result":null,"error":{"description":"No data found"}}}"#).unwrap();
        assert!(parse_chart(&none, true).is_err());
    }
}

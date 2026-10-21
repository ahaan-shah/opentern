//! Lists and headlines for the side widgets: hotlists (top gainers,
//! losers, volume leaders) and news for a symbol. Both are best effort
//! from public endpoints (Binance's 24-hour tickers, Yahoo's predefined
//! screeners, spark quotes for the NIFTY 50, and search); in demo mode
//! they're made up.

use crate::provider::{self, Error, Result};
use opentern_core::{AssetKind, Symbol};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotMarket {
    Crypto,
    UsStocks,
    /// Indian stocks: the NIFTY 50's movers (no volume list: Yahoo's batch
    /// quotes don't carry it).
    India,
}

/// The NIFTY 50, by NSE ticker.
const NIFTY50: [&str; 50] = [
    "RELIANCE",
    "TCS",
    "HDFCBANK",
    "ICICIBANK",
    "INFY",
    "BHARTIARTL",
    "ITC",
    "SBIN",
    "LT",
    "HINDUNILVR",
    "KOTAKBANK",
    "AXISBANK",
    "BAJFINANCE",
    "MARUTI",
    "SUNPHARMA",
    "HCLTECH",
    "TITAN",
    "ASIANPAINT",
    "WIPRO",
    "ULTRACEMCO",
    "NTPC",
    "POWERGRID",
    "ONGC",
    "TATAMOTORS",
    "TATASTEEL",
    "JSWSTEEL",
    "M&M",
    "ADANIENT",
    "ADANIPORTS",
    "COALINDIA",
    "BAJAJFINSV",
    "NESTLEIND",
    "TECHM",
    "GRASIM",
    "HINDALCO",
    "CIPLA",
    "DRREDDY",
    "BRITANNIA",
    "EICHERMOT",
    "HEROMOTOCO",
    "APOLLOHOSP",
    "DIVISLAB",
    "INDUSINDBK",
    "SBILIFE",
    "HDFCLIFE",
    "TATACONSUM",
    "BAJAJ-AUTO",
    "SHRIRAMFIN",
    "TRENT",
    "BEL",
];

fn india_symbol(t: &str) -> Symbol {
    Symbol {
        id: format!("NSE:{t}"),
        description: String::new(),
        kind: AssetKind::Stock,
        currency: "INR".into(),
        tick_size: 0.05,
        min_qty: 0.0,
        timezone: "Asia/Kolkata".into(),
    }
}

/// Parses Yahoo's spark reply (`{"TCS.NS": {"fulldayPrice": …, "fulldayChangePercent": …}}`).
pub(crate) fn parse_spark(v: &serde_json::Value) -> Vec<HotRow> {
    v.as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, q)| {
            let t = k.strip_suffix(".NS")?;
            Some(HotRow {
                symbol: india_symbol(t),
                last: q["fulldayPrice"].as_f64()?,
                change_pct: q["fulldayChangePercent"].as_f64()?,
                volume: 0.0,
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotKind {
    Gainers,
    Losers,
    Volume,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HotRow {
    pub symbol: Symbol,
    pub last: f64,
    pub change_pct: f64,
    /// In the quote currency (crypto) or shares (stocks).
    pub volume: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NewsItem {
    pub title: String,
    pub publisher: String,
    pub link: String,
    /// Unix ms.
    pub time: i64,
}

fn crypto_symbol(t: &str) -> Symbol {
    Symbol {
        id: format!("BINANCE:{t}"),
        description: String::new(),
        kind: AssetKind::Crypto,
        currency: "USDT".into(),
        tick_size: 0.01,
        min_qty: 0.0,
        timezone: "UTC".into(),
    }
}

fn us_symbol(exchange: &str, t: &str, name: &str) -> Symbol {
    let ex = match exchange {
        "NMS" | "NGM" | "NCM" | "NAS" => "NASDAQ",
        "NYQ" | "NYS" => "NYSE",
        _ => "AMEX",
    };
    Symbol {
        id: format!("{ex}:{t}"),
        description: name.into(),
        kind: AssetKind::Stock,
        currency: "USD".into(),
        tick_size: 0.01,
        min_qty: 0.0,
        timezone: "America/New_York".into(),
    }
}

/// Stablecoins and leveraged tokens aren't worth listing as movers.
fn listable(t: &str) -> bool {
    let Some(base) = t.strip_suffix("USDT") else {
        return false;
    };
    !base.is_empty()
        && ![
            "USDC", "FDUSD", "TUSD", "BUSD", "DAI", "USDP", "EUR", "PAX", "USDE", "AEUR",
        ]
        .contains(&base)
        && !base.ends_with("UP")
        && !base.ends_with("DOWN")
        && !base.ends_with("BULL")
        && !base.ends_with("BEAR")
}

fn sort_rows(rows: &mut Vec<HotRow>, kind: HotKind, n: usize) {
    match kind {
        HotKind::Gainers => rows.sort_by(|a, b| b.change_pct.total_cmp(&a.change_pct)),
        HotKind::Losers => rows.sort_by(|a, b| a.change_pct.total_cmp(&b.change_pct)),
        HotKind::Volume => rows.sort_by(|a, b| (b.volume * b.last).total_cmp(&(a.volume * a.last))),
    }
    rows.truncate(n);
}

/// Parses Binance's `/api/v3/ticker/24hr` reply.
pub(crate) fn parse_binance_24h(v: &serde_json::Value, kind: HotKind) -> Vec<HotRow> {
    let num = |x: &serde_json::Value| x.as_str().and_then(|s| s.parse::<f64>().ok());
    let mut rows: Vec<HotRow> = v
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|t| {
            let sym = t["symbol"].as_str()?;
            if !listable(sym) {
                return None;
            }
            let last = num(&t["lastPrice"])?;
            let quote_vol = num(&t["quoteVolume"])?;
            // Pairs that don't trade any more report zeros.
            if last <= 0.0 || quote_vol <= 0.0 {
                return None;
            }
            Some(HotRow {
                symbol: crypto_symbol(sym),
                last,
                change_pct: num(&t["priceChangePercent"])?,
                volume: quote_vol / last,
            })
        })
        .collect();
    sort_rows(&mut rows, kind, 30);
    rows
}

/// Parses Yahoo's predefined screener reply.
pub(crate) fn parse_yahoo_screener(v: &serde_json::Value) -> Vec<HotRow> {
    v["finance"]["result"][0]["quotes"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|q| {
            let t = q["symbol"].as_str()?;
            let name = q["shortName"].as_str().or(q["longName"].as_str()).unwrap_or(t);
            Some(HotRow {
                symbol: us_symbol(q["exchange"].as_str().unwrap_or(""), t, name),
                last: q["regularMarketPrice"].as_f64()?,
                change_pct: q["regularMarketChangePercent"].as_f64().unwrap_or(0.0),
                volume: q["regularMarketVolume"].as_f64().unwrap_or(0.0),
            })
        })
        .collect()
}

pub fn hotlist(market: HotMarket, kind: HotKind, demo: bool) -> Result<Vec<HotRow>> {
    if demo {
        return Ok(demo_hotlist(market, kind));
    }
    let agent = provider::agent();
    match market {
        HotMarket::Crypto => {
            let v = provider::get_json(&agent, "https://api.binance.com/api/v3/ticker/24hr")?;
            Ok(parse_binance_24h(&v, kind))
        }
        HotMarket::UsStocks => {
            let id = match kind {
                HotKind::Gainers => "day_gainers",
                HotKind::Losers => "day_losers",
                HotKind::Volume => "most_actives",
            };
            let url =
                format!("https://query1.finance.yahoo.com/v1/finance/screener/predefined/saved?scrIds={id}&count=30");
            let rows = parse_yahoo_screener(&provider::get_json(&agent, &url)?);
            if rows.is_empty() {
                return Err(Error::Bad(
                    "Yahoo didn't send a list this time. Try again later.".into(),
                ));
            }
            Ok(rows)
        }
        HotMarket::India => {
            // Twenty symbols a request is what spark takes.
            let mut rows = Vec::new();
            for chunk in NIFTY50.chunks(20) {
                let list: Vec<String> = chunk
                    .iter()
                    .map(|t| crate::yahoo::urlencode(&format!("{t}.NS")))
                    .collect();
                let url = format!(
                    "https://query1.finance.yahoo.com/v8/finance/spark?symbols={}&range=1d&interval=1d",
                    list.join(",")
                );
                rows.extend(parse_spark(&provider::get_json(&agent, &url)?));
            }
            if rows.is_empty() {
                return Err(Error::Bad(
                    "Yahoo didn't send prices this time. Try again later.".into(),
                ));
            }
            sort_rows(&mut rows, kind, 30);
            Ok(rows)
        }
    }
}

fn demo_hotlist(market: HotMarket, kind: HotKind) -> Vec<HotRow> {
    let names: &[&str] = match market {
        HotMarket::Crypto => &[
            "BTCUSDT", "ETHUSDT", "SOLUSDT", "XRPUSDT", "DOGEUSDT", "ADAUSDT", "LINKUSDT", "AVAXUSDT", "DOTUSDT",
            "LTCUSDT",
        ],
        HotMarket::UsStocks => &[
            "AAPL", "MSFT", "NVDA", "AMZN", "GOOGL", "META", "TSLA", "AMD", "NFLX", "JPM",
        ],
        HotMarket::India => &NIFTY50[..10],
    };
    let mut rows: Vec<HotRow> = names
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let k = (i as f64 * 1.7).sin();
            HotRow {
                symbol: match market {
                    HotMarket::Crypto => crypto_symbol(t),
                    HotMarket::UsStocks => us_symbol("NMS", t, t),
                    HotMarket::India => india_symbol(t),
                },
                last: 50.0 + i as f64 * 37.3,
                change_pct: k * 6.0,
                volume: 1e6 * (1.0 + i as f64),
            }
        })
        .collect();
    sort_rows(&mut rows, kind, 30);
    rows
}

/// The query Yahoo's search understands for a symbol's news.
fn news_query(sym: &Symbol) -> String {
    match sym.exchange() {
        "BINANCE" | "BYBIT" | "OKX" | "COINBASE" | "KRAKEN" => {
            let t = sym.ticker();
            let base = ["USDT", "USDC", "USD", "BTC", "EUR"]
                .iter()
                .find_map(|q| t.strip_suffix(q).filter(|b| !b.is_empty()))
                .unwrap_or(t);
            format!("{base}-USD")
        }
        _ => crate::yahoo::ticker(&sym.id),
    }
}

pub(crate) fn parse_news(v: &serde_json::Value) -> Vec<NewsItem> {
    v["news"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|n| {
            Some(NewsItem {
                title: n["title"].as_str()?.to_string(),
                publisher: n["publisher"].as_str().unwrap_or("").to_string(),
                link: n["link"].as_str()?.to_string(),
                time: n["providerPublishTime"].as_i64().unwrap_or(0) * 1000,
            })
        })
        .collect()
}

/// Headlines from an RSS 2.0 feed (title, link, pubDate per item).
pub(crate) fn parse_rss(xml: &str, publisher: &str) -> Vec<NewsItem> {
    let tag = |item: &str, name: &str| -> Option<String> {
        let open = format!("<{name}>");
        let close = format!("</{name}>");
        let a = item.find(&open)? + open.len();
        let b = a + item[a..].find(&close)?;
        let raw = item[a..b].trim();
        let raw = raw
            .strip_prefix("<![CDATA[")
            .and_then(|x| x.strip_suffix("]]>"))
            .unwrap_or(raw);
        Some(
            raw.replace("&amp;", "&")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&#39;", "'")
                .replace("&#039;", "'")
                .trim()
                .to_string(),
        )
    };
    xml.split("<item>")
        .skip(1)
        .filter_map(|item| {
            let title = tag(item, "title")?;
            let link = tag(item, "link")?;
            let time = tag(item, "pubDate")
                .and_then(|d| jiff::fmt::rfc2822::parse(&d).ok())
                .map(|z| z.timestamp().as_millisecond())
                .unwrap_or(0);
            Some(NewsItem {
                title,
                publisher: publisher.into(),
                link,
                time,
            })
        })
        .collect()
}

fn get_text(url: &str) -> Result<String> {
    let mut resp = provider::agent().get(url).call().map_err(|_| Error::Offline)?;
    resp.body_mut()
        .with_config()
        .limit(8 * 1024 * 1024)
        .read_to_string()
        .map_err(|_| Error::Offline)
}

pub fn news(sym: &Symbol, demo: bool) -> Result<Vec<NewsItem>> {
    if demo {
        let now = provider::now_ms();
        return Ok((0..6)
            .map(|i| NewsItem {
                title: format!("Sample headline {} about {}", i + 1, sym.ticker()),
                publisher: "Demo".into(),
                link: "https://example.com".into(),
                time: now - i * 3_600_000,
            })
            .collect());
    }
    // Yahoo has no crypto headlines; CoinDesk's public feed does.
    if sym.kind == AssetKind::Crypto {
        let mut items = parse_rss(
            &get_text("https://www.coindesk.com/arc/outboundfeeds/rss/")?,
            "CoinDesk",
        );
        // Headlines naming the coin first.
        let base = news_query(sym).trim_end_matches("-USD").to_lowercase();
        let names: &[&str] = match base.as_str() {
            "btc" => &["bitcoin", "btc"],
            "eth" => &["ether", "eth"],
            "sol" => &["solana", "sol"],
            "xrp" => &["xrp", "ripple"],
            "doge" => &["doge"],
            _ => &[],
        };
        let mentions = |n: &NewsItem| {
            let t = n.title.to_lowercase();
            names.iter().any(|k| t.contains(k)) || (!base.is_empty() && t.contains(&base))
        };
        items.sort_by_key(|n| (!mentions(n), -n.time));
        items.truncate(20);
        return Ok(items);
    }
    let q = news_query(sym);
    let url = format!(
        "https://query2.finance.yahoo.com/v1/finance/search?q={}&quotesCount=0&newsCount=15",
        q.replace('^', "%5E").replace('=', "%3D")
    );
    let v = provider::get_json(&provider::agent(), &url)?;
    Ok(parse_news(&v))
}

#[cfg(test)]
mod tests {

    #[test]
    fn parses_spark_quotes() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"TCS.NS":{"symbol":"TCS.NS","fulldayChangePercent":2.767,"fulldayPrice":2529.0},
                "M&M.NS":{"symbol":"M&M.NS","fulldayChangePercent":-1.2,"fulldayPrice":3000.5},
                "BAD.NS":{"symbol":"BAD.NS","fulldayPrice":null}}"#,
        )
        .unwrap();
        let mut rows = parse_spark(&v);
        sort_rows(&mut rows, HotKind::Gainers, 30);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].symbol.id, "NSE:TCS");
        assert_eq!(rows[1].symbol.id, "NSE:M&M");
        assert_eq!(rows[0].last, 2529.0);
    }

    use super::*;

    #[test]
    fn binance_movers_skip_stablecoins_and_dead_pairs() {
        let v: serde_json::Value = serde_json::from_str(
            r#"[
            {"symbol":"BTCUSDT","lastPrice":"60000","priceChangePercent":"2.5","quoteVolume":"900000000"},
            {"symbol":"USDCUSDT","lastPrice":"1","priceChangePercent":"0.01","quoteVolume":"999999999"},
            {"symbol":"OLDUSDT","lastPrice":"0","priceChangePercent":"0","quoteVolume":"0"},
            {"symbol":"ETHBTC","lastPrice":"0.05","priceChangePercent":"9","quoteVolume":"10"},
            {"symbol":"SOLUSDT","lastPrice":"150","priceChangePercent":"-4","quoteVolume":"300000000"}
        ]"#,
        )
        .unwrap();
        let g = parse_binance_24h(&v, HotKind::Gainers);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].symbol.id, "BINANCE:BTCUSDT");
        let l = parse_binance_24h(&v, HotKind::Losers);
        assert_eq!(l[0].symbol.id, "BINANCE:SOLUSDT");
    }

    #[test]
    fn news_and_screener_replies_parse() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"news":[{"title":"A","publisher":"P","link":"https://x","providerPublishTime":1700000000}]}"#,
        )
        .unwrap();
        let n = parse_news(&v);
        assert_eq!(n[0].time, 1_700_000_000_000);
        let v: serde_json::Value = serde_json::from_str(
            r#"{"finance":{"result":[{"quotes":[{"symbol":"AAPL","exchange":"NMS","shortName":"Apple","regularMarketPrice":200.5,"regularMarketChangePercent":1.2,"regularMarketVolume":5000}]}]}}"#,
        )
        .unwrap();
        let r = parse_yahoo_screener(&v);
        assert_eq!(r[0].symbol.id, "NASDAQ:AAPL");
        let btc = Symbol::bare("BINANCE:BTCUSDT");
        assert_eq!(news_query(&btc), "BTC-USD");
        let rss = "<rss><channel><item><title><![CDATA[Bitcoin &amp; friends]]></title><link>https://a/b</link>\
                   <pubDate>Sat, 10 Oct 2026 06:45:51 +0000</pubDate></item></channel></rss>";
        let n = parse_rss(rss, "X");
        assert_eq!(n[0].title, "Bitcoin & friends");
        assert!(n[0].time > 1_790_000_000_000);
    }
}

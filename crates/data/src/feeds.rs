//! Lists and headlines for the side widgets: hotlists (top gainers,
//! losers, volume leaders) and news for a symbol. Both are best effort
//! from public endpoints (Binance's 24-hour tickers, Yahoo's predefined
//! screeners, spark quotes for the NIFTY 50, and search; headlines from
//! Yahoo, Google News search and a few public RSS feeds); in demo mode
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
            let v = crate::binance::get(&agent, "/api/v3/ticker/24hr")?;
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
            let mut title = tag(item, "title")?;
            let link = tag(item, "link")?;
            // Google News names each item's outlet in <source url="…">,
            // and repeats it at the end of the title.
            let source = item
                .find("<source")
                .and_then(|a| Some((a, item[a..].find('>')? + a + 1)))
                .and_then(|(_, b)| Some(item[b..b + item[b..].find("</source>")?].trim().to_string()))
                .filter(|s| !s.is_empty());
            if let Some(s) = &source
                && let Some(t) = title.strip_suffix(&format!(" - {s}"))
            {
                title = t.to_string();
            }
            let time = tag(item, "pubDate")
                .and_then(|d| jiff::fmt::rfc2822::parse(&d).ok())
                .map(|z| z.timestamp().as_millisecond())
                .unwrap_or(0);
            Some(NewsItem {
                title,
                publisher: source.unwrap_or_else(|| publisher.into()),
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

/// Coins' names, for news searches and for spotting them in headlines.
const COINS: [(&str, &str); 30] = [
    ("BTC", "Bitcoin"),
    ("ETH", "Ethereum"),
    ("SOL", "Solana"),
    ("XRP", "XRP"),
    ("BNB", "BNB"),
    ("DOGE", "Dogecoin"),
    ("ADA", "Cardano"),
    ("TRX", "Tron"),
    ("AVAX", "Avalanche"),
    ("LINK", "Chainlink"),
    ("DOT", "Polkadot"),
    ("TON", "Toncoin"),
    ("SHIB", "Shiba Inu"),
    ("LTC", "Litecoin"),
    ("BCH", "Bitcoin Cash"),
    ("XLM", "Stellar"),
    ("SUI", "Sui"),
    ("NEAR", "Near Protocol"),
    ("APT", "Aptos"),
    ("UNI", "Uniswap"),
    ("PEPE", "Pepe"),
    ("ATOM", "Cosmos"),
    ("ARB", "Arbitrum"),
    ("OP", "Optimism"),
    ("FIL", "Filecoin"),
    ("HBAR", "Hedera"),
    ("ICP", "Internet Computer"),
    ("AAVE", "Aave"),
    ("ZEC", "Zcash"),
    ("HYPE", "Hyperliquid"),
];

/// The name people write in headlines: "Reliance Industries" for
/// "Reliance Industries Limited", "Bitcoin" for BTCUSDT.
fn news_name(sym: &Symbol) -> String {
    if sym.kind == AssetKind::Crypto {
        let base = news_query(sym);
        let base = base.trim_end_matches("-USD");
        return COINS
            .iter()
            .find(|c| c.0 == base)
            .map(|c| c.1.to_string())
            .unwrap_or_else(|| base.to_string());
    }
    let mut name = sym.description.split(" / ").next().unwrap_or("").trim().to_string();
    if name.is_empty() || name == sym.ticker() {
        return sym.ticker().to_string();
    }
    for end in [
        " Limited",
        " Ltd.",
        " Ltd",
        ", Inc.",
        " Inc.",
        " Inc",
        " Corporation",
        " Corp.",
        " Corp",
        " Co.",
        " plc",
        " PLC",
        " Index",
    ] {
        if let Some(n) = name.strip_suffix(end) {
            name = n.trim_end_matches(',').to_string();
            break;
        }
    }
    name
}

fn indian(sym: &Symbol) -> bool {
    matches!(sym.exchange(), "NSE" | "BSE")
}

/// Google News search, last week.
fn google_news(query: &str, india: bool) -> Result<Vec<NewsItem>> {
    let (hl, gl) = if india { ("en-IN", "IN") } else { ("en-US", "US") };
    let q: String = format!("{query} when:7d")
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    let url = format!("https://news.google.com/rss/search?q={q}&hl={hl}&gl={gl}&ceid={gl}:en");
    Ok(parse_rss(&get_text(&url)?, "Google News"))
}

/// Does the headline name the symbol? Whole words only, so "SOL" doesn't
/// match "solution".
fn mentions(title: &str, terms: &[String]) -> bool {
    let words: Vec<String> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    terms.iter().any(|t| {
        let tw: Vec<&str> = t.split(' ').collect();
        words.windows(tw.len()).any(|w| w.iter().zip(&tw).all(|(a, b)| a == b))
    })
}

/// Same story from two sources: compare titles by letters and digits only.
fn title_key(t: &str) -> String {
    t.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .take(60)
        .collect()
}

type Fetch = Box<dyn FnOnce() -> Result<Vec<NewsItem>> + Send>;

/// Headlines for a symbol from several sources at once, newest first. A
/// source that fails is skipped; only when every one fails is it an error.
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
    let name = news_name(sym);
    let mut terms = vec![name.to_lowercase()];
    let ticker = match sym.kind {
        AssetKind::Crypto => news_query(sym).trim_end_matches("-USD").to_lowercase(),
        _ => sym.ticker().to_lowercase(),
    };
    if ticker.len() >= 3 && !terms.contains(&ticker) {
        terms.push(ticker);
    }
    let india = indian(sym);
    // General feeds keep only headlines naming the symbol; an Indian
    // index's news is the market's news, so it keeps them all.
    let feed = |url: &'static str, publisher: &'static str, all: bool| -> Fetch {
        let terms = terms.clone();
        Box::new(move || {
            let items = parse_rss(&get_text(url)?, publisher);
            Ok(items
                .into_iter()
                .filter(|n| all || mentions(&n.title, &terms))
                .collect())
        })
    };
    let mut sources: Vec<Fetch> = Vec::new();
    match sym.kind {
        AssetKind::Crypto => {
            let q = format!("\"{name}\" crypto");
            sources.push(Box::new(move || google_news(&q, false)));
            sources.push(feed(
                "https://www.coindesk.com/arc/outboundfeeds/rss/",
                "CoinDesk",
                false,
            ));
            sources.push(feed("https://cointelegraph.com/rss", "Cointelegraph", false));
            sources.push(feed("https://decrypt.co/feed", "Decrypt", false));
            sources.push(feed("https://www.theblock.co/rss.xml", "The Block", false));
        }
        _ => {
            let q = match sym.kind {
                AssetKind::Stock if india => format!("\"{name}\" share"),
                AssetKind::Stock | AssetKind::Fund => format!("\"{name}\" stock"),
                AssetKind::Commodity => format!("{name} price"),
                _ => name.clone(),
            };
            sources.push(Box::new(move || google_news(&q, india)));
            let yq = news_query(sym);
            sources.push(Box::new(move || {
                let url = format!(
                    "https://query2.finance.yahoo.com/v1/finance/search?q={}&quotesCount=0&newsCount=15",
                    yq.replace('^', "%5E").replace('=', "%3D")
                );
                Ok(parse_news(&provider::get_json(&provider::agent(), &url)?))
            }));
            if india {
                let all = sym.kind == AssetKind::Index;
                sources.push(feed(
                    "https://economictimes.indiatimes.com/markets/rssfeeds/1977021501.cms",
                    "The Economic Times",
                    all,
                ));
                sources.push(feed("https://www.livemint.com/rss/markets", "Mint", all));
            }
        }
    }
    let results: Vec<Result<Vec<NewsItem>>> = std::thread::scope(|s| {
        let handles: Vec<_> = sources.into_iter().map(|f| s.spawn(f)).collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or(Err(Error::Offline)))
            .collect()
    });
    let mut first_err = None;
    let mut items = Vec::new();
    for r in results {
        match r {
            Ok(v) => items.extend(v),
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }
    if items.is_empty()
        && let Some(e) = first_err
    {
        return Err(e);
    }
    Ok(merge(items, provider::now_ms()))
}

/// Newest first, a story once, nothing older than two weeks, at most 50.
fn merge(mut items: Vec<NewsItem>, now: i64) -> Vec<NewsItem> {
    items.retain(|n| n.time == 0 || now - n.time < 14 * 86_400_000);
    items.sort_by_key(|n| std::cmp::Reverse(n.time));
    let mut seen = std::collections::HashSet::new();
    items.retain(|n| seen.insert(title_key(&n.title)));
    items.truncate(50);
    items
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
        // Google News: the outlet comes from <source>, and leaves the title.
        let rss = "<rss><item><title>Infosys falls 4% - Reuters</title><link>https://g/1</link>\
                   <pubDate>Mon, 26 Oct 2026 03:50:00 GMT</pubDate><source url=\"https://r\">Reuters</source></item></rss>";
        let n = parse_rss(rss, "Google News");
        assert_eq!(
            (n[0].title.as_str(), n[0].publisher.as_str()),
            ("Infosys falls 4%", "Reuters")
        );
    }

    #[test]
    fn news_names_matching_and_merging() {
        let s = |id: &str, desc: &str, kind| Symbol {
            description: desc.into(),
            kind,
            ..Symbol::bare(id)
        };
        assert_eq!(
            news_name(&s("BINANCE:SOLUSDT", "SOL / USDT", AssetKind::Crypto)),
            "Solana"
        );
        assert_eq!(
            news_name(&s("NSE:INFY", "Infosys Limited", AssetKind::Stock)),
            "Infosys"
        );
        assert_eq!(news_name(&s("NASDAQ:AAPL", "Apple Inc.", AssetKind::Stock)), "Apple");
        assert_eq!(
            news_name(&s("NSE:NIFTY", "Nifty 50 Index", AssetKind::Index)),
            "Nifty 50"
        );
        assert_eq!(news_name(&s("NSE:TCS", "", AssetKind::Stock)), "TCS");
        let terms = vec!["solana".to_string(), "sol".to_string()];
        assert!(mentions("Mirae issues tokenized notes on Solana", &terms));
        assert!(!mentions("A solution for stablecoins", &terms));
        assert!(mentions(
            "Reliance Industries Q2 beats",
            &["reliance industries".into()]
        ));
        let item = |title: &str, time| NewsItem {
            title: title.into(),
            publisher: String::new(),
            link: String::new(),
            time,
        };
        let now = 100 * 86_400_000;
        let m = merge(
            vec![
                item("Old", 1),
                item("Infosys falls 4%", now - 5),
                item("Infosys falls 4%.", now - 2),
                item("New", now - 1),
            ],
            now,
        );
        let titles: Vec<&str> = m.iter().map(|n| n.title.as_str()).collect();
        assert_eq!(titles, ["New", "Infosys falls 4%."]);
    }
}

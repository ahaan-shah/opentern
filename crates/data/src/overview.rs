//! The market overview: every member of a market's main index with its
//! sector, market cap and performance, for the heatmap that takes over
//! the chart area (`app/src/overview.rs`).
//!
//! - **US stocks**: the S&P 500. Members and GICS sectors come from the
//!   public-domain `datasets/s-and-p-500-companies` list (ODC-PDDL-1.0,
//!   through jsDelivr, weekly; a copy ships in `assets/overview/`). Market
//!   caps from Yahoo's public large-cap screener, which needs no session.
//! - **India**: the NIFTY 500, with NSE's industries, from NSE's own
//!   constituent file (weekly; a copy ships too). Market caps from Yahoo's
//!   batch quotes (which need Yahoo's session, like fundamentals).
//! - **Crypto**: the largest coins by market cap from CoinGecko's free API
//!   (one request), kept to coins a supported exchange trades; categories
//!   from the table below, "Other" for the rest. When CoinGecko can't be
//!   reached, Binance's 24-hour volume sizes the tiles instead.
//!
//! Prices and history come from Yahoo's spark (stocks) and Binance's daily
//! bars (crypto). A stock's share count (cap ÷ price) is kept in
//! `<cache>/overview/`, so caps still follow the price when a later fetch
//! of them fails. Demo mode makes everything up from the bundled lists.

use crate::provider::{self, Error, Result};
use opentern_core::{AssetKind, Symbol};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Market {
    #[default]
    Us,
    India,
    Crypto,
}

impl Market {
    pub const ALL: [Market; 3] = [Market::Us, Market::India, Market::Crypto];

    pub fn label(self) -> &'static str {
        match self {
            Market::Us => "US stocks",
            Market::India => "India",
            Market::Crypto => "Crypto",
        }
    }

    /// What the map holds.
    pub fn universe(self) -> &'static str {
        match self {
            Market::Us => "S&P 500",
            Market::India => "NIFTY 500",
            Market::Crypto => "Top 150",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Market::Us => "us",
            Market::India => "india",
            Market::Crypto => "crypto",
        }
    }

    pub fn from_key(s: &str) -> Option<Market> {
        Market::ALL
            .into_iter()
            .find(|m| m.key().eq_ignore_ascii_case(s) || m.label().eq_ignore_ascii_case(s))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Period {
    #[default]
    Day,
    Week,
    Month,
    Quarter,
    Ytd,
    Year,
}

impl Period {
    pub const ALL: [Period; 6] = [
        Period::Day,
        Period::Week,
        Period::Month,
        Period::Quarter,
        Period::Ytd,
        Period::Year,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Period::Day => "1D",
            Period::Week => "1W",
            Period::Month => "1M",
            Period::Quarter => "3M",
            Period::Ytd => "YTD",
            Period::Year => "1Y",
        }
    }

    pub fn from_label(s: &str) -> Option<Period> {
        Period::ALL
            .into_iter()
            .find(|p| p.label().eq_ignore_ascii_case(s.trim()))
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// One tile.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub symbol: Symbol,
    pub name: String,
    pub sector: String,
    pub industry: String,
    /// In the market's currency; NaN when unknown.
    pub cap: f64,
    pub price: f64,
    /// Percent change per `Period` (by `Period::index`); NaN when unknown.
    pub change: [f64; 6],
    /// Daily closes, oldest first (the last three months or so), for the
    /// hover card's line.
    pub closes: Vec<f32>,
}

impl Member {
    pub fn change(&self, p: Period) -> f64 {
        self.change[p.index()]
    }
}

/// What tiles are sized by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sizing {
    MarketCap,
    /// Crypto without CoinGecko: 24-hour volume.
    Volume,
    /// No caps came: every tile the same size.
    Equal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Board {
    pub market: Market,
    pub members: Vec<Member>,
    pub sizing: Sizing,
    /// Where the sizes came from, for the legend's tooltip.
    pub source: String,
    pub currency: String,
    /// Unix ms.
    pub at: i64,
}

// ------------------------------------------------------------ entry

/// Builds a market's board. `dir` is the hub's cache folder.
pub fn board(market: Market, dir: &Path, demo: bool) -> Result<Board> {
    if demo {
        return Ok(made_up(market));
    }
    match market {
        Market::Us => us_board(dir),
        Market::India => india_board(dir),
        Market::Crypto => crypto_board(dir),
    }
}

// ------------------------------------------------------------ lists

const SP500: &str = include_str!("../assets/overview/sp500.csv");
const NIFTY500: &str = include_str!("../assets/overview/nifty500.csv");
const SP500_URL: &str = "https://cdn.jsdelivr.net/gh/datasets/s-and-p-500-companies@main/data/constituents.csv";
const NIFTY500_URLS: [&str; 2] = [
    "https://niftyindices.com/IndexConstituent/ind_nifty500list.csv",
    "https://archives.nseindia.com/content/indices/ind_nifty500list.csv",
];
const WEEK_MS: i64 = 7 * 86_400_000;

/// Splits CSV text into rows of fields (quotes and doubled quotes
/// handled; a header row is kept).
pub(crate) fn csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => row.push(std::mem::take(&mut field)),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                if row.iter().any(|f| !f.is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// A company from a list.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Listed {
    /// As the app writes it (`BRK-B`, `M&M`).
    pub ticker: String,
    pub name: String,
    pub sector: String,
    pub industry: String,
    /// `NASDAQ`, `NYSE`, `AMEX`; empty when the list doesn't say.
    pub exchange: String,
}

/// Rows by header name, so both the bundled copy and the live file read.
fn by_header(text: &str, cols: &[&str]) -> Vec<Vec<String>> {
    let mut rows = csv_rows(text).into_iter();
    let Some(head) = rows.next() else { return Vec::new() };
    let at: Vec<Option<usize>> = cols
        .iter()
        .map(|c| head.iter().position(|h| h.trim().eq_ignore_ascii_case(c)))
        .collect();
    rows.map(|r| {
        at.iter()
            .map(|i| {
                i.and_then(|i| r.get(i))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default()
            })
            .collect()
    })
    .collect()
}

pub(crate) fn parse_sp500(text: &str) -> Vec<Listed> {
    by_header(
        text,
        &["Symbol", "Security", "GICS Sector", "GICS Sub-Industry", "Exchange"],
    )
    .into_iter()
    .filter(|r| !r[0].is_empty() && !r[2].is_empty())
    .map(|r| Listed {
        ticker: r[0].replace('.', "-"),
        name: r[1].clone(),
        sector: us_sector(&r[2]).to_string(),
        industry: r[3].clone(),
        exchange: r[4].clone(),
    })
    .collect()
}

pub(crate) fn parse_nifty500(text: &str) -> Vec<Listed> {
    by_header(text, &["Symbol", "Company Name", "Industry", "Series"])
        .into_iter()
        .filter(|r| !r[0].is_empty() && !r[2].is_empty() && (r[3].is_empty() || r[3] == "EQ"))
        .map(|r| Listed {
            ticker: r[0].clone(),
            name: company_name(&r[1]),
            sector: india_sector(&r[2]).to_string(),
            industry: r[2].clone(),
            exchange: "NSE".into(),
        })
        .collect()
}

/// "Reliance Industries Ltd." → "Reliance Industries".
fn company_name(s: &str) -> String {
    let s = s.trim();
    for end in [" Ltd.", " Ltd", " Limited"] {
        if let Some(b) = s.strip_suffix(end) {
            return b.trim().to_string();
        }
    }
    s.to_string()
}

/// GICS sectors, shorter.
fn us_sector(s: &str) -> &str {
    match s {
        "Information Technology" => "Technology",
        "Communication Services" => "Communication",
        "Consumer Discretionary" => "Consumer cyclical",
        "Consumer Staples" => "Consumer staples",
        "Financials" => "Finance",
        "Health Care" => "Health care",
        "Real Estate" => "Real estate",
        s => s,
    }
}

/// NSE's industries, shorter.
fn india_sector(s: &str) -> &str {
    match s {
        "Financial Services" => "Finance",
        "Information Technology" => "Technology",
        "Automobile and Auto Components" => "Automobiles",
        "Fast Moving Consumer Goods" => "FMCG",
        "Oil Gas & Consumable Fuels" => "Oil and gas",
        "Metals & Mining" => "Metals and mining",
        "Media Entertainment & Publication" => "Media",
        "Construction Materials" => "Construction materials",
        "Consumer Services" => "Consumer services",
        "Consumer Durables" => "Consumer durables",
        "Capital Goods" => "Capital goods",
        s => s,
    }
}

fn get_text(agent: &ureq::Agent, url: &str) -> Result<String> {
    let mut resp = agent.get(url).call().map_err(|e| match e {
        ureq::Error::StatusCode(c) => Error::Bad(format!("The data service answered with an error ({c}).")),
        _ => Error::Offline,
    })?;
    resp.body_mut()
        .with_config()
        .limit(8 * 1024 * 1024)
        .read_to_string()
        .map_err(|_| Error::Offline)
}

fn store_dir(dir: &Path) -> std::path::PathBuf {
    dir.join("overview")
}

fn age_ms(path: &Path) -> Option<i64> {
    let m = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(std::time::SystemTime::now().duration_since(m).ok()?.as_millis() as i64)
}

/// A list: the copy fetched within the week, else a fresh one (kept when
/// it parses to at least `min` rows), else the last fetched or bundled.
fn list(
    dir: &Path,
    file: &str,
    urls: &[&str],
    bundled: &str,
    min: usize,
    parse: fn(&str) -> Vec<Listed>,
) -> Vec<Listed> {
    let path = store_dir(dir).join(file);
    let kept = std::fs::read_to_string(&path)
        .ok()
        .map(|t| parse(&t))
        .filter(|l| l.len() >= min);
    if let Some(l) = &kept
        && age_ms(&path).is_some_and(|a| a < WEEK_MS)
    {
        return l.clone();
    }
    let agent = provider::agent();
    for url in urls {
        if let Ok(text) = get_text(&agent, url) {
            let l = parse(&text);
            if l.len() >= min {
                let _ = std::fs::create_dir_all(store_dir(dir));
                let _ = std::fs::write(&path, &text);
                return l;
            }
        }
    }
    kept.unwrap_or_else(|| parse(bundled))
}

pub(crate) fn us_list(dir: &Path) -> Vec<Listed> {
    let mut l = list(dir, "sp500.csv", &[SP500_URL], SP500, 400, parse_sp500);
    // The live list doesn't say where each trades: the bundled one does.
    let known: HashMap<String, String> = parse_sp500(SP500).into_iter().map(|x| (x.ticker, x.exchange)).collect();
    for x in &mut l {
        if x.exchange.is_empty() {
            x.exchange = known.get(&x.ticker).cloned().unwrap_or_default();
        }
    }
    l
}

pub(crate) fn india_list(dir: &Path) -> Vec<Listed> {
    list(dir, "nifty500.csv", &NIFTY500_URLS, NIFTY500, 400, parse_nifty500)
}

fn us_symbol(x: &Listed, exchange: &str) -> Symbol {
    let ex = if exchange.is_empty() { "NYSE" } else { exchange };
    Symbol {
        id: format!("{ex}:{}", x.ticker),
        description: x.name.clone(),
        kind: AssetKind::Stock,
        currency: "USD".into(),
        tick_size: 0.01,
        min_qty: 0.0,
        timezone: "America/New_York".into(),
    }
}

fn india_symbol(x: &Listed) -> Symbol {
    Symbol {
        id: format!("NSE:{}", x.ticker),
        description: x.name.clone(),
        kind: AssetKind::Stock,
        currency: "INR".into(),
        tick_size: 0.05,
        min_qty: 0.0,
        timezone: "Asia/Kolkata".into(),
    }
}

// ------------------------------------------------------------ performance

/// Percent changes per period from daily closes (`times` in seconds,
/// oldest first) up to `last`; `day` is the 1D change when the source
/// gives one (else from the closes).
pub(crate) fn changes(times: &[i64], closes: &[f64], last: f64, day: Option<f64>, now_s: i64) -> [f64; 6] {
    let mut out = [f64::NAN; 6];
    if !last.is_finite() || times.len() != closes.len() {
        return out;
    }
    let pct = |base: f64| {
        if base.is_finite() && base > 0.0 {
            (last / base - 1.0) * 100.0
        } else {
            f64::NAN
        }
    };
    // The last close at or before a time.
    let at = |cut: i64| -> f64 {
        let k = times.partition_point(|t| *t <= cut);
        (0..k)
            .rev()
            .map(|i| closes[i])
            .find(|c| c.is_finite())
            .unwrap_or(f64::NAN)
    };
    let zone = jiff::Timestamp::from_second(now_s)
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
        .to_zoned(jiff::tz::TimeZone::UTC);
    let back = |span: jiff::Span| zone.checked_sub(span).map(|z| z.timestamp().as_second()).unwrap_or(0);
    out[Period::Day.index()] = match day {
        Some(d) => d,
        // The last close of an earlier day than the latest bar's: the
        // previous session's close, as the watchlist and charts use.
        None => {
            let latest = (0..closes.len()).rev().find(|&i| closes[i].is_finite());
            latest.map_or(f64::NAN, |i| {
                let today = times[i].div_euclid(86_400);
                (0..i)
                    .rev()
                    .find(|&j| closes[j].is_finite() && times[j].div_euclid(86_400) < today)
                    .map_or(f64::NAN, |j| pct(closes[j]))
            })
        }
    };
    out[Period::Week.index()] = pct(at(back(jiff::Span::new().days(7))));
    out[Period::Month.index()] = pct(at(back(jiff::Span::new().months(1))));
    out[Period::Quarter.index()] = pct(at(back(jiff::Span::new().months(3))));
    out[Period::Year.index()] = pct(at(back(jiff::Span::new().years(1))));
    let jan1 = jiff::civil::date(zone.year(), 1, 1)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .map(|z| z.timestamp().as_second())
        .unwrap_or(0);
    out[Period::Ytd.index()] = pct(at(jan1 - 1));
    // History too short to reach back that far.
    if let Some(first) = times.first() {
        for (p, cut) in [
            (Period::Week, back(jiff::Span::new().days(7))),
            (Period::Month, back(jiff::Span::new().months(1))),
            (Period::Quarter, back(jiff::Span::new().months(3))),
            (Period::Year, back(jiff::Span::new().years(1))),
            (Period::Ytd, jan1 - 1),
        ] {
            if *first > cut {
                out[p.index()] = f64::NAN;
            }
        }
    }
    out
}

/// The last ~3 months of closes for the hover card.
fn tail(closes: &[f64]) -> Vec<f32> {
    let v: Vec<f32> = closes.iter().filter(|c| c.is_finite()).map(|c| *c as f32).collect();
    v[v.len().saturating_sub(66)..].to_vec()
}

/// One stock's spark: price, closes and times. Yahoo's
/// `fulldayChangePercent` isn't used: it isn't measured from the previous
/// close (NVDA +3.91% when the move was 181.60 → 186.64, +2.78%).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Spark {
    pub price: f64,
    pub times: Vec<i64>,
    pub closes: Vec<f64>,
}

/// Parses Yahoo's spark reply, keyed by Yahoo ticker.
pub(crate) fn parse_spark(v: &serde_json::Value) -> HashMap<String, Spark> {
    v.as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, q)| {
            let times: Vec<i64> = q["timestamp"]
                .as_array()?
                .iter()
                .map(|t| t.as_i64().unwrap_or(0))
                .collect();
            let closes: Vec<f64> = q["close"]
                .as_array()?
                .iter()
                .map(|c| c.as_f64().unwrap_or(f64::NAN))
                .collect();
            let price = q["fulldayPrice"]
                .as_f64()
                .or_else(|| closes.iter().rev().copied().find(|c| c.is_finite()))?;
            Some((k.clone(), Spark { price, times, closes }))
        })
        .collect()
}

/// Spark for many Yahoo tickers: twenty a request, a few at once.
fn sparks(tickers: &[String]) -> HashMap<String, Spark> {
    let chunks: Vec<Vec<String>> = tickers.chunks(20).map(|c| c.to_vec()).collect();
    let out = Mutex::new(HashMap::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                let agent = provider::agent();
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(chunk) = chunks.get(i) else { break };
                    let list: Vec<String> = chunk.iter().map(|t| crate::yahoo::urlencode(t)).collect();
                    let url = format!(
                        "https://query1.finance.yahoo.com/v8/finance/spark?symbols={}&range=2y&interval=1d",
                        list.join(",")
                    );
                    if let Ok(v) = provider::get_json(&agent, &url)
                        && let Ok(mut o) = out.lock()
                    {
                        o.extend(parse_spark(&v));
                    }
                }
            });
        }
    });
    out.into_inner().unwrap_or_default()
}

// ------------------------------------------------------------ caps

/// Share counts kept from the last good fetch (`ticker → shares`).
fn kept_shares(dir: &Path, market: Market) -> (HashMap<String, f64>, Option<i64>) {
    let path = store_dir(dir).join(format!("shares-{}.json", market.key()));
    let map = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<HashMap<String, f64>>(&t).ok())
        .unwrap_or_default();
    (map, age_ms(&path))
}

fn keep_shares(dir: &Path, market: Market, map: &HashMap<String, f64>) {
    let _ = std::fs::create_dir_all(store_dir(dir));
    if let Ok(t) = serde_json::to_string(map) {
        let _ = std::fs::write(store_dir(dir).join(format!("shares-{}.json", market.key())), t);
    }
}

/// From Yahoo's screener or quote replies: ticker → (cap, price, exchange).
pub(crate) fn parse_quotes(v: &serde_json::Value) -> Vec<(String, f64, f64, String)> {
    let list = v["finance"]["result"][0]["quotes"]
        .as_array()
        .or_else(|| v["quoteResponse"]["result"].as_array());
    list.into_iter()
        .flatten()
        .filter_map(|q| {
            let t = q["symbol"].as_str()?.to_string();
            let cap = q["marketCap"].as_f64()?;
            let price = q["regularMarketPrice"].as_f64().unwrap_or(f64::NAN);
            let ex = match q["exchange"].as_str().unwrap_or("") {
                "NMS" | "NGM" | "NCM" | "NAS" => "NASDAQ",
                "NYQ" | "NYS" => "NYSE",
                "PCX" | "ASE" | "BTS" => "AMEX",
                _ => "",
            };
            Some((t, cap, price, ex.to_string()))
        })
        .collect()
}

/// US caps from Yahoo's largest-companies screener (no session needed),
/// page by page until every wanted ticker is found.
fn screener_caps(wanted: &[String]) -> Vec<(String, f64, f64, String)> {
    let agent = provider::agent();
    let mut left: std::collections::HashSet<&str> = wanted.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    for page in 0..12 {
        let url = format!(
            "https://query1.finance.yahoo.com/v1/finance/screener/predefined/saved?scrIds=largest_market_cap&count=250&start={}",
            page * 250
        );
        let Ok(v) = provider::get_json(&agent, &url) else { break };
        let rows = parse_quotes(&v);
        let empty = v["finance"]["result"][0]["quotes"]
            .as_array()
            .is_none_or(|a| a.is_empty());
        for r in rows {
            if left.remove(r.0.as_str()) {
                out.push(r);
            }
        }
        if left.is_empty() || empty {
            break;
        }
    }
    out
}

/// Caps from Yahoo's batch quotes (needs the session fundamentals use).
fn quote_caps(tickers: &[String]) -> Result<Vec<(String, f64, f64, String)>> {
    use crate::fundamentals::{new_session, session, text};
    let agent = provider::agent();
    let mut out = Vec::new();
    for chunk in tickers.chunks(50) {
        let list: Vec<String> = chunk.iter().map(|t| crate::yahoo::urlencode(t)).collect();
        let mut done = false;
        for attempt in 0..2 {
            let held = session().lock().ok().and_then(|s| s.clone());
            let (cookie, crumb) = match held {
                Some(s) if attempt == 0 => s,
                _ => {
                    let s = new_session(&agent)?;
                    if let Ok(mut g) = session().lock() {
                        *g = Some(s.clone());
                    }
                    s
                }
            };
            let url = format!(
                "https://query2.finance.yahoo.com/v7/finance/quote?symbols={}&fields=marketCap,regularMarketPrice,exchange&crumb={}",
                list.join(","),
                crate::yahoo::urlencode(&crumb)
            );
            let (status, body, _) = text(&agent, &url, Some(&cookie))?;
            match status {
                401 | 403 if attempt == 0 => continue,
                429 => return Err(Error::Bad("Yahoo is limiting requests; try again in a minute.".into())),
                _ => {}
            }
            let v: serde_json::Value = serde_json::from_str(&body)
                .map_err(|_| Error::Bad(format!("Yahoo answered with an error ({status}).")))?;
            out.extend(parse_quotes(&v));
            done = true;
            break;
        }
        if !done {
            return Err(Error::Bad("Yahoo didn't start a session.".into()));
        }
    }
    Ok(out)
}

/// Fresh share counts when the kept ones are over a day old (or missing
/// many), merged over the kept ones; plus exchanges seen.
fn shares(
    dir: &Path,
    market: Market,
    tickers: &[String],
    fetch: impl Fn(&[String]) -> Vec<(String, f64, f64, String)>,
) -> (HashMap<String, f64>, HashMap<String, String>) {
    let (mut kept, age) = kept_shares(dir, market);
    let missing = tickers.iter().filter(|t| !kept.contains_key(*t)).count();
    let mut exchanges = HashMap::new();
    if age.is_none_or(|a| a > 86_400_000) || missing > tickers.len() / 10 {
        let got = fetch(tickers);
        for (t, cap, price, ex) in got {
            if cap > 0.0 && price > 0.0 {
                kept.insert(t.clone(), cap / price);
            }
            if !ex.is_empty() {
                exchanges.insert(t, ex);
            }
        }
        if !kept.is_empty() {
            keep_shares(dir, market, &kept);
        }
    }
    (kept, exchanges)
}

// ------------------------------------------------------------ stocks

fn stock_board(market: Market, listed: Vec<Listed>, dir: &Path) -> Result<Board> {
    let yahoo: Vec<String> = listed
        .iter()
        .map(|x| match market {
            Market::India => format!("{}.NS", x.ticker),
            _ => x.ticker.clone(),
        })
        .collect();
    let spark = sparks(&yahoo);
    if spark.is_empty() {
        return Err(Error::Bad(
            "Yahoo didn't send prices this time. Try again in a minute.".into(),
        ));
    }
    let (shares, exchanges) = match market {
        Market::India => shares(dir, market, &yahoo, |t| quote_caps(t).unwrap_or_default()),
        _ => shares(dir, market, &yahoo, |t| {
            let mut got = screener_caps(t);
            // The few the screener's pages don't reach.
            let seen: std::collections::HashSet<&str> = got.iter().map(|g| g.0.as_str()).collect();
            let rest: Vec<String> = t.iter().filter(|x| !seen.contains(x.as_str())).cloned().collect();
            if !rest.is_empty() {
                got.extend(quote_caps(&rest).unwrap_or_default());
            }
            got
        }),
    };
    let now_s = provider::now_ms() / 1000;
    let mut members: Vec<Member> = listed
        .iter()
        .zip(&yahoo)
        .filter_map(|(x, y)| {
            let s = spark.get(y)?;
            let symbol = match market {
                Market::India => india_symbol(x),
                _ => us_symbol(x, exchanges.get(y).map(String::as_str).unwrap_or(&x.exchange)),
            };
            Some(Member {
                symbol,
                name: x.name.clone(),
                sector: x.sector.clone(),
                industry: x.industry.clone(),
                cap: shares.get(y).map_or(f64::NAN, |n| n * s.price),
                price: s.price,
                change: changes(&s.times, &s.closes, s.price, None, now_s),
                closes: tail(&s.closes),
            })
        })
        .collect();
    let known = members.iter().filter(|m| m.cap.is_finite()).count();
    let sizing = if known * 2 < members.len() {
        Sizing::Equal
    } else {
        // The few without a cap are among the smallest: give them the
        // smallest known one rather than leave them out.
        let least = members
            .iter()
            .map(|m| m.cap)
            .filter(|c| c.is_finite())
            .fold(f64::INFINITY, f64::min);
        for m in &mut members {
            if !m.cap.is_finite() {
                m.cap = least;
            }
        }
        Sizing::MarketCap
    };
    Ok(Board {
        market,
        members,
        sizing,
        source: "Yahoo".into(),
        currency: if market == Market::India { "INR" } else { "USD" }.into(),
        at: provider::now_ms(),
    })
}

fn us_board(dir: &Path) -> Result<Board> {
    stock_board(Market::Us, us_list(dir), dir)
}

fn india_board(dir: &Path) -> Result<Board> {
    stock_board(Market::India, india_list(dir), dir)
}

// ------------------------------------------------------------ crypto

/// Coins grouped the way traders talk about them; others are "Other".
const CATEGORIES: [(&str, &[&str]); 9] = [
    (
        "Bitcoin and payments",
        &[
            "BTC", "XRP", "XLM", "LTC", "BCH", "BSV", "XMR", "ZEC", "DASH", "XEC", "DCR", "KAS", "ETC", "DGB", "RVN",
            "ZEN",
        ],
    ),
    (
        "Layer 1",
        &[
            "ETH", "SOL", "TRX", "ADA", "AVAX", "SUI", "TON", "NEAR", "DOT", "HBAR", "ICP", "APT", "ALGO", "ATOM",
            "VET", "SEI", "XTZ", "KAIA", "EGLD", "FLR", "XDC", "S", "FTM", "MON", "EOS", "NEO", "IOTA", "QTUM", "ONE",
            "MINA", "KSM", "CFX", "BERA", "INJ", "HASH", "XPL", "LUNC", "LUNA", "WAVES", "ZIL", "ICX", "ROSE", "CELO",
            "KDA", "A",
        ],
    ),
    (
        "Layer 2",
        &[
            "ARB", "OP", "POL", "MATIC", "MNT", "STRK", "STX", "ZK", "IMX", "METIS", "SKL", "MANTA", "BLAST", "LRC",
            "CELR", "ZRO", "TAIKO", "SCR",
        ],
    ),
    (
        "Exchange tokens",
        &[
            "BNB", "OKB", "LEO", "CRO", "WBT", "BGB", "GT", "KCS", "HTX", "MX", "NEXO", "TWT", "BTSE", "BORG",
        ],
    ),
    (
        "DeFi",
        &[
            "HYPE", "UNI", "AAVE", "ENA", "ONDO", "SKY", "MKR", "MORPHO", "JUP", "CAKE", "RAY", "CRV", "PENDLE", "LDO",
            "COMP", "RUNE", "AERO", "JTO", "CVX", "SYRUP", "ETHFI", "ASTER", "DYDX", "1INCH", "SNX", "GMX", "SUSHI",
            "KMNO", "FLUID", "MET", "LIT", "WLFI", "JST", "SUN", "FF", "OHM", "BAL", "YFI", "CAKE", "EIGEN", "ORCA",
            "DRIFT", "COW", "RPL", "QNT",
        ],
    ),
    (
        "Infrastructure",
        &[
            "LINK", "FIL", "AR", "GRT", "PYTH", "TIA", "THETA", "ENS", "TRAC", "W", "AXL", "BAT", "BTT", "STORJ",
            "API3", "BAND", "ANKR", "JASMY", "HNT", "IOTX", "POWR", "GEOD",
        ],
    ),
    (
        "AI",
        &[
            "TAO", "FET", "RENDER", "RNDR", "VIRTUAL", "WLD", "GRASS", "AIOZ", "AKT", "AI16Z", "AGIX", "OCEAN", "ARKM",
            "IO", "KAITO", "VVV", "NIGHT",
        ],
    ),
    (
        "Memes",
        &[
            "DOGE", "SHIB", "PEPE", "BONK", "FLOKI", "WIF", "TRUMP", "PENGU", "SPX", "FARTCOIN", "PUMP", "BOME", "MOG",
            "NEIRO", "POPCAT", "TURBO", "MEW", "BRETT", "PNUT", "1000SATS", "MEME", "USELESS", "M", "DOGS", "NOT",
        ],
    ),
    (
        "Gaming and metaverse",
        &[
            "AXS", "SAND", "MANA", "GALA", "BEAM", "CHZ", "APE", "ENJ", "ILV", "YGG", "PIXEL", "RON", "FLOW",
        ],
    ),
];

pub fn category(base: &str) -> &'static str {
    let b = base.to_ascii_uppercase();
    CATEGORIES
        .iter()
        .find(|(_, list)| list.contains(&b.as_str()))
        .map_or("Other", |(name, _)| name)
}

/// Stablecoins, wrapped and staked copies, tokenized funds and stocks:
/// not what a crypto map is for.
pub(crate) fn excluded(symbol: &str, name: &str, price: f64) -> bool {
    let s = symbol.to_ascii_uppercase();
    let n = name.to_ascii_lowercase();
    const STABLE: [&str; 30] = [
        "USDT", "USDC", "USDE", "DAI", "FDUSD", "USDS", "PYUSD", "TUSD", "USDD", "USD1", "RLUSD", "FRAX", "USDP",
        "GUSD", "LUSD", "SUSD", "EURC", "EURT", "USDG", "USDY", "USDF", "BFUSD", "USD0", "GHO", "CRVUSD", "USDX",
        "AUSD", "USDAI", "BUSD", "USX",
    ];
    STABLE.contains(&s.as_str())
        || ["XAUT", "PAXG", "KAU", "KAG", "BUIDL", "USYC", "USTB", "OUSG", "JAAA"].contains(&s.as_str())
        || s.contains("USD")
        || [
            "wrapped",
            "staked",
            "bridged",
            "restaked",
            "liquid staking",
            "tokenized",
            "xstock",
            "bstocks",
            "t-bill",
            "treasur",
            " fund",
            "money market",
        ]
        .iter()
        .any(|w| n.contains(w))
        || (s.starts_with('W') && ["WBTC", "WETH", "WBETH", "WSTETH", "WEETH", "WBNB", "WSOL"].contains(&s.as_str()))
        || [
            "STETH", "RETH", "CBBTC", "CBETH", "JITOSOL", "MSOL", "LBTC", "SOLVBTC", "BNSOL", "METH", "EZETH", "RSETH",
            "TBTC", "CLBTC", "SUSDE", "WEETH",
        ]
        .contains(&s.as_str())
        || (price > 0.97 && price < 1.03 && n.contains("dollar"))
}

/// The pair to chart a coin on: Binance's USDT pair first, then the other
/// exchanges'; from the symbol lists search keeps.
pub(crate) fn crypto_pairs(known: &[Symbol]) -> HashMap<String, Symbol> {
    let rank = |s: &Symbol| {
        let ex = match s.exchange() {
            "BINANCE" => 0,
            "COINBASE" => 1,
            "BYBIT" => 2,
            "OKX" => 3,
            "KRAKEN" => 4,
            _ => 9,
        };
        let q = match s.currency.as_str() {
            "USDT" => 0,
            "USD" => 1,
            "USDC" => 2,
            _ => 9,
        };
        (q.min(9), ex)
    };
    let mut out: HashMap<String, Symbol> = HashMap::new();
    for s in known.iter().filter(|s| s.kind == AssetKind::Crypto) {
        let Some((base, quote)) = s.description.split_once(" / ") else {
            continue;
        };
        if !matches!(quote.trim(), "USDT" | "USD" | "USDC") {
            continue;
        }
        let base = match base.trim() {
            "XBT" => "BTC",
            "XDG" => "DOGE",
            b => b,
        }
        .to_ascii_uppercase();
        let better = out.get(&base).is_none_or(|o| rank(s) < rank(o));
        if better {
            out.insert(base, s.clone());
        }
    }
    out
}

fn binance_pair(base: &str) -> Symbol {
    Symbol {
        id: format!("BINANCE:{base}USDT"),
        description: format!("{base} / USDT"),
        kind: AssetKind::Crypto,
        currency: "USDT".into(),
        tick_size: 0.0001,
        min_qty: 0.0,
        timezone: "UTC".into(),
    }
}

/// One coin from CoinGecko's markets list.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Coin {
    pub symbol: String,
    pub name: String,
    pub cap: f64,
    pub price: f64,
    pub day: f64,
    pub week: f64,
    pub month: f64,
    pub year: f64,
}

pub(crate) fn parse_coingecko(v: &serde_json::Value) -> Vec<Coin> {
    let f = |q: &serde_json::Value, k: &str| q[k].as_f64().unwrap_or(f64::NAN);
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|q| {
            Some(Coin {
                symbol: q["symbol"].as_str()?.to_ascii_uppercase(),
                name: q["name"].as_str()?.to_string(),
                cap: q["market_cap"].as_f64().filter(|c| *c > 0.0)?,
                price: f(q, "current_price"),
                day: f(q, "price_change_percentage_24h_in_currency"),
                week: f(q, "price_change_percentage_7d_in_currency"),
                month: f(q, "price_change_percentage_30d_in_currency"),
                year: f(q, "price_change_percentage_1y_in_currency"),
            })
        })
        .collect()
}

/// Daily Binance closes per pair, kept an hour (the map refreshes far
/// more often than a daily close changes).
fn binance_closes(pairs: &[String]) -> HashMap<String, (Vec<i64>, Vec<f64>)> {
    type Kept = HashMap<String, (i64, Vec<i64>, Vec<f64>)>;
    static KEPT: Mutex<Option<Kept>> = Mutex::new(None);
    let now = provider::now_ms();
    let fresh = |k: &Kept, p: &str| k.get(p).is_some_and(|(at, _, _)| now - at < 3_600_000);
    let want: Vec<String> = {
        let g = KEPT.lock().ok();
        let k = g.as_ref().and_then(|g| g.as_ref());
        pairs
            .iter()
            .filter(|p| k.is_none_or(|k| !fresh(k, p)))
            .cloned()
            .collect()
    };
    let got = Mutex::new(Vec::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..6 {
            s.spawn(|| {
                let agent = provider::agent();
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(p) = want.get(i) else { break };
                    let path = format!("/api/v3/klines?symbol={p}&interval=1d&limit=400");
                    if let Ok(v) = crate::binance::get(&agent, &path) {
                        let rows = v.as_array().cloned().unwrap_or_default();
                        let times = rows.iter().map(|r| r[0].as_i64().unwrap_or(0) / 1000).collect();
                        let closes = rows.iter().map(|r| provider::num(&r[4]).unwrap_or(f64::NAN)).collect();
                        if let Ok(mut g) = got.lock() {
                            g.push((p.clone(), times, closes));
                        }
                    }
                }
            });
        }
    });
    let mut g = KEPT.lock().ok();
    let k = g.as_mut().map(|g| g.get_or_insert_with(HashMap::new));
    let mut out = HashMap::new();
    if let Some(k) = k {
        for (p, t, c) in got.into_inner().unwrap_or_default() {
            k.insert(p, (now, t, c));
        }
        for p in pairs {
            if let Some((_, t, c)) = k.get(p) {
                out.insert(p.clone(), (t.clone(), c.clone()));
            }
        }
    }
    out
}

/// Binance's current UTC day per pair: (open, last), as the watchlist's
/// daily change uses. A hundred pairs a request.
fn binance_days(pairs: &[String]) -> HashMap<String, (f64, f64)> {
    let agent = provider::agent();
    let mut out = HashMap::new();
    for chunk in pairs.chunks(100) {
        let list: Vec<String> = chunk.iter().map(|p| format!("%22{p}%22")).collect();
        let path = format!("/api/v3/ticker/tradingDay?type=MINI&symbols=%5B{}%5D", list.join(","));
        if let Ok(v) = crate::binance::get(&agent, &path) {
            out.extend(parse_days(&v));
        }
    }
    out
}

pub(crate) fn parse_days(v: &serde_json::Value) -> HashMap<String, (f64, f64)> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let open = provider::num(&r["openPrice"]).filter(|o| *o > 0.0)?;
            let last = provider::num(&r["lastPrice"]).filter(|l| *l > 0.0)?;
            Some((r["symbol"].as_str()?.to_string(), (open, last)))
        })
        .collect()
}

/// How many coins the map shows.
const COINS: usize = 150;

fn crypto_board(dir: &Path) -> Result<Board> {
    let known = crate::cache::Cache::open(dir)
        .ok()
        .and_then(|c| c.symbols().ok())
        .unwrap_or_default();
    let pairs = crypto_pairs(&known);
    let pair_for = |base: &str| -> Option<Symbol> {
        pairs
            .get(base)
            .cloned()
            // Before the exchanges' lists are in: the coins named above.
            .or_else(|| (pairs.is_empty() && category(base) != "Other").then(|| binance_pair(base)))
    };
    let agent = provider::agent();
    let url = "https://api.coingecko.com/api/v3/coins/markets?vs_currency=usd&order=market_cap_desc&per_page=250&page=1&price_change_percentage=24h,7d,30d,1y";
    let gecko = provider::get_json(&agent, url).map(|v| parse_coingecko(&v));
    let now_s = provider::now_ms() / 1000;
    let (mut members, sizing, source) = match gecko {
        Ok(coins) if !coins.is_empty() => {
            let mut seen = std::collections::HashSet::new();
            let members: Vec<Member> = coins
                .into_iter()
                .filter(|c| !excluded(&c.symbol, &c.name, c.price))
                .filter(|c| seen.insert(c.symbol.clone()))
                .filter_map(|c| {
                    let symbol = pair_for(&c.symbol)?;
                    let mut change = [f64::NAN; 6];
                    change[Period::Day.index()] = c.day;
                    change[Period::Week.index()] = c.week;
                    change[Period::Month.index()] = c.month;
                    change[Period::Year.index()] = c.year;
                    Some(Member {
                        symbol,
                        sector: category(&c.symbol).to_string(),
                        industry: String::new(),
                        name: c.name,
                        cap: c.cap,
                        price: c.price,
                        change,
                        closes: Vec::new(),
                    })
                })
                .take(COINS)
                .collect();
            (members, Sizing::MarketCap, "CoinGecko".to_string())
        }
        _ => {
            // Binance's 24-hour tickers: sized by volume.
            let v = crate::binance::get(&agent, "/api/v3/ticker/24hr")?;
            let mut rows: Vec<(String, f64, f64, f64)> = v
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| {
                    let t = r["symbol"].as_str()?;
                    let base = t.strip_suffix("USDT")?;
                    let vol = provider::num(&r["quoteVolume"])?;
                    let price = provider::num(&r["lastPrice"])?;
                    let ch = provider::num(&r["priceChangePercent"])?;
                    (!base.is_empty()
                        && price > 0.0
                        && !excluded(base, "", price)
                        && !base.ends_with("UP")
                        && !base.ends_with("DOWN"))
                    .then(|| (base.to_string(), vol, price, ch))
                })
                .collect();
            rows.sort_by(|a, b| b.1.total_cmp(&a.1));
            let members = rows
                .into_iter()
                .take(COINS)
                .map(|(base, vol, price, ch)| {
                    let mut change = [f64::NAN; 6];
                    change[Period::Day.index()] = ch;
                    Member {
                        symbol: pairs.get(&base).cloned().unwrap_or_else(|| binance_pair(&base)),
                        sector: category(&base).to_string(),
                        industry: String::new(),
                        name: base,
                        cap: vol,
                        price,
                        change,
                        closes: Vec::new(),
                    }
                })
                .collect();
            (members, Sizing::Volume, "Binance 24h volume".to_string())
        }
    };
    // Binance wherever a coin trades there, so every period is measured
    // like the charts and watchlist: 1D from today's UTC open, the rest
    // from daily closes, all to the live price. CoinGecko's rolling
    // figures only fill in where Binance has too little history.
    let binance: Vec<String> = members
        .iter()
        .filter_map(|m| m.symbol.id.strip_prefix("BINANCE:").map(str::to_string))
        .collect();
    let (closes, days) = std::thread::scope(|s| {
        let days = s.spawn(|| binance_days(&binance));
        (binance_closes(&binance), days.join().unwrap_or_default())
    });
    for m in &mut members {
        let Some(pair) = m.symbol.id.strip_prefix("BINANCE:") else {
            continue;
        };
        let day = days.get(pair).copied();
        if let Some((_, last)) = day {
            m.price = last;
        }
        if let Some((t, c)) = closes.get(pair) {
            for (i, v) in changes(t, c, m.price, None, now_s).iter().enumerate() {
                if v.is_finite() {
                    m.change[i] = *v;
                }
            }
            m.closes = tail(c);
        }
        if let Some((open, last)) = day {
            m.change[Period::Day.index()] = (last / open - 1.0) * 100.0;
        }
    }
    if members.is_empty() {
        return Err(Error::Bad(
            "No coin prices came this time. Try again in a minute.".into(),
        ));
    }
    Ok(Board {
        market: Market::Crypto,
        members,
        sizing,
        source,
        currency: "USD".into(),
        at: provider::now_ms(),
    })
}

// ------------------------------------------------------------ demo

/// Rough sizes for the names a demo map should lead with (made up, in
/// billions; only the order matters).
const DEMO_CAPS: [(&str, f64); 48] = [
    ("NVDA", 4500.0),
    ("AAPL", 3900.0),
    ("MSFT", 3800.0),
    ("GOOGL", 2900.0),
    ("AMZN", 2400.0),
    ("META", 1800.0),
    ("AVGO", 1600.0),
    ("TSLA", 1300.0),
    ("BRK-B", 1050.0),
    ("JPM", 800.0),
    ("LLY", 850.0),
    ("WMT", 820.0),
    ("V", 680.0),
    ("ORCL", 700.0),
    ("MA", 530.0),
    ("XOM", 480.0),
    ("NFLX", 500.0),
    ("COST", 420.0),
    ("JNJ", 420.0),
    ("RELIANCE", 230.0),
    ("HDFCBANK", 180.0),
    ("TCS", 140.0),
    ("BHARTIARTL", 130.0),
    ("ICICIBANK", 120.0),
    ("SBIN", 90.0),
    ("INFY", 80.0),
    ("BAJFINANCE", 70.0),
    ("HINDUNILVR", 65.0),
    ("ITC", 60.0),
    ("LT", 58.0),
    ("MARUTI", 50.0),
    ("M&M", 45.0),
    ("SUNPHARMA", 48.0),
    ("KOTAKBANK", 50.0),
    ("HCLTECH", 52.0),
    ("BTC", 1500.0),
    ("ETH", 270.0),
    ("BNB", 90.0),
    ("XRP", 74.0),
    ("SOL", 53.0),
    ("TRX", 30.0),
    ("DOGE", 11.0),
    ("ADA", 7.0),
    ("LINK", 8.0),
    ("HYPE", 18.0),
    ("AVAX", 3.6),
    ("SUI", 3.5),
    ("XLM", 5.3),
];

fn hash(s: &str) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

/// A steady number in [0, 1) from a key.
fn unit(s: &str, salt: u64) -> f64 {
    let h = hash(s) ^ salt.wrapping_mul(0x9E3779B97F4A7C15);
    let h = h ^ (h >> 31);
    (h.wrapping_mul(0xBF58476D1CE4E5B9) >> 11) as f64 / (1u64 << 53) as f64
}

fn made_up(market: Market) -> Board {
    let demo_cap = |t: &str| DEMO_CAPS.iter().find(|(k, _)| *k == t).map(|(_, c)| c * 1e9);
    let listed: Vec<(Symbol, Listed)> = match market {
        Market::Us => parse_sp500(SP500)
            .into_iter()
            .map(|x| (us_symbol(&x, &x.exchange.clone()), x))
            .collect(),
        Market::India => parse_nifty500(NIFTY500)
            .into_iter()
            .map(|x| (india_symbol(&x), x))
            .collect(),
        Market::Crypto => CATEGORIES
            .iter()
            .flat_map(|(cat, list)| list.iter().map(move |b| (*cat, *b)))
            .take(COINS)
            .map(|(cat, b)| {
                (
                    binance_pair(b),
                    Listed {
                        ticker: b.to_string(),
                        name: b.to_string(),
                        sector: cat.to_string(),
                        industry: String::new(),
                        exchange: "BINANCE".into(),
                    },
                )
            })
            .collect(),
    };
    let scale = match market {
        Market::Us => 30e9,
        Market::India => 3e9,
        Market::Crypto => 0.6e9,
    };
    // A mood per sector, so sectors read as groups.
    let now_s = provider::now_ms() / 1000;
    let members = listed
        .into_iter()
        .map(|(symbol, x)| {
            let u = unit(&x.ticker, 1).max(1e-9);
            let cap = demo_cap(&x.ticker).unwrap_or(scale * (0.12 + (-u.ln()).powf(1.6) * 0.5));
            let mood = (unit(&x.sector, 2) - 0.5) * 2.4;
            let price = 20.0 + unit(&x.ticker, 3) * 400.0;
            // A walk of a year of daily closes ending at the price.
            let n = 280;
            let mut closes = vec![0.0; n];
            let mut v = price;
            for i in (0..n).rev() {
                closes[i] = v;
                let step = (unit(&x.ticker, 100 + i as u64) - 0.5 - mood * 0.02) * 0.04;
                v = (v * (1.0 - step)).max(0.01);
            }
            let times: Vec<i64> = (0..n as i64)
                .map(|i| now_s - (n as i64 - 1 - i) * 86_400 * 7 / 5)
                .collect();
            let day = mood + (unit(&x.ticker, 4) - 0.5) * 3.0;
            Member {
                symbol,
                name: x.name,
                sector: x.sector,
                industry: x.industry,
                cap,
                price,
                change: changes(&times, &closes, price, Some(day), now_s),
                closes: tail(&closes),
            }
        })
        .collect();
    Board {
        market,
        members,
        sizing: Sizing::MarketCap,
        source: "Demo".into(),
        currency: if market == Market::India { "INR" } else { "USD" }.into(),
        at: provider::now_ms(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_lists_read() {
        let us = parse_sp500(SP500);
        assert!(us.len() >= 495, "{}", us.len());
        let brk = us.iter().find(|x| x.ticker == "BRK-B").expect("BRK-B");
        assert_eq!(brk.sector, "Finance");
        assert_eq!(brk.exchange, "NYSE");
        assert!(us.iter().all(|x| !x.exchange.is_empty()));
        let aapl = us.iter().find(|x| x.ticker == "AAPL").expect("AAPL");
        assert_eq!((aapl.sector.as_str(), aapl.exchange.as_str()), ("Technology", "NASDAQ"));
        let india = parse_nifty500(NIFTY500);
        assert!(india.len() >= 495, "{}", india.len());
        let mm = india.iter().find(|x| x.ticker == "M&M").expect("M&M");
        assert_eq!(mm.sector, "Automobiles");
        assert!(!india.iter().any(|x| x.name.ends_with("Ltd.")));
    }

    #[test]
    fn csv_quotes_and_live_list_without_exchange() {
        let t = "Symbol,Security,GICS Sector,GICS Sub-Industry,Headquarters Location\r\nAOS,A. O. Smith,Industrials,Building Products,\"Milwaukee, Wisconsin\"\r\nX,\"Say \"\"hi\"\"\",Energy,Oil,\"A\"\r\n";
        let l = parse_sp500(t);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].industry, "Building Products");
        assert_eq!(l[1].name, "Say \"hi\"");
        assert_eq!(l[0].exchange, "");
    }

    #[test]
    fn period_changes_from_closes() {
        // Daily closes for 400 days, rising 1 a day, the last at `now`.
        let now = jiff::civil::date(2026, 11, 6)
            .at(20, 0, 0, 0)
            .to_zoned(jiff::tz::TimeZone::UTC)
            .unwrap()
            .timestamp()
            .as_second();
        let times: Vec<i64> = (0..400).map(|i| now - (399 - i) * 86_400).collect();
        let closes: Vec<f64> = (0..400).map(|i| 100.0 + i as f64).collect();
        let c = changes(&times, &closes, 499.0, None, now);
        assert!((c[Period::Day.index()] - (499.0 / 498.0 - 1.0) * 100.0).abs() < 1e-9);
        assert!((c[Period::Week.index()] - (499.0 / 492.0 - 1.0) * 100.0).abs() < 1e-9);
        // 6 Oct → 6 Nov is 31 days.
        assert!((c[Period::Month.index()] - (499.0 / 468.0 - 1.0) * 100.0).abs() < 1e-9);
        // YTD: against 31 Dec 2025's close (310 days before 6 Nov 2026).
        assert!((c[Period::Ytd.index()] - (499.0 / 189.0 - 1.0) * 100.0).abs() < 1e-9);
        assert!((c[Period::Year.index()] - (499.0 / 134.0 - 1.0) * 100.0).abs() < 1e-9);
        // Too little history for a year: unknown, not wrong.
        let short = changes(&times[300..], &closes[300..], 499.0, Some(1.5), now);
        assert!(short[Period::Year.index()].is_nan());
        assert_eq!(short[Period::Day.index()], 1.5);
        assert!(short[Period::Month.index()].is_finite());
    }

    #[test]
    fn spark_screener_and_coingecko_replies_parse() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"AAPL":{"timestamp":[1,2,3],"close":[1.0,null,3.0],"fulldayPrice":3.1,"fulldayChangePercent":-0.4}}"#,
        )
        .unwrap();
        let s = &parse_spark(&v)["AAPL"];
        assert_eq!(s.price, 3.1);
        assert!(s.closes[1].is_nan());
        let v: serde_json::Value = serde_json::from_str(
            r#"{"finance":{"result":[{"quotes":[{"symbol":"NVDA","marketCap":4.5e12,"regularMarketPrice":186.6,"exchange":"NMS"},{"symbol":"GS-PD","regularMarketPrice":20}]}]}}"#,
        )
        .unwrap();
        assert_eq!(parse_quotes(&v), vec![("NVDA".into(), 4.5e12, 186.6, "NASDAQ".into())]);
        let v: serde_json::Value = serde_json::from_str(
            r#"[{"symbol":"btc","name":"Bitcoin","market_cap":1.5e12,"current_price":75000,"price_change_percentage_24h_in_currency":-0.4,"price_change_percentage_7d_in_currency":-2.3,"price_change_percentage_30d_in_currency":null,"price_change_percentage_1y_in_currency":-27.0}]"#,
        )
        .unwrap();
        let c = &parse_coingecko(&v)[0];
        assert_eq!((c.symbol.as_str(), c.day, c.week), ("BTC", -0.4, -2.3));
        assert!(c.month.is_nan());
    }

    #[test]
    fn crypto_filters_and_pairs() {
        for (s, n, p) in [
            ("USDT", "Tether", 1.0),
            ("WBTC", "Wrapped Bitcoin", 75000.0),
            ("STETH", "Lido Staked Ether", 2200.0),
            ("XAUT", "Tether Gold", 4000.0),
            ("SPCXB", "SpaceX (bStocks Token)", 200.0),
            ("USYC", "Circle USYC", 1.1),
        ] {
            assert!(excluded(s, n, p), "{s}");
        }
        for (s, n, p) in [
            ("BTC", "Bitcoin", 75000.0),
            ("DOGE", "Dogecoin", 0.2),
            ("S", "Sonic", 0.3),
        ] {
            assert!(!excluded(s, n, p), "{s}");
        }
        assert_eq!(category("btc"), "Bitcoin and payments");
        assert_eq!(category("PEPE"), "Memes");
        assert_eq!(category("NOTACOIN"), "Other");
        let sym = |id: &str, desc: &str, cur: &str| Symbol {
            id: id.into(),
            description: desc.into(),
            kind: AssetKind::Crypto,
            currency: cur.into(),
            tick_size: 0.01,
            min_qty: 0.0,
            timezone: "UTC".into(),
        };
        let known = vec![
            sym("KRAKEN:XBTUSD", "XBT / USD", "USD"),
            sym("COINBASE:BTCUSD", "BTC / USD", "USD"),
            sym("BINANCE:BTCUSDT", "BTC / USDT", "USDT"),
            sym("COINBASE:NEWUSD", "NEW / USD", "USD"),
            sym("BINANCE:ETHBTC", "ETH / BTC", "BTC"),
        ];
        let p = crypto_pairs(&known);
        assert_eq!(p["BTC"].id, "BINANCE:BTCUSDT");
        assert_eq!(p["NEW"].id, "COINBASE:NEWUSD");
        assert!(!p.contains_key("ETH"));
    }

    #[test]
    fn demo_boards_hold_together() {
        for m in Market::ALL {
            let b = made_up(m);
            assert!(b.members.len() >= 100, "{m:?}");
            assert!(b.members.iter().all(|x| x.cap > 0.0 && x.price > 0.0));
            assert!(
                b.members.iter().all(|x| x.change.iter().all(|c| c.is_finite())),
                "{m:?}"
            );
            let top = b.members.iter().max_by(|a, c| a.cap.total_cmp(&c.cap)).unwrap();
            let want = match m {
                Market::Us => "NASDAQ:NVDA",
                Market::India => "NSE:RELIANCE",
                Market::Crypto => "BINANCE:BTCUSDT",
            };
            assert_eq!(top.symbol.id, want);
        }
    }
}

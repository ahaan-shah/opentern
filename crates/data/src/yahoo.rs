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
    /// Pre- and after-hours bars too (US stocks).
    prepost: bool,
}

impl Default for Yahoo {
    fn default() -> Self {
        Yahoo {
            agent: provider::agent(),
            prepost: false,
        }
    }
}

impl Yahoo {
    /// Bars from pre-market (04:00) to after-hours (20:00) New York.
    pub fn extended() -> Yahoo {
        Yahoo {
            prepost: true,
            ..Yahoo::default()
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

/// Last price and previous close from a few daily bars. The previous close
/// is the last bar of a day before the one now trading (the day of
/// `regularMarketTime`): with `range=1d`, Yahoo's `chartPreviousClose` is
/// two sessions old (NIFTY showed -0.24% for -0.22%), and during a session
/// Yahoo's bars skip (Muhurat trading) the latest bar is the day before's.
fn quote_from(bars: &[Bar], meta: &serde_json::Value) -> Result<(f64, f64)> {
    let last = meta["regularMarketPrice"].as_f64().or(bars.last().map(|b| b.close));
    let tz = meta["exchangeTimezoneName"]
        .as_str()
        .and_then(|n| jiff::tz::TimeZone::get(n).ok())
        .unwrap_or(jiff::tz::TimeZone::UTC);
    let today = meta["regularMarketTime"]
        .as_i64()
        .map(|t| to_utc_midnight(t * 1000, &tz))
        .or(bars.last().map(|b| b.time));
    let prev = match today {
        Some(day) => bars.iter().rev().find(|b| b.time < day).map(|b| b.close),
        None => None,
    }
    .or_else(|| meta["chartPreviousClose"].as_f64())
    .or_else(|| meta["previousClose"].as_f64());
    match (last, prev) {
        (Some(l), Some(p)) => Ok((l, p)),
        _ => Err(Error::Bad("Yahoo: no quote".into())),
    }
}

/// Dividends and splits, dated like daily bars (00:00 UTC of the
/// exchange-local date).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Corporate {
    /// (ex-date, amount per share).
    pub dividends: Vec<(i64, f64)>,
    /// (date, ratio new shares per old: 4 for a 4-for-1 split).
    pub splits: Vec<(i64, f64)>,
    /// Earnings reports, oldest first (the last may be upcoming).
    pub earnings: Vec<crate::earnings::Earning>,
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

/// Adjusts for splits (and bonus issues, which Yahoo lists as splits) that
/// the prices don't reflect yet: a source that serves raw prices, or one
/// that hasn't caught up. Bars before it are divided by the factor and
/// their volume multiplied, so a 2-for-1 split doesn't read as a 50% fall;
/// prices already adjusted show no gap and are left alone. The factor is
/// the listed ratio when the price gap across the date matches it, else
/// the gap itself when it's large and a clean ratio (5, 5/2, 1/10): Yahoo
/// lists a split plus a bonus on one day as the split alone (Bajaj
/// Finance, June 2025: "2:1" for ×10 the shares), and its intraday bars
/// are adjusted by the listed part only. `tz` dates intraday bars.
pub fn split_adjust(bars: &[Bar], splits: &[(i64, f64)], tz: &jiff::tz::TimeZone) -> Option<Vec<Bar>> {
    let day = |t: i64| to_utc_midnight(t, tz);
    let mut factor = vec![1.0f64; bars.len()];
    let mut any = false;
    for &(date, ratio) in splits {
        if !ratio.is_finite() || ratio <= 0.0 {
            continue;
        }
        let k = bars.partition_point(|b| day(b.time) < date);
        if k == 0 || k >= bars.len() || day(bars[k].time) - date > 7 * 86_400_000 {
            continue;
        }
        let (before, after) = (bars[k - 1].close, bars[k].open);
        if before <= 0.0 || after <= 0.0 {
            continue;
        }
        let gap = (before / after).ln();
        // Small ratios can't be told apart from an ordinary day's move.
        let listed = ratio.ln().abs() >= 1.2f64.ln() && (gap - ratio.ln()).abs() < 0.4 * ratio.ln().abs();
        let f = if listed { Some(ratio) } else { clean_ratio(gap) };
        if let Some(f) = f {
            for x in factor.iter_mut().take(k) {
                *x /= f;
            }
            any = true;
        }
    }
    any.then(|| {
        bars.iter()
            .zip(factor)
            .map(|(b, f)| Bar {
                open: b.open * f,
                high: b.high * f,
                low: b.low * f,
                close: b.close * f,
                volume: if f > 0.0 { b.volume / f } else { b.volume },
                ..*b
            })
            .collect()
    })
}

/// A gap (log of before ÷ after) that is a split's or bonus's factor
/// within 3%: a whole number or a half (2, 2.5, 3, 5, 10, …; inverted for
/// a reverse split). Nothing under 2×, which a day's move could be.
fn clean_ratio(gap: f64) -> Option<f64> {
    if !gap.is_finite() || gap.abs() < 1.9f64.ln() {
        return None;
    }
    let g = gap.abs().exp();
    let r = (g * 2.0).round() / 2.0;
    (r >= 2.0 && (r.ln() - g.ln()).abs() < 0.03).then(|| if gap > 0.0 { r } else { 1.0 / r })
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
        // Indian weeks and months are built from the daily bars, which get
        // the special sessions Yahoo leaves out (its 1wk/1mo bars lack them).
        if !tf.is_intraday() && tf.unit != opentern_core::Unit::Day && crate::nse::covers(sym) {
            let day = Timeframe::parse("1D").ok_or_else(|| Error::Bad("No daily interval".into()))?;
            let whole = range == "max";
            let (daily, meta) = self.chart(sym, day, if whole { "max" } else { "2y" })?;
            let mut out = crate::resample::resample(&daily, tf, &opentern_core::calendar::Market::India.tz());
            // A window that starts mid-week or mid-month: its first bar is partial.
            if !whole && !out.is_empty() {
                out.remove(0);
            }
            return Ok((out, meta));
        }
        let (iv, _) = interval(tf).ok_or_else(|| Error::Bad(format!("Yahoo has no {} bars", tf.label())))?;
        // `range=max` quietly switches to 3-month bars, so ask for the whole
        // history by dates instead.
        let span = if range == "max" {
            format!("period1=0&period2={}", provider::now_ms() / 1000)
        } else {
            format!("range={range}")
        };
        let url = format!(
            "{CHART}/{}?interval={iv}&{span}&includePrePost={}&events=split",
            urlencode(&ticker(&sym.id)),
            self.prepost
        );
        let v = provider::get_json(&self.agent, &url)?;
        let (mut bars, meta) = parse_chart(&v, !tf.is_intraday())?;
        // Splits Yahoo hasn't (fully) adjusted these bars for.
        let splits = parse_events(&v).splits;
        if !splits.is_empty() {
            let tz = meta["exchangeTimezoneName"]
                .as_str()
                .and_then(|n| jiff::tz::TimeZone::get(n).ok())
                .unwrap_or(jiff::tz::TimeZone::UTC);
            if let Some(adj) = split_adjust(&bars, &splits, &tz) {
                bars = adj;
            }
        }
        // Sessions the exchange held that Yahoo's bars skip (Muhurat trading).
        if !tf.is_intraday() {
            crate::nse::fill_daily(sym, &mut bars);
            if let Some(b) = skipped_day(sym, &meta, &bars) {
                bars.push(b);
            }
        }
        // Never pass off bars of another length as the ones asked for.
        if let Some(g) = meta["dataGranularity"].as_str()
            && g != iv
            && !(g == "1h" && iv == "60m")
        {
            return Err(Error::Bad(format!("Yahoo sent {g} bars instead of {iv}.")));
        }
        Ok((bars, meta))
    }

    /// Bars from `start` and the reply's `meta` (the live quote).
    fn since_with_meta(&self, sym: &Symbol, tf: Timeframe, start: i64) -> Result<(Vec<Bar>, serde_json::Value)> {
        let age = provider::now_ms() - start;
        let range = match age / 86_400_000 {
            0..=4 if tf.is_intraday() => "5d",
            0..=25 => "1mo",
            _ => interval(tf).map(|(_, r)| r).unwrap_or("max"),
        };
        let (bars, meta) = self.chart(sym, tf, range)?;
        Ok((bars.into_iter().filter(|b| b.time >= start).collect(), meta))
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
        let day = Timeframe::parse("1D").ok_or_else(|| Error::Bad("No daily interval".into()))?;
        let (bars, meta) = self.chart(sym, day, "5d")?;
        quote_from(&bars, &meta)
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
        Ok(self.since_with_meta(sym, tf, start)?.0)
    }

    /// Yahoo only serves its fixed range per interval, all at once.
    fn before(&self, _: &Symbol, _: Timeframe, _: i64, _: usize) -> Result<Vec<Bar>> {
        Ok(Vec::new())
    }

    fn stream(&self, sym: &Symbol, tf: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()> {
        let mut last_poll = Instant::now();
        let mut since = provider::now_ms() - tf.millis().unwrap_or(86_400_000) * 2;
        let mut own = OwnBar::default();
        loop {
            if last_poll.elapsed() >= POLL {
                last_poll = Instant::now();
                let (bars, meta) = self.since_with_meta(sym, tf, since)?;
                let newest = bars.last().copied();
                for b in bars {
                    since = b.time;
                    if !sink(Live::Bar(b)) {
                        return Ok(());
                    }
                }
                // Trading Yahoo's bars don't show (yet): build it from the quote.
                if let Some(b) = own.update(sym, tf, &meta, newest, provider::now_ms())
                    && !sink(Live::Bar(b))
                {
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(250));
            if !sink(Live::Idle) {
                return Ok(());
            }
        }
    }
}

/// Today's bar from Yahoo's quote, when its bars skip a session that's
/// running or ended today (Muhurat trading) and the exchange's file isn't
/// out yet: high, low, close and volume are the quote's; the open is the
/// previous official close (price − Yahoo's day change), as Yahoo gives no
/// open. The exchange's file replaces it that evening (same time).
pub(crate) fn skipped_day(sym: &Symbol, meta: &serde_json::Value, bars: &[Bar]) -> Option<Bar> {
    use opentern_core::calendar::{self, Market};
    if !crate::nse::covers(sym) {
        return None;
    }
    let price = meta["regularMarketPrice"]
        .as_f64()
        .filter(|p| p.is_finite() && *p > 0.0)?;
    let t = meta["regularMarketTime"].as_i64()? * 1000;
    let tz = Market::India.tz();
    let local = jiff::Timestamp::from_millisecond(t).ok()?.to_zoned(tz.clone()).date();
    let skipped = calendar::special(Market::India, local).is_some()
        || matches!(
            local.weekday(),
            jiff::civil::Weekday::Saturday | jiff::civil::Weekday::Sunday
        )
        || calendar::holiday(Market::India, local).is_some();
    let day = to_utc_midnight(t, &tz);
    if !skipped || bars.last().is_some_and(|b| b.time >= day) {
        return None;
    }
    let prev = meta["fulldayChange"]
        .as_f64()
        .map(|c| price - c)
        .filter(|p| *p > 0.0)
        .unwrap_or(price);
    let high = meta["regularMarketDayHigh"]
        .as_f64()
        .unwrap_or(price)
        .max(price)
        .max(prev);
    let low = meta["regularMarketDayLow"]
        .as_f64()
        .unwrap_or(price)
        .min(price)
        .min(prev);
    Some(Bar {
        time: day,
        open: prev,
        high,
        low,
        close: price,
        volume: meta["regularMarketVolume"].as_f64().unwrap_or(0.0),
    })
}

/// An intraday bar built from Yahoo's live quote (`meta.regularMarketPrice`
/// and `regularMarketTime`) for trading its bars don't show. Its quotes count
/// Diwali's Muhurat session, but its bars leave it out (8 Nov 2026), so
/// without this a chart would sit still while the session runs. Only for
/// Indian symbols, only while the quote is fresh (≤ 10 minutes old) and
/// the exchange is in a session at the quote's time, or it's a day with
/// no regular session (a special session not yet listed in the
/// calendar). Once the exchange's file or Yahoo's own bar has the period,
/// that replaces this one (same time).
#[derive(Default)]
pub(crate) struct OwnBar {
    bar: Option<Bar>,
    /// The day's volume when the bar started (intraday bars count the change).
    day_volume_at_start: f64,
}

impl OwnBar {
    pub(crate) fn update(
        &mut self,
        sym: &Symbol,
        tf: Timeframe,
        meta: &serde_json::Value,
        newest: Option<Bar>,
        now: i64,
    ) -> Option<Bar> {
        use opentern_core::calendar::{self, Market, State};
        if !crate::nse::covers(sym) {
            return None;
        }
        let price = meta["regularMarketPrice"]
            .as_f64()
            .filter(|p| p.is_finite() && *p > 0.0)?;
        let t = meta["regularMarketTime"].as_i64()? * 1000;
        if now - t > 10 * 60_000 {
            self.bar = None;
            return None;
        }
        let tz = Market::India.tz();
        let local = jiff::Timestamp::from_millisecond(t).ok()?.to_zoned(tz.clone()).date();
        let unlisted = matches!(
            local.weekday(),
            jiff::civil::Weekday::Saturday | jiff::civil::Weekday::Sunday
        ) || calendar::holiday(Market::India, local).is_some();
        if calendar::state(Market::India, t) != State::Open && !unlisted {
            return None;
        }
        if !tf.is_intraday() {
            // Days come with `skipped_day`; weeks and months are built from days.
            return None;
        }
        let day_volume = meta["regularMarketVolume"].as_f64().unwrap_or(0.0);
        let bucket = crate::builder::BarBuilder::new(tf, tz.clone(), None).bucket(t);
        if newest.is_some_and(|n| n.time >= bucket) {
            // Yahoo has this period itself.
            self.bar = None;
            return None;
        }
        let bar = match self.bar {
            Some(b) if b.time == bucket => Bar {
                high: b.high.max(price),
                low: b.low.min(price),
                close: price,
                ..b
            },
            _ => {
                self.day_volume_at_start = day_volume;
                Bar {
                    time: bucket,
                    open: price,
                    high: price,
                    low: price,
                    close: price,
                    volume: 0.0,
                }
            }
        };
        let bar = Bar {
            volume: (day_volume - self.day_volume_at_start).max(0.0),
            ..bar
        };
        self.bar = Some(bar);
        Some(bar)
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
    fn missing_splits_are_adjusted_and_adjusted_ones_left_alone() {
        let day = 86_400_000;
        let mk = |closes: &[f64]| -> Vec<Bar> {
            closes
                .iter()
                .enumerate()
                .map(|(i, c)| Bar {
                    time: i as i64 * day,
                    open: *c,
                    high: *c,
                    low: *c,
                    close: *c,
                    volume: 100.0,
                })
                .collect()
        };
        let utc = jiff::tz::TimeZone::UTC;
        // Raw prices: 2-for-1 on day 2.
        let raw = mk(&[200.0, 202.0, 101.0, 100.0]);
        let adj = split_adjust(&raw, &[(2 * day, 2.0)], &utc).expect("adjusted");
        assert_eq!(
            adj.iter().map(|b| b.close).collect::<Vec<_>>(),
            vec![100.0, 101.0, 101.0, 100.0]
        );
        assert_eq!(adj[0].volume, 200.0);
        assert_eq!(adj[3].volume, 100.0);
        // Already adjusted: nothing to do.
        assert!(split_adjust(&adj, &[(2 * day, 2.0)], &utc).is_none());
        // Yahoo lists a split plus a bonus as the split alone: a clean gap
        // on the date is taken instead (Bajaj Finance's hourly bars).
        let both = split_adjust(&mk(&[4665.5, 931.5]), &[(day, 2.0)], &utc).expect("adjusted");
        assert!((both[0].close - 933.1).abs() < 1e-9);
        // An ordinary or untidy move on the date is left alone.
        assert!(split_adjust(&mk(&[100.0, 97.0]), &[(day, 2.0)], &utc).is_none());
        assert!(split_adjust(&mk(&[100.0, 70.0]), &[(day, 2.0)], &utc).is_none());
        // Reverse split 1-for-5.
        let rev = split_adjust(&mk(&[2.0, 2.1, 10.4]), &[(2 * day, 0.2)], &utc).expect("adjusted");
        assert!((rev[0].close - 10.0).abs() < 1e-9);
    }

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
        // The previous close is the bar before the latest, not chartPreviousClose.
        assert_eq!(quote_from(&bars, &meta).unwrap(), (3.0, 1.2));
        // Trading on a day Yahoo has no bar for yet (a Muhurat session): the
        // latest bar is the previous close.
        let mut live = meta.clone();
        live["regularMarketTime"] = serde_json::json!(1_705_100_000); // 12 Jan, no bar yet
        assert_eq!(quote_from(&bars, &live).unwrap(), (3.0, 2.2));
        let none: serde_json::Value =
            serde_json::from_str(r#"{"chart":{"result":null,"error":{"description":"No data found"}}}"#).unwrap();
        assert!(parse_chart(&none, true).is_err());
    }

    fn sbin() -> Symbol {
        Symbol {
            id: "NSE:SBIN".into(),
            description: "State Bank of India".into(),
            kind: AssetKind::Stock,
            currency: "INR".into(),
            tick_size: 0.05,
            min_qty: 0.0,
            timezone: "Asia/Kolkata".into(),
        }
    }

    /// 2026-11-08 (Sunday, Muhurat trading) at `hh:mm` IST, in seconds.
    fn muhurat(hh: i64, mm: i64) -> i64 {
        1_794_096_000 + (hh * 60 + mm) * 60 - 19_800
    }

    fn quote(t: i64, price: f64, volume: f64) -> serde_json::Value {
        serde_json::json!({
            "exchangeTimezoneName": "Asia/Kolkata",
            "regularMarketTime": t,
            "regularMarketPrice": price,
            "regularMarketDayHigh": 1025.0,
            "regularMarketDayLow": 1015.1,
            "regularMarketVolume": volume,
            "fulldayChange": price - 1018.5,
        })
    }

    const DAY: i64 = 86_400_000;

    #[test]
    fn a_session_yahoos_bars_skip_gets_a_bar_from_the_quote() {
        let sunday = 1_794_096_000_000;
        let friday = sunday - 2 * DAY;
        let bars = vec![Bar {
            time: friday,
            open: 996.0,
            high: 1020.0,
            low: 990.5,
            close: 1018.5,
            volume: 1.0,
        }];
        let meta = quote(muhurat(14, 50), 1016.0, 1_950_983.0);
        let b = skipped_day(&sbin(), &meta, &bars).expect("a bar for the Muhurat session");
        assert_eq!(b.time, sunday);
        assert_eq!(
            (b.open, b.high, b.low, b.close, b.volume),
            (1018.5, 1025.0, 1015.1, 1016.0, 1_950_983.0)
        );
        // Once the day has a bar (the exchange's file), nothing is added.
        assert!(skipped_day(&sbin(), &meta, &[bars[0], b]).is_none());
        // A regular weekday is Yahoo's own business.
        let monday = quote(muhurat(14, 50) + 86_400, 1042.0, 1.0);
        assert!(skipped_day(&sbin(), &monday, &bars).is_none());
        // Not for US stocks.
        let aapl = Symbol {
            id: "NASDAQ:AAPL".into(),
            ..sbin()
        };
        assert!(skipped_day(&aapl, &meta, &bars).is_none());
    }

    #[test]
    fn intraday_bars_are_built_from_the_quote_while_yahoo_skips_a_session() {
        let tf = Timeframe::parse("5m").unwrap();
        let friday_last = Bar {
            time: (muhurat(15, 25) - 2 * 86_400) * 1000,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1018.5,
            volume: 1.0,
        };
        let mut own = OwnBar::default();
        let at = |hh, mm| muhurat(hh, mm) * 1000 + 5_000;
        let a = own
            .update(
                &sbin(),
                tf,
                &quote(muhurat(14, 21), 1023.2, 1000.0),
                Some(friday_last),
                at(14, 21),
            )
            .unwrap();
        assert_eq!(a.time, muhurat(14, 20) * 1000);
        assert_eq!((a.open, a.close, a.volume), (1023.2, 1023.2, 0.0));
        let b = own
            .update(
                &sbin(),
                tf,
                &quote(muhurat(14, 23), 1025.0, 1600.0),
                Some(friday_last),
                at(14, 23),
            )
            .unwrap();
        assert_eq!(
            (b.time, b.open, b.high, b.close, b.volume),
            (a.time, 1023.2, 1025.0, 1025.0, 600.0)
        );
        let c = own
            .update(
                &sbin(),
                tf,
                &quote(muhurat(14, 26), 1019.0, 2000.0),
                Some(friday_last),
                at(14, 26),
            )
            .unwrap();
        assert_eq!((c.time, c.open, c.volume), (muhurat(14, 25) * 1000, 1019.0, 0.0));
        // Yahoo's own bar for the period wins.
        let yahoo = Bar {
            time: c.time,
            ..friday_last
        };
        assert!(
            own.update(
                &sbin(),
                tf,
                &quote(muhurat(14, 27), 1019.5, 2100.0),
                Some(yahoo),
                at(14, 27)
            )
            .is_none()
        );
        // A stale quote (the session ended long ago) builds nothing.
        assert!(
            own.update(
                &sbin(),
                tf,
                &quote(muhurat(15, 30), 1016.0, 3000.0),
                Some(friday_last),
                at(17, 0)
            )
            .is_none()
        );
        // After a regular close, a late quote time isn't a new bar.
        let monday_late = muhurat(15, 45) + 86_400;
        assert!(
            own.update(
                &sbin(),
                tf,
                &quote(monday_late, 1042.0, 1.0),
                Some(friday_last),
                monday_late * 1000 + 5_000
            )
            .is_none()
        );
    }
}

//! The exchanges' own end-of-day files ("bhavcopy"): the ground truth for
//! Indian prices. Yahoo's bars leave out sessions held on a weekend or
//! holiday (Diwali's Muhurat trading, special Saturday sessions; Muhurat
//! on Sunday 8 Nov 2026 is missing from Yahoo's daily *and* intraday
//! bars, though its quotes count it). Every such session is listed in
//! `opentern_core::calendar::special_sessions`; [`fill_daily`] adds the
//! missing days from these files, so charts, indicators, the screener,
//! the watchlist and the overview all see what the exchange traded.
//!
//! Sources (public, no key): NSE equities
//! (`sec_bhavdata_full_DDMMYYYY.csv` since late 2019, else the zipped
//! `cmDDMONYYYYbhav.csv`), NSE indices (`ind_close_all_DDMMYYYY.csv`) and
//! BSE equities (`BhavCopy_BSE_CM_…_YYYYMMDD_F_0000.CSV`, the UDiFF file).
//! A day's files never change once published, so each is parsed once
//! and kept as JSON under `<data dir>/nse/`.

use crate::provider::{self, Error, Result};
use jiff::civil::Date;
use opentern_core::calendar::{self, Market};
use opentern_core::{Bar, Symbol};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// One instrument's session, as the exchange reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    /// The previous session's official close (adjusted by the exchange
    /// for corporate actions taking effect that day).
    pub prev: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl Row {
    fn to_array(self) -> [f64; 6] {
        [self.prev, self.open, self.high, self.low, self.close, self.volume]
    }

    fn from_array(a: [f64; 6]) -> Row {
        let [prev, open, high, low, close, volume] = a;
        Row {
            prev,
            open,
            high,
            low,
            close,
            volume,
        }
    }
}

/// One day's rows, keyed by our symbol id (`NSE:SBIN`, `NSE:NIFTY`, `BSE:SBIN`).
pub type Day = HashMap<String, Row>;

static DIR: OnceLock<PathBuf> = OnceLock::new();
type Memo = Mutex<HashMap<(Source, Date), Arc<Day>>>;
static MEMO: OnceLock<Memo> = OnceLock::new();

/// Where parsed files are kept (the hub's data dir). Without it nothing is
/// saved to disk, but lookups still work.
pub fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir.join("nse"));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Source {
    NseStocks,
    NseIndices,
    BseStocks,
}

impl Source {
    fn of(sym: &Symbol) -> Option<Source> {
        match sym.exchange() {
            "NSE" if index_name(&sym.id).is_some() => Some(Source::NseIndices),
            "NSE" => Some(Source::NseStocks),
            // SENSEX and other BSE indices: BSE's index files aren't read yet.
            "BSE" if sym.kind == opentern_core::AssetKind::Index => None,
            "BSE" => Some(Source::BseStocks),
            _ => None,
        }
    }

    fn file(self, d: Date) -> String {
        let tag = match self {
            Source::NseStocks => "nse",
            Source::NseIndices => "nse-indices",
            Source::BseStocks => "bse",
        };
        format!("{tag}-{}.json", d.strftime("%Y%m%d"))
    }
}

/// NSE's name for an index we chart.
fn index_name(id: &str) -> Option<&'static str> {
    match id {
        "NSE:NIFTY" => Some("Nifty 50"),
        "NSE:BANKNIFTY" => Some("Nifty Bank"),
        _ => None,
    }
}

/// Whether the exchanges' files cover `sym` (NSE and BSE stocks, NIFTY
/// and Bank Nifty).
pub fn covers(sym: &Symbol) -> bool {
    Source::of(sym).is_some()
}

/// The exchange's row for `sym` on local date `d`, if it traded.
pub fn row(sym: &Symbol, d: Date) -> Option<Row> {
    let src = Source::of(sym)?;
    let key = match src {
        Source::NseIndices => index_name(&sym.id)?.to_string(),
        _ => sym.id.clone(),
    };
    day(src, d).ok()?.get(&key).copied()
}

/// Downloads run one at a time (the overview asks for hundreds of symbols
/// at once, all on the same few files), and a file that couldn't be had
/// isn't asked for again for an hour.
static FETCH: Mutex<()> = Mutex::new(());
static FAILED: OnceLock<Mutex<HashMap<(Source, Date), std::time::Instant>>> = OnceLock::new();
const RETRY: std::time::Duration = std::time::Duration::from_secs(3600);

fn day(src: Source, d: Date) -> Result<Arc<Day>> {
    let memo = MEMO.get_or_init(|| Mutex::new(HashMap::new()));
    let cached = || memo.lock().ok().and_then(|m| m.get(&(src, d)).cloned());
    if let Some(day) = cached() {
        return Ok(day);
    }
    let failed = FAILED.get_or_init(|| Mutex::new(HashMap::new()));
    let recent_failure = || {
        failed
            .lock()
            .ok()
            .and_then(|f| f.get(&(src, d)).copied())
            .is_some_and(|at| at.elapsed() < RETRY)
    };
    if recent_failure() {
        return Err(Error::Offline);
    }
    let _one = FETCH.lock().unwrap_or_else(|e| e.into_inner());
    // Another thread may have fetched it while this one waited.
    if let Some(day) = cached() {
        return Ok(day);
    }
    if recent_failure() {
        return Err(Error::Offline);
    }
    let path = DIR.get().map(|dir| dir.join(src.file(d)));
    let saved = path
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<HashMap<String, [f64; 6]>>(&b).ok())
        .map(|m| m.into_iter().map(|(k, a)| (k, Row::from_array(a))).collect::<Day>());
    let day = match saved {
        Some(day) => day,
        None => match download(src, d) {
            Ok(day) if !day.is_empty() => {
                save(path.as_ref(), &day);
                day
            }
            // Never published (or only a copy of another day's): once the
            // date is well past, that's final.
            Ok(_) | Err(Fail::Missing) if is_old(d) => {
                save(path.as_ref(), &Day::new());
                Day::new()
            }
            _ => {
                if let Ok(mut f) = failed.lock() {
                    f.insert((src, d), std::time::Instant::now());
                }
                return Err(Error::Offline);
            }
        },
    };
    let day = Arc::new(day);
    if let Ok(mut m) = memo.lock() {
        m.insert((src, d), day.clone());
    }
    Ok(day)
}

fn save(path: Option<&PathBuf>, day: &Day) {
    let Some(p) = path else { return };
    let _ = std::fs::create_dir_all(p.parent().unwrap_or(p));
    let plain: HashMap<&String, [f64; 6]> = day.iter().map(|(k, r)| (k, r.to_array())).collect();
    if let Ok(json) = serde_json::to_vec(&plain) {
        let _ = std::fs::write(p, json);
    }
}

/// More than a month ago: the exchange has published whatever it will.
fn is_old(d: Date) -> bool {
    let today = jiff::Zoned::now().with_time_zone(Market::India.tz()).date();
    today.since(d).is_ok_and(|s| s.get_days() > 31)
}

/// Why a file couldn't be had.
#[derive(Debug)]
enum Fail {
    /// The exchange has no such file (404).
    Missing,
    /// Blocked, offline or damaged: try again later.
    Later,
}

const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

fn download(src: Source, d: Date) -> std::result::Result<Day, Fail> {
    let agent = provider::agent();
    match src {
        Source::NseStocks => {
            let full = format!(
                "https://archives.nseindia.com/products/content/sec_bhavdata_full_{}.csv",
                d.strftime("%d%m%Y")
            );
            match get_bytes(&agent, &full, &[]) {
                Ok(b) => Ok(parse_nse_full(&String::from_utf8_lossy(&b), d)),
                // Before late 2019 only the older zipped file exists.
                Err(Fail::Missing) => {
                    let mon = MONTHS[d.month() as usize - 1];
                    let zipped = format!(
                        "https://nsearchives.nseindia.com/content/historical/EQUITIES/{}/{mon}/cm{:02}{mon}{}bhav.csv.zip",
                        d.year(),
                        d.day(),
                        d.year()
                    );
                    let text = unzip(&get_bytes(&agent, &zipped, &[])?).map_err(|_| Fail::Later)?;
                    Ok(parse_nse_cm(&text, d))
                }
                Err(e) => Err(e),
            }
        }
        Source::NseIndices => {
            let url = format!(
                "https://archives.nseindia.com/content/indices/ind_close_all_{}.csv",
                d.strftime("%d%m%Y")
            );
            Ok(parse_nse_indices(
                &String::from_utf8_lossy(&get_bytes(&agent, &url, &[])?),
                d,
            ))
        }
        Source::BseStocks => {
            let url = format!(
                "https://www.bseindia.com/download/BhavCopy/Equity/BhavCopy_BSE_CM_0_0_0_{}_F_0000.CSV",
                d.strftime("%Y%m%d")
            );
            let b = get_bytes(&agent, &url, &[("Referer", "https://www.bseindia.com/")])?;
            Ok(parse_bse(&String::from_utf8_lossy(&b), d))
        }
    }
}

fn get_bytes(agent: &ureq::Agent, url: &str, headers: &[(&str, &str)]) -> std::result::Result<Vec<u8>, Fail> {
    let mut req = agent.get(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut resp = req.call().map_err(|e| match e {
        ureq::Error::StatusCode(404) => Fail::Missing,
        _ => Fail::Later,
    })?;
    resp.body_mut()
        .with_config()
        .limit(32 * 1024 * 1024)
        .read_to_vec()
        .map_err(|_| Fail::Later)
}

fn unzip(bytes: &[u8]) -> Result<String> {
    use std::io::Read;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| Error::Bad("The exchange's file is damaged.".into()))?;
    let mut f = z
        .by_index(0)
        .map_err(|_| Error::Bad("The exchange's file is damaged.".into()))?;
    let mut s = String::new();
    f.read_to_string(&mut s)
        .map_err(|_| Error::Bad("The exchange's file is damaged.".into()))?;
    Ok(s)
}

/// Whether a file's date field is `d`. Files must be checked: NSE serves
/// some non-trading days as a copy of the day before (the file named
/// 23 Apr 2023, a Sunday, holds 21 Apr's trading).
fn same_day(field: &str, d: Date) -> bool {
    ["%d-%b-%Y", "%d-%m-%Y", "%Y-%m-%d"]
        .iter()
        .any(|f| d.strftime(f).to_string().eq_ignore_ascii_case(field.trim()))
}

/// Splits a CSV line, honouring double quotes.
fn fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur).trim().to_string()),
            '\r' => {}
            _ => cur.push(c),
        }
    }
    out.push(cur.trim().to_string());
    out
}

/// Reads a CSV into rows of named columns.
fn table(text: &str) -> (Vec<String>, Vec<Vec<String>>) {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let head = lines.next().map(fields).unwrap_or_default();
    (head, lines.map(fields).collect())
}

fn col(head: &[String], name: &str) -> Option<usize> {
    head.iter().position(|h| h.eq_ignore_ascii_case(name))
}

fn num(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Equity series that trade on the normal market (EQ, plus the
/// trade-for-trade and SME ones Yahoo also lists under `.NS`).
fn equity_series(s: &str) -> Option<u8> {
    match s {
        "EQ" => Some(0),
        "BE" | "BZ" => Some(1),
        "SM" | "ST" => Some(2),
        _ => None,
    }
}

fn put(out: &mut Day, rank: &mut HashMap<String, u8>, key: String, r: u8, row: Row) {
    if rank.get(&key).is_none_or(|old| r < *old) {
        rank.insert(key.clone(), r);
        out.insert(key, row);
    }
}

/// NSE's `sec_bhavdata_full` file.
pub(crate) fn parse_nse_full(text: &str, d: Date) -> Day {
    let (head, rows) = table(text);
    let c = |n| col(&head, n);
    let (Some(sym), Some(ser), Some(prev), Some(o), Some(h), Some(l), Some(cl), Some(v)) = (
        c("SYMBOL"),
        c("SERIES"),
        c("PREV_CLOSE"),
        c("OPEN_PRICE"),
        c("HIGH_PRICE"),
        c("LOW_PRICE"),
        c("CLOSE_PRICE"),
        c("TTL_TRD_QNTY"),
    ) else {
        return Day::new();
    };
    let date = c("DATE1");
    let (mut out, mut rank) = (Day::new(), HashMap::new());
    for f in rows {
        let get = |i: usize| f.get(i).map(String::as_str).unwrap_or("");
        if !date.is_some_and(|i| same_day(get(i), d)) {
            continue;
        }
        let Some(r) = equity_series(get(ser)) else { continue };
        if let (Some(prev), Some(open), Some(high), Some(low), Some(close)) =
            (num(get(prev)), num(get(o)), num(get(h)), num(get(l)), num(get(cl)))
        {
            let row = Row {
                prev,
                open,
                high,
                low,
                close,
                volume: num(get(v)).unwrap_or(0.0),
            };
            put(&mut out, &mut rank, format!("NSE:{}", get(sym)), r, row);
        }
    }
    out
}

/// NSE's older zipped `cm…bhav.csv`.
pub(crate) fn parse_nse_cm(text: &str, d: Date) -> Day {
    let (head, rows) = table(text);
    let c = |n| col(&head, n);
    let (Some(sym), Some(ser), Some(prev), Some(o), Some(h), Some(l), Some(cl), Some(v)) = (
        c("SYMBOL"),
        c("SERIES"),
        c("PREVCLOSE"),
        c("OPEN"),
        c("HIGH"),
        c("LOW"),
        c("CLOSE"),
        c("TOTTRDQTY"),
    ) else {
        return Day::new();
    };
    let date = c("TIMESTAMP");
    let (mut out, mut rank) = (Day::new(), HashMap::new());
    for f in rows {
        let get = |i: usize| f.get(i).map(String::as_str).unwrap_or("");
        if !date.is_some_and(|i| same_day(get(i), d)) {
            continue;
        }
        let Some(r) = equity_series(get(ser)) else { continue };
        if let (Some(prev), Some(open), Some(high), Some(low), Some(close)) =
            (num(get(prev)), num(get(o)), num(get(h)), num(get(l)), num(get(cl)))
        {
            let row = Row {
                prev,
                open,
                high,
                low,
                close,
                volume: num(get(v)).unwrap_or(0.0),
            };
            put(&mut out, &mut rank, format!("NSE:{}", get(sym)), r, row);
        }
    }
    out
}

/// NSE's `ind_close_all` file, keyed by NSE's index name.
pub(crate) fn parse_nse_indices(text: &str, d: Date) -> Day {
    let (head, rows) = table(text);
    let c = |n| col(&head, n);
    let (Some(name), Some(o), Some(h), Some(l), Some(cl), Some(chg)) = (
        c("Index Name"),
        c("Open Index Value"),
        c("High Index Value"),
        c("Low Index Value"),
        c("Closing Index Value"),
        c("Points Change"),
    ) else {
        return Day::new();
    };
    let v = c("Volume");
    let date = c("Index Date");
    let mut out = Day::new();
    for f in rows {
        let get = |i: usize| f.get(i).map(String::as_str).unwrap_or("");
        if !date.is_some_and(|i| same_day(get(i), d)) {
            continue;
        }
        if let (Some(open), Some(high), Some(low), Some(close), Some(change)) =
            (num(get(o)), num(get(h)), num(get(l)), num(get(cl)), num(get(chg)))
        {
            let volume = v.and_then(|v| num(get(v))).unwrap_or(0.0);
            out.insert(
                get(name).to_string(),
                Row {
                    prev: close - change,
                    open,
                    high,
                    low,
                    close,
                    volume,
                },
            );
        }
    }
    out
}

/// BSE's UDiFF equity file.
pub(crate) fn parse_bse(text: &str, d: Date) -> Day {
    let (head, rows) = table(text);
    let c = |n| col(&head, n);
    let (Some(sym), Some(tp), Some(prev), Some(o), Some(h), Some(l), Some(cl), Some(v)) = (
        c("TckrSymb"),
        c("FinInstrmTp"),
        c("PrvsClsgPric"),
        c("OpnPric"),
        c("HghPric"),
        c("LwPric"),
        c("ClsPric"),
        c("TtlTradgVol"),
    ) else {
        return Day::new();
    };
    let date = c("TradDt");
    let mut out = Day::new();
    for f in rows {
        let get = |i: usize| f.get(i).map(String::as_str).unwrap_or("");
        if !date.is_some_and(|i| same_day(get(i), d)) {
            continue;
        }
        if get(tp) != "STK" {
            continue;
        }
        if let (Some(prev), Some(open), Some(high), Some(low), Some(close)) =
            (num(get(prev)), num(get(o)), num(get(h)), num(get(l)), num(get(cl)))
        {
            let row = Row {
                prev,
                open,
                high,
                low,
                close,
                volume: num(get(v)).unwrap_or(0.0),
            };
            out.insert(format!("BSE:{}", get(sym)), row);
        }
    }
    out
}

/// Local dates of special sessions that have ended by `now` (ms) and fall
/// after `from` (ms).
fn ended_sessions(from: i64, now: i64) -> Vec<(Date, i64)> {
    calendar::special_sessions(Market::India)
        .into_iter()
        .filter_map(|(d, _, close)| {
            let midnight = midnight_ms(d)?;
            (midnight > from && close <= now).then_some((d, midnight))
        })
        .collect()
}

fn midnight_ms(d: Date) -> Option<i64> {
    d.to_zoned(jiff::tz::TimeZone::UTC)
        .ok()
        .map(|z| z.timestamp().as_millisecond())
}

/// Adds the special sessions the exchange held that `bars` (daily, stamped
/// 00:00 UTC of the local date, oldest first) are missing. Prices are
/// scaled to the bars' own basis (a later split Yahoo has adjusted for):
/// the factor is the bar before's close over the exchange's previous
/// close, which is the same session. Sessions before the first bar are
/// left out (the series starts where its source starts).
pub fn fill_daily(sym: &Symbol, bars: &mut Vec<Bar>) {
    if Source::of(sym).is_none() || bars.is_empty() {
        return;
    }
    let now = provider::now_ms();
    let first = bars[0].time;
    let mut added = false;
    for (d, t) in ended_sessions(first, now) {
        let k = bars.partition_point(|b| b.time < t);
        if bars.get(k).is_some_and(|b| b.time == t) {
            continue;
        }
        let Some(r) = row(sym, d) else { continue };
        let factor = match bars.get(k.wrapping_sub(1)) {
            Some(before) if k > 0 && r.prev > 0.0 && before.close > 0.0 => {
                let f = before.close / r.prev;
                if same_basis(f) { 1.0 } else { f }
            }
            _ => 1.0,
        };
        bars.insert(
            k,
            Bar {
                time: t,
                open: r.open * factor,
                high: r.high * factor,
                low: r.low * factor,
                close: r.close * factor,
                volume: r.volume / factor,
            },
        );
        added = true;
    }
    if added {
        bars.dedup_by_key(|b| b.time);
    }
}

/// No adjustment between the source's bars and the exchange's file: the
/// same close on both sides, give or take Yahoo's single-precision prices
/// (NIFTY's 25030.35 arrives as 25030.349609…). Real adjustments are
/// whole ratios (2:1, 1:10), far outside this.
fn same_basis(factor: f64) -> bool {
    (factor - 1.0).abs() < 1e-6
}

/// [`fill_daily`] for a series of daily closes with times in seconds (as
/// Yahoo's spark gives them); a filled session is stamped at its local
/// midnight.
pub fn fill_closes(sym: &Symbol, times: &mut Vec<i64>, closes: &mut Vec<f64>) {
    if Source::of(sym).is_none() || times.len() != closes.len() || times.is_empty() {
        return;
    }
    let kolkata = Market::India.tz();
    let local_midnight = |s: i64| {
        jiff::Timestamp::from_second(s)
            .ok()
            .and_then(|ts| midnight_ms(ts.to_zoned(kolkata.clone()).date()))
    };
    let now = provider::now_ms();
    for (d, t) in ended_sessions(local_midnight(times[0]).unwrap_or(i64::MAX), now) {
        let days: Vec<Option<i64>> = times.iter().map(|s| local_midnight(*s)).collect();
        if days.contains(&Some(t)) {
            continue;
        }
        let k = days.partition_point(|m| m.is_some_and(|m| m < t));
        let Some(r) = row(sym, d) else { continue };
        let before = (0..k).rev().map(|i| closes[i]).find(|c| c.is_finite());
        let factor = match before {
            Some(b) if r.prev > 0.0 && b > 0.0 && !same_basis(b / r.prev) => b / r.prev,
            _ => 1.0,
        };
        times.insert(k, t / 1000);
        closes.insert(k, r.close * factor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = "SYMBOL, SERIES, DATE1, PREV_CLOSE, OPEN_PRICE, HIGH_PRICE, LOW_PRICE, LAST_PRICE, CLOSE_PRICE, AVG_PRICE, TTL_TRD_QNTY, TURNOVER_LACS, NO_OF_TRADES, DELIV_QTY, DELIV_PER
SBIN, EQ, 08-Nov-2026, 1018.50, 1023.20, 1025.00, 1015.10, 1016.00, 1016.00, 1019.43, 1950983, 19888.96, 43456, 938945, 48.13
SBIN, N1, 08-Nov-2026, 100.00, 100.00, 100.00, 100.00, 100.00, 100.00, 100.00, 5, 0.01, 1, -, -
M&M, BE, 08-Nov-2026, 3000.00, 3010.00, 3020.00, 2990.00, 3005.00, 3005.00, 3004.00, 1000, 30.0, 10, 500, 50.00
";

    #[test]
    fn only_dates_well_past_count_as_final() {
        assert!(is_old(jiff::civil::date(2010, 11, 5)));
        let today = jiff::Zoned::now().with_time_zone(Market::India.tz()).date();
        assert!(!is_old(today));
        assert!(!is_old(today.yesterday().unwrap()));
    }

    #[test]
    fn reads_the_exchanges_files() {
        let nov8 = jiff::civil::date(2026, 11, 8);
        let d = parse_nse_full(FULL, nov8);
        // A copy of another day's file counts for nothing.
        assert!(parse_nse_full(FULL, jiff::civil::date(2026, 11, 7)).is_empty());
        let sbin = d["NSE:SBIN"];
        assert_eq!(
            (sbin.prev, sbin.open, sbin.high, sbin.low, sbin.close),
            (1018.5, 1023.2, 1025.0, 1015.1, 1016.0)
        );
        assert_eq!(sbin.volume, 1950983.0);
        assert_eq!(d["NSE:M&M"].close, 3005.0);

        let cm = "SYMBOL,SERIES,OPEN,HIGH,LOW,CLOSE,LAST,PREVCLOSE,TOTTRDQTY,TOTTRDVAL,TIMESTAMP,TOTALTRADES,ISIN,\n\
                  SBIN,EQ,1.5,2.5,1.0,2.0,2.0,1.4,10,20,11-NOV-2015,3,X,\n";
        assert_eq!(parse_nse_cm(cm, jiff::civil::date(2015, 11, 11))["NSE:SBIN"].prev, 1.4);

        let idx = "Index Name,Index Date,Open Index Value,High Index Value,Low Index Value,Closing Index Value,Points Change,Change(%),Volume,Turnover (Rs. Cr.),P/E,P/B,Div Yield\n\
                   Nifty 50,08-11-2026,25070.45,25131.85,24985.3,24986.4,-43.95,-.18,68360599,4963.05,20.61,3.07,1.38\n";
        let n = parse_nse_indices(idx, nov8)["Nifty 50"];
        assert!((n.prev - 25030.35).abs() < 1e-6);

        let bse = "TradDt,BizDt,Sgmt,Src,FinInstrmTp,FinInstrmId,ISIN,TckrSymb,SctySrs,XpryDt,FininstrmActlXpryDt,StrkPric,OptnTp,FinInstrmNm,OpnPric,HghPric,LwPric,ClsPric,LastPric,PrvsClsgPric,UndrlygPric,SttlmPric,OpnIntrst,ChngInOpnIntrst,TtlTradgVol,TtlTrfVal,TtlNbOfTxsExctd,SsnId,NewBrdLotQty,Rmks,Rsvd1,Rsvd2,Rsvd3,Rsvd4\n\
                   2026-11-08,2026-11-08,CM,BSE,STK,500002,INE117A01022,ABB,A,,,,,\"ABB INDIA, LIMITED\",6798.00,6903.05,6785.55,6875.00,6875.00,6774.00,,6859.69,,,4308,29461900.00,861,F1,1,,,,,\n";
        let abb = parse_bse(bse, nov8)["BSE:ABB"];
        assert_eq!((abb.prev, abb.close, abb.volume), (6774.0, 6875.0, 4308.0));
    }

    fn sbin() -> Symbol {
        opentern_core::starter_watchlist()
            .into_iter()
            .find(|s| s.exchange() == "NSE" && s.kind == opentern_core::AssetKind::Stock)
            .map(|mut s| {
                s.id = "NSE:SBIN".into();
                s
            })
            .unwrap()
    }

    fn day_ms(y: i16, m: i8, d: i8) -> i64 {
        midnight_ms(jiff::civil::date(y, m, d)).unwrap()
    }

    fn bar(t: i64, c: f64) -> Bar {
        Bar {
            time: t,
            open: c,
            high: c,
            low: c,
            close: c,
            volume: 1.0,
        }
    }

    /// Muhurat trading on Sunday 8 Nov 2026, as NSE reported it.
    fn seed_muhurat() {
        let memo = MEMO.get_or_init(|| Mutex::new(HashMap::new()));
        memo.lock().unwrap().insert(
            (Source::NseStocks, jiff::civil::date(2026, 11, 8)),
            Arc::new(parse_nse_full(FULL, jiff::civil::date(2026, 11, 8))),
        );
    }

    #[test]
    fn a_missing_muhurat_session_is_filled_from_nse() {
        seed_muhurat();
        let mut bars = vec![bar(day_ms(2026, 11, 6), 1018.5), bar(day_ms(2026, 11, 9), 1042.0)];
        fill_daily(&sbin(), &mut bars);
        assert_eq!(bars.len(), 3);
        let m = bars[1];
        assert_eq!(m.time, day_ms(2026, 11, 8));
        assert_eq!(
            (m.open, m.high, m.low, m.close, m.volume),
            (1023.2, 1025.0, 1015.1, 1016.0, 1950983.0)
        );
        // Yahoo's single-precision copy of the same close: still exact.
        let mut f32ish = vec![
            bar(day_ms(2026, 11, 6), 1018.5f32 as f64 + 1e-5),
            bar(day_ms(2026, 11, 9), 1042.0),
        ];
        fill_daily(&sbin(), &mut f32ish);
        assert_eq!(f32ish[1].close, 1016.0);
        // Already there: nothing changes.
        fill_daily(&sbin(), &mut bars);
        assert_eq!(bars.len(), 3);
    }

    #[test]
    fn a_filled_session_follows_a_later_split() {
        seed_muhurat();
        // Bars adjusted for a 1:2 split after the session: everything halves.
        let mut bars = vec![bar(day_ms(2026, 11, 6), 509.25), bar(day_ms(2026, 11, 9), 521.0)];
        fill_daily(&sbin(), &mut bars);
        assert!((bars[1].close - 508.0).abs() < 1e-9);
        assert!((bars[1].volume - 3_901_966.0).abs() < 1e-6);
    }

    #[test]
    fn spark_closes_get_the_session_too() {
        seed_muhurat();
        // Yahoo's spark stamps daily closes at the 09:15 IST open.
        let open = |y, m, d| day_ms(y, m, d) / 1000 + 13_500;
        let mut times = vec![open(2026, 11, 6), open(2026, 11, 9)];
        let mut closes = vec![1018.5, 1042.0];
        fill_closes(&sbin(), &mut times, &mut closes);
        assert_eq!(closes, vec![1018.5, 1016.0, 1042.0]);
        assert_eq!(times[1], day_ms(2026, 11, 8) / 1000);
    }
}

//! Prices checked against a second, independent source, and old prices
//! that must never change. Not run by default (real services); run before
//! every release:
//!   cargo test -p opentern-data --test crosscheck -- --ignored --nocapture --test-threads=1
//!
//! A failure here means a source changed what it sends, or our parsing of it
//! broke: find out which before releasing. NSE's and BSE's own files are
//! never fetched (their terms); Indian prices are checked NSE listing
//! against BSE listing instead, and by hand against NSE when in doubt.

use opentern_core::{AssetKind, Bar, Symbol, Timeframe};
use opentern_data::provider::Provider;
use std::collections::BTreeMap;

const DAY: i64 = 86_400_000;

fn sym(id: &str, kind: AssetKind, tz: &str) -> Symbol {
    Symbol {
        timezone: tz.into(),
        kind,
        ..Symbol::bare(id)
    }
}

/// The newest `n` daily bars (some sources send more than asked).
fn daily(p: &dyn Provider, s: &Symbol, n: usize) -> Vec<Bar> {
    let mut bars = p
        .latest(s, Timeframe::parse("1D").unwrap(), n)
        .unwrap_or_else(|e| panic!("{}: {e}", s.id));
    bars.drain(..bars.len().saturating_sub(n));
    bars
}

/// Closed days only (today's bar may still be forming), keyed by day.
fn closed(bars: &[Bar]) -> BTreeMap<i64, Bar> {
    let today = jiff::Timestamp::now().as_millisecond() / DAY * DAY;
    bars.iter().filter(|b| b.time < today).map(|b| (b.time, *b)).collect()
}

fn rel(a: f64, b: f64) -> f64 {
    (a / b - 1.0).abs()
}

/// Nasdaq's own daily closes (its site's history API: about the last two
/// years), keyed by UTC midnight of the session's date.
fn nasdaq_closes(ticker: &str, class: &str) -> BTreeMap<i64, f64> {
    let to = jiff::Zoned::now().date();
    let from = to.checked_sub(jiff::Span::new().days(45)).unwrap();
    let url = format!(
        "https://api.nasdaq.com/api/quote/{ticker}/historical?assetclass={class}&fromdate={from}&todate={to}&limit=60"
    );
    let text = ureq::get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .header("Accept", "application/json")
        .call()
        .unwrap_or_else(|e| panic!("Nasdaq {ticker}: {e}"))
        .body_mut()
        .read_to_string()
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let rows = v["data"]["tradesTable"]["rows"].as_array().cloned().unwrap_or_default();
    rows.iter()
        .filter_map(|r| {
            let (m, rest) = r["date"].as_str()?.split_once('/')?;
            let (d, y) = rest.split_once('/')?;
            let date = jiff::civil::date(y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
            let t = date
                .to_zoned(jiff::tz::TimeZone::UTC)
                .ok()?
                .timestamp()
                .as_millisecond();
            let close: f64 = r["close"]
                .as_str()?
                .trim_start_matches('$')
                .replace(',', "")
                .parse()
                .ok()?;
            Some((t, close))
        })
        .collect()
}

/// US daily closes from Yahoo (what the charts use) equal Nasdaq's own, to
/// the cent, on the same set of sessions.
#[test]
#[ignore]
fn us_closes_match_nasdaq() {
    let y = opentern_data::yahoo::Yahoo::default();
    for (id, class) in [
        ("NASDAQ:AAPL", "stocks"),
        ("NASDAQ:NVDA", "stocks"),
        ("NYSE:JPM", "stocks"),
        ("AMEX:SPY", "etf"),
    ] {
        let s = sym(id, AssetKind::Stock, "America/New_York");
        let ours = closed(&daily(&y, &s, 60));
        let theirs = nasdaq_closes(s.ticker(), class);
        assert!(theirs.len() >= 20, "{id}: Nasdaq gave {} days", theirs.len());
        let from = *theirs.keys().next().unwrap();
        let ours: BTreeMap<_, _> = ours.range(from..).collect();
        let missing: Vec<_> = theirs.keys().filter(|t| !ours.contains_key(t)).collect();
        let extra: Vec<_> = ours.keys().filter(|t| !theirs.contains_key(t)).collect();
        assert!(missing.is_empty(), "{id}: days Nasdaq has and we don't: {missing:?}");
        assert!(extra.is_empty(), "{id}: days we have and Nasdaq doesn't: {extra:?}");
        for (t, c) in &theirs {
            let b = ours[t];
            assert!((b.close - c).abs() < 0.006, "{id} {t}: close {} vs Nasdaq {c}", b.close);
            assert!(b.low <= b.close && b.close <= b.high && b.low <= b.open && b.open <= b.high);
        }
        println!("{id}: {} sessions match Nasdaq", theirs.len());
    }
}

/// Binance's daily bars (USDT) against Coinbase's (USD): the same UTC days,
/// closes within 1% (the USDT peg and exchange spreads), mostly far closer.
#[test]
#[ignore]
fn crypto_closes_agree_across_exchanges() {
    let b = opentern_data::binance::Binance::default();
    let c = opentern_data::coinbase::Coinbase::default();
    for (ours, other) in [
        ("BINANCE:BTCUSDT", "COINBASE:BTCUSD"),
        ("BINANCE:ETHUSDT", "COINBASE:ETHUSD"),
    ] {
        let x = closed(&daily(&b, &sym(ours, AssetKind::Crypto, "UTC"), 40));
        let y = closed(&daily(&c, &sym(other, AssetKind::Crypto, "UTC"), 40));
        let common: Vec<_> = x.keys().filter(|t| y.contains_key(t)).collect();
        assert!(common.len() >= 25, "{ours}: only {} common days", common.len());
        let mut diffs: Vec<f64> = common.iter().map(|t| rel(x[t].close, y[t].close)).collect();
        diffs.sort_by(f64::total_cmp);
        let (median, worst) = (diffs[diffs.len() / 2], diffs[diffs.len() - 1]);
        assert!(
            worst < 0.01,
            "{ours} vs {other}: a close differs by {:.2}%",
            worst * 100.0
        );
        assert!(median < 0.003, "{ours} vs {other}: median gap {:.3}%", median * 100.0);
        println!(
            "{ours} vs {other}: {} days, median {:.3}%, worst {:.3}%",
            common.len(),
            median * 100.0,
            worst * 100.0
        );
    }
}

/// The same company on NSE and on BSE (both from Yahoo): the same trading
/// days, closes within 0.5%. Yahoo's history for some BSE tickers is a
/// single bar (a known Yahoo problem): those are skipped, not failed.
#[test]
#[ignore]
fn indian_closes_agree_across_nse_and_bse() {
    let y = opentern_data::yahoo::Yahoo::default();
    let mut checked = 0;
    for t in ["TCS", "INFY", "HDFCBANK", "ITC", "LT"] {
        let n = closed(&daily(
            &y,
            &sym(&format!("NSE:{t}"), AssetKind::Stock, "Asia/Kolkata"),
            40,
        ));
        let b = closed(&daily(
            &y,
            &sym(&format!("BSE:{t}"), AssetKind::Stock, "Asia/Kolkata"),
            40,
        ));
        if b.len() < 15 {
            println!("{t}: Yahoo has {} BSE days, skipped", b.len());
            continue;
        }
        let from = *n.keys().next().unwrap().max(b.keys().next().unwrap());
        let nd: Vec<_> = n.range(from..).map(|(t, _)| *t).collect();
        let bd: Vec<_> = b.range(from..).map(|(t, _)| *t).collect();
        assert_eq!(nd, bd, "{t}: NSE and BSE trading days differ");
        for d in &nd {
            let gap = rel(n[d].close, b[d].close);
            assert!(gap < 0.005, "{t} {d}: NSE {} vs BSE {}", n[d].close, b[d].close);
        }
        println!("{t}: {} days agree", nd.len());
        checked += 1;
    }
    // Most BSE listings have no history on Yahoo (checked 12 Nov 2026:
    // INFY.BO and ITC.BO are one bar, TCS.BO is whole).
    assert!(checked >= 1, "no BSE history to compare");
}

/// Old prices never change (only a split may rescale a stock's). Each was
/// read from the source when the test was written; a change means the
/// source rewrote history or our parsing moved.
#[test]
#[ignore]
fn old_prices_stay_put() {
    let at = |s: &str| {
        s.parse::<jiff::civil::Date>()
            .unwrap()
            .to_zoned(jiff::tz::TimeZone::UTC)
            .unwrap()
            .timestamp()
            .as_millisecond()
    };
    let b = opentern_data::binance::Binance::default();
    let y = opentern_data::yahoo::Yahoo::default();
    let d = Timeframe::parse("1D").unwrap();
    let cases: [(&dyn Provider, Symbol, &str, [f64; 4]); 4] = [
        (
            &b,
            sym("BINANCE:BTCUSDT", AssetKind::Crypto, "UTC"),
            "2024-01-01",
            FROZEN[0],
        ),
        (
            &b,
            sym("BINANCE:ETHUSDT", AssetKind::Crypto, "UTC"),
            "2024-03-12",
            FROZEN[1],
        ),
        (
            &y,
            sym("NASDAQ:MSFT", AssetKind::Stock, "America/New_York"),
            "2024-01-02",
            FROZEN[2],
        ),
        (
            &y,
            sym("NSE:INFY", AssetKind::Stock, "Asia/Kolkata"),
            "2024-01-02",
            FROZEN[3],
        ),
    ];
    let mut moved = Vec::new();
    for (p, s, day, want) in cases {
        let t = at(day);
        // Yahoo sends its whole history at once and doesn't page back.
        let find = |bars: Result<Vec<Bar>, _>| bars.ok().and_then(|v| v.into_iter().find(|b| b.time == t));
        let got = find(p.before(&s, d, t + DAY, 3))
            .or_else(|| find(p.latest(&s, d, 0)))
            .unwrap_or_else(|| panic!("{} has no bar on {day}", s.id));
        let have = [got.open, got.high, got.low, got.close];
        println!("{} {day}: {have:?}", s.id);
        if have.iter().zip(want).any(|(h, w)| rel(*h, w) >= 1e-6) {
            moved.push(format!("{} {day}: {have:?}, was {want:?}", s.id));
        }
    }
    assert!(moved.is_empty(), "old prices changed:\n{}", moved.join("\n"));
}

/// Open, high, low, close for each case in `old_prices_stay_put`, as read
/// on 12 Nov 2026. BTC's and MSFT's closes match public records; INFY's is
/// Yahoo's (not yet checked against NSE's file by hand).
const FROZEN: [[f64; 4]; 4] = [
    [42283.58, 44184.1, 42180.77, 44179.55],
    [4064.8, 4093.92, 3828.98, 3979.96],
    [373.86, 375.90, 366.77, 370.87],
    [1546.0, 1549.15, 1523.0, 1534.40],
];

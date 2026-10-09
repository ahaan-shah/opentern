//! Talks to the real services. Not run by default (CI has no business
//! depending on Binance or Yahoo being up):
//!   cargo test -p opentern-data --test live -- --ignored --nocapture

use opentern_core::{Symbol, Timeframe};
use opentern_data::{Event, Hub, Request, Status};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn open(symbol: Symbol, tf: &str, want_tick: bool) {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::start(dir.path().to_path_buf(), false, Arc::new(|| {}));
    let id = symbol.id.clone();
    hub.send(Request::Open {
        chart: 1,
        symbol,
        tf: Timeframe::parse(tf).unwrap(),
    });
    let deadline = Instant::now() + Duration::from_secs(40);
    let (mut bars_n, mut ticked) = (0, false);
    while Instant::now() < deadline && (bars_n == 0 || (want_tick && !ticked)) {
        for e in hub.poll() {
            match e {
                Event::Bars { bars, .. } => {
                    assert!(bars.windows(2).all(|w| w[0].time < w[1].time));
                    bars_n = bars.len();
                }
                Event::Tick { bar, .. } => {
                    ticked = true;
                    println!("{id} {tf} tick: {bar:?}");
                }
                Event::Status {
                    status: Status::Error(e),
                    ..
                } => panic!("{id}: {e}"),
                Event::Status { status, .. } => println!("{id} {tf}: {status:?}"),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("{id} {tf}: {bars_n} bars, ticked: {ticked}");
    assert!(bars_n > 0, "{id} {tf}: no bars");
    assert!(!want_tick || ticked, "{id} {tf}: no live tick");
}

#[test]
#[ignore]
fn binance_history_and_stream() {
    let btc = opentern_core::starter_watchlist().remove(0);
    open(btc.clone(), "1m", true);
    open(btc, "45m", true);
}

#[test]
#[ignore]
fn yahoo_history() {
    let w = opentern_core::starter_watchlist();
    for id in ["NASDAQ:AAPL", "NSE:RELIANCE", "NSE:NIFTY", "FX:EURUSD"] {
        let s = w.iter().find(|s| s.id == id).unwrap().clone();
        open(s, "1D", false);
    }
    let aapl = w.iter().find(|s| s.id == "NASDAQ:AAPL").unwrap().clone();
    open(aapl, "15m", false);
}

#[test]
#[ignore]
fn search_and_quotes() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::start(dir.path().to_path_buf(), false, Arc::new(|| {}));
    hub.send(Request::Search {
        seq: 1,
        query: "eth".into(),
    });
    hub.send(Request::Quotes {
        symbols: opentern_core::starter_watchlist(),
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let (mut found, mut quotes) = (false, std::collections::HashSet::new());
    while Instant::now() < deadline && !(found && quotes.len() >= 8) {
        for e in hub.poll() {
            match e {
                Event::Search { results, .. } => {
                    println!(
                        "search: {:?}",
                        results.iter().take(5).map(|s| &s.id).collect::<Vec<_>>()
                    );
                    assert_eq!(results[0].id, "BINANCE:ETHUSDT");
                    found = true;
                }
                Event::Quote { id, last, prev } if quotes.insert(id.clone()) => {
                    println!("quote {id}: {last} (prev {prev})");
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(found);
    assert!(quotes.len() >= 8, "only {} quotes", quotes.len());
}

#[test]
#[ignore]
fn binance_loads_older_history() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::start(dir.path().to_path_buf(), false, Arc::new(|| {}));
    let btc = opentern_core::starter_watchlist().remove(0);
    hub.send(Request::Open {
        chart: 1,
        symbol: btc,
        tf: Timeframe::parse("1h").unwrap(),
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let (mut first_len, mut asked, mut grown) = (0, false, false);
    while Instant::now() < deadline && !grown {
        for e in hub.poll() {
            if let Event::Bars { bars, .. } = e {
                if first_len == 0 {
                    first_len = bars.len();
                } else if bars.len() > first_len {
                    println!("history grew from {first_len} to {} bars", bars.len());
                    grown = true;
                }
            }
        }
        if first_len > 0 && !asked {
            hub.send(Request::More { chart: 1 });
            asked = true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(grown, "older history never arrived");
}

#[test]
#[ignore]
fn formula_history_and_stream() {
    let known = opentern_core::starter_watchlist();
    let s = opentern_data::expr::symbol("BTCUSDT/ETHUSDT", &known).unwrap();
    open(s, "1m", true);
    let s = opentern_data::expr::symbol("AAPL/SPY", &known).unwrap();
    open(s, "1D", false);
}

fn crypto(id: &str) -> Symbol {
    Symbol {
        id: id.into(),
        description: String::new(),
        kind: opentern_core::AssetKind::Crypto,
        currency: "USD".into(),
        tick_size: 0.01,
        timezone: "UTC".into(),
    }
}

/// History through the providers directly: recent bars, paging back, and
/// bars since a time, all sorted with no duplicates.
fn provider_checks(p: &dyn opentern_data::provider::Provider, s: &Symbol) {
    let tf = Timeframe::parse("1h").unwrap();
    let latest = p.latest(s, tf, 400).unwrap_or_else(|e| panic!("{}: {e}", s.id));
    assert!(latest.len() >= 300, "{}: {} bars", s.id, latest.len());
    assert!(latest.windows(2).all(|w| w[0].time < w[1].time), "{}: unsorted", s.id);
    assert!(
        latest.iter().all(|b| b.time % 3_600_000 == 0),
        "{}: off the hour grid",
        s.id
    );
    let first = latest[0].time;
    let older = p.before(s, tf, first, 50).unwrap();
    assert!(older.iter().all(|b| b.time < first), "{}: before() overlaps", s.id);
    let since = p.since(s, tf, latest[latest.len() - 10].time).unwrap();
    assert!((9..=12).contains(&since.len()), "{}: since gave {}", s.id, since.len());
    let daily = p.latest(s, Timeframe::parse("1D").unwrap(), 30).unwrap();
    assert!(
        daily.iter().all(|b| b.time % 86_400_000 == 0),
        "{}: days not at UTC midnight",
        s.id
    );
    println!(
        "{}: {} hourly, {} older, {} since, {} daily",
        s.id,
        latest.len(),
        older.len(),
        since.len(),
        daily.len()
    );
}

#[test]
#[ignore]
fn crypto_exchanges_history() {
    use opentern_data::provider::Provider;
    use opentern_data::{bybit::Bybit, coinbase::Coinbase, kraken::Kraken, okx::Okx};
    provider_checks(&Bybit::default(), &crypto("BYBIT:BTCUSDT"));
    provider_checks(&Okx::default(), &crypto("OKX:BTCUSDT"));
    provider_checks(&Coinbase::default(), &crypto("COINBASE:BTCUSD"));
    // Kraken keeps only its latest 720 bars; before() is empty by design.
    let k = Kraken::default();
    let s = crypto("KRAKEN:BTCUSD");
    let tf = Timeframe::parse("1h").unwrap();
    let latest = k.latest(&s, tf, 400).unwrap();
    assert!(latest.len() >= 300 && latest.windows(2).all(|w| w[0].time < w[1].time));
    println!("KRAKEN: {} hourly", latest.len());
}

#[test]
#[ignore]
fn crypto_exchanges_stream_through_the_hub() {
    for id in ["BYBIT:BTCUSDT", "OKX:BTCUSDT", "COINBASE:BTCUSD", "KRAKEN:BTCUSD"] {
        open(crypto(id), "1m", true);
    }
}

#[test]
#[ignore]
fn crypto_symbol_lists() {
    use opentern_data::{bybit::Bybit, coinbase::Coinbase, kraken::Kraken, okx::Okx};
    for (name, list) in [
        ("bybit", Bybit::default().symbols()),
        ("okx", Okx::default().symbols()),
        ("coinbase", Coinbase::default().symbols()),
        ("kraken", Kraken::default().symbols()),
    ] {
        let list = list.unwrap_or_else(|e| panic!("{name}: {e}"));
        println!("{name}: {} symbols", list.len());
        assert!(list.len() > 50, "{name}");
        assert!(list.iter().any(|s| s.ticker().starts_with("BTC")), "{name}: no BTC");
    }
}

#[test]
#[ignore]
fn yahoo_dividends_and_splits_reach_the_chart() {
    let aapl = opentern_core::starter_watchlist()
        .into_iter()
        .find(|s| s.id == "NASDAQ:AAPL")
        .unwrap();
    let c = opentern_data::yahoo::Yahoo::default().corporate(&aapl).unwrap();
    println!("AAPL: {} dividends, splits {:?}", c.dividends.len(), c.splits);
    assert!(c.dividends.len() > 40);
    // 4-for-1 on 2020-08-31.
    assert!(
        c.splits.iter().any(|(d, r)| *d == 1_598_832_000_000 && *r == 4.0),
        "{:?}",
        c.splits
    );
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::start(dir.path().to_path_buf(), false, Arc::new(|| {}));
    hub.send(Request::Open {
        chart: 1,
        symbol: aapl,
        tf: Timeframe::parse("1D").unwrap(),
    });
    let deadline = Instant::now() + Duration::from_secs(40);
    let mut got = false;
    while Instant::now() < deadline && !got {
        got = hub.poll().into_iter().any(|e| matches!(e, Event::Corporate { .. }));
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(got, "no corporate event");
}

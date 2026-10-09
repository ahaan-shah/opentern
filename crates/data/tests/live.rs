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

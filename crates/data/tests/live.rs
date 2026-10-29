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
    let symbols = opentern_core::starter_watchlist();
    // Binance quotes arrive over their own websocket: each must show up.
    let crypto: Vec<String> = symbols
        .iter()
        .filter(|s| s.id.starts_with("BINANCE:"))
        .map(|s| s.id.clone())
        .collect();
    assert!(!crypto.is_empty());
    hub.send(Request::Quotes { symbols });
    let deadline = Instant::now() + Duration::from_secs(60);
    let (mut found, mut quotes) = (false, std::collections::HashSet::new());
    let done = |found: bool, quotes: &std::collections::HashSet<String>| {
        found && quotes.len() >= 8 && crypto.iter().all(|c| quotes.contains(c))
    };
    while Instant::now() < deadline && !done(found, &quotes) {
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
    for c in &crypto {
        assert!(quotes.contains(c), "no quote for {c}");
    }
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
        min_qty: 0.0,
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
    use opentern_data::{binance::Binance, bybit::Bybit, coinbase::Coinbase, kraken::Kraken, okx::Okx};
    for (name, list) in [
        ("binance", Binance::default().symbols()),
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

#[test]
#[ignore]
fn order_books_from_crypto_exchanges() {
    for id in [
        "BINANCE:BTCUSDT",
        "BYBIT:BTCUSDT",
        "OKX:BTCUSDT",
        "COINBASE:BTCUSD",
        "KRAKEN:BTCUSD",
    ] {
        let sym = opentern_core::Symbol::bare(id);
        match opentern_data::depth::book(&sym, 0.0, false) {
            Ok(b) => {
                assert!(!b.bids.is_empty() && !b.asks.is_empty(), "{id}: empty book");
                assert!(b.bids[0].0 < b.asks[0].0, "{id}: crossed book");
                println!(
                    "{id}: {} bids, {} asks, spread {}",
                    b.bids.len(),
                    b.asks.len(),
                    b.asks[0].0 - b.bids[0].0
                );
            }
            Err(e) => println!("{id}: {e}"),
        }
    }
}

/// India's hotlist: the NIFTY 50 from Yahoo's spark quotes, in three batches.
#[test]
#[ignore]
fn india_hotlist() {
    use opentern_data::feeds::{HotKind, HotMarket, hotlist};
    for kind in [HotKind::Gainers, HotKind::Losers] {
        let rows = hotlist(HotMarket::India, kind, false).expect("India hotlist");
        assert_eq!(rows.len(), 30, "the top 30 of the NIFTY 50");
        let top = &rows[0];
        println!("{kind:?}: {} {:.2} {:+.2}%", top.symbol.id, top.last, top.change_pct);
    }
}

/// Crypto hotlists: Binance's 24-hour tickers.
#[test]
#[ignore]
fn crypto_hotlist() {
    use opentern_data::feeds::{HotKind, HotMarket, hotlist};
    for kind in [HotKind::Gainers, HotKind::Losers, HotKind::Volume] {
        let rows = hotlist(HotMarket::Crypto, kind, false).expect("crypto hotlist");
        assert!(!rows.is_empty(), "{kind:?}");
        let top = &rows[0];
        println!("{kind:?}: {} {} {:+.2}%", top.symbol.id, top.last, top.change_pct);
    }
}

/// Headlines from every source for a coin, Indian and US stocks, an index
/// and a currency pair.
#[test]
#[ignore]
fn news_for_each_market() {
    use opentern_core::AssetKind;
    let s = |id: &str, desc: &str, kind| Symbol {
        description: desc.into(),
        kind,
        ..Symbol::bare(id)
    };
    for sym in [
        s("BINANCE:BTCUSDT", "Bitcoin / TetherUS", AssetKind::Crypto),
        s("BINANCE:SOLUSDT", "SOL / USDT", AssetKind::Crypto),
        s("NSE:INFY", "Infosys Limited", AssetKind::Stock),
        s("NSE:RELIANCE", "Reliance Industries", AssetKind::Stock),
        s("NSE:NIFTY", "Nifty 50 Index", AssetKind::Index),
        s("NASDAQ:AAPL", "Apple Inc.", AssetKind::Stock),
        s("FX:EURUSD", "EUR/USD", AssetKind::Forex),
    ] {
        let items = opentern_data::feeds::news(&sym, false).expect("news");
        let mut by: std::collections::BTreeMap<&str, usize> = Default::default();
        for n in &items {
            *by.entry(n.publisher.as_str()).or_default() += 1;
        }
        println!("{} {} headlines from {} outlets", sym.id, items.len(), by.len());
        for n in items.iter().take(4) {
            println!("   {} | {}", n.publisher, n.title);
        }
        assert!(items.len() >= 5, "{}", sym.id);
    }
}

/// Free float and fundamentals for a few NSE stocks from Yahoo, as the
/// custom indices use them.
#[test]
#[ignore]
fn nse_fundamentals() {
    for t in ["RELIANCE", "MARUTI", "HDFCBANK", "M&M", "BAJAJ-AUTO", "TATAMOTORS"] {
        let s = opentern_core::Symbol {
            timezone: "Asia/Kolkata".into(),
            ..opentern_core::Symbol::bare(&format!("NSE:{t}"))
        };
        match opentern_data::fundamentals::fetch(&s, false) {
            Ok(f) => println!(
                "{t:12} shares {:>14.0} float {:>14.0} ({:>5.1}%)  cap {:>16.0}  P/E {:>6.1}  EV/EBITDA {:>6.1}  gross {:>5.3} ebitda {:>5.3}",
                f.shares,
                f.float_shares,
                f.float_shares / f.shares * 100.0,
                f.market_cap,
                f.pe,
                f.ev_ebitda,
                f.gross_margin,
                f.ebitda_margin
            ),
            Err(e) => println!("{t:12} error: {e}"),
        }
        std::thread::sleep(std::time::Duration::from_millis(800));
    }
}

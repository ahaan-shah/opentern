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
        ext: false,
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

/// Weeks start on Monday and months on the 1st, and the current one runs to
/// the latest day (Yahoo's own end at yesterday, with today as a bar apart).
#[test]
#[ignore]
fn yahoo_weeks_and_months_reach_today() {
    use opentern_data::provider::Provider;
    let y = opentern_data::yahoo::Yahoo::default();
    let w = opentern_core::starter_watchlist();
    let tf = |s: &str| opentern_core::Timeframe::parse(s).unwrap();
    for id in ["NASDAQ:AAPL", "FX:EURUSD", "NSE:RELIANCE"] {
        let s = w.iter().find(|s| s.id == id).unwrap().clone();
        let days = y.latest(&s, tf("1D"), 0).unwrap();
        let today = days.last().unwrap();
        for (t, check) in [("1W", "%a Mon"), ("1M", "%d 01")] {
            let bars = y.latest(&s, tf(t), 0).unwrap();
            let (fmt, want) = check.split_once(' ').unwrap();
            for b in &bars {
                let d = jiff::Timestamp::from_millisecond(b.time)
                    .unwrap()
                    .to_zoned(jiff::tz::TimeZone::UTC);
                assert_eq!(d.strftime(fmt).to_string(), want, "{id} {t} bar at {d}");
            }
            let last = bars.last().unwrap();
            assert_eq!(last.close, today.close, "{id} {t} ends at the latest day");
            assert!(last.high >= today.high && last.low <= today.low, "{id} {t}");
            println!("{id} {t}: {} bars, last {} close {}", bars.len(), last.time, last.close);
        }
    }
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
        ext: false,
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
        ext: false,
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
    for t in ["RELIANCE", "MARUTI", "HDFCBANK", "M&M", "INFY", "SWIGGY", "WIPRO"] {
        let s = opentern_core::Symbol {
            timezone: "Asia/Kolkata".into(),
            ..opentern_core::Symbol::bare(&format!("NSE:{t}"))
        };
        match opentern_data::fundamentals::fetch(&s, false) {
            Ok(f) => println!(
                "{t:12} shares {:>14.0} float {:>14.0} ({:>5.1}%)  cap {:>16.0}  P/E {:>6.1} (cap/net {:>6.1})  EV/EBITDA {:>6.1} (ev/ebitda {:>6.1})  revenue {:>16.0}  gross {:>5.3} ebitda {:>5.3}",
                f.shares,
                f.float_shares,
                f.float_shares / f.shares * 100.0,
                f.market_cap,
                f.pe,
                f.market_cap / f.net_income,
                f.ev_ebitda,
                f.enterprise_value / f.ebitda,
                f.revenue,
                f.gross_margin,
                f.ebitda_margin
            ),
            Err(e) => println!("{t:12} error: {e}"),
        }
        std::thread::sleep(std::time::Duration::from_millis(800));
    }
}

#[test]
#[ignore]
fn earnings_for_a_us_stock() {
    let s = opentern_core::starter_watchlist()
        .into_iter()
        .find(|s| s.id == "NASDAQ:AAPL")
        .expect("AAPL");
    let e = opentern_data::earnings::fetch(&s).expect("earnings");
    let reported = e.iter().filter(|x| x.eps.is_some()).count();
    eprintln!(
        "{} reports, {reported} with EPS; latest: {:?}",
        e.len(),
        e.iter().rev().take(3).collect::<Vec<_>>()
    );
    assert!(reported >= 4);
    // Nasdaq's figures cover the last year, past the calendar's gap.
    let year_ago = jiff::Timestamp::now().as_millisecond() - 400 * 86_400_000;
    assert!(e.iter().any(|x| x.eps.is_some() && x.time > year_ago));
}

#[test]
#[ignore]
fn overview_boards() {
    use opentern_data::overview::{Market, Period, Sizing, board};
    let dir = tempfile::tempdir().unwrap();
    for m in Market::ALL {
        let t = std::time::Instant::now();
        match board(m, dir.path(), false) {
            Ok(b) => {
                // A delisted pair's old closes once made up +331% for XMR.
                if m == Market::Crypto {
                    if let Some(x) = b.members.iter().find(|x| x.name == "XMR") {
                        println!("  XMR via {}: 1D {:+.2}%", x.symbol.id, x.change(Period::Day));
                    }
                    let wild: Vec<_> = b
                        .members
                        .iter()
                        .filter(|x| x.change(Period::Day).abs() > 60.0)
                        .map(|x| format!("{} {:+.1}%", x.name, x.change(Period::Day)))
                        .collect();
                    assert!(wild.is_empty(), "implausible 1D moves: {wild:?}");
                }
                let mut top: Vec<_> = b.members.iter().collect();
                top.sort_by(|a, c| c.cap.total_cmp(&a.cap));
                let with = |p: Period| b.members.iter().filter(|x| x.change(p).is_finite()).count();
                println!(
                    "{m:?}: {} members, {:?} from {}, {:.1}s; 1D {} 1W {} 3M {} YTD {} 1Y {}; sectors {}",
                    b.members.len(),
                    b.sizing,
                    b.source,
                    t.elapsed().as_secs_f32(),
                    with(Period::Day),
                    with(Period::Week),
                    with(Period::Quarter),
                    with(Period::Ytd),
                    with(Period::Year),
                    b.members
                        .iter()
                        .map(|x| x.sector.as_str())
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                );
                for x in top.iter().take(6) {
                    println!(
                        "  {} {} {} cap {:.0}B price {} 1D {:+.2}% 1W {:+.2}% 1M {:+.2}% 3M {:+.2}% YTD {:+.2}% 1Y {:+.2}% closes {}",
                        x.symbol.id,
                        x.name,
                        x.sector,
                        x.cap / 1e9,
                        x.price,
                        x.change(Period::Day),
                        x.change(Period::Week),
                        x.change(Period::Month),
                        x.change(Period::Quarter),
                        x.change(Period::Ytd),
                        x.change(Period::Year),
                        x.closes.len()
                    );
                }
                assert!(b.members.len() >= 100);
                if m != Market::India {
                    assert_ne!(b.sizing, Sizing::Equal);
                }
                // The light refresh between boards.
                let keys: Vec<String> = b
                    .members
                    .iter()
                    .map(|x| x.key.clone())
                    .filter(|k| !k.is_empty())
                    .collect();
                let t = std::time::Instant::now();
                let prices = opentern_data::overview::prices(m, &keys, false).expect("prices");
                println!(
                    "  prices: {} of {} in {:.1}s",
                    prices.len(),
                    keys.len(),
                    t.elapsed().as_secs_f32()
                );
                for x in top.iter().take(3) {
                    if let Some(q) = prices.get(&x.key) {
                        println!("  {} {} 1D {:+.2}% ext {:?}", x.symbol.id, q.price, q.day, q.ext);
                        assert!((q.day - x.change(Period::Day)).abs() < 1.0, "{} 1D moved", x.symbol.id);
                    }
                }
                assert!(prices.len() * 10 >= keys.len() * 9);
                if m == Market::Crypto {
                    let pairs = vec!["BTCUSDT".to_string(), "ETHUSDT".to_string()];
                    let p = opentern_data::overview::prices(m, &pairs, false).expect("coin prices");
                    println!("  coins: {:?}", p);
                    assert_eq!(p.len(), 2);
                }
            }
            Err(e) => println!("{m:?}: {e}"),
        }
    }
    let new = opentern_data::binance::Binance::default()
        .symbol("BTCUSDT")
        .expect("BTCUSDT");
    assert_eq!(new.id, "BINANCE:BTCUSDT");
    assert!(
        opentern_data::binance::Binance::default()
            .symbol("NOSUCHCOINUSDT")
            .is_none()
    );
}

/// Indian quotes take the previous close from Yahoo's day change, which
/// counts sessions its bars skip. On 9-10 Nov 2026 SBIN's was 1016.00 (the
/// Muhurat close, NSE's own figure), not the 1018.50 of 6 Nov's bar.
#[test]
#[ignore]
fn indian_quotes_count_skipped_sessions() {
    use opentern_data::yahoo::Yahoo;
    let sbin = Symbol {
        id: "NSE:SBIN".into(),
        description: "State Bank of India".into(),
        kind: opentern_core::AssetKind::Stock,
        currency: "INR".into(),
        tick_size: 0.05,
        min_qty: 0.0,
        timezone: "Asia/Kolkata".into(),
    };
    let (last, prev) = Yahoo::default().quote(&sbin).unwrap();
    println!("SBIN {last} (previous close {prev})");
    assert!(last > 0.0 && prev > 0.0);
}

/// Yahoo quotes come in batches (spark, twenty a request), shared with the
/// market overview, equal Yahoo's own per-symbol quote, and come again
/// every 30 s while the market trades. Run while US stocks trade.
#[test]
#[ignore]
fn yahoo_quotes_are_batched_shared_and_live() {
    use opentern_data::overview;
    use std::collections::HashMap;
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::start(dir.path().to_path_buf(), false, Arc::new(|| {}));
    let us = [
        "AAPL", "MSFT", "NVDA", "AMZN", "GOOGL", "META", "TSLA", "AVGO", "JPM", "V", "WMT", "XOM", "UNH", "MA", "COST",
        "HD", "PG", "NFLX", "JNJ", "BAC", "ORCL", "KO",
    ];
    let mut symbols: Vec<Symbol> = us
        .iter()
        .map(|t| Symbol {
            kind: opentern_core::AssetKind::Stock,
            timezone: "America/New_York".into(),
            ..Symbol::bare(&format!("NASDAQ:{t}"))
        })
        .collect();
    symbols.extend(
        opentern_core::starter_watchlist()
            .into_iter()
            .filter(|s| !s.id.starts_with("BINANCE:")),
    );
    symbols.push(Symbol::bare("NSE:RELIANCE"));
    let formula = "(NASDAQ:AAPL + NASDAQ:MSFT)/2";
    symbols.push(Symbol::bare(formula));
    let mut ids: Vec<String> = symbols.iter().map(|s| s.id.clone()).collect();
    ids.sort();
    ids.dedup();
    let before = overview::spark_requests();
    hub.send(Request::Quotes {
        symbols: symbols.clone(),
    });
    let mut first: HashMap<String, (f64, f64, Instant)> = HashMap::new();
    let mut again: HashMap<String, Duration> = HashMap::new();
    let mut shared = None;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(75) {
        for e in hub.poll() {
            if let Event::Quote { id, last, prev } = e {
                match first.get(&id) {
                    None => {
                        first.insert(id, (last, prev, Instant::now()));
                    }
                    Some((_, _, at)) => {
                        again.entry(id).or_insert(at.elapsed());
                    }
                }
                // Right after a round, the overview asking for the same
                // tickers is answered from it.
                if shared.is_none() && first.len() == ids.len() {
                    let before = overview::spark_requests();
                    let keys: Vec<String> = us.iter().map(|t| t.to_string()).collect();
                    let prices = overview::prices(overview::Market::Us, &keys, false).unwrap();
                    assert_eq!(prices.len(), keys.len());
                    shared = Some(overview::spark_requests() - before);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let missing: Vec<&String> = ids.iter().filter(|i| !first.contains_key(*i)).collect();
    println!("{} symbols, quotes for {}; missing {missing:?}", ids.len(), first.len());
    assert!(missing.is_empty());
    // Each stock against Yahoo's own quote for it, asked right now.
    let yahoo = opentern_data::yahoo::Yahoo::default();
    for s in &symbols {
        if s.id == formula {
            continue;
        }
        let (last, prev, _) = first[&s.id];
        let (yl, yp) = yahoo.quote(s).unwrap();
        println!(
            "{:<18} batch {last:>10.4} / {prev:>10.4}   own {yl:>10.4} / {yp:>10.4}",
            s.id
        );
        assert!(
            (prev / yp - 1.0).abs() < 1e-6,
            "{}: previous close {prev} vs {yp}",
            s.id
        );
        assert!((last / yl - 1.0).abs() < 0.01, "{}: price {last} vs {yl}", s.id);
    }
    let (a, m) = (first["NASDAQ:AAPL"].0, first["NASDAQ:MSFT"].0);
    let f = first[formula].0;
    assert!((f - (a + m) / 2.0).abs() < 1e-6 * f, "formula {f} vs {}", (a + m) / 2.0);
    // Requests: the first round in twenties, then one round per 30 s.
    let sent = overview::spark_requests() - before;
    println!("spark requests in 75 s: {sent}");
    for (id, d) in &again {
        if id.starts_with("NASDAQ:") {
            println!("{id} again after {:.1} s", d.as_secs_f64());
        }
    }
    if opentern_core::calendar::trading(
        opentern_core::calendar::Market::Us,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64,
    ) {
        for t in us {
            let d = again.get(&format!("NASDAQ:{t}")).copied();
            assert!(
                d.is_some_and(|d| d < Duration::from_secs(35)),
                "NASDAQ:{t} not again: {d:?}"
            );
        }
        let n = symbols.len() - 1;
        assert!(sent <= 3 * n.div_ceil(20) + 3, "{sent} requests for {n} symbols");
    }
    println!("overview right after a round: {shared:?} new requests");
    assert_eq!(shared, Some(0));
}

/// Bybit, OKX, Coinbase and Kraken pairs in the watchlist tick live (their
/// own streams), not every 30 seconds.
#[test]
#[ignore]
fn other_exchanges_quote_live() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::start(dir.path().to_path_buf(), false, Arc::new(|| {}));
    let ids = ["BYBIT:BTCUSDT", "OKX:BTCUSDT", "COINBASE:BTCUSD", "KRAKEN:BTCUSD"];
    let symbols: Vec<Symbol> = ids.iter().copied().map(crypto).collect();
    hub.send(Request::Quotes { symbols });
    let mut seen: std::collections::HashMap<String, Vec<f64>> = Default::default();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(25) {
        for e in hub.poll() {
            if let Event::Quote { id, last, prev } = e {
                assert!(last > 0.0 && prev > 0.0, "{id}: {last} {prev}");
                seen.entry(id).or_default().push(last);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    for id in ids {
        let n = seen.get(id).map_or(0, |v| v.len());
        println!(
            "{id}: {n} quotes in 25 s, last {:?}",
            seen.get(id).and_then(|v| v.last())
        );
    }
    // Each pair updated several times (a 30 s poll would give one).
    let live = ids
        .iter()
        .filter(|id| seen.get(**id).is_some_and(|v| v.len() >= 3))
        .count();
    assert!(live >= 3, "{seen:?}");
    // No faster than once a second each.
    assert!(seen.values().all(|v| v.len() <= 27), "{seen:?}");
}

/// Binance still lists XMRUSDT (status BREAK) but its daily list leaves it
/// out, so a saved XMRUSDT carries on where Monero trades.
#[test]
#[ignore]
fn delisted_pairs_find_where_the_coin_trades() {
    let mut known = opentern_data::binance::Binance::default().symbols().unwrap();
    known.extend(opentern_data::kraken::Kraken::default().symbols().unwrap());
    known.extend(
        opentern_data::coinbase::Coinbase::default()
            .symbols()
            .unwrap_or_default(),
    );
    let xmr = opentern_core::Symbol {
        id: "BINANCE:XMRUSDT".into(),
        description: "XMR / USDT".into(),
        kind: opentern_core::AssetKind::Crypto,
        currency: "USDT".into(),
        tick_size: 0.01,
        min_qty: 0.0,
        timezone: "UTC".into(),
    };
    let to = opentern_data::hub::successor(&known, &xmr);
    println!("BINANCE:XMRUSDT → {:?}", to.as_ref().map(|s| &s.id));
    assert!(to.is_some_and(|s| s.id.ends_with("XMRUSD") || s.id.ends_with("XMRUSDT")));
    let btc = known.iter().find(|s| s.id == "BINANCE:BTCUSDT").unwrap();
    assert_eq!(opentern_data::hub::successor(&known, btc), None);
}

/// Perpetual futures on Binance, Bybit and OKX: their lists, history, a
/// live bar from the stream, and the funding rate.
#[test]
#[ignore]
fn perpetuals_list_chart_stream_and_fund() {
    use opentern_data::provider::{Live, Provider};
    let tf = opentern_core::Timeframe::parse("1m").unwrap();
    let day = opentern_core::Timeframe::parse("1D").unwrap();
    let binance = opentern_data::binance::Binance::perp();
    let bybit = opentern_data::bybit::Bybit::perp();
    let okx = opentern_data::okx::Okx::default();
    let lists = [
        ("binance", binance.perps().unwrap()),
        ("bybit", bybit.perps().unwrap()),
        ("okx", okx.perps().unwrap()),
    ];
    for (name, list) in &lists {
        let btc = list.iter().find(|s| s.ticker() == "BTCUSDT.P").expect("BTC perpetual");
        println!(
            "{name}: {} perpetuals; {} \"{}\" tick {}",
            list.len(),
            btc.id,
            btc.description,
            btc.tick_size
        );
    }
    let providers: [(&dyn Provider, &str); 3] = [
        (&binance, "BINANCE:BTCUSDT.P"),
        (&bybit, "BYBIT:BTCUSDT.P"),
        (&okx, "OKX:BTCUSDT.P"),
    ];
    for (p, id) in providers {
        let sym = lists.iter().flat_map(|(_, l)| l).find(|s| s.id == id).unwrap().clone();
        let bars = p.latest(&sym, day, 400).unwrap();
        let last = bars.last().unwrap();
        println!(
            "{id}: {} daily bars, last close {} vol {:.0}",
            bars.len(),
            last.close,
            last.volume
        );
        assert!(bars.len() >= 300 && last.close > 1000.0);
        let mut got = None;
        let started = std::time::Instant::now();
        let _ = p.stream(&sym, tf, &mut |item| {
            if let Live::Bar(b) = item {
                got = Some(b.close);
            }
            got.is_none() && started.elapsed().as_secs() < 20
        });
        println!("  stream: {got:?}");
        assert!(got.is_some(), "{id}: no live bar");
    }
    for (id, f) in [
        (
            "BINANCE",
            binance.funding(&opentern_core::Symbol::bare("BINANCE:BTCUSDT.P")),
        ),
        ("BYBIT", bybit.funding(&opentern_core::Symbol::bare("BYBIT:BTCUSDT.P"))),
        ("OKX", okx.funding(&opentern_core::Symbol::bare("OKX:BTCUSDT.P"))),
    ] {
        let (rate, next) = f.unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let mins = (next - now) / 60_000;
        println!("{id} funding {:+.4}% in {mins} min", rate * 100.0);
        assert!(rate.abs() < 0.01 && (0..=8 * 60).contains(&mins));
    }
}

/// Hyperliquid perpetuals: the list, daily and minute history, paging
/// back until its 5000-candle limit, a live bar, funding and the book.
#[test]
#[ignore]
fn hyperliquid_perpetuals() {
    use opentern_data::provider::{Live, Provider};
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    };
    let hl = opentern_data::hyperliquid::Hyperliquid::default();
    let list = hl.perps().unwrap();
    let btc = list
        .iter()
        .find(|s| s.id == "HYPERLIQUID:BTCUSDC.P")
        .expect("BTC")
        .clone();
    let odd: Vec<&str> = list
        .iter()
        .filter(|s| s.ticker().starts_with('k'))
        .map(|s| s.ticker())
        .take(3)
        .collect();
    println!(
        "{} perpetuals; {} \"{}\" tick {} min qty {}; k-coins {odd:?}",
        list.len(),
        btc.id,
        btc.description,
        btc.tick_size,
        btc.min_qty
    );
    assert!(list.len() > 50);
    let day = opentern_core::Timeframe::parse("1D").unwrap();
    let bars = hl.latest(&btc, day, 400).unwrap();
    let last = *bars.last().unwrap();
    println!(
        "1D: {} bars, last close {} vol {:.0}",
        bars.len(),
        last.close,
        last.volume
    );
    assert!(bars.len() >= 300 && last.close > 1000.0);
    assert!(bars.windows(2).all(|w| w[0].time < w[1].time));
    let m1 = opentern_core::Timeframe::parse("1m").unwrap();
    let recent = hl.latest(&btc, m1, 500).unwrap();
    println!(
        "1m: {} bars, newest {} ms ago",
        recent.len(),
        now() - recent.last().unwrap().time
    );
    assert!(recent.len() >= 450);
    // Back in pages until it runs out.
    let mut end = recent[0].time;
    let mut total = recent.len();
    for _ in 0..10 {
        let older = hl.before(&btc, m1, end, 1000).unwrap();
        if older.is_empty() {
            break;
        }
        assert!(older.last().unwrap().time < end);
        end = older[0].time;
        total += older.len();
    }
    let days = (now() - end) as f64 / 86_400_000.0;
    println!("1m history: {total} bars, {days:.1} days back");
    let since = hl.since(&btc, m1, recent[recent.len() - 30].time).unwrap();
    println!("since: {} bars", since.len());
    assert!(since.len() >= 29);
    let mut got = None;
    let started = std::time::Instant::now();
    let _ = hl.stream(&btc, m1, &mut |item| {
        if let Live::Bar(b) = item {
            got = Some(b.close);
        }
        got.is_none() && started.elapsed().as_secs() < 20
    });
    println!("stream: {got:?}");
    assert!(got.is_some(), "no live bar");
    let (rate, next) = hl.funding(&btc).unwrap();
    let mins = (next - now()) / 60_000;
    println!("funding {:+.5}%/h, next in {mins} min", rate * 100.0);
    assert!(rate.abs() < 0.01 && (0..=60).contains(&mins));
    let book = opentern_data::depth::book(&btc, last.close, false).unwrap();
    println!(
        "book: {} bids, {} asks, best {:?} / {:?}",
        book.bids.len(),
        book.asks.len(),
        book.bids.first(),
        book.asks.first()
    );
    assert!(!book.bids.is_empty() && book.bids[0].0 < book.asks[0].0);
    // Builder-deployed dexes: gold, silver, Brent, the euro, the S&P 500.
    let builders: Vec<&Symbol> = list.iter().filter(|s| s.ticker().contains('_')).collect();
    println!("builder-dex markets: {}", builders.len());
    for t in [
        "XYZ_GOLDUSDC.P",
        "XYZ_SILVERUSDC.P",
        "XYZ_BRENTOILUSDC.P",
        "XYZ_EURUSDC.P",
        "XYZ_SP500USDC.P",
    ] {
        let s = builders
            .iter()
            .find(|s| s.ticker() == t)
            .unwrap_or_else(|| panic!("{t} listed"));
        let bars = hl.latest(s, day, 30).unwrap();
        let (rate, _) = hl.funding(s).unwrap();
        println!(
            "{t} \"{}\" tick {}: {} daily bars, close {}, funding {:+.5}%/h",
            s.description,
            s.tick_size,
            bars.len(),
            bars.last().unwrap().close,
            rate * 100.0
        );
        assert!(!bars.is_empty());
    }
    let gold = builders.iter().find(|s| s.ticker() == "XYZ_GOLDUSDC.P").unwrap();
    let mut got = None;
    let started = std::time::Instant::now();
    let _ = hl.stream(gold, m1, &mut |item| {
        if let Live::Bar(b) = item {
            got = Some(b.close);
        }
        got.is_none() && started.elapsed().as_secs() < 20
    });
    println!("gold stream: {got:?}");
    assert!(got.is_some(), "no live gold bar");
    let book = opentern_data::depth::book(gold, 0.0, false).unwrap();
    assert!(!book.bids.is_empty());
}

/// Symbol search's row prices: Binance, Yahoo (US, NSE) and forex.
#[test]
#[ignore]
fn search_peeks_prices() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::start(dir.path().to_path_buf(), false, Arc::new(|| {}));
    let mut want: Vec<Symbol> = ["BINANCE:BTCUSDT", "NASDAQ:AAPL", "NSE:KOTAKBANK", "FX:EURUSD"]
        .iter()
        .map(|id| Symbol::bare(id))
        .collect();
    want[0].kind = opentern_core::AssetKind::Crypto;
    let started = Instant::now();
    hub.send(Request::Peek {
        seq: 1,
        symbols: want.clone(),
    });
    let mut got = std::collections::HashMap::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && got.len() < want.len() {
        for e in hub.poll() {
            if let Event::Peek { quotes, .. } = e {
                for (id, last, prev) in quotes {
                    println!("{id}: {last} (prev {prev}) after {:?}", started.elapsed());
                    assert!(last > 0.0 && prev > 0.0);
                    got.insert(id, (last, prev));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(got.len(), want.len(), "{got:?}");
}

/// The New York Fed's effective federal funds rate: the whole history in
/// one request, a chart through the hub, and a second ask served from the
/// memo (no second request until the next publication time).
#[test]
#[ignore]
fn fed_funds_rate_from_the_new_york_fed() {
    let days = opentern_data::nyfed::days().unwrap();
    assert!(days.len() > 6000, "{} days", days.len());
    let last = days.last().unwrap();
    let age = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        - last.time)
        / 86_400_000;
    assert!(age <= 6, "newest day is {age} days old");
    assert!(
        last.target
            .is_some_and(|(a, b)| a <= last.rate + 0.25 && last.rate - 0.25 <= b)
    );
    let again = opentern_data::nyfed::days().unwrap();
    assert!(Arc::ptr_eq(&days, &again), "asked the network twice");
    println!("EFFR: {} days, last {last:?}", days.len());
    open(opentern_core::fed_funds(), "1D", false);
    open(opentern_core::fed_funds(), "1W", false);
}

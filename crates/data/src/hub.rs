//! The data hub: the UI sends requests, and background threads answer with
//! events. One session thread per open chart (cache first, then network,
//! then the live stream, reconnecting as needed), one thread for watchlist
//! quotes, one for symbol search. Nothing here blocks the UI.

use crate::binance::{self, Binance};
use crate::cache::Cache;
pub use crate::creds::Credentials;
use crate::demo::{self, Demo};
use crate::provider::{Error, Live, Provider};
use crate::resample;
use crate::yahoo::Yahoo;
use crossbeam_channel::{Receiver, Sender, unbounded};
use jiff::tz::TimeZone;
use opentern_core::{Bar, Symbol, Timeframe};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

/// Bars shown when a chart opens; more load as you scroll back.
const INITIAL: usize = 5000;
/// Base bars fetched at most for a resampled interval.
const MAX_BASE: usize = 40_000;
/// Bars per "load more" step.
const MORE: usize = 5000;

pub type Waker = Arc<dyn Fn() + Send + Sync>;

pub enum Request {
    Open {
        chart: u64,
        symbol: Symbol,
        tf: Timeframe,
        /// US stocks on Yahoo: pre- and after-hours bars too.
        ext: bool,
    },
    /// Load older history for a chart.
    More {
        chart: u64,
    },
    Close {
        chart: u64,
    },
    /// Follow last price and change for these symbols (replaces the list).
    Quotes {
        symbols: Vec<Symbol>,
    },
    Search {
        seq: u64,
        query: String,
    },
    /// Keys for the providers that need an account (from the keychain).
    Credentials(Credentials),
    /// Price and previous close once, for symbol search's rows; answered
    /// in parts (`Event::Peek`) as each source replies.
    Peek {
        seq: u64,
        symbols: Vec<Symbol>,
    },
    /// One-off history, no streaming (screener, portfolio backtests, other
    /// symbols in `request.security`): the latest `count` bars.
    Fetch {
        seq: u64,
        symbol: Symbol,
        tf: Timeframe,
        count: usize,
    },
    /// A company's fundamentals (custom indices).
    Fundamentals {
        seq: u64,
        symbol: Symbol,
    },
    /// The order book for the DOM widget (`last` seeds the demo book).
    Depth {
        seq: u64,
        symbol: Symbol,
        last: f64,
    },
    /// Top movers for the Hotlists widget.
    Hotlist {
        seq: u64,
        market: crate::feeds::HotMarket,
        kind: crate::feeds::HotKind,
    },
    /// Headlines for a symbol (the News widget).
    News {
        seq: u64,
        symbol: Symbol,
    },
    /// A market's heatmap: every index member's sector, cap and changes.
    Overview {
        seq: u64,
        market: crate::overview::Market,
    },
    /// The latest prices for an open heatmap's members (`Member::key`).
    OverviewPrices {
        seq: u64,
        market: crate::overview::Market,
        keys: Vec<String>,
    },
    /// A perpetual future's funding rate (`….P` symbols).
    Funding {
        symbol: Symbol,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Loading,
    Live,
    /// Showing cached or partial data; the message says why.
    Offline(String),
    Error(String),
}

pub enum Event {
    /// The chart's whole series (replaces what it had). `complete` means no
    /// older history is available.
    Bars {
        chart: u64,
        bars: Vec<Bar>,
        complete: bool,
    },
    /// The forming bar changed, or a new one started.
    Tick {
        chart: u64,
        bar: Bar,
    },
    Status {
        chart: u64,
        status: Status,
    },
    Quote {
        id: String,
        last: f64,
        prev: f64,
    },
    /// A perpetual's funding rate (a fraction per period, e.g. 0.0001 =
    /// 0.01%) and when it's next paid (ms).
    Funding {
        id: String,
        result: Result<(f64, i64), String>,
    },
    /// A crypto pair its exchange no longer trades (it left the exchange's
    /// list) and the same coin where it still does: charts and quotes
    /// carry on with `to`.
    Replaced {
        from: Symbol,
        to: Symbol,
    },
    /// A US stock's pre-market or after-hours price (`None` once those
    /// hours are over), for symbols in `Request::Quotes`.
    Extended {
        id: String,
        ext: Option<crate::yahoo::Ext>,
    },
    /// Symbol search results. Those from the symbols already known come
    /// first (`done: false`); the full answer follows.
    Search {
        seq: u64,
        results: Vec<Symbol>,
        done: bool,
    },
    /// The chart symbol's minimum quantity (lot step), when it wasn't
    /// known at open: strategies round their sizes down to it.
    Lot {
        chart: u64,
        min_qty: f64,
    },
    /// A stock's dividends and splits (for the chart's markers and ADJ).
    Corporate {
        chart: u64,
        corporate: crate::yahoo::Corporate,
    },
    Hotlist {
        seq: u64,
        result: Result<Vec<crate::feeds::HotRow>, String>,
    },
    News {
        seq: u64,
        result: Result<Vec<crate::feeds::NewsItem>, String>,
    },
    /// The answer to a `Request::Overview`.
    Overview {
        seq: u64,
        market: crate::overview::Market,
        result: Result<crate::overview::Board, String>,
    },
    /// The answer to a `Request::OverviewPrices`.
    OverviewPrices {
        seq: u64,
        market: crate::overview::Market,
        result: Result<HashMap<String, crate::overview::Price>, String>,
    },
    /// The answer to a `Request::Depth`.
    Depth {
        seq: u64,
        result: Result<crate::depth::Book, String>,
    },
    /// Part of the answer to a `Request::Peek`: (id, last, previous close).
    Peek {
        seq: u64,
        quotes: Vec<(String, f64, f64)>,
    },
    /// The answer to a `Request::Fundamentals`.
    Fundamentals {
        seq: u64,
        symbol: String,
        result: Result<crate::fundamentals::Fundamentals, String>,
    },
    /// The answer to a `Request::Fetch`.
    Fetched {
        seq: u64,
        symbol: String,
        tf: Timeframe,
        result: Result<Vec<Bar>, String>,
    },
}

pub struct Hub {
    tx: Sender<Request>,
    rx: Receiver<Event>,
}

impl Hub {
    /// Starts the hub. `dir` is the cache folder; `demo` serves made-up
    /// prices with no network at all; `wake` asks the UI to repaint.
    pub fn start(dir: PathBuf, demo: bool, wake: Waker) -> Hub {
        let _ = CACHE_DIR.set(dir.clone());
        DEMO.store(demo, std::sync::atomic::Ordering::Relaxed);
        let (tx, req_rx) = unbounded();
        let (ev_tx, rx) = unbounded();
        let out = Out { tx: ev_tx, wake };
        std::thread::Builder::new()
            .name("data-hub".into())
            .spawn(move || manager(dir, demo, req_rx, out))
            .expect("spawn data hub");
        Hub { tx, rx }
    }

    pub fn send(&self, r: Request) {
        let _ = self.tx.send(r);
    }

    /// A handle for sending requests from another thread.
    pub fn sender(&self) -> Sender<Request> {
        self.tx.clone()
    }

    /// Everything that arrived since the last call.
    pub fn poll(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}

#[derive(Clone)]
struct Out {
    tx: Sender<Event>,
    wake: Waker,
}

impl Out {
    fn send(&self, e: Event) {
        if self.tx.send(e).is_ok() {
            (self.wake)();
        }
    }
}

enum Cmd {
    More,
    Stop,
}

fn manager(dir: PathBuf, demo: bool, rx: Receiver<Request>, out: Out) {
    let mut sessions: HashMap<u64, Sender<Cmd>> = HashMap::new();
    // What each chart shows, to restart it when keys change.
    let mut opened: HashMap<u64, (Symbol, Timeframe, bool)> = HashMap::new();
    let spawn = |chart: u64, symbol: Symbol, tf: Timeframe, ext: bool| -> Option<Sender<Cmd>> {
        let (tx, cmd_rx) = unbounded();
        let (dir, out) = (dir.clone(), out.clone());
        std::thread::Builder::new()
            .name(format!("data-{}", symbol.ticker()))
            .spawn(move || {
                let symbol = carry_on(&dir, demo, vec![symbol], &out).remove(0);
                Session::run(chart, symbol, tf, ext, dir, demo, cmd_rx, out)
            })
            .ok()
            .map(|_| tx)
    };
    let mut quotes: Option<Sender<Cmd>> = None;
    let (search_tx, search_rx) = unbounded::<(u64, String)>();
    {
        let (dir, out) = (dir.clone(), out.clone());
        std::thread::Builder::new()
            .name("data-search".into())
            .spawn(move || search_worker(dir, demo, search_rx, out))
            .expect("spawn search");
    }
    // A few threads answer one-off fetches, so a screener over a hundred
    // symbols doesn't start a hundred threads.
    let (fetch_tx, fetch_rx) = unbounded::<(u64, Symbol, Timeframe, usize)>();
    for k in 0..4 {
        let (dir, out, rx) = (dir.clone(), out.clone(), fetch_rx.clone());
        let _ = std::thread::Builder::new()
            .name(format!("data-fetch-{k}"))
            .spawn(move || {
                for (seq, symbol, tf, count) in rx {
                    let result = fetch_once(&dir, demo, &symbol, tf, count);
                    out.send(Event::Fetched {
                        seq,
                        symbol: symbol.id.clone(),
                        tf,
                        result,
                    });
                }
            });
    }
    // Fundamentals one at a time: Yahoo limits bursts.
    let (fund_tx, fund_rx) = unbounded::<(u64, Symbol)>();
    {
        let out = out.clone();
        let _ = std::thread::Builder::new()
            .name("data-fundamentals".into())
            .spawn(move || {
                for (seq, symbol) in fund_rx {
                    let result = crate::fundamentals::fetch(&symbol, demo).map_err(|e| e.to_string());
                    out.send(Event::Fundamentals {
                        seq,
                        symbol: symbol.id.clone(),
                        result,
                    });
                }
            });
    }
    for req in rx {
        match req {
            Request::Fetch { seq, symbol, tf, count } => {
                let _ = fetch_tx.send((seq, symbol, tf, count));
            }
            Request::Fundamentals { seq, symbol } => {
                let _ = fund_tx.send((seq, symbol));
            }
            Request::Hotlist { seq, market, kind } => {
                let out = out.clone();
                let _ = std::thread::Builder::new().name("data-hotlist".into()).spawn(move || {
                    let result = crate::feeds::hotlist(market, kind, demo).map_err(|e| e.to_string());
                    out.send(Event::Hotlist { seq, result });
                });
            }
            Request::Peek { seq, symbols } => {
                let out = out.clone();
                let _ = std::thread::Builder::new()
                    .name("data-peek".into())
                    .spawn(move || peek(seq, symbols, demo, &out));
            }
            Request::Overview { seq, market } => {
                let out = out.clone();
                let dir = dir.clone();
                let _ = std::thread::Builder::new().name("data-overview".into()).spawn(move || {
                    let result = crate::overview::board(market, &dir, demo).map_err(|e| e.to_string());
                    out.send(Event::Overview { seq, market, result });
                });
            }
            Request::OverviewPrices { seq, market, keys } => {
                let out = out.clone();
                let _ = std::thread::Builder::new().name("data-overview".into()).spawn(move || {
                    let result = crate::overview::prices(market, &keys, demo).map_err(|e| e.to_string());
                    out.send(Event::OverviewPrices { seq, market, result });
                });
            }
            Request::Depth { seq, symbol, last } => {
                let out = out.clone();
                let _ = std::thread::Builder::new().name("data-depth".into()).spawn(move || {
                    let result = crate::depth::book(&symbol, last, demo).map_err(|e| e.to_string());
                    out.send(Event::Depth { seq, result });
                });
            }
            Request::News { seq, symbol } => {
                let out = out.clone();
                let _ = std::thread::Builder::new().name("data-news".into()).spawn(move || {
                    let result = crate::feeds::news(&symbol, demo).map_err(|e| e.to_string());
                    out.send(Event::News { seq, result });
                });
            }
            Request::Funding { symbol } => {
                let out = out.clone();
                let _ = std::thread::Builder::new().name("data-funding".into()).spawn(move || {
                    let result = funding(&symbol, demo).map_err(|e| e.to_string());
                    out.send(Event::Funding {
                        id: symbol.id.clone(),
                        result,
                    });
                });
            }
            Request::Open { chart, symbol, tf, ext } => {
                if let Some(old) = sessions.remove(&chart) {
                    let _ = old.send(Cmd::Stop);
                }
                opened.insert(chart, (symbol.clone(), tf, ext));
                if symbol.min_qty == 0.0
                    && !demo
                    && !crate::binance::is_perp(&symbol)
                    && let Some(ticker) = symbol.id.strip_prefix("BINANCE:").map(str::to_string)
                {
                    let out = out.clone();
                    let _ = std::thread::Builder::new().name("data-lot".into()).spawn(move || {
                        if let Ok(min_qty) = Binance::default().min_qty(&ticker) {
                            out.send(Event::Lot { chart, min_qty });
                        }
                    });
                }
                if let Some(tx) = spawn(chart, symbol, tf, ext) {
                    sessions.insert(chart, tx);
                }
            }
            Request::More { chart } => {
                if let Some(s) = sessions.get(&chart) {
                    let _ = s.send(Cmd::More);
                }
            }
            Request::Close { chart } => {
                if let Some(s) = sessions.remove(&chart) {
                    let _ = s.send(Cmd::Stop);
                }
                opened.remove(&chart);
            }
            Request::Quotes { symbols } => {
                if let Some(q) = quotes.take() {
                    let _ = q.send(Cmd::Stop);
                }
                let (tx, cmd_rx) = unbounded();
                let out = out.clone();
                let dir = dir.clone();
                if std::thread::Builder::new()
                    .name("data-quotes".into())
                    .spawn(move || {
                        let symbols = carry_on(&dir, demo, symbols, &out);
                        quote_worker(symbols, demo, cmd_rx, out)
                    })
                    .is_ok()
                {
                    quotes = Some(tx);
                }
            }
            Request::Search { seq, query } => {
                let _ = search_tx.send((seq, query));
            }
            Request::Credentials(c) => {
                let changed = *creds().read().unwrap_or_else(|e| e.into_inner()) != c;
                *creds().write().unwrap_or_else(|e| e.into_inner()) = c;
                // Charts may now come from another source: restart them.
                if changed {
                    for (chart, (symbol, tf, ext)) in &opened {
                        if let Some(old) = sessions.remove(chart) {
                            let _ = old.send(Cmd::Stop);
                        }
                        if let Some(tx) = spawn(*chart, symbol.clone(), *tf, *ext) {
                            sessions.insert(*chart, tx);
                        }
                    }
                }
            }
        }
    }
}

/// Where to top cached bars up from: a few bars before the newest, so the
/// answer overlaps bars the cache holds as final (see `history_changed`).
fn overlap_start(cached: &[Bar]) -> i64 {
    cached[cached.len().saturating_sub(3)].time
}

/// A closed bar's close that moved by more than this between the cache and
/// the provider means the provider re-adjusted its history.
const READJUSTED: f64 = 0.02;

/// True when the provider's copy of bars the cache holds as final differs:
/// it adjusted a split or bonus into its history since they were cached,
/// so everything cached is in the old units (a 2-for-1 split would read
/// as a 50% fall). The newest cached bar may have been saved while it was
/// still forming, so it doesn't count.
pub(crate) fn history_changed(cached: &[Bar], fresh: &[Bar]) -> bool {
    let settled = &cached[..cached.len().saturating_sub(1)];
    fresh.iter().any(|f| {
        settled.binary_search_by_key(&f.time, |b| b.time).is_ok_and(|i| {
            let c = settled[i].close;
            c > 0.0 && f.close > 0.0 && (f.close / c - 1.0).abs() > READJUSTED
        })
    })
}

/// The latest `count` bars of a symbol: from the cache, topped up from
/// the network (and saved back), resampled to `tf`.
/// Symbols the source said it no longer has (Yahoo's 404), since this
/// run started; cleared when one answers again. Their cached bars still
/// serve, so this is how they're told apart.
static GONE_IDS: OnceLock<RwLock<std::collections::HashSet<String>>> = OnceLock::new();

fn gone_ids() -> &'static RwLock<std::collections::HashSet<String>> {
    GONE_IDS.get_or_init(Default::default)
}

/// Whether the source said it no longer has `id` (renamed or delisted).
pub fn is_gone(id: &str) -> bool {
    gone_ids().read().is_ok_and(|g| g.contains(id))
}

/// The workspace the hub caches bars in (set when it starts).
static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Bars through the cache when the hub runs (custom index members), else
/// straight from the source.
pub(crate) fn fetch_cached(
    symbol: &Symbol,
    tf: Timeframe,
    count: usize,
    demo: bool,
) -> crate::provider::Result<Vec<Bar>> {
    match CACHE_DIR.get() {
        Some(dir) => fetch_bars(dir, demo, symbol, tf, count),
        None => provider_for(symbol, demo).latest(symbol, tf, count),
    }
}

pub(crate) fn fetch_once(
    dir: &std::path::Path,
    demo: bool,
    symbol: &Symbol,
    tf: Timeframe,
    count: usize,
) -> Result<Vec<Bar>, String> {
    fetch_bars(dir, demo, symbol, tf, count).map_err(|e| e.to_string())
}

/// The newest `count` bars of `tf`: the cache first, topped up from the
/// source (or downloaded again when a split changed the past).
fn fetch_bars(
    dir: &std::path::Path,
    demo: bool,
    symbol: &Symbol,
    tf: Timeframe,
    count: usize,
) -> crate::provider::Result<Vec<Bar>> {
    let provider = provider_for(symbol, demo);
    let base = resample::choose_base(tf, &provider.natives()).ok_or_else(|| {
        crate::provider::Error::Bad(format!("{} bars aren't available for {}.", tf.label(), symbol.ticker()))
    })?;
    let ratio = match (tf.millis(), base.millis()) {
        (Some(t), Some(b)) if b > 0 => (t / b).max(1) as usize,
        _ => 22,
    };
    let wanted = (count.max(1) * ratio + ratio).min(MAX_BASE);
    let cache = if demo || crate::expr::is_custom(&symbol.id) {
        None
    } else {
        Cache::open(dir).ok()
    };
    let mut bars = cache
        .as_ref()
        .and_then(|c| c.last(&symbol.id, base, wanted).ok())
        .unwrap_or_default();
    let fresh = match bars.last() {
        Some(last) if bars.len() >= wanted / 2 => {
            let behind = (crate::provider::now_ms() - last.time) / base.millis().unwrap_or(86_400_000);
            if behind as usize > wanted {
                bars.clear();
                provider.latest(symbol, base, wanted)
            } else {
                match provider.since(symbol, base, overlap_start(&bars)) {
                    // A split or bonus adjusted in since the bars were cached.
                    Ok(f) if history_changed(&bars, &f) => {
                        if let Some(c) = &cache {
                            let _ = c.clear(&symbol.id, base);
                        }
                        bars.clear();
                        provider.latest(symbol, base, wanted)
                    }
                    other => other,
                }
            }
        }
        _ => {
            bars.clear();
            provider.latest(symbol, base, wanted)
        }
    };
    if let Ok(mut g) = gone_ids().write() {
        match &fresh {
            Err(e) if e.to_string().starts_with(crate::provider::GONE) => g.insert(symbol.id.clone()),
            Ok(_) => g.remove(&symbol.id),
            Err(_) => false,
        };
    }
    let fresh = match fresh {
        Ok(f) => f,
        // Offline: what the cache has is better than nothing.
        Err(_) if !bars.is_empty() => Vec::new(),
        Err(e) => return Err(e),
    };
    if let Some(c) = &cache {
        let _ = c.save(&symbol.id, base, &fresh);
    }
    for b in fresh {
        match bars.last() {
            Some(l) if l.time == b.time => *bars.last_mut().expect("non-empty") = b,
            Some(l) if l.time > b.time => {
                if let Ok(i) = bars.binary_search_by_key(&b.time, |x| x.time) {
                    bars[i] = b;
                }
            }
            _ => bars.push(b),
        }
    }
    let tz = TimeZone::get(&symbol.timezone).unwrap_or(TimeZone::UTC);
    let mut series = if base == tf {
        bars
    } else {
        resample::resample(&bars, tf, &tz)
    };
    let skip = series.len().saturating_sub(count);
    series.drain(..skip);
    Ok(series)
}

/// The keys the app handed over (one hub per process).
fn creds() -> &'static RwLock<Credentials> {
    static C: OnceLock<RwLock<Credentials>> = OnceLock::new();
    C.get_or_init(|| RwLock::new(Credentials::default()))
}

/// A provider that can't be used yet, and says why.
struct Missing(String);

impl Provider for Missing {
    fn natives(&self) -> Vec<Timeframe> {
        ["1m", "1h", "1D"].iter().filter_map(|s| Timeframe::parse(s)).collect()
    }
    fn latest(&self, _: &Symbol, _: Timeframe, _: usize) -> crate::provider::Result<Vec<Bar>> {
        Err(crate::provider::Error::Bad(self.0.clone()))
    }
    fn since(&self, _: &Symbol, _: Timeframe, _: i64) -> crate::provider::Result<Vec<Bar>> {
        Err(crate::provider::Error::Bad(self.0.clone()))
    }
    fn before(&self, _: &Symbol, _: Timeframe, _: i64, _: usize) -> crate::provider::Result<Vec<Bar>> {
        Err(crate::provider::Error::Bad(self.0.clone()))
    }
    fn stream(
        &self,
        _: &Symbol,
        _: Timeframe,
        _: &mut dyn FnMut(crate::provider::Live) -> bool,
    ) -> crate::provider::Result<()> {
        Err(crate::provider::Error::Bad(self.0.clone()))
    }
}

/// Why a chart can't have `tf` bars of `sym`.
fn no_bars(sym: &Symbol, tf: Timeframe) -> String {
    if opentern_core::daily_only(&sym.id) {
        crate::nyfed::DAILY_ONLY.into()
    } else {
        format!("{} bars aren't available for this symbol.", tf.label())
    }
}

/// Which source serves a symbol: by its exchange prefix, using an account
/// provider (Alpaca for US stocks, Upstox for NSE/BSE) when its keys are
/// set, else Yahoo.
pub(crate) fn provider_for(sym: &Symbol, demo: bool) -> Arc<dyn Provider> {
    if crate::expr::is_custom(&sym.id) {
        return match crate::expr::custom_segments(&sym.id)
            .and_then(|s| crate::expr::ExprProvider::from_segments(&s, demo))
        {
            Some(p) => Arc::new(p),
            None => Arc::new(Missing("This index no longer exists.".into())),
        };
    }
    if crate::expr::is_expr(&sym.id)
        && let Some(p) = crate::expr::ExprProvider::new(sym, demo)
    {
        return Arc::new(p);
    }
    if sym.id == crate::nyfed::ID {
        return Arc::new(crate::nyfed::NyFed::new(demo));
    }
    if demo {
        return Arc::new(Demo);
    }
    let c = creds().read().unwrap_or_else(|e| e.into_inner()).clone();
    match sym.exchange() {
        "BINANCE" if crate::binance::is_perp(sym) => Arc::new(Binance::perp()),
        "BINANCE" => Arc::new(Binance::default()),
        "BYBIT" if crate::binance::is_perp(sym) => Arc::new(crate::bybit::Bybit::perp()),
        "BYBIT" => Arc::new(crate::bybit::Bybit::default()),
        "OKX" => Arc::new(crate::okx::Okx::default()),
        "COINBASE" => Arc::new(crate::coinbase::Coinbase::default()),
        "KRAKEN" => Arc::new(crate::kraken::Kraken::default()),
        "HYPERLIQUID" => Arc::new(crate::hyperliquid::Hyperliquid::default()),
        "OANDA" => match &c.oanda {
            Some((token, account, live)) => Arc::new(crate::oanda::Oanda::new(token, account, *live)),
            None => Arc::new(Missing(
                "Add your OANDA API token in Settings → Data providers to see OANDA prices.".into(),
            )),
        },
        "IBKR" => Arc::new(crate::ibkr::Ibkr::new(
            c.ibkr.as_deref().unwrap_or("https://localhost:5000"),
        )),
        "NASDAQ" | "NYSE" | "AMEX" if c.alpaca.is_some() => {
            let (k, s, sip) = c.alpaca.clone().unwrap_or_default();
            Arc::new(crate::alpaca::Alpaca::new(&k, &s, sip))
        }
        "NSE" | "BSE" if c.upstox.is_some() => {
            Arc::new(crate::upstox::Upstox::new(c.upstox.as_deref().unwrap_or_default()))
        }
        _ => Arc::new(Yahoo::default()),
    }
}

/// The exchanges whose whole list of pairs is kept (refreshed daily): a
/// pair missing from its list has stopped trading there.
fn listed_source(exchange: &str) -> Option<&'static str> {
    Some(match exchange {
        "BINANCE" => "binance",
        "BYBIT" => "bybit",
        "OKX" => "okx",
        "COINBASE" => "coinbase",
        "KRAKEN" => "kraken",
        _ => return None,
    })
}

/// Where a crypto pair carries on when its exchange stopped trading it:
/// the same coin on the best other exchange (USDT/USD first; Binance,
/// Coinbase, Bybit, OKX, Kraken). None while it still trades, when the
/// exchange's list isn't in yet, or when no exchange has the coin.
pub fn successor(known: &[Symbol], sym: &Symbol) -> Option<Symbol> {
    // Perpetuals carry on as perpetuals or not at all (no spot stand-in).
    if sym.kind != opentern_core::AssetKind::Crypto || crate::binance::is_perp(sym) {
        return None;
    }
    listed_source(sym.exchange())?;
    let prefix = format!("{}:", sym.exchange());
    let mut has_list = false;
    for s in known {
        if s.id == sym.id {
            return None;
        }
        has_list |= s.id.starts_with(&prefix);
    }
    if !has_list {
        return None;
    }
    let base = match sym.description.split_once(" / ") {
        Some((b, _)) => b.trim().to_ascii_uppercase(),
        None => crate::provider::split_pair(sym.ticker())?.0.to_ascii_uppercase(),
    };
    crate::overview::crypto_pairs(known).remove(&base)
}

/// A perpetual's funding rate and next payment, from its exchange.
fn funding(sym: &Symbol, demo: bool) -> crate::provider::Result<(f64, i64)> {
    if demo {
        let next = (crate::provider::now_ms() / 28_800_000 + 1) * 28_800_000;
        return Ok((0.0001, next));
    }
    match sym.exchange() {
        "BINANCE" => Binance::perp().funding(sym),
        "BYBIT" => crate::bybit::Bybit::perp().funding(sym),
        "OKX" => crate::okx::Okx::default().funding(sym),
        "HYPERLIQUID" => crate::hyperliquid::Hyperliquid::default().funding(sym),
        _ => Err(crate::provider::Error::Bad("No funding rate for this symbol.".into())),
    }
}

/// When the exchanges' symbol lists were last fetched. Renamed when the
/// lists gain a source (v2: perpetuals; v3: Hyperliquid; v4: its USDC names; v5: its builder dexes;
/// v6: their names), so they're fetched again at once.
const LISTS_AT: &str = "symbol_lists_at_v6";

/// Swaps delisted pairs for their successors, saying so once each.
fn carry_on(dir: &std::path::Path, demo: bool, symbols: Vec<Symbol>, out: &Out) -> Vec<Symbol> {
    if demo || !symbols.iter().any(|s| s.kind == opentern_core::AssetKind::Crypto) {
        return symbols;
    }
    let known = Cache::open(dir).ok().and_then(|c| c.symbols().ok()).unwrap_or_default();
    symbols
        .into_iter()
        .map(|s| match successor(&known, &s) {
            Some(to) => {
                out.send(Event::Replaced {
                    from: s,
                    to: to.clone(),
                });
                to
            }
            None => s,
        })
        .collect()
}

pub(crate) static DEMO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// How many minutes behind the exchange a symbol's free prices are, when
/// known: CME's and ICE's futures (our commodities and futures, from Yahoo) reach free
/// sources at least 10 minutes late. Daily closes are the settlements.
pub fn delayed_minutes(sym: &Symbol) -> Option<u32> {
    (matches!(
        sym.kind,
        opentern_core::AssetKind::Commodity | opentern_core::AssetKind::Futures
    ) && !DEMO.load(std::sync::atomic::Ordering::Relaxed)
        && yahoo_routed(sym))
    .then_some(10)
}

/// Whether Yahoo serves a symbol (its quotes come from Yahoo's quote API).
fn yahoo_routed(sym: &Symbol) -> bool {
    let c = creds().read().unwrap_or_else(|e| e.into_inner());
    match sym.exchange() {
        "BINANCE" | "BYBIT" | "OKX" | "COINBASE" | "KRAKEN" | "HYPERLIQUID" | "OANDA" | "IBKR" | "NYFED" => false,
        "NASDAQ" | "NYSE" | "AMEX" => c.alpaca.is_none(),
        "NSE" | "BSE" => c.upstox.is_none(),
        _ => true,
    }
}

/// Crypto exchanges other than Binance: their public streams are free and
/// take many connections, so each watched pair gets its own (Binance's
/// pairs share one combined stream instead).
fn streams_quotes(sym: &Symbol) -> bool {
    matches!(sym.exchange(), "BYBIT" | "OKX" | "COINBASE" | "KRAKEN" | "HYPERLIQUID")
        || (sym.exchange() == "BINANCE" && crate::binance::is_perp(sym))
}

/// A pair's live price and previous daily close, from the exchange's own
/// stream of daily bars (as its chart gets them), until `stop` is set.
/// Reconnects with backoff.
fn stream_quote(sym: Symbol, tx: Sender<(String, f64, f64)>, stop: Arc<AtomicBool>) {
    let Some(day) = Timeframe::parse("1D") else { return };
    let stopped = || stop.load(Ordering::Relaxed);
    let mut backoff = Duration::from_secs(1);
    while !stopped() {
        let started = Instant::now();
        let p = provider_for(&sym, false);
        if let Ok(bars) = p.latest(&sym, day, 2)
            && let Some(&last) = bars.last()
        {
            let mut prev = if bars.len() >= 2 {
                bars[bars.len() - 2].close
            } else {
                last.open
            };
            let (mut today, mut close) = (last.time, last.close);
            let _ = tx.send((sym.id.clone(), close, prev));
            // At most one update a second (Coinbase sends every trade).
            let (mut sent, mut pending) = (Instant::now(), false);
            let _ = p.stream(&sym, day, &mut |item| {
                if let Live::Bar(b) = item {
                    // A new day: yesterday's last price is the new base.
                    if b.time > today {
                        (prev, today) = (close, b.time);
                    }
                    if b.time == today && b.close != close {
                        close = b.close;
                        pending = true;
                    }
                }
                if pending && sent.elapsed() >= Duration::from_secs(1) {
                    let _ = tx.send((sym.id.clone(), close, prev));
                    (sent, pending) = (Instant::now(), false);
                }
                !stopped()
            });
        }
        if started.elapsed() > Duration::from_secs(60) {
            backoff = Duration::from_secs(1);
        }
        let until = Instant::now() + backoff;
        while Instant::now() < until && !stopped() {
            std::thread::sleep(Duration::from_millis(200));
        }
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

/// The quote worker's per-pair streams (`stream_quote`), stopped when
/// dropped (the worker restarts with every new set of symbols).
#[derive(Default)]
struct PairStreams {
    running: HashMap<String, Arc<AtomicBool>>,
}

impl PairStreams {
    fn start(&mut self, sym: &Symbol, tx: &Sender<(String, f64, f64)>) {
        if self.running.contains_key(&sym.id) {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        self.running.insert(sym.id.clone(), stop.clone());
        let (sym, tx) = (sym.clone(), tx.clone());
        std::thread::Builder::new()
            .name("quote-stream".into())
            .spawn(move || stream_quote(sym, tx, stop))
            .ok();
    }
}

impl Drop for PairStreams {
    fn drop(&mut self) {
        for stop in self.running.values() {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

/// Last price and previous close from the source's daily bars (for
/// sources without a quote API).
fn daily_quote(sym: &Symbol) -> Option<(f64, f64)> {
    let p = provider_for(sym, false);
    let bars = p.latest(sym, Timeframe::parse("1D")?, 2).ok()?;
    let last = bars.last()?;
    let prev = if bars.len() >= 2 {
        bars[bars.len() - 2].close
    } else {
        last.open
    };
    Some((last.close, prev))
}

/// Prices for symbol search's rows, by the cheapest route per source:
/// Binance pairs in one trading-day batch (the watchlist's daily change),
/// Yahoo symbols in one spark batch (shared with the quote feed's
/// cache), anything else from its two latest daily bars. Each source's
/// part is sent as soon as it's in.
fn peek(seq: u64, symbols: Vec<Symbol>, demo: bool, out: &Out) {
    if demo {
        let quotes = symbols
            .iter()
            .map(|s| {
                let (last, prev) = demo::quote(s);
                (s.id.clone(), last, prev)
            })
            .collect();
        out.send(Event::Peek { seq, quotes });
        return;
    }
    let mut pairs = Vec::new();
    let mut yahoo = Vec::new();
    let mut rest = Vec::new();
    for s in symbols {
        if crate::expr::is_expr(&s.id) {
            continue;
        }
        if s.exchange() == "BINANCE" && !s.ticker().ends_with(".P") {
            pairs.push(s);
        } else if yahoo_routed(&s) && !crate::yahoo::ticker(&s.id).ends_with("=X") {
            yahoo.push(s);
        } else {
            rest.push(s);
        }
    }
    std::thread::scope(|sc| {
        sc.spawn(|| {
            if pairs.is_empty() {
                return;
            }
            let tickers: Vec<String> = pairs.iter().map(|s| s.ticker().to_string()).collect();
            let days = crate::overview::binance_days(&tickers);
            let quotes: Vec<_> = pairs
                .iter()
                .filter_map(|s| days.get(s.ticker()).map(|(open, last)| (s.id.clone(), *last, *open)))
                .collect();
            out.send(Event::Peek { seq, quotes });
        });
        sc.spawn(|| {
            if yahoo.is_empty() {
                return;
            }
            let tickers: Vec<String> = yahoo.iter().map(|s| crate::yahoo::ticker(&s.id)).collect();
            let sparks = crate::overview::sparks(&tickers, "5d");
            let quotes: Vec<_> = yahoo
                .iter()
                .zip(&tickers)
                .filter_map(|(s, t)| {
                    let indian = matches!(s.exchange(), "NSE" | "BSE");
                    let (last, prev) = sparks.get(t)?.quote(indian)?;
                    Some((s.id.clone(), last, prev))
                })
                .collect();
            out.send(Event::Peek { seq, quotes });
        });
        for s in &rest {
            sc.spawn(move || {
                if let Some((last, prev)) = daily_quote(s) {
                    out.send(Event::Peek {
                        seq,
                        quotes: vec![(s.id.clone(), last, prev)],
                    });
                }
            });
        }
    });
}

/// Exchanges in the order their symbols are offered when names tie.
fn exchange_rank(ex: &str) -> u32 {
    match ex {
        "BINANCE" => 0,
        "NASDAQ" | "NYSE" | "AMEX" | "NSE" => 1,
        "COINBASE" => 2,
        "BYBIT" => 3,
        "OKX" => 4,
        "KRAKEN" => 5,
        "HYPERLIQUID" => 6,
        _ => 9,
    }
}

/// One chart's data: cache, fetch, stream, reconnect.
struct Session {
    chart: u64,
    /// The cache's name for the series (pre/post-hours bars apart).
    cache_id: String,
    symbol: Symbol,
    tf: Timeframe,
    base: Timeframe,
    tz: TimeZone,
    provider: Arc<dyn Provider>,
    cache: Option<Cache>,
    bars: Vec<Bar>,
    complete: bool,
    /// Splits and bonus issues (date, ratio), for prices that don't
    /// reflect them yet (`yahoo::split_adjust`).
    splits: Vec<(i64, f64)>,
    cmds: Receiver<Cmd>,
    out: Out,
}

impl Session {
    #[allow(clippy::too_many_arguments)]
    fn run(
        chart: u64,
        symbol: Symbol,
        tf: Timeframe,
        ext: bool,
        dir: PathBuf,
        demo: bool,
        cmds: Receiver<Cmd>,
        out: Out,
    ) {
        // Pre- and after-hours bars: Yahoo's, for US stocks and funds on
        // intraday intervals. Cached apart from the regular-hours ones.
        let ext = ext
            && !demo
            && tf.is_intraday()
            && opentern_core::calendar::Market::of(&symbol) == opentern_core::calendar::Market::Us
            && yahoo_routed(&symbol);
        let provider: Arc<dyn Provider> = if ext {
            Arc::new(Yahoo::extended())
        } else {
            provider_for(&symbol, demo)
        };
        let cache_id = if ext {
            format!("{}#ext", symbol.id)
        } else {
            symbol.id.clone()
        };
        let Some(base) = resample::choose_base(tf, &provider.natives()) else {
            out.send(Event::Status {
                chart,
                status: Status::Error(no_bars(&symbol, tf)),
            });
            return;
        };
        let tz = TimeZone::get(&symbol.timezone).unwrap_or(TimeZone::UTC);
        // Demo data is made up on the spot; never cache it. Nor a custom
        // index: its members and weights can change under the same name.
        let cache = if demo || crate::expr::is_custom(&symbol.id) {
            None
        } else {
            Cache::open(&dir).ok()
        };
        let mut s = Session {
            chart,
            cache_id,
            symbol,
            tf,
            base,
            tz,
            provider,
            cache,
            bars: Vec::new(),
            complete: false,
            splits: Vec::new(),
            cmds,
            out,
        };
        s.status(Status::Loading);
        s.main();
    }

    fn status(&self, status: Status) {
        self.out.send(Event::Status {
            chart: self.chart,
            status,
        });
    }

    /// Base bars wanted for the first view.
    fn wanted(&self) -> usize {
        if self.base == self.tf {
            return INITIAL;
        }
        let ratio = match (self.tf.millis(), self.base.millis()) {
            (Some(t), Some(b)) if b > 0 => (t / b).max(1) as usize,
            _ => 22,
        };
        (INITIAL / 2 * ratio).min(MAX_BASE)
    }

    fn series(&self) -> Vec<Bar> {
        let adjusted = crate::yahoo::split_adjust(&self.bars, &self.splits, &self.tz);
        let bars = adjusted.as_deref().unwrap_or(&self.bars);
        if self.base == self.tf {
            bars.to_vec()
        } else {
            resample::resample(bars, self.tf, &self.tz)
        }
    }

    fn publish(&self) {
        self.out.send(Event::Bars {
            chart: self.chart,
            bars: self.series(),
            complete: self.complete,
        });
    }

    fn save(&self, bars: &[Bar]) {
        if let Some(c) = &self.cache {
            let _ = c.save(&self.cache_id, self.base, bars);
        }
    }

    /// Gets the series up to date: cache first (shown at once), then the
    /// missing tail from the network.
    fn load(&mut self) -> Result<(), Error> {
        let wanted = self.wanted();
        if self.bars.is_empty() {
            if let Some(c) = &self.cache {
                self.bars = c.last(&self.cache_id, self.base, wanted).unwrap_or_default();
            }
            if !self.bars.is_empty() {
                self.publish();
            }
        }
        let fresh = match self.bars.last() {
            Some(last) => {
                let behind = (crate::provider::now_ms() - last.time) / self.base.millis().unwrap_or(86_400_000);
                if behind as usize > wanted {
                    // Too far behind to patch: start over.
                    if let Some(c) = &self.cache {
                        let _ = c.clear(&self.cache_id, self.base);
                    }
                    self.bars.clear();
                    self.provider.latest(&self.symbol, self.base, wanted)?
                } else {
                    let fresh = self
                        .provider
                        .since(&self.symbol, self.base, overlap_start(&self.bars))?;
                    if history_changed(&self.bars, &fresh) {
                        // A split or bonus was adjusted into the provider's
                        // history: the cached bars are in old units.
                        if let Some(c) = &self.cache {
                            let _ = c.clear(&self.cache_id, self.base);
                        }
                        self.bars.clear();
                        self.complete = false;
                        self.provider.latest(&self.symbol, self.base, wanted)?
                    } else {
                        fresh
                    }
                }
            }
            None => self.provider.latest(&self.symbol, self.base, wanted)?,
        };
        self.save(&fresh);
        for b in fresh {
            self.upsert(b);
        }
        self.publish();
        Ok(())
    }

    /// Adds or replaces a bar, keeping the series sorted.
    fn upsert(&mut self, b: Bar) -> bool {
        match self.bars.last() {
            Some(last) if last.time == b.time => {
                *self.bars.last_mut().expect("non-empty") = b;
                false
            }
            Some(last) if last.time > b.time => {
                // An older bar the series lacked (a special session's bar
                // built from the quote) goes in its place.
                match self.bars.binary_search_by_key(&b.time, |x| x.time) {
                    Ok(i) => self.bars[i] = b,
                    Err(i) => self.bars.insert(i, b),
                }
                false
            }
            _ => {
                self.bars.push(b);
                true
            }
        }
    }

    /// Older history: from the cache if it has some, else the network.
    fn more(&mut self) {
        if self.complete {
            return;
        }
        let Some(first) = self.bars.first().map(|b| b.time) else {
            return;
        };
        let n = if self.base == self.tf { MORE } else { self.wanted() / 2 };
        let mut older = self
            .cache
            .as_ref()
            .and_then(|c| c.before(&self.cache_id, self.base, first, n).ok())
            .unwrap_or_default();
        if older.len() < n {
            let end = older.first().map(|b| b.time).unwrap_or(first);
            match self.provider.before(&self.symbol, self.base, end, n - older.len()) {
                Ok(net) => {
                    if net.is_empty() {
                        self.complete = true;
                    }
                    self.save(&net);
                    older.splice(0..0, net);
                }
                Err(e) => {
                    self.status(Status::Offline(e.to_string()));
                    return;
                }
            }
        }
        if !older.is_empty() {
            self.bars.splice(0..0, older);
        }
        self.publish();
    }

    /// Drains pending commands. Returns false when the chart closed.
    fn handle_cmds(&mut self) -> bool {
        while let Ok(cmd) = self.cmds.try_recv() {
            match cmd {
                Cmd::More => self.more(),
                Cmd::Stop => return false,
            }
        }
        true
    }

    /// Dividends, splits and earnings, for stocks and funds (from Yahoo
    /// and Nasdaq, whichever source serves the prices). Demo stocks get
    /// made-up earnings.
    fn corporate(&mut self, demo: bool) {
        use opentern_core::AssetKind;
        let stocky = matches!(self.symbol.kind, AssetKind::Stock | AssetKind::Fund)
            && matches!(
                self.symbol.exchange(),
                "NASDAQ" | "NYSE" | "AMEX" | "NSE" | "BSE" | "YAHOO"
            );
        if !stocky {
            return;
        }
        let earns = crate::earnings::applies(&self.symbol);
        let c = if demo {
            crate::yahoo::Corporate {
                earnings: if earns {
                    crate::earnings::made_up(&self.symbol, crate::provider::now_ms())
                } else {
                    Vec::new()
                },
                ..Default::default()
            }
        } else {
            let mut c = Yahoo::default().corporate(&self.symbol).unwrap_or_default();
            if earns {
                c.earnings = crate::earnings::fetch(&self.symbol).unwrap_or_default();
            }
            c
        };
        if !c.splits.is_empty() {
            self.splits = c.splits.clone();
            if crate::yahoo::split_adjust(&self.bars, &self.splits, &self.tz).is_some() {
                self.publish();
            }
        }
        if !c.dividends.is_empty() || !c.splits.is_empty() || !c.earnings.is_empty() {
            self.out.send(Event::Corporate {
                chart: self.chart,
                corporate: c,
            });
        }
    }

    fn main(&mut self) {
        let mut backoff = Duration::from_secs(1);
        let mut asked_corporate = false;
        loop {
            match self.load() {
                Ok(()) => {
                    // The answer came: any earlier error goes away now,
                    // not when the stream's first price arrives.
                    self.status(Status::Live);
                    if !asked_corporate {
                        asked_corporate = true;
                        self.corporate(self.cache.is_none());
                    }
                }
                Err(e) if self.bars.is_empty() => self.status(Status::Error(e.to_string())),
                Err(e) => self.status(Status::Offline(e.to_string())),
            }
            if !self.handle_cmds() {
                return;
            }
            let started = Instant::now();
            let mut live = false;
            let mut closed = false;
            // A second handle, so requests handled mid-stream (older
            // history) can still use the provider.
            let provider = self.provider.clone();
            let (symbol, base) = (self.symbol.clone(), self.base);
            let result = provider.stream(&symbol, base, &mut |item| {
                if let Live::Bar(b) = item {
                    if !live {
                        live = true;
                        self.status(Status::Live);
                    }
                    self.tick(b);
                }
                if !self.handle_cmds() {
                    closed = true;
                    return false;
                }
                true
            });
            if closed {
                if let Some(last) = self.bars.last().copied() {
                    self.save(&[last]);
                }
                return;
            }
            if started.elapsed() > Duration::from_secs(60) {
                backoff = Duration::from_secs(1);
            }
            let msg = match result {
                Err(e) => e.to_string(),
                Ok(()) => "The live connection closed.".into(),
            };
            self.status(Status::Offline(format!("{msg} Reconnecting…")));
            // Wait, still answering requests, then reconnect.
            let until = Instant::now() + backoff;
            while Instant::now() < until {
                if !self.handle_cmds() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    }

    fn tick(&mut self, b: Bar) {
        let prev = self.bars.last().copied();
        if self.upsert(b) {
            // A new bar started, so the previous one is final: keep it.
            if let Some(p) = prev {
                self.save(&[p]);
            }
        }
        let bar = if self.base == self.tf {
            Some(b)
        } else {
            resample::resample_last(&self.bars, self.tf, &self.tz)
        };
        if let Some(bar) = bar {
            self.out.send(Event::Tick { chart: self.chart, bar });
        }
    }
}

/// Sends quotes, and derives quotes for formulas (`BTCUSDT/ETHUSDT`) from
/// their legs' quotes.
struct Quoter {
    out: Out,
    exprs: Vec<(String, crate::expr::Expr, Vec<String>)>,
    last: HashMap<String, (f64, f64)>,
}

impl Quoter {
    fn send(&mut self, id: String, last: f64, prev: f64) {
        self.set(id.clone(), last, prev);
        self.formulas(&[id]);
    }

    /// A symbol's quote, without working out the formulas that use it (a
    /// batch does that once, at the end).
    fn set(&mut self, id: String, last: f64, prev: f64) {
        self.last.insert(id.clone(), (last, prev));
        self.out.send(Event::Quote { id, last, prev });
    }

    /// Quotes for the formulas (custom indices) using any of `changed`.
    fn formulas(&self, changed: &[String]) {
        for (eid, e, legs) in &self.exprs {
            if !legs.iter().any(|l| changed.contains(l)) {
                continue;
            }
            let qs: Option<Vec<(f64, f64)>> = legs.iter().map(|l| self.last.get(l).copied()).collect();
            if let Some(qs) = qs {
                let l: Vec<f64> = qs.iter().map(|q| q.0).collect();
                let p: Vec<f64> = qs.iter().map(|q| q.1).collect();
                self.out.send(Event::Quote {
                    id: eid.clone(),
                    last: e.eval(&l),
                    prev: e.eval(&p),
                });
            }
        }
    }
}

/// A Yahoo quote is asked for this often while its market trades (US
/// stocks: with pre-market and after hours)…
const QUOTE_EVERY: i64 = 30_000;
/// …and this often while it's shut, as a safety net (one more look right
/// after the close catches the final price).
const QUOTE_SHUT: i64 = 300_000;

/// The quote worker's Yahoo symbols, fetched together: spark answers
/// twenty a request and shares its replies with the market overview
/// (`overview::sparks`); a ticker spark leaves out is asked on its own.
struct YahooQuotes {
    syms: Vec<Symbol>,
    /// When each symbol was last asked for (ms).
    tried: HashMap<String, i64>,
    /// The newest spark used per ticker (fetched here or by the overview).
    seen: HashMap<String, Instant>,
    round: Option<Instant>,
}

impl YahooQuotes {
    fn due(&self, s: &Symbol, now: i64) -> bool {
        let Some(&at) = self.tried.get(&s.id) else {
            return true;
        };
        let m = opentern_core::calendar::Market::of(s);
        let age = now - at;
        age >= QUOTE_SHUT
            || (age >= QUOTE_EVERY
                && (opentern_core::calendar::trading(m, now) || opentern_core::calendar::closed_between(m, at, now)))
    }

    fn poll(&mut self, yahoo: &Yahoo, q: &std::cell::RefCell<Quoter>) {
        if self.round.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            return;
        }
        self.round = Some(Instant::now());
        let now = crate::provider::now_ms();
        let mut sent = Vec::new();
        // Sparks the market overview fetched since: used at once (and the
        // symbol's next turn counted from them), so the watchlist and the
        // map change together instead of up to 30 s apart.
        let routed: Vec<(Symbol, String)> = self
            .syms
            .iter()
            .filter(|s| yahoo_routed(s))
            .map(|s| (s.clone(), crate::yahoo::ticker(&s.id)))
            .filter(|(_, t)| !t.ends_with("=X"))
            .collect();
        let tickers: Vec<String> = routed.iter().map(|(_, t)| t.clone()).collect();
        let fresh = crate::overview::cached_sparks(&tickers);
        for (s, t) in &routed {
            let Some((at, sp)) = fresh.get(t) else { continue };
            if self.seen.get(t).is_some_and(|seen| seen >= at) {
                continue;
            }
            self.seen.insert(t.clone(), *at);
            self.tried.insert(s.id.clone(), now - at.elapsed().as_millis() as i64);
            if let Some((last, prev)) = sp.quote(matches!(s.exchange(), "NSE" | "BSE")) {
                q.borrow_mut().set(s.id.clone(), last, prev);
                sent.push(s.id.clone());
            }
            if opentern_core::calendar::has_extended_hours(s) {
                let ext = crate::yahoo::Ext::at(now, sp.price, sp.regular);
                q.borrow().out.send(Event::Extended { id: s.id.clone(), ext });
            }
        }
        // Symbols a keyed provider serves (keys can change while running)
        // are the worker's other loop's.
        let due: Vec<Symbol> = self
            .syms
            .iter()
            .filter(|s| yahoo_routed(s) && self.due(s, now))
            .cloned()
            .collect();
        if due.is_empty() {
            q.borrow().formulas(&sent);
            return;
        }
        let tickers: Vec<String> = due.iter().map(|s| crate::yahoo::ticker(&s.id)).collect();
        // Spark rounds currencies to four decimals (EURUSD's 1.099288
        // came as 1.0993): they're asked on their own.
        let batched: Vec<String> = tickers.iter().filter(|t| !t.ends_with("=X")).cloned().collect();
        let sparks = crate::overview::sparks(&batched, "5d");
        let got = Instant::now();
        for t in sparks.keys() {
            self.seen.insert(t.clone(), got);
        }
        for (s, t) in due.iter().zip(&tickers) {
            self.tried.insert(s.id.clone(), now);
            let spark = sparks.get(t);
            let quote = match spark {
                Some(sp) => sp.quote(matches!(s.exchange(), "NSE" | "BSE")),
                // Nothing came back at all: the network, not the ticker.
                None if sparks.is_empty() && !batched.is_empty() => None,
                None => yahoo.quote(s).ok(),
            };
            if let Some((last, prev)) = quote {
                q.borrow_mut().set(s.id.clone(), last, prev);
                sent.push(s.id.clone());
            }
            if opentern_core::calendar::has_extended_hours(s)
                && (spark.is_some() || opentern_core::calendar::us_extended(now).is_none())
            {
                let ext = spark.and_then(|sp| crate::yahoo::Ext::at(now, sp.price, sp.regular));
                q.borrow().out.send(Event::Extended { id: s.id.clone(), ext });
            }
        }
        q.borrow().formulas(&sent);
    }
}

fn quote_worker(symbols: Vec<Symbol>, demo: bool, cmds: Receiver<Cmd>, out: Out) {
    let stop = |cmds: &Receiver<Cmd>| cmds.try_iter().any(|c| matches!(c, Cmd::Stop));
    // Formulas quote through their legs.
    let mut exprs = Vec::new();
    let mut plain: Vec<Symbol> = Vec::new();
    for s in symbols {
        let formula = crate::expr::custom_formula(&s.id).unwrap_or_else(|| s.id.clone());
        match crate::expr::parse(&formula) {
            Some((e, legs)) if crate::expr::is_expr(&formula) => {
                for l in &legs {
                    if !plain.iter().any(|p| p.id == *l) {
                        plain.push(Symbol {
                            timezone: crate::expr::guess_tz(l),
                            ..Symbol::bare(l)
                        });
                    }
                }
                exprs.push((s.id.clone(), e, legs));
            }
            _ => {
                if !plain.iter().any(|p| p.id == s.id) {
                    plain.push(s);
                }
            }
        }
    }
    let q = std::cell::RefCell::new(Quoter {
        out,
        exprs,
        last: HashMap::new(),
    });
    // US stocks and funds: pre-market and after-hours prices beside the
    // regular ones.
    let us: Vec<(String, String)> = plain
        .iter()
        .filter(|s| opentern_core::calendar::has_extended_hours(s))
        .map(|s| (s.id.clone(), crate::yahoo::ticker(&s.id)))
        .collect();
    if demo {
        let now = crate::provider::now_ms();
        for s in &plain {
            let (last, prev) = demo::quote(s);
            q.borrow_mut().send(s.id.clone(), last, prev);
            if us.iter().any(|(id, _)| *id == s.id) {
                let ext = crate::yahoo::Ext::at(now, last * (1.0 + (last.fract() - 0.5) * 0.03), last);
                q.borrow().out.send(Event::Extended { id: s.id.clone(), ext });
            }
        }
        return;
    }
    // Binance spot: one combined stream. Perpetuals stream on their own.
    let (crypto, others): (Vec<_>, Vec<_>) = plain
        .into_iter()
        .partition(|s| s.id.starts_with("BINANCE:") && !crate::binance::is_perp(s));
    let pairs: Vec<String> = crypto.iter().map(|s| s.ticker().to_string()).collect();
    let mut yq = YahooQuotes {
        syms: others.clone(),
        tried: HashMap::new(),
        seen: HashMap::new(),
        round: None,
    };
    let yahoo = Yahoo::default();
    let (pair_tx, pair_rx) = unbounded();
    let mut pair_streams = PairStreams::default();
    let mut keyed_poll: Option<Instant> = None;
    let mut backoff = Duration::from_secs(1);
    loop {
        let mut poll_yahoo = |keyed_poll: &mut Option<Instant>| {
            yq.poll(&yahoo, &q);
            // Bybit, OKX, Coinbase and Kraken pairs: live, from their streams.
            let streamed: Vec<String> = pair_rx
                .try_iter()
                .map(|(id, last, prev): (String, f64, f64)| {
                    q.borrow_mut().set(id.clone(), last, prev);
                    id
                })
                .collect();
            if !streamed.is_empty() {
                q.borrow().formulas(&streamed);
            }
            // Keyed providers' symbols (Alpaca, Upstox, …): two daily bars
            // every 30 seconds.
            if keyed_poll.is_none_or(|t| t.elapsed() > Duration::from_secs(30)) {
                *keyed_poll = Some(Instant::now());
                for s in others.iter().filter(|s| streams_quotes(s)) {
                    pair_streams.start(s, &pair_tx);
                }
                for s in others.iter().filter(|s| !yahoo_routed(s) && !streams_quotes(s)) {
                    if let Some((last, prev)) = daily_quote(s) {
                        q.borrow_mut().send(s.id.clone(), last, prev);
                    }
                }
            }
        };
        if pairs.is_empty() {
            poll_yahoo(&mut keyed_poll);
            for _ in 0..10 {
                if stop(&cmds) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            continue;
        }
        let mut stopped = false;
        let started = Instant::now();
        let _ = binance::stream_quotes(&pairs, &mut |item| {
            if let Some((pair, last, open)) = item {
                q.borrow_mut().send(format!("BINANCE:{pair}"), last, open);
            }
            poll_yahoo(&mut keyed_poll);
            if stop(&cmds) {
                stopped = true;
                return false;
            }
            true
        });
        if stopped {
            return;
        }
        if started.elapsed() > Duration::from_secs(60) {
            backoff = Duration::from_secs(1);
        }
        // Stocks keep their quotes while Binance is unreachable (they used
        // to freeze until it answered again).
        poll_yahoo(&mut keyed_poll);
        let until = Instant::now() + backoff;
        while Instant::now() < until {
            if stop(&cmds) {
                return;
            }
            poll_yahoo(&mut keyed_poll);
            std::thread::sleep(Duration::from_millis(200));
        }
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

/// Ranks a symbol against a query: lower is better, None is no match. A
/// crypto pair whose base asset is the query ("ETH" → ETHUSDT) ranks just
/// after an exact ticker, with USDT pairs first.
fn rank(s: &Symbol, q: &str) -> Option<u32> {
    let full = s.ticker().to_uppercase();
    let mut t = full.clone();
    // A Hyperliquid builder dex's market ranks by its own name: "GOLD"
    // finds XYZ_GOLDUSDC.P as it finds GOLDUSDT.
    if s.exchange() == "HYPERLIQUID"
        && let Some((_, rest)) = t.split_once('_')
    {
        t = rest.to_string();
    }
    let d = s.description.to_uppercase();
    if t == q {
        return Some(0);
    }
    // Other names for it ("fed funds" → EFFR).
    let aliases = opentern_core::search_aliases(&s.id);
    if aliases.contains(&q) {
        return Some(1);
    }
    if q.len() >= 3 && aliases.iter().any(|a| a.starts_with(q)) {
        return Some(30);
    }
    if s.id.starts_with("BINANCE:")
        && let Some(quote) = t.strip_prefix(q)
    {
        let pref = ["USDT", "USDC", "FDUSD", "BTC", "ETH", "EUR", "TRY"];
        if let Some(i) = pref.iter().position(|p| *p == quote) {
            return Some(1 + i as u32);
        }
    }
    let score = if t.starts_with(q) {
        20 + (t.len() - q.len()) as u32
    } else if full != t && full.starts_with(q) {
        20 + (full.len() - q.len()) as u32
    } else if let Some(r) = coin_name_rank(s, &d, q) {
        r
    } else if d.starts_with(q) {
        50
    } else if word_starts(&d, q) {
        55
    } else if t.contains(q) {
        70
    } else if d.contains(q) {
        90
    } else {
        return loose_rank(s, &t, &d, q).or_else(|| typo_rank(&t, &d, q));
    };
    Some(score)
}

/// A word of `d` (after a space, a dash or a bracket) starts with `q`:
/// "MAHINDRA" in "KOTAK MAHINDRA BANK".
fn word_starts(d: &str, q: &str) -> bool {
    d.match_indices(q)
        .any(|(i, _)| i == 0 || d[..i].ends_with(|c: char| !c.is_alphanumeric()))
}

/// A coin found by its name ("BITCOIN", "SOL" → "SOLANA" too): the
/// exchanges describe pairs as "BTC / USDT", without it. Ranked like a
/// ticker match, preferring the same quotes.
fn coin_name_rank(s: &Symbol, d: &str, q: &str) -> Option<u32> {
    if s.kind != opentern_core::AssetKind::Crypto || q.len() < 3 {
        return None;
    }
    let (base, quote) = d.split_once(" / ")?;
    // "BTC / USDT" from the exchanges, "Bitcoin / TetherUS" in the starter
    // list (TradingView's wording): either names the coin.
    let base = base.trim();
    let (_, name) = crate::feeds::COINS
        .iter()
        .find(|(b, n)| *b == base || n.to_uppercase() == base)?;
    let name = name.to_uppercase();
    if !(name.starts_with(q) || compact(&name).starts_with(&compact(q))) {
        return None;
    }
    let pref = ["USDT", "USDC", "FDUSD", "BTC", "ETH", "EUR", "TRY"];
    let quote = match quote.trim().trim_end_matches(" PERPETUAL") {
        "TETHERUS" => "USDT",
        q => q,
    };
    // The whole name first: "BITCOIN" is BTC before Bitcoin Cash.
    let whole = if compact(&name) == compact(q) { 30 } else { 45 };
    Some(whole + pref.iter().position(|p| *p == quote).unwrap_or(9) as u32 + 10 * u32::from(s.ticker().ends_with(".P")))
}

/// A query one or two letters off a ticker or a word of the name
/// ("RELAINCE", "TESLE"): ranked after every real match. One edit for
/// short queries, two from seven letters; the start of a word is
/// compared too, so it works while still typing ("RELAIN").
fn typo_rank(t: &str, d: &str, q: &str) -> Option<u32> {
    let n = q.chars().count();
    if n < 4 || !q.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let most = if n >= 7 { 2 } else { 1 };
    let close = |w: &str| {
        let w: Vec<char> = w.chars().collect();
        if w.len() + most < n {
            return false;
        }
        let whole = edits(&w, q, most) <= most;
        whole || (w.len() > n && edits(&w[..n], q, most) <= most)
    };
    if close(&compact(t)) {
        return Some(97);
    }
    d.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 4)
        .any(close)
        .then_some(99)
}

/// Edits (insert, delete, change, swap two neighbours) from `a` to `b`,
/// giving up past `most`.
fn edits(a: &[char], b: &str, most: usize) -> usize {
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > most {
        return most + 1;
    }
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=n {
        let mut low = usize::MAX;
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
            low = low.min(v);
        }
        if low > most {
            return most + 1;
        }
    }
    d[n][m]
}

/// Letters and digits only: "S&P 500" and "SP500" read the same.
fn compact(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// A query that doesn't match as a whole: every word of it found in the
/// ticker, the name or the exchange ("NSE VIX" → India VIX, "nifty bank",
/// "apple nasdaq"), or the query with spaces and punctuation dropped
/// ("SP 500"). Ranks after whole matches.
fn loose_rank(s: &Symbol, t: &str, d: &str, q: &str) -> Option<u32> {
    let ex = s.exchange().to_uppercase();
    let (ct, cd) = (compact(t), compact(d));
    let cq = compact(q);
    if !cq.is_empty() && (ct.contains(&cq) || cd.contains(&cq)) {
        return Some(if ct.starts_with(&cq) || cd.starts_with(&cq) {
            92
        } else {
            95
        });
    }
    let words: Vec<String> = q.split_whitespace().map(compact).filter(|w| !w.is_empty()).collect();
    if words.len() < 2 {
        return None;
    }
    let mut on_ticker = false;
    for w in &words {
        let in_ticker = ct.contains(w.as_str());
        let found = in_ticker || cd.contains(w.as_str()) || ex == *w || (w.len() >= 3 && ex.starts_with(w.as_str()));
        if !found {
            return None;
        }
        on_ticker |= in_ticker;
    }
    Some(if on_ticker { 96 } else { 98 })
}

/// Searches numbered from here are each answered, even when newer ones are
/// waiting (the agent's lookups); below it only the latest is (typing).
pub const SEARCH_ALL: u64 = 1 << 40;

/// The exchanges' symbol lists are fetched again after a day; sooner
/// (but not more than every half hour) once a search found a new coin.
const LISTS_DAY: i64 = 86_400_000;
const LISTS_SOON: i64 = 1_800_000;

/// Binance pairs worth asking about when a search found no coin for `q`:
/// the query as a pair, and its USDT pair. Nothing for queries that can't
/// be a ticker or that already matched a coin.
pub(crate) fn coin_lookups(q: &str, hits: &[&Symbol]) -> Vec<String> {
    if q.len() < 2 || q.len() > 20 || !q.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Vec::new();
    }
    let coin = |s: &&&Symbol| s.kind == opentern_core::AssetKind::Crypto;
    if hits.iter().filter(coin).any(|s| s.ticker().starts_with(q)) {
        return Vec::new();
    }
    let quoted = ["USDT", "USDC", "FDUSD", "BTC", "ETH", "BNB", "TRY", "EUR"]
        .iter()
        .any(|x| q.len() > x.len() && q.ends_with(x));
    if quoted {
        vec![q.to_string()]
    } else {
        vec![format!("{q}USDT")]
    }
}

/// The Hyperliquid perpetuals that follow a US share or fund
/// (`NASDAQ:TSLA` → `HYPERLIQUID:XYZ_TSLAUSDC.P`).
fn hyperliquid_of(known: &[Symbol], s: &Symbol) -> Vec<Symbol> {
    use opentern_core::AssetKind;
    if !matches!(s.kind, AssetKind::Stock | AssetKind::Fund)
        || !matches!(s.exchange(), "NASDAQ" | "NYSE" | "AMEX" | "ARCA" | "BATS")
    {
        return Vec::new();
    }
    known
        .iter()
        .filter(|k| crate::hyperliquid::underlying(k) == Some(s.ticker()))
        .cloned()
        .collect()
}

/// A company in the results ("tesla" finds NASDAQ:TSLA) brings its
/// Hyperliquid perpetuals, ranked as it is so they sit right after it.
fn with_hyperliquid(known: &[Symbol], hits: &mut Vec<(u32, Symbol)>) {
    let lifts: Vec<(u32, Symbol)> = hits
        .iter()
        .flat_map(|(r, s)| hyperliquid_of(known, s).into_iter().map(move |p| (*r, p)))
        .collect();
    for (r, p) in lifts {
        // Already found by a looser match: moved up beside its company.
        match hits.iter_mut().find(|(_, h)| h.id == p.id) {
            Some(hit) => hit.0 = hit.0.min(r),
            None => hits.push((r, p)),
        }
    }
}

/// How many symbols an exchange's prefix alone lists ("BINANCE:").
const BROWSE: usize = 500;

/// The coins listed first when browsing an exchange, biggest first.
const MAJORS: [&str; 24] = [
    "BTC", "ETH", "XRP", "BNB", "SOL", "DOGE", "TRX", "ADA", "HYPE", "LINK", "AVAX", "SUI", "XLM", "BCH", "TON", "LTC",
    "DOT", "SHIB", "UNI", "NEAR", "APT", "AAVE", "PEPE", "ETC",
];

/// An exchange's known symbols for its prefix alone: the major coins
/// first (each one's USDT, USDC, then USD pair, spot before perpetual),
/// then the rest by ticker.
fn browse(known: &[Symbol], exchange: &str) -> Vec<Symbol> {
    let quotes = ["USDT", "USDC", "USD"];
    let key = |s: &Symbol| {
        let t = s.ticker().to_uppercase();
        let perp = t.ends_with(".P");
        let pair = t.trim_end_matches(".P");
        let (base, q) = quotes
            .iter()
            .enumerate()
            .find_map(|(i, q)| {
                pair.strip_suffix(q)
                    .filter(|b| !b.is_empty())
                    .map(|b| (b.to_string(), i))
            })
            .unwrap_or_else(|| (pair.to_string(), quotes.len()));
        // Hyperliquid's thousand-unit coins (kPEPE) rank as their coin.
        let base = base
            .strip_prefix('K')
            .filter(|b| MAJORS.contains(b))
            .map_or(base.clone(), str::to_string);
        let major = MAJORS.iter().position(|m| *m == base).unwrap_or(MAJORS.len());
        (major, q, perp, t)
    };
    let mut list: Vec<(_, Symbol)> = known
        .iter()
        .filter(|s| s.exchange() == exchange)
        .map(|s| (key(s), s.clone()))
        .collect();
    list.sort_by(|a, b| a.0.cmp(&b.0));
    list.into_iter().take(BROWSE).map(|(_, s)| s).collect()
}

fn search_worker(dir: PathBuf, demo: bool, rx: Receiver<(u64, String)>, out: Out) {
    let cache = Cache::open(&dir).ok();
    // The exchanges' symbol lists refresh daily on their own thread, so a
    // search never waits for five downloads (OKX alone can time out). A
    // search that finds no coin also asks Binance about it right away, so
    // a new listing shows up the day it lists.
    let (fresh_tx, fresh_rx) = crossbeam_channel::bounded::<()>(1);
    let refreshing = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let refresh = |cache: &Option<Cache>, within: i64| {
        let stale = cache.as_ref().is_some_and(|c| {
            c.meta(LISTS_AT)
                .and_then(|t| t.parse::<i64>().ok())
                .is_none_or(|t| crate::provider::now_ms() - t > within)
        });
        if !stale || demo || refreshing.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let dir = dir.clone();
        let fresh_tx = fresh_tx.clone();
        let refreshing = refreshing.clone();
        let _ = std::thread::Builder::new().name("symbol-lists".into()).spawn(move || {
            if let Ok(c) = Cache::open(&dir) {
                let lists: [(&str, crate::provider::Result<Vec<Symbol>>); 9] = [
                    ("binance", Binance::default().symbols()),
                    ("bybit", crate::bybit::Bybit::default().symbols()),
                    ("okx", crate::okx::Okx::default().symbols()),
                    ("coinbase", crate::coinbase::Coinbase::default().symbols()),
                    ("kraken", crate::kraken::Kraken::default().symbols()),
                    ("binance_perp", Binance::perp().perps()),
                    ("bybit_perp", crate::bybit::Bybit::perp().perps()),
                    ("okx_perp", crate::okx::Okx::default().perps()),
                    ("hyperliquid_perp", crate::hyperliquid::Hyperliquid::default().perps()),
                ];
                let mut any = false;
                for (source, list) in lists {
                    if let Ok(list) = list {
                        let _ = c.save_symbols(source, &list);
                        any = true;
                    }
                }
                if any {
                    let _ = c.set_meta(LISTS_AT, &crate::provider::now_ms().to_string());
                    let _ = fresh_tx.try_send(());
                }
            }
            refreshing.store(false, std::sync::atomic::Ordering::SeqCst);
        });
    };
    refresh(&cache, LISTS_DAY);
    let mut checked = std::time::Instant::now();
    // Coins asked of Binance that it didn't have, and when.
    let mut missed: HashMap<String, std::time::Instant> = HashMap::new();
    let load = |cache: &Option<Cache>| {
        let mut known: Vec<Symbol> = opentern_core::starter_watchlist();
        known.extend(opentern_core::commodities());
        known.extend(opentern_core::spot_commodities());
        known.extend(opentern_core::futures());
        known.extend(opentern_core::indices());
        known.extend(crate::overview::listed_stocks());
        if let Some(c) = cache {
            known.extend(c.symbols().unwrap_or_default());
        }
        known.sort_by(|a, b| a.id.cmp(&b.id));
        known.dedup_by(|a, b| a.id == b.id);
        known
    };
    let mut known = load(&cache);
    // Results from the symbols already known go out at once (`done:
    // false`); Binance lookups and Yahoo's search run on their own thread
    // and their merged answer follows (`done: true`). One typed search is
    // asked of the network at a time; while it runs, only the latest
    // query waits for its turn, so typing never queues up round trips.
    struct Job {
        seq: u64,
        q: String,
        hits: Vec<(u32, Symbol)>,
        coins: Vec<(String, Option<Symbol>)>,
        found: Option<Vec<Symbol>>,
    }
    let (net_tx, net_rx) = crossbeam_channel::unbounded::<Job>();
    let start = |seq: u64, q: String, want_ex: Option<String>, hits: Vec<(u32, Symbol)>, asks: Vec<String>| {
        let net_tx = net_tx.clone();
        let _ = std::thread::Builder::new().name("symbol-search".into()).spawn(move || {
            let (coins, found) = std::thread::scope(|s| {
                let coins = (!asks.is_empty()).then(|| {
                    s.spawn(|| {
                        asks.iter()
                            .map(|t| (t.clone(), Binance::default().symbol(t)))
                            .collect::<Vec<_>>()
                    })
                });
                let found = if !demo && want_ex.is_none() {
                    Yahoo::default().search(&q).ok()
                } else {
                    None
                };
                (coins.and_then(|h| h.join().ok()).unwrap_or_default(), found)
            });
            let _ = net_tx.send(Job {
                seq,
                q,
                hits,
                coins,
                found,
            });
        });
    };
    let sorted = |mut hits: Vec<(u32, Symbol)>| -> Vec<Symbol> {
        hits.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(exchange_rank(a.1.exchange()).cmp(&exchange_rank(b.1.exchange())))
                .then(a.1.id.len().cmp(&b.1.id.len()))
        });
        hits.into_iter().take(60).map(|(_, s)| s).collect()
    };
    type Waiting = (u64, String, Option<String>, Vec<(u32, Symbol)>, Vec<String>);
    // A typed search's network part is running; the latest one waiting.
    let mut busy = false;
    let mut waiting: Option<Waiting> = None;
    let mut queue: std::collections::VecDeque<(u64, String)> = Default::default();
    loop {
        crossbeam_channel::select! {
            recv(rx) -> r => {
                let Ok(r) = r else { break };
                queue.push_back(r);
            }
            recv(net_rx) -> j => {
                let Ok(j) = j else { continue };
                let Job { seq, q, mut hits, coins, found } = j;
                let mut new_coin = false;
                for (t, got) in coins {
                    match got {
                        Some(s) => {
                            new_coin = true;
                            if let Some(r) = rank(&s, &q) {
                                hits.push((r, s.clone()));
                            }
                            known.push(s);
                        }
                        None => {
                            missed.insert(t, std::time::Instant::now());
                        }
                    }
                }
                if new_coin {
                    // The other exchanges may list it too: fetch their lists again.
                    refresh(&cache, LISTS_SOON);
                }
                if let Some(found) = found {
                    for s in found {
                        if !hits.iter().any(|(_, h)| h.id == s.id)
                            && let Some(r) = rank(&s, &q).or(Some(80))
                        {
                            hits.push((r, s));
                        }
                    }
                }
                with_hyperliquid(&known, &mut hits);
                out.send(Event::Search { seq, results: sorted(hits), done: true });
                if seq < SEARCH_ALL {
                    busy = false;
                    if let Some((seq, q, ex, hits, asks)) = waiting.take() {
                        busy = true;
                        start(seq, q, ex, hits, asks);
                    }
                }
                continue;
            }
        }
        // Typing in the search box: only the latest query matters. Lookups
        // numbered from `SEARCH_ALL` (the agent's) all get answers.
        queue.extend(rx.try_iter());
        if let Some(last) = queue.iter().rposition(|(s, _)| *s < SEARCH_ALL) {
            let mut k = 0;
            queue.retain(|(s, _)| {
                k += 1;
                *s >= SEARCH_ALL || k - 1 == last
            });
        }
        while let Some(req) = queue.pop_front() {
            // A window left open for days still gets each day's lists.
            if checked.elapsed() > std::time::Duration::from_secs(600) {
                checked = std::time::Instant::now();
                refresh(&cache, LISTS_DAY);
            }
            if fresh_rx.try_recv().is_ok() {
                known = load(&cache);
            }
            let (seq, query) = req;
            if crate::expr::is_expr(&query) {
                // A formula: offer it as the first result.
                let results = crate::expr::symbol(&query, &known).into_iter().collect();
                out.send(Event::Search {
                    seq,
                    results,
                    done: true,
                });
                continue;
            }
            let q = query.trim().to_uppercase();
            // "OKX:BTC" searches one exchange.
            let (want_ex, q) = match q.split_once(':') {
                Some((e, t)) => (Some(e.trim().to_string()), t.trim().to_string()),
                None => (None, q),
            };
            if q.is_empty() {
                // "HYPERLIQUID:" alone lists that exchange's symbols.
                let results = match &want_ex {
                    Some(e) => browse(&known, e),
                    None => opentern_core::starter_watchlist(),
                };
                out.send(Event::Search {
                    seq,
                    results,
                    done: true,
                });
                continue;
            }
            // OANDA's spot commodities only once there's a key to chart them.
            let oanda = creds().read().unwrap_or_else(|e| e.into_inner()).oanda.is_some();
            let mut hits: Vec<(u32, Symbol)> = known
                .iter()
                .filter(|s| want_ex.as_deref().is_none_or(|e| s.exchange() == e))
                .filter(|s| oanda || s.exchange() != "OANDA")
                .filter_map(|s| Some((rank(s, &q)?, s.clone())))
                .collect();
            // Sources without a symbol list: take "OANDA:EURUSD" as typed.
            if let Some(e) = want_ex.as_deref()
                && matches!(e, "OANDA" | "IBKR")
                && !hits.iter().any(|(_, h)| h.ticker() == q)
            {
                let forex = e == "OANDA";
                let spot = opentern_core::spot_commodities().into_iter().find(|s| s.ticker() == q);
                hits.push((
                    0,
                    spot.unwrap_or_else(|| Symbol {
                        id: format!("{e}:{q}"),
                        description: if forex {
                            "OANDA".into()
                        } else {
                            "Interactive Brokers".into()
                        },
                        kind: if forex {
                            opentern_core::AssetKind::Forex
                        } else {
                            opentern_core::AssetKind::Stock
                        },
                        currency: if forex {
                            q.get(3..).unwrap_or("USD").into()
                        } else {
                            "USD".into()
                        },
                        tick_size: if forex { 0.00001 } else { 0.01 },
                        min_qty: 0.0,
                        timezone: if forex { "UTC".into() } else { "America/New_York".into() },
                    }),
                ));
            }
            // No coin by that name yet: perhaps it listed today.
            let asks: Vec<String> = if !demo && want_ex.as_deref().is_none_or(|e| e == "BINANCE") {
                coin_lookups(&q, &hits.iter().map(|(_, s)| s).collect::<Vec<_>>())
                    .into_iter()
                    .filter(|t| {
                        missed
                            .get(t)
                            .is_none_or(|at| at.elapsed() > std::time::Duration::from_secs(600))
                    })
                    .collect()
            } else {
                Vec::new()
            };
            if want_ex.is_none() {
                with_hyperliquid(&known, &mut hits);
            }
            let network = !asks.is_empty() || (!demo && want_ex.is_none());
            out.send(Event::Search {
                seq,
                results: sorted(hits.clone()),
                done: !network,
            });
            if !network {
                continue;
            }
            if seq >= SEARCH_ALL {
                start(seq, q, want_ex, hits, asks);
            } else if busy {
                waiting = Some((seq, q, want_ex, hits, asks));
            } else {
                busy = true;
                start(seq, q, want_ex, hits, asks);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(id: &str) -> Symbol {
        let (_, t) = id.split_once(':').unwrap();
        let (b, q) = crate::provider::split_pair(t).unwrap();
        Symbol {
            id: id.into(),
            description: format!("{b} / {q}"),
            kind: opentern_core::AssetKind::Crypto,
            currency: q.into(),
            tick_size: 0.01,
            min_qty: 0.0,
            timezone: "UTC".into(),
        }
    }

    #[test]
    fn a_delisted_pair_carries_on_where_the_coin_still_trades() {
        let xmr = pair("BINANCE:XMRUSDT");
        let known = vec![pair("BINANCE:BTCUSDT"), pair("KRAKEN:XMRUSD"), pair("KRAKEN:XMREUR")];
        // Binance's list no longer has it: Kraken's USD pair.
        assert_eq!(successor(&known, &xmr).map(|s| s.id), Some("KRAKEN:XMRUSD".into()));
        // Still listed: stays.
        assert_eq!(successor(&known, &pair("BINANCE:BTCUSDT")), None);
        // Binance's list not in yet: no guessing.
        let no_binance = vec![pair("KRAKEN:XMRUSD")];
        assert_eq!(successor(&no_binance, &xmr), None);
        // Nowhere else: stays (and shows it's stale).
        assert_eq!(successor(&[pair("BINANCE:BTCUSDT")], &xmr), None);
        // Not crypto: never.
        let mut stock = pair("BINANCE:XMRUSDT");
        stock.kind = opentern_core::AssetKind::Stock;
        assert_eq!(successor(&known, &stock), None);
    }

    /// Real Yahoo: a cache holding pre-split prices (HDFC Bank's 1:1 bonus,
    /// Aug 2025, undone by doubling what's cached) is replaced.
    #[test]
    #[ignore]
    fn live_cache_from_before_a_bonus_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let sym = Symbol {
            timezone: "Asia/Kolkata".into(),
            ..Symbol::bare("NSE:HDFCBANK")
        };
        let tf = Timeframe::parse("1D").unwrap();
        let first = fetch_once(dir.path(), false, &sym, tf, 300).unwrap();
        let c = Cache::open(dir.path()).unwrap();
        let mut stale = c.last(&sym.id, tf, 10_000).unwrap();
        stale.pop();
        for b in &mut stale {
            b.close *= 2.0;
        }
        c.save(&sym.id, tf, &stale).unwrap();
        let again = fetch_once(dir.path(), false, &sym, tf, 300).unwrap();
        let (a, b) = (first[first.len() - 30].close, again[again.len() - 30].close);
        assert!((a / b - 1.0).abs() < 0.01, "{a} vs {b}");
    }

    /// Real Yahoo: Bajaj Finance's hourly bars jump ×5 on 16 Jun 2025 (a
    /// split plus a bonus listed as "2:1"); they come out continuous.
    #[test]
    #[ignore]
    fn live_intraday_split_and_bonus_is_adjusted() {
        let dir = tempfile::tempdir().unwrap();
        let sym = Symbol {
            timezone: "Asia/Kolkata".into(),
            ..Symbol::bare("NSE:BAJFINANCE")
        };
        let bars = fetch_once(dir.path(), false, &sym, Timeframe::parse("1h").unwrap(), 5000).unwrap();
        let worst = bars
            .windows(2)
            .map(|w| (w[1].open / w[0].close).ln().abs())
            .fold(0.0, f64::max);
        assert!(
            bars.len() > 3000 && worst < 0.2,
            "{} bars, worst gap {worst}",
            bars.len()
        );
    }

    #[test]
    fn a_split_adjusted_in_after_caching_is_noticed() {
        let bar = |t: i64, c: f64| Bar {
            time: t,
            open: c,
            high: c,
            low: c,
            close: c,
            volume: 1.0,
        };
        let cached = vec![bar(1, 200.0), bar(2, 202.0), bar(3, 204.0), bar(4, 206.0)];
        assert_eq!(overlap_start(&cached), 2);
        // Same history: the newest bar was still forming when cached.
        assert!(!history_changed(
            &cached,
            &[bar(2, 202.0), bar(3, 204.0), bar(4, 230.0), bar(5, 210.0)]
        ));
        // The provider halved its history for a 2-for-1 split.
        assert!(history_changed(
            &cached,
            &[bar(2, 101.0), bar(3, 102.0), bar(4, 103.0), bar(5, 104.0)]
        ));
    }

    #[test]
    fn a_coin_search_with_no_match_asks_binance() {
        let coin = |id: &str| Symbol {
            id: id.into(),
            description: String::new(),
            kind: opentern_core::AssetKind::Crypto,
            currency: "USDT".into(),
            tick_size: 0.01,
            min_qty: 0.0,
            timezone: "UTC".into(),
        };
        assert_eq!(coin_lookups("NEWCOIN", &[]), vec!["NEWCOINUSDT".to_string()]);
        assert_eq!(coin_lookups("NEWCOINUSDT", &[]), vec!["NEWCOINUSDT".to_string()]);
        assert_eq!(coin_lookups("NEWBTC", &[]), vec!["NEWBTC".to_string()]);
        let btc = coin("BINANCE:BTCUSDT");
        assert!(coin_lookups("BTC", &[&btc]).is_empty());
        assert!(coin_lookups("BTCUSDT", &[&btc]).is_empty());
        assert!(coin_lookups("M&M", &[]).is_empty());
        assert!(coin_lookups("A", &[]).is_empty());
        // A stock with the same letters doesn't count as the coin.
        let mut stock = coin("NASDAQ:NEWC");
        stock.kind = opentern_core::AssetKind::Stock;
        assert_eq!(coin_lookups("NEWC", &[&stock]), vec!["NEWCUSDT".to_string()]);
    }

    /// A company the stock search finds by name brings its Hyperliquid
    /// perpetuals ("tesla" → NASDAQ:TSLA, then XYZ_TSLAUSDC.P); SpaceX,
    /// with no listing, is found by the name Hyperliquid's list carries.
    #[test]
    fn searching_a_company_finds_its_hyperliquid_perpetual() {
        let known = vec![
            Symbol::bare("HYPERLIQUID:XYZ_TSLAUSDC.P"),
            Symbol::bare("HYPERLIQUID:IO_NVDAUSDC.P"),
            Symbol::bare("HYPERLIQUID:BTCUSDC.P"),
            Symbol {
                description: "SpaceX / USDC Perpetual (XYZ)".into(),
                ..Symbol::bare("HYPERLIQUID:XYZ_SPCXUSDC.P")
            },
        ];
        let stock = |id: &str| Symbol {
            kind: opentern_core::AssetKind::Stock,
            ..Symbol::bare(id)
        };
        let ids = |v: Vec<Symbol>| v.into_iter().map(|s| s.id).collect::<Vec<_>>();
        assert_eq!(
            ids(hyperliquid_of(&known, &stock("NASDAQ:TSLA"))),
            ["HYPERLIQUID:XYZ_TSLAUSDC.P"]
        );
        assert_eq!(
            ids(hyperliquid_of(&known, &stock("NASDAQ:NVDA"))),
            ["HYPERLIQUID:IO_NVDAUSDC.P"]
        );
        assert!(hyperliquid_of(&known, &stock("NSE:TSLA")).is_empty());
        // Wherever the company came from, its perpetual sits right after it.
        let mut hits = vec![(20, stock("NASDAQ:TSLA")), (30, stock("NASDAQ:AAPL"))];
        with_hyperliquid(&known, &mut hits);
        assert_eq!(
            hits.last().map(|(r, s)| (*r, s.id.as_str())),
            Some((20, "HYPERLIQUID:XYZ_TSLAUSDC.P"))
        );
        with_hyperliquid(&known, &mut hits);
        assert_eq!(hits.len(), 3, "added once");
        // Found already by a loose match: moved up beside the company.
        let mut hits = vec![(99, known[0].clone()), (50, stock("NASDAQ:TSLA"))];
        with_hyperliquid(&known, &mut hits);
        assert_eq!(hits[0].0, 50);
        assert!(rank(&known[3], "SPACEX").is_some());
    }

    /// An exchange's prefix alone lists that exchange only, the major
    /// coins first (spot, then perpetual), the rest by ticker.
    #[test]
    fn an_exchange_prefix_alone_lists_its_symbols() {
        let ids = [
            "HYPERLIQUID:XYZ_GOLDUSDC.P",
            "HYPERLIQUID:ATOMUSDC.P",
            "HYPERLIQUID:kPEPEUSDC.P",
            "HYPERLIQUID:ETHUSDC.P",
            "HYPERLIQUID:BTCUSDC.P",
            "BINANCE:BTCUSDT.P",
            "BINANCE:ETHBTC",
            "BINANCE:BTCUSDT",
            "BINANCE:ETHUSDT",
            "OKX:BTCUSDT",
        ];
        let known: Vec<Symbol> = ids.iter().map(|id| Symbol::bare(id)).collect();
        let got = |e: &str| browse(&known, e).into_iter().map(|s| s.id).collect::<Vec<_>>();
        assert_eq!(
            got("HYPERLIQUID"),
            [
                "HYPERLIQUID:BTCUSDC.P",
                "HYPERLIQUID:ETHUSDC.P",
                "HYPERLIQUID:kPEPEUSDC.P",
                "HYPERLIQUID:ATOMUSDC.P",
                "HYPERLIQUID:XYZ_GOLDUSDC.P",
            ]
        );
        assert_eq!(
            got("BINANCE"),
            [
                "BINANCE:BTCUSDT",
                "BINANCE:BTCUSDT.P",
                "BINANCE:ETHUSDT",
                "BINANCE:ETHBTC"
            ]
        );
        assert!(got("KRAKEN").is_empty());
    }

    /// Hyperliquid's builder-dex markets are found by the asset's name,
    /// by the forex pair as people type it, and by their full ticker.
    #[test]
    fn search_finds_hyperliquid_builder_markets() {
        let mk = |id: &str, d: &str| Symbol {
            description: d.into(),
            ..Symbol::bare(id)
        };
        let gold = mk("HYPERLIQUID:XYZ_GOLDUSDC.P", "GOLD / USDC Perpetual (XYZ)");
        let eur = mk("HYPERLIQUID:XYZ_EURUSDC.P", "EUR / USDC Perpetual (XYZ)");
        let sp = mk("HYPERLIQUID:XYZ_SP500USDC.P", "SP500 / USDC Perpetual (XYZ)");
        let crude = mk("HYPERLIQUID:XYZ_BRENTOILUSDC.P", "BRENTOIL / USDC Perpetual (XYZ)");
        assert!(rank(&gold, "GOLD").unwrap() < 30, "a prefix match");
        assert!(rank(&sp, "SP500").unwrap() < 30);
        assert!(rank(&crude, "BRENT").unwrap() < 30);
        assert!(rank(&eur, "EURUSD").unwrap() < 30);
        assert!(rank(&gold, "XYZ_GOLD").unwrap() < 30);
        assert_eq!(rank(&gold, "SILVER"), None);
    }

    #[test]
    fn search_ranks_exact_then_prefix() {
        let mk = |id: &str, d: &str| Symbol {
            description: d.into(),
            ..Symbol::bare(id)
        };
        let btc = mk("BINANCE:BTCUSDT", "BTC / USDT");
        let btcfdusd = mk("BINANCE:BTCFDUSD", "BTC / FDUSD");
        let aapl = mk("NASDAQ:AAPL", "Apple Inc.");
        assert!(rank(&btc, "BTC").unwrap() < rank(&btcfdusd, "BTC").unwrap());
        assert_eq!(rank(&aapl, "AAPL"), Some(0));
        assert!(rank(&aapl, "APPLE").is_some());
        assert_eq!(rank(&aapl, "MSFT"), None);
        // Words in any order, across ticker, name and exchange.
        let vix = mk("NSE:INDIAVIX", "India VIX");
        assert!(rank(&vix, "NSE VIX").is_some());
        assert!(rank(&vix, "VIX INDIA").is_some());
        assert!(rank(&vix, "INDIA VIX").is_some());
        assert_eq!(rank(&vix, "NSE APPLE"), None);
        assert!(rank(&aapl, "APPLE NASDAQ").is_some());
        let spx = mk("SP:SPX", "S&P 500");
        assert!(rank(&spx, "SP500").is_some());
        // The Fed funds rate by its other names.
        let effr = opentern_core::fed_funds();
        for q in [
            "EFFR",
            "FED FUNDS",
            "FEDFUNDS",
            "INTEREST RATE",
            "FED FU",
            "FEDERAL FUNDS",
        ] {
            assert!(rank(&effr, q).is_some_and(|r| r <= 55), "{q}: {:?}", rank(&effr, q));
        }
        assert_eq!(rank(&aapl, "FED FUNDS"), None);
    }

    #[test]
    fn search_finds_names_words_and_typos() {
        let mk = |id: &str, d: &str, kind| Symbol {
            description: d.into(),
            kind,
            ..Symbol::bare(id)
        };
        use opentern_core::AssetKind::{Crypto, Stock};
        let btc = mk("BINANCE:BTCUSDT", "BTC / USDT", Crypto);
        let btceur = mk("BINANCE:BTCEUR", "BTC / EUR", Crypto);
        let perp = mk("BYBIT:BTCUSDT.P", "BTC / USDT Perpetual", Crypto);
        let bch = mk("BINANCE:BCHUSDT", "BCH / USDT", Crypto);
        // Coins by name, the USDT pair first, spot before perpetuals.
        let r = |s: &Symbol, q: &str| rank(s, q).unwrap_or(u32::MAX);
        assert!(r(&btc, "BITCOIN") < r(&btceur, "BITCOIN"));
        assert!(r(&btc, "BITCOIN") < r(&perp, "BITCOIN"));
        assert!(r(&btc, "BITCOIN") < r(&bch, "BITCOIN"));
        // The starter list's wording ranks the same, ahead of other exchanges.
        let starter = mk("BINANCE:BTCUSDT", "Bitcoin / TetherUS", Crypto);
        let coinbase = mk("COINBASE:BTCUSDT", "BTC / USDT", Crypto);
        assert_eq!(r(&starter, "BITCOIN"), r(&coinbase, "BITCOIN"));
        assert!(rank(&btc, "BITC").is_some());
        assert!(rank(&btc, "BI").is_none());
        // A word of the name.
        let kotak = mk("NSE:KOTAKBANK", "Kotak Mahindra Bank", Stock);
        let mm = mk("NSE:M&M", "Mahindra & Mahindra", Stock);
        assert!(r(&kotak, "MAHINDRA") < 70);
        assert!(r(&mm, "MAHINDRA") < r(&kotak, "MAHINDRA"));
        // Typos, ranked after every real match.
        let rel = mk("NSE:RELIANCE", "Reliance Industries", Stock);
        assert!(rank(&rel, "RELAINCE").is_some());
        assert!(rank(&rel, "RELAIN").is_some());
        assert!(rank(&rel, "RELIENCE").is_some());
        assert!(r(&rel, "RELAINCE") > 95);
        assert!(rank(&rel, "RXLXXNCE").is_none());
        assert!(rank(&rel, "REL").is_some());
        let tsla = mk("NASDAQ:TSLA", "Tesla, Inc.", Stock);
        assert!(rank(&tsla, "TESLE").is_some());
        assert!(rank(&tsla, "TSAL").is_some());
        assert!(rank(&tsla, "AAPL").is_none());
    }

    #[test]
    fn demo_session_loads_and_ticks() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Hub::start(dir.path().to_path_buf(), true, Arc::new(|| {}));
        let sym = opentern_core::starter_watchlist().remove(0);
        hub.send(Request::Open {
            chart: 1,
            symbol: sym,
            tf: Timeframe::parse("7m").unwrap(),
            ext: false,
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let (mut got_bars, mut got_tick) = (false, false);
        while Instant::now() < deadline && !(got_bars && got_tick) {
            for e in hub.poll() {
                match e {
                    Event::Bars { bars, .. } => {
                        assert!(bars.len() > 100);
                        assert!(bars.windows(2).all(|w| w[0].time < w[1].time));
                        got_bars = true;
                    }
                    Event::Tick { .. } => got_tick = true,
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(got_bars && got_tick);
        hub.send(Request::Close { chart: 1 });
    }
}

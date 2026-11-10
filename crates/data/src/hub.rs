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
    Search {
        seq: u64,
        results: Vec<Symbol>,
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
    /// The answer to a `Request::Depth`.
    Depth {
        seq: u64,
        result: Result<crate::depth::Book, String>,
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
        let (tx, req_rx) = unbounded();
        let (ev_tx, rx) = unbounded();
        let out = Out { tx: ev_tx, wake };
        crate::nse::set_dir(dir.clone());
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
            .spawn(move || Session::run(chart, symbol, tf, ext, dir, demo, cmd_rx, out))
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
            Request::Overview { seq, market } => {
                let out = out.clone();
                let dir = dir.clone();
                let _ = std::thread::Builder::new().name("data-overview".into()).spawn(move || {
                    let result = crate::overview::board(market, &dir, demo).map_err(|e| e.to_string());
                    out.send(Event::Overview { seq, market, result });
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
            Request::Open { chart, symbol, tf, ext } => {
                if let Some(old) = sessions.remove(&chart) {
                    let _ = old.send(Cmd::Stop);
                }
                opened.insert(chart, (symbol.clone(), tf, ext));
                if symbol.min_qty == 0.0
                    && !demo
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
                if std::thread::Builder::new()
                    .name("data-quotes".into())
                    .spawn(move || quote_worker(symbols, demo, cmd_rx, out))
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
pub(crate) fn fetch_once(
    dir: &std::path::Path,
    demo: bool,
    symbol: &Symbol,
    tf: Timeframe,
    count: usize,
) -> Result<Vec<Bar>, String> {
    let provider = provider_for(symbol, demo);
    let base = resample::choose_base(tf, &provider.natives())
        .ok_or_else(|| format!("{} bars aren't available for {}.", tf.label(), symbol.ticker()))?;
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
    let fresh = match fresh {
        Ok(f) => f,
        // Offline: what the cache has is better than nothing.
        Err(_) if !bars.is_empty() => Vec::new(),
        Err(e) => return Err(e.to_string()),
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
    if demo {
        return Arc::new(Demo);
    }
    let c = creds().read().unwrap_or_else(|e| e.into_inner()).clone();
    match sym.exchange() {
        "BINANCE" => Arc::new(Binance::default()),
        "BYBIT" => Arc::new(crate::bybit::Bybit::default()),
        "OKX" => Arc::new(crate::okx::Okx::default()),
        "COINBASE" => Arc::new(crate::coinbase::Coinbase::default()),
        "KRAKEN" => Arc::new(crate::kraken::Kraken::default()),
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

/// Whether Yahoo serves a symbol (its quotes come from Yahoo's quote API).
fn yahoo_routed(sym: &Symbol) -> bool {
    let c = creds().read().unwrap_or_else(|e| e.into_inner());
    match sym.exchange() {
        "BINANCE" | "BYBIT" | "OKX" | "COINBASE" | "KRAKEN" | "OANDA" | "IBKR" => false,
        "NASDAQ" | "NYSE" | "AMEX" => c.alpaca.is_none(),
        "NSE" | "BSE" => c.upstox.is_none(),
        _ => true,
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

/// Exchanges in the order their symbols are offered when names tie.
fn exchange_rank(ex: &str) -> u32 {
    match ex {
        "BINANCE" => 0,
        "NASDAQ" | "NYSE" | "AMEX" | "NSE" => 1,
        "COINBASE" => 2,
        "BYBIT" => 3,
        "OKX" => 4,
        "KRAKEN" => 5,
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
                status: Status::Error(format!("{} bars aren't available for this symbol.", tf.label())),
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
                // An older bar the series lacked (a special session filled
                // in from the exchange's file) goes in its place.
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
        self.last.insert(id.clone(), (last, prev));
        self.out.send(Event::Quote {
            id: id.clone(),
            last,
            prev,
        });
        for (eid, e, legs) in &self.exprs {
            if !legs.contains(&id) {
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
    if demo {
        for s in &plain {
            let (last, prev) = demo::quote(s);
            q.borrow_mut().send(s.id.clone(), last, prev);
        }
        return;
    }
    let (crypto, others): (Vec<_>, Vec<_>) = plain.into_iter().partition(|s| s.id.starts_with("BINANCE:"));
    let pairs: Vec<String> = crypto.iter().map(|s| s.ticker().to_string()).collect();
    let yahoo = Yahoo::default();
    let mut last_poll: Option<Instant> = None;
    let mut backoff = Duration::from_secs(1);
    loop {
        // Yahoo symbols: a poll every 30 seconds.
        let poll_yahoo = |last_poll: &mut Option<Instant>| {
            if last_poll.is_none_or(|t| t.elapsed() > Duration::from_secs(30)) {
                *last_poll = Some(Instant::now());
                for s in &others {
                    let quote = if yahoo_routed(s) {
                        yahoo.quote(s).ok()
                    } else {
                        daily_quote(s)
                    };
                    if let Some((last, prev)) = quote {
                        q.borrow_mut().send(s.id.clone(), last, prev);
                    }
                }
            }
        };
        if pairs.is_empty() {
            poll_yahoo(&mut last_poll);
            for _ in 0..10 {
                if stop(&cmds) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            continue;
        }
        let mut stopped = false;
        let _ = binance::stream_quotes(&pairs, &mut |item| {
            if let Some((pair, last, open)) = item {
                q.borrow_mut().send(format!("BINANCE:{pair}"), last, open);
            }
            poll_yahoo(&mut last_poll);
            if stop(&cmds) {
                stopped = true;
                return false;
            }
            true
        });
        if stopped {
            return;
        }
        let until = Instant::now() + backoff;
        while Instant::now() < until {
            if stop(&cmds) {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

/// Ranks a symbol against a query: lower is better, None is no match. A
/// crypto pair whose base asset is the query ("ETH" → ETHUSDT) ranks just
/// after an exact ticker, with USDT pairs first.
fn rank(s: &Symbol, q: &str) -> Option<u32> {
    let t = s.ticker().to_uppercase();
    let d = s.description.to_uppercase();
    if t == q {
        return Some(0);
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
    } else if d.starts_with(q) {
        50
    } else if t.contains(q) {
        70
    } else if d.contains(q) {
        90
    } else {
        return None;
    };
    Some(score)
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

fn search_worker(dir: PathBuf, demo: bool, rx: Receiver<(u64, String)>, out: Out) {
    let cache = Cache::open(&dir).ok();
    let yahoo = Yahoo::default();
    // The exchanges' symbol lists refresh daily on their own thread, so a
    // search never waits for five downloads (OKX alone can time out). A
    // search that finds no coin also asks Binance about it right away, so
    // a new listing shows up the day it lists.
    let (fresh_tx, fresh_rx) = crossbeam_channel::bounded::<()>(1);
    let refreshing = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let refresh = |cache: &Option<Cache>, within: i64| {
        let stale = cache.as_ref().is_some_and(|c| {
            c.meta("binance_symbols_at")
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
                let lists: [(&str, crate::provider::Result<Vec<Symbol>>); 5] = [
                    ("binance", Binance::default().symbols()),
                    ("bybit", crate::bybit::Bybit::default().symbols()),
                    ("okx", crate::okx::Okx::default().symbols()),
                    ("coinbase", crate::coinbase::Coinbase::default().symbols()),
                    ("kraken", crate::kraken::Kraken::default().symbols()),
                ];
                let mut any = false;
                for (source, list) in lists {
                    if let Ok(list) = list {
                        let _ = c.save_symbols(source, &list);
                        any = true;
                    }
                }
                if any {
                    let _ = c.set_meta("binance_symbols_at", &crate::provider::now_ms().to_string());
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
        if let Some(c) = cache {
            known.extend(c.symbols().unwrap_or_default());
        }
        known.sort_by(|a, b| a.id.cmp(&b.id));
        known.dedup_by(|a, b| a.id == b.id);
        known
    };
    let mut known = load(&cache);
    let mut queue: std::collections::VecDeque<(u64, String)> = Default::default();
    loop {
        if queue.is_empty() {
            match rx.recv() {
                Ok(r) => queue.push_back(r),
                Err(_) => break,
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
        let Some(req) = queue.pop_front() else { continue };
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
            out.send(Event::Search { seq, results });
            continue;
        }
        let q = query.trim().to_uppercase();
        // "OKX:BTC" searches one exchange.
        let (want_ex, q) = match q.split_once(':') {
            Some((e, t)) => (Some(e.to_string()), t.to_string()),
            None => (None, q),
        };
        if q.is_empty() {
            out.send(Event::Search {
                seq,
                results: opentern_core::starter_watchlist(),
            });
            continue;
        }
        let mut hits: Vec<(u32, Symbol)> = known
            .iter()
            .filter(|s| want_ex.as_deref().is_none_or(|e| s.exchange() == e))
            .filter_map(|s| Some((rank(s, &q)?, s.clone())))
            .collect();
        // Sources without a symbol list: take "OANDA:EURUSD" as typed.
        if let Some(e) = want_ex.as_deref()
            && matches!(e, "OANDA" | "IBKR")
            && !q.is_empty()
            && !hits.iter().any(|(_, h)| h.ticker() == q)
        {
            let forex = e == "OANDA";
            hits.push((
                0,
                Symbol {
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
                },
            ));
        }
        // No coin by that name yet: perhaps it listed today. Binance is
        // asked while Yahoo searches, so stock searches don't wait longer.
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
        let (coins, found) = std::thread::scope(|s| {
            let coins = (!asks.is_empty()).then(|| {
                s.spawn(|| {
                    asks.iter()
                        .map(|t| (t.clone(), Binance::default().symbol(t)))
                        .collect::<Vec<_>>()
                })
            });
            let found = if !demo && want_ex.is_none() {
                yahoo.search(&q).ok()
            } else {
                None
            };
            (coins.and_then(|h| h.join().ok()).unwrap_or_default(), found)
        });
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
        hits.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(exchange_rank(a.1.exchange()).cmp(&exchange_rank(b.1.exchange())))
                .then(a.1.id.len().cmp(&b.1.id.len()))
        });
        out.send(Event::Search {
            seq,
            results: hits.into_iter().take(60).map(|(_, s)| s).collect(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

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
    /// The answer to a `Request::Depth`.
    Depth {
        seq: u64,
        result: Result<crate::depth::Book, String>,
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
    let mut opened: HashMap<u64, (Symbol, Timeframe)> = HashMap::new();
    let spawn = |chart: u64, symbol: Symbol, tf: Timeframe| -> Option<Sender<Cmd>> {
        let (tx, cmd_rx) = unbounded();
        let (dir, out) = (dir.clone(), out.clone());
        std::thread::Builder::new()
            .name(format!("data-{}", symbol.ticker()))
            .spawn(move || Session::run(chart, symbol, tf, dir, demo, cmd_rx, out))
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
    for req in rx {
        match req {
            Request::Fetch { seq, symbol, tf, count } => {
                let _ = fetch_tx.send((seq, symbol, tf, count));
            }
            Request::Hotlist { seq, market, kind } => {
                let out = out.clone();
                let _ = std::thread::Builder::new().name("data-hotlist".into()).spawn(move || {
                    let result = crate::feeds::hotlist(market, kind, demo).map_err(|e| e.to_string());
                    out.send(Event::Hotlist { seq, result });
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
            Request::Open { chart, symbol, tf } => {
                if let Some(old) = sessions.remove(&chart) {
                    let _ = old.send(Cmd::Stop);
                }
                opened.insert(chart, (symbol.clone(), tf));
                if let Some(tx) = spawn(chart, symbol, tf) {
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
                    for (chart, (symbol, tf)) in &opened {
                        if let Some(old) = sessions.remove(chart) {
                            let _ = old.send(Cmd::Stop);
                        }
                        if let Some(tx) = spawn(*chart, symbol.clone(), *tf) {
                            sessions.insert(*chart, tx);
                        }
                    }
                }
            }
        }
    }
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
    let cache = if demo { None } else { Cache::open(dir).ok() };
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
                provider.since(symbol, base, last.time)
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
    symbol: Symbol,
    tf: Timeframe,
    base: Timeframe,
    tz: TimeZone,
    provider: Arc<dyn Provider>,
    cache: Option<Cache>,
    bars: Vec<Bar>,
    complete: bool,
    cmds: Receiver<Cmd>,
    out: Out,
}

impl Session {
    fn run(chart: u64, symbol: Symbol, tf: Timeframe, dir: PathBuf, demo: bool, cmds: Receiver<Cmd>, out: Out) {
        let provider = provider_for(&symbol, demo);
        let Some(base) = resample::choose_base(tf, &provider.natives()) else {
            out.send(Event::Status {
                chart,
                status: Status::Error(format!("{} bars aren't available for this symbol.", tf.label())),
            });
            return;
        };
        let tz = TimeZone::get(&symbol.timezone).unwrap_or(TimeZone::UTC);
        // Demo data is made up on the spot; never cache it.
        let cache = if demo { None } else { Cache::open(&dir).ok() };
        let mut s = Session {
            chart,
            symbol,
            tf,
            base,
            tz,
            provider,
            cache,
            bars: Vec::new(),
            complete: false,
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
        if self.base == self.tf {
            self.bars.clone()
        } else {
            resample::resample(&self.bars, self.tf, &self.tz)
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
            let _ = c.save(&self.symbol.id, self.base, bars);
        }
    }

    /// Gets the series up to date: cache first (shown at once), then the
    /// missing tail from the network.
    fn load(&mut self) -> Result<(), Error> {
        let wanted = self.wanted();
        if self.bars.is_empty() {
            if let Some(c) = &self.cache {
                self.bars = c.last(&self.symbol.id, self.base, wanted).unwrap_or_default();
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
                        let _ = c.clear(&self.symbol.id, self.base);
                    }
                    self.bars.clear();
                    self.provider.latest(&self.symbol, self.base, wanted)?
                } else {
                    self.provider.since(&self.symbol, self.base, last.time)?
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
                if let Ok(i) = self.bars.binary_search_by_key(&b.time, |x| x.time) {
                    self.bars[i] = b;
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
            .and_then(|c| c.before(&self.symbol.id, self.base, first, n).ok())
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

    /// Dividends and splits, for stocks and funds (from Yahoo, whichever
    /// source serves the prices).
    fn corporate(&self, demo: bool) {
        use opentern_core::AssetKind;
        let stocky = matches!(self.symbol.kind, AssetKind::Stock | AssetKind::Fund)
            && matches!(
                self.symbol.exchange(),
                "NASDAQ" | "NYSE" | "AMEX" | "NSE" | "BSE" | "YAHOO"
            );
        if demo || !stocky {
            return;
        }
        if let Ok(c) = Yahoo::default().corporate(&self.symbol)
            && (!c.dividends.is_empty() || !c.splits.is_empty())
        {
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
        match crate::expr::parse(&s.id) {
            Some((e, legs)) if crate::expr::is_expr(&s.id) => {
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

fn search_worker(dir: PathBuf, demo: bool, rx: Receiver<(u64, String)>, out: Out) {
    let cache = Cache::open(&dir).ok();
    let yahoo = Yahoo::default();
    // The exchanges' symbol lists refresh weekly on their own thread, so a
    // search never waits for five downloads (OKX alone can time out).
    let (fresh_tx, fresh_rx) = crossbeam_channel::bounded::<()>(1);
    let stale = cache.as_ref().is_some_and(|c| {
        c.meta("binance_symbols_at")
            .and_then(|t| t.parse::<i64>().ok())
            .is_none_or(|t| crate::provider::now_ms() - t > 7 * 86_400_000)
    });
    if stale && !demo {
        let dir = dir.clone();
        let _ = std::thread::Builder::new().name("symbol-lists".into()).spawn(move || {
            let Ok(c) = Cache::open(&dir) else { return };
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
                let _ = fresh_tx.send(());
            }
        });
    }
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
    while let Ok(mut req) = rx.recv() {
        // Only the latest query matters.
        while let Ok(newer) = rx.try_recv() {
            req = newer;
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
                    timezone: if forex { "UTC".into() } else { "America/New_York".into() },
                },
            ));
        }
        if !demo
            && want_ex.is_none()
            && let Ok(found) = yahoo.search(&q)
        {
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

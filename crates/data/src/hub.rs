//! The data hub: the UI sends requests, and background threads answer with
//! events. One session thread per open chart (cache first, then network,
//! then the live stream, reconnecting as needed), one thread for watchlist
//! quotes, one for symbol search. Nothing here blocks the UI.

use crate::binance::{self, Binance};
use crate::cache::Cache;
use crate::demo::{self, Demo};
use crate::provider::{Error, Live, Provider};
use crate::resample;
use crate::yahoo::Yahoo;
use crossbeam_channel::{Receiver, Sender, unbounded};
use jiff::tz::TimeZone;
use opentern_core::{Bar, Symbol, Timeframe};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
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
    let mut quotes: Option<Sender<Cmd>> = None;
    let (search_tx, search_rx) = unbounded::<(u64, String)>();
    {
        let (dir, out) = (dir.clone(), out.clone());
        std::thread::Builder::new()
            .name("data-search".into())
            .spawn(move || search_worker(dir, demo, search_rx, out))
            .expect("spawn search");
    }
    for req in rx {
        match req {
            Request::Open { chart, symbol, tf } => {
                if let Some(old) = sessions.remove(&chart) {
                    let _ = old.send(Cmd::Stop);
                }
                let (tx, cmd_rx) = unbounded();
                let (dir, out) = (dir.clone(), out.clone());
                let spawned = std::thread::Builder::new()
                    .name(format!("data-{}", symbol.ticker()))
                    .spawn(move || Session::run(chart, symbol, tf, dir, demo, cmd_rx, out));
                if spawned.is_ok() {
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
        }
    }
}

pub(crate) fn provider_for(sym: &Symbol, demo: bool) -> Arc<dyn Provider> {
    if crate::expr::is_expr(&sym.id)
        && let Some(p) = crate::expr::ExprProvider::new(sym, demo)
    {
        return Arc::new(p);
    }
    if demo {
        Arc::new(Demo)
    } else if sym.id.starts_with("BINANCE:") {
        Arc::new(Binance::default())
    } else {
        Arc::new(Yahoo::default())
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

    fn main(&mut self) {
        let mut backoff = Duration::from_secs(1);
        loop {
            match self.load() {
                Ok(()) => {}
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
                    if let Ok((last, prev)) = yahoo.quote(s) {
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
    let mut known: Vec<Symbol> = opentern_core::starter_watchlist();
    let binance = Binance::default();
    let yahoo = Yahoo::default();
    let mut loaded = false;
    while let Ok(mut req) = rx.recv() {
        // Only the latest query matters.
        while let Ok(newer) = rx.try_recv() {
            req = newer;
        }
        if !loaded {
            loaded = true;
            if let Some(c) = &cache {
                let stale = c
                    .meta("binance_symbols_at")
                    .and_then(|t| t.parse::<i64>().ok())
                    .is_none_or(|t| crate::provider::now_ms() - t > 7 * 86_400_000);
                if stale
                    && !demo
                    && let Ok(list) = binance.symbols()
                {
                    let _ = c.save_symbols("binance", &list);
                    let _ = c.set_meta("binance_symbols_at", &crate::provider::now_ms().to_string());
                }
                known.extend(c.symbols().unwrap_or_default());
            }
            known.sort_by(|a, b| a.id.cmp(&b.id));
            known.dedup_by(|a, b| a.id == b.id);
        }
        let (seq, query) = req;
        if crate::expr::is_expr(&query) {
            // A formula: offer it as the first result.
            let results = crate::expr::symbol(&query, &known).into_iter().collect();
            out.send(Event::Search { seq, results });
            continue;
        }
        let q = query.trim().to_uppercase();
        let q = q.split_once(':').map(|(_, t)| t.to_string()).unwrap_or(q);
        if q.is_empty() {
            out.send(Event::Search {
                seq,
                results: opentern_core::starter_watchlist(),
            });
            continue;
        }
        let mut hits: Vec<(u32, Symbol)> = known.iter().filter_map(|s| Some((rank(s, &q)?, s.clone()))).collect();
        if !demo && let Ok(found) = yahoo.search(&q) {
            for s in found {
                if !hits.iter().any(|(_, h)| h.id == s.id)
                    && let Some(r) = rank(&s, &q).or(Some(80))
                {
                    hits.push((r, s));
                }
            }
        }
        hits.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.id.len().cmp(&b.1.id.len())));
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

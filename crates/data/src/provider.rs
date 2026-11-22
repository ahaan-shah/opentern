//! What every market data source provides, plus the shared HTTP and
//! websocket plumbing.

use opentern_core::{Bar, Symbol, Timeframe};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::time::Duration;

#[derive(Debug)]
pub enum Error {
    /// Couldn't reach the service at all.
    Offline,
    /// The service answered with something we can't use.
    Bad(String),
    /// The service is down or too slow while the internet works; the
    /// message names it.
    Down(String),
}

/// What an `Offline` error says when nothing has answered lately: the
/// only case that blames the connection.
pub const NO_INTERNET: &str = "Couldn't connect. Check your internet connection.";

/// The start of what Yahoo's 404 says: a symbol it no longer has.
pub const GONE: &str = "Yahoo has no data for this symbol";

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Something else answered lately (a stream, another request):
            // the internet works, this service doesn't.
            Error::Offline if net_ok() => write!(f, "Couldn't reach the data service."),
            Error::Offline => write!(f, "{NO_INTERNET}"),
            Error::Bad(m) | Error::Down(m) => write!(f, "{m}"),
        }
    }
}

/// When any service last answered (ms since the epoch).
static LAST_OK: AtomicI64 = AtomicI64::new(0);

/// Notes that a service answered: the internet works.
pub(crate) fn mark_ok() {
    LAST_OK.store(now_ms(), Ordering::Relaxed);
}

/// Whether anything answered in the last two minutes.
pub fn net_ok() -> bool {
    now_ms() - LAST_OK.load(Ordering::Relaxed) < 120_000
}

pub type Result<T> = std::result::Result<T, Error>;

/// One message from a live stream.
pub enum Live {
    /// The forming (or just-closed) bar of the streamed interval.
    Bar(Bar),
    /// Nothing arrived for a moment: a chance to handle requests.
    Idle,
}

pub trait Provider: Send + Sync {
    /// Intervals the source serves directly; others are resampled.
    fn natives(&self) -> Vec<Timeframe>;
    /// The newest `n` bars, oldest first.
    fn latest(&self, sym: &Symbol, tf: Timeframe, n: usize) -> Result<Vec<Bar>>;
    /// Bars from `start` (inclusive) up to now.
    fn since(&self, sym: &Symbol, tf: Timeframe, start: i64) -> Result<Vec<Bar>>;
    /// Up to `n` bars older than `end`; empty when history runs out.
    fn before(&self, sym: &Symbol, tf: Timeframe, end: i64, n: usize) -> Result<Vec<Bar>>;
    /// Streams the forming bar until `sink` returns false or the connection
    /// drops (then the caller reconnects).
    fn stream(&self, sym: &Symbol, tf: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()>;
}

pub(crate) fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        // A blocked host often swallows the connection attempt; give up on
        // it soon so a fallback host gets its turn.
        .timeout_connect(Some(Duration::from_secs(8)))
        .user_agent(concat!(
            "Mozilla/5.0 (compatible; opentern/",
            env!("CARGO_PKG_VERSION"),
            ")"
        ))
        .build()
        .into()
}

/// Why a GET failed, before it's put in words.
#[derive(Debug)]
enum Fail {
    Status(u16),
    /// Connected but no answer in time.
    Slow,
    /// No connection (or a network that intercepts the host).
    Connect,
    Other(String),
}

/// A GET with an optional time limit of its own.
fn fetch_json(agent: &ureq::Agent, url: &str, limit: Option<Duration>) -> std::result::Result<serde_json::Value, Fail> {
    let mut req = agent.get(url);
    if let Some(d) = limit {
        req = req.config().timeout_global(Some(d)).build();
    }
    let mut resp = req.call().map_err(|e| match e {
        ureq::Error::StatusCode(c) => Fail::Status(c),
        ureq::Error::Timeout(_) | ureq::Error::BodyStalled => Fail::Slow,
        ureq::Error::Io(_)
        | ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        // A network that intercepts a blocked host fails the handshake or
        // answers with something that isn't the service.
        | ureq::Error::Tls(_)
        | ureq::Error::Rustls(_)
        | ureq::Error::Protocol(_) => Fail::Connect,
        e => Fail::Other(e.to_string()),
    })?;
    let text = resp
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_string()
        .map_err(|_| Fail::Slow)?;
    mark_ok();
    serde_json::from_str(&text).map_err(|e| Fail::Other(format!("Unexpected reply from the data service: {e}")))
}

pub(crate) fn get_json(agent: &ureq::Agent, url: &str) -> Result<serde_json::Value> {
    if let Some(path) = YAHOO.iter().find_map(|h| url.strip_prefix(h)) {
        return yahoo_get(agent, &YAHOO, &YAHOO_AT, YAHOO_FIRST, path);
    }
    fetch_json(agent, url, None).map_err(|f| match f {
        Fail::Status(c) => Error::Bad(format!("The data service answered with an error ({c}).")),
        Fail::Slow | Fail::Connect => Error::Offline,
        Fail::Other(m) => Error::Bad(m),
    })
}

/// Yahoo's two API hosts; the same data on each.
const YAHOO: [&str; 2] = ["https://query1.finance.yahoo.com", "https://query2.finance.yahoo.com"];
/// Which of them answered last (asked first).
static YAHOO_AT: AtomicUsize = AtomicUsize::new(0);
/// How long the first host gets before the other is asked.
const YAHOO_FIRST: Duration = Duration::from_secs(5);

/// A Yahoo GET on whichever host answers: the one that answered last gets
/// `YAHOO_FIRST`, then the other is asked (a slow or failing host, seen
/// for NSE charts in Nov 2026: 502s after 15 s on query1 while query2
/// answered at once). A 404 or other refusal is the same on both.
fn yahoo_get(
    agent: &ureq::Agent,
    hosts: &[&str],
    at: &AtomicUsize,
    first: Duration,
    path: &str,
) -> Result<serde_json::Value> {
    let start = at.load(Ordering::Relaxed);
    let mut last = Fail::Connect;
    for k in 0..hosts.len() {
        let i = (start + k) % hosts.len();
        let limit = (k == 0).then_some(first);
        match fetch_json(agent, &format!("{}{path}", hosts[i]), limit) {
            Ok(v) => {
                at.store(i, Ordering::Relaxed);
                return Ok(v);
            }
            Err(Fail::Status(404)) => {
                return Err(Error::Bad(format!("{GONE}. It may have been renamed or delisted.")));
            }
            Err(Fail::Status(c)) if c < 500 && c != 429 => {
                return Err(Error::Bad(format!("Yahoo answered with an error ({c}).")));
            }
            Err(f) => last = f,
        }
    }
    Err(match last {
        Fail::Status(429) => Error::Down("Yahoo is limiting requests right now.".into()),
        Fail::Status(c) => Error::Down(format!("Yahoo isn't answering right now (error {c}).")),
        Fail::Slow => Error::Down("Yahoo is too slow to answer right now.".into()),
        Fail::Connect if net_ok() => Error::Down("Couldn't reach Yahoo.".into()),
        Fail::Connect => Error::Offline,
        Fail::Other(m) => Error::Bad(m),
    })
}

/// The largest websocket message accepted (Binance's combined quote stream
/// sends well under 100 KB).
const WS_MAX_MESSAGE: usize = 8 << 20;

pub(crate) type Socket = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>;

/// Opens a TLS websocket. Returns the socket and a second handle on the
/// TCP stream, used to set a short read timeout so the stream loop can
/// handle requests between messages.
pub(crate) fn websocket(host: &str, port: u16, path: &str) -> Result<(Socket, TcpStream)> {
    // Without a timeout a blocked host keeps a connect hanging for minutes.
    let tcp = (host, port)
        .to_socket_addrs()
        .map_err(|_| Error::Offline)?
        .find_map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(8)).ok())
        .ok_or(Error::Offline)?;
    tcp.set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| Error::Offline)?;
    let handle = tcp.try_clone().map_err(|_| Error::Offline)?;
    let url = format!("wss://{host}:{port}{path}");
    // Market data comes in small messages; a service (or anything posing as
    // one) can't make the app hold more than this at once.
    let limits = tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(WS_MAX_MESSAGE))
        .max_frame_size(Some(WS_MAX_MESSAGE));
    let (ws, _) = tungstenite::client_tls_with_config(url, tcp, Some(limits), None).map_err(|_| Error::Offline)?;
    handle
        .set_read_timeout(Some(Duration::from_millis(250)))
        .map_err(|_| Error::Offline)?;
    Ok((ws, handle))
}

/// How long a websocket may stay silent before it's asked for a pong, and
/// how long that pong may take before the connection counts as dead.
const QUIET: Duration = Duration::from_secs(30);
const PONG_WAIT: Duration = Duration::from_secs(20);

/// Notices a connection that died without closing (the laptop slept, the
/// network changed): reads only time out then, and without this the chart
/// stayed "live" on frozen prices for good.
pub(crate) struct Alive {
    heard: std::time::Instant,
    pinged: Option<std::time::Instant>,
    quiet: Duration,
    pong_wait: Duration,
}

impl Default for Alive {
    fn default() -> Alive {
        Alive::within(QUIET, PONG_WAIT)
    }
}

impl Alive {
    fn within(quiet: Duration, pong_wait: Duration) -> Alive {
        Alive {
            heard: std::time::Instant::now(),
            pinged: None,
            quiet,
            pong_wait,
        }
    }

    /// After a read timed out: ping a quiet connection, give up on one
    /// that didn't answer.
    fn quiet(&mut self, ws: &mut Socket) -> Result<()> {
        match self.pinged {
            Some(at) if at.elapsed() >= self.pong_wait => Err(Error::Offline),
            None if self.heard.elapsed() >= self.quiet => {
                ws.send(tungstenite::Message::Ping(Vec::new().into()))
                    .map_err(|_| Error::Offline)?;
                self.pinged = Some(std::time::Instant::now());
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// Reads one text message, or `None` on a read timeout or a non-text
/// frame. Errors mean the connection is gone, including one that went
/// silent and didn't answer a ping (`Alive`).
pub(crate) fn read_text(ws: &mut Socket, alive: &mut Alive) -> Result<Option<String>> {
    use tungstenite::Message;
    let got = match ws.read() {
        Ok(Message::Text(t)) => Some(t.to_string()),
        Ok(Message::Close(_)) => return Err(Error::Offline),
        Ok(_) => None,
        Err(tungstenite::Error::Io(e))
            if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
        {
            alive.quiet(ws)?;
            return Ok(None);
        }
        Err(_) => return Err(Error::Offline),
    };
    alive.heard = std::time::Instant::now();
    alive.pinged = None;
    mark_ok();
    Ok(got)
}

pub(crate) fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

/// A JSON number or numeric string as f64 (Binance sends prices as strings).
pub(crate) fn num(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::String(s) => s.parse().ok(),
        serde_json::Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

/// GET with extra headers (API keys, bearer tokens).
pub(crate) fn get_json_with(agent: &ureq::Agent, url: &str, headers: &[(&str, &str)]) -> Result<serde_json::Value> {
    // These carry the user's keys (some in headers of the service's own
    // naming, which a redirect would pass on): never follow one elsewhere.
    let mut req = agent.get(url).config().max_redirects(0).build();
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut resp = req.call().map_err(|e| match e {
        ureq::Error::StatusCode(401 | 403) => {
            Error::Bad("The data service refused the key. Check it in Settings → Data providers.".into())
        }
        ureq::Error::StatusCode(c) => Error::Bad(format!("The data service answered with an error ({c}).")),
        ureq::Error::Io(_)
        | ureq::Error::Timeout(_)
        | ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::BodyStalled
        // A network that intercepts a blocked host fails the handshake or
        // answers with something that isn't the service.
        | ureq::Error::Tls(_)
        | ureq::Error::Rustls(_)
        | ureq::Error::Protocol(_) => Error::Offline,
        e => Error::Bad(e.to_string()),
    })?;
    let text = resp
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_string()
        .map_err(|_| Error::Offline)?;
    mark_ok();
    serde_json::from_str(&text).map_err(|e| Error::Bad(format!("Unexpected reply from the data service: {e}")))
}

/// Up to `n` bars older than `end`, for APIs that return the newest page
/// ending at a time: `fetch(end_inclusive)` gives bars oldest first.
pub(crate) fn page_back(mut fetch: impl FnMut(i64) -> Result<Vec<Bar>>, end: i64, n: usize) -> Result<Vec<Bar>> {
    let mut out: Vec<Bar> = Vec::new();
    let mut end = end - 1;
    while out.len() < n {
        let page: Vec<Bar> = fetch(end)?.into_iter().filter(|b| b.time <= end).collect();
        let Some(first) = page.first() else { break };
        end = first.time - 1;
        out.splice(0..0, page);
    }
    if out.len() > n {
        out.drain(..out.len() - n);
    }
    Ok(out)
}

/// Bars from `start` to `until` (now, usually), for APIs that take a time
/// window: `fetch(from, to)` gives bars oldest first; windows are `span` ms.
pub(crate) fn page_forward(
    mut fetch: impl FnMut(i64, i64) -> Result<Vec<Bar>>,
    start: i64,
    until: i64,
    span: i64,
) -> Result<Vec<Bar>> {
    let mut out: Vec<Bar> = Vec::new();
    let mut from = start;
    let span = span.max(1);
    while from <= until {
        let to = from + span - 1;
        let page = fetch(from, to)?;
        for b in page {
            if b.time >= from && out.last().is_none_or(|l| b.time > l.time) {
                out.push(b);
            }
        }
        from = to + 1;
    }
    Ok(out)
}

/// Sends a text frame.
pub(crate) fn send_text(ws: &mut Socket, text: &str) -> Result<()> {
    ws.send(tungstenite::Message::Text(text.into()))
        .map_err(|_| Error::Offline)
}

/// A websocket stream of candles: subscribe, keep alive with `ping` every
/// `every`, and pass each parsed bar to `sink`.
pub(crate) fn ws_candles(
    host: &str,
    port: u16,
    path: &str,
    subscribe: &str,
    ping: Option<(&str, Duration)>,
    parse: impl Fn(&serde_json::Value) -> Vec<Bar>,
    sink: &mut dyn FnMut(Live) -> bool,
) -> Result<()> {
    let (mut ws, _tcp) = websocket(host, port, path)?;
    if !subscribe.is_empty() {
        send_text(&mut ws, subscribe)?;
    }
    let mut last_ping = std::time::Instant::now();
    let mut alive = Alive::default();
    loop {
        if let Some((msg, every)) = ping
            && last_ping.elapsed() >= every
        {
            send_text(&mut ws, msg)?;
            last_ping = std::time::Instant::now();
        }
        let bars = match read_text(&mut ws, &mut alive)? {
            Some(text) => serde_json::from_str::<serde_json::Value>(&text)
                .map(|v| parse(&v))
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let mut go = true;
        if bars.is_empty() {
            go = sink(Live::Idle);
        }
        for b in bars {
            go &= sink(Live::Bar(b));
        }
        if !go {
            let _ = ws.close(None);
            return Ok(());
        }
    }
}

/// RFC 3339 text for a time in ms (UTC).
pub(crate) fn rfc3339(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|t| t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default()
}

/// RFC 3339 / ISO text (any offset, any fraction) → ms.
/// Unix seconds from a reply → ms, for a time a market could have (1900
/// to 2200); None for anything else (a hostile or broken reply's
/// `i64::MAX` would overflow the multiplication).
pub(crate) fn secs_ms(s: i64) -> Option<i64> {
    (-2_208_988_800..=7_258_118_400).contains(&s).then(|| s * 1000)
}

/// A time in ms from a reply, if it's in that range.
pub(crate) fn sane_ms(ms: i64) -> Option<i64> {
    (-2_208_988_800_000..=7_258_118_400_000).contains(&ms).then_some(ms)
}

pub(crate) fn parse_time(s: &str) -> Option<i64> {
    s.parse::<jiff::Timestamp>().ok().map(|t| t.as_millisecond())
}

/// "BTCUSDT" → ("BTC", "USDT"), splitting at a known quote currency.
pub(crate) fn split_pair(t: &str) -> Option<(&str, &str)> {
    const QUOTES: [&str; 12] = [
        "USDT", "USDC", "FDUSD", "BUSD", "DAI", "USD", "EUR", "GBP", "JPY", "BTC", "ETH", "TRY",
    ];
    QUOTES
        .iter()
        .find_map(|q| t.strip_suffix(q).filter(|b| !b.is_empty()).map(|b| (b, *q)))
}

#[cfg(test)]
mod yahoo_host_tests {
    use super::*;
    use std::io::{Read, Write};

    /// A local server answering every request with `status` and `body`
    /// after `wait`.
    fn serve(status: u16, body: &'static str, wait: Duration) -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            for mut c in l.incoming().flatten() {
                std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let _ = c.read(&mut buf);
                    std::thread::sleep(wait);
                    let _ = write!(
                        c,
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                });
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn a_slow_or_failing_host_falls_back_and_is_remembered() {
        let agent = agent();
        let good = serve(200, r#"{"ok":true}"#, Duration::ZERO);
        let at = AtomicUsize::new(0);
        // Slow: given up on after the first host's limit.
        let slow = serve(200, r#"{"ok":false}"#, Duration::from_secs(3));
        let t = std::time::Instant::now();
        let v = yahoo_get(&agent, &[&slow, &good], &at, Duration::from_millis(300), "/x").unwrap();
        assert_eq!(v["ok"], true);
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        assert_eq!(
            at.load(Ordering::Relaxed),
            1,
            "the host that answered goes first next time"
        );
        // A server error on one host: the other.
        let at = AtomicUsize::new(0);
        let broken = serve(502, "", Duration::ZERO);
        assert_eq!(
            yahoo_get(&agent, &[&broken, &good], &at, Duration::from_secs(5), "/x").unwrap()["ok"],
            true
        );
        // A 404 is the same on both: said at once, as a symbol that's gone.
        let gone = serve(404, "", Duration::ZERO);
        let e = yahoo_get(
            &agent,
            &[&gone, &good],
            &AtomicUsize::new(0),
            Duration::from_secs(5),
            "/x",
        )
        .unwrap_err();
        assert!(e.to_string().starts_with(GONE), "{e}");
        // Both failing while the internet works: Yahoo is named, not the
        // connection.
        mark_ok();
        let e = yahoo_get(
            &agent,
            &[&broken, &broken],
            &AtomicUsize::new(0),
            Duration::from_secs(5),
            "/x",
        )
        .unwrap_err();
        assert_eq!(e.to_string(), "Yahoo isn't answering right now (error 502).");
        assert!(
            !Error::Offline.to_string().contains("internet"),
            "something answered just now"
        );
    }
}

#[cfg(test)]
mod paging_tests {
    use super::*;

    fn bar(t: i64) -> Bar {
        Bar {
            time: t,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 0.0,
        }
    }

    #[test]
    fn pages_back_and_forward() {
        // A source with a bar every 10 ms from 0 to 990, pages of 7.
        let all: Vec<Bar> = (0..100).map(|i| bar(i * 10)).collect();
        let back = page_back(
            |end| {
                let v: Vec<Bar> = all.iter().filter(|b| b.time <= end).cloned().collect();
                Ok(v[v.len().saturating_sub(7)..].to_vec())
            },
            500,
            20,
        )
        .unwrap();
        assert_eq!(back.len(), 20);
        assert_eq!((back[0].time, back[19].time), (300, 490));
        let fwd = page_forward(
            |from, to| Ok(all.iter().filter(|b| b.time >= from && b.time <= to).cloned().collect()),
            955,
            1000,
            30,
        );
        let fwd = fwd.unwrap();
        assert_eq!(fwd.first().map(|b| b.time), Some(960));
        assert_eq!(fwd.last().map(|b| b.time), Some(990));
        assert_eq!(split_pair("BTCUSDT"), Some(("BTC", "USDT")));
        assert_eq!(split_pair("ETHBTC"), Some(("ETH", "BTC")));
        assert_eq!(parse_time("2024-01-02T14:30:00.000000000Z"), Some(1_704_205_800_000));
        assert_eq!(rfc3339(1_704_205_800_000), "2024-01-02T14:30:00Z");
    }
}

#[cfg(test)]
mod alive_tests {
    use super::*;
    use std::net::TcpListener;
    use tungstenite::stream::MaybeTlsStream;

    /// A local server that answers pings only when `answers`, else holds
    /// the connection open and silent (a dead network looks like that).
    fn client(answers: bool) -> Socket {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(tcp).unwrap();
            if answers {
                // Reading is what makes tungstenite answer a ping.
                while ws.read().is_ok() {}
            } else {
                std::thread::sleep(Duration::from_secs(5));
            }
        });
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (ws, _) = tungstenite::client(format!("ws://127.0.0.1:{port}/"), MaybeTlsStream::Plain(tcp)).unwrap();
        if let MaybeTlsStream::Plain(t) = ws.get_ref() {
            t.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        }
        ws
    }

    fn run(ws: &mut Socket, for_ms: u64) -> Result<()> {
        let mut alive = Alive::within(Duration::from_millis(100), Duration::from_millis(200));
        let until = std::time::Instant::now() + Duration::from_millis(for_ms);
        while std::time::Instant::now() < until {
            read_text(ws, &mut alive)?;
        }
        Ok(())
    }

    #[test]
    fn a_silent_connection_that_ignores_pings_counts_as_gone() {
        assert!(matches!(run(&mut client(false), 2000), Err(Error::Offline)));
    }

    #[test]
    fn a_quiet_connection_that_answers_pings_stays() {
        assert!(run(&mut client(true), 1000).is_ok());
    }
}

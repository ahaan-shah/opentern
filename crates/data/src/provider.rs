//! What every market data source provides, plus the shared HTTP and
//! websocket plumbing.

use opentern_core::{Bar, Symbol, Timeframe};
use std::net::TcpStream;
use std::time::Duration;

#[derive(Debug)]
pub enum Error {
    /// Couldn't reach the service at all.
    Offline,
    /// The service answered with something we can't use.
    Bad(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Offline => write!(f, "Couldn't connect. Check your internet connection."),
            Error::Bad(m) => write!(f, "{m}"),
        }
    }
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
        .user_agent(concat!(
            "Mozilla/5.0 (compatible; opentern/",
            env!("CARGO_PKG_VERSION"),
            ")"
        ))
        .build()
        .into()
}

pub(crate) fn get_json(agent: &ureq::Agent, url: &str) -> Result<serde_json::Value> {
    let mut resp = agent.get(url).call().map_err(|e| match e {
        ureq::Error::StatusCode(c) => Error::Bad(format!("The data service answered with an error ({c}).")),
        ureq::Error::Io(_)
        | ureq::Error::Timeout(_)
        | ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::BodyStalled => Error::Offline,
        e => Error::Bad(e.to_string()),
    })?;
    let text = resp
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_string()
        .map_err(|_| Error::Offline)?;
    serde_json::from_str(&text).map_err(|e| Error::Bad(format!("Unexpected reply from the data service: {e}")))
}

pub(crate) type Socket = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>;

/// Opens a TLS websocket. Returns the socket and a second handle on the
/// TCP stream, used to set a short read timeout so the stream loop can
/// handle requests between messages.
pub(crate) fn websocket(host: &str, port: u16, path: &str) -> Result<(Socket, TcpStream)> {
    let tcp = TcpStream::connect((host, port)).map_err(|_| Error::Offline)?;
    tcp.set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| Error::Offline)?;
    let handle = tcp.try_clone().map_err(|_| Error::Offline)?;
    let url = format!("wss://{host}:{port}{path}");
    let (ws, _) = tungstenite::client_tls(url, tcp).map_err(|_| Error::Offline)?;
    handle
        .set_read_timeout(Some(Duration::from_millis(250)))
        .map_err(|_| Error::Offline)?;
    Ok((ws, handle))
}

/// Reads one text message, or `None` on a read timeout. Errors mean the
/// connection is gone.
pub(crate) fn read_text(ws: &mut Socket) -> Result<Option<String>> {
    use tungstenite::Message;
    match ws.read() {
        Ok(Message::Text(t)) => Ok(Some(t.to_string())),
        Ok(Message::Close(_)) => Err(Error::Offline),
        Ok(_) => Ok(None),
        Err(tungstenite::Error::Io(e))
            if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
        {
            Ok(None)
        }
        Err(_) => Err(Error::Offline),
    }
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
    let mut req = agent.get(url);
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
        | ureq::Error::BodyStalled => Error::Offline,
        e => Error::Bad(e.to_string()),
    })?;
    let text = resp
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_string()
        .map_err(|_| Error::Offline)?;
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
    loop {
        if let Some((msg, every)) = ping
            && last_ping.elapsed() >= every
        {
            send_text(&mut ws, msg)?;
            last_ping = std::time::Instant::now();
        }
        let bars = match read_text(&mut ws)? {
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

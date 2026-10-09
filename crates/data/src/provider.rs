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

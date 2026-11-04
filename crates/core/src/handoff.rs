//! Sending an export file straight to another device on the same network.
//!
//! No server and no account. The sender listens only while its code shows.
//! The receiver finds it with a UDP broadcast (or a typed address) and they
//! connect over TCP. The six-digit code goes into SPAKE2, so it's never sent
//! and can't be guessed from a recording: each wrong guess needs a live
//! attempt, and the sender stops after [`MAX_WRONG`]. Everything after the
//! exchange is encrypted with ChaCha20-Poly1305, a key per direction.
//!
//! Wire: frames of a big-endian `u32` length and a payload.
//!   R→S hello: magic, then R's SPAKE2 message
//!   S→R 1 + S's SPAKE2 message (or 0: incompatible versions)
//!   R→S sealed: R's device name (proves R has the key; else a wrong code)
//!   S→R sealed: JSON `Meta`
//!   S→R sealed: the file, in chunks
//!   R→S sealed: "ok" once the file checks out

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use std::collections::HashSet;
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

/// The port both the broadcast and the transfer try first.
pub const PORT: u16 = 47811;
pub const CODE_LEN: usize = 6;
/// Wrong codes before the sender stops listening.
pub const MAX_WRONG: u32 = 5;
const MAGIC: &[u8] = b"OPENTERN-TRANSFER/1";
const PROBE: &[u8] = b"OPENTERN-FIND/1";
const REPLY: &[u8] = b"OPENTERN-HERE/1";
const HELLO: &[u8] = b"OPENTERN-HELLO/1";
/// Devices one search will try the code with before it stops. Each try lets
/// whoever answered test one guess, so a device pretending to be the sender
/// can't answer again and again until it hits the code.
const MAX_TRIES: u32 = 3;
const CHUNK: usize = 256 * 1024;
const MAX_FRAME: usize = CHUNK + 1024;
const MAX_FILE: u64 = 2 << 30;
/// How long the code stays good.
const CODE_LIFE: Duration = Duration::from_secs(15 * 60);
/// How long a receiver looks for the sender.
const SEARCH: Duration = Duration::from_secs(8);
/// After a wrong code: a little longer, in case another device is sending.
const AFTER_WRONG: Duration = Duration::from_millis(1200);
const CONNECT: Duration = Duration::from_secs(3);
/// A connection with no traffic for this long has dropped.
const IDLE: Duration = Duration::from_secs(30);
const HANDSHAKE_IDLE: Duration = Duration::from_secs(10);
/// A connection has this long to prove the code, so a stranger trickling
/// bytes can't keep the real device waiting.
const HANDSHAKE_LIMIT: Duration = Duration::from_secs(20);

/// A random six-digit code.
pub fn new_code() -> String {
    loop {
        let mut b = [0u8; 4];
        if getrandom::fill(&mut b).is_err() {
            // No system randomness: nothing safe to offer. Practically never.
            return String::new();
        }
        let n = u32::from_le_bytes(b);
        // Rejection keeps every code equally likely.
        if n < u32::MAX - (u32::MAX % 1_000_000) {
            return format!("{:06}", n % 1_000_000);
        }
    }
}

/// This computer's name, as the other device will show it.
pub fn device_name() -> String {
    let name = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_default();
    let name = name.split('.').next().unwrap_or("").trim().to_string();
    if name.is_empty() { "Other device".into() } else { name }
}

/// This computer's addresses on its networks, likeliest first.
pub fn local_addresses() -> Vec<Ipv4Addr> {
    let Ok(ifs) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    // Virtual machines, containers and VPNs: not where another device is.
    // Windows names adapters in words, macOS and Linux in short codes.
    let virtual_if = |n: &str| {
        let n = n.to_lowercase();
        n.starts_with("br-")
            || [
                "docker",
                "veth",
                "virbr",
                "vmnet",
                "vboxnet",
                "vethernet",
                "virtualbox",
                "vmware",
                "hyper-v",
                "wsl",
                "tailscale",
                "zerotier",
                "utun",
                "awdl",
                "llw",
                "bridge",
            ]
            .iter()
            .any(|p| n.contains(p))
    };
    let mut out: Vec<Ipv4Addr> = ifs
        .iter()
        .filter(|i| !i.is_loopback() && !virtual_if(&i.name))
        .filter_map(|i| match i.ip() {
            IpAddr::V4(a) if !a.is_link_local() => Some(a),
            _ => None,
        })
        .collect();
    out.sort_by_key(|a| match a.octets() {
        [192, 168, ..] => 0,
        [10, ..] => 1,
        [172, b, ..] if (16..32).contains(&b) => 2,
        _ => 3,
    });
    out.dedup();
    out
}

/// Addresses a transfer may involve: this computer and private networks
/// (with VPNs' shared range). Transfers never cross the internet, even on a
/// computer that's directly reachable from it.
fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => {
            let [x, y, ..] = a.octets();
            a.is_loopback() || a.is_private() || a.is_link_local() || (x == 100 && (64..128).contains(&y))
        }
        IpAddr::V6(_) => false,
    }
}

fn broadcast_targets(port: u16) -> Vec<SocketAddr> {
    let mut out = vec![
        SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        SocketAddr::from((Ipv4Addr::BROADCAST, port)),
    ];
    if let Ok(ifs) = if_addrs::get_if_addrs() {
        for i in ifs {
            if let if_addrs::IfAddr::V4(v4) = i.addr
                && let Some(b) = v4.broadcast
                && !v4.ip.is_loopback()
            {
                out.push(SocketAddr::from((b, port)));
            }
        }
    }
    out
}

/// A background transfer. Dropping it cancels.
pub struct Job<E> {
    rx: Receiver<E>,
    cancel: Arc<AtomicBool>,
}

impl<E> Job<E> {
    /// What's happened since the last look.
    pub fn poll(&self) -> Vec<E> {
        self.rx.try_iter().collect()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl<E> Drop for Job<E> {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SendEvent {
    /// Ready for the other device, and where it can be reached by hand.
    Listening {
        addresses: Vec<SocketAddr>,
    },
    /// Someone tried a wrong code.
    WrongCode,
    Connected {
        peer: String,
    },
    Progress(f32),
    /// The other device has the file and it checked out.
    Done,
    /// The connection dropped mid-way; the code still works.
    Dropped,
    /// It stopped, for this reason.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum RecvEvent {
    Connected { peer: String },
    Progress(f32),
    Done { path: PathBuf, peer: String },
    Failed(RecvError),
}

#[derive(Clone, Debug, PartialEq)]
pub enum RecvError {
    /// Nothing answered.
    NotFound,
    /// Something answered, but the code didn't match.
    WrongCode,
    /// The other device runs a version that can't talk to this one.
    Incompatible,
    /// The connection dropped mid-way.
    Dropped,
    Other(String),
}

// --------------------------------------------------------------------- crypto

#[derive(Serialize, Deserialize)]
struct Meta {
    name: String,
    size: u64,
    sha256: String,
}

fn subkey(key: &[u8], label: &[u8]) -> ChaCha20Poly1305 {
    let mut h = Sha256::new();
    h.update(b"opentern-transfer ");
    h.update(label);
    h.update(key);
    let digest = h.finalize();
    ChaCha20Poly1305::new(Key::from_slice(&digest))
}

/// One direction of the encrypted channel: nonces count up, so a frame
/// can't be replayed, reordered or dropped unnoticed.
struct Seal {
    cipher: ChaCha20Poly1305,
    counter: u64,
}

impl Seal {
    fn new(key: &[u8], label: &[u8]) -> Seal {
        Seal {
            cipher: subkey(key, label),
            counter: 0,
        }
    }

    fn nonce(&mut self) -> Nonce {
        let mut n = [0u8; 12];
        n[4..].copy_from_slice(&self.counter.to_be_bytes());
        self.counter += 1;
        *Nonce::from_slice(&n)
    }

    fn seal(&mut self, plain: &[u8]) -> Option<Vec<u8>> {
        let nonce = self.nonce();
        self.cipher.encrypt(&nonce, plain).ok()
    }

    fn open(&mut self, sealed: &[u8]) -> Option<Vec<u8>> {
        let nonce = self.nonce();
        self.cipher.decrypt(&nonce, sealed).ok()
    }
}

fn identities() -> (Identity, Identity) {
    (Identity::new(b"opentern-receiver"), Identity::new(b"opentern-sender"))
}

// ------------------------------------------------------------------ the wire

enum WireError {
    Cancelled,
    Closed,
    Io,
}

/// A stream whose reads notice a cancel and give up after a silence.
struct Wire {
    stream: TcpStream,
    cancel: Arc<AtomicBool>,
    idle: Duration,
    /// Until the code is proved.
    limit: Option<Instant>,
}

impl Wire {
    fn new(stream: TcpStream, cancel: Arc<AtomicBool>, idle: Duration) -> std::io::Result<Wire> {
        stream.set_nonblocking(false)?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(Duration::from_millis(200)))?;
        stream.set_write_timeout(Some(IDLE))?;
        Ok(Wire {
            stream,
            cancel,
            idle,
            limit: Some(Instant::now() + HANDSHAKE_LIMIT),
        })
    }

    fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<(), WireError> {
        let mut last = Instant::now();
        while !buf.is_empty() {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(WireError::Cancelled);
            }
            if self.limit.is_some_and(|l| Instant::now() > l) {
                return Err(WireError::Io);
            }
            match self.stream.read(buf) {
                Ok(0) => return Err(WireError::Closed),
                Ok(n) => {
                    buf = &mut buf[n..];
                    last = Instant::now();
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                    ) =>
                {
                    if last.elapsed() > self.idle {
                        return Err(WireError::Io);
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::BrokenPipe
                    ) =>
                {
                    return Err(WireError::Closed);
                }
                Err(_) => return Err(WireError::Io),
            }
        }
        Ok(())
    }

    fn read_frame(&mut self, max: usize) -> Result<Vec<u8>, WireError> {
        let mut len = [0u8; 4];
        self.read_exact(&mut len)?;
        let len = u32::from_be_bytes(len) as usize;
        if len > max {
            return Err(WireError::Io);
        }
        let mut buf = vec![0u8; len];
        self.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn write_frame(&mut self, payload: &[u8]) -> Result<(), WireError> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(WireError::Cancelled);
        }
        let mut framed = Vec::with_capacity(payload.len() + 4);
        framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        framed.extend_from_slice(payload);
        self.stream.write_all(&framed).map_err(|_| WireError::Closed)
    }
}

/// Keeps a name printable and short: it arrives from the other device.
fn clean_name(raw: &str) -> String {
    // Text-direction marks could make one name read like another.
    let bidi = |c: char| matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
    let name: String = raw.chars().filter(|c| !c.is_control() && !bidi(*c)).take(48).collect();
    let name = name.trim().to_string();
    if name.is_empty() { "Other device".into() } else { name }
}

// --------------------------------------------------------------------- sender

enum Served {
    Done,
    Wrong,
    /// Not one of ours (a port scan, a stray connection): ignored.
    Stray,
    Dropped,
    Cancelled,
}

/// Offers `file` under `code` until someone takes it or it's cancelled.
pub fn send(file: PathBuf, code: String, name: String, port: u16, wake: impl Fn() + Send + 'static) -> Job<SendEvent> {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    let spare = tx.clone();
    let spawned = std::thread::Builder::new().name("transfer-send".into()).spawn(move || {
        let tell = move |e: SendEvent| {
            let _ = tx.send(e);
            wake();
        };
        send_loop(&file, &code, &name, port, &c, &tell);
    });
    if spawned.is_err() {
        let _ = spare.send(SendEvent::Failed("Couldn't start the transfer.".into()));
    }
    Job { rx, cancel }
}

fn send_loop(file: &Path, code: &str, name: &str, port: u16, cancel: &Arc<AtomicBool>, tell: &impl Fn(SendEvent)) {
    // The fixed port if it's free, so an address can be typed; any otherwise.
    let listener =
        TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).or_else(|_| TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)));
    let listener = match listener.and_then(|l| l.set_nonblocking(true).map(|()| l)) {
        Ok(l) => l,
        Err(e) => {
            tell(SendEvent::Failed(format!("Couldn't open a connection: {e}")));
            return;
        }
    };
    let tcp_port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    // Discovery can fail (port taken); a typed address still works.
    let udp = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)).ok();
    if let Some(u) = &udp {
        let _ = u.set_nonblocking(true);
        // macOS asks about the local network on the first traffic. Saying
        // hello now puts its question up while the person is here, not
        // later when the other device's first search arrives. Nobody acts on it.
        let _ = u.set_broadcast(true);
        for t in broadcast_targets(port) {
            let _ = u.send_to(HELLO, t);
        }
    }
    // So a device answering on two networks is only tried once.
    let mut session = [0u8; 4];
    let _ = getrandom::fill(&mut session);
    let mut reply = REPLY.to_vec();
    reply.extend_from_slice(&tcp_port.to_be_bytes());
    reply.extend_from_slice(&session);

    let addresses = local_addresses()
        .into_iter()
        .map(|a| SocketAddr::from((a, tcp_port)))
        .collect();
    tell(SendEvent::Listening { addresses });

    let started = Instant::now();
    let mut wrong = 0;
    let mut probe = [0u8; 64];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        if started.elapsed() > CODE_LIFE {
            tell(SendEvent::Failed("The code expired. Start again.".into()));
            return;
        }
        if let Some(u) = &udp {
            while let Ok((n, from)) = u.recv_from(&mut probe) {
                if &probe[..n] == PROBE && is_local(from.ip()) {
                    let _ = u.send_to(&reply, from);
                }
            }
        }
        match listener.accept() {
            // Dropping the stream closes it, unanswered.
            Ok((_, peer)) if !is_local(peer.ip()) => {}
            Ok((stream, _)) => match serve(stream, file, code, name, cancel, tell) {
                Served::Done => {
                    tell(SendEvent::Done);
                    return;
                }
                Served::Wrong => {
                    wrong += 1;
                    if wrong >= MAX_WRONG {
                        tell(SendEvent::Failed("Too many wrong codes. Start again.".into()));
                        return;
                    }
                    tell(SendEvent::WrongCode);
                }
                Served::Dropped => tell(SendEvent::Dropped),
                Served::Stray => {}
                Served::Cancelled => return,
            },
            Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(40)),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn serve(
    stream: TcpStream,
    file: &Path,
    code: &str,
    name: &str,
    cancel: &Arc<AtomicBool>,
    tell: &impl Fn(SendEvent),
) -> Served {
    let Ok(mut w) = Wire::new(stream, cancel.clone(), HANDSHAKE_IDLE) else {
        return Served::Stray;
    };
    let dropped = |e: WireError| match e {
        WireError::Cancelled => Served::Cancelled,
        _ => Served::Dropped,
    };
    let stray = |e: WireError| match e {
        WireError::Cancelled => Served::Cancelled,
        _ => Served::Stray,
    };
    let hello = match w.read_frame(256) {
        Ok(h) => h,
        Err(e) => return stray(e),
    };
    let Some(theirs) = hello.strip_prefix(MAGIC) else {
        // Another version of us says so; anything else just gets closed.
        if hello.starts_with(b"OPENTERN-TRANSFER/") {
            let _ = w.write_frame(&[0]);
        }
        return Served::Stray;
    };
    let (receiver, sender) = identities();
    let (state, mine) = Spake2::<Ed25519Group>::start_b(&Password::new(code.as_bytes()), &receiver, &sender);
    let mut answer = vec![1u8];
    answer.extend_from_slice(&mine);
    if let Err(e) = w.write_frame(&answer) {
        return stray(e);
    }
    let Ok(key) = state.finish(theirs) else {
        return Served::Stray;
    };
    let mut to_them = Seal::new(&key, b"sender");
    let mut from_them = Seal::new(&key, b"receiver");

    // Their proof of the code: their name, sealed with the shared key.
    let sealed = match w.read_frame(1024) {
        Ok(s) => s,
        Err(e) => return stray(e),
    };
    let Some(peer) = from_them.open(&sealed) else {
        return Served::Wrong;
    };
    let peer = clean_name(&String::from_utf8_lossy(&peer));

    let (size, sha256) = match file_digest(file) {
        Ok(d) => d,
        Err(_) => return Served::Dropped,
    };
    let meta = Meta {
        name: name.to_string(),
        size,
        sha256,
    };
    let Some(sealed) = serde_json::to_vec(&meta).ok().and_then(|m| to_them.seal(&m)) else {
        return Served::Dropped;
    };
    if let Err(e) = w.write_frame(&sealed) {
        return dropped(e);
    }
    tell(SendEvent::Connected { peer });
    w.idle = IDLE;
    w.limit = None;

    let Ok(mut f) = std::fs::File::open(file) else {
        return Served::Dropped;
    };
    let mut buf = vec![0u8; CHUNK];
    let mut sent = 0u64;
    let mut last_tell = Instant::now();
    while sent < size {
        let n = match f.read(&mut buf) {
            Ok(0) | Err(_) => return Served::Dropped,
            Ok(n) => n,
        };
        let Some(sealed) = to_them.seal(&buf[..n]) else {
            return Served::Dropped;
        };
        if let Err(e) = w.write_frame(&sealed) {
            return dropped(e);
        }
        sent += n as u64;
        if last_tell.elapsed() > Duration::from_millis(50) || sent == size {
            tell(SendEvent::Progress(sent as f32 / size.max(1) as f32));
            last_tell = Instant::now();
        }
    }
    let ack = match w.read_frame(64) {
        Ok(a) => a,
        Err(e) => return dropped(e),
    };
    match from_them.open(&ack) {
        Some(ok) if ok == b"ok" => Served::Done,
        _ => Served::Dropped,
    }
}

fn file_digest(path: &Path) -> std::io::Result<(u64, String)> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        size += n as u64;
    }
    Ok((size, hex(&h.finalize())))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ------------------------------------------------------------------- receiver

/// Where to look for the sender.
#[derive(Clone, Debug, PartialEq)]
pub enum Find {
    /// Ask the local network, on this discovery port.
    Search(u16),
    /// A typed address (`192.168.1.20` or `192.168.1.20:47811`).
    Address(String),
}

/// Fetches the file offered under `code` into `out`.
pub fn receive(
    code: String,
    find: Find,
    out: PathBuf,
    name: String,
    wake: impl Fn() + Send + 'static,
) -> Job<RecvEvent> {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    let spare = tx.clone();
    let spawned = std::thread::Builder::new()
        .name("transfer-receive".into())
        .spawn(move || {
            let tell = move |e: RecvEvent| {
                let _ = tx.send(e);
                wake();
            };
            let result = match find {
                Find::Search(port) => search(&code, port, &out, &name, &c, &tell),
                Find::Address(a) => direct(&a, &code, &out, &name, &c, &tell),
            };
            if let Some(e) = result {
                tell(RecvEvent::Failed(e));
            }
        });
    if spawned.is_err() {
        let _ = spare.send(RecvEvent::Failed(RecvError::Other(
            "Couldn't start the transfer.".into(),
        )));
    }
    Job { rx, cancel }
}

enum Attempt {
    /// It worked and has been reported.
    Finished,
    Failed(RecvError),
    Cancelled,
}

fn direct(
    address: &str,
    code: &str,
    out: &Path,
    name: &str,
    cancel: &Arc<AtomicBool>,
    tell: &impl Fn(RecvEvent),
) -> Option<RecvError> {
    let text = address.trim();
    let with_port = if text.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) {
        text.to_string()
    } else {
        format!("{text}:{PORT}")
    };
    let Some(addr) = with_port.to_socket_addrs().ok().and_then(|mut a| a.next()) else {
        return Some(RecvError::Other("That address doesn't look right.".into()));
    };
    if !is_local(addr.ip()) {
        return Some(RecvError::Other("That address isn't on your local network.".into()));
    }
    match attempt(addr, code, out, name, cancel, tell) {
        Attempt::Finished | Attempt::Cancelled => None,
        Attempt::Failed(e) => Some(e),
    }
}

fn search(
    code: &str,
    port: u16,
    out: &Path,
    name: &str,
    cancel: &Arc<AtomicBool>,
    tell: &impl Fn(RecvEvent),
) -> Option<RecvError> {
    let udp = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) {
        Ok(u) => u,
        Err(e) => return Some(RecvError::Other(format!("Couldn't reach the network: {e}"))),
    };
    let _ = udp.set_broadcast(true);
    let _ = udp.set_read_timeout(Some(Duration::from_millis(100)));
    let targets = broadcast_targets(port);
    let mut deadline = Instant::now() + SEARCH;
    let mut last_probe: Option<Instant> = None;
    let mut seen: HashSet<[u8; 4]> = HashSet::new();
    let mut failure = None;
    let mut tries = 0;
    let mut buf = [0u8; 64];
    while Instant::now() < deadline {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        if last_probe.is_none_or(|t| t.elapsed() > Duration::from_millis(500)) {
            for t in &targets {
                let _ = udp.send_to(PROBE, t);
            }
            last_probe = Some(Instant::now());
        }
        let Ok((n, from)) = udp.recv_from(&mut buf) else {
            continue;
        };
        let Some(rest) = buf[..n].strip_prefix(REPLY) else {
            continue;
        };
        let [p0, p1, s0, s1, s2, s3] = *rest else { continue };
        if !seen.insert([s0, s1, s2, s3]) {
            continue;
        }
        if !is_local(from.ip()) {
            continue;
        }
        let addr = SocketAddr::new(from.ip(), u16::from_be_bytes([p0, p1]));
        match attempt(addr, code, out, name, cancel, tell) {
            Attempt::Finished | Attempt::Cancelled => return None,
            Attempt::Failed(RecvError::WrongCode) => {
                tries += 1;
                if tries >= MAX_TRIES {
                    return Some(RecvError::WrongCode);
                }
                failure = Some(RecvError::WrongCode);
                deadline = deadline.min(Instant::now() + AFTER_WRONG);
            }
            // It answered but wouldn't connect, perhaps only for now (a
            // permission question on screen, another of its networks).
            Attempt::Failed(RecvError::NotFound) => {
                seen.remove(&[s0, s1, s2, s3]);
                failure = Some(RecvError::NotFound);
            }
            // The right device, but it went wrong: no point looking on.
            Attempt::Failed(e @ (RecvError::Dropped | RecvError::Other(_))) => return Some(e),
            Attempt::Failed(e) => failure = Some(e),
        }
    }
    Some(failure.unwrap_or(RecvError::NotFound))
}

fn attempt(
    addr: SocketAddr,
    code: &str,
    out: &Path,
    name: &str,
    cancel: &Arc<AtomicBool>,
    tell: &impl Fn(RecvEvent),
) -> Attempt {
    let stream = match TcpStream::connect_timeout(&addr, CONNECT) {
        Ok(s) => s,
        Err(_) => return Attempt::Failed(RecvError::NotFound),
    };
    let Ok(mut w) = Wire::new(stream, cancel.clone(), HANDSHAKE_IDLE) else {
        return Attempt::Failed(RecvError::Dropped);
    };
    let fail = |e: WireError, or: RecvError| match e {
        WireError::Cancelled => Attempt::Cancelled,
        _ => Attempt::Failed(or),
    };

    let (receiver, sender) = identities();
    let (state, mine) = Spake2::<Ed25519Group>::start_a(&Password::new(code.as_bytes()), &receiver, &sender);
    let mut hello = MAGIC.to_vec();
    hello.extend_from_slice(&mine);
    if let Err(e) = w.write_frame(&hello) {
        return fail(e, RecvError::Dropped);
    }
    let answer = match w.read_frame(256) {
        Ok(a) => a,
        Err(e) => return fail(e, RecvError::Dropped),
    };
    let theirs = match answer.split_first() {
        Some((1, theirs)) => theirs,
        _ => return Attempt::Failed(RecvError::Incompatible),
    };
    let Ok(key) = state.finish(theirs) else {
        return Attempt::Failed(RecvError::WrongCode);
    };
    let mut to_them = Seal::new(&key, b"receiver");
    let mut from_them = Seal::new(&key, b"sender");

    let Some(sealed) = to_them.seal(name.as_bytes()) else {
        return Attempt::Failed(RecvError::Dropped);
    };
    if let Err(e) = w.write_frame(&sealed) {
        return fail(e, RecvError::Dropped);
    }
    // A sender that can't read our proof hangs up: the codes differ.
    let sealed = match w.read_frame(4096) {
        Ok(s) => s,
        Err(e) => return fail(e, RecvError::WrongCode),
    };
    let Some(meta) = from_them
        .open(&sealed)
        .and_then(|m| serde_json::from_slice::<Meta>(&m).ok())
    else {
        return Attempt::Failed(RecvError::WrongCode);
    };
    if meta.size > MAX_FILE {
        return Attempt::Failed(RecvError::Other("That export is too big to receive.".into()));
    }
    let peer = clean_name(&meta.name);
    tell(RecvEvent::Connected { peer: peer.clone() });
    w.idle = IDLE;
    w.limit = None;

    let part = out.with_extension("part");
    let result = receive_body(&mut w, &mut from_them, &part, meta.size, &meta.sha256, tell);
    if let Err(a) = result {
        let _ = std::fs::remove_file(&part);
        return a;
    }
    if let Err(e) = std::fs::rename(&part, out) {
        let _ = std::fs::remove_file(&part);
        return Attempt::Failed(RecvError::Other(format!("Couldn't save the file: {e}")));
    }
    // Reported first: the file is safe here even if the "ok" goes astray.
    if let Some(ok) = to_them.seal(b"ok") {
        let _ = w.write_frame(&ok);
    }
    tell(RecvEvent::Done {
        path: out.to_path_buf(),
        peer,
    });
    Attempt::Finished
}

fn receive_body(
    w: &mut Wire,
    from_them: &mut Seal,
    part: &Path,
    size: u64,
    sha256: &str,
    tell: &impl Fn(RecvEvent),
) -> Result<(), Attempt> {
    if let Some(parent) = part.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Attempt::Failed(RecvError::Other(format!("Couldn't save the file: {e}"))))?;
    }
    let mut f = std::fs::File::create(part)
        .map_err(|e| Attempt::Failed(RecvError::Other(format!("Couldn't save the file: {e}"))))?;
    let mut h = Sha256::new();
    let mut got = 0u64;
    let mut last_tell = Instant::now();
    while got < size {
        let sealed = w.read_frame(MAX_FRAME).map_err(|e| match e {
            WireError::Cancelled => Attempt::Cancelled,
            _ => Attempt::Failed(RecvError::Dropped),
        })?;
        let Some(plain) = from_them.open(&sealed) else {
            return Err(Attempt::Failed(RecvError::Dropped));
        };
        if plain.is_empty() || got + plain.len() as u64 > size {
            return Err(Attempt::Failed(RecvError::Dropped));
        }
        f.write_all(&plain)
            .map_err(|e| Attempt::Failed(RecvError::Other(format!("Couldn't save the file: {e}"))))?;
        h.update(&plain);
        got += plain.len() as u64;
        if last_tell.elapsed() > Duration::from_millis(50) || got == size {
            tell(RecvEvent::Progress(got as f32 / size.max(1) as f32));
            last_tell = Instant::now();
        }
    }
    f.sync_all().ok();
    if hex(&h.finalize()) != sha256 {
        return Err(Attempt::Failed(RecvError::Dropped));
    }
    Ok(())
}

// ------------------------------------------------------------------- Windows

/// Letting a sender accept connections through Windows Firewall. Windows asks
/// by itself the first time a program listens, but a Cancel there saves a block
/// rule and it never asks again, and a network marked Public may stay blocked.
pub mod firewall {
    use super::PORT;
    use std::path::Path;
    use std::process::Command;

    pub const RULE: &str = "Opentern transfer";

    /// A program in Windows's own folder, by full path: what runs as
    /// administrator mustn't depend on the search path.
    fn system32(rel: &str) -> std::path::PathBuf {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        Path::new(&root).join("System32").join(rel)
    }

    fn quiet(program: &Path) -> Command {
        #[allow(unused_mut)]
        let mut c = Command::new(program);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            c.creation_flags(CREATE_NO_WINDOW);
        }
        c
    }

    fn netsh(args: &[&str]) -> Result<bool, String> {
        let mut c = quiet(&system32("netsh.exe"));
        c.args(["advfirewall", "firewall"]).args(args);
        c.output().map(|o| o.status.success()).map_err(|e| e.to_string())
    }

    /// Allows `program` in on the transfer port, on every kind of network.
    /// Needs administrator rights (`opentern --allow-firewall`).
    pub fn allow(program: &Path) -> Result<(), String> {
        let prog = format!("program={}", program.display());
        // A block beats any allow, so the program's old rules go first. This
        // reports failure when there are none, which is fine.
        let _ = netsh(&["delete", "rule", "name=all", &prog]);
        let (name, port) = (format!("name={RULE}"), format!("localport={PORT}"));
        for protocol in ["protocol=TCP", "protocol=UDP"] {
            let added = netsh(&[
                "add",
                "rule",
                &name,
                "dir=in",
                "action=allow",
                "enable=yes",
                "profile=any",
                &prog,
                protocol,
                &port,
            ])?;
            if !added {
                return Err("Windows refused the rule.".into());
            }
        }
        Ok(())
    }

    /// Runs a program as administrator (Windows's permission prompt) and
    /// returns its exit code. Declining the prompt is an error.
    pub fn run_elevated(program: &Path, args: &str) -> Result<i32, String> {
        let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
        let script = format!(
            "$ErrorActionPreference = 'Stop'; \
             $p = Start-Process -FilePath {} -ArgumentList {} -Verb RunAs -WindowStyle Hidden -Wait -PassThru; \
             if (-not $p) {{ exit 1 }}; exit $p.ExitCode",
            quote(&program.display().to_string()),
            quote(args),
        );
        let out = quiet(&system32(r"WindowsPowerShell\v1.0\powershell.exe"))
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &script,
            ])
            .output()
            .map_err(|e| e.to_string())?;
        out.status.code().ok_or_else(|| "It was stopped.".to_string())
    }

    /// Asks Windows to let this program accept transfers.
    pub fn ask_to_allow() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        match run_elevated(&exe, "--allow-firewall")? {
            0 => Ok(()),
            n => Err(format!("It ended with code {n}.")),
        }
    }

    /// Real rules, on CI's Windows machines only (administrators, and
    /// disposable). Checks Windows accepts exactly what's sent.
    #[cfg(windows)]
    #[test]
    fn windows_takes_the_rules_and_a_block_is_cleared() {
        if std::env::var_os("CI").is_none() {
            return;
        }
        let program = Path::new(r"C:\Program Files\Opentern Test\opentern's.exe");
        let prog = format!("program={}", program.display());
        let block = [
            "add",
            "rule",
            "name=Opentern test block",
            "dir=in",
            "action=block",
            &prog,
        ];
        assert!(netsh(&block).unwrap(), "couldn't add the block rule");
        allow(program).unwrap();
        assert!(
            !netsh(&["show", "rule", "name=Opentern test block"]).unwrap(),
            "block stayed"
        );
        let shown = quiet(&system32("netsh.exe"))
            .args([
                "advfirewall",
                "firewall",
                "show",
                "rule",
                &format!("name={RULE}"),
                "verbose",
            ])
            .output()
            .unwrap();
        let shown = String::from_utf8_lossy(&shown.stdout).to_string();
        let _ = netsh(&["delete", "rule", &format!("name={RULE}")]);
        assert!(shown.contains("opentern's.exe"), "{shown}");
        assert!(shown.contains("47811"), "{shown}");
        assert!(shown.contains("TCP") && shown.contains("UDP"), "{shown}");
    }

    #[cfg(windows)]
    #[test]
    fn elevated_programs_report_their_exit_code() {
        if std::env::var_os("CI").is_none() {
            return;
        }
        let cmd = std::path::PathBuf::from(std::env::var("ComSpec").unwrap());
        assert_eq!(run_elevated(&cmd, "/c exit 7"), Ok(7));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU16;

    static NEXT_PORT: AtomicU16 = AtomicU16::new(48100);

    fn port() -> u16 {
        NEXT_PORT.fetch_add(1, Ordering::Relaxed)
    }

    /// Waits for an event the test cares about, collecting the rest.
    fn wait_for<E: Clone + std::fmt::Debug>(job: &Job<E>, mut f: impl FnMut(&E) -> bool) -> Vec<E> {
        let start = Instant::now();
        let mut all = Vec::new();
        loop {
            for e in job.poll() {
                let hit = f(&e);
                all.push(e);
                if hit {
                    return all;
                }
            }
            assert!(start.elapsed() < Duration::from_secs(40), "timed out: {all:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn offer(dir: &Path, bytes: &[u8], code: &str, port: u16) -> Job<SendEvent> {
        let file = dir.join("offer.opentern");
        std::fs::write(&file, bytes).unwrap();
        let job = send(file, code.into(), "sender-box".into(), port, || {});
        wait_for(&job, |e| matches!(e, SendEvent::Listening { .. }));
        job
    }

    #[test]
    fn codes_are_six_digits() {
        let mut seen = HashSet::new();
        for _ in 0..200 {
            let c = new_code();
            assert_eq!(c.len(), CODE_LEN);
            assert!(c.bytes().all(|b| b.is_ascii_digit()));
            seen.insert(c);
        }
        assert!(seen.len() > 190);
    }

    #[test]
    fn a_file_arrives_intact_by_search() {
        let d = tempfile::tempdir().unwrap();
        // Bigger than a chunk, and not a whole number of them.
        let bytes: Vec<u8> = (0..CHUNK * 3 + 777).map(|i| (i * 31 % 251) as u8).collect();
        let p = port();
        let sending = offer(d.path(), &bytes, "123456", p);
        let out = d.path().join("in/received.opentern");
        let job = receive(
            "123456".into(),
            Find::Search(p),
            out.clone(),
            "receiver-box".into(),
            || {},
        );
        let events = wait_for(&job, |e| matches!(e, RecvEvent::Done { .. } | RecvEvent::Failed(_)));
        assert!(
            events.contains(&RecvEvent::Connected {
                peer: "sender-box".into()
            }),
            "{events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, RecvEvent::Done { peer, .. } if peer == "sender-box")),
            "{events:?}"
        );
        assert_eq!(std::fs::read(&out).unwrap(), bytes);
        assert!(!out.with_extension("part").exists());
        let s = wait_for(&sending, |e| matches!(e, SendEvent::Done));
        assert!(s.contains(&SendEvent::Connected {
            peer: "receiver-box".into()
        }));
    }

    #[test]
    fn a_typed_address_works_and_an_empty_file_is_fine() {
        let d = tempfile::tempdir().unwrap();
        let p = port();
        let sending = offer(d.path(), b"", "000042", p);
        let out = d.path().join("got.opentern");
        let job = receive(
            "000042".into(),
            Find::Address(format!("127.0.0.1:{p}")),
            out.clone(),
            "r".into(),
            || {},
        );
        wait_for(&job, |e| matches!(e, RecvEvent::Done { .. }));
        assert_eq!(std::fs::read(&out).unwrap(), b"");
        wait_for(&sending, |e| matches!(e, SendEvent::Done));
    }

    #[test]
    fn a_wrong_code_gets_no_data_and_the_right_one_still_works() {
        let d = tempfile::tempdir().unwrap();
        let p = port();
        let secret = b"SECRET-CONTENTS".repeat(100);
        let sending = offer(d.path(), &secret, "111111", p);
        let out = d.path().join("got.opentern");
        let job = receive("222222".into(), Find::Search(p), out.clone(), "r".into(), || {});
        let events = wait_for(&job, |e| matches!(e, RecvEvent::Failed(_) | RecvEvent::Done { .. }));
        assert_eq!(events.last(), Some(&RecvEvent::Failed(RecvError::WrongCode)));
        assert!(!out.exists());
        wait_for(&sending, |e| matches!(e, SendEvent::WrongCode));

        let job = receive("111111".into(), Find::Search(p), out.clone(), "r".into(), || {});
        wait_for(&job, |e| matches!(e, RecvEvent::Done { .. }));
        assert_eq!(std::fs::read(&out).unwrap(), secret);
    }

    #[test]
    fn too_many_wrong_codes_end_the_offer() {
        let d = tempfile::tempdir().unwrap();
        let p = port();
        let sending = offer(d.path(), b"x", "111111", p);
        for i in 0..MAX_WRONG {
            let job = receive(
                format!("99999{i}"),
                Find::Address(format!("127.0.0.1:{p}")),
                d.path().join("no.opentern"),
                "r".into(),
                || {},
            );
            wait_for(&job, |e| matches!(e, RecvEvent::Failed(_)));
        }
        let events = wait_for(&sending, |e| matches!(e, SendEvent::Failed(_)));
        assert!(events.contains(&SendEvent::Failed("Too many wrong codes. Start again.".into())));
        // Nobody's listening now, even with the right code.
        let job = receive(
            "111111".into(),
            Find::Address(format!("127.0.0.1:{p}")),
            d.path().join("no.opentern"),
            "r".into(),
            || {},
        );
        let ev = wait_for(&job, |e| matches!(e, RecvEvent::Failed(_)));
        assert_eq!(ev.last(), Some(&RecvEvent::Failed(RecvError::NotFound)));
    }

    #[test]
    fn nothing_listening_says_not_found() {
        let d = tempfile::tempdir().unwrap();
        let job = receive(
            "123456".into(),
            Find::Address(format!("127.0.0.1:{}", port())),
            d.path().join("no.opentern"),
            "r".into(),
            || {},
        );
        let ev = wait_for(&job, |e| matches!(e, RecvEvent::Failed(_)));
        assert_eq!(ev.last(), Some(&RecvEvent::Failed(RecvError::NotFound)));
    }

    #[test]
    fn a_stray_connection_doesnt_count_or_stop_the_offer() {
        let d = tempfile::tempdir().unwrap();
        let p = port();
        let sending = offer(d.path(), b"hello", "333333", p);
        // A port scan, garbage, and an oversized frame.
        drop(TcpStream::connect(("127.0.0.1", p)).unwrap());
        let mut s = TcpStream::connect(("127.0.0.1", p)).unwrap();
        s.write_all(&5u32.to_be_bytes()).unwrap();
        s.write_all(b"noise").unwrap();
        drop(s);
        let mut s = TcpStream::connect(("127.0.0.1", p)).unwrap();
        s.write_all(&u32::MAX.to_be_bytes()).unwrap();
        drop(s);
        let out = d.path().join("got.opentern");
        let job = receive(
            "333333".into(),
            Find::Address(format!("127.0.0.1:{p}")),
            out.clone(),
            "r".into(),
            || {},
        );
        wait_for(&job, |e| matches!(e, RecvEvent::Done { .. }));
        assert_eq!(std::fs::read(&out).unwrap(), b"hello");
        let events = wait_for(&sending, |e| matches!(e, SendEvent::Done));
        assert!(!events.contains(&SendEvent::WrongCode));
    }

    #[test]
    fn another_version_is_told_apart_from_a_wrong_code() {
        let d = tempfile::tempdir().unwrap();
        let p = port();
        let sending = offer(d.path(), b"x", "444444", p);
        let mut s = TcpStream::connect(("127.0.0.1", p)).unwrap();
        let hello = b"OPENTERN-TRANSFER/9 hello";
        s.write_all(&(hello.len() as u32).to_be_bytes()).unwrap();
        s.write_all(hello).unwrap();
        let mut w = Wire::new(s, Arc::new(AtomicBool::new(false)), Duration::from_secs(5)).unwrap();
        assert_eq!(w.read_frame(16).ok(), Some(vec![0]));
        drop(sending);
    }

    #[test]
    fn cancelling_stops_listening() {
        let d = tempfile::tempdir().unwrap();
        let p = port();
        let sending = offer(d.path(), b"x", "555555", p);
        drop(sending);
        let start = Instant::now();
        // The thread lets go of the port within a moment.
        loop {
            if TcpListener::bind((Ipv4Addr::UNSPECIFIED, p)).is_ok() {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_drop_mid_way_leaves_no_file_and_the_code_keeps_working() {
        let d = tempfile::tempdir().unwrap();
        let p = port();
        let bytes = vec![7u8; CHUNK * 40];
        let sending = offer(d.path(), &bytes, "666666", p);
        let out = d.path().join("got.opentern");
        // Cancel as soon as it has connected.
        let job = receive(
            "666666".into(),
            Find::Address(format!("127.0.0.1:{p}")),
            out.clone(),
            "r".into(),
            || {},
        );
        wait_for(&job, |e| matches!(e, RecvEvent::Connected { .. }));
        drop(job);
        wait_for(&sending, |e| matches!(e, SendEvent::Dropped));
        assert!(!out.exists());

        let job = receive(
            "666666".into(),
            Find::Address(format!("127.0.0.1:{p}")),
            out.clone(),
            "r".into(),
            || {},
        );
        wait_for(&job, |e| matches!(e, RecvEvent::Done { .. }));
        assert_eq!(std::fs::read(&out).unwrap().len(), bytes.len());
    }

    #[test]
    fn nothing_readable_crosses_the_wire() {
        // The sealed frames don't contain the plaintext, and tampering or
        // replaying one is refused.
        let key = [9u8; 32];
        let mut a = Seal::new(&key, b"sender");
        let mut b = Seal::new(&key, b"sender");
        let first = a.seal(b"SECRET-STRATEGY").unwrap();
        assert!(!first.windows(6).any(|w| w == b"SECRET"));
        let mut bad = first.clone();
        bad[0] ^= 1;
        assert!(Seal::new(&key, b"sender").open(&bad).is_none());
        assert_eq!(b.open(&first).unwrap(), b"SECRET-STRATEGY");
        assert!(b.open(&first).is_none(), "a replay fails");
        assert!(
            Seal::new(&key, b"receiver").open(&first).is_none(),
            "keys differ per direction"
        );
    }

    #[test]
    fn only_local_addresses_count() {
        for a in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.9",
            "192.168.1.20",
            "169.254.1.1",
            "100.100.1.1",
        ] {
            assert!(is_local(a.parse().unwrap()), "{a}");
        }
        for a in ["8.8.8.8", "172.32.0.1", "100.128.0.1", "1.1.1.1", "2001:db8::1", "::1"] {
            assert!(!is_local(a.parse().unwrap()), "{a}");
        }
    }

    #[test]
    fn a_typed_address_on_the_internet_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let job = receive(
            "123456".into(),
            Find::Address("8.8.8.8:47811".into()),
            d.path().join("no.opentern"),
            "r".into(),
            || {},
        );
        let ev = wait_for(&job, |e| matches!(e, RecvEvent::Failed(_)));
        assert_eq!(
            ev.last(),
            Some(&RecvEvent::Failed(RecvError::Other(
                "That address isn't on your local network.".into()
            )))
        );
    }

    /// Pretends to be senders: every answer is a new device, and each one
    /// that's reached gets to test one guess.
    #[test]
    fn a_search_stops_trying_after_a_few_impostors() {
        let d = tempfile::tempdir().unwrap();
        let p = port();
        let tcp = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let tcp_port = tcp.local_addr().unwrap().port();
        let udp = UdpSocket::bind(("127.0.0.1", p)).unwrap();
        udp.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        let tries = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (t2, s2) = (tries.clone(), stop.clone());
        std::thread::spawn(move || {
            tcp.set_nonblocking(true).unwrap();
            let mut session = 0u32;
            let mut buf = [0u8; 64];
            while !s2.load(Ordering::Relaxed) {
                if let Ok((_, from)) = udp.recv_from(&mut buf) {
                    for _ in 0..50 {
                        session += 1;
                        let mut reply = REPLY.to_vec();
                        reply.extend_from_slice(&tcp_port.to_be_bytes());
                        reply.extend_from_slice(&session.to_be_bytes());
                        let _ = udp.send_to(&reply, from);
                    }
                }
                if let Ok((s, _)) = tcp.accept() {
                    t2.fetch_add(1, Ordering::Relaxed);
                    let mut w = Wire::new(s, Arc::new(AtomicBool::new(false)), Duration::from_secs(2)).unwrap();
                    let hello = w.read_frame(256).ok();
                    let theirs = hello.as_deref().and_then(|h| h.strip_prefix(MAGIC)).map(<[u8]>::to_vec);
                    if let Some(theirs) = theirs {
                        let (r, o) = identities();
                        let (state, mine) = Spake2::<Ed25519Group>::start_b(&Password::new(b"000000"), &r, &o);
                        let mut answer = vec![1u8];
                        answer.extend_from_slice(&mine);
                        let _ = w.write_frame(&answer);
                        let _ = state.finish(&theirs);
                        let _ = w.read_frame(1024);
                    }
                    // Hanging up says the codes differ.
                }
            }
        });
        let job = receive(
            "123456".into(),
            Find::Search(p),
            d.path().join("no.opentern"),
            "r".into(),
            || {},
        );
        let ev = wait_for(&job, |e| matches!(e, RecvEvent::Failed(_) | RecvEvent::Done { .. }));
        stop.store(true, Ordering::Relaxed);
        assert_eq!(ev.last(), Some(&RecvEvent::Failed(RecvError::WrongCode)));
        assert_eq!(tries.load(Ordering::Relaxed), MAX_TRIES);
    }

    #[test]
    fn a_connection_that_never_proves_the_code_is_cut_off() {
        let (a, b) = {
            let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let a = TcpStream::connect(l.local_addr().unwrap()).unwrap();
            (a, l.accept().unwrap().0)
        };
        let mut w = Wire::new(a, Arc::new(AtomicBool::new(false)), Duration::from_secs(60)).unwrap();
        w.limit = Some(Instant::now() + Duration::from_millis(300));
        let start = Instant::now();
        assert!(w.read_frame(16).is_err());
        assert!(start.elapsed() < Duration::from_secs(5));
        drop(b);
    }

    #[test]
    fn the_hello_is_not_mistaken_for_a_search() {
        assert_ne!(HELLO, PROBE);
        assert!(!HELLO.starts_with(REPLY));
    }

    #[test]
    fn names_from_the_other_device_are_cleaned() {
        assert_eq!(clean_name("  my\nlaptop\u{7} "), "mylaptop");
        assert_eq!(clean_name(""), "Other device");
        assert_eq!(clean_name(&"x".repeat(100)).len(), 48);
        assert_eq!(clean_name("evil\u{202E}pot"), "evilpot");
    }
}

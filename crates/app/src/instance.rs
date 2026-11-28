//! One Opentern per workspace. The window that has it open holds `app.lock`
//! (see `daemon.rs`); a second launch asks that window to come forward and
//! quits, so two copies never save over each other's settings.
//!
//! The running window listens on a loopback port and writes the port and a
//! random token to `instance` in the workspace; only a request carrying
//! the token is obeyed. A restart (an update, a staged import) waits for
//! the closing window to let go instead of asking it to show.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const FILE: &str = "instance";
const NAME: &str = "opentern";
/// Set for the process a restart starts.
pub const RELAUNCH: &str = "OPENTERN_RELAUNCH";

/// The lock taken before the window opened, for `App::new`.
static LOCK: Mutex<Option<File>> = Mutex::new(None);
/// The window's context, so a request can wake it.
static CTX: OnceLock<egui::Context> = OnceLock::new();
static SHOW: AtomicBool = AtomicBool::new(false);

pub enum Outcome {
    /// This is the only window on the workspace.
    Only,
    /// Another window has it and was asked to come forward.
    Surfaced,
    /// Another window has it and didn't answer.
    Busy,
}

/// Claims the workspace before anything opens it.
/// `relaunch`: started by a restart of the window that has it.
pub fn claim(dir: &Path, relaunch: bool) -> Outcome {
    let start = Instant::now();
    loop {
        if let Some(lock) = crate::daemon::take_app_lock(dir) {
            *LOCK.lock().unwrap_or_else(|e| e.into_inner()) = Some(lock);
            listen(dir);
            return Outcome::Only;
        }
        // A restart: the window closing lets go within moments.
        if relaunch {
            if start.elapsed() > Duration::from_secs(15) {
                return Outcome::Busy;
            }
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        if ask(dir).is_ok() {
            return Outcome::Surfaced;
        }
        // It may be starting up and not listening yet.
        if start.elapsed() > Duration::from_secs(3) {
            return Outcome::Busy;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// The lock `claim` took, once.
pub fn take_lock() -> Option<File> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Called by the app each frame: whether a second launch asked to see it.
pub fn poll(ctx: &egui::Context) -> bool {
    let _ = CTX.set(ctx.clone());
    SHOW.swap(false, Ordering::Relaxed)
}

fn token() -> String {
    let mut b = [0u8; 16];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn listen(dir: &Path) {
    let Ok(listener) = TcpListener::bind(("127.0.0.1", 0)) else {
        return;
    };
    let Ok(addr) = listener.local_addr() else { return };
    let token = token();
    if std::fs::write(dir.join(FILE), format!("{} {token}\n", addr.port())).is_err() {
        return;
    }
    let _ = std::thread::Builder::new().name("instance".into()).spawn(move || {
        for conn in listener.incoming().flatten() {
            let _ = conn.set_read_timeout(Some(Duration::from_secs(1)));
            let mut line = String::new();
            let mut r = BufReader::new(&conn);
            if r.read_line(&mut line).is_err() {
                continue;
            }
            if line.trim_end() == format!("{NAME} show {token}") {
                SHOW.store(true, Ordering::Relaxed);
                if let Some(ctx) = CTX.get() {
                    ctx.request_repaint();
                }
                let _ = (&conn).write_all(b"ok\n");
            }
        }
    });
}

/// Asks the window that has `dir` open to come forward.
fn ask(dir: &Path) -> std::io::Result<()> {
    let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "no answer");
    let text = std::fs::read_to_string(dir.join(FILE))?;
    let (port, token) = text.trim().split_once(' ').ok_or_else(bad)?;
    let port: u16 = port.parse().map_err(|_| bad())?;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(500))?;
    s.set_read_timeout(Some(Duration::from_secs(1)))?;
    writeln!(s, "{NAME} show {token}")?;
    let mut reply = String::new();
    BufReader::new(&s).read_line(&mut reply)?;
    if reply.trim() == "ok" { Ok(()) } else { Err(bad()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_launch_brings_the_first_forward() {
        let dir = std::env::temp_dir().join(format!("opentern-instance-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(matches!(claim(&dir, false), Outcome::Only));
        let lock = take_lock().expect("lock");
        // The lock is held, so a second claim asks the first.
        assert!(ask(&dir).is_ok());
        assert!(SHOW.swap(false, Ordering::Relaxed));
        // A wrong token is ignored.
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        let port = text.split(' ').next().unwrap();
        std::fs::write(dir.join(FILE), format!("{port} nope\n")).unwrap();
        assert!(ask(&dir).is_err());
        assert!(!SHOW.load(Ordering::Relaxed));
        drop(lock);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

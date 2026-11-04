//! Alerts while Opentern is closed: `opentern --daemon` watches the
//! workspace's active alerts with no window, and sends them as the app
//! would (desktop notification, sound, webhook, Telegram, email).
//!
//! One evaluator at a time per workspace. The app holds `app.lock` while
//! it runs; the background process holds `alerts.lock`, looks at
//! `app.lock` a few times a second and leaves as soon as the app opens
//! (the app waits for it, then takes over). Closing the app starts it
//! again when "Keep alerts running" is on and an alert is active; "Start
//! at login" adds it to the system's login items. It exits by itself once
//! no alert is active.

use crate::alerts::Alerts;
use opentern_core::Store;
use opentern_data::Hub;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Settings: keep running in the background (default on), start at login
/// (default off). Values "on" / "off".
pub const BACKGROUND: &str = "alerts_background";
pub const AT_LOGIN: &str = "alerts_at_login";
/// Alerts that fired while the app was closed, for its notification.
pub const FIRED_AWAY: &str = "alerts_fired_away";

const APP_LOCK: &str = "app.lock";
const DAEMON_LOCK: &str = "alerts.lock";

pub fn background_on(store: &Store) -> bool {
    store.value(BACKGROUND).as_deref() != Some("off")
}

fn open_lock(dir: &Path, name: &str) -> Option<File> {
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(name))
        .ok()
}

/// Whether some process holds `name` in `dir` right now.
fn held(dir: &Path, name: &str) -> bool {
    let Some(f) = open_lock(dir, name) else { return false };
    match f.try_lock() {
        Ok(()) => {
            let _ = f.unlock();
            false
        }
        Err(_) => true,
    }
}

/// The app's side: holds `app.lock` for as long as it runs, after the
/// background process (if any) has stepped down. `None` when another
/// window already has this workspace open.
pub fn take_app_lock(dir: &Path) -> Option<File> {
    let f = open_lock(dir, APP_LOCK)?;
    f.try_lock().ok()?;
    // The background process checks every 250 ms; give it time to go so
    // the alerts the app loads are the latest.
    let start = Instant::now();
    while held(dir, DAEMON_LOCK) && start.elapsed() < Duration::from_millis(1500) {
        std::thread::sleep(Duration::from_millis(25));
    }
    Some(f)
}

/// Applies a staged import once nothing else is using the workspace: the
/// closing app lets go within moments, and the background process hands
/// over. If another window keeps it open, the import waits for a later start.
pub fn apply_pending_import(dir: &Path) {
    let start = Instant::now();
    loop {
        if let Some(lock) = take_app_lock(dir) {
            opentern_core::transfer::apply_pending(dir);
            drop(lock);
            return;
        }
        if start.elapsed() > Duration::from_secs(10) {
            crate::diag::note("IMPORT", "another window has the workspace open; the import waits");
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Starts the background process for `dir` (the app is closing).
pub fn spawn(dir: &Path, demo: bool) {
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--daemon").arg("--data-dir").arg(dir);
    if demo {
        cmd.arg("--demo");
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Its own process group: closing the terminal the app was started
    // from doesn't take it down.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    match cmd.spawn() {
        Ok(_) => crate::diag::crumb("alerts: background process started"),
        Err(e) => crate::diag::crumb(format!("alerts: couldn't start the background process: {e}")),
    }
}

/// `opentern --daemon`: watches the alerts until the app opens or none is
/// active. Returns the exit code.
pub fn run(dir: PathBuf, demo: bool) -> i32 {
    // The app is restarting to apply an import: it'll take over from here.
    if opentern_core::transfer::pending(&dir) {
        return 0;
    }
    let Some(lock) = open_lock(&dir, DAEMON_LOCK) else {
        return 1;
    };
    if lock.try_lock().is_err() {
        // Another one is already watching this workspace.
        return 0;
    }
    // Started by the closing app: wait for it to finish closing.
    let start = Instant::now();
    while held(&dir, APP_LOCK) {
        if start.elapsed() > Duration::from_secs(30) {
            return 0;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut store = match Store::open(&dir) {
        Ok(s) => s,
        Err(e) => {
            crate::diag::note("ALERTS", &format!("couldn't open the workspace: {e}"));
            return 1;
        }
    };
    if !background_on(&store) {
        return 0;
    }
    let scripts = crate::settings::Folders::load(&store)
        .scripts
        .unwrap_or_else(|| dir.join("scripts"));
    let mut alerts = Alerts::load(&store, scripts);
    if !alerts.list.iter().any(|(a, _)| a.active) {
        return 0;
    }
    let (wake_tx, wake_rx) = crossbeam_channel::bounded::<()>(1);
    let hub = Hub::start(
        dir.join("cache"),
        demo,
        std::sync::Arc::new(move || {
            let _ = wake_tx.try_send(());
        }),
    );
    // Keys for the providers that need one (the keychain).
    if !demo {
        let settings: Vec<(String, Option<String>)> = crate::dialogs::providers::SETTINGS
            .iter()
            .map(|k| (k.to_string(), store.value(k)))
            .collect();
        hub.send(opentern_data::Request::Credentials(
            crate::dialogs::providers::load_credentials(&settings),
        ));
    }
    crate::diag::crumb(format!(
        "alerts: watching {} in the background",
        alerts.list.iter().filter(|(a, _)| a.active).count()
    ));
    let mut fired_away: u64 = store.value(FIRED_AWAY).and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut last_app_check = Instant::now();
    loop {
        let _ = wake_rx.recv_timeout(Duration::from_millis(250));
        if last_app_check.elapsed() >= Duration::from_millis(200) {
            last_app_check = Instant::now();
            if held(&dir, APP_LOCK) {
                crate::diag::crumb("alerts: the app opened, handing over");
                break;
            }
        }
        for e in hub.poll() {
            let _ = alerts.watcher.apply(e);
        }
        let active: Vec<_> = alerts
            .list
            .iter()
            .filter(|(a, _)| a.active)
            .map(|(a, _)| a.clone())
            .collect();
        if active.is_empty() {
            crate::diag::crumb("alerts: none active, stopping");
            break;
        }
        alerts.watcher.sync(active.iter(), &|r| hub.send(r));
        let fire = alerts.evaluate(&mut store, |a, w, now| w.probe(a, now));
        for (i, msg) in fire {
            let price = alerts.watcher.price(&alerts.list[i].0.symbol).unwrap_or(f64::NAN);
            alerts.fire(&mut store, i, price, msg);
            fired_away += 1;
            let _ = store.set_value(FIRED_AWAY, &fired_away.to_string());
        }
    }
    alerts.flush();
    drop(lock);
    0
}

// ------------------------------------------------------------ at login

/// Adds or removes the background process from the system's login items.
pub fn set_at_login(dir: &Path, separate: bool, on: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut args = vec!["--daemon".to_string()];
    if separate {
        args.push("--data-dir".into());
        args.push(dir.display().to_string());
    }
    login::set(&exe, &args, on)
}

#[cfg(target_os = "linux")]
mod login {
    use std::path::{Path, PathBuf};

    fn file() -> Option<PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(config.join("autostart").join("opentern-alerts.desktop"))
    }

    fn quote(s: &str) -> String {
        if s.chars().any(|c| c.is_whitespace() || "\"'\\$`".contains(c)) {
            format!(
                "\"{}\"",
                s.replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('$', "\\$")
                    .replace('`', "\\`")
            )
        } else {
            s.to_string()
        }
    }

    pub fn set(exe: &Path, args: &[String], on: bool) -> Result<(), String> {
        let path = file().ok_or("No home folder.")?;
        if !on {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        let mut exec = quote(&exe.display().to_string());
        for a in args {
            exec.push(' ');
            exec.push_str(&quote(a));
        }
        let text = format!(
            "[Desktop Entry]\nType=Application\nName=Opentern alerts\nComment=Keeps your Opentern alerts running\nExec={exec}\nIcon=opentern\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n"
        );
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, text).map_err(|e| e.to_string())
    }

    #[cfg(test)]
    pub fn entry_path() -> Option<PathBuf> {
        file()
    }
}

#[cfg(target_os = "macos")]
mod login {
    use std::path::{Path, PathBuf};

    const LABEL: &str = "io.github.ahaan-shah.opentern.alerts";

    fn file() -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        Some(home.join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
    }

    fn esc(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    pub fn set(exe: &Path, args: &[String], on: bool) -> Result<(), String> {
        let path = file().ok_or("No home folder.")?;
        if !on {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        let mut items = format!("<string>{}</string>", esc(&exe.display().to_string()));
        for a in args {
            items.push_str(&format!("<string>{}</string>", esc(a)));
        }
        let text = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{LABEL}</string><key>ProgramArguments</key><array>{items}</array><key>RunAtLoad</key><true/></dict></plist>\n"
        );
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, text).map_err(|e| e.to_string())
    }
}

#[cfg(windows)]
mod login {
    use std::os::windows::process::CommandExt;
    use std::path::Path;

    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    const NAME: &str = "Opentern alerts";
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn set(exe: &Path, args: &[String], on: bool) -> Result<(), String> {
        let mut cmd = std::process::Command::new("reg");
        cmd.creation_flags(CREATE_NO_WINDOW);
        if on {
            let mut value = format!("\"{}\"", exe.display());
            for a in args {
                value.push_str(&format!(" \"{a}\""));
            }
            cmd.args(["add", KEY, "/v", NAME, "/t", "REG_SZ", "/d", &value, "/f"]);
        } else {
            cmd.args(["delete", KEY, "/v", NAME, "/f"]);
        }
        match cmd.output() {
            Ok(o) if o.status.success() || !on => Ok(()),
            Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod login {
    pub fn set(_: &std::path::Path, _: &[String], _: bool) -> Result<(), String> {
        Err("Not supported on this system.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_lock_shows_as_held_and_frees_on_drop() {
        let dir = std::env::temp_dir().join(format!("opentern-locks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!held(&dir, APP_LOCK));
        let lock = take_app_lock(&dir).expect("lock");
        assert!(held(&dir, APP_LOCK));
        // A second window on the same workspace doesn't get it.
        assert!(take_app_lock(&dir).is_none());
        drop(lock);
        assert!(!held(&dir, APP_LOCK));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The whole loop on demo data: an alert that's already true fires
    /// once, is logged and switched off, and the process stops.
    #[test]
    fn fires_in_the_background_and_stops_when_nothing_is_left() {
        use opentern_engine::alerts::{Alert, Condition, Freq, Op};
        // SAFETY: tests that read OPENTERN_HEADLESS all want it set.
        unsafe { std::env::set_var("OPENTERN_HEADLESS", "1") };
        let dir = std::env::temp_dir().join(format!("opentern-daemon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = Store::open(&dir).unwrap();
        let a = Alert {
            name: "Above zero".into(),
            symbol: "BINANCE:BTCUSDT".into(),
            tf: "1h".into(),
            condition: Condition::Price {
                op: Op::GreaterThan,
                value: 0.0,
            },
            freq: Freq::Once,
            sym: opentern_core::starter_watchlist().into_iter().next(),
            ..Alert::default()
        };
        store.save_alert(0, &serde_json::to_string(&a).unwrap()).unwrap();
        drop(store);
        let (tx, rx) = crossbeam_channel::bounded(1);
        let d = dir.clone();
        std::thread::spawn(move || {
            let _ = tx.send(run(d, true));
        });
        let code = rx
            .recv_timeout(Duration::from_secs(20))
            .expect("still running after 20 s");
        assert_eq!(code, 0);
        let store = Store::open(&dir).unwrap();
        assert_eq!(store.alert_log(10).len(), 1);
        let (_, json) = store.alerts().remove(0);
        let saved: Alert = serde_json::from_str(&json).unwrap();
        assert!(!saved.active);
        assert_eq!(saved.fired, 1);
        assert_eq!(store.value(FIRED_AWAY).as_deref(), Some("1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn login_entry_is_written_and_removed() {
        let home = std::env::temp_dir().join(format!("opentern-login-{}", std::process::id()));
        // SAFETY: this test is the only one reading XDG_CONFIG_HOME.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &home) };
        let exe = Path::new("/opt/Open tern/opentern");
        login::set(exe, &["--daemon".into()], true).unwrap();
        let path = login::entry_path().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Exec=\"/opt/Open tern/opentern\" --daemon"), "{text}");
        login::set(exe, &[], false).unwrap();
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&home);
    }
}

//! Update checks and installs, run off the UI thread. Opentern looks for a new
//! release on launch (unless turned off in Settings) and when asked, but only
//! downloads and installs when the person clicks Update, and only restarts
//! when they click Restart.

use crate::i18n::{msg, nt};
use opentern_core::update::{self, Plan, Release, Step};
use std::sync::mpsc;

/// The spinner shows for at least this long, so a fast check still reads as
/// "checked" rather than a flicker.
const MIN_CHECK: f64 = 0.9;
/// Launch check delay: let the first frames and data load settle first.
const LAUNCH_DELAY: f64 = 2.5;

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    /// Can't install in place here; offer the release page instead.
    Manual(Release, String),
    Installing(Release),
    /// An install that didn't finish: why, in words, and Try again.
    Retry(Release, String),
    /// Installed; takes effect after a restart.
    Ready(Release),
    Failed(String),
}

enum Msg {
    Checked(Result<Option<Release>, String>),
    Step(Step),
    Installed(Result<(), String>),
}

pub struct Updater {
    pub phase: Phase,
    pub progress: f32,
    /// Where the install under way has got to.
    pub step: Step,
    rx: Option<mpsc::Receiver<Msg>>,
    started: f64,
    /// A check result waiting out MIN_CHECK.
    held: Option<Result<Option<Release>, String>>,
    launch_done: bool,
    manual: bool,
    restart_on_exit: bool,
    /// Restarting the same version: not an update, so no What's new.
    relaunch_only: bool,
    /// `OPENTERN_STILL=update:install`: install what the check finds, as a
    /// click on Update would (end-to-end runs against a test feed).
    pub install_found: bool,
    /// Where this copy was launched from, read before any update swaps it:
    /// afterwards Linux reports the renamed old file ("….old (deleted)").
    exe: Option<std::path::PathBuf>,
}

impl Default for Updater {
    fn default() -> Self {
        update::cleanup();
        Updater {
            phase: Phase::Idle,
            progress: 0.0,
            step: Step::Downloading(0.0),
            rx: None,
            started: 0.0,
            held: None,
            launch_done: false,
            manual: false,
            restart_on_exit: false,
            relaunch_only: false,
            install_found: false,
            exe: std::env::current_exe().ok(),
        }
    }
}

impl Updater {
    pub fn busy(&self) -> bool {
        matches!(self.phase, Phase::Checking | Phase::Installing(_))
    }

    /// Whether the sidebar should show the update / restart prompt.
    pub fn prompt(&self) -> bool {
        matches!(
            self.phase,
            Phase::Available(_) | Phase::Manual(..) | Phase::Installing(_) | Phase::Retry(..) | Phase::Ready(_)
        )
    }

    /// The once-per-launch background check.
    pub fn launch_check(&mut self, ctx: &egui::Context, enabled: bool) {
        if self.launch_done || crate::app::headless() {
            return;
        }
        let now = ctx.input(|i| i.time);
        if now < LAUNCH_DELAY {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(LAUNCH_DELAY - now + 0.05));
            return;
        }
        self.launch_done = true;
        // Never over an update already under way, finished or waiting to be tried again.
        if enabled && self.phase == Phase::Idle {
            self.check(ctx, false);
        }
    }

    /// Looks for a newer release. `manual` checks report failures; the
    /// launch check stays quiet if offline.
    pub fn check(&mut self, ctx: &egui::Context, manual: bool) {
        if self.busy() || matches!(self.phase, Phase::Ready(_)) {
            return;
        }
        self.manual = manual;
        self.started = ctx.input(|i| i.time);
        self.held = None;
        if manual {
            self.phase = Phase::Checking;
        }
        let (tx, rx) = mpsc::channel();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let r = update::check().map_err(|e| match e {
                opentern_core::Error::Http(m) if m == update::OFFLINE => {
                    nt!("Couldn't check. Make sure you are connected to the internet.").to_string()
                }
                e => {
                    crate::diag::crumb(format!("update check failed: {e}"));
                    nt!("Couldn't check for updates right now. Try again later.").to_string()
                }
            });
            let _ = tx.send(Msg::Checked(r));
            ctx2.request_repaint();
        });
        self.rx = Some(rx);
        crate::diag::crumb(if manual {
            "update check"
        } else {
            "update check (launch)"
        });
    }

    /// Downloads and installs the available release. Only ever called from
    /// a click.
    pub fn install(&mut self, ctx: &egui::Context) {
        let (Phase::Available(release) | Phase::Retry(release, _)) = self.phase.clone() else {
            return;
        };
        self.phase = Phase::Installing(release.clone());
        self.progress = 0.0;
        self.step = Step::Downloading(0.0);
        let (tx, rx) = mpsc::channel();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let tx2 = tx.clone();
            let ctx3 = ctx2.clone();
            let r = update::install(&release, &move |s| {
                let _ = tx2.send(Msg::Step(s));
                ctx3.request_repaint();
            })
            .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Installed(r));
            ctx2.request_repaint();
        });
        self.rx = Some(rx);
        crate::diag::crumb("update install");
    }

    /// Closes Opentern and starts the new version once this one has exited.
    pub fn restart(&mut self, ctx: &egui::Context) {
        if matches!(self.phase, Phase::Ready(_)) {
            self.restart_on_exit = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Restarts the same version (an import was staged).
    pub fn relaunch(&mut self, ctx: &egui::Context) {
        self.restart_on_exit = true;
        self.relaunch_only = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    #[cfg(test)]
    pub fn relaunching(&self) -> bool {
        self.relaunch_only && self.restart_on_exit
    }

    /// Called from `App::on_exit`, after everything is saved.
    pub fn on_exit(&self) {
        if !self.restart_on_exit {
            return;
        }
        let Some(exe) = self.exe.clone() else { return };
        #[cfg(target_os = "macos")]
        {
            // Relaunch the bundle so macOS treats it as the same app.
            if let Some(app) = exe
                .ancestors()
                .nth(3)
                .filter(|a| a.extension().is_some_and(|e| e == "app"))
            {
                let mut open = std::process::Command::new("open");
                open.arg("-n").arg(app);
                // `open` passes on nothing by itself, and a staged import
                // lives in the workspace this run was given.
                let args: Vec<_> = std::env::args_os().skip(1).collect();
                if !args.is_empty() {
                    open.arg("--args").args(args);
                }
                let _ = open.spawn();
                return;
            }
        }
        let mut cmd = std::process::Command::new(exe);
        cmd.args(std::env::args_os().skip(1));
        if self.relaunch_only {
            // A screenshot's instructions are for the run that got them.
            cmd.env_remove("OPENTERN_STILL");
        } else {
            // So the new version can say what's new right away.
            cmd.env("OPENTERN_UPDATED_FROM", env!("CARGO_PKG_VERSION"));
        }
        let _ = cmd.spawn();
    }

    /// Applies results from the worker thread. Returns a message for a toast
    /// when something worth saying happened.
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<Result<String, String>> {
        let mut note = None;
        if let Some(rx) = &self.rx {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    Msg::Checked(r) => self.held = Some(r),
                    Msg::Step(s) => {
                        self.step = s;
                        if let Step::Downloading(p) = s {
                            self.progress = p;
                        }
                    }
                    Msg::Installed(Ok(())) => {
                        if let Phase::Installing(r) = &self.phase {
                            note = Some(Ok(msg!("Opentern {} is installed. Restart to use it.", r.version)));
                            self.phase = Phase::Ready(r.clone());
                        }
                    }
                    Msg::Installed(Err(e)) => {
                        crate::diag::crumb(format!("update failed: {e}"));
                        let why = reason(&e);
                        note = Some(Err(msg!("Couldn't update: {e}", e = why)));
                        if let Phase::Installing(r) = &self.phase {
                            self.phase = Phase::Retry(r.clone(), why);
                        }
                    }
                }
            }
        }
        if let Some(held) = self.held.take() {
            let now = ctx.input(|i| i.time);
            if self.manual && now - self.started < MIN_CHECK {
                self.held = Some(held);
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(MIN_CHECK - (now - self.started)));
            } else {
                self.rx = None;
                self.phase = match held {
                    Ok(Some(r)) => match update::plan() {
                        Plan::Replace { .. } => Phase::Available(r),
                        Plan::Manual(why) => Phase::Manual(r, why),
                    },
                    Ok(None) => {
                        if self.manual {
                            Phase::UpToDate
                        } else {
                            Phase::Idle
                        }
                    }
                    Err(e) => {
                        crate::diag::crumb(format!("update check failed: {e}"));
                        if self.manual { Phase::Failed(e) } else { Phase::Idle }
                    }
                };
            }
        }
        if self.install_found && matches!(self.phase, Phase::Available(_)) {
            self.install_found = false;
            self.install(ctx);
        }
        if self.busy() {
            ctx.request_repaint();
        }
        note
    }
}

/// An update error in words people use. The core's are technical ("network:
/// offline", "Disk quota exceeded (os error 122)").
pub fn reason(e: &str) -> String {
    let l = e.to_lowercase();
    if l.contains(update::OFFLINE) {
        nt!("GitHub couldn't be reached. Check your connection and try again.").into()
    } else if l.contains("quota") || l.contains("no space") || l.contains("os error 28") {
        nt!("There isn't room on this computer to download it.").into()
    } else if l.contains("checksum") || l.contains("corrupted") {
        nt!("The download came through damaged. Try again.").into()
    } else if l.contains("permission denied") || l.contains("os error 13") {
        nt!("Opentern isn't allowed to replace itself where it's installed.").into()
    } else {
        e.trim_start_matches("network: ").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::reason;

    #[test]
    fn errors_read_as_words() {
        assert!(reason("network: offline").starts_with("GitHub couldn't be reached"));
        assert!(reason("Disk quota exceeded (os error 122)").starts_with("There isn't room"));
        assert!(reason("the download is corrupted (checksum mismatch)").starts_with("The download came"));
        assert_eq!(reason("network: http status: 500"), "http status: 500");
    }
}

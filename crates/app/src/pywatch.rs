//! The one-time notice about Python: a watch on the scripts folder (and
//! every folder in it) that fires the moment a `.py` file appears there,
//! whoever put it there (the editor, an agent, a copy). Once the notice has
//! shown, a flag file in the per-machine config folder keeps it from ever
//! showing again on this computer, in any workspace.

use notify::{EventKind, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The flag file's name in `opentern_core::config_dir()`.
const FLAG: &str = "python-notice-shown";

pub struct PyWatch {
    /// The folder watched (rebuilt when Settings moves the scripts folder).
    pub dir: PathBuf,
    /// Written once the notice shows.
    pub flag: PathBuf,
    /// A `.py` turned up.
    hit: Arc<AtomicBool>,
    /// A folder turned up (moved or copied in whole): look inside it.
    rescan: Arc<AtomicBool>,
    _watcher: Option<notify::RecommendedWatcher>,
}

impl PyWatch {
    /// The watch for a launch, unless the notice has shown on this computer
    /// or nobody would see it (headless, stills, tests).
    pub fn at_launch(dir: PathBuf, ctx: &egui::Context) -> Option<PyWatch> {
        if crate::app::headless() || std::env::var("OPENTERN_STILL").is_ok() {
            return None;
        }
        let flag = opentern_core::config_dir()?.join(FLAG);
        if flag.exists() {
            return None;
        }
        Some(PyWatch::start(dir, flag, ctx))
    }

    /// Watches `dir` (made if missing: a new workspace has none yet) and
    /// looks once for a `.py` already there.
    pub fn start(dir: PathBuf, flag: PathBuf, ctx: &egui::Context) -> PyWatch {
        let hit = Arc::new(AtomicBool::new(has_python(&dir)));
        let rescan = Arc::new(AtomicBool::new(false));
        let _ = std::fs::create_dir_all(&dir);
        let (h, r, c) = (hit.clone(), rescan.clone(), ctx.clone());
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(e) = res else { return };
            if matches!(e.kind, EventKind::Access(_) | EventKind::Remove(_)) {
                return;
            }
            for p in &e.paths {
                if is_python(p) {
                    h.store(true, Ordering::Relaxed);
                } else if p.is_dir() {
                    r.store(true, Ordering::Relaxed);
                } else {
                    continue;
                }
                c.request_repaint();
            }
        })
        .ok()
        .and_then(|mut w| w.watch(&dir, RecursiveMode::Recursive).ok().map(|_| w));
        PyWatch {
            dir,
            flag,
            hit,
            rescan,
            _watcher: watcher,
        }
    }

    /// Whether a `.py` is there now.
    pub fn found(&self) -> bool {
        if self.rescan.swap(false, Ordering::Relaxed) && has_python(&self.dir) {
            self.hit.store(true, Ordering::Relaxed);
        }
        self.hit.load(Ordering::Relaxed)
    }
}

fn is_python(p: &Path) -> bool {
    p.extension().is_some_and(|x| x.eq_ignore_ascii_case("py"))
}

fn has_python(dir: &Path) -> bool {
    crate::scripting::scan_scripts(dir)
        .0
        .iter()
        .any(|f| is_python(Path::new(f)))
}

impl crate::app::App {
    /// Shows the notice once a `.py` is in the scripts folder, then stops
    /// watching and writes the flag. Waits for other dialogs and onboarding.
    pub(crate) fn python_notice(&mut self) {
        let Some(w) = &self.python_notice else { return };
        let dir = self.scripts_dir();
        if w.dir != dir {
            let flag = w.flag.clone();
            self.python_notice = Some(PyWatch::start(dir, flag, &self.egui_ctx));
            return;
        }
        if !w.found() || self.modal.is_some() || self.onboarding.is_some() {
            return;
        }
        if let Some(d) = w.flag.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&w.flag, "");
        self.python_notice = None;
        self.modal = Some(crate::dialogs::Modal::PythonNotice);
        self.egui_ctx.request_repaint();
    }
}

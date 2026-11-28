//! Opentern's engine: market data model, persistence, settings and updates.
//! Nothing in here knows about the UI.

pub mod bartime;
pub mod calendar;
pub mod db;
#[doc(hidden)]
pub mod fuzzing;
pub mod handoff;
pub mod model;
pub mod secrets;
pub mod store;
pub mod transfer;
pub mod update;
mod venue_days;

pub use model::*;
pub use store::{Settings, Store};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("network: {0}")]
    Http(String),
    #[error("{0}")]
    Msg(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Whether this is a development copy: the binary is named
/// `opentern-dev` (what `scripts/dev-install.sh` installs). It keeps its own
/// workspace, config and keychain entries, has its own window class, and
/// never updates itself
/// (an update would put the release over it).
pub fn dev_copy() -> bool {
    static DEV: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DEV.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_stem().map(|s| s == "opentern-dev"))
            .unwrap_or(false)
    })
}

/// The app's name for its folders and window: "opentern", or
/// "opentern-dev" for a development copy.
pub fn app_name() -> &'static str {
    if dev_copy() { "opentern-dev" } else { "opentern" }
}

/// Opentern's per-machine config folder (`~/.config/opentern` on Linux,
/// the Application Support / AppData equivalents elsewhere): for flags
/// that belong to the computer rather than to one workspace.
pub fn config_dir() -> Option<std::path::PathBuf> {
    directories::ProjectDirs::from("dev", "opentern", app_name()).map(|d| d.config_dir().to_path_buf())
}

/// Where Opentern keeps the workspace of whoever is using it:
/// `~/.local/share/opentern` (`opentern-dev` for a development copy) on Linux, `~/Library/Application Support/opentern`
/// on macOS, `%APPDATA%\opentern` on Windows. `OPENTERN_DATA_DIR` overrides it
/// (for demos and tests). Never hardcode a path anywhere else.
pub fn data_dir() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("OPENTERN_DATA_DIR") {
        return p.into();
    }
    directories::ProjectDirs::from("dev", "opentern", app_name())
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from(".opentern"))
}

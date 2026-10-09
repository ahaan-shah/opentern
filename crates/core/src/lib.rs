//! Opentern's engine: market data model, persistence, settings and updates.
//! Nothing in here knows about the UI.

pub mod db;
pub mod model;
pub mod store;
pub mod update;

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

/// Where Opentern keeps the workspace of whoever is using it:
/// `~/.local/share/opentern` on Linux, `~/Library/Application Support/opentern`
/// on macOS, `%APPDATA%\opentern` on Windows. `OPENTERN_DATA_DIR` overrides it
/// (for demos and tests). Never hardcode a path anywhere else.
pub fn data_dir() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("OPENTERN_DATA_DIR") {
        return p.into();
    }
    directories::ProjectDirs::from("dev", "opentern", "opentern")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from(".opentern"))
}

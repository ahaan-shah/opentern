//! Moving a workspace to another device: the `.opentern` export file.
//!
//! The file is a zip of versioned JSON and script files, never the raw
//! database, so devices on different versions of Opentern can exchange it and
//! nothing machine-specific travels (folders, login items, agent accounts).
//! API keys and passwords stay in the keychain and are never exported.
//!
//! Importing changes a running app's every panel, so it happens at startup:
//! the app verifies the file, saves a backup, stages the import
//! ([`stage`]) and restarts; `main` applies it before the workspace opens
//! ([`apply_pending`]).

use crate::{Error, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

/// The file's format. Bump it only for a change older apps can't read:
/// they refuse the file and ask for an update.
pub const FORMAT: u32 = 1;
pub const EXTENSION: &str = "opentern";
/// What a file may unpack to, so a hostile one can't fill the disk.
const MAX_UNPACKED: u64 = 1 << 30;
/// Backups kept in `<workspace>/backups`.
const KEEP_BACKUPS: usize = 5;
/// The extensions the Script Editor lists (`scripting::Lang::of`).
const SCRIPT_EXTENSIONS: [&str; 6] = ["py", "pine", "luau", "lua", "js", "ts"];
/// Telegram and email details an alert delivers through. The bot token and
/// the mail password are in the keychain and stay there.
const ALERT_SETTINGS: [&str; 5] = [
    "telegram_chat_id",
    "email_smtp_host",
    "email_smtp_port",
    "email_smtp_user",
    "email_to",
];
const PROPS_PREFIX: &str = "strategy_props:";
/// The outcome of the last import, for the app to report on launch.
pub const RESULT_KEY: &str = "import_result";

const MANIFEST: &str = "manifest.json";
/// Entries in a file: well past anyone's real workspace.
const MAX_ENTRIES: usize = 100_000;
/// Fingerprints of imported Python scripts nobody has agreed to run yet.
/// Python has full access to the computer, so a file (which anyone could have
/// made or changed) must never start one by itself, say from a saved layout.
/// By content, so no spelling of the path gets past it. Never exported.
pub const PYTHON_HELD_KEY: &str = "python_held";

/// What can be picked for export or import.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Item {
    Watchlists,
    Layouts,
    Drawings,
    Templates,
    Favorites,
    Alerts,
    Indices,
    Paper,
    Scripts,
    Screener,
    Lab,
}

impl Item {
    pub const ALL: [Item; 11] = [
        Item::Watchlists,
        Item::Layouts,
        Item::Drawings,
        Item::Templates,
        Item::Favorites,
        Item::Alerts,
        Item::Indices,
        Item::Paper,
        Item::Scripts,
        Item::Screener,
        Item::Lab,
    ];

    fn key(self) -> &'static str {
        match self {
            Item::Watchlists => "watchlists",
            Item::Layouts => "layouts",
            Item::Drawings => "drawings",
            Item::Templates => "templates",
            Item::Favorites => "favorites",
            Item::Alerts => "alerts",
            Item::Indices => "indices",
            Item::Paper => "paper",
            Item::Scripts => "scripts",
            Item::Screener => "screener",
            Item::Lab => "lab",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Item::Watchlists => "Watchlists",
            Item::Layouts => "Layouts",
            Item::Drawings => "Drawings",
            Item::Templates => "Templates",
            Item::Favorites => "Favorites",
            Item::Alerts => "Alerts",
            Item::Indices => "Custom indices",
            Item::Paper => "Paper accounts",
            Item::Scripts => "Scripts",
            Item::Screener => "Screener settings",
            Item::Lab => "Strategy Lab settings",
        }
    }

    /// Settings are one saved thing each: no count worth showing.
    pub fn is_setting(self) -> bool {
        matches!(self, Item::Screener | Item::Lab)
    }

    fn from_key(key: &str) -> Option<Item> {
        Item::ALL.into_iter().find(|i| i.key() == key)
    }
}

/// How an import treats what's already on this device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// Keeps what's here and updates anything with the same name.
    #[default]
    Merge,
    /// The picked items become exactly what's in the file.
    Replace,
}

/// The file's table of contents, readable without unpacking the rest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    /// The Opentern version that made it.
    pub app: String,
    /// When (ms).
    pub created: i64,
    /// Item key → how many. Unknown keys (from a newer app) are ignored.
    pub items: BTreeMap<String, usize>,
    /// Python scripts among them: they run with this computer's full access.
    #[serde(default)]
    pub python: usize,
}

impl Manifest {
    /// The items in the file that have anything in them, in display order.
    pub fn present(&self) -> Vec<(Item, usize)> {
        Item::ALL
            .into_iter()
            .filter_map(|i| self.items.get(i.key()).map(|n| (i, *n)))
            .filter(|(_, n)| *n > 0)
            .collect()
    }
}

// ------------------------------------------------------------------- payloads

#[derive(Serialize, Deserialize)]
struct Row {
    id: i64,
    name: String,
    json: String,
    #[serde(default)]
    updated: i64,
}

#[derive(Serialize, Deserialize)]
struct Lists {
    /// The name of the one in use.
    active: Option<String>,
    rows: Vec<Row>,
}

#[derive(Serialize, Deserialize)]
struct DrawingRow {
    symbol: String,
    json: String,
}

#[derive(Serialize, Deserialize)]
struct TemplateRow {
    kind: String,
    name: String,
    json: String,
    is_default: bool,
}

#[derive(Serialize, Deserialize)]
struct DefaultRow {
    study: String,
    json: String,
}

#[derive(Serialize, Deserialize)]
struct Templates {
    templates: Vec<TemplateRow>,
    defaults: Vec<DefaultRow>,
}

#[derive(Serialize, Deserialize)]
struct FavoriteRow {
    kind: String,
    key: String,
    pos: i64,
}

#[derive(Serialize, Deserialize)]
struct Alerts {
    rows: Vec<String>,
    settings: BTreeMap<String, String>,
}

/// Paper accounts as the app saves them, opaque apart from `name`.
#[derive(Serialize, Deserialize)]
struct PaperSaved {
    accounts: Vec<serde_json::Value>,
    active: usize,
}

#[derive(Serialize, Deserialize)]
struct ScriptsMeta {
    /// Script file → its Strategy settings overrides.
    props: BTreeMap<String, String>,
}

// -------------------------------------------------------------------- reading

fn value(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0))
        .optional()?)
}

/// The scripts folder of the workspace at `dir` (Settings → General may
/// have moved it).
fn scripts_dir(conn: &Connection, dir: &Path) -> Result<PathBuf> {
    Ok(value(conn, "scripts_dir")?
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join("scripts")))
}

fn is_script(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SCRIPT_EXTENSIONS.contains(&e))
}

/// Script files under `root` as relative `/` paths, by the same rules as
/// the Script Editor's list.
fn script_files(root: &Path) -> Vec<String> {
    fn walk(root: &Path, rel: &str, depth: usize, out: &mut Vec<String>) {
        if depth > 8 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(root.join(rel)) else {
            return;
        };
        for e in rd.flatten() {
            let Ok(name) = e.file_name().into_string() else {
                continue;
            };
            if name.starts_with('.') || name == "__pycache__" {
                continue;
            }
            let path = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                walk(root, &path, depth + 1, out);
            } else if is_script(&name) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, "", 0, &mut out);
    out.sort();
    out
}

fn read_only(dir: &Path) -> Result<Connection> {
    // Through the normal path first, so a new workspace has its tables.
    drop(crate::db::open(dir)?);
    let conn = Connection::open_with_flags(
        dir.join("opentern.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}

fn lists(conn: &Connection, table: &str, active_key: &str) -> Result<Lists> {
    let (order, updated) = if table == "layouts" {
        ("updated DESC", "updated")
    } else {
        ("pos, id", "0")
    };
    let sql = format!("SELECT id, name, json, {updated} FROM {table} ORDER BY {order}");
    let mut q = conn.prepare(&sql)?;
    let rows = q
        .query_map([], |r| {
            Ok(Row {
                id: r.get(0)?,
                name: r.get(1)?,
                json: r.get(2)?,
                updated: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let active = value(conn, active_key)?
        .and_then(|v| v.parse::<i64>().ok())
        .and_then(|id| rows.iter().find(|r| r.id == id).map(|r| r.name.clone()));
    Ok(Lists { active, rows })
}

/// An item's JSON and how many things it holds.
fn gather(conn: &Connection, dir: &Path, item: Item) -> Result<(String, usize)> {
    Ok(match item {
        Item::Watchlists => {
            let l = lists(conn, "watchlists", "watchlist_id")?;
            let n = l.rows.len();
            (serde_json::to_string(&l)?, n)
        }
        Item::Layouts => {
            let l = lists(conn, "layouts", "layout_id")?;
            let n = l.rows.len();
            (serde_json::to_string(&l)?, n)
        }
        Item::Drawings => {
            let mut q = conn.prepare("SELECT symbol, json FROM drawings ORDER BY symbol")?;
            let rows = q
                .query_map([], |r| {
                    Ok(DrawingRow {
                        symbol: r.get(0)?,
                        json: r.get(1)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            // Each row is one symbol's list of drawings.
            let n = rows
                .iter()
                .map(|r| {
                    serde_json::from_str::<serde_json::Value>(&r.json)
                        .ok()
                        .and_then(|v| v.as_array().map(Vec::len))
                        .unwrap_or(1)
                })
                .sum();
            (serde_json::to_string(&rows)?, n)
        }
        Item::Templates => {
            let mut q = conn.prepare("SELECT kind, name, json, is_default FROM templates ORDER BY kind, name")?;
            let templates = q
                .query_map([], |r| {
                    Ok(TemplateRow {
                        kind: r.get(0)?,
                        name: r.get(1)?,
                        json: r.get(2)?,
                        is_default: r.get::<_, i64>(3)? != 0,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut q = conn.prepare("SELECT study, json FROM indicator_defaults ORDER BY study")?;
            let defaults = q
                .query_map([], |r| {
                    Ok(DefaultRow {
                        study: r.get(0)?,
                        json: r.get(1)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let n = templates.len() + defaults.len();
            (serde_json::to_string(&Templates { templates, defaults })?, n)
        }
        Item::Favorites => {
            let mut q = conn.prepare("SELECT kind, key, pos FROM favorites ORDER BY kind, pos, key")?;
            let rows = q
                .query_map([], |r| {
                    Ok(FavoriteRow {
                        kind: r.get(0)?,
                        key: r.get(1)?,
                        pos: r.get(2)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let n = rows.len();
            (serde_json::to_string(&rows)?, n)
        }
        Item::Alerts => {
            let mut q = conn.prepare("SELECT json FROM alerts ORDER BY id")?;
            let rows = q
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut settings = BTreeMap::new();
            for k in ALERT_SETTINGS {
                if let Some(v) = value(conn, k)?.filter(|v| !v.is_empty()) {
                    settings.insert(k.to_string(), v);
                }
            }
            let n = rows.len();
            (serde_json::to_string(&Alerts { rows, settings })?, n)
        }
        Item::Indices => {
            let v = value(conn, "custom_indices")?.unwrap_or_else(|| "[]".into());
            let n = serde_json::from_str::<Vec<serde_json::Value>>(&v)
                .map(|a| a.len())
                .unwrap_or(0);
            (v, n)
        }
        Item::Paper => {
            let saved = paper_saved(conn)?;
            let n = saved.accounts.len();
            (serde_json::to_string(&saved)?, n)
        }
        Item::Scripts => {
            let mut q = conn.prepare("SELECT key, value FROM settings WHERE key LIKE 'strategy_props:%'")?;
            let props = q
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .filter_map(|(k, v)| Some((k.strip_prefix(PROPS_PREFIX)?.to_string(), v)))
                .collect();
            let n = script_files(&scripts_dir(conn, dir)?).len();
            (serde_json::to_string(&ScriptsMeta { props })?, n)
        }
        Item::Screener | Item::Lab => {
            let key = item.key();
            let v = value(conn, key)?;
            let n = usize::from(v.is_some());
            (v.unwrap_or_else(|| "null".into()), n)
        }
    })
}

/// Paper accounts, including the single-account form older versions saved.
fn paper_saved(conn: &Connection) -> Result<PaperSaved> {
    if let Some(s) = value(conn, "paper_accounts")?.and_then(|v| serde_json::from_str::<PaperSaved>(&v).ok())
        && !s.accounts.is_empty()
    {
        return Ok(s);
    }
    let old = value(conn, "paper_account")?.and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok());
    Ok(PaperSaved {
        accounts: old.into_iter().collect(),
        active: 0,
    })
}

/// How many things each item holds right now, for the checkboxes.
pub fn counts(dir: &Path) -> Result<BTreeMap<Item, usize>> {
    let conn = read_only(dir)?;
    let mut out = BTreeMap::new();
    for item in Item::ALL {
        out.insert(item, gather(&conn, dir, item)?.1);
    }
    Ok(out)
}

// ------------------------------------------------------------------ exporting

/// Writes the picked items to `out` (via a temporary file, so a failed
/// export never leaves half a file). Returns its table of contents.
pub fn export(dir: &Path, items: &[Item], out: &Path) -> Result<Manifest> {
    let mut conn = read_only(dir)?;
    // One read transaction, so the file is one moment in time.
    let tx = conn.transaction()?;
    let part = out.with_extension("part");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let written = write_zip(&tx, dir, items, &part);
    let manifest = match written {
        Ok(m) => m,
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    };
    if let Err(e) = std::fs::rename(&part, out) {
        let _ = std::fs::remove_file(&part);
        return Err(e.into());
    }
    Ok(manifest)
}

fn write_zip(conn: &Connection, dir: &Path, items: &[Item], part: &Path) -> Result<Manifest> {
    let file = std::fs::File::create(part)?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut manifest = Manifest {
        format: FORMAT,
        app: env!("CARGO_PKG_VERSION").to_string(),
        created: jiff::Timestamp::now().as_millisecond(),
        items: BTreeMap::new(),
        python: 0,
    };
    let mut total = 0u64;
    let zip_err = |e: zip::result::ZipError| Error::Msg(format!("Couldn't write the file: {e}"));
    for &item in items {
        let (json, n) = gather(conn, dir, item)?;
        manifest.items.insert(item.key().to_string(), n);
        zip.start_file(format!("data/{}.json", item.key()), opts)
            .map_err(zip_err)?;
        zip.write_all(json.as_bytes())?;
        if item == Item::Scripts {
            let root = scripts_dir(conn, dir)?;
            for rel in script_files(&root) {
                let bytes = std::fs::read(root.join(&rel))
                    .map_err(|e| Error::Msg(format!("Couldn't read the script {rel}: {e}")))?;
                total += bytes.len() as u64;
                if total > MAX_UNPACKED {
                    return Err(Error::Msg("The scripts folder is too big to export.".into()));
                }
                if rel.ends_with(".py") {
                    manifest.python += 1;
                }
                zip.start_file(format!("scripts/{rel}"), opts).map_err(zip_err)?;
                zip.write_all(&bytes)?;
            }
        }
    }
    zip.start_file(MANIFEST, opts).map_err(zip_err)?;
    zip.write_all(serde_json::to_string(&manifest)?.as_bytes())?;
    let mut w = zip.finish().map_err(zip_err)?;
    w.flush()?;
    w.get_ref().sync_all()?;
    Ok(manifest)
}

// ------------------------------------------------------------------ importing

fn not_ours() -> Error {
    Error::Msg("That isn't an Opentern export file.".into())
}

/// The file's table of contents, or a plain error saying why it can't be
/// used.
pub fn read_manifest(path: &Path) -> Result<Manifest> {
    let file = std::fs::File::open(path).map_err(|e| Error::Msg(format!("Couldn't open that file: {e}")))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| not_ours())?;
    let mut entry = zip.by_name(MANIFEST).map_err(|_| not_ours())?;
    let mut text = String::new();
    entry
        .by_ref()
        .take(1 << 20)
        .read_to_string(&mut text)
        .map_err(|_| not_ours())?;
    let m: Manifest = serde_json::from_str(&text).map_err(|_| not_ours())?;
    if m.format > FORMAT {
        return Err(Error::Msg(
            "This file was made by a newer version of Opentern. Update Opentern to import it.".into(),
        ));
    }
    Ok(m)
}

/// A script's path inside a file, if it's safe to write: relative, staying
/// inside the scripts folder, and a script by its extension.
/// Whether a script path from saved data stays inside the scripts folder.
/// Layouts, alerts and screens name their scripts, and those can come from a
/// file: an absolute path or `..` would reach anything on the computer.
pub fn stays_inside(rel: &str) -> bool {
    !rel.is_empty()
        && Path::new(rel)
            .components()
            .all(|c| matches!(c, Component::Normal(s) if s.to_str().is_some_and(|s| !s.starts_with('.'))))
}

/// A script's fingerprint for [`PYTHON_HELD_KEY`].
pub fn script_hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// What a file would really do, read from its contents: its manifest says
/// whatever whoever made the file wrote there.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inspection {
    pub python: usize,
    /// Where its alerts would send things.
    pub webhook_hosts: Vec<String>,
}

pub fn inspect(path: &Path) -> Inspection {
    let mut out = Inspection::default();
    let Some(mut zip) = std::fs::File::open(path)
        .ok()
        .and_then(|f| zip::ZipArchive::new(f).ok())
    else {
        return out;
    };
    out.python = zip
        .file_names()
        .flatten()
        .filter_map(|n| n.strip_prefix("scripts/").map(str::to_string))
        .filter(|n| n.ends_with(".py") && safe_script_path(n).is_some())
        .count();
    if let Ok(e) = zip.by_name("data/alerts.json") {
        let mut text = String::new();
        let _ = e.take(MAX_UNPACKED).read_to_string(&mut text);
        let rows = serde_json::from_str::<Alerts>(&text)
            .map(|a| a.rows)
            .unwrap_or_default();
        for row in rows {
            let url = serde_json::from_str::<serde_json::Value>(&row)
                .ok()
                .and_then(|v| v.get("webhook")?.as_str().map(str::to_string))
                .unwrap_or_default();
            let host = url
                .split("://")
                .last()
                .unwrap_or("")
                .split(['/', '?', '#'])
                .next()
                .unwrap_or("");
            let host = host.rsplit('@').next().unwrap_or("").to_string();
            if !host.is_empty() && !out.webhook_hosts.contains(&host) {
                out.webhook_hosts.push(host);
            }
        }
    }
    out
}

/// Marks a file's Python scripts as not yet agreed to.
fn hold_python(tx: &Transaction, scripts: &[(String, Vec<u8>)]) -> Result<()> {
    let mut held: Vec<String> = value(tx, PYTHON_HELD_KEY)?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default();
    for (_, bytes) in scripts.iter().filter(|(rel, _)| rel.ends_with(".py")) {
        let hash = script_hash(bytes);
        if !held.contains(&hash) {
            held.push(hash);
        }
    }
    upsert_setting(tx, PYTHON_HELD_KEY, &serde_json::to_string(&held)?)
}

fn safe_script_path(rel: &str) -> Option<&str> {
    let drive = matches!(rel.as_bytes(), [a, b':', ..] if a.is_ascii_alphabetic());
    if rel.contains('\\') || rel.contains('\0') || drive || !is_script(rel) {
        return None;
    }
    stays_inside(rel).then_some(rel)
}

/// Whether Windows can save this path as it stands. Linux and macOS allow
/// names it doesn't, and `a:b` there would land in a hidden stream of `a`.
fn windows_can_save(rel: &str) -> bool {
    rel.split('/').all(|part| {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit());
        !reserved && !part.ends_with(['.', ' ']) && !part.chars().any(|c| c.is_control() || "<>:\"|?*".contains(c))
    })
}

/// Everything in a file, read and checked (the zip's checksums too) before
/// anything is changed.
struct Contents {
    manifest: Manifest,
    data: HashMap<Item, String>,
    scripts: Vec<(String, Vec<u8>)>,
}

fn read_contents(path: &Path) -> Result<Contents> {
    let manifest = read_manifest(path)?;
    let damaged = || Error::Msg("That file is damaged. Export it again.".into());
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| damaged())?;
    if zip.len() > MAX_ENTRIES {
        return Err(Error::Msg("That file is too big to import.".into()));
    }
    let (mut data, mut scripts) = (HashMap::new(), Vec::new());
    let mut left = MAX_UNPACKED;
    for i in 0..zip.len() {
        let mut e = zip.by_index(i).map_err(|_| damaged())?;
        if e.is_dir() {
            continue;
        }
        let name = e.name().map_err(|_| damaged())?.to_string();
        let mut bytes = Vec::new();
        // One past the limit, so going over is seen.
        let got = e
            .by_ref()
            .take(left + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| damaged())? as u64;
        if got > left {
            return Err(Error::Msg("That file is too big to import.".into()));
        }
        left -= got;
        if let Some(rel) = name.strip_prefix("scripts/") {
            // Anything that would land outside the folder is dropped.
            if let Some(rel) = safe_script_path(rel) {
                scripts.push((rel.to_string(), bytes));
            }
        } else if let Some(key) = name.strip_prefix("data/").and_then(|n| n.strip_suffix(".json"))
            && let Some(item) = Item::from_key(key)
        {
            data.insert(item, String::from_utf8(bytes).map_err(|_| damaged())?);
        }
    }
    Ok(Contents {
        manifest,
        data,
        scripts,
    })
}

/// Reads the whole file: checks it's sound before an import is started.
pub fn verify(path: &Path) -> Result<()> {
    read_contents(path).map(|_| ())
}

fn upsert_setting(tx: &Transaction, key: &str, value: &str) -> Result<()> {
    tx.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// Watchlists and layouts: rows matched by name. An incoming row keeps its
/// id when it's free, so settings that point at a list (the screener's
/// universe) still find it.
fn apply_lists(tx: &Transaction, table: &str, active_key: &str, lists: Lists, mode: Mode) -> Result<()> {
    if mode == Mode::Replace {
        tx.execute(&format!("DELETE FROM {table}"), [])?;
    }
    let mut existing: HashMap<String, Vec<i64>> = HashMap::new();
    let mut taken: HashSet<i64> = HashSet::new();
    {
        let mut q = tx.prepare(&format!("SELECT id, name FROM {table} ORDER BY id"))?;
        for row in q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
            let (id, name) = row?;
            existing.entry(name).or_default().push(id);
            taken.insert(id);
        }
    }
    let mut next_pos: i64 = if table == "watchlists" {
        tx.query_row("SELECT COALESCE(MAX(pos), -1) + 1 FROM watchlists", [], |r| r.get(0))?
    } else {
        0
    };
    let mut active_id = None;
    for row in lists.rows {
        // Two rows with one name each take their own match.
        let matched = existing
            .get_mut(&row.name)
            .and_then(|ids| if ids.is_empty() { None } else { Some(ids.remove(0)) });
        let id = match matched {
            Some(id) => {
                tx.execute(
                    &format!("UPDATE {table} SET json = ?1 WHERE id = ?2"),
                    params![row.json, id],
                )?;
                id
            }
            None => {
                let id = if taken.contains(&row.id) {
                    taken.iter().max().copied().unwrap_or(0) + 1
                } else {
                    row.id
                };
                taken.insert(id);
                if table == "watchlists" {
                    tx.execute(
                        "INSERT INTO watchlists (id, pos, name, json) VALUES (?1, ?2, ?3, ?4)",
                        params![id, next_pos, row.name, row.json],
                    )?;
                    next_pos += 1;
                } else {
                    tx.execute(
                        "INSERT INTO layouts (id, name, json, updated) VALUES (?1, ?2, ?3, ?4)",
                        params![id, row.name, row.json, row.updated],
                    )?;
                }
                id
            }
        };
        if table == "layouts" {
            tx.execute(
                "UPDATE layouts SET updated = ?1 WHERE id = ?2",
                params![row.updated, id],
            )?;
        }
        if lists.active.as_deref() == Some(row.name.as_str()) && active_id.is_none() {
            active_id = Some(id);
        }
    }
    if let Some(id) = active_id {
        upsert_setting(tx, active_key, &id.to_string())?;
    }
    Ok(())
}

/// What makes two alerts the same alert: everything you set, nothing that
/// changes as it runs.
fn alert_identity(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let f = |k: &str| v.get(k).cloned().unwrap_or(serde_json::Value::Null);
    serde_json::to_string(&(f("name"), f("symbol"), f("tf"), f("condition"), f("message"))).ok()
}

/// Merges JSON objects by one string field: same value → the incoming one
/// takes its place, new ones go on the end.
fn merge_by(mut base: Vec<serde_json::Value>, incoming: Vec<serde_json::Value>, field: &str) -> Vec<serde_json::Value> {
    for v in incoming {
        let key = v.get(field).cloned();
        match base.iter_mut().find(|b| key.is_some() && b.get(field).cloned() == key) {
            Some(slot) => *slot = v,
            None => base.push(v),
        }
    }
    base
}

fn apply_item(tx: &Transaction, item: Item, json: &str, mode: Mode) -> Result<()> {
    match item {
        Item::Watchlists => apply_lists(tx, "watchlists", "watchlist_id", serde_json::from_str(json)?, mode)?,
        Item::Layouts => apply_lists(tx, "layouts", "layout_id", serde_json::from_str(json)?, mode)?,
        Item::Drawings => {
            if mode == Mode::Replace {
                tx.execute("DELETE FROM drawings", [])?;
            }
            for r in serde_json::from_str::<Vec<DrawingRow>>(json)? {
                tx.execute(
                    "INSERT INTO drawings (symbol, json) VALUES (?1, ?2)
                     ON CONFLICT(symbol) DO UPDATE SET json = excluded.json",
                    params![r.symbol, r.json],
                )?;
            }
        }
        Item::Templates => {
            if mode == Mode::Replace {
                tx.execute("DELETE FROM templates", [])?;
                tx.execute("DELETE FROM indicator_defaults", [])?;
            }
            let t: Templates = serde_json::from_str(json)?;
            for r in t.templates {
                if r.is_default {
                    tx.execute("UPDATE templates SET is_default = 0 WHERE kind = ?1", params![r.kind])?;
                }
                tx.execute(
                    "INSERT INTO templates (kind, name, json, is_default) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(kind, name) DO UPDATE SET json = excluded.json, is_default = excluded.is_default",
                    params![r.kind, r.name, r.json, r.is_default as i64],
                )?;
            }
            for r in t.defaults {
                tx.execute(
                    "INSERT INTO indicator_defaults (study, json) VALUES (?1, ?2)
                     ON CONFLICT(study) DO UPDATE SET json = excluded.json",
                    params![r.study, r.json],
                )?;
            }
        }
        Item::Favorites => {
            if mode == Mode::Replace {
                tx.execute("DELETE FROM favorites", [])?;
            }
            // New ones go after what's here, in the file's order.
            for r in serde_json::from_str::<Vec<FavoriteRow>>(json)? {
                let base: i64 = tx.query_row(
                    "SELECT COALESCE(MAX(pos), -1) + 1 FROM favorites WHERE kind = ?1",
                    params![r.kind],
                    |q| q.get(0),
                )?;
                let pos = if mode == Mode::Replace { r.pos } else { base };
                tx.execute(
                    "INSERT OR IGNORE INTO favorites (kind, key, pos) VALUES (?1, ?2, ?3)",
                    params![r.kind, r.key, pos],
                )?;
            }
        }
        Item::Alerts => {
            let a: Alerts = serde_json::from_str(json)?;
            if mode == Mode::Replace {
                tx.execute("DELETE FROM alerts", [])?;
                // The log points at the old alerts' ids.
                tx.execute("DELETE FROM alert_log", [])?;
            }
            let mut here: HashMap<String, i64> = HashMap::new();
            {
                let mut q = tx.prepare("SELECT id, json FROM alerts")?;
                for row in q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
                    let (id, j) = row?;
                    if let Some(k) = alert_identity(&j) {
                        here.insert(k, id);
                    }
                }
            }
            for j in a.rows {
                match alert_identity(&j).and_then(|k| here.get(&k).copied()) {
                    Some(id) => tx.execute("UPDATE alerts SET json = ?1 WHERE id = ?2", params![j, id])?,
                    None => tx.execute("INSERT INTO alerts (json) VALUES (?1)", params![j])?,
                };
            }
            // Delivery details only fill in what isn't set up. The mail
            // password here is sent to whatever server these name, so a file
            // must never be able to change that server.
            for group in [&ALERT_SETTINGS[..1], &ALERT_SETTINGS[1..]] {
                let mut unset = true;
                for k in group {
                    unset &= value(tx, k)?.is_none_or(|v| v.is_empty());
                }
                if unset {
                    for k in group {
                        if let Some(v) = a.settings.get(*k) {
                            upsert_setting(tx, k, v)?;
                        }
                    }
                }
            }
        }
        Item::Indices => {
            let incoming: Vec<serde_json::Value> = serde_json::from_str(json)?;
            let merged = if mode == Mode::Replace {
                incoming
            } else {
                let base = value(tx, "custom_indices")?
                    .and_then(|v| serde_json::from_str(&v).ok())
                    .unwrap_or_default();
                merge_by(base, incoming, "ticker")
            };
            upsert_setting(tx, "custom_indices", &serde_json::to_string(&merged)?)?;
        }
        Item::Paper => {
            let incoming: PaperSaved = serde_json::from_str(json)?;
            let active_name = incoming
                .accounts
                .get(incoming.active)
                .and_then(|a| a.get("name"))
                .cloned();
            let accounts = if mode == Mode::Replace {
                incoming.accounts
            } else {
                merge_by(paper_saved(tx)?.accounts, incoming.accounts, "name")
            };
            let active = accounts
                .iter()
                .position(|a| active_name.is_some() && a.get("name").cloned() == active_name)
                .unwrap_or(0);
            let saved = PaperSaved { accounts, active };
            upsert_setting(tx, "paper_accounts", &serde_json::to_string(&saved)?)?;
            tx.execute("DELETE FROM settings WHERE key = 'paper_account'", [])?;
        }
        Item::Scripts => {
            let meta: ScriptsMeta = serde_json::from_str(json)?;
            if mode == Mode::Replace {
                tx.execute("DELETE FROM settings WHERE key LIKE 'strategy_props:%'", [])?;
            }
            for (file, props) in meta.props {
                upsert_setting(tx, &format!("{PROPS_PREFIX}{file}"), &props)?;
            }
        }
        Item::Screener | Item::Lab => {
            if json.trim() != "null" {
                upsert_setting(tx, item.key(), json)?;
            }
        }
    }
    Ok(())
}

/// Writes the file's scripts into the folder. Replace removes the script
/// files that are there first (and only script files; the backup has them).
/// Writes the scripts, going on past any this computer can't save (names
/// differ between systems). Returns the ones left out.
fn apply_scripts(root: &Path, scripts: Vec<(String, Vec<u8>)>, mode: Mode) -> Result<Vec<String>> {
    std::fs::create_dir_all(root)?;
    if mode == Mode::Replace {
        for rel in script_files(root) {
            let _ = std::fs::remove_file(root.join(rel));
        }
    }
    let mut left_out = Vec::new();
    for (rel, bytes) in scripts {
        let path = root.join(&rel);
        let tmp = path.with_extension("import-part");
        let saved = (!cfg!(windows) || windows_can_save(&rel))
            && path.parent().is_none_or(|p| std::fs::create_dir_all(p).is_ok())
            && std::fs::write(&tmp, &bytes).is_ok()
            && std::fs::rename(&tmp, &path).is_ok();
        if !saved {
            let _ = std::fs::remove_file(&tmp);
            left_out.push(rel);
        }
    }
    Ok(left_out)
}

/// Applies a file to the workspace at `dir`. The database changes all
/// together or not at all; nothing is touched if the file doesn't read.
/// Run with no app or background process using the workspace.
pub fn apply(dir: &Path, file: &Path, items: &[Item], mode: Mode) -> Result<()> {
    let contents = read_contents(file)?;
    let mut conn = crate::db::open(dir)?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    let root = scripts_dir(&conn, dir)?;
    let tx = conn.transaction()?;
    let wanted = |item: Item| items.contains(&item) && contents.manifest.items.contains_key(item.key());
    for item in Item::ALL {
        if wanted(item)
            && let Some(json) = contents.data.get(&item)
        {
            apply_item(&tx, item, json, mode).map_err(|e| {
                Error::Msg(format!(
                    "Couldn't read the {} in that file: {e}",
                    item.title().to_lowercase()
                ))
            })?;
        }
    }
    // In the same commit as everything else, and before any file lands.
    if wanted(Item::Scripts) {
        hold_python(&tx, &contents.scripts)?;
    }
    // Someone who imported their data has no need of the welcome.
    tx.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('onboarded', '1')",
        [],
    )?;
    tx.commit()?;
    if wanted(Item::Scripts) {
        let left_out = apply_scripts(&root, contents.scripts, mode)?;
        if !left_out.is_empty() {
            return Err(Error::Msg(format!(
                "Everything else came in, but this computer can't save these scripts: {}. Rename them on the other device and export again.",
                left_out.join(", ")
            )));
        }
    }
    Ok(())
}

// -------------------------------------------------------------------- staging

#[derive(Serialize, Deserialize)]
struct Staged {
    items: Vec<Item>,
    mode: Mode,
    backup: PathBuf,
}

/// What the last import did, reported once by the app on launch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    /// `None` when it worked; else why not, in plain words.
    pub error: Option<String>,
    pub backup: PathBuf,
}

fn import_dir(dir: &Path) -> PathBuf {
    dir.join("import")
}

fn staged_file(dir: &Path) -> PathBuf {
    import_dir(dir).join("pending.opentern")
}

fn staged_choice(dir: &Path) -> PathBuf {
    import_dir(dir).join("pending.json")
}

/// Where a file received from another device waits for its review.
pub fn incoming_file(dir: &Path) -> PathBuf {
    import_dir(dir).join("incoming.opentern")
}

/// Where the file being sent to another device is made.
pub fn outgoing_file(dir: &Path) -> PathBuf {
    import_dir(dir).join("outgoing.opentern")
}

/// Removes the working copies of a file being sent or received. An import
/// waiting for the next start is kept.
pub fn remove_working_files(dir: &Path) {
    for f in [incoming_file(dir), outgoing_file(dir)] {
        let _ = std::fs::remove_file(&f);
        let _ = std::fs::remove_file(f.with_extension("part"));
    }
}

/// Whether an import is waiting for the next start.
pub fn pending(dir: &Path) -> bool {
    staged_choice(dir).exists()
}

/// Saves everything here to a new file in `<workspace>/backups`, keeping
/// the latest few. Importing that file puts it all back.
pub fn backup(dir: &Path) -> Result<PathBuf> {
    let stamp = jiff::Zoned::now().strftime("%Y-%m-%d-%H%M%S%.3f").to_string();
    backup_named(dir, "before-import-", &stamp, KEEP_BACKUPS)
}

/// The day's automatic backup in `<workspace>/backups` (`daily-<date>`),
/// unless today's is there already; the latest week's are kept. Restored
/// like any export (Settings → Export and import → From a file).
pub fn daily_backup(dir: &Path) -> Result<Option<PathBuf>> {
    let day = jiff::Zoned::now().strftime("%Y-%m-%d").to_string();
    if dir.join("backups").join(format!("daily-{day}.{EXTENSION}")).exists() || !dir.join("opentern.db").exists() {
        return Ok(None);
    }
    backup_named(dir, "daily-", &day, KEEP_DAILY).map(Some)
}

/// How many daily backups are kept.
const KEEP_DAILY: usize = 7;

fn backup_named(dir: &Path, prefix: &str, stamp: &str, keep: usize) -> Result<PathBuf> {
    let backups = dir.join("backups");
    std::fs::create_dir_all(&backups)?;
    let path = backups.join(format!("{prefix}{stamp}.{EXTENSION}"));
    export(dir, &Item::ALL, &path)?;
    let mut old: Vec<PathBuf> = std::fs::read_dir(&backups)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == EXTENSION)
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(prefix))
        })
        .collect();
    old.sort();
    let extra = old.len().saturating_sub(keep);
    for p in old.into_iter().take(extra) {
        let _ = std::fs::remove_file(p);
    }
    Ok(path)
}

/// Gets an import ready for the next start: checks the whole file, backs
/// up what's here, then records the choice (the last step, so a failure
/// anywhere before it changes nothing).
pub fn stage(dir: &Path, file: &Path, items: &[Item], mode: Mode) -> Result<()> {
    verify(file)?;
    let backup = backup(dir)?;
    std::fs::create_dir_all(import_dir(dir))?;
    let target = staged_file(dir);
    if file != target {
        std::fs::copy(file, &target)?;
    }
    let choice = serde_json::to_string(&Staged {
        items: items.to_vec(),
        mode,
        backup,
    })?;
    let tmp = staged_choice(dir).with_extension("part");
    std::fs::write(&tmp, choice)?;
    std::fs::rename(&tmp, staged_choice(dir))?;
    Ok(())
}

/// Applies a staged import, if there is one, and records how it went for
/// the app to report. Called at startup before the workspace opens. The
/// choice is removed first: an import that crashes is never retried.
pub fn apply_pending(dir: &Path) {
    let path = staged_choice(dir);
    let Some(staged) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<Staged>(&s).ok())
    else {
        let _ = std::fs::remove_file(&path);
        return;
    };
    let _ = std::fs::remove_file(&path);
    let error = apply(dir, &staged_file(dir), &staged.items, staged.mode)
        .err()
        .map(|e| e.to_string());
    let outcome = Outcome {
        error,
        backup: staged.backup,
    };
    if let Ok(mut conn) = crate::db::open(dir)
        && let Ok(json) = serde_json::to_string(&outcome)
        && let Ok(tx) = conn.transaction()
    {
        let _ = upsert_setting(&tx, RESULT_KEY, &json);
        let _ = tx.commit();
    }
    // The working files have done their job.
    let _ = std::fs::remove_dir_all(import_dir(dir));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    /// A workspace with something of everything.
    fn seeded(dir: &Path) {
        let mut s = Store::open(dir).unwrap();
        s.save_watchlists(&[
            (7, "Crypto".into(), "[\"BTC\"]".into()),
            (9, "Stocks".into(), "[\"AAPL\"]".into()),
        ])
        .unwrap();
        s.set_value("watchlist_id", "9").unwrap();
        let main = s.create_layout("Main", "{\"v\":\"main\"}").unwrap();
        s.create_layout("Second", "{\"v\":\"second\"}").unwrap();
        s.set_value("layout_id", &main.to_string()).unwrap();
        s.save_drawings("BINANCE:BTCUSDT", Some("[1,2,3]")).unwrap();
        s.save_template("chart", "Night", "{\"n\":1}").unwrap();
        s.set_default_template("chart", Some("Night")).unwrap();
        s.set_indicator_default("rsi", Some("{\"length\":7}")).unwrap();
        s.set_favorites("interval", &["1m".into(), "1h".into()]).unwrap();
        s.save_alert(0, "{\"name\":\"Cross\",\"symbol\":\"BTC\",\"tf\":\"1h\",\"fired\":3}")
            .unwrap();
        s.set_value("telegram_chat_id", "42").unwrap();
        s.set_value("custom_indices", "[{\"ticker\":\"IDX\",\"name\":\"Mine\"}]")
            .unwrap();
        s.set_value(
            "paper_accounts",
            "{\"accounts\":[{\"name\":\"Main\",\"balance\":5},{\"name\":\"Risky\",\"balance\":9}],\"active\":1}",
        )
        .unwrap();
        s.set_value("screener", "{\"tf\":\"1D\"}").unwrap();
        s.set_value("lab", "{\"trials\":77}").unwrap();
        s.set_value("strategy_props:rsi.pine", "{\"pyramiding\":2}").unwrap();
        // Machine-specific, and not ours to move.
        s.set_value("snapshots_dir", "/somewhere/else").unwrap();
        s.set_value("alerts_at_login", "on").unwrap();
        let scripts = dir.join("scripts");
        std::fs::create_dir_all(scripts.join("sub")).unwrap();
        std::fs::write(scripts.join("rsi.pine"), "plot(close)").unwrap();
        std::fs::write(scripts.join("sub/py.py"), "print(1)").unwrap();
        std::fs::write(scripts.join("notes.txt"), "not a script").unwrap();
    }

    fn file_in(dir: &Path) -> PathBuf {
        dir.join(format!("export.{EXTENSION}"))
    }

    #[test]
    fn one_daily_backup_a_day_and_a_week_kept() {
        let d = tempfile::tempdir().unwrap();
        assert!(daily_backup(d.path()).unwrap().is_none(), "nothing to back up yet");
        seeded(d.path());
        let first = daily_backup(d.path()).unwrap().expect("today's backup");
        verify(&first).unwrap();
        assert!(daily_backup(d.path()).unwrap().is_none(), "only one a day");
        // Older days' files: only the latest week stays.
        let backups = d.path().join("backups");
        for day in 1..=9 {
            std::fs::copy(&first, backups.join(format!("daily-2020-01-{day:02}.{EXTENSION}"))).unwrap();
        }
        std::fs::remove_file(&first).unwrap();
        daily_backup(d.path()).unwrap().expect("today's again");
        let mut left: Vec<String> = std::fs::read_dir(&backups)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left.len(), KEEP_DAILY);
        assert_eq!(left[0], format!("daily-2020-01-04.{EXTENSION}"));
    }

    #[test]
    fn counts_say_what_is_there() {
        let d = tempfile::tempdir().unwrap();
        seeded(d.path());
        let c = counts(d.path()).unwrap();
        assert_eq!(c[&Item::Watchlists], 2);
        assert_eq!(c[&Item::Layouts], 2);
        assert_eq!(c[&Item::Drawings], 3);
        assert_eq!(c[&Item::Templates], 2);
        assert_eq!(c[&Item::Favorites], 2);
        assert_eq!(c[&Item::Alerts], 1);
        assert_eq!(c[&Item::Indices], 1);
        assert_eq!(c[&Item::Paper], 2);
        assert_eq!(c[&Item::Scripts], 2);
        assert_eq!(c[&Item::Screener], 1);
        assert_eq!(c[&Item::Lab], 1);
    }

    #[test]
    fn everything_arrives_on_an_empty_device() {
        let (from, to, tmp) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        seeded(from.path());
        let f = file_in(tmp.path());
        let m = export(from.path(), &Item::ALL, &f).unwrap();
        assert_eq!(m.python, 1);
        assert_eq!(read_manifest(&f).unwrap(), m);

        apply(to.path(), &f, &Item::ALL, Mode::Merge).unwrap();
        let s = Store::open(to.path()).unwrap();
        let names: Vec<_> = s.watchlists().iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["Crypto", "Stocks"]);
        assert_eq!(s.watchlists()[0].id, 7, "ids stay when they're free");
        assert_eq!(s.value("watchlist_id").as_deref(), Some("9"));
        assert_eq!(s.layouts().len(), 2);
        let active: i64 = s.value("layout_id").unwrap().parse().unwrap();
        assert_eq!(s.layout_json(active).unwrap().as_deref(), Some("{\"v\":\"main\"}"));
        assert_eq!(s.drawings("BINANCE:BTCUSDT"), Some("[1,2,3]"));
        assert_eq!(s.default_template("chart").unwrap().name, "Night");
        assert_eq!(s.indicator_default("rsi"), Some("{\"length\":7}"));
        assert_eq!(s.favorites("interval"), ["1m", "1h"]);
        assert_eq!(s.alerts().len(), 1);
        assert_eq!(s.value("telegram_chat_id").as_deref(), Some("42"));
        assert!(s.value("custom_indices").unwrap().contains("IDX"));
        let paper: serde_json::Value = serde_json::from_str(&s.value("paper_accounts").unwrap()).unwrap();
        assert_eq!(paper["active"], 1, "the account in use stays in use");
        assert_eq!(s.value("screener").as_deref(), Some("{\"tf\":\"1D\"}"));
        assert_eq!(s.value("lab").as_deref(), Some("{\"trials\":77}"));
        assert_eq!(
            s.value("strategy_props:rsi.pine").as_deref(),
            Some("{\"pyramiding\":2}")
        );
        assert_eq!(s.value("onboarded").as_deref(), Some("1"));
        let scripts = to.path().join("scripts");
        assert_eq!(
            std::fs::read_to_string(scripts.join("rsi.pine")).unwrap(),
            "plot(close)"
        );
        assert_eq!(std::fs::read_to_string(scripts.join("sub/py.py")).unwrap(), "print(1)");
        assert!(!scripts.join("notes.txt").exists(), "only scripts travel");
        // Machine-specific settings stay behind.
        assert_eq!(s.value("snapshots_dir"), None);
        assert_eq!(s.value("alerts_at_login"), None);
    }

    #[test]
    fn unticked_items_stay_out_of_the_file_and_out_of_an_import() {
        let (from, to, tmp) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        seeded(from.path());
        let f = file_in(tmp.path());
        let m = export(from.path(), &[Item::Watchlists, Item::Layouts, Item::Scripts], &f).unwrap();
        let present: Vec<_> = m.present().into_iter().map(|(i, _)| i).collect();
        assert_eq!(present, [Item::Watchlists, Item::Layouts, Item::Scripts]);
        // Asked for something the file doesn't have, and leaves out Scripts.
        apply(to.path(), &f, &[Item::Watchlists, Item::Paper], Mode::Merge).unwrap();
        let s = Store::open(to.path()).unwrap();
        assert_eq!(s.watchlists().len(), 2);
        assert!(s.layouts().is_empty());
        assert_eq!(s.value("paper_accounts"), None);
        assert!(!to.path().join("scripts/rsi.pine").exists());
    }

    /// The other device has things of its own, some with the same names.
    fn other_device(dir: &Path) {
        let mut s = Store::open(dir).unwrap();
        s.save_watchlists(&[
            (7, "Own".into(), "[\"OWN\"]".into()),
            (8, "Crypto".into(), "[\"OLD\"]".into()),
        ])
        .unwrap();
        s.create_layout("Main", "{\"v\":\"old\"}").unwrap();
        s.create_layout("Own layout", "{\"v\":\"own\"}").unwrap();
        s.save_drawings("NSE:INFY", Some("[9]")).unwrap();
        s.save_drawings("BINANCE:BTCUSDT", Some("[0]")).unwrap();
        s.set_favorites("interval", &["1h".into(), "5m".into()]).unwrap();
        s.save_alert(0, "{\"name\":\"Cross\",\"symbol\":\"BTC\",\"tf\":\"1h\",\"fired\":0}")
            .unwrap();
        s.save_alert(0, "{\"name\":\"Own\",\"symbol\":\"ETH\",\"tf\":\"1h\"}")
            .unwrap();
        s.log_alert(1, 5, "{}").unwrap();
        s.set_value(
            "custom_indices",
            "[{\"ticker\":\"IDX\",\"name\":\"Old\"},{\"ticker\":\"OWN\",\"name\":\"Own\"}]",
        )
        .unwrap();
        s.set_value("paper_account", "{\"name\":\"Solo\",\"balance\":1}")
            .unwrap();
        let scripts = dir.join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        std::fs::write(scripts.join("rsi.pine"), "old").unwrap();
        std::fs::write(scripts.join("own.pine"), "mine").unwrap();
        std::fs::write(scripts.join("notes.txt"), "keep me").unwrap();
    }

    #[test]
    fn merge_keeps_what_is_here_and_updates_same_names() {
        let (from, to, tmp) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        seeded(from.path());
        other_device(to.path());
        let f = file_in(tmp.path());
        export(from.path(), &Item::ALL, &f).unwrap();
        apply(to.path(), &f, &Item::ALL, Mode::Merge).unwrap();

        let s = Store::open(to.path()).unwrap();
        let lists: Vec<_> = s
            .watchlists()
            .iter()
            .map(|w| (w.name.as_str(), w.json.as_str()))
            .collect();
        assert_eq!(
            lists,
            [("Own", "[\"OWN\"]"), ("Crypto", "[\"BTC\"]"), ("Stocks", "[\"AAPL\"]")]
        );
        let ids: HashSet<_> = s.watchlists().iter().map(|w| w.id).collect();
        assert_eq!(ids.len(), 3, "no id clashes");
        assert_eq!(s.layouts().len(), 3);
        let main = s.layouts().iter().find(|l| l.name == "Main").unwrap().id;
        assert_eq!(s.layout_json(main).unwrap().as_deref(), Some("{\"v\":\"main\"}"));
        assert_eq!(s.drawings("NSE:INFY"), Some("[9]"));
        assert_eq!(s.drawings("BINANCE:BTCUSDT"), Some("[1,2,3]"));
        assert_eq!(s.favorites("interval"), ["1h", "5m", "1m"]);
        let alerts = s.alerts();
        assert_eq!(alerts.len(), 2, "the same alert isn't added twice");
        assert!(alerts[0].1.contains("\"fired\":3"));
        assert_eq!(s.alert_log(10).len(), 1, "merge keeps the log");
        let idx: Vec<serde_json::Value> = serde_json::from_str(&s.value("custom_indices").unwrap()).unwrap();
        assert_eq!(idx.len(), 2);
        assert_eq!(idx[0]["name"], "Mine");
        assert_eq!(idx[1]["ticker"], "OWN");
        let paper: serde_json::Value = serde_json::from_str(&s.value("paper_accounts").unwrap()).unwrap();
        let names: Vec<_> = paper["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["Solo", "Main", "Risky"], "the older single account is kept");
        assert_eq!(paper["active"], 2);
        let scripts = to.path().join("scripts");
        assert_eq!(
            std::fs::read_to_string(scripts.join("rsi.pine")).unwrap(),
            "plot(close)"
        );
        assert_eq!(std::fs::read_to_string(scripts.join("own.pine")).unwrap(), "mine");
        drop(s);

        // Importing the same file again changes nothing more.
        apply(to.path(), &f, &Item::ALL, Mode::Merge).unwrap();
        let s = Store::open(to.path()).unwrap();
        assert_eq!(s.watchlists().len(), 3);
        assert_eq!(s.layouts().len(), 3);
        assert_eq!(s.alerts().len(), 2);
        assert_eq!(s.favorites("interval").len(), 3);
    }

    #[test]
    fn replace_makes_picked_items_match_the_file_and_leaves_the_rest() {
        let (from, to, tmp) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        seeded(from.path());
        other_device(to.path());
        let f = file_in(tmp.path());
        export(from.path(), &Item::ALL, &f).unwrap();
        apply(
            to.path(),
            &f,
            &[Item::Watchlists, Item::Alerts, Item::Scripts, Item::Paper],
            Mode::Replace,
        )
        .unwrap();
        let s = Store::open(to.path()).unwrap();
        let names: Vec<_> = s.watchlists().iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["Crypto", "Stocks"]);
        assert_eq!(s.watchlists()[0].id, 7);
        assert_eq!(s.alerts().len(), 1);
        assert!(s.alert_log(10).is_empty());
        assert_eq!(s.value("paper_account"), None);
        let scripts = to.path().join("scripts");
        assert!(!scripts.join("own.pine").exists());
        assert_eq!(std::fs::read_to_string(scripts.join("notes.txt")).unwrap(), "keep me");
        // Not picked: untouched.
        assert_eq!(s.layouts().len(), 2);
        assert_eq!(s.drawings("NSE:INFY"), Some("[9]"));
    }

    #[test]
    fn a_moved_scripts_folder_is_read_and_written() {
        let (from, to, tmp) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let moved_from = tmp.path().join("my scripts");
        std::fs::create_dir_all(&moved_from).unwrap();
        std::fs::write(moved_from.join("a.pine"), "a").unwrap();
        Store::open(from.path())
            .unwrap()
            .set_value("scripts_dir", moved_from.to_str().unwrap())
            .unwrap();
        let moved_to = tmp.path().join("theirs");
        Store::open(to.path())
            .unwrap()
            .set_value("scripts_dir", moved_to.to_str().unwrap())
            .unwrap();
        let f = file_in(tmp.path());
        assert_eq!(export(from.path(), &[Item::Scripts], &f).unwrap().items["scripts"], 1);
        apply(to.path(), &f, &[Item::Scripts], Mode::Merge).unwrap();
        assert_eq!(std::fs::read_to_string(moved_to.join("a.pine")).unwrap(), "a");
        // Each device keeps its own folder setting.
        let s = Store::open(to.path()).unwrap();
        assert_eq!(s.value("scripts_dir").as_deref(), moved_to.to_str());
    }

    /// A zip by hand, for files Opentern would never write.
    fn zip_of(path: &Path, entries: &[(&str, &[u8])]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        for (name, bytes) in entries {
            z.start_file(*name, o).unwrap();
            z.write_all(bytes).unwrap();
        }
        z.finish().unwrap();
    }

    fn manifest_json(format: u32, items: &str) -> Vec<u8> {
        format!("{{\"format\":{format},\"app\":\"0\",\"created\":0,\"items\":{items}}}").into_bytes()
    }

    #[test]
    fn scripts_cant_escape_the_folder() {
        let (to, tmp) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let f = file_in(tmp.path());
        zip_of(
            &f,
            &[
                (MANIFEST, &manifest_json(1, "{\"scripts\":3}")),
                ("data/scripts.json", b"{\"props\":{}}"),
                ("scripts/ok.pine", b"good"),
                ("scripts/../../escape.pine", b"bad"),
                ("scripts//etc/abs.pine", b"bad"),
                ("scripts/C:/win.pine", b"bad"),
                ("scripts/a\\..\\..\\win2.pine", b"bad"),
                ("scripts/.hidden/x.pine", b"bad"),
                ("scripts/run.sh", b"bad"),
            ],
        );
        apply(to.path(), &f, &[Item::Scripts], Mode::Merge).unwrap();
        assert_eq!(script_files(&to.path().join("scripts")), ["ok.pine"]);
        assert!(!to.path().join("escape.pine").exists());
        assert!(!to.path().parent().unwrap().join("escape.pine").exists());
        assert!(safe_script_path("sub/fine.py").is_some());
        assert!(safe_script_path("../up.py").is_none());
        assert!(safe_script_path("/abs.py").is_none());
    }

    /// Damaged or doctored at random, a file gets refused or read: it never
    /// panics, and nothing lands outside the workspace.
    #[test]
    fn thousands_of_corrupted_files_do_no_harm() {
        let (from, tmp) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        seeded(from.path());
        let good = file_in(tmp.path());
        export(from.path(), &Item::ALL, &good).unwrap();
        let bytes = std::fs::read(&good).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let to = outside.path().join("workspace");
        let bad = tmp.path().join("bad.opentern");
        let mut x = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as usize
        };
        for round in 0..3000 {
            let mut b = bytes.clone();
            for _ in 0..1 + next() % 8 {
                let at = next() % b.len();
                b[at] = next() as u8;
            }
            if round % 5 == 0 {
                let keep = next() % b.len();
                b.truncate(keep);
            }
            std::fs::write(&bad, &b).unwrap();
            let _ = read_manifest(&bad);
            let _ = inspect(&bad);
            // The database is slow to reopen thousands of times.
            if round % 10 == 0 {
                std::fs::create_dir_all(&to).unwrap();
                let _ = apply(&to, &bad, &Item::ALL, Mode::Merge);
            }
        }
        let names: Vec<_> = std::fs::read_dir(outside.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, ["workspace"], "nothing beside the workspace");
    }

    #[test]
    fn a_tampered_file_cant_hide_python_or_start_it() {
        let (to, tmp) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let f = file_in(tmp.path());
        // The manifest claims no Python at all.
        zip_of(
            &f,
            &[
                (MANIFEST, &manifest_json(1, "{\"scripts\":1,\"alerts\":1}")),
                ("data/scripts.json", b"{\"props\":{}}"),
                (
                    "data/alerts.json",
                    br#"{"rows":["{\"webhook\":\"https://me:pw@evil.example/x?y=1\"}"],"settings":{}}"#,
                ),
                ("scripts/sub/evil.py", b"import os"),
                ("scripts/fine.pine", b"//@version=6"),
            ],
        );
        assert_eq!(read_manifest(&f).unwrap().python, 0);
        let seen = inspect(&f);
        assert_eq!(seen.python, 1);
        assert_eq!(seen.webhook_hosts, ["evil.example"]);

        apply(to.path(), &f, &[Item::Scripts], Mode::Merge).unwrap();
        let conn = crate::db::open(to.path()).unwrap();
        let held: Vec<String> = serde_json::from_str(&value(&conn, PYTHON_HELD_KEY).unwrap().unwrap()).unwrap();
        assert_eq!(held, [script_hash(b"import os")], "only Python, and by content");
    }

    #[test]
    fn saved_script_paths_cant_leave_the_folder() {
        for good in ["a.py", "sub/dir/a.py", "sub/./a.py"] {
            assert!(stays_inside(good), "{good}");
        }
        for bad in ["", "/etc/a.py", "../a.py", "sub/../../a.py", ".hidden/a.py", "./a.py"] {
            assert!(!stays_inside(bad), "{bad}");
        }
        #[cfg(windows)]
        for bad in [r"C:\a.py", r"\\server\share\a.py", r"sub\..\..\a.py", "C:a.py"] {
            assert!(!stays_inside(bad), "{bad}");
        }
    }

    #[test]
    fn a_file_cant_redirect_mail_or_telegram_that_is_set_up() {
        let (to, tmp) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let f = file_in(tmp.path());
        {
            let mut conn = crate::db::open(to.path()).unwrap();
            let tx = conn.transaction().unwrap();
            upsert_setting(&tx, "email_smtp_host", "smtp.mine.example").unwrap();
            tx.commit().unwrap();
        }
        let alerts = br#"{"rows":[],"settings":{"email_smtp_host":"smtp.evil.example","email_to":"evil@example.com","telegram_chat_id":"42"}}"#;
        zip_of(
            &f,
            &[
                (MANIFEST, &manifest_json(1, "{\"alerts\":0}")),
                ("data/alerts.json", alerts),
            ],
        );
        apply(to.path(), &f, &[Item::Alerts], Mode::Replace).unwrap();
        let conn = crate::db::open(to.path()).unwrap();
        assert_eq!(value(&conn, "email_smtp_host").unwrap().unwrap(), "smtp.mine.example");
        assert_eq!(
            value(&conn, "email_to").unwrap(),
            None,
            "the email settings go together"
        );
        assert_eq!(
            value(&conn, "telegram_chat_id").unwrap().unwrap(),
            "42",
            "nothing set up: it fills in"
        );
    }

    #[test]
    fn names_windows_cant_save_are_spotted() {
        for good in [
            "a.pine",
            "sub dir/rsi (2).py",
            "console.js",
            "com10.lua",
            "mine.nul.pine",
        ] {
            assert!(windows_can_save(good), "{good}");
        }
        for bad in [
            "a:b.pine",
            "what?.py",
            "x*.js",
            "con.pine",
            "NUL.py",
            "lpt1.lua",
            "sub./a.py",
            "s /a.py",
            "a|b.ts",
        ] {
            assert!(!windows_can_save(bad), "{bad}");
        }
    }

    #[test]
    fn a_script_that_cant_be_saved_is_named_and_the_rest_arrive() {
        let (to, tmp) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let f = file_in(tmp.path());
        // A file where its folder would go stands in for a name this
        // computer refuses.
        std::fs::create_dir_all(to.path().join("scripts")).unwrap();
        std::fs::write(to.path().join("scripts/blocked"), b"a file").unwrap();
        zip_of(
            &f,
            &[
                (MANIFEST, &manifest_json(1, "{\"scripts\":2,\"favorites\":0}")),
                ("data/scripts.json", b"{\"props\":{}}"),
                ("scripts/blocked/no.pine", b"lost"),
                ("scripts/ok.pine", b"good"),
            ],
        );
        let e = apply(to.path(), &f, &[Item::Scripts], Mode::Merge)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("blocked/no.pine") && e.contains("Everything else came in"),
            "{e}"
        );
        assert_eq!(std::fs::read(to.path().join("scripts/ok.pine")).unwrap(), b"good");
    }

    #[test]
    fn plain_errors_for_files_that_cant_be_used() {
        let tmp = tempfile::tempdir().unwrap();
        let junk = tmp.path().join("junk.opentern");
        std::fs::write(&junk, b"not a zip").unwrap();
        assert_eq!(
            read_manifest(&junk).unwrap_err().to_string(),
            "That isn't an Opentern export file."
        );
        let other = tmp.path().join("other.zip");
        zip_of(&other, &[("hello.txt", b"hi")]);
        assert_eq!(
            read_manifest(&other).unwrap_err().to_string(),
            "That isn't an Opentern export file."
        );
        let newer = tmp.path().join("newer.opentern");
        zip_of(&newer, &[(MANIFEST, &manifest_json(FORMAT + 1, "{}"))]);
        assert!(read_manifest(&newer).unwrap_err().to_string().contains("newer version"));
        // A newer app's unknown items are ignored.
        let extra = tmp.path().join("extra.opentern");
        zip_of(
            &extra,
            &[
                (MANIFEST, &manifest_json(1, "{\"hologram\":3,\"lab\":1}")),
                ("data/hologram.json", b"[]"),
                ("data/lab.json", b"{\"trials\":1}"),
            ],
        );
        assert_eq!(read_manifest(&extra).unwrap().present(), [(Item::Lab, 1)]);
    }

    #[test]
    fn bad_contents_change_nothing() {
        let (to, tmp) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        other_device(to.path());
        let f = file_in(tmp.path());
        // Good watchlists, then layouts that don't read.
        zip_of(
            &f,
            &[
                (MANIFEST, &manifest_json(1, "{\"watchlists\":1,\"layouts\":1}")),
                (
                    "data/watchlists.json",
                    b"{\"active\":null,\"rows\":[{\"id\":50,\"name\":\"New\",\"json\":\"[]\"}]}",
                ),
                ("data/layouts.json", b"{\"nonsense\":true}"),
            ],
        );
        let err = apply(to.path(), &f, &Item::ALL, Mode::Replace).unwrap_err();
        assert!(err.to_string().contains("layouts"), "{err}");
        let s = Store::open(to.path()).unwrap();
        assert_eq!(s.watchlists().len(), 2, "rolled back");
        assert_eq!(s.value("onboarded"), None);
    }

    #[test]
    fn a_failed_export_leaves_no_file() {
        let (from, tmp) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        seeded(from.path());
        // The output's folder is a file: it can't be created.
        let blocker = tmp.path().join("blocker");
        std::fs::write(&blocker, "x").unwrap();
        assert!(export(from.path(), &Item::ALL, &blocker.join("out.opentern")).is_err());
        let f = file_in(tmp.path());
        export(from.path(), &Item::ALL, &f).unwrap();
        assert!(!f.with_extension("part").exists());
    }

    #[test]
    fn staged_imports_apply_at_the_next_start_and_the_backup_undoes_them() {
        let (from, to, tmp) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        seeded(from.path());
        other_device(to.path());
        let f = file_in(tmp.path());
        export(from.path(), &Item::ALL, &f).unwrap();

        assert!(!pending(to.path()));
        stage(to.path(), &f, &Item::ALL, Mode::Replace).unwrap();
        assert!(pending(to.path()));
        // Nothing has changed yet.
        assert_eq!(Store::open(to.path()).unwrap().watchlists()[0].name, "Own");

        apply_pending(to.path());
        assert!(!pending(to.path()));
        assert!(!to.path().join("import").exists());
        let s = Store::open(to.path()).unwrap();
        assert_eq!(s.watchlists()[0].name, "Crypto");
        let outcome: Outcome = serde_json::from_str(&s.value(RESULT_KEY).unwrap()).unwrap();
        assert_eq!(outcome.error, None);
        assert!(outcome.backup.exists());
        assert!(!to.path().join("scripts/own.pine").exists());
        drop(s);

        // The backup puts everything back.
        apply(to.path(), &outcome.backup, &Item::ALL, Mode::Replace).unwrap();
        let s = Store::open(to.path()).unwrap();
        assert_eq!(s.watchlists()[0].name, "Own");
        assert_eq!(s.watchlists().len(), 2);
        assert_eq!(s.drawings("NSE:INFY"), Some("[9]"));
        assert_eq!(
            std::fs::read_to_string(to.path().join("scripts/own.pine")).unwrap(),
            "mine"
        );
    }

    #[test]
    fn a_file_that_fails_at_startup_is_reported_not_retried() {
        let (to, tmp) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        other_device(to.path());
        let f = file_in(tmp.path());
        zip_of(
            &f,
            &[
                (MANIFEST, &manifest_json(1, "{\"layouts\":1}")),
                ("data/layouts.json", b"not json"),
            ],
        );
        stage(to.path(), &f, &Item::ALL, Mode::Merge).unwrap();
        apply_pending(to.path());
        assert!(!pending(to.path()));
        let s = Store::open(to.path()).unwrap();
        let outcome: Outcome = serde_json::from_str(&s.value(RESULT_KEY).unwrap()).unwrap();
        assert!(outcome.error.unwrap().contains("layouts"));
        assert_eq!(s.layouts().len(), 2);
    }

    #[test]
    fn staging_refuses_a_damaged_file_before_touching_anything() {
        let (from, to, tmp) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        seeded(from.path());
        let f = file_in(tmp.path());
        export(from.path(), &Item::ALL, &f).unwrap();
        // Damage the middle of the file.
        let mut bytes = std::fs::read(&f).unwrap();
        let mid = bytes.len() / 2;
        for b in &mut bytes[mid - 40..mid] {
            *b ^= 0x5a;
        }
        std::fs::write(&f, &bytes).unwrap();
        assert!(stage(to.path(), &f, &Item::ALL, Mode::Replace).is_err());
        assert!(!pending(to.path()));
        assert!(!to.path().join("backups").exists());
    }

    #[test]
    fn only_the_latest_backups_are_kept() {
        let d = tempfile::tempdir().unwrap();
        seeded(d.path());
        for _ in 0..KEEP_BACKUPS + 2 {
            backup(d.path()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        let n = std::fs::read_dir(d.path().join("backups")).unwrap().count();
        assert_eq!(n, KEEP_BACKUPS);
    }
}

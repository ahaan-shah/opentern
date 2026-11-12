//! SQLite persistence (WAL). Migrations are append-only, keyed by
//! `PRAGMA user_version`: to change the schema, add a new SQL string to the
//! end of [`MIGRATIONS`]. Never edit an old one.

use crate::Result;
use rusqlite::Connection;
use std::path::Path;

pub(crate) const MIGRATIONS: &[&str] = &[
    // v1
    r#"
    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    "#,
    // v2: chart layouts, templates, per-indicator defaults, favourites.
    // Payloads are versioned JSON owned by the app, so old ones keep loading.
    r#"
    CREATE TABLE layouts (
        id      INTEGER PRIMARY KEY,
        name    TEXT NOT NULL,
        json    TEXT NOT NULL,
        updated INTEGER NOT NULL
    );
    CREATE TABLE templates (
        kind       TEXT NOT NULL,
        name       TEXT NOT NULL,
        json       TEXT NOT NULL,
        is_default INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (kind, name)
    );
    CREATE TABLE indicator_defaults (
        study TEXT PRIMARY KEY,
        json  TEXT NOT NULL
    );
    CREATE TABLE favorites (
        kind TEXT NOT NULL,
        key  TEXT NOT NULL,
        pos  INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (kind, key)
    );
    "#,
    // v3: drawings per symbol, editable watchlists.
    r#"
    CREATE TABLE drawings (
        symbol TEXT PRIMARY KEY,
        json   TEXT NOT NULL
    );
    CREATE TABLE watchlists (
        id   INTEGER PRIMARY KEY,
        pos  INTEGER NOT NULL,
        name TEXT NOT NULL,
        json TEXT NOT NULL
    );
    "#,
    // v4: alerts and the log of alerts that fired.
    r#"
    CREATE TABLE alerts (
        id   INTEGER PRIMARY KEY,
        json TEXT NOT NULL
    );
    CREATE TABLE alert_log (
        id       INTEGER PRIMARY KEY,
        alert_id INTEGER NOT NULL,
        time     INTEGER NOT NULL,
        json     TEXT NOT NULL
    );
    CREATE INDEX alert_log_time ON alert_log (time);
    "#,
];

pub fn open(dir: &Path) -> Result<Connection> {
    std::fs::create_dir_all(dir)?;
    let conn = Connection::open(dir.join("opentern.db"))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let version = version.max(0) as usize;
    if version > MIGRATIONS.len() {
        return Err(crate::Error::Msg(
            "This workspace was made by a newer version of Opentern. Update Opentern to open it.".into(),
        ));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_fresh_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let c = open(dir.path()).unwrap();
        let v: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v as usize, MIGRATIONS.len());
        drop(c);
        open(dir.path()).unwrap();
    }

    #[test]
    fn every_older_workspace_upgrades_and_keeps_its_settings() {
        // A workspace left at each earlier version (an old release that was
        // never updated) opens with the current one, settings intact.
        for k in 1..MIGRATIONS.len() {
            let dir = tempfile::tempdir().unwrap();
            {
                let c = Connection::open(dir.path().join("opentern.db")).unwrap();
                for (i, sql) in MIGRATIONS[..k].iter().enumerate() {
                    c.execute_batch(sql).unwrap();
                    c.pragma_update(None, "user_version", (i + 1) as i64).unwrap();
                }
                c.execute("INSERT INTO settings (key, value) VALUES ('kept', 'yes')", [])
                    .unwrap();
            }
            let c = open(dir.path()).unwrap_or_else(|e| panic!("from v{k}: {e}"));
            let v: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
            assert_eq!(v as usize, MIGRATIONS.len(), "from v{k}");
            let kept: String = c
                .query_row("SELECT value FROM settings WHERE key = 'kept'", [], |r| r.get(0))
                .unwrap();
            assert_eq!(kept, "yes", "from v{k}");
            drop(c);
            crate::Store::open(dir.path()).unwrap_or_else(|e| panic!("store from v{k}: {e}"));
        }
    }

    #[test]
    fn refuses_a_newer_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let c = open(dir.path()).unwrap();
        c.pragma_update(None, "user_version", (MIGRATIONS.len() + 1) as i64)
            .unwrap();
        drop(c);
        assert!(open(dir.path()).is_err());
    }
}

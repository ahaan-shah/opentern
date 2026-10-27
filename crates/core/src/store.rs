//! The workspace: settings now; layouts, watchlists, templates and drawings
//! as their phases land. Everything is loaded into memory at open and written
//! through to SQLite, so frames never touch the database.

use crate::Result;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub theme: String,
    pub font: String,
    /// Look for a new release on launch (never installs without a click).
    pub check_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: "Tern Dark".into(),
            font: "Inter".into(),
            check_updates: true,
        }
    }
}

/// A named chart layout (its content is loaded when it's opened).
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutInfo {
    pub id: i64,
    pub name: String,
    pub updated: i64,
}

/// A watchlist: `json` is the app's list of symbols.
#[derive(Clone, Debug, PartialEq)]
pub struct WatchlistRow {
    pub id: i64,
    pub name: String,
    pub json: String,
}

/// A saved template: chart settings, a set of indicators, or a drawing
/// style. `json` is owned by the app.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    pub name: String,
    pub json: String,
    pub is_default: bool,
}

pub struct Store {
    conn: Connection,
    dir: PathBuf,
    settings: Settings,
    version: u64,
    layouts: Vec<LayoutInfo>,
    templates: BTreeMap<String, Vec<Template>>,
    drawings: BTreeMap<String, String>,
    watchlists: Vec<WatchlistRow>,
    indicator_defaults: BTreeMap<String, String>,
    favorites: BTreeMap<String, Vec<String>>,
}

impl Store {
    pub fn open(dir: &Path) -> Result<Store> {
        let conn = crate::db::open(dir)?;
        let mut settings = Settings::default();
        {
            let mut q = conn.prepare("SELECT key, value FROM settings")?;
            let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (k, v) = row?;
                match k.as_str() {
                    "theme" => settings.theme = v,
                    "font" => settings.font = v,
                    "check_updates" => settings.check_updates = v != "0",
                    _ => {}
                }
            }
        }
        let layouts = {
            let mut q = conn.prepare("SELECT id, name, updated FROM layouts ORDER BY updated DESC")?;
            q.query_map([], |r| {
                Ok(LayoutInfo {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    updated: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let drawings = {
            let mut q = conn.prepare("SELECT symbol, json FROM drawings")?;
            q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<BTreeMap<String, String>>>()?
        };
        let watchlists = {
            let mut q = conn.prepare("SELECT id, name, json FROM watchlists ORDER BY pos, id")?;
            q.query_map([], |r| {
                Ok(WatchlistRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    json: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut templates: BTreeMap<String, Vec<Template>> = BTreeMap::new();
        {
            let mut q = conn.prepare("SELECT kind, name, json, is_default FROM templates ORDER BY name")?;
            let rows = q.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    Template {
                        name: r.get(1)?,
                        json: r.get(2)?,
                        is_default: r.get::<_, i64>(3)? != 0,
                    },
                ))
            })?;
            for row in rows {
                let (kind, t) = row?;
                templates.entry(kind).or_default().push(t);
            }
        }
        let indicator_defaults = {
            let mut q = conn.prepare("SELECT study, json FROM indicator_defaults")?;
            q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<BTreeMap<String, String>>>()?
        };
        let mut favorites: BTreeMap<String, Vec<String>> = BTreeMap::new();
        {
            let mut q = conn.prepare("SELECT kind, key FROM favorites ORDER BY pos, key")?;
            let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (kind, key) = row?;
                favorites.entry(kind).or_default().push(key);
            }
        }
        Ok(Store {
            conn,
            dir: dir.to_path_buf(),
            settings,
            version: 0,
            layouts,
            templates,
            drawings,
            watchlists,
            indicator_defaults,
            favorites,
        })
    }

    /// Named layouts, most recently saved first.
    pub fn layouts(&self) -> &[LayoutInfo] {
        &self.layouts
    }

    /// A layout's content.
    pub fn layout_json(&self, id: i64) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT json FROM layouts WHERE id = ?1", params![id], |r| r.get(0))
            .optional()?)
    }

    /// Saves a layout's content (creating it if `id` is new).
    pub fn save_layout(&mut self, id: i64, name: &str, json: &str) -> Result<()> {
        let now = jiff::Timestamp::now().as_millisecond();
        self.conn.execute(
            "INSERT INTO layouts (id, name, json, updated) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, json = excluded.json, updated = excluded.updated",
            params![id, name, json, now],
        )?;
        self.layouts.retain(|l| l.id != id);
        self.layouts.insert(
            0,
            LayoutInfo {
                id,
                name: name.to_string(),
                updated: now,
            },
        );
        Ok(())
    }

    /// A new layout; returns its id.
    pub fn create_layout(&mut self, name: &str, json: &str) -> Result<i64> {
        let id = self.layouts.iter().map(|l| l.id).max().unwrap_or(0) + 1;
        self.save_layout(id, name, json)?;
        self.version += 1;
        Ok(id)
    }

    pub fn delete_layout(&mut self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM layouts WHERE id = ?1", params![id])?;
        self.layouts.retain(|l| l.id != id);
        self.version += 1;
        Ok(())
    }

    /// Drawings saved for a symbol (JSON owned by the app).
    pub fn drawings(&self, symbol: &str) -> Option<&str> {
        self.drawings.get(symbol).map(|s| s.as_str())
    }

    pub fn save_drawings(&mut self, symbol: &str, json: Option<&str>) -> Result<()> {
        match json {
            Some(j) => {
                self.conn.execute(
                    "INSERT INTO drawings (symbol, json) VALUES (?1, ?2)
                     ON CONFLICT(symbol) DO UPDATE SET json = excluded.json",
                    params![symbol, j],
                )?;
                self.drawings.insert(symbol.to_string(), j.to_string());
            }
            None => {
                self.conn
                    .execute("DELETE FROM drawings WHERE symbol = ?1", params![symbol])?;
                self.drawings.remove(symbol);
            }
        }
        Ok(())
    }

    pub fn watchlists(&self) -> &[WatchlistRow] {
        &self.watchlists
    }

    /// Replaces all watchlists (they're small), keeping ids and order.
    pub fn save_watchlists(&mut self, lists: &[(i64, String, String)]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM watchlists", [])?;
        for (pos, (id, name, json)) in lists.iter().enumerate() {
            tx.execute(
                "INSERT INTO watchlists (id, pos, name, json) VALUES (?1, ?2, ?3, ?4)",
                params![id, pos as i64, name, json],
            )?;
        }
        tx.commit()?;
        self.watchlists = lists
            .iter()
            .map(|(id, name, json)| WatchlistRow {
                id: *id,
                name: name.clone(),
                json: json.clone(),
            })
            .collect();
        self.version += 1;
        Ok(())
    }

    /// Saved alerts as (id, JSON), oldest first.
    pub fn alerts(&self) -> Vec<(i64, String)> {
        let Ok(mut st) = self.conn.prepare("SELECT id, json FROM alerts ORDER BY id") else {
            return Vec::new();
        };
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    /// Saves an alert; id 0 makes a new one. Returns its id.
    pub fn save_alert(&mut self, id: i64, json: &str) -> Result<i64> {
        if id == 0 {
            self.conn
                .execute("INSERT INTO alerts (json) VALUES (?1)", params![json])?;
            Ok(self.conn.last_insert_rowid())
        } else {
            self.conn.execute(
                "INSERT INTO alerts (id, json) VALUES (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET json = excluded.json",
                params![id, json],
            )?;
            Ok(id)
        }
    }

    pub fn delete_alert(&mut self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM alerts WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Records a fired alert; the log keeps the latest 2,000.
    pub fn log_alert(&mut self, alert_id: i64, time: i64, json: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO alert_log (alert_id, time, json) VALUES (?1, ?2, ?3)",
            params![alert_id, time, json],
        )?;
        self.conn.execute(
            "DELETE FROM alert_log WHERE id NOT IN (SELECT id FROM alert_log ORDER BY id DESC LIMIT 2000)",
            [],
        )?;
        Ok(())
    }

    /// The alert log as (alert id, time, JSON), newest first.
    pub fn alert_log(&self, limit: usize) -> Vec<(i64, i64, String)> {
        let Ok(mut st) = self
            .conn
            .prepare("SELECT alert_id, time, json FROM alert_log ORDER BY id DESC LIMIT ?1")
        else {
            return Vec::new();
        };
        st.query_map(params![limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    pub fn clear_alert_log(&mut self) -> Result<()> {
        self.conn.execute("DELETE FROM alert_log", [])?;
        Ok(())
    }

    /// A free-form setting the app owns (current layout, active watchlist).
    pub fn value(&self, key: &str) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0))
            .ok()
    }

    pub fn set_value(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn templates(&self, kind: &str) -> &[Template] {
        self.templates.get(kind).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Saves (or overwrites) a named template.
    pub fn save_template(&mut self, kind: &str, name: &str, json: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO templates (kind, name, json) VALUES (?1, ?2, ?3)
             ON CONFLICT(kind, name) DO UPDATE SET json = excluded.json",
            params![kind, name, json],
        )?;
        let list = self.templates.entry(kind.to_string()).or_default();
        match list.iter_mut().find(|t| t.name == name) {
            Some(t) => t.json = json.to_string(),
            None => {
                list.push(Template {
                    name: name.to_string(),
                    json: json.to_string(),
                    is_default: false,
                });
                list.sort_by_key(|a| a.name.to_lowercase());
            }
        }
        self.version += 1;
        Ok(())
    }

    pub fn delete_template(&mut self, kind: &str, name: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM templates WHERE kind = ?1 AND name = ?2",
            params![kind, name],
        )?;
        if let Some(list) = self.templates.get_mut(kind) {
            list.retain(|t| t.name != name);
        }
        self.version += 1;
        Ok(())
    }

    /// Makes `name` the default template of its kind (`None` clears it).
    pub fn set_default_template(&mut self, kind: &str, name: Option<&str>) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("UPDATE templates SET is_default = 0 WHERE kind = ?1", params![kind])?;
        if let Some(n) = name {
            tx.execute(
                "UPDATE templates SET is_default = 1 WHERE kind = ?1 AND name = ?2",
                params![kind, n],
            )?;
        }
        tx.commit()?;
        if let Some(list) = self.templates.get_mut(kind) {
            for t in list {
                t.is_default = Some(t.name.as_str()) == name;
            }
        }
        self.version += 1;
        Ok(())
    }

    pub fn default_template(&self, kind: &str) -> Option<&Template> {
        self.templates(kind).iter().find(|t| t.is_default)
    }

    /// The saved defaults for one indicator ("Save as default").
    pub fn indicator_default(&self, study: &str) -> Option<&str> {
        self.indicator_defaults.get(study).map(|s| s.as_str())
    }

    pub fn set_indicator_default(&mut self, study: &str, json: Option<&str>) -> Result<()> {
        match json {
            Some(j) => {
                self.conn.execute(
                    "INSERT INTO indicator_defaults (study, json) VALUES (?1, ?2)
                     ON CONFLICT(study) DO UPDATE SET json = excluded.json",
                    params![study, j],
                )?;
                self.indicator_defaults.insert(study.to_string(), j.to_string());
            }
            None => {
                self.conn
                    .execute("DELETE FROM indicator_defaults WHERE study = ?1", params![study])?;
                self.indicator_defaults.remove(study);
            }
        }
        self.version += 1;
        Ok(())
    }

    /// Starred items of a kind ("interval", "indicator", "chart_type", "tool").
    pub fn favorites(&self, kind: &str) -> &[String] {
        self.favorites.get(kind).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn is_favorite(&self, kind: &str, key: &str) -> bool {
        self.favorites(kind).iter().any(|k| k == key)
    }

    /// Replaces the favourites of a kind, in order.
    pub fn set_favorites(&mut self, kind: &str, keys: &[String]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM favorites WHERE kind = ?1", params![kind])?;
        for (i, k) in keys.iter().enumerate() {
            tx.execute(
                "INSERT OR IGNORE INTO favorites (kind, key, pos) VALUES (?1, ?2, ?3)",
                params![kind, k, i as i64],
            )?;
        }
        tx.commit()?;
        self.favorites.insert(kind.to_string(), keys.to_vec());
        self.version += 1;
        Ok(())
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Bumped on every change, for memoising derived data in the UI.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn set_settings(&mut self, s: Settings) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (k, v) in [
            ("theme", s.theme.clone()),
            ("font", s.font.clone()),
            ("check_updates", if s.check_updates { "1" } else { "0" }.to_string()),
        ] {
            tx.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![k, v],
            )?;
        }
        tx.commit()?;
        self.settings = s;
        self.version += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_defaults_favorites_and_layout_persist() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        s.save_template("chart", "Night", "{}").unwrap();
        s.save_template("chart", "Day", "{\"a\":1}").unwrap();
        s.set_default_template("chart", Some("Day")).unwrap();
        s.set_indicator_default("rsi", Some("{\"length\":7}")).unwrap();
        s.set_favorites("interval", &["1m".into(), "1h".into()]).unwrap();
        let id = s.create_layout("Main", "{\"v\":1}").unwrap();
        s.save_drawings("BINANCE:BTCUSDT", Some("[1]")).unwrap();
        s.save_watchlists(&[(1, "Crypto".into(), "[]".into())]).unwrap();
        s.set_value("layout_id", &id.to_string()).unwrap();
        drop(s);
        let s = Store::open(dir.path()).unwrap();
        let names: Vec<_> = s.templates("chart").iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Day", "Night"]);
        assert_eq!(s.default_template("chart").unwrap().json, "{\"a\":1}");
        assert_eq!(s.indicator_default("rsi"), Some("{\"length\":7}"));
        assert_eq!(s.favorites("interval"), ["1m", "1h"]);
        assert_eq!(s.layouts()[0].name, "Main");
        assert_eq!(s.layout_json(s.layouts()[0].id).unwrap().as_deref(), Some("{\"v\":1}"));
        assert_eq!(s.drawings("BINANCE:BTCUSDT"), Some("[1]"));
        assert_eq!(s.watchlists()[0].name, "Crypto");
        assert_eq!(s.value("layout_id").as_deref(), Some("1"));
    }

    #[test]
    fn settings_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        assert_eq!(s.settings(), &Settings::default());
        let mut changed = s.settings().clone();
        changed.theme = "Opentern Light".into();
        changed.check_updates = false;
        s.set_settings(changed.clone()).unwrap();
        drop(s);
        assert_eq!(Store::open(dir.path()).unwrap().settings(), &changed);
    }
}

#[cfg(test)]
mod alert_tests {
    use super::*;

    #[test]
    fn alerts_and_their_log_persist() {
        let dir = std::env::temp_dir().join(format!("opentern-alerts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        {
            let mut s = Store::open(&dir).unwrap();
            let id = s.save_alert(0, "{\"a\":1}").unwrap();
            assert_eq!(s.save_alert(id, "{\"a\":2}").unwrap(), id);
            let other = s.save_alert(0, "{}").unwrap();
            s.delete_alert(other).unwrap();
            s.log_alert(id, 5, "{\"m\":\"x\"}").unwrap();
            s.log_alert(id, 6, "{\"m\":\"y\"}").unwrap();
        }
        let mut s = Store::open(&dir).unwrap();
        assert_eq!(s.alerts(), vec![(1, "{\"a\":2}".to_string())]);
        let log = s.alert_log(10);
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].1, 6);
        s.clear_alert_log().unwrap();
        assert!(s.alert_log(10).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

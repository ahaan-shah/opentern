//! The bar cache: `<workspace>/cache/bars.db`, separate from the workspace
//! database because it's disposable (deleting it only means downloading
//! again) and because each data thread opens its own connection to it.

use opentern_core::{AssetKind, Bar, Symbol, Timeframe};
use rusqlite::{Connection, params};
use std::path::Path;

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

pub struct Cache {
    conn: Connection,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS bars (
    sym TEXT NOT NULL,
    tf  TEXT NOT NULL,
    t   INTEGER NOT NULL,
    o   REAL NOT NULL,
    h   REAL NOT NULL,
    l   REAL NOT NULL,
    c   REAL NOT NULL,
    v   REAL NOT NULL,
    PRIMARY KEY (sym, tf, t)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS symbols (
    id          TEXT PRIMARY KEY,
    source      TEXT NOT NULL,
    description TEXT NOT NULL,
    kind        TEXT NOT NULL,
    currency    TEXT NOT NULL,
    tick        REAL NOT NULL,
    tz          TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

fn kind_str(k: AssetKind) -> &'static str {
    match k {
        AssetKind::Crypto => "crypto",
        AssetKind::Stock => "stock",
        AssetKind::Fund => "fund",
        AssetKind::Index => "index",
        AssetKind::Forex => "forex",
        AssetKind::Futures => "futures",
        AssetKind::Commodity => "commodity",
    }
}

fn kind_from(s: &str) -> AssetKind {
    match s {
        "crypto" => AssetKind::Crypto,
        "fund" => AssetKind::Fund,
        "index" => AssetKind::Index,
        "forex" => AssetKind::Forex,
        "futures" => AssetKind::Futures,
        "commodity" => AssetKind::Commodity,
        _ => AssetKind::Stock,
    }
}

impl Cache {
    pub fn open(dir: &Path) -> Result<Cache> {
        let _ = std::fs::create_dir_all(dir);
        let conn = Connection::open(dir.join("bars.db"))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        Ok(Cache { conn })
    }

    /// The newest `limit` bars, oldest first.
    pub fn last(&self, sym: &str, tf: Timeframe, limit: usize) -> Result<Vec<Bar>> {
        let mut q = self
            .conn
            .prepare_cached("SELECT t, o, h, l, c, v FROM bars WHERE sym = ?1 AND tf = ?2 ORDER BY t DESC LIMIT ?3")?;
        let mut bars: Vec<Bar> = q
            .query_map(params![sym, tf.label(), limit as i64], row_bar)?
            .collect::<Result<_>>()?;
        bars.reverse();
        Ok(bars)
    }

    /// Up to `limit` bars older than `before`, oldest first.
    pub fn before(&self, sym: &str, tf: Timeframe, before: i64, limit: usize) -> Result<Vec<Bar>> {
        let mut q = self.conn.prepare_cached(
            "SELECT t, o, h, l, c, v FROM bars WHERE sym = ?1 AND tf = ?2 AND t < ?3 ORDER BY t DESC LIMIT ?4",
        )?;
        let mut bars: Vec<Bar> = q
            .query_map(params![sym, tf.label(), before, limit as i64], row_bar)?
            .collect::<Result<_>>()?;
        bars.reverse();
        Ok(bars)
    }

    pub fn save(&self, sym: &str, tf: Timeframe, bars: &[Bar]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut q = tx.prepare_cached(
                "INSERT OR REPLACE INTO bars (sym, tf, t, o, h, l, c, v) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            let tf = tf.label();
            for b in bars {
                q.execute(params![sym, tf, b.time, b.open, b.high, b.low, b.close, b.volume])?;
            }
        }
        tx.commit()
    }

    /// Forgets the cached bars of one series (when they've fallen too far
    /// behind to patch).
    pub fn clear(&self, sym: &str, tf: Timeframe) -> Result<()> {
        self.conn
            .execute("DELETE FROM bars WHERE sym = ?1 AND tf = ?2", params![sym, tf.label()])?;
        Ok(())
    }

    pub fn save_symbols(&self, source: &str, symbols: &[Symbol]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM symbols WHERE source = ?1", params![source])?;
        {
            let mut q = tx.prepare_cached(
                "INSERT OR REPLACE INTO symbols (id, source, description, kind, currency, tick, tz)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for s in symbols {
                q.execute(params![
                    s.id,
                    source,
                    s.description,
                    kind_str(s.kind),
                    s.currency,
                    s.tick_size,
                    s.timezone
                ])?;
            }
        }
        tx.commit()
    }

    pub fn symbols(&self) -> Result<Vec<Symbol>> {
        let mut q = self
            .conn
            .prepare("SELECT id, description, kind, currency, tick, tz FROM symbols")?;
        q.query_map([], |r| {
            Ok(Symbol {
                id: r.get(0)?,
                description: r.get(1)?,
                kind: kind_from(&r.get::<_, String>(2)?),
                currency: r.get(3)?,
                tick_size: r.get(4)?,
                timezone: r.get(5)?,
            })
        })?
        .collect()
    }

    pub fn meta(&self, key: &str) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| r.get(0))
            .ok()
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }
}

fn row_bar(r: &rusqlite::Row) -> rusqlite::Result<Bar> {
    Ok(Bar {
        time: r.get(0)?,
        open: r.get(1)?,
        high: r.get(2)?,
        low: r.get(3)?,
        close: r.get(4)?,
        volume: r.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_loads_and_pages() {
        let dir = tempfile::tempdir().unwrap();
        let c = Cache::open(dir.path()).unwrap();
        let tf = Timeframe::parse("1m").unwrap();
        let bars: Vec<Bar> = (0..10)
            .map(|i| Bar {
                time: i * 60_000,
                open: 1.0,
                high: 2.0,
                low: 0.5,
                close: i as f64,
                volume: 1.0,
            })
            .collect();
        c.save("BINANCE:BTCUSDT", tf, &bars).unwrap();
        let last = c.last("BINANCE:BTCUSDT", tf, 3).unwrap();
        assert_eq!(last.iter().map(|b| b.close).collect::<Vec<_>>(), vec![7.0, 8.0, 9.0]);
        let older = c.before("BINANCE:BTCUSDT", tf, last[0].time, 2).unwrap();
        assert_eq!(older.iter().map(|b| b.close).collect::<Vec<_>>(), vec![5.0, 6.0]);
        // Saving again replaces (a forming bar that has since closed).
        let mut fixed = bars[9];
        fixed.close = 99.0;
        c.save("BINANCE:BTCUSDT", tf, &[fixed]).unwrap();
        assert_eq!(c.last("BINANCE:BTCUSDT", tf, 1).unwrap()[0].close, 99.0);
    }
}

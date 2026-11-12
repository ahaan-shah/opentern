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

/// Checks a folder's cache file once per run (the first open waits for
/// it); a damaged one is moved aside to `bars.db.broken` so it's
/// downloaded again rather than half-read. Returns whether it was.
fn check_once(dir: &Path) -> bool {
    use std::sync::{Mutex, OnceLock};
    static CHECKED: OnceLock<Mutex<std::collections::HashSet<std::path::PathBuf>>> = OnceLock::new();
    let mut seen = CHECKED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if !seen.insert(dir.to_path_buf()) {
        return false;
    }
    repair(dir)
}

/// Moves a damaged cache file aside. Returns whether it was damaged.
fn repair(dir: &Path) -> bool {
    let path = dir.join("bars.db");
    if !path.exists() || healthy(&path) {
        return false;
    }
    let _ = std::fs::rename(&path, dir.join("bars.db.broken"));
    for side in ["bars.db-wal", "bars.db-shm"] {
        let _ = std::fs::remove_file(dir.join(side));
    }
    true
}

/// SQLite's own quick check of every page and index.
fn healthy(path: &Path) -> bool {
    let Ok(conn) = Connection::open(path) else { return false };
    conn.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
        .is_ok_and(|s| s == "ok")
}

/// How much room the cache takes on disk, in bytes.
pub fn size(dir: &Path) -> u64 {
    ["bars.db", "bars.db-wal", "bars.db-shm"]
        .iter()
        .filter_map(|f| std::fs::metadata(dir.join(f)).ok())
        .map(|m| m.len())
        .sum()
}

/// Empties the bar cache and gives the room back (Settings → General).
/// Open charts keep what they show; prices download again when needed.
/// Symbol lists stay, so search keeps working offline.
pub fn clear(dir: &Path) -> Result<()> {
    let c = Cache::open(dir)?;
    c.conn.execute("DELETE FROM bars", [])?;
    // VACUUM writes the smaller file through the WAL; the checkpoint after
    // it moves that into the file and truncates the WAL.
    let _ = c.conn.execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);");
    Ok(())
}

impl Cache {
    pub fn open(dir: &Path) -> Result<Cache> {
        let _ = std::fs::create_dir_all(dir);
        check_once(dir);
        let conn = Connection::open(dir.join("bars.db"))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        // Added later: the symbol's minimum quantity (lot step).
        let has_lot: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('symbols') WHERE name = 'lot'")?
            .exists([])?;
        if !has_lot {
            conn.execute_batch("ALTER TABLE symbols ADD COLUMN lot REAL NOT NULL DEFAULT 0")?;
        }
        // Bars cached before splits were checked for (Nov 2026) may hold
        // unadjusted prices: download them once more.
        const BARS_VERSION: &str = "2";
        let c = Cache { conn };
        if c.meta("bars_version").as_deref() != Some(BARS_VERSION) {
            c.conn.execute("DELETE FROM bars", [])?;
            c.set_meta("bars_version", BARS_VERSION)?;
        }
        // Yahoo's "latest price" point used to be kept as a bar of its own
        // (Nov 2026): a flat, volume-less intraday bar at the session's
        // close or off the minute grid (`yahoo::fold_now_point`), and on
        // weekly and monthly charts a "week" or "month" starting on the day
        // it was fetched (weeks and months are built from days now, so
        // Yahoo's are fetched again whole). Remove those once.
        if c.meta("yahoo_now_points").is_none() {
            c.drop_yahoo_now_points()?;
            c.set_meta("yahoo_now_points", "1")?;
        }
        Ok(c)
    }

    fn drop_yahoo_now_points(&self) -> Result<()> {
        use opentern_core::calendar::{Market, closes_at};
        const NOT_YAHOO: &str = "sym NOT LIKE 'BINANCE:%' AND sym NOT LIKE 'BYBIT:%' AND sym NOT LIKE 'OKX:%'
            AND sym NOT LIKE 'COINBASE:%' AND sym NOT LIKE 'KRAKEN:%' AND sym NOT LIKE 'OANDA:%'
            AND sym NOT LIKE 'IBKR:%'";
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            &format!("DELETE FROM bars WHERE tf IN ('1W', '1M') AND {NOT_YAHOO}"),
            [],
        )?;
        // Stocks only (forex and others have no volume, so flat bars are
        // real there), and never extended-hours series (#ext), whose bars
        // run past the close.
        let stray: Vec<(String, String, i64)> = tx
            .prepare(
                "SELECT sym, tf, t FROM bars WHERE v = 0 AND o = h AND h = l AND l = c
                   AND substr(tf, -1) IN ('s', 'm', 'h') AND sym NOT LIKE '%#%'
                   AND (sym LIKE 'NASDAQ:%' OR sym LIKE 'NYSE:%' OR sym LIKE 'AMEX:%'
                     OR sym LIKE 'NSE:%' OR sym LIKE 'BSE:%')",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_>>()?;
        for (sym, tf, t) in stray {
            let market = if sym.starts_with("NSE:") || sym.starts_with("BSE:") {
                Market::India
            } else {
                Market::Us
            };
            if t % 60_000 != 0 || closes_at(market, t) {
                tx.execute(
                    "DELETE FROM bars WHERE sym = ?1 AND tf = ?2 AND t = ?3",
                    params![sym, tf, t],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
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
                "INSERT OR REPLACE INTO symbols (id, source, description, kind, currency, tick, tz, lot)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for s in symbols {
                q.execute(params![
                    s.id,
                    source,
                    s.description,
                    kind_str(s.kind),
                    s.currency,
                    s.tick_size,
                    s.timezone,
                    s.min_qty
                ])?;
            }
        }
        tx.commit()
    }

    pub fn symbols(&self) -> Result<Vec<Symbol>> {
        let mut q = self
            .conn
            .prepare("SELECT id, description, kind, currency, tick, tz, lot FROM symbols")?;
        q.query_map([], |r| {
            Ok(Symbol {
                id: r.get(0)?,
                description: r.get(1)?,
                kind: kind_from(&r.get::<_, String>(2)?),
                currency: r.get(3)?,
                tick_size: r.get(4)?,
                min_qty: r.get(6)?,
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
    fn clearing_empties_bars_keeps_symbols_and_shrinks_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let tf = Timeframe::parse("1m").unwrap();
        let bars: Vec<Bar> = (0..20_000)
            .map(|i| Bar {
                time: i * 60_000,
                open: 1.0,
                high: 2.0,
                low: 0.5,
                close: 1.5,
                volume: 3.0,
            })
            .collect();
        let c = Cache::open(dir.path()).unwrap();
        c.save("BINANCE:BTCUSDT", tf, &bars).unwrap();
        c.save_symbols("binance", &[Symbol::bare("BINANCE:BTCUSDT")]).unwrap();
        let before = size(dir.path());
        // Another connection stays open meanwhile, as a chart's would.
        clear(dir.path()).unwrap();
        assert!(c.last("BINANCE:BTCUSDT", tf, 10).unwrap().is_empty());
        assert!(size(dir.path()) < before / 4, "{} → {}", before, size(dir.path()));
        assert_eq!(c.symbols().unwrap().len(), 1);
    }

    #[test]
    fn a_damaged_cache_is_moved_aside_and_started_again() {
        let dir = tempfile::tempdir().unwrap();
        let tf = Timeframe::parse("1m").unwrap();
        let bars: Vec<Bar> = (0..3000)
            .map(|i| Bar {
                time: i * 60_000,
                open: 1.0,
                high: 2.0,
                low: 0.5,
                close: 1.5,
                volume: 3.0,
            })
            .collect();
        {
            let c = Cache::open(dir.path()).unwrap();
            c.save("BINANCE:BTCUSDT", tf, &bars).unwrap();
        }
        assert!(!repair(dir.path()), "a sound file stays");
        // Scribble over a page in the middle (a crash mid-write, a bad disk).
        let path = dir.path().join("bars.db");
        let mut bytes = std::fs::read(&path).unwrap();
        assert!(bytes.len() > 40_000);
        bytes[16_384..32_768].fill(0xA5);
        std::fs::write(&path, bytes).unwrap();
        assert!(repair(dir.path()));
        assert!(dir.path().join("bars.db.broken").exists());
        let c = Cache::open(dir.path()).unwrap();
        assert!(c.last("BINANCE:BTCUSDT", tf, 10).unwrap().is_empty());
        c.save("BINANCE:BTCUSDT", tf, &bars[..5]).unwrap();
        assert_eq!(c.last("BINANCE:BTCUSDT", tf, 10).unwrap().len(), 5);
    }

    #[test]
    fn yahoos_stray_bars_are_removed_once() {
        let dir = tempfile::tempdir().unwrap();
        let tf = |s: &str| Timeframe::parse(s).unwrap();
        let bar = |t: i64, flat: bool| Bar {
            time: t,
            open: 1.0,
            high: if flat { 1.0 } else { 2.0 },
            low: if flat { 1.0 } else { 0.5 },
            close: 1.0,
            volume: if flat { 0.0 } else { 5.0 },
        };
        let close = 1_794_430_800_000; // 11 Nov 2026 21:00 UTC, NYSE's close
        let quiet = close - 3_600_000; // 15:00 New York: in session
        {
            let c = Cache::open(dir.path()).unwrap();
            let m15 = tf("15m");
            c.save(
                "NASDAQ:NVDA",
                m15,
                &[bar(quiet, true), bar(close - 900_000, false), bar(close, true)],
            )
            .unwrap();
            c.save(
                "NASDAQ:NVDA",
                tf("1m"),
                &[bar(close - 60_000, false), bar(close + 26_000, true)],
            )
            .unwrap();
            c.save("NASDAQ:NVDA#ext", m15, &[bar(close, true)]).unwrap();
            c.save("FX:EURUSD", m15, &[bar(close, true)]).unwrap();
            c.save("BINANCE:BTCUSDT", m15, &[bar(close, true)]).unwrap();
            c.save("NASDAQ:NVDA", tf("1W"), &[bar(1_794_182_400_000, false)])
                .unwrap();
            c.save("BINANCE:BTCUSDT", tf("1W"), &[bar(1_794_182_400_000, false)])
                .unwrap();
            c.conn
                .execute("DELETE FROM meta WHERE key = 'yahoo_now_points'", [])
                .unwrap();
        }
        let c = Cache::open(dir.path()).unwrap();
        let times = |s: &str, t: &str| c.last(s, tf(t), 10).unwrap().iter().map(|b| b.time).collect::<Vec<_>>();
        assert_eq!(times("NASDAQ:NVDA", "15m"), vec![quiet, close - 900_000]);
        assert_eq!(times("NASDAQ:NVDA", "1m"), vec![close - 60_000]);
        assert_eq!(times("NASDAQ:NVDA#ext", "15m"), vec![close]);
        assert_eq!(times("FX:EURUSD", "15m"), vec![close]);
        assert_eq!(times("BINANCE:BTCUSDT", "15m"), vec![close]);
        assert!(times("NASDAQ:NVDA", "1W").is_empty());
        assert_eq!(times("BINANCE:BTCUSDT", "1W").len(), 1);
    }

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

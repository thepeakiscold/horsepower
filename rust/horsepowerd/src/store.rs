//! SQLite persistence for the per-minute energy log.
//!
//! The schema is deliberately the same as the Python implementation's, so an
//! existing database keeps its history when the service is upgraded:
//!
//! * `samples` — one row per minute, holding watt-seconds and seconds. Storing
//!   both means the average power can be reconstructed at full resolution.
//! * `daily` — one row per local day, never pruned.
//! * `meta` — schema version and the first time data was recorded.
//!
//! Local days are computed by SQLite (`'localtime'`), which keeps one source of
//! truth for day boundaries and inherits the system time zone and DST rules.

use std::path::Path;

use rusqlite::{params, Connection, OpenFlags};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS samples (
    bucket  INTEGER PRIMARY KEY,
    ws      REAL NOT NULL,
    seconds REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS daily (
    day TEXT PRIMARY KEY,
    wh  REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

PRAGMA user_version = 1;
";

/// One point of the minute-by-minute history.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryPoint {
    pub bucket: i64,
    pub watts: f64,
    pub wh: f64,
}

pub struct Store {
    connection: Connection,
    retention_days: i64,
    last_prune: i64,
    first_seen: i64,
}

impl Store {
    pub fn open(path: &Path, retention_days: i64) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_URI,
        )?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.execute_batch(SCHEMA)?;

        let now = now_seconds();
        connection.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES('first_seen', ?1)",
            params![now.to_string()],
        )?;
        let first_seen: i64 = connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'first_seen'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(now);

        restict_permissions(path);

        Ok(Self {
            connection,
            retention_days: retention_days.max(1),
            last_prune: 0,
            first_seen,
        })
    }

    pub fn first_seen(&self) -> i64 {
        self.first_seen
    }

    pub fn retention_days(&self) -> i64 {
        self.retention_days
    }

    /// The local calendar day for a Unix timestamp, as `YYYY-MM-DD`.
    pub fn local_day(&self, timestamp: i64) -> String {
        self.connection
            .query_row(
                "SELECT date(?1, 'unixepoch', 'localtime')",
                params![timestamp],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_else(|_| "1970-01-01".to_string())
    }

    /// `(lifetime_wh, today_wh)` from the daily aggregates.
    pub fn totals(&self, day: &str) -> (f64, f64) {
        let total: f64 = self
            .connection
            .query_row("SELECT COALESCE(SUM(wh), 0.0) FROM daily", [], |row| {
                row.get(0)
            })
            .unwrap_or(0.0);
        (total, self.today_total(day))
    }

    pub fn today_total(&self, day: &str) -> f64 {
        self.connection
            .query_row(
                "SELECT COALESCE((SELECT wh FROM daily WHERE day = ?1), 0.0)",
                params![day],
                |row| row.get(0),
            )
            .unwrap_or(0.0)
    }

    /// Add a completed (or partial) interval to the log atomically.
    ///
    /// Partial minutes accumulate in place, so restarting the service never
    /// loses the minute it was in the middle of.
    pub fn record(
        &mut self,
        bucket: i64,
        day: &str,
        wh: f64,
        watt_seconds: f64,
        seconds: f64,
    ) -> rusqlite::Result<()> {
        if wh < 0.0 || watt_seconds < 0.0 || seconds <= 0.0 {
            return Ok(());
        }
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO samples(bucket, ws, seconds) VALUES(?1, ?2, ?3)
             ON CONFLICT(bucket) DO UPDATE SET
                 ws = samples.ws + excluded.ws,
                 seconds = samples.seconds + excluded.seconds",
            params![bucket, watt_seconds, seconds],
        )?;
        transaction.execute(
            "INSERT INTO daily(day, wh) VALUES(?1, ?2)
             ON CONFLICT(day) DO UPDATE SET wh = daily.wh + excluded.wh",
            params![day, wh],
        )?;
        transaction.commit()
    }

    /// Per-minute averages for the last `minutes` minutes.
    pub fn history(&self, minutes: u32, now: i64) -> Vec<HistoryPoint> {
        let minutes = (minutes.max(1) as i64).min(7 * 24 * 60);
        let cutoff = (now / 60) * 60 - (minutes - 1) * 60;
        let Ok(mut statement) = self.connection.prepare(
            "SELECT bucket, ws, seconds FROM samples
             WHERE bucket >= ?1 AND bucket <= ?2 ORDER BY bucket",
        ) else {
            return Vec::new();
        };
        let rows = statement.query_map(params![cutoff, now], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, f64>(2)?,
            ))
        });
        let Ok(rows) = rows else {
            return Vec::new();
        };
        rows.filter_map(|row| row.ok())
            .filter_map(|(bucket, watt_seconds, seconds)| {
                if seconds <= 0.0 {
                    return None;
                }
                Some(HistoryPoint {
                    bucket,
                    watts: (watt_seconds / seconds).max(0.0),
                    wh: (watt_seconds / 3600.0).max(0.0),
                })
            })
            .collect()
    }

    /// Drop minute rows past the retention window. Daily rows are kept.
    pub fn prune(&mut self, now: i64) {
        if now - self.last_prune < 3600 {
            return;
        }
        self.last_prune = now;
        let cutoff = now - self.retention_days * 86_400;
        let _ = self
            .connection
            .execute("DELETE FROM samples WHERE bucket < ?1", params![cutoff]);
    }

    /// Flush the WAL back into the main database file.
    pub fn checkpoint(&self) {
        let _ = self
            .connection
            .pragma_update(None, "wal_checkpoint", "PASSIVE");
    }
}

fn restict_permissions(path: &Path) {
    // Telemetry is not secret, but it is nobody else's business. The systemd
    // unit also sets UMask, this covers manual runs.
    use std::os::unix::fs::PermissionsExt;
    for suffix in ["", "-wal", "-shm"] {
        let candidate = if suffix.is_empty() {
            path.to_path_buf()
        } else {
            path.with_file_name(format!(
                "{}{suffix}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ))
        };
        let _ = std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o640));
    }
}

/// Seconds since the Unix epoch, as an integer.
pub fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Store::open(&dir.path().join("power.db"), 2).expect("open");
        (dir, store)
    }

    #[test]
    fn empty_totals() {
        let (_dir, store) = store();
        assert_eq!(store.totals("2026-11-01"), (0.0, 0.0));
        assert!(store.first_seen() > 0);
    }

    #[test]
    fn partial_buckets_accumulate() {
        let (_dir, mut store) = store();
        store
            .record(600, "2026-11-01", 0.5, 1800.0, 60.0)
            .expect("record");
        store
            .record(600, "2026-11-01", 0.5, 1800.0, 60.0)
            .expect("record");

        let (total, today) = store.totals("2026-11-01");
        assert!((total - 1.0).abs() < 1e-12);
        assert!((today - 1.0).abs() < 1e-12);

        let points = store.history(60, 660);
        assert_eq!(points.len(), 1);
        assert!((points[0].watts - 30.0).abs() < 1e-12);
        assert!((points[0].wh - 1.0).abs() < 1e-12);
    }

    #[test]
    fn totals_span_days() {
        let (_dir, mut store) = store();
        store
            .record(600, "2026-11-01", 2.0, 7200.0, 60.0)
            .expect("record");
        store
            .record(1200, "2026-11-02", 3.0, 10800.0, 60.0)
            .expect("record");
        let (total, today) = store.totals("2026-11-02");
        assert!((total - 5.0).abs() < 1e-12);
        assert!((today - 3.0).abs() < 1e-12);
    }

    #[test]
    fn prune_keeps_daily_totals() {
        let (_dir, mut store) = store();
        let old_bucket = 1_000_000 - 10 * 86_400;
        store
            .record(old_bucket, "2026-10-20", 4.0, 14_400.0, 60.0)
            .expect("record");
        store
            .record(1_000_000, "2026-11-01", 1.0, 3600.0, 60.0)
            .expect("record");

        store.last_prune = 0;
        store.prune(1_000_000);

        let (total, _) = store.totals("2026-11-01");
        assert!((total - 5.0).abs() < 1e-12);
        assert_eq!(store.history(60, 1_000_000).len(), 1);
    }

    #[test]
    fn history_is_clamped_and_ordered() {
        let (_dir, mut store) = store();
        for index in 0..5 {
            store
                .record(600 + index * 60, "2026-11-01", 0.1, 360.0, 60.0)
                .expect("record");
        }
        let points = store.history(4, 900);
        assert_eq!(
            points.iter().map(|p| p.bucket).collect::<Vec<_>>(),
            vec![720, 780, 840]
        );
        assert!(points.iter().all(|p| (p.watts - 6.0).abs() < 1e-12));
    }

    #[test]
    fn invalid_intervals_are_ignored() {
        let (_dir, mut store) = store();
        store
            .record(600, "2026-11-01", 1.0, 3600.0, 0.0)
            .expect("record");
        store
            .record(600, "2026-11-01", -1.0, 3600.0, 60.0)
            .expect("record");
        assert_eq!(store.totals("2026-11-01"), (0.0, 0.0));
    }

    #[test]
    fn local_day_is_iso_formatted() {
        let (_dir, store) = store();
        let day = store.local_day(1_790_000_000);
        assert_eq!(day.len(), 10, "{day}");
        assert_eq!(day.as_bytes()[4], b'-');
        assert_eq!(day.as_bytes()[7], b'-');
    }

    #[test]
    fn reopening_keeps_history_and_first_seen() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("power.db");
        let first;
        {
            let mut store = Store::open(&path, 90).expect("open");
            store
                .record(600, "2026-11-01", 1.5, 5400.0, 60.0)
                .expect("record");
            first = store.first_seen();
        }
        let store = Store::open(&path, 90).expect("reopen");
        assert_eq!(store.first_seen(), first);
        assert!((store.totals("2026-11-01").0 - 1.5).abs() < 1e-12);
    }
}

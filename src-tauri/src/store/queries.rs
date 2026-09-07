//! Traffic-history persistence + pre-aggregated queries.
//!
//! Storage: a single `traffic_samples` table with (ts, upload, download).
//! Insert is a per-row write behind a Mutex; query is read-only and goes
//! through the same mutex (rusqlite `Connection` is not Sync).
//!
//! Aggregation: SQLite-side `SUM` grouped by integer bucket to avoid
//! shipping 500k raw rows to the frontend. Bucket sizes are chosen so
//! the chart always has 28-60 points regardless of range.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::error::{AppError, Result};
use crate::store::migrations;

/// One row of aggregated history data. `ts` is the bucket START (epoch ms).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HistoryPoint {
    pub ts: i64,
    pub upload: i64,
    pub download: i64,
}

/// Result of `query_history()`. The frontend renders `buckets` directly
/// onto the chart; `totals` are shown as headline numbers.
#[derive(Debug, Clone, Serialize)]
pub struct TrafficHistory {
    pub range: String,
    pub bucket_ms: i64,
    pub buckets: Vec<HistoryPoint>,
    pub total_upload: i64,
    pub total_download: i64,
}

/// Database handle, cheaply cloneable. Owns a Mutex<Connection> because
/// rusqlite's `Connection` is `!Sync`.
#[derive(Clone)]
pub struct HistoryDb {
    conn: std::sync::Arc<Mutex<Connection>>,
    path: std::path::PathBuf,
}

impl HistoryDb {
    /// Open the database at `path` and run migrations. Creates the parent
    /// directory if missing. Falls back to in-memory if `path` is empty
    /// (used by unit tests).
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent).map_err(AppError::from)?;
            }
        }
        let conn = Connection::open(path)?;
        migrations::migrate(&conn)?;
        Ok(Self {
            conn: std::sync::Arc::new(Mutex::new(conn)),
            path: path.to_path_buf(),
        })
    }

    /// In-memory database, used by tests. Always runs migrations.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        migrations::migrate(&conn)?;
        Ok(Self {
            conn: std::sync::Arc::new(Mutex::new(conn)),
            path: PathBuf::from(":memory:"),
        })
    }

    pub fn path(&self) -> &Path { &self.path }

    /// Append a single sample. Called by the sampler task every 5s.
    pub fn insert_sample(&self, ts_ms: i64, upload: i64, download: i64) -> Result<()> {
        let g = self.conn.lock().expect("history db mutex poisoned");
        g.execute(
            "INSERT INTO traffic_samples(ts, upload, download) VALUES (?1, ?2, ?3)",
            params![ts_ms, upload, download],
        )?;
        Ok(())
    }

    /// Drop rows older than the retention window. Returns deleted count.
    pub fn prune(&self) -> Result<usize> {
        let g = self.conn.lock().expect("history db mutex poisoned");
        migrations::prune(&g)
    }

    /// Count rows in the table (used by health checks / tests).
    pub fn count(&self) -> Result<i64> {
        let g = self.conn.lock().expect("history db mutex poisoned");
        let n: i64 = g.query_row("SELECT COUNT(*) FROM traffic_samples", [], |r| r.get(0))?;
        Ok(n)
    }

    /// Pre-aggregated history for the requested range. Returns up to
    /// `max_buckets` points, zero-filled for empty windows so the chart
    /// always has a stable X axis.
    ///
    /// `range` must be one of: "1h" (60 × 1m), "24h" (24 × 1h), "7d"
    /// (28 × 6h). Returns an error for unknown values.
    pub fn query_history(&self, range: &str) -> Result<TrafficHistory> {
        let (window_ms, bucket_ms, max_buckets) = match range {
            "1h"  => (60 * 60 * 1000i64,                60 * 1000i64,            60),
            "24h" => (24 * 60 * 60 * 1000i64,           60 * 60 * 1000i64,       24),
            "7d"  => (7 * 24 * 60 * 60 * 1000i64,       6 * 60 * 60 * 1000i64,   28),
            other => return Err(AppError::Other(format!("history range '{other}' is not supported"))),
        };
        let now = migrations::now_ms();
        // `start` is the conceptual lower bound; the actual SQL filter
        // (`query_start` below) widens it by one bucket so a row that
        // landed on the leading edge isn't dropped.
        let _start = now - window_ms;
        let g = self.conn.lock().expect("history db mutex poisoned");
        // GROUP BY integer bucket (ms / bucket_ms) so the SUM runs over
        // raw rows — SQLite's int math is exact, no float drift.
        let mut stmt = g.prepare(
            "SELECT (ts / ?1) * ?1 AS bucket,
                    COALESCE(SUM(upload), 0),
                    COALESCE(SUM(download), 0)
             FROM traffic_samples
             WHERE ts >= ?2
             GROUP BY bucket
             ORDER BY bucket ASC",
        )?;
        let mut map: std::collections::BTreeMap<i64, (i64, i64)> = std::collections::BTreeMap::new();
        // Anchor the array on the END of the window (the bucket
        // containing `now`) so the most-recent bucket is always at the
        // last index. The SQL `WHERE ts >= ?2` filter uses one bucket
        // before `first_bucket` to catch a row that landed on the
        // leading edge; we drop that sentinel key below.
        let end_bucket = (now / bucket_ms) * bucket_ms;
        let first_bucket = end_bucket - (max_buckets - 1) * bucket_ms;
        let query_start = first_bucket - bucket_ms;
        let mut rows = stmt.query(params![bucket_ms, query_start])?;
        while let Some(r) = rows.next()? {
            let b: i64 = r.get(0)?;
            let u: i64 = r.get(1)?;
            let d: i64 = r.get(2)?;
            map.insert(b, (u, d));
        }
        // Drop the leading catch-all bucket so it doesn't shift the array.
        map.remove(&query_start);
        // Build a fully-populated bucket array — zero-fill empty windows
        // so the chart can render a continuous line even when the kernel
        // was off for a stretch.
        let mut buckets = Vec::with_capacity(max_buckets as usize);
        let mut total_upload = 0i64;
        let mut total_download = 0i64;
        for i in 0..max_buckets {
            let ts = first_bucket + i * bucket_ms;
            let (u, d) = map.get(&ts).copied().unwrap_or((0, 0));
            total_upload += u;
            total_download += d;
            buckets.push(HistoryPoint { ts, upload: u, download: d });
        }
        Ok(TrafficHistory {
            range: range.to_string(),
            bucket_ms,
            buckets,
            total_upload,
            total_download,
        })
    }
}

use std::path::PathBuf;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrations::RETENTION_DAYS;

    #[test]
    fn open_in_memory_works() {
        let db = HistoryDb::open_in_memory().unwrap();
        assert_eq!(db.count().unwrap(), 0);
    }

    #[test]
    fn insert_and_count() {
        let db = HistoryDb::open_in_memory().unwrap();
        let now = migrations::now_ms();
        for i in 0..10 {
            db.insert_sample(now - i * 1000, 100, 200).unwrap();
        }
        assert_eq!(db.count().unwrap(), 10);
    }

    #[test]
    fn query_1h_returns_60_buckets_with_zero_fill() {
        let db = HistoryDb::open_in_memory().unwrap();
        let now = migrations::now_ms();
        // Three samples all clustered in the same minute bucket
        let bucket_start = (now / 60_000) * 60_000;
        db.insert_sample(bucket_start + 100, 10, 20).unwrap();
        db.insert_sample(bucket_start + 200, 30, 40).unwrap();
        db.insert_sample(bucket_start + 300, 5, 15).unwrap();
        let h = db.query_history("1h").unwrap();
        assert_eq!(h.range, "1h");
        assert_eq!(h.bucket_ms, 60_000);
        assert_eq!(h.buckets.len(), 60);
        assert_eq!(h.total_upload, 45);
        assert_eq!(h.total_download, 75);
        // The active bucket should hold the sum; all others zero.
        let active: Vec<&HistoryPoint> = h.buckets.iter().filter(|p| p.upload > 0).collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].ts, bucket_start);
    }

    #[test]
    fn query_24h_returns_24_buckets() {
        let db = HistoryDb::open_in_memory().unwrap();
        let h = db.query_history("24h").unwrap();
        assert_eq!(h.bucket_ms, 3_600_000);
        assert_eq!(h.buckets.len(), 24);
    }

    #[test]
    fn query_7d_returns_28_buckets() {
        let db = HistoryDb::open_in_memory().unwrap();
        let h = db.query_history("7d").unwrap();
        assert_eq!(h.bucket_ms, 6 * 3_600_000);
        assert_eq!(h.buckets.len(), 28);
    }

    #[test]
    fn query_rejects_unknown_range() {
        let db = HistoryDb::open_in_memory().unwrap();
        let err = db.query_history("2h").unwrap_err();
        assert!(err.to_string().contains("not supported"), "got: {err}");
    }

    #[test]
    fn prune_drops_aged_rows() {
        let db = HistoryDb::open_in_memory().unwrap();
        let now = migrations::now_ms();
        // Insert a row well outside the 30-day window.
        let ancient = now - (RETENTION_DAYS + 1) * 24 * 60 * 60 * 1000;
        db.insert_sample(ancient, 1, 1).unwrap();
        db.insert_sample(now - 1000, 1, 1).unwrap();
        assert_eq!(db.count().unwrap(), 2);
        let n = db.prune().unwrap();
        assert_eq!(n, 1);
        assert_eq!(db.count().unwrap(), 1);
    }

    #[test]
    fn aggregation_sums_both_directions() {
        let db = HistoryDb::open_in_memory().unwrap();
        let now = migrations::now_ms();
        // Two samples one second apart, same 1-minute bucket.
        let b = (now / 60_000) * 60_000;
        db.insert_sample(b + 1000, 1_000, 4_000).unwrap();
        db.insert_sample(b + 2000, 2_000, 3_000).unwrap();
        let h = db.query_history("1h").unwrap();
        let total_up: i64 = h.buckets.iter().map(|p| p.upload).sum();
        let total_dn: i64 = h.buckets.iter().map(|p| p.download).sum();
        assert_eq!(total_up, 3_000);
        assert_eq!(total_dn, 7_000);
    }
}

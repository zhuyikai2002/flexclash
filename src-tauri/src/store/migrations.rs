//! M10 schema migration.
//!
//! Single source of truth for the on-disk history table. Idempotent: the
//! `IF NOT EXISTS` + `user_version` PRAGMA combo means re-running
//! `migrate()` on an already-initialised database is a no-op.
//!
//! Retention: rows older than 30 days are pruned by `prune()` (called
//! hourly from the sampler task), not on every insert. We keep the
//! `idx_traffic_ts` index so the time-range queries stay O(log n) even
//! after a month of 5-second samples (~518k rows).

use rusqlite::Connection;

use crate::error::Result;

/// Bump this when adding new tables or columns. `migrate()` compares
/// against `user_version` to decide whether work is needed.
const SCHEMA_VERSION: i32 = 1;

/// Maximum age of a sample (days). Older rows are dropped on prune.
pub const RETENTION_DAYS: i64 = 30;

const CREATE_TRAFFIC: &str = r#"
CREATE TABLE IF NOT EXISTS traffic_samples (
    ts       INTEGER NOT NULL,
    upload   INTEGER NOT NULL,
    download INTEGER NOT NULL
)"#;

const CREATE_IDX: &str =
    "CREATE INDEX IF NOT EXISTS idx_traffic_ts ON traffic_samples(ts)";

/// Run the migration on a fresh or already-initialised connection.
pub fn migrate(conn: &Connection) -> Result<()> {
    let current: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current < SCHEMA_VERSION {
        let tx = conn.unchecked_transaction()?;
        tx.execute(CREATE_TRAFFIC, [])?;
        tx.execute(CREATE_IDX, [])?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.commit()?;
    }
    // Set performance pragmas once per open — these are per-connection
    // and cheap to reapply, so it's safe to call migrate() multiple times.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(())
}

/// Drop rows older than `RETENTION_DAYS`. Returns the number of rows
/// deleted so callers can surface it in logs / a future admin command.
pub fn prune(conn: &Connection) -> Result<usize> {
    let cutoff_ms: i64 = now_ms() - RETENTION_DAYS * 24 * 60 * 60 * 1000;
    let n = conn.execute(
        "DELETE FROM traffic_samples WHERE ts < ?1",
        [cutoff_ms],
    )?;
    Ok(n)
}

pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

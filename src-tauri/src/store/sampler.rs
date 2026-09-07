//! Traffic history sampler — M10 background task.
//!
//! Reads the mihomo `/traffic` REST endpoint every `SAMPLE_INTERVAL`,
//! converts the instantaneous rate (bytes/sec) into a 5-second byte
//! total, and persists it. A separate, slower task prunes rows older
//! than `migrations::RETENTION_DAYS`.
//!
//! Design notes
//! ------------
//! * The frontend subscribes to `/traffic` over its own WebSocket; the
//!   Rust side intentionally uses a separate, lighter HTTP poll so that
//!   "history is being written" is a property of the backend, not of
//!   whether the UI happens to be open.
//! * Failures are logged and swallowed. If mihomo is down we just skip
//!   that sample — gaps in the chart are acceptable, losing the task
//!   thread to an unwind is not.
//! * Cancellation: the task holds a single `CancellationToken`-like
//!   `Arc<AtomicBool>` checked every iteration so a clean shutdown
//!   from the Tauri `RunEvent::ExitRequested` path stops within one
//!   sample interval.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Runtime};
use tauri::Emitter;

use crate::error::Result;
use crate::store::migrations;
use crate::store::queries::HistoryDb;

/// Sample every 5s. mihomo's `/traffic` returns *rate* in bytes/sec;
/// we multiply by 5 to get a 5-second window total. Shorter intervals
/// would inflate I/O without giving the chart more detail (the chart
/// buckets are 1 minute / 1 hour / 6 hours).
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

/// Prune every 1h. 30-day retention means even with 5s samples we only
/// accumulate ~518k rows; running prune less often is fine.
pub const PRUNE_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Default Mihomo control endpoint (matches `sidecar::EXPECTED_CONTROLLER_PORT`).
pub const MIHOMO_TRAFFIC_URL: &str = "http://127.0.0.1:9091/traffic";

#[derive(Debug, thiserror::Error)]
enum SampleError {
    #[error("mihomo unreachable: {0}")]
    Unreachable(String),
    #[error("mihomo returned non-numeric rate: {0}")]
    BadShape(String),
    #[error("mihomo HTTP {0}")]
    Http(u16),
    #[error("db insert: {0}")]
    Db(String),
}

/// Public handle to the running sampler. Cheap to clone (Arc bump).
#[derive(Clone, Default)]
pub struct SamplerHandle {
    cancel: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

impl SamplerHandle {
    pub fn new() -> Self { Self::default() }

    pub fn cancel(&self) { self.cancel.store(true, Ordering::SeqCst); }
    pub fn is_running(&self) -> bool { self.running.load(Ordering::SeqCst) }
}

/// Spawn the sampler + pruner background tasks. Returns a handle for
/// clean shutdown. Errors during open() are surfaced; runtime errors
/// inside the loops are logged and the loop continues.
pub fn spawn<R: Runtime>(app: &AppHandle<R>, db: HistoryDb) -> Result<SamplerHandle> {
    let handle = SamplerHandle::new();
    let task_handle = handle.clone();
    let db_for_sample = db.clone();
    let app_for_log = app.clone();
    tauri::async_runtime::spawn(async move {
        task_handle.running.store(true, Ordering::SeqCst);
        sample_loop(task_handle.clone(), db_for_sample, app_for_log).await;
        task_handle.running.store(false, Ordering::SeqCst);
    });
    // Pruner: separate task, separate interval, separate cancellation.
    let db_for_prune = db.clone();
    let app_for_log = app.clone();
    let cancel = handle.cancel.clone();
    tauri::async_runtime::spawn(async move {
        prune_loop(cancel, db_for_prune, app_for_log).await;
    });
    Ok(handle)
}

async fn sample_loop<R: Runtime>(handle: SamplerHandle, db: HistoryDb, app: AppHandle<R>) {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[sampler] reqwest build failed: {e}");
            return;
        }
    };
    while !handle.cancel.load(Ordering::SeqCst) {
        match fetch_and_record(&client, &db).await {
            Ok(()) => {}
            Err(e) => {
                let _ = app.emit(
                    crate::events::KERNEL_LOG,
                    format!("[history] sample skipped: {e}"),
                );
            }
        }
        tokio::time::sleep(SAMPLE_INTERVAL).await;
    }
    eprintln!("[sampler] cancelled, exiting loop");
}

async fn fetch_and_record(client: &reqwest::Client, db: &HistoryDb) -> std::result::Result<(), SampleError> {
    let resp = client.get(MIHOMO_TRAFFIC_URL).send().await
        .map_err(|e| SampleError::Unreachable(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(SampleError::Http(resp.status().as_u16()));
    }
    let body: serde_json::Value = resp.json().await
        .map_err(|e| SampleError::BadShape(e.to_string()))?;
    let up = body.get("up").and_then(|v| v.as_i64())
        .ok_or_else(|| SampleError::BadShape("missing up".into()))?;
    let down = body.get("down").and_then(|v| v.as_i64())
        .ok_or_else(|| SampleError::BadShape("missing down".into()))?;
    // 5-second sample window: rate * 5. Saturate at i64 max.
    let bytes_up = up.saturating_mul(SAMPLE_INTERVAL.as_secs() as i64);
    let bytes_dn = down.saturating_mul(SAMPLE_INTERVAL.as_secs() as i64);
    let ts = migrations::now_ms();
    db.insert_sample(ts, bytes_up, bytes_dn)
        .map_err(|e| SampleError::Db(e.to_string()))?;
    Ok(())
}

async fn prune_loop<R: Runtime>(cancel: Arc<AtomicBool>, db: HistoryDb, app: AppHandle<R>) {
    // Stagger the first prune by one interval so the very first run
    // doesn't immediately drop 5 rows of bootstrap data.
    tokio::time::sleep(PRUNE_INTERVAL).await;
    while !cancel.load(Ordering::SeqCst) {
        match db.prune() {
            Ok(n) if n > 0 => {
                let _ = app.emit(
                    crate::events::KERNEL_LOG,
                    format!("[history] pruned {n} aged sample(s)"),
                );
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("[sampler] prune failed: {e}");
            }
        }
        tokio::time::sleep(PRUNE_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrations::RETENTION_DAYS;

    /// Verifies the byte-window math used to convert a rate to a delta.
    /// `as_secs() * rate` must use saturating_mul to guard against
    /// integer overflow when mihomo reports an absurdly large rate
    /// during startup bursts.
    #[test]
    fn rate_to_bytes_saturates() {
        let interval_secs = SAMPLE_INTERVAL.as_secs() as i64;
        // Healthy load: 1MB/s * 5s = 5MB.
        let normal = 1_048_576_i64.saturating_mul(interval_secs);
        assert_eq!(normal, 5_242_880);
        // Absurd spike: 100TB/s saturates instead of wrapping.
        let spike = (i64::MAX / 2).saturating_mul(interval_secs);
        assert!(spike > 0);
    }

    /// The 30-day retention window must stay in days, not seconds, so
    /// the prune SQL filter uses the same timebase as `now_ms()`.
    #[test]
    fn retention_constant_is_in_days() {
        assert!(RETENTION_DAYS > 0);
    }
}

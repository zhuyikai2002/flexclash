//! M10 traffic-history persistence + sampling.
//!
//! The frontend never opens the SQLite file directly — it goes through
//! the `get_traffic_history` Tauri command, which delegates to
//! `queries::HistoryDb` and returns pre-aggregated buckets.
//!
//! Lifecycle:
//!   `lib::run` → `store::open()` → `store::spawn_sampler()` →
//!   sampler/pruner tasks live for the rest of the process.
//!
//! Tests (open-in-memory DB) live alongside `queries.rs` and
//! `sampler.rs`.

pub mod migrations;
pub mod queries;
pub mod sampler;

use std::path::PathBuf;

use tauri::{AppHandle, Manager, Runtime};

use crate::error::Result;
use crate::store::queries::HistoryDb;
use crate::store::sampler::SamplerHandle;

/// File name of the history database. Resolved against
/// `app.path().app_local_data_dir()`.
const DB_FILENAME: &str = "history.db";

/// Resolve the absolute path of the history DB without opening it.
pub fn db_path_for<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf> {
    let base = app
        .path()
        .app_local_data_dir()
        .map_err(|e| crate::error::AppError::Path(format!("app_local_data_dir: {e}")))?;
    Ok(base.join(DB_FILENAME))
}

/// Open the on-disk history DB. Idempotent: re-opening a managed handle
/// just returns another cheap clone.
pub fn open<R: Runtime>(app: &AppHandle<R>) -> Result<HistoryDb> {
    let path = db_path_for(app)?;
    HistoryDb::open(&path)
}

/// Spawn the sampler and pruner background tasks. The returned handle
/// is the one the shutdown path uses to cancel both loops.
pub fn spawn_sampler<R: Runtime>(app: &AppHandle<R>, db: HistoryDb) -> Result<SamplerHandle> {
    sampler::spawn(app, db)
}

//! M10 history Tauri commands.
//!
//! The only surface the frontend uses to talk to the SQLite store.
//! `get_traffic_history` runs the pre-aggregated query and returns
//! ready-to-render `TrafficHistory` rows. `get_history_db_path` is
//! used by the settings page to show the on-disk file location.

use tauri::State;

use crate::error::Result;
use crate::store::queries::{HistoryDb, TrafficHistory};

/// Fetch a pre-aggregated traffic history for one of the three ranges.
/// The buckets are zero-filled so the frontend can render a continuous
/// line even when the kernel was off for part of the window.
#[tauri::command]
pub fn get_traffic_history(
    db: State<'_, HistoryDb>,
    range: String,
) -> Result<TrafficHistory> {
    db.query_history(&range)
}

/// Diagnostic: absolute path of the SQLite file. Used by the
/// "where is my data stored?" line in the settings card.
#[tauri::command]
pub fn get_history_db_path(db: State<'_, HistoryDb>) -> String {
    db.path().to_string_lossy().into_owned()
}

/// Count of stored samples. Used by the live-monitor chip on the
/// history view to show "currently buffering N rows".
#[tauri::command]
pub fn get_history_sample_count(db: State<'_, HistoryDb>) -> Result<i64> {
    db.count()
}

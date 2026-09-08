// ============================================================================
// commands/updater.rs — Update check / install facade (Phase UX: updater).
//
// Thin typed wrapper around `tauri-plugin-updater`. The frontend calls
// `check_update()` and `install_update()` instead of reaching into the
// plugin directly; download progress is broadcast as `updater://progress`
// so the settings card can render a live bar.
// ============================================================================

use serde::Serialize;
use specta::Type;
use specta::specta;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

pub const UPDATER_PROGRESS: &str = "updater://progress";

/// Lightweight update descriptor pushed to the UI.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// Version announced by the remote feed (e.g. "0.2.1").
    pub version: String,
    /// Version currently installed in this build.
    pub current_version: String,
    /// Release body / notes, if the feed carries them.
    pub body: Option<String>,
}

/// Check GitHub's update feed for a newer release. Returns `None` when the
/// current version is already the latest. Network failures are surfaced as
/// readable errors — never panics, never crashes the renderer.
#[tauri::command]
#[specta]
pub async fn check_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(u)) => Ok(Some(UpdateInfo {
            version: u.version,
            current_version: u.current_version,
            body: u.body,
        })),
        Ok(None) => Ok(None),
        Err(e) => Err(format!("update check failed: {e}")),
    }
}

/// Download the latest update and trigger install. Progress is emitted as
/// `updater://progress` with `{ downloaded, total }`; `total` may be null
/// until the server reports a content length.
#[tauri::command]
#[specta]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no update available".to_string())?;

    update
        .download_and_install(
            |chunk_len, content_len| {
                let _ = app.emit(
                    UPDATER_PROGRESS,
                    serde_json::json!({
                        "downloaded": chunk_len,
                        "total": content_len,
                    }),
                );
            },
            || {
                // All bytes received; the installer is about to run.
            },
        )
        .await
        .map_err(|e| format!("download/install failed: {e}"))
}

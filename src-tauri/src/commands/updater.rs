// ============================================================================
// commands/updater.rs — Update check / download / install facade.
//
// Thin typed wrapper around `tauri-plugin-updater`. The frontend calls
// `check_update()` / `download_update()` / `install_update()` instead of
// reaching into the plugin directly, which is what lets the UI own a real
// state machine (CHECKING -> AVAILABLE -> DOWNLOADING -> READY_TO_RESTART)
// rather than the plugin's all-in-one `download_and_install`.
//
// Download progress is broadcast as `updater://progress` with a *cumulative*
// `{ downloaded, total }`. The plugin's `download` callback hands us per-chunk
// lengths (`chunk.len()`), so the accumulation happens here — emitting the raw
// chunk length would show a wildly wrong percentage.
// ============================================================================

use std::sync::Mutex;

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{UpdaterExt, Update};

pub const UPDATER_PROGRESS: &str = "updater://progress";

/// Lightweight update descriptor pushed to the UI.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// Version announced by the remote feed (e.g. "0.5.0").
    pub version: String,
    /// Version currently installed in this build.
    pub current_version: String,
    /// Release body / notes, if the feed carries them.
    pub body: Option<String>,
}

/// Holds the verified installer between `download_update` and
/// `install_update`, so the UI can sit in `READY_TO_RESTART` until the user
/// explicitly confirms. `Update` is `Send + Sync` (its closure fields are all
/// `Arc<dyn Fn(...) + Send + Sync>`), so the tuple is safe inside managed
/// state. `install` needs the original `Update` (extract path, NSIS args,
/// on_before_exit), which is why the handle travels with the bytes.
#[derive(Default)]
pub struct PendingUpdate(pub Mutex<Option<(Update, Vec<u8>)>>);

/// Check GitHub's update feed for a newer release. Returns `None` when the
/// current version is already the latest. Network failures are surfaced as
/// readable errors — never panics, never crashes the renderer.
#[tauri::command]
#[specta::specta]
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

/// Download (and signature-verify) the latest update, but do NOT install it.
/// The verified bytes are parked in `PendingUpdate` for a later
/// `install_update` call. Progress is emitted as `updater://progress` with a
/// cumulative `{ downloaded, total }`; `total` may be null until the server
/// reports a content length.
#[tauri::command]
#[specta::specta]
pub async fn download_update(
    app: AppHandle,
    state: State<'_, PendingUpdate>,
) -> Result<UpdateInfo, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no update available".to_string())?;

    let info = UpdateInfo {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        body: update.body.clone(),
    };

    // The plugin's on_chunk gives `chunk.len()` — a per-chunk length, not a
    // running total. Accumulate so the emitted `downloaded` is monotonic.
    let mut downloaded: u64 = 0;
    let bytes = update
        .download(
            |chunk_len, content_len| {
                downloaded += chunk_len as u64;
                let _ = app.emit(
                    UPDATER_PROGRESS,
                    serde_json::json!({
                        "downloaded": downloaded,
                        "total": content_len,
                    }),
                );
            },
            || {
                // All bytes received and verified; awaiting the user's go.
            },
        )
        .await
        .map_err(|e| format!("download failed: {e}"))?;

    *state.0.lock().expect("pending-update mutex poisoned") = Some((update, bytes));
    Ok(info)
}

/// Install the update downloaded by `download_update`, then let the updater
/// relaunch the app. On Windows `Update::install` runs `ShellExecuteW` and
/// then `std::process::exit(0)`, which *skips* the window-close hook in
/// `core::shutdown` — so we perform the sidecar teardown ourselves, right
/// here, before the installer launches. The NSIS `NSIS_HOOK_PREINSTALL`
/// taskkill is the belt-and-suspenders net on top of this clean shutdown.
#[tauri::command]
#[specta::specta]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, PendingUpdate>,
) -> Result<(), String> {
    let (update, bytes) = state
        .0
        .lock()
        .expect("pending-update mutex poisoned")
        .take()
        .ok_or_else(|| "no downloaded update to install".to_string())?;

    // (1) Restore the system proxy before the sidecar dies (same ordering as
    //     core::shutdown's real-exit path), so no ERR_PROXY_CONNECTION_FAILED.
    let _ = crate::proxy::disable_system_proxy();
    // (2) Graceful sidecar stop, then the taskkill fallback.
    if let Some(handle) = app.try_state::<crate::core::sidecar::SidecarHandle>() {
        let _ = crate::core::sidecar::stop(handle.inner().clone());
    }
    crate::core::sidecar::hard_cleanup();

    update
        .install(bytes)
        .map_err(|e| format!("install failed: {e}"))
}

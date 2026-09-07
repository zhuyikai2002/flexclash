// ============================================================================
// commands/desktop.rs — M7: desktop integration (autostart + silent boot).
//
// Wraps `tauri-plugin-autostart` so the frontend talks to a tiny,
// stable command surface instead of plugin-internal identifiers.
// ============================================================================

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_autostart::ManagerExt;

use crate::core::startup::{self, SilentFlag};
use crate::error::{AppError, Result};

#[derive(Debug, Serialize)]
pub struct AutostartStatus {
    pub enabled: bool,
    pub silent: bool,
}

/// Read the actual `HKCU\...\Run\FlexClash` state via the autostart plugin.
/// Plugins store this under the app identifier on Windows.
#[tauri::command]
pub fn get_autostart_status<R: Runtime>(app: AppHandle<R>) -> AutostartStatus {
    let enabled = app
        .autolaunch()
        .is_enabled()
        .unwrap_or(false);
    let silent = app
        .try_state::<SilentFlag>()
        .map(|s| s.is_silent())
        .unwrap_or(false);
    AutostartStatus { enabled, silent }
}

/// Enable or disable the autostart entry. When enabling on Windows, the
/// plugin writes `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\FlexClash`
/// pointing at our binary with the `--silent` argument, so the next boot
/// starts in the tray.
#[tauri::command]
pub fn set_autostart<R: Runtime>(app: AppHandle<R>, enabled: bool) -> Result<AutostartStatus> {
    let launcher = app.autolaunch();
    if enabled {
        launcher
            .enable()
            .map_err(|e| AppError::Desktop(format!("autostart enable: {e}")))?;
    } else {
        launcher
            .disable()
            .map_err(|e| AppError::Desktop(format!("autostart disable: {e}")))?;
    }
    Ok(get_autostart_status(app))
}

/// Read the silent-launch flag (set once at boot from `argv`).
#[tauri::command]
pub fn get_silent_flag<R: Runtime>(app: AppHandle<R>) -> bool {
    app.try_state::<SilentFlag>()
        .map(|s| s.is_silent())
        .unwrap_or(false)
}

/// Run the residual-route sweep (M9 hook, no-op in M7). Exposed so the
/// frontend can show "checked for leftover routes" feedback even when the
/// real implementation is not yet wired in.
#[tauri::command]
pub fn sweep_residual_routes() -> startup::SweepResult {
    startup::sweep_residual_routes()
}

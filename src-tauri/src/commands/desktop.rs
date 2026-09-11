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
use crate::core::task_autostart;
use crate::error::{AppError, Result};

#[derive(Debug, Serialize)]
pub struct AutostartStatus {
    pub enabled: bool,
    pub silent: bool,
}

/// Status of the elevated (Task Scheduler) autostart.
#[derive(Debug, Serialize)]
pub struct SilentAutostartStatus {
    /// The logon task is registered.
    pub enabled: bool,
    /// False on non-Windows, or where the platform cannot support it. The
    /// UI hides/disables the switch rather than offering a control that
    /// can only fail.
    pub available: bool,
    /// True while the registry entry is the active mechanism, so the UI can
    /// explain which of the two mutually exclusive switches is in charge.
    pub registry_active: bool,
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

// ---------------------------------------------------------------------------
// Elevated autostart (Windows Task Scheduler)
// ---------------------------------------------------------------------------
//
// The two autostart mechanisms are mutually exclusive, and that is enforced
// here rather than left to the UI. Both would launch the app at logon --
// one with the filtered token, one with an administrator token -- and two
// instances fight over the reserved inbound port (7897). Whichever wins is
// non-deterministic, and the losing one shows the user "the kernel keeps
// crashing". Making the exclusion a backend invariant means a buggy or
// stale frontend cannot create that state.

#[tauri::command]
pub fn get_silent_autostart_status<R: Runtime>(app: AppHandle<R>) -> SilentAutostartStatus {
    SilentAutostartStatus {
        enabled: task_autostart::is_enabled(),
        available: task_autostart::is_available(),
        registry_active: app.autolaunch().is_enabled().unwrap_or(false),
    }
}

/// Enable the elevated logon task. **Shows a UAC prompt.**
///
/// Ordering is deliberate: the registry entry is removed *first*. If the
/// task creation is then cancelled at the UAC prompt we are left with no
/// autostart at all — which is recoverable by flicking the switch again —
/// whereas the reverse order could leave both mechanisms armed and produce
/// the double-launch the exclusion exists to prevent.
#[tauri::command]
pub fn set_silent_autostart<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<SilentAutostartStatus> {
    if !task_autostart::is_available() {
        return Err(AppError::Desktop(
            "silent autostart is Windows-only".into(),
        ));
    }

    if enabled {
        if app.autolaunch().is_enabled().unwrap_or(false) {
            app.autolaunch().disable().map_err(|e| {
                AppError::Desktop(format!("could not clear the registry autostart entry: {e}"))
            })?;
        }
        let exe = std::env::current_exe()
            .map_err(|e| AppError::Path(format!("current_exe: {e}")))?;
        task_autostart::enable(&exe)?;
    } else {
        task_autostart::disable()?;
    }

    Ok(get_silent_autostart_status(app))
}

/// Enable or disable the autostart entry. When enabling on Windows, the
/// plugin writes `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\FlexClash`
/// pointing at our binary with the `--silent` argument, so the next boot
/// starts in the tray.
///
/// Enabling this also removes the elevated logon task, for the reason
/// spelled out above.
#[tauri::command]
pub fn set_autostart<R: Runtime>(app: AppHandle<R>, enabled: bool) -> Result<AutostartStatus> {
    let launcher = app.autolaunch();
    if enabled {
        // Remove the scheduled task first so a failure here does not leave
        // both mechanisms armed.
        if task_autostart::is_enabled() {
            task_autostart::disable()?;
        }
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

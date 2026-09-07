// ============================================================================
// commands/tun.rs — Tauri command surface for the M9 TUN manager.
//
// Mirrors the shape of `commands::kernel` / `commands::desktop` so the
// frontend can reach every TUN operation through a single, stable
// invoke target. All actual work happens in `core::tun::TunManager`.
// ============================================================================

use tauri::{AppHandle, Manager, Runtime};

use crate::core::sidecar::SidecarHandle;
use crate::core::tun::{TunManager, TunStatus};
use crate::error::Result;
use crate::tray;

/// Cheap status read. Frontend polls this on mount + on every
/// `tun://state-changed` event so the toggle always reflects the
/// authoritative backend state.
#[tauri::command]
pub fn get_tun_state<R: Runtime>(app: AppHandle<R>) -> Result<TunStatus> {
    let mgr = app
        .try_state::<TunManager>()
        .ok_or_else(|| crate::error::AppError::Tun("TunManager not registered".into()))?;
    Ok(mgr.status())
}

/// Enable TUN. Triggers the full state machine:
///   * sweep → patch config → runas-elevate mihomo → wait for healthy
///   * on any failure, roll back to Off/Failed and surface the error.
#[tauri::command]
pub fn enable_tun<R: Runtime>(app: AppHandle<R>) -> Result<TunStatus> {
    let mgr = app
        .try_state::<TunManager>()
        .ok_or_else(|| crate::error::AppError::Tun("TunManager not registered".into()))?;
    let sidecar = app
        .try_state::<SidecarHandle>()
        .ok_or_else(|| crate::error::AppError::Tun("SidecarHandle not registered".into()))?;
    let work = sidecar.work_dir();
    let storage = crate::config::profile::ProfileStorage::new(&work);
    let res = mgr.enable(&app, &storage);
    // Refresh the tray icon regardless of success — a Failed transition
    // is still a state change worth reflecting.
    let _ = tray::update_tray_icon(&app);
    res
}

/// Disable TUN. Symmetric to `enable_tun`; rolls forward even on
/// partial failure (best-effort cleanup) and returns the final state.
#[tauri::command]
pub fn disable_tun<R: Runtime>(app: AppHandle<R>) -> Result<TunStatus> {
    let mgr = app
        .try_state::<TunManager>()
        .ok_or_else(|| crate::error::AppError::Tun("TunManager not registered".into()))?;
    let sidecar = app
        .try_state::<SidecarHandle>()
        .ok_or_else(|| crate::error::AppError::Tun("SidecarHandle not registered".into()))?;
    let work = sidecar.work_dir();
    let storage = crate::config::profile::ProfileStorage::new(&work);
    let res = mgr.disable(&app, &storage);
    let _ = tray::update_tray_icon(&app);
    res
}

/// Best-effort sweep exposed for the "fix it now" button on the UI.
/// Returns the M9 richer shape (`SweepResultFull`) so the UI can
/// render routes + adapters separately.
#[tauri::command]
pub fn sweep_tun_routes(app: AppHandle) -> Result<crate::core::startup::SweepResultFull> {
    let _ = app; // reserved for future "scope to current app dir" logic
    Ok(crate::core::route_guard::sweep_residual_routes())
}

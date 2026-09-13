// ============================================================================
// commands/tun.rs — Tauri command surface for the M9 TUN manager.
//
// Mirrors the shape of `commands::kernel` / `commands::desktop` so the
// frontend can reach every TUN operation through a single, stable
// invoke target. All actual work happens in `core::tun::TunManager`.
// ============================================================================

use tauri::{AppHandle, Manager, Runtime};

use crate::config::profile::TunAdvanced;
use crate::core::sidecar;
use crate::core::tun::{TunManager, TunStatus};
use crate::error::Result;
use crate::tray;

/// The active config location, resolved from the Tauri path API.
///
/// Deliberately does **not** go through `SidecarHandle::work_dir()`. That
/// method only knows the real answer once the kernel has been started in
/// this process; before that it has to guess, and a TUN enable that patches
/// the wrong `config.yaml` — or hands the elevated mihomo a wrong `-d` —
/// comes up with no profiles, no rules and no DNS: "TUN is on but nothing is
/// proxied". `work_dir_for` derives the path from `app.path()` instead, so
/// it is correct from the very first call.
fn storage_for<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<crate::config::profile::ProfileStorage> {
    let work = sidecar::work_dir_for(app)?;
    Ok(crate::config::profile::ProfileStorage::new(&work))
}

/// Resolve the two "TUN Advanced" switches, falling back to
/// [`TunAdvanced::default`] for a caller that does not send them (the
/// frontend always does — see `src/services/tun.ts`).
fn advanced_from(strict_route: Option<bool>, dns_hijack: Option<bool>) -> TunAdvanced {
    let d = TunAdvanced::default();
    TunAdvanced {
        strict_route: strict_route.unwrap_or(d.strict_route),
        dns_hijack: dns_hijack.unwrap_or(d.dns_hijack),
    }
}

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
///
/// `strict_route` / `dns_hijack` are the two Settings -> "TUN Advanced"
/// switches. They are stamped onto the `tun:` block during the config patch
/// (step 2), which is the only moment the yaml is ours to rewrite — the
/// elevated kernel then boots straight from it.
///
/// `rename_all = "snake_case"` is deliberate: Tauri 2 defaults command
/// arguments to camelCase, and pinning the convention here is what keeps the
/// Rust parameter names and the `invoke` payload in `src/services/tun.ts`
/// from silently drifting apart.
#[tauri::command(rename_all = "snake_case")]
pub fn enable_tun<R: Runtime>(
    app: AppHandle<R>,
    strict_route: Option<bool>,
    dns_hijack: Option<bool>,
) -> Result<TunStatus> {
    let mgr = app
        .try_state::<TunManager>()
        .ok_or_else(|| crate::error::AppError::Tun("TunManager not registered".into()))?;
    let storage = storage_for(&app)?;
    let res = mgr.enable(&app, &storage, advanced_from(strict_route, dns_hijack));
    // Refresh the tray icon regardless of success — a Failed transition
    // is still a state change worth reflecting.
    let _ = tray::update_tray_icon(&app);
    res
}

/// Push a changed "TUN Advanced" switch into the *running* kernel.
///
/// Without this the two cards would only take effect on the next
/// disable/enable cycle, so the UI would show "on" while `config.yaml` —
/// and therefore mihomo — still disagreed.
///
/// The yaml is re-stamped first and the hot-reload second, so a failed
/// reload (controller briefly unreachable, kernel restarting) still leaves a
/// correct file behind for the next start; the failure is logged rather than
/// surfaced, because the persisted switch is not lost. With TUN off this is
/// a no-op: there is no kernel holding the old yaml.
#[tauri::command(rename_all = "snake_case")]
pub async fn apply_tun_advanced<R: Runtime>(
    app: AppHandle<R>,
    strict_route: bool,
    dns_hijack: bool,
) -> Result<TunStatus> {
    let mgr = app
        .try_state::<TunManager>()
        .ok_or_else(|| crate::error::AppError::Tun("TunManager not registered".into()))?;
    let storage = storage_for(&app)?;
    let advanced = TunAdvanced { strict_route, dns_hijack };

    match mgr.repatch_for_advanced(&storage, advanced)? {
        // TUN is off (or mid-transition): nothing running to reload.
        None => Ok(mgr.status()),
        Some(outcome) => {
            if let Err(e) =
                crate::commands::profile::reload_via_controller(&outcome.config_path).await
            {
                eprintln!("[tun] advanced hot-reload skipped: {e}");
            }
            Ok(mgr.status())
        }
    }
}

/// Disable TUN. Symmetric to `enable_tun`; rolls forward even on
/// partial failure (best-effort cleanup) and returns the final state.
#[tauri::command]
pub fn disable_tun<R: Runtime>(app: AppHandle<R>) -> Result<TunStatus> {
    let mgr = app
        .try_state::<TunManager>()
        .ok_or_else(|| crate::error::AppError::Tun("TunManager not registered".into()))?;
    let storage = storage_for(&app)?;
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

// ============================================================================
// core/tun.rs — M9 TUN mode state machine.
//
// What this module owns
// ---------------------
// 1. A serialised state machine (Disabled -> Enabling -> Active ->
//    Disabling) so two concurrent `enable_tun()` calls cannot race.
// 2. The sequence of side-effects when flipping TUN on:
//      a. Persist the tun block to the active config (config::profile).
//      b. Sweep residual routes (defensive).
//      c. Request elevation via core::elevate (which ShellExecuteExW's
//         the mihomo binary as a separate process, runas).
//      d. Wait for the elevated mihomo to publish its REST API on
//         EXPECTED_CONTROLLER_PORT and report TUN state via /version
//         and the runtime tun-flag.
//      e. If any step fails, roll back: re-stop the elevated child,
//         strip the tun block, sweep again, and surface the error.
// 3. The sequence when flipping TUN off:
//      a. Tell the running mihomo to reload config (PUT /configs?force=true).
//         Mihomo drops TUN on its own when the new yaml has no `tun:`.
//      b. Wait for the Wintun adapter to disappear (best-effort poll).
//      c. Sweep residual routes.
//      d. On failure, leave the config with `tun: enable: false` so a
//         subsequent boot will not loop.
//
// Why a separate `core::elevate` module
// ------------------------------------
// Elevation via `ShellExecuteExW(verb="runas")` is platform-specific,
// needs a careful handle-release, and the UAC dialog is a *blocking*
// call that must not hold the tun mutex. The TUN module owns the
// state machine; elevation only owns "spawn an elevated child and
// report its PID/port".
// ============================================================================

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use crate::config::profile::{self, ProfileStorage, TunAdvanced, TunPatchOutcome};
use crate::core::route_guard;
use crate::error::{AppError, Result};
use crate::events::TUN_STATE_CHANGED;

/// Public TUN state, serialised both to the frontend (event payload) and
/// the `get_tun_state` Tauri command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TunState {
    /// User has TUN off (default).
    Off,
    /// Enable request is in flight (writing config, sweeping, requesting
    /// elevation). UI should show a spinner.
    Enabling,
    /// Elevated mihomo is up and reported TUN active.
    On,
    /// Disable request is in flight (reloading config, sweeping).
    Disabling,
    /// The most recent transition failed. The user-visible reason is
    /// stored alongside (e.g. "UAC cancelled", "elevated mihomo did
    /// not become healthy in 8s"). `last_error` is a separate field
    /// so the UI can keep showing the state badge while the toast
    /// fades.
    Failed,
}

impl TunState {
    pub fn as_str(self) -> &'static str {
        match self {
            TunState::Off => "off",
            TunState::Enabling => "enabling",
            TunState::On => "on",
            TunState::Disabling => "disabling",
            TunState::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TunStatus {
    pub state: TunState,
    pub enabled: bool,
    pub device: String,
    pub last_error: Option<String>,
    pub last_changed_at_ms: i64,
    /// Last sweep outcome (richer than the M7 contract; UI may use the
    /// `message` field to render a small status hint).
    pub last_sweep: Option<route_guard::SweepResult>,
}

impl Default for TunStatus {
    fn default() -> Self {
        TunStatus {
            state: TunState::Off,
            enabled: false,
            device: profile::TUN_DEVICE.to_string(),
            last_error: None,
            last_changed_at_ms: 0,
            last_sweep: None,
        }
    }
}

/// Shared, behind-a-mutex state. All transitions go through
/// `transition()`, which is the only method that takes the lock and
/// mutates. We never hold the lock across `.await` (no async in here).
pub struct TunManager {
    inner: Mutex<TunStatus>,
    /// Serialisation token: only one transition at a time across the
    /// whole process. Held in a `Mutex<()>` separately so the
    /// critical-section bookkeeping is obvious.
    gate: Mutex<()>,
}

impl TunManager {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(TunStatus::default()),
            gate: Mutex::new(()),
        }
    }

    pub fn snapshot(&self) -> TunStatus {
        self.inner.lock().expect("tun mutex poisoned").clone()
    }

    fn set_state(&self, new_state: TunState, last_error: Option<String>) {
        let mut g = self.inner.lock().expect("tun mutex poisoned");
        let now = chrono::Utc::now().timestamp_millis();
        g.state = new_state;
        g.enabled = matches!(new_state, TunState::On);
        g.last_error = last_error;
        g.last_changed_at_ms = now;
        let snap = g.clone();
        drop(g);
        // Emit on a best-effort basis; if the AppHandle is gone we
        // simply skip the broadcast.
        crate::core::tun::tun_events::broadcast_state(&snap);
    }

    fn record_sweep(&self, r: route_guard::SweepResult) {
        let mut g = self.inner.lock().expect("tun mutex poisoned");
        g.last_sweep = Some(r);
    }

    /// Frontend-facing status read. Cheap; only clones the struct.
    pub fn status(&self) -> TunStatus {
        self.snapshot()
    }

    /// Apply the `tun: enable: true` block to the active config. Pure
    /// file I/O; does not touch the mihomo process. Exposed so the
    /// `enable_tun` Tauri command can dry-run.
    pub fn patch_config(
        &self,
        storage: &ProfileStorage,
        enable: bool,
        advanced: TunAdvanced,
    ) -> Result<TunPatchOutcome> {
        profile::inject_tun_config(storage, enable, advanced)
    }

    /// Re-stamp the `tun:` block for the "TUN Advanced" switches while TUN
    /// is already up, so a flipped switch does not have to wait for the
    /// next enable/disable cycle.
    ///
    /// Returns `Ok(None)` when TUN is not in the `On` state: there is then
    /// no running kernel to reload, and the caller should simply leave the
    /// persisted switch to be applied by the next [`Self::enable`].
    ///
    /// Takes the transition gate so this cannot interleave with an
    /// in-flight enable/disable — the write is a single small file, so
    /// holding it briefly is cheaper than reasoning about the race.
    pub fn repatch_for_advanced(
        &self,
        storage: &ProfileStorage,
        advanced: TunAdvanced,
    ) -> Result<Option<TunPatchOutcome>> {
        let _gate = self.gate.lock().expect("tun gate poisoned");
        if !matches!(self.snapshot().state, TunState::On) {
            return Ok(None);
        }
        profile::inject_tun_config(storage, true, advanced).map(Some)
    }

    /// Drive a full enable transition. Must NOT be called re-entrantly
    /// — the gate mutex ensures that.
    pub fn enable<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        storage: &ProfileStorage,
        advanced: TunAdvanced,
    ) -> Result<TunStatus> {
        let _gate = self.gate.lock().expect("tun gate poisoned");

        if matches!(self.snapshot().state, TunState::Enabling | TunState::On) {
            return Err(AppError::Tun(format!(
                "tun already {}",
                self.snapshot().state.as_str()
            )));
        }

        self.set_state(TunState::Enabling, None);

        // 1. Sweep first so the elevated mihomo starts from a clean slate.
        let pre_sweep = route_guard::sweep_residual_routes();
        self.record_sweep(pre_sweep);

        // 2. Patch the active config.
        if let Err(e) = profile::inject_tun_config(storage, true, advanced) {
            self.set_state(TunState::Failed, Some(format!("config inject: {e}")));
            return Err(e);
        }

        // 3. Ask the elevate layer to launch mihomo with TUN. On UAC
        //    cancel it returns Err(ElevateCancelled); we map that to
        //    the "failed" state and roll back the config patch.
        match crate::core::elevate::spawn_elevated_mihomo::<R>(app) {
            Ok(_pid) => {
                // 4. Block (up to a budget) until the elevated mihomo
                //    is healthy. Health = /version returns 200 on the
                //    controller port. We do NOT poll /traffic here —
                //    the existing kernel manager owns that lifecycle.
                let wait = crate::core::elevate::wait_until_healthy(
                    std::time::Duration::from_secs(8),
                );
                match wait {
                    Ok(()) => {
                        self.set_state(TunState::On, None);
                        // The main app already has the controller URL
                        // pinned to 127.0.0.1:9091 so the elevated
                        // child's REST API is reachable directly.
                        let _ = app.emit(TUN_STATE_CHANGED, self.snapshot());
                        Ok(self.snapshot())
                    }
                    Err(e) => {
                        // Roll back: stop child, strip tun, sweep, fail.
                        let _ = crate::core::elevate::stop_elevated_mihomo();
                        let _ = profile::inject_tun_config(storage, false, advanced);
                        let post = route_guard::sweep_residual_routes();
                        self.record_sweep(post);
                        let msg = format!("elevated mihomo not healthy: {e}");
                        self.set_state(TunState::Failed, Some(msg.clone()));
                        Err(AppError::Tun(msg))
                    }
                }
            }
            Err(e) => {
                // UAC cancel or runas failure: roll back.
                let _ = profile::inject_tun_config(storage, false, advanced);
                let post = route_guard::sweep_residual_routes();
                self.record_sweep(post);
                let msg = format!("elevation failed: {e}");
                self.set_state(TunState::Failed, Some(msg.clone()));
                Err(AppError::Tun(msg))
            }
        }
    }

    /// Drive a full disable transition.
    pub fn disable<R: Runtime>(&self, app: &AppHandle<R>, storage: &ProfileStorage) -> Result<TunStatus> {
        let _gate = self.gate.lock().expect("tun gate poisoned");
        if matches!(self.snapshot().state, TunState::Off | TunState::Disabling) {
            return Ok(self.snapshot());
        }
        self.set_state(TunState::Disabling, None);

        // 1. Stop the elevated mihomo first; without it the kernel
        //    owned by `SidecarHandle` would briefly claim the port.
        if let Err(e) = crate::core::elevate::stop_elevated_mihomo() {
            // Non-fatal: still try to strip the config + sweep.
            eprintln!("[tun] elevated stop reported: {e}");
        }

        // 2. Strip the tun block. The active kernel (if any) will
        //    reload on next request; for the TUN disable path we
        //    don't auto-restart the kernel — the user can decide
        //    whether to keep the regular kernel running. `advanced`
        //    is irrelevant here: disabling removes the whole block.
        if let Err(e) = profile::inject_tun_config(storage, false, TunAdvanced::default()) {
            self.set_state(TunState::Failed, Some(format!("config strip: {e}")));
            return Err(e);
        }

        // 3. Sweep residual routes / adapter.
        let post = route_guard::sweep_residual_routes();
        self.record_sweep(post);

        self.set_state(TunState::Off, None);
        let _ = app.emit(TUN_STATE_CHANGED, self.snapshot());
        Ok(self.snapshot())
    }
}

impl Default for TunManager {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Event broadcasting helper (lives in its own sub-module so the
// `tauri::Manager` import is in one place).
// ============================================================================
pub mod tun_events {
    use super::TunStatus;
    use crate::events::TUN_STATE_CHANGED;
    use tauri::{AppHandle, Emitter};

    /// Broadcast the current TUN status to every window. Best-effort:
    /// if the AppHandle cannot be located (e.g. very early boot), the
    /// state is still recorded in `TunManager.inner` and the next
    /// `get_tun_state` poll will pick it up.
    pub fn broadcast_state(snap: &TunStatus) {
        if let Some(app) = try_app_handle() {
            let _ = app.emit(TUN_STATE_CHANGED, snap.clone());
        }
    }

    fn try_app_handle() -> Option<AppHandle> {
        // The Manager is stored in a thread-local-free zone; we look
        // it up via a global registry that the runtime installer
        // populates once `setup` runs. See `core::elevate::install`.
        crate::core::elevate::current_app_handle()
    }
}

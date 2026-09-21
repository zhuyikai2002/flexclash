//! Kernel lifecycle commands — the only entry points the frontend uses to
//! control the Mihomo sidecar. Frontend reads live data DIRECTLY from
//! Mihomo's REST/WS, never through Rust.

use tauri::{AppHandle, Emitter, Runtime, State};

use crate::core::sidecar::{self, KernelState, SidecarHandle};
use crate::core::tun;
use crate::error::Result;

#[tauri::command]
#[specta::specta]
pub async fn start_kernel<R: Runtime>(
    app: AppHandle<R>,
    handle: State<'_, SidecarHandle>,
) -> Result<KernelState> {
    // TUN owns the controller/mixed ports while Enabling/On. Return
    // immediately with a `Stopped` snapshot and never touch `sidecar::start`
    // (nor the underlying spawn). This is what keeps the frontend's
    // `ensureRunning` / cold-mount / webview-reload path from resurrecting
    // the regular kernel behind TUN's back.
    if tun::owns_ports(&app) {
        let msg = "[kernel] start_kernel ignored — TUN owns the kernel (9091/7897)";
        eprintln!("{msg}");
        let _ = app.emit(crate::events::KERNEL_LOG, msg);
        return Ok(KernelState::Stopped);
    }
    sidecar::start(&app, handle.inner().clone()).await?;
    Ok(handle.state())
}

#[tauri::command]
#[specta::specta]
pub async fn stop_kernel(handle: State<'_, SidecarHandle>) -> Result<KernelState> {
    sidecar::stop(handle.inner().clone())?;
    Ok(handle.state())
}

#[tauri::command]
#[specta::specta]
pub async fn restart_kernel<R: Runtime>(
    app: AppHandle<R>,
    handle: State<'_, SidecarHandle>,
) -> Result<KernelState> {
    // Same hard stop as `start_kernel`: TUN owns the ports, so return a
    // `Stopped` snapshot without entering `sidecar::restart`/`start`.
    if tun::owns_ports(&app) {
        let msg = "[kernel] restart_kernel ignored — TUN owns the kernel (9091/7897)";
        eprintln!("{msg}");
        let _ = app.emit(crate::events::KERNEL_LOG, msg);
        return Ok(KernelState::Stopped);
    }
    sidecar::restart(&app, handle.inner().clone()).await?;
    Ok(handle.state())
}

#[tauri::command]
#[specta::specta]
pub fn get_kernel_state(handle: State<'_, SidecarHandle>) -> KernelState {
    handle.state()
}

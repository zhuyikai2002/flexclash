//! Kernel lifecycle commands — the only entry points the frontend uses to
//! control the Mihomo sidecar. Frontend reads live data DIRECTLY from
//! Mihomo's REST/WS, never through Rust.

use tauri::{AppHandle, Emitter, Runtime, State};

use crate::core::sidecar::{self, KernelState, SidecarHandle};
use crate::core::tun;
use crate::error::{AppError, Result};

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
        // TUN is the running kernel: report `Running` so the UI keeps proxy
        // switching / speed tests / traffic enabled, but do NOT spawn anything.
        return Ok(KernelState::Running);
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
        return Ok(KernelState::Running);
    }
    sidecar::restart(&app, handle.inner().clone()).await?;
    Ok(handle.state())
}

#[tauri::command]
#[specta::specta]
pub fn get_kernel_state<R: Runtime>(
    app: AppHandle<R>,
    handle: State<'_, SidecarHandle>,
) -> KernelState {
    // While TUN owns the controller the elevated kernel serves 9091/7897, so
    // the authoritative state is `Running` even though the regular sidecar
    // handle is `Stopped`. This is what keeps the renderer's policy-group
    // selector, node speed tests and traffic graph enabled.
    if tun::owns_ports(&app) {
        return KernelState::Running;
    }
    handle.state()
}

/// Version of the bundled mihomo binary, e.g. `"v1.19.32"`.
///
/// Deliberately distinct from `commands::mihomo::get_mihomo_version`, which
/// asks the *running* controller (`GET /version`) and therefore fails whenever
/// no kernel is up. This one reads the binary itself, so the About panel and
/// the subscribe dialog's `User-Agent` hint work before the proxy has ever been
/// started — and it stays correct across a kernel swap, because there is no
/// literal on the frontend to drift.
#[tauri::command]
#[specta::specta]
pub async fn get_kernel_binary_version<R: Runtime>(app: AppHandle<R>) -> Result<String> {
    // `mihomo -v` spawns a process and waits for it. Doing that on an async
    // worker would occupy a tokio thread, and because every Tauri IPC call is
    // dispatched onto the same runtime, a single blocked worker makes the
    // entire app look frozen — the exact failure `commands::tun::off_runtime`
    // was introduced to eliminate. Never call `probe_kernel_version` inline.
    match tauri::async_runtime::spawn_blocking(move || {
        crate::core::kernel_version::probe_kernel_version(&app)
    })
    .await
    {
        Ok(res) => res,
        Err(e) => Err(AppError::Shell(format!(
            "kernel version probe task failed: {e}"
        ))),
    }
}

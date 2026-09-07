//! Kernel lifecycle commands — the only entry points the frontend uses to
//! control the Mihomo sidecar. Frontend reads live data DIRECTLY from
//! Mihomo's REST/WS, never through Rust.

use tauri::{AppHandle, Runtime, State};

use crate::core::sidecar::{self, KernelState, SidecarHandle};
use crate::error::Result;

#[tauri::command]
pub async fn start_kernel<R: Runtime>(
    app: AppHandle<R>,
    handle: State<'_, SidecarHandle>,
) -> Result<KernelState> {
    sidecar::start(&app, handle.inner().clone()).await?;
    Ok(handle.state())
}

#[tauri::command]
pub async fn stop_kernel(handle: State<'_, SidecarHandle>) -> Result<KernelState> {
    sidecar::stop(handle.inner().clone())?;
    Ok(handle.state())
}

#[tauri::command]
pub async fn restart_kernel<R: Runtime>(
    app: AppHandle<R>,
    handle: State<'_, SidecarHandle>,
) -> Result<KernelState> {
    sidecar::restart(&app, handle.inner().clone()).await?;
    Ok(handle.state())
}

#[tauri::command]
pub fn get_kernel_state(handle: State<'_, SidecarHandle>) -> KernelState {
    handle.state()
}

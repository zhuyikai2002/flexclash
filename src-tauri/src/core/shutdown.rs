// ============================================================================
// core/shutdown.rs — Application-exit coordination.
//
// Background-resident behaviour (M6):
//   - The "X" button on the window MUST NOT terminate the kernel or the
//     user-space proxy settings. Instead the window is hidden and the app
//     keeps running in the system tray so traffic continues to flow.
//   - The only way to *actually* exit is via the tray "Quit" menu item,
//     which sets a global flag and then triggers a second close.
//
// Order of operations on a real exit:
//   1. Restore the Windows system proxy (idempotent, even when the user
//      never enabled it). MUST run *before* the sidecar dies, otherwise
//      the user's browser sees `ERR_PROXY_CONNECTION_FAILED` for ~200ms.
//   2. Kill the mihomo sidecar gracefully.
//   3. `hard_cleanup` (taskkill fallback) for any zombie children.
// ============================================================================

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Manager, Runtime, WindowEvent};

use crate::core::sidecar::{self, SidecarHandle};
use crate::proxy;

/// Global flag: when true, the next `CloseRequested` performs a real exit
/// (proxy restore + sidecar kill + drop window). When false, the close is
/// intercepted and the window is hidden (background-resident behaviour).
#[derive(Default, Clone)]
pub struct ExitFlag(pub Arc<AtomicBool>);

impl ExitFlag {
    pub fn request(&self) { self.0.store(true, Ordering::SeqCst); }
    pub fn should_exit(&self) -> bool { self.0.load(Ordering::SeqCst) }
}

/// Install window-close hook. Call once from `setup`.
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    install_window_close_hook(app);
}

fn install_window_close_hook<R: Runtime>(app: &AppHandle<R>) {
    let Some(win) = app.get_webview_window("main") else { return; };
    let app_handle = app.clone();
    win.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            let want_exit = app_handle
                .try_state::<ExitFlag>()
                .map(|s| s.should_exit())
                .unwrap_or(false);

            if !want_exit {
                // User clicked "X" (or system forced close). Stay alive in
                // the tray instead of tearing the app down. This keeps the
                // proxy up, the kernel running, and lets the user reopen
                // the window with one tray click.
                eprintln!("[shutdown] CloseRequested intercepted -> hide to tray");
                api.prevent_close();
                if let Some(w) = app_handle.get_webview_window("main") {
                    let _ = w.hide();
                }
                return;
            }

            // Real exit path.
            eprintln!("[shutdown] CloseRequested (real exit) — running cleanup");

            // (1) Restore the system proxy FIRST.
            if let Err(e) = proxy::disable_system_proxy() {
                eprintln!("[shutdown] WARNING: failed to restore system proxy: {e}");
            }

            // (2) Graceful sidecar shutdown.
            if let Some(state) = app_handle.try_state::<SidecarHandle>() {
                let h = state.inner().clone();
                if let Err(e) = h.try_kill() {
                    eprintln!("[shutdown] sidecar kill failed: {e}");
                }
            }

            // (3) Hard cleanup fallback.
            sidecar::hard_cleanup();
        }
    });
}

// ---------------------------------------------------------------------------
// Read-only helpers used by the UI layer (commands/proxy.rs).
// ---------------------------------------------------------------------------

/// Current effective system-proxy state. Not part of the shutdown path.
pub fn current_system_proxy_state() -> Option<proxy::ProxyStatus> {
    proxy::query_system_proxy_status()
}

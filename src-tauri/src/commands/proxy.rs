// ============================================================================
// commands/proxy.rs — Tauri command surface for system-proxy control.
// ============================================================================

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use crate::core::shutdown;
use crate::error::AppError;
use crate::events::SYSTEM_PROXY_CHANGED;
use crate::proxy::{self, ProxyStatus};

type CmdResult<T> = Result<T, AppError>;

#[derive(Debug, Serialize)]
pub struct ProxyToggleResult {
    pub enabled: bool,
    pub port: Option<u16>,
    pub detail: String,
}

/// Turn the Windows system proxy on, pointing at `127.0.0.1:<port>`.
/// If `port` is None we use mihomo's default mixed-port (7890).
#[tauri::command]
pub fn enable_system_proxy<R: Runtime>(
    app: AppHandle<R>,
    port: Option<u16>,
) -> CmdResult<ProxyToggleResult> {
    let p = port.unwrap_or(7890);
    proxy::set_system_proxy(p).map_err(AppError::Proxy)?;
    let _ = app.emit(
        SYSTEM_PROXY_CHANGED,
        serde_json::json!({
            "enabled": true,
            "port": p,
            "source": "command",
        }),
    );
    Ok(ProxyToggleResult {
        enabled: true,
        port: Some(p),
        detail: format!("system proxy -> 127.0.0.1:{p}"),
    })
}

#[tauri::command]
pub fn disable_system_proxy<R: Runtime>(app: AppHandle<R>) -> CmdResult<ProxyToggleResult> {
    proxy::disable_system_proxy().map_err(AppError::Proxy)?;
    let _ = app.emit(
        SYSTEM_PROXY_CHANGED,
        serde_json::json!({
            "enabled": false,
            "port": Option::<u16>::None,
            "source": "command",
        }),
    );
    Ok(ProxyToggleResult {
        enabled: false,
        port: None,
        detail: "system proxy disabled".into(),
    })
}

#[tauri::command]
pub fn get_system_proxy_status() -> CmdResult<Option<ProxyStatus>> {
    Ok(shutdown::current_system_proxy_state())
}

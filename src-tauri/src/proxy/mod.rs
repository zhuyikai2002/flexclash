//! proxy — Cross-platform system proxy abstraction.
//!
//! Phase 1 implementation lives entirely on Windows (M5). Other OSes
//! provide stub implementations that succeed without effect so the rest
//! of the app can still compile and run in dev mode.

use std::time::Duration;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod unix;

/// Public façade. On Windows this delegates to the Win32 / WinINET path;
/// on other platforms it returns a `Unsupported` result so the UI can
/// surface a clear "not implemented" toast.
pub fn set_system_proxy(port: u16) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    { windows::set_system_proxy(port) }
    #[cfg(not(target_os = "windows"))]
    { unix::set_system_proxy(port) }
}

pub fn disable_system_proxy() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    { windows::disable_system_proxy() }
    #[cfg(not(target_os = "windows"))]
    { unix::disable_system_proxy() }
}

/// Read the current effective system-proxy state from the registry.
/// Returns:
///   * `Some((true,  "host:port"))` if a proxy is currently set
///   * `Some((false, ...))` if the user has explicitly disabled it
///   * `None` if the keys are missing entirely
pub fn query_system_proxy_status() -> Option<ProxyStatus> {
    #[cfg(target_os = "windows")]
    { windows::query_system_proxy_status() }
    #[cfg(not(target_os = "windows"))]
    { unix::query_system_proxy_status() }
}

/// Convenience: parse "host:port" into `(host, port)`.
pub fn parse_proxy_server(s: &str) -> Option<(&str, u16)> {
    let (h, p) = s.rsplit_once(':')?;
    let port = p.parse::<u16>().ok()?;
    Some((h, port))
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProxyStatus {
    /// Whether the user has the system proxy enabled (ProxyEnable=1).
    pub enabled: bool,
    /// Raw "host:port" string. May be empty even when `enabled` is true
    /// (rare but possible on freshly-built Windows images).
    pub server: String,
    /// Current `ProxyOverride` (bypass rules) — informational only.
    pub override_rules: String,
}

/// Default bypass list we apply when we *enable* the proxy. Keeps LAN
/// resources reachable and avoids routing the loopback to mihomo.
pub const DEFAULT_PROXY_OVERRIDE: &str =
    "localhost;127.*;10.*;172.16.*;192.168.*;<local>";

/// Extra time we wait between writing the registry and refreshing WinINET.
/// 200ms is enough for HKCU writes to settle on Windows 10/11.
pub const POST_WRITE_SETTLE: Duration = Duration::from_millis(200);

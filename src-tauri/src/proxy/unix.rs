// ============================================================================
// proxy/unix.rs — Stub. Linux (gsettings) and macOS (networksetup) will be
// filled in a later phase. Returning `Err` keeps the rest of the codebase
// honest about the platform limitation.
// ============================================================================

use super::ProxyStatus;

pub fn set_system_proxy(_port: u16) -> Result<(), String> {
    Err("system proxy is only implemented on Windows in this build".into())
}

pub fn disable_system_proxy() -> Result<(), String> {
    Err("system proxy is only implemented on Windows in this build".into())
}

pub fn query_system_proxy_status() -> Option<ProxyStatus> { None }

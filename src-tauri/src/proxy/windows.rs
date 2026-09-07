// ============================================================================
// proxy/windows.rs — Windows system-proxy implementation.
//
// Talks to:
//   HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings
//     ProxyEnable   (DWORD32)  0 / 1
//     ProxyServer   (REG_SZ)   "127.0.0.1:7890"
//     ProxyOverride (REG_SZ)   "localhost;127.*;10.*;172.16.*;192.168.*;<local>"
//
// Then calls `InternetSetOptionW(NULL, INTERNET_OPTION_SETTINGS_CHANGED, ...)`
// + `INTERNET_OPTION_REFRESH` to force all WinINET consumers (Edge, Chrome,
// Spotify, Outlook, the Settings app, …) to drop their cached proxy state.
//
// Refs:
//   - https://learn.microsoft.com/en-us/windows/win32/wininet/option-flags
//   - https://learn.microsoft.com/en-us/windows/win32/api/wininet/nf-wininet-internetsetoptionw
// ============================================================================

use windows::Win32::Networking::WinInet::{
    InternetSetOptionW, INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED,
};
use winreg::enums::*;
use winreg::RegKey;

use super::{parse_proxy_server, ProxyStatus, DEFAULT_PROXY_OVERRIDE, POST_WRITE_SETTLE};

const INTERNET_SETTINGS_PATH: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

// ---------------------------------------------------------------------------
// Public surface
// ---------------------------------------------------------------------------

pub fn set_system_proxy(port: u16) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(INTERNET_SETTINGS_PATH)
        .map_err(|e| format!("open HKCU Internet Settings: {e}"))?;

    let server = format!("127.0.0.1:{port}");
    let override_str = DEFAULT_PROXY_OVERRIDE.to_string();

    // The order matters on some Windows versions: write `ProxyServer` and
    // `ProxyOverride` BEFORE flipping `ProxyEnable` to 1, otherwise WinINET
    // can briefly read a half-configured state.
    key.set_value("ProxyServer", &server)
        .map_err(|e| format!("set ProxyServer: {e}"))?;
    key.set_value("ProxyOverride", &override_str)
        .map_err(|e| format!("set ProxyOverride: {e}"))?;
    let enable: u32 = 1;
    key.set_value("ProxyEnable", &enable)
        .map_err(|e| format!("set ProxyEnable: {e}"))?;

    notify_wininet()
}

/// Disable the system proxy. Idempotent — safe to call when already off.
pub fn disable_system_proxy() -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(INTERNET_SETTINGS_PATH)
        .map_err(|e| format!("open HKCU Internet Settings: {e}"))?;
    let zero: u32 = 0;
    // Set ProxyEnable=0 but LEAVE ProxyServer in place — that way a
    // re-enable can reuse the same value, and we don't disturb other
    // software that may also write that key.
    key.set_value("ProxyEnable", &zero)
        .map_err(|e| format!("clear ProxyEnable: {e}"))?;

    notify_wininet()
}

pub fn query_system_proxy_status() -> Option<ProxyStatus> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey(INTERNET_SETTINGS_PATH).ok()?;
    let enable: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    let server: String = key.get_value("ProxyServer").unwrap_or_default();
    let override_rules: String = key.get_value("ProxyOverride").unwrap_or_default();
    // Only treat as "set" if the user enabled it AND the server parses
    // (i.e. "host:port"). A naked `ProxyEnable=1` with no server is
    // garbage from some other installer and we should not present it.
    if enable == 1 {
        parse_proxy_server(&server)?;
    }
    Some(ProxyStatus {
        enabled: enable == 1,
        server,
        override_rules,
    })
}

// ---------------------------------------------------------------------------
// WinINET refresh — broadcast the proxy change to all listening apps.
// ---------------------------------------------------------------------------

fn notify_wininet() -> Result<(), String> {
    // Wait briefly for the registry to settle. Without this, some apps
    // (notably Edge) occasionally read the OLD value after the notification.
    std::thread::sleep(POST_WRITE_SETTLE);

    // Passing `None` for the HINTERNET is the documented way to broadcast
    // to all WinINET clients. Both flags must be sent.
    unsafe {
        InternetSetOptionW(None, INTERNET_OPTION_SETTINGS_CHANGED, None, 0)
            .map_err(|e| format!("InternetSetOptionW(SETTINGS_CHANGED) failed: {e}"))?;
        InternetSetOptionW(None, INTERNET_OPTION_REFRESH, None, 0)
            .map_err(|e| format!("InternetSetOptionW(REFRESH) failed: {e}"))?;
    }
    Ok(())
}

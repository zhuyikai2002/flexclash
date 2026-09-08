// ============================================================================
// proxy/unix.rs — System-proxy implementation for Unix desktops.
//
//   Linux  → GNOME gsettings (`org.gnome.system.proxy`).  This is the
//            de-facto standard on GNOME Shell / Ubuntu / Arch+GNOME.
//            Non-GNOME desktops (KDE, etc.) keep returning an explicit
//            "not implemented" error so the UI can surface it.
//   macOS  → Stub (networksetup to be filled in a later phase).
//
// The port/UI contract matches `proxy/mod.rs`: the app only ever targets
// 127.0.0.1:<port>, so `set_system_proxy(port)` = enable manual proxy at
// 127.0.0.1:<port>. `disable_system_proxy()` resets mode to 'none'.
// ============================================================================

use super::ProxyStatus;

const HOST: &str = "127.0.0.1";

/// Run a gsettings command and map failures to a readable string.
fn gset(schema_key: &str, value: &str) -> Result<(), String> {
    // gsettings expects the value *already shell-quoted* for strings
    // (e.g. "'manual'") but plain integers pass through unquoted.
    let out = std::process::Command::new("gsettings")
        .arg("set")
        .arg(schema_key)
        .arg(value)
        .output()
        .map_err(|e| format!("gsettings not found / spawn failed: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        return Err(format!("gsettings set {schema_key} {value} -> {stderr}"));
    }
    Ok(())
}

/// Enable a manual proxy for one gsettings "family" (http / https / ftp).
fn set_family(family: &str, port: u16) -> Result<(), String> {
    // A family may not exist in the schema on every desktop (e.g. some
    // stripped GNOME builds omit ftp). Missing schemas would fail loudly,
    // so we only touch the families that are universally present.
    if matches!(family, "http" | "https" | "ftp") {
        gset(&format!("org.gnome.system.proxy.{family}.host"), &format!("'{HOST}'"))?;
        gset(&format!("org.gnome.system.proxy.{family}.port"), &port.to_string())?;
    }
    Ok(())
}

pub fn set_system_proxy(port: u16) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        gset("org.gnome.system.proxy.mode", "'manual'")?;
        for family in ["http", "https", "ftp"] {
            set_family(family, port)?;
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let _ = port;
        Err("system proxy is only implemented on Windows/Linux in this build".into())
    }
}

pub fn disable_system_proxy() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        gset("org.gnome.system.proxy.mode", "'none'")
    }
    #[cfg(target_os = "macos")]
    {
        Err("system proxy is only implemented on Windows/Linux in this build".into())
    }
}

pub fn query_system_proxy_status() -> Option<ProxyStatus> {
    #[cfg(target_os = "linux")]
    {
        let out = std::process::Command::new("gsettings")
            .arg("get")
            .arg("org.gnome.system.proxy.mode")
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let raw = String::from_utf8_lossy(&out.stdout);
        let raw = raw.trim().trim_matches('\'');
        // gsettings returns `'manual'` / `'none'` (or "none" on some
        // backends). Only `manual` counts as enabled.
        if raw != "manual" {
            return Some(ProxyStatus {
                enabled: false,
                server: String::new(),
                override_rules: String::new(),
            });
        }
        // Read the http port so the server string is complete.
        let port = std::process::Command::new("gsettings")
            .arg("get")
            .arg("org.gnome.system.proxy.http.port")
            .output()
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout).trim().parse::<u16>().ok()
            })
            .unwrap_or(0);
        Some(ProxyStatus {
            enabled: true,
            server: format!("{HOST}:{port}"),
            override_rules: String::new(),
        })
    }
    #[cfg(target_os = "macos")]
    {
        None
    }
}

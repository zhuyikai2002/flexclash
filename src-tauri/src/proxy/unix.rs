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

/// Run a `gsettings set` command and map failures to a readable string.
///
/// `gsettings set` takes the schema and key as **separate** argv elements:
///
/// ```text
/// gsettings set org.gnome.system.proxy mode manual
/// ```
///
/// Previously the schema and key were joined into one argument
/// (`org.gnome.system.proxy.mode`), which gsettings rejected with its
/// "usage" error. String values (`manual`, `none`, `127.0.0.1`) must also
/// be passed as bare strings — no outer single/double quotes — because
/// `Command` performs no shell interpretation.
fn gset(schema: &str, key: &str, value: &str) -> Result<(), String> {
    let out = std::process::Command::new("gsettings")
        .arg("set")
        .arg(schema)
        .arg(key)
        .arg(value)
        .output()
        .map_err(|e| format!("gsettings not found / spawn failed: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        return Err(format!("gsettings set {schema} {key} {value} -> {stderr}"));
    }
    Ok(())
}

/// Enable a manual proxy for one gsettings "family" (http / https / socks).
fn set_family(family: &str, port: u16) -> Result<(), String> {
    // A family may not exist in the schema on every desktop, but http,
    // https and socks are the universally present GNOME proxy families.
    // mihomo's `mixed-port` serves HTTP and SOCKS on the same port, so
    // all three families share the single inbound port.
    if matches!(family, "http" | "https" | "socks") {
        gset(&format!("org.gnome.system.proxy.{family}"), "host", HOST)?;
        gset(
            &format!("org.gnome.system.proxy.{family}"),
            "port",
            &port.to_string(),
        )?;
    }
    Ok(())
}

pub fn set_system_proxy(port: u16) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        gset("org.gnome.system.proxy", "mode", "manual")?;
        for family in ["http", "https", "socks"] {
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
        gset("org.gnome.system.proxy", "mode", "none")
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
            .arg("org.gnome.system.proxy")
            .arg("mode")
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
            .arg("org.gnome.system.proxy.http")
            .arg("port")
            .output()
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<u16>()
                    .ok()
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

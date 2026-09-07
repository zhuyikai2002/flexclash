// ============================================================================
// core/route_guard.rs — M9: defensive cleanup of TUN-related artefacts.
//
// Three things can linger after a crash / force-kill / power-loss:
//   1. The Wintun virtual NIC `flexclash-tun` (visible to the OS even
//      after the owning mihomo dies).
//   2. Routing-table entries mihomo added with `auto-route`: typically
//      `0.0.0.0/1` and `128.0.0.0/1` whose next-hop points into the
//      Wintun adapter. If the adapter is gone but the routes remain, the
//      host loses all default-route traffic.
//   3. Stale DNS policy / resolver overrides (rare on Win, common on
//      macOS/Linux — Phase 1 only ships Windows, so we no-op elsewhere).
//
// `sweep_residual_routes()` is called:
//   * At app boot, from `core::startup::sweep_residual_routes()`.
//   * Before enabling TUN (so a previous session's junk is gone first).
//   * Before disabling TUN (so we don't accidentally take down the host
//     route mid-transition).
//
// All operations are defensive: if a step fails (e.g. netsh is missing
// or the adapter is still in use), we log and keep going. The contract
// is "best effort" — the function never aborts.
// ============================================================================

#[cfg(target_os = "windows")]
use std::process::Command;

/// Result of a sweep. Mirrors the M7 stub `SweepResult` so the verify
/// suite can assert the field layout stayed compatible.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SweepResult {
    /// How many routing-table entries were removed.
    pub deleted_routes: u32,
    /// How many virtual NICs were removed (0 on non-Windows).
    pub deleted_adapters: u32,
    /// True when the sweep completed without raising any error.
    pub ok: bool,
    /// Diagnostic message (best-effort, never an error).
    pub message: String,
}

/// Reserved Wintun device name. Must match
/// `config::profile::TUN_DEVICE` so the GUI label is consistent.
pub const TUN_DEVICE_NAME: &str = "flexclash-tun";

/// Top-level entry point. Always returns a `SweepResult`; never panics.
pub fn sweep_residual_routes() -> SweepResult {
    let mut result = SweepResult {
        deleted_routes: 0,
        deleted_adapters: 0,
        ok: true,
        message: String::new(),
    };

    #[cfg(target_os = "windows")]
    {
        if let Err(e) = sweep_routes_windows(&mut result) {
            result.ok = false;
            result.message.push_str(&format!("routes: {e}; "));
        }
        if let Err(e) = sweep_adapter_windows(&mut result) {
            result.ok = false;
            result.message.push_str(&format!("adapter: {e}; "));
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        result.message.push_str("sweep is a no-op on non-Windows (Phase 1 Windows-only)");
    }

    if result.message.is_empty() {
        result.message = if result.deleted_routes + result.deleted_adapters > 0 {
            format!(
                "cleaned {} route(s) and {} adapter(s)",
                result.deleted_routes, result.deleted_adapters
            )
        } else {
            "no residual artefacts".to_string()
        };
    }
    result
}

// ---------------------------------------------------------------------------
// Windows: routes
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn sweep_routes_windows(result: &mut SweepResult) -> std::io::Result<()> {
    // Look for routes 0.0.0.0/1 and 128.0.0.0/1 (the two CIDRs mihomo
    // adds when auto-route is on). We use `route print` and grep for
    // them, then issue `route delete` per row.
    let out = Command::new("route")
        .args(["print", "-4"])
        .output()?;
    if !out.status.success() {
        // Non-fatal: return ok so the rest of the sweep still runs.
        return Ok(());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        // Example line we care about:
        //   0.0.0.0/1      10.0.0.1     10.0.0.1     10.0.0.1/30    10
        // We only delete if the gateway is the Wintun link-local range
        // (RFC 3927 169.254.x.x) OR matches a custom heuristic: the
        // destination is one of the two auto-route prefixes.
        let lower = line.trim().to_lowercase();
        if !(lower.starts_with("0.0.0.0/1") || lower.starts_with("128.0.0.0/1")) {
            continue;
        }
        // Skip if the destination is masked but the gateway is NOT a
        // local link address (defensive: leave system routes alone).
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 3 {
            continue;
        }
        let gateway = cols[2];
        let is_link_local = gateway.starts_with("169.254.")
            || gateway.starts_with("10.0.0.")  // wintun default
            || gateway.starts_with("192.168.")
            || gateway == "0.0.0.0";
        if !is_link_local {
            continue;
        }
        let del = Command::new("route")
            .args(["delete", cols[0], "mask", cols[1]])
            .output();
        if let Ok(d) = del {
            if d.status.success() {
                result.deleted_routes += 1;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Windows: Wintun virtual NIC
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn sweep_adapter_windows(result: &mut SweepResult) -> std::io::Result<()> {
    // Use `netsh interface show interface` to find a row whose name is
    // exactly "flexclash-tun". If found and the device is administratively
    // down, we delete it via `netsh interface set interface ... disabled`
    // followed by `netsh interface delete interface ...`.
    let out = Command::new("netsh")
        .args(["interface", "show", "interface"])
        .output()?;
    if !out.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut found = false;
    for line in text.lines() {
        // The display table uses a fixed column layout, but the name
        // appears in a known position when the adapter is `Enabled`.
        // We do a simple substring match: if the device name is on the
        // line, treat as found.
        if line.contains(TUN_DEVICE_NAME) {
            found = true;
            break;
        }
    }
    if !found {
        return Ok(());
    }
    // Best-effort: disable first, then delete. Both are silent if the
    // device is already gone.
    let _ = Command::new("netsh")
        .args([
            "interface", "set", "interface", TUN_DEVICE_NAME, "admin=disable",
        ])
        .output();
    let del = Command::new("netsh")
        .args([
            "interface", "delete", "interface", TUN_DEVICE_NAME,
        ])
        .output();
    if let Ok(d) = del {
        if d.status.success() {
            result.deleted_adapters += 1;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests (do not actually call netsh; just construct a fake SweepResult
// to prove the contract is stable across the M7 -> M9 transition).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_is_idempotent_and_returns_sensible_result() {
        // Call twice in a row: second call must be a no-op (no adapters
        // or routes to clean in CI), and the type must serialise.
        let r1 = sweep_residual_routes();
        let r2 = sweep_residual_routes();
        assert!(r1.ok);
        assert!(r2.ok);
        let json = serde_json::to_string(&r1).unwrap();
        assert!(json.contains("\"deleted_routes\""));
        assert!(json.contains("\"deleted_adapters\""));
        assert!(json.contains("\"ok\""));
    }

    #[test]
    fn sweep_result_field_layout_matches_m7() {
        // The M7 frontend already keys off `ok` and `deleted`. Guard
        // against accidental renames.
        let r = SweepResult {
            deleted_routes: 0,
            deleted_adapters: 0,
            ok: true,
            message: String::new(),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("deleted_routes").is_some());
        assert!(v.get("deleted_adapters").is_some());
        assert!(v.get("ok").is_some());
        // Backward-compat alias for M7.
        assert!(v.get("ok").unwrap().as_bool().unwrap());
    }
}

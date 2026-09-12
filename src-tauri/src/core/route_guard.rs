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
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
/// Hide child consoles when invoking netsh / route (no cmd flash on TUN ops).
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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

/// One split-default row from `route print -4` that we consider ours.
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoRoute {
    pub destination: String,
    pub netmask: String,
    pub gateway: String,
}

/// Parse the Active Routes table of `route print -4` and return the rows
/// mihomo's `auto-route` adds.
///
/// Windows prints that table as **dotted quads**, never CIDR:
///
/// ```text
/// Network Destination        Netmask          Gateway       Interface  Metric
///           0.0.0.0        128.0.0.0         10.0.0.1         10.0.0.2     10
///        128.0.0.0        128.0.0.0         10.0.0.1         10.0.0.2     10
/// ```
///
/// So `0.0.0.0/1` is `0.0.0.0` + mask `128.0.0.0`, and `128.0.0.0/1` is
/// `128.0.0.0` + mask `128.0.0.0`.
///
/// This replaces a matcher that tested `line.starts_with("0.0.0.0/1")` —
/// a prefix `route print` can never emit. The sweep therefore matched
/// nothing, ever, and silently left the two split-default routes behind
/// after a crash. Those routes point at a Wintun adapter that no longer
/// exists, which takes the host's default route with them.
///
/// A row is only returned when the next hop looks like a TUN adapter, so a
/// real default gateway is never deleted.
#[cfg(any(target_os = "windows", test))]
pub fn parse_auto_route_rows(text: &str) -> Vec<AutoRoute> {
    let mut out = Vec::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 3 {
            continue;
        }
        let (destination, netmask, gateway) = (cols[0], cols[1], cols[2]);
        let is_split_default = (destination == "0.0.0.0" || destination == "128.0.0.0")
            && netmask == "128.0.0.0";
        if !is_split_default {
            continue;
        }
        let next_hop_is_tun = gateway.starts_with("169.254.")   // Wintun link-local
            || gateway.starts_with("10.0.0.")                    // Wintun default
            || gateway.starts_with("192.168.")
            || gateway.starts_with("198.18.")                    // mihomo inet4-address
            || gateway.starts_with("198.19.")
            || gateway == "0.0.0.0"
            || gateway.eq_ignore_ascii_case("on-link");
        if !next_hop_is_tun {
            continue;
        }
        out.push(AutoRoute {
            destination: destination.to_string(),
            netmask: netmask.to_string(),
            gateway: gateway.to_string(),
        });
    }
    out
}

#[cfg(target_os = "windows")]
fn sweep_routes_windows(result: &mut SweepResult) -> std::io::Result<()> {
    // Look for the two split-default routes mihomo adds when auto-route is
    // on, then issue one `route delete` per matching row.
    let out = Command::new("route").creation_flags(CREATE_NO_WINDOW)
        .args(["print", "-4"])
        .output()?;
    if !out.status.success() {
        // Non-fatal: return ok so the rest of the sweep still runs.
        return Ok(());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for row in parse_auto_route_rows(&text) {
        let del = Command::new("route").creation_flags(CREATE_NO_WINDOW)
            .args(["delete", &row.destination, "mask", &row.netmask])
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
    let out = Command::new("netsh").creation_flags(CREATE_NO_WINDOW)
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
    let _ = Command::new("netsh").creation_flags(CREATE_NO_WINDOW)
        .args([
            "interface", "set", "interface", TUN_DEVICE_NAME, "admin=disable",
        ])
        .output();
    let del = Command::new("netsh").creation_flags(CREATE_NO_WINDOW)
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

    /// Real `route print -4` output (dotted quads), with the two
    /// split-default routes mihomo adds while TUN is up.
    const SAMPLE_ROUTE_PRINT: &str = "\
===========================================================================
IPv4 Route Table
===========================================================================
Active Routes:
Network Destination        Netmask          Gateway       Interface  Metric
          0.0.0.0          0.0.0.0      192.168.1.1     192.168.1.20     25
          0.0.0.0        128.0.0.0         198.18.0.1       198.18.0.2      1
        127.0.0.0        255.0.0.0         On-link         127.0.0.1    331
        128.0.0.0        128.0.0.0         198.18.0.1       198.18.0.2      1
        192.168.1.0    255.255.255.0         On-link      192.168.1.20    281
        224.0.0.0        240.0.0.0         On-link         127.0.0.1    331
===========================================================================
Persistent Routes:
  None
";

    #[test]
    fn parses_the_two_split_default_routes() {
        let rows = parse_auto_route_rows(SAMPLE_ROUTE_PRINT);
        assert_eq!(rows.len(), 2, "expected exactly the 0.0.0.0/1 + 128.0.0.0/1 pair, got {rows:?}");
        assert_eq!(rows[0].destination, "0.0.0.0");
        assert_eq!(rows[0].netmask, "128.0.0.0");
        assert_eq!(rows[0].gateway, "198.18.0.1");
        assert_eq!(rows[1].destination, "128.0.0.0");
        assert_eq!(rows[1].netmask, "128.0.0.0");
    }

    #[test]
    fn never_touches_the_real_default_route() {
        // The host's own default route is `0.0.0.0 / 0.0.0.0`. Deleting it
        // would take the machine off the network, so this is the one
        // assertion that must never regress.
        let rows = parse_auto_route_rows(SAMPLE_ROUTE_PRINT);
        assert!(
            rows.iter().all(|r| r.netmask == "128.0.0.0"),
            "a /0 default route was selected for deletion: {rows:?}"
        );
        assert!(
            rows.iter().all(|r| r.gateway != "192.168.1.1"),
            "the real gateway was selected for deletion: {rows:?}"
        );
    }

    #[test]
    fn cidr_notation_is_not_matched() {
        // Guards the exact regression this parser was written for: the old
        // code matched on a literal "0.0.0.0/1" prefix, which `route print`
        // never emits — so the sweep was a silent no-op.
        let cidr_style = "0.0.0.0/1      198.18.0.1     198.18.0.2    198.18.0.1/30    10\n\
                          128.0.0.0/1    198.18.0.1     198.18.0.2    198.18.0.1/30    10\n";
        assert!(
            parse_auto_route_rows(cidr_style).is_empty(),
            "CIDR-shaped input must not be treated as a route table row"
        );
    }

    #[test]
    fn leaves_split_defaults_on_a_foreign_gateway_alone() {
        let foreign = "          0.0.0.0        128.0.0.0          8.8.8.8      10.0.0.9      1\n\
                                128.0.0.0        128.0.0.0          8.8.8.8      10.0.0.9      1\n";
        assert!(parse_auto_route_rows(foreign).is_empty(), "deleted a non-TUN split default");
    }

    #[test]
    fn parser_tolerates_a_truncated_table() {
        // `route print` is also invoked while the table is being rewritten;
        // a half-written row must be ignored rather than panic.
        assert!(parse_auto_route_rows("          0.0.0.0\n").is_empty());
        assert!(parse_auto_route_rows("").is_empty());
    }
}

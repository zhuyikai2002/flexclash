// ============================================================================
// core/startup.rs — M7: startup argument parsing + OS feature detection.
//
// Responsibilities:
//   * Parse `argv` to detect the `--silent` / `--minimized` launch flag.
//     When the OS invokes us through the autostart `HKCU\...\Run\FlexClash`
//     entry the plugin appends `--silent`, so the main window must stay
//     hidden and only the tray icon is visible.
//   * Detect Windows 11 (build >= 22000) to decide between Mica and Acrylic
//     backdrops for `window-vibrancy`.
//   * `SilentFlag` managed state, mirroring the `ExitFlag` pattern.
// ============================================================================

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Flag set at boot when the binary was launched with `--silent` or
/// `--minimized`. Used by `setup` to hide the main window before the
/// first paint and by the UI to render a "started in background" notice.
#[derive(Default, Clone)]
pub struct SilentFlag(pub Arc<AtomicBool>);

impl SilentFlag {
    pub fn set(&self, v: bool) {
        self.0.store(v, Ordering::SeqCst);
    }
    pub fn is_silent(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Inspect `std::env::args()` and return true if the user (or the autostart
/// hook) requested a silent launch.
pub fn parse_silent_flag() -> bool {
    std::env::args().any(|a| a == "--silent" || a == "--minimized" || a == "/silent")
}

/// Identifier the autostart plugin uses as the value name inside
/// `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. Kept here so the
/// round-trip test in `tests/autostart_roundtrip.rs` and any future CLI
/// tooling agree on the exact key.
pub const AUTOSTART_REGISTRY_VALUE: &str = "FlexClash";

/// Path the autostart plugin writes to on Windows. The plugin's
/// `WindowsLauncher` uses the app identifier as the value name by
/// default; we override to the constant above for easier testing.
pub fn autostart_registry_path() -> &'static str {
    r"Software\Microsoft\Windows\CurrentVersion\Run"
}

/// Read-only helper: returns true if the Windows autostart entry exists
/// in the current user's `Run` key. This is the *authoritative* check;
/// the plugin's own `is_enabled()` ultimately queries the same location.
#[cfg(target_os = "windows")]
pub fn is_autostart_registry_entry_present() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let Ok(run_key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(autostart_registry_path())
    else {
        return false;
    };
    run_key
        .get_value::<String, _>(AUTOSTART_REGISTRY_VALUE)
        .is_ok()
}

#[cfg(not(target_os = "windows"))]
pub fn is_autostart_registry_entry_present() -> bool {
    false
}

/// Test-only helper: write/delete the registry entry directly so
/// integration tests can assert on the same value name the plugin uses.
/// NOT exposed to the frontend; lives under `#[cfg(any(test, feature = "testing"))]`.
#[doc(hidden)]
pub fn write_autostart_registry_entry_for_test(present: bool) {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let run = hkcu
            .open_subkey_with_flags(
                autostart_registry_path(),
                winreg::enums::KEY_ALL_ACCESS,
            )
            .expect("open Run key for test");
        if present {
            run.set_value(
                AUTOSTART_REGISTRY_VALUE,
                &r#""C:\fake\path\flexclash.exe" --silent"#,
            )
            .expect("set Run value");
        } else {
            let _ = run.delete_value(AUTOSTART_REGISTRY_VALUE);
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = present;
    }
}

/// Windows 11 build number. Anything >= 22000 is Windows 11 and qualifies
/// for Mica. Windows 10 (1903+) falls back to Acrylic.
#[cfg(target_os = "windows")]
pub fn is_windows_11_or_greater() -> bool {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;
    let Ok(nt) = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
    else {
        return false;
    };
    let build: String = nt.get_value("CurrentBuildNumber").unwrap_or_default();
    build.parse::<u32>().map(|n| n >= 22000).unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
pub fn is_windows_11_or_greater() -> bool {
    false
}

/// Apply the most appropriate native backdrop to the main window. Win11
/// gets Mica, everything else gets Acrylic. Silently no-ops on failure
/// (older Windows builds, or DWM unavailable).
#[cfg(target_os = "windows")]
pub fn apply_native_backdrop(window: &tauri::WebviewWindow) {
    use window_vibrancy::{apply_acrylic, apply_mica};

    if is_windows_11_or_greater() {
        match apply_mica(window, Some(true)) {
            Ok(()) => eprintln!("[startup] backdrop = Mica (Win11, dark=true)"),
            Err(e) => eprintln!("[startup] mica failed, falling back: {e}"),
        }
    } else {
        // Acrylic tint: very dark, low alpha so the dark theme stays readable.
        match apply_acrylic(window, Some((15, 15, 18, 160))) {
            Ok(()) => eprintln!("[startup] backdrop = Acrylic"),
            Err(e) => eprintln!("[startup] acrylic failed, no backdrop: {e}"),
        }
    }
    // Force the webview to honour transparency (background already done in CSS).
    let _ = window.set_decorations(true);
    let _ = window.set_shadow(true);
}

#[cfg(not(target_os = "windows"))]
pub fn apply_native_backdrop(_window: &tauri::WebviewWindow) {
    // No-op on non-Windows (Phase 1 only targets Windows).
}

// ============================================================================
// M9 route guard — delegates to the real implementation in
// `crate::core::route_guard`. The M7-era frontend contract (M7
// `SweepResult { deleted: u32, ok: bool }`) is preserved so the existing
// `sweep_residual_routes` Tauri command keeps returning the same shape.
// The richer M9 SweepResult (routes + adapters + message) is also
// re-exported here for any in-process Rust consumer (TUN manager) that
// needs the full breakdown.
// ============================================================================

pub use crate::core::route_guard::SweepResult as SweepResultFull;

/// M7-shape sweep result kept stable for the `sweep_residual_routes`
/// Tauri command. `deleted = deleted_routes + deleted_adapters`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SweepResult {
    pub deleted: u32,
    pub ok: bool,
}

impl From<crate::core::route_guard::SweepResult> for SweepResult {
    fn from(r: crate::core::route_guard::SweepResult) -> Self {
        SweepResult {
            deleted: r.deleted_routes.saturating_add(r.deleted_adapters),
            ok: r.ok,
        }
    }
}

/// Boot-time / on-demand residual route + adapter sweep. See
/// `crate::core::route_guard::sweep_residual_routes` for the real work.
pub fn sweep_residual_routes() -> SweepResult {
    crate::core::route_guard::sweep_residual_routes().into()
}

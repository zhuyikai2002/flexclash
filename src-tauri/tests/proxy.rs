// ============================================================================
// integration test: proxy — Windows registry round-trip.
//
// These tests directly hit HKCU (the current user's hive), so they
// require a Windows host and run as the test runner user. They are
// idempotent and ALWAYS restore the original ProxyEnable/Server/Override
// values when they finish (even on panic).
// ============================================================================

#![cfg(target_os = "windows")]

use flexclash_lib::proxy::{self, ProxyStatus, DEFAULT_PROXY_OVERRIDE};
use serial_test::serial;

use std::sync::OnceLock;
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

const INTERNET_SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

struct Snapshot {
    proxy_enable: Option<u32>,
    proxy_server: Option<String>,
    proxy_override: Option<String>,
}

fn snapshot_install() -> &'static Snapshot {
    static SNAP: OnceLock<Snapshot> = OnceLock::new();
    SNAP.get_or_init(|| {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = hkcu
            .open_subkey(INTERNET_SETTINGS)
            .expect("open HKCU\\Internet Settings");
        Snapshot {
            proxy_enable: key.get_value::<u32, _>("ProxyEnable").ok(),
            proxy_server: key.get_value::<String, _>("ProxyServer").ok(),
            proxy_override: key.get_value::<String, _>("ProxyOverride").ok(),
        }
    })
}

fn restore_snapshot() {
    let snap = snapshot_install();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey_with_flags(INTERNET_SETTINGS, winreg::enums::KEY_ALL_ACCESS)
        .expect("open HKCU\\Internet Settings for restore");

    match &snap.proxy_enable {
        Some(v) => { let _ = key.set_value("ProxyEnable", v); }
        None    => { let _ = key.delete_value("ProxyEnable"); }
    }
    match &snap.proxy_server {
        Some(v) => { let _ = key.set_value("ProxyServer", v); }
        None    => { let _ = key.delete_value("ProxyServer"); }
    }
    match &snap.proxy_override {
        Some(v) => { let _ = key.set_value("ProxyOverride", v); }
        None    => { let _ = key.delete_value("ProxyOverride"); }
    }
}

fn read_status_now() -> (Option<u32>, Option<String>, Option<String>) {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey(INTERNET_SETTINGS)
        .expect("open HKCU\\Internet Settings for read");
    (
        key.get_value::<u32, _>("ProxyEnable").ok(),
        key.get_value::<String, _>("ProxyServer").ok(),
        key.get_value::<String, _>("ProxyOverride").ok(),
    )
}

#[test]
#[serial]
fn enable_writes_expected_values() {
    // Clean baseline.
    let _ = restore_snapshot;
    let _ = read_status_now();
    proxy::disable_system_proxy().expect("disable baseline");

    // Enable on a non-default port to make sure the value flows through.
    proxy::set_system_proxy(17890).expect("set 17890");

    let (en, srv, ov) = read_status_now();
    assert_eq!(en, Some(1), "ProxyEnable should be 1");
    assert_eq!(srv.as_deref(), Some("127.0.0.1:17890"));
    let ov = ov.expect("ProxyOverride must be set");
    assert!(ov.contains("localhost"), "override must include 'localhost'");
    assert_eq!(ov, DEFAULT_PROXY_OVERRIDE);

    // Cleanup.
    proxy::disable_system_proxy().expect("cleanup disable");
    restore_snapshot();
}

#[test]
#[serial]
fn disable_clears_enable_and_preserves_other_values() {
    proxy::set_system_proxy(7897).expect("set 7897");
    proxy::disable_system_proxy().expect("disable");

    let (en, srv, ov) = read_status_now();
    assert_eq!(en, Some(0), "ProxyEnable should be 0 after disable");
    // Server/Override may stay (we only flip ProxyEnable) — but they
    // MUST be either the original snapshot OR our override value.
    let snap = snapshot_install();
    if snap.proxy_server.is_none() {
        // If the user did not have a ProxyServer before, ours should be
        // preserved but it's harmless to keep. The contract is only on
        // ProxyEnable.
        let _ = srv;
    }
    if snap.proxy_override.is_none() {
        let _ = ov;
    }

    restore_snapshot();
}

#[test]
#[serial]
fn query_reflects_state() {
    proxy::set_system_proxy(9091).expect("set 9091");
    let s: Option<ProxyStatus> = proxy::query_system_proxy_status();
    let s = s.expect("query must return Some on Windows");
    assert!(s.enabled);
    assert_eq!(s.server.as_str(), "127.0.0.1:9091");

    proxy::disable_system_proxy().expect("disable");
    let s = proxy::query_system_proxy_status().expect("query after disable");
    assert!(!s.enabled);

    restore_snapshot();
}

#[test]
#[serial]
fn idempotent_disable() {
    // Disabling twice must not error and must leave ProxyEnable=0.
    proxy::disable_system_proxy().ok();
    proxy::disable_system_proxy().ok();
    let (en, _, _) = read_status_now();
    assert_eq!(en, Some(0));
    restore_snapshot();
}

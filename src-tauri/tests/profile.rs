// ============================================================================
// Integration tests for the profile / subscription module.
//
// These tests do NOT spin up mihomo. They exercise the on-disk CRUD layer
// (`ProfileStorage`) and the YAML sanitiser (`patch_and_sanitize_yaml`) —
// the two pieces that can be verified without a running sidecar.
//
// Run with:  cargo test -p flexclash --test profile
// ============================================================================

use std::fs;

use flexclash_lib::config::profile as profile_ops;
use flexclash_lib::config::subscription as sub_ops;

mod helper {
    pub fn tmp(label: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("flexclash-test-{label}-{stamp}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}

// ---------------------------------------------------------------------------
// Sanitisation
// ---------------------------------------------------------------------------

const HOSTILE_YAML: &str = r#"
mixed-port: 9999
allow-lan: true
mode: global
log-level: debug
ipv6: true
secret: supersecret
external-controller: 0.0.0.0:9090
external-controller-cors:
  allow-origins:
    - "*"
proxies:
  - { name: "ss1", type: ss, server: 1.2.3.4, port: 8388 }
"#;

#[test]
fn sanitize_overwrites_reserved_fields() {
    let out = profile_ops::patch_and_sanitize_yaml(HOSTILE_YAML)
        .expect("sanitize ok");
    let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
    let m = v.as_mapping().unwrap();

    // Reserved fields must be overwritten regardless of subscription.
    assert_eq!(
        m.get("external-controller").unwrap().as_str(),
        Some("127.0.0.1:9091"),
    );
    assert_eq!(m.get("allow-lan").unwrap().as_bool(), Some(false));
    assert_eq!(m.get("mode").unwrap().as_str(), Some("rule"));
    assert_eq!(m.get("log-level").unwrap().as_str(), Some("info"));
    assert_eq!(m.get("ipv6").unwrap().as_bool(), Some(false));
    assert_eq!(m.get("secret").unwrap().as_str(), Some(""));

    // mixed-port 9999 was a custom value but we ONLY fill in if absent.
    // The hostile yaml HAD a mixed-port, so it should be preserved.
    assert_eq!(m.get("mixed-port").unwrap().as_u64(), Some(9999));
}

#[test]
fn sanitize_injects_cors() {
    let out = profile_ops::patch_and_sanitize_yaml(HOSTILE_YAML).unwrap();
    let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
    let m = v.as_mapping().unwrap();

    let cors = m
        .get("external-controller-cors")
        .expect("cors injected")
        .as_mapping()
        .unwrap();
    let origins = cors
        .get("allow-origins")
        .unwrap()
        .as_sequence()
        .unwrap();
    let has_tauri = origins
        .iter()
        .any(|o| o.as_str() == Some("tauri://localhost"));
    let has_vite = origins
        .iter()
        .any(|o| o.as_str() == Some("http://localhost:5173"));
    assert!(has_tauri, "cors must allow tauri://localhost");
    assert!(has_vite, "cors must allow http://localhost:5173");
}

#[test]
fn sanitize_fills_in_mixed_port_if_absent() {
    let input = "mode: rule\nproxies: []\n";
    let out = profile_ops::patch_and_sanitize_yaml(input).unwrap();
    let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
    assert_eq!(
        v.as_mapping().unwrap().get("mixed-port").unwrap().as_u64(),
        Some(7897),
    );
}

#[test]
fn sanitize_rejects_garbage() {
    let bad = "this: is: not: valid: yaml: [[[";
    assert!(profile_ops::patch_and_sanitize_yaml(bad).is_err());
}

#[test]
fn sanitize_rejects_non_mapping_root() {
    let bad = "- a\n- b\n";
    assert!(profile_ops::patch_and_sanitize_yaml(bad).is_err());
}

// ---------------------------------------------------------------------------
// Storage CRUD
// ---------------------------------------------------------------------------

#[test]
fn profile_storage_save_list_delete_roundtrip() {
    let dir = helper::tmp("crud");
    let storage = profile_ops::ProfileStorage::new(&dir);
    let body = r#"
mixed-port: 7897
proxies:
  - { name: "ss1", type: ss, server: 1.2.3.4, port: 8388 }
  - { name: "ss2", type: ss, server: 1.2.3.5, port: 8388 }
proxy-groups:
  - name: PROXY
    type: select
    proxies: [ss1, ss2]
"#;
    let meta = profile_ops::save_profile(
        &storage,
        None,
        "TestProfile".into(),
        body.into(),
        "https://example.com/sub".into(),
    )
    .expect("save");

    // The yaml on disk must have the reserved fields overwritten.
    let written = fs::read_to_string(&meta.file_path).unwrap();
    assert!(written.contains("external-controller: 127.0.0.1:9091"));
    assert!(written.contains("mode: rule"));

    let list = profile_ops::list_profiles(&storage).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, meta.id);
    assert_eq!(list[0].name, "TestProfile");
    assert_eq!(list[0].node_count, 4); // 2 proxies + 2 group children

    // delete
    profile_ops::delete_profile(&storage, &meta.id).unwrap();
    let list2 = profile_ops::list_profiles(&storage).unwrap();
    assert!(list2.is_empty(), "after delete, list must be empty");
}

#[test]
fn profile_storage_activate_copies_yaml_to_active_path() {
    let dir = helper::tmp("activate");
    let storage = profile_ops::ProfileStorage::new(&dir);

    let body = "mixed-port: 7897\nmode: rule\n";
    let meta = profile_ops::save_profile(
        &storage,
        None,
        "P1".into(),
        body.into(),
        String::new(),
    )
    .unwrap();

    // The active config path should not exist yet.
    assert!(!storage.active_config().exists());

    let activated =
        profile_ops::activate_profile(&storage, &meta.id).unwrap();
    assert_eq!(activated.id, meta.id);
    assert!(storage.active_config().exists());

    let active_yaml = fs::read_to_string(storage.active_config()).unwrap();
    assert!(active_yaml.contains("external-controller: 127.0.0.1:9091"));

    let active_meta = profile_ops::get_active_profile(&storage).unwrap();
    assert!(active_meta.is_some());
    assert_eq!(active_meta.unwrap().id, meta.id);
}

#[test]
fn profile_storage_activate_unknown_id_errors() {
    let dir = helper::tmp("activate-err");
    let storage = profile_ops::ProfileStorage::new(&dir);
    let err = profile_ops::activate_profile(&storage, "ghost-id").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("profile id not found"), "got: {msg}");
}

// ---------------------------------------------------------------------------
// Subscription header parsing
// ---------------------------------------------------------------------------

#[test]
fn subscription_user_info_full_header() {
    let h = "upload=0; download=1234567890; total=5368709120; expire=1893456000";
    let u = sub_ops::parse_user_info_header(Some(h));
    assert_eq!(u.upload, Some(0));
    assert_eq!(u.download, Some(1234567890));
    assert_eq!(u.total, Some(5368709120));
    assert!(u.expire_at.is_some());
}

#[test]
fn subscription_user_info_partial_header() {
    let h = "download=10";
    let u = sub_ops::parse_user_info_header(Some(h));
    assert_eq!(u.download, Some(10));
    assert!(u.upload.is_none());
    assert!(u.total.is_none());
    assert!(u.expire_at.is_none());
}

#[test]
fn subscription_user_info_missing_header() {
    let u = sub_ops::parse_user_info_header(None);
    assert!(u.total.is_none());
    assert!(u.download.is_none());
}

#[test]
fn subscription_user_info_garbage_header() {
    let h = "not a real header at all";
    let u = sub_ops::parse_user_info_header(Some(h));
    assert!(u.total.is_none());
}

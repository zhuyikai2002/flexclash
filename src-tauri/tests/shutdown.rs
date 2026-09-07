// ============================================================================
// integration test: shutdown — ExitFlag semantics.
// ============================================================================

use flexclash_lib::core::shutdown::ExitFlag;

#[test]
fn exit_flag_starts_false() {
    let f = ExitFlag::default();
    assert!(!f.should_exit());
}

#[test]
fn exit_flag_request_latches() {
    let f = ExitFlag::default();
    f.request();
    assert!(f.should_exit());
    // Subsequent should_exit() still returns true; flag is not auto-cleared
    // (Tauri only ever re-arms it before app exit, so latching is correct).
    assert!(f.should_exit());
}

#[test]
fn exit_flag_clone_shares_state() {
    let f = ExitFlag::default();
    let g = f.clone();
    g.request();
    assert!(f.should_exit(), "clones must see the same flag");
}

#[test]
fn exit_flag_request_is_idempotent() {
    let f = ExitFlag::default();
    f.request();
    f.request();
    f.request();
    assert!(f.should_exit());
}

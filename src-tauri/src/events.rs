//! Tauri event channel names emitted to the frontend.
//! Centralised to avoid magic strings scattered across modules.

pub const KERNEL_STATE: &str = "kernel://state";
pub const KERNEL_LOG: &str = "kernel://log";
pub const KERNEL_TERMINATED: &str = "kernel://terminated";
pub const KERNEL_CONFIG_REFRESHED: &str = "kernel://config-refreshed";

pub const PROFILE_LIST_CHANGED: &str = "profile://list-changed";
pub const PROFILE_RELOADED: &str = "profile://reloaded";
pub const SYSTEM_PROXY_CHANGED: &str = "system-proxy://changed";

/// M9: TUN state machine transitions. Payload = `TunStatePayload`.
/// Listened by the frontend to drive the `TunModeToggle` UI and toast
/// notifications on elevation success / failure / cancellation.
pub const TUN_STATE_CHANGED: &str = "tun://state-changed";

/// Phase 8: "Reset Application" finished. Payload = `ResetReport`.
/// The frontend reacts by clearing localStorage, showing a
/// "Reset complete" modal, and (1s later) calling `app.exit(0)`
/// to trigger a clean restart.
pub const APP_RESET_COMPLETED: &str = "app://reset-completed";

/// Phase 9: window-level native drag-and-drop, intercepted in
/// `Builder::on_window_event` and re-broadcast as plain Tauri
/// events so the renderer never has to bind DOM / webview-level
/// listeners.  The WebView2 child window's own drop handlers are
/// noisy and unreliable when the process is elevated.
///
///   - `NATIVE_FILE_DRAG_ENTER`  payload: ()
///   - `NATIVE_FILE_DRAG_LEAVE`  payload: ()
///   - `NATIVE_FILE_DROP`        payload: Vec<String>   (only `.yaml`/`.yml`)
pub const NATIVE_FILE_DRAG_ENTER: &str = "native-file-drag-enter";
pub const NATIVE_FILE_DRAG_LEAVE: &str = "native-file-drag-leave";
pub const NATIVE_FILE_DROP: &str = "native-file-drop";

/// Rust-native speed test: incremental progress. Payload = `DelayBatch`.
///
/// Emitted once per `BATCH_SIZE` completed probes (and once more for the
/// trailing partial batch), so the UI paints latency pills as the pool drains
/// instead of waiting for the slowest node in a several-hundred-node group.
pub const PROXY_DELAY_BATCH: &str = "proxy://delay-batch";

/// Rust-native speed test: terminal event. Payload = `DelayDone`.
///
/// Exactly one per run — including a cancelled run and one that failed to
/// start — so the renderer can always clear its "testing" state. Without a
/// guaranteed terminal event a cancelled run would leave every node spinner
/// stuck forever.
pub const PROXY_DELAY_DONE: &str = "proxy://delay-done";


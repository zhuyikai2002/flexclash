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


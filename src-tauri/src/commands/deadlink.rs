//! Dead-link autopilot controls (v0.6.x Step 5).
//!
//! Two commands, and they exist for one reason: an automatic cleanup is the
//! only thing in this app that *deletes* user traffic without being asked, so
//! the switch has to be reachable from the UI rather than only from an
//! environment variable.
//!
//! Note what is deliberately absent — persistence. The override lives for the
//! session and resets to the environment's answer on the next launch. A
//! persisted "off" is exactly the kind of setting a user forgets they set and
//! then reports as "the app stopped healing itself". `FLEXCLASH_AUTOKILL=off`
//! is the durable switch, and it is owned by whoever launched the process.

use crate::core::deadlink::{self, AutokillState};

/// Read the current state of the autopilot's kill switch.
///
/// The renderer uses `env_locked` to disable its toggle rather than presenting
/// a control that cannot change the outcome.
#[tauri::command]
#[specta::specta]
pub fn get_autokill_state() -> AutokillState {
    deadlink::autokill_state()
}

/// Turn the runtime override on or off; returns the resulting state.
///
/// Always succeeds from the caller's point of view — even a request that the
/// environment vetoes is answered with the honest resulting state, so the UI
/// can re-render instead of having to reconcile an error with a stale value.
#[tauri::command]
#[specta::specta]
pub fn set_autokill(enabled: bool) -> AutokillState {
    deadlink::set_autokill_runtime(enabled)
}

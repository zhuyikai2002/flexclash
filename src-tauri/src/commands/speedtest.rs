// ============================================================================
// commands/speedtest.rs — Tauri surface for the Rust-native speed-test engine.
//
// Thin on purpose: the whole engine lives in `core::speedtest`, and this module
// only translates IPC into a `spawn_run` call. Two commands, both non-blocking:
//
//   speed_test_group   → returns the run id immediately, results stream back
//                        over `proxy://delay-batch` / `proxy://delay-done`
//   cancel_speed_test  → supersedes the run owning a group
//
// Why the command returns immediately instead of awaiting the pool: a
// few-hundred-node run takes tens of seconds, and the caller must be able to
// paint the first results long before the last probe lands. Awaiting here would
// also make the run's lifetime hostage to a single IPC call that a route change
// could abandon.
//
// NOTE ON BINDINGS: these commands are intentionally not registered with
// `tauri-specta` (no `#[specta]`, absent from `collect_commands!` in `lib.rs`).
// They follow the plain-command convention used by `tun` / `profile` / `proxy`,
// with the payload types declared on the renderer side in
// `src/services/speedtest.ts`. The reason is that the interesting types here are
// the *event* payloads, and specta types the command signature rather than the
// events — so registering would buy a typed `speedTestGroup` while the
// `DelayBatch` / `DelayDone` shapes stayed hand-written anyway. Not worth
// forcing a `src/bindings.ts` regeneration (which only happens when the GUI
// starts) for half the benefit.
// ============================================================================

use std::sync::Arc;

use tauri::{AppHandle, Manager};

use crate::core::speedtest::SpeedTestRegistry;
use crate::error::AppError;

type CmdResult<T> = Result<T, AppError>;

/// Resolve the managed registry.
///
/// `Arc<SpeedTestRegistry>` is the managed type so the spawned pool can hold an
/// owned handle instead of borrowing `State<'_, _>` across an `'static` future.
fn registry(app: &AppHandle) -> CmdResult<Arc<SpeedTestRegistry>> {
    app.try_state::<Arc<SpeedTestRegistry>>()
        .map(|s| s.inner().clone())
        .ok_or_else(|| AppError::Other("SpeedTestRegistry not registered".into()))
}

/// Probe every node in `nodes` concurrently and stream the results.
///
/// * `group`       — the owning group; used for cancellation and so the UI can
///                   route a batch to the right card set.
/// * `nodes`       — node names to probe. The caller filters out the
///                   un-probeable types (`Direct`, `Reject`, …) because only it
///                   knows the proxy tree.
/// * `url`         — probe target; defaults to the 204 endpoint.
/// * `timeout_ms`  — Mihomo's per-probe budget; defaults to 5000.
/// * `concurrency` — in-flight cap; defaults to 64, clamped by the engine.
///
/// Returns the run id. Every result carries it, so a client that superseded a
/// run can drop the stragglers — and so can we.
#[tauri::command]
#[specta::specta]
pub async fn speed_test_group(
    app: AppHandle,
    group: String,
    nodes: Vec<String>,
    url: Option<String>,
    timeout_ms: Option<u32>,
    concurrency: Option<u32>,
) -> CmdResult<u32> {
    let registry = registry(&app)?;

    // Empty input is a legitimate no-op, but returning a run id for a run that
    // will never emit anything would leave the caller waiting for a `done`
    // event that cannot come. Fail loudly instead — the UI never sends an
    // empty list, so this is a programming error, not a user-facing path.
    if nodes.is_empty() {
        return Err(AppError::Other(
            "speed_test_group called with no nodes".into(),
        ));
    }

    Ok(crate::core::speedtest::spawn_run(
        app,
        registry,
        group,
        nodes,
        url,
        timeout_ms,
        concurrency,
    ))
}

/// Cancel the run currently owning `group`.
///
/// Idempotent: cancelling an idle group is a successful no-op. The run stops at
/// its next probe completion and still emits a terminal
/// `proxy://delay-done` with `cancelled: true`, so the UI always gets to clear
/// its spinners.
#[tauri::command]
#[specta::specta]
pub fn cancel_speed_test(app: AppHandle, group: String) -> CmdResult<()> {
    registry(&app)?.cancel(&group);
    Ok(())
}

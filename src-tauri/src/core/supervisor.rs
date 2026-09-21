// ============================================================================
// core/supervisor.rs — crash auto-recovery: exponential backoff + circuit breaker.
//
// WHY THIS MODULE EXISTS
// ----------------------
// Until v0.6, the kernel's crash recovery lived in the *renderer*
// (`stores/kernel.ts::autoRestartOnCrash`). That was fatal for a
// background-resident proxy: the frontend window can be closed, reloaded, or
// destroyed, and the moment it is, the kernel's self-healing dies with it. The
// backend must own the restart decision, not the UI.
//
// This task is the single writer of the recovery lifecycle. It claims the
// sidecar's exit channel (`SidecarHandle::take_crash_rx`) and, for every
// abnormal exit, applies:
//
//   * exponential backoff — 1s, 2s, 4s, … capped at 30s, with a ±jitter-free
//     deterministic schedule (the crash sequence itself is the source of
//     entropy);
//   * a circuit breaker — `MAX_CRASHES_IN_WINDOW` crashes within
//     `CRASH_WINDOW` trips the breaker: auto-restarting stops, the kernel is
//     parked in `Crashed`, and a typed `SupervisorEvent::GaveUp` (with the
//     reason) is pushed to the renderer;
//   * a stability window — a kernel that survives a supervised restart for
//     `STABLE_UPTIME` clears the crash history and the backoff exponent.
//
// Zero-panic by construction (matching `core/ingest`): nothing in the loop
// uses `unwrap`/`expect`/indexing, because under `panic = "abort"` a panic in
// this task would take the whole app down — the exact opposite of resilience.
// ============================================================================

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_specta::Event;

use crate::core::sidecar::{self, ExitEvent, KernelState, SidecarHandle};
use crate::error::AppError;
use crate::events;

// -- backoff ----------------------------------------------------------------
const BACKOFF_BASE: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
// -- circuit breaker --------------------------------------------------------
const CRASH_WINDOW: Duration = Duration::from_secs(10);
const MAX_CRASHES_IN_WINDOW: usize = 4;
// -- stability reset --------------------------------------------------------
const STABLE_UPTIME: Duration = Duration::from_secs(10);

/// Typed event pushed whenever the supervisor acts. Registered in
/// `bindings.rs` so the renderer gets a typed listener with no hand mirror.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type, Event)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SupervisorEvent {
    /// A supervised restart is scheduled after a crash.
    Recovering {
        /// 1-based attempt number within the current crash run.
        attempt: u32,
        /// Backoff delay before the relaunch, in milliseconds.
        delay_ms: u32,
        /// Crashes recorded inside the breaker window (incl. this one).
        consecutive_crashes: u32,
    },
    /// The kernel survived `STABLE_UPTIME` after a supervised restart.
    Recovered {
        /// The attempt that ultimately succeeded.
        attempt: u32,
    },
    /// The circuit breaker tripped: no further auto-restarts until the user
    /// intervenes (manual start/stop) or a clean stop resets the breaker.
    GaveUp {
        /// Human-readable reason (why the breaker tripped).
        reason: String,
        /// Crashes recorded inside the breaker window.
        consecutive_crashes: u32,
    },
}

/// Crash-recovery policy. Pure and deterministic apart from the injected
/// `now`, so it is unit-testable without a live kernel.
#[derive(Debug)]
struct RecoveryPolicy {
    window: Duration,
    max_crashes: usize,
    base: Duration,
    max_backoff: Duration,
    crashes: VecDeque<Instant>,
    attempt: u32,
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Trip {
        consecutive: usize,
    },
    Retry {
        delay: Duration,
        attempt: u32,
        consecutive: usize,
    },
}

impl RecoveryPolicy {
    fn new() -> Self {
        Self {
            window: CRASH_WINDOW,
            max_crashes: MAX_CRASHES_IN_WINDOW,
            base: BACKOFF_BASE,
            max_backoff: BACKOFF_MAX,
            crashes: VecDeque::new(),
            attempt: 0,
        }
    }

    /// Record a crash at `now`. Prunes records older than the breaker window,
    /// then decides: trip, or retry after a backoff delay.
    ///
    /// `attempt` is reset for an *isolated* crash (no other crash inside the
    /// window) so a kernel that crashes once an hour never ratchets the
    /// backoff toward its cap — the backoff only grows for a genuine rapid
    /// crash loop, which is exactly what it is meant to punish.
    fn on_crash(&mut self, now: Instant) -> Decision {
        while self
            .crashes
            .front()
            .is_some_and(|t| now.duration_since(*t) >= self.window)
        {
            self.crashes.pop_front();
        }
        let isolated = self.crashes.is_empty();
        self.crashes.push_back(now);
        let consecutive = self.crashes.len();

        if consecutive >= self.max_crashes {
            return Decision::Trip { consecutive };
        }

        if isolated {
            self.attempt = 0;
        }
        self.attempt += 1;
        // 1s, 2s, 4s, 8s, … capped at 30s. `saturating_mul` keeps the shift
        // from overflowing in the (pathological) very-long-crash-run case.
        let multiplier = 1u32 << (self.attempt - 1).min(31);
        let delay = self.base.saturating_mul(multiplier).min(self.max_backoff);
        Decision::Retry {
            delay,
            attempt: self.attempt,
            consecutive,
        }
    }

    /// Clear all recovery bookkeeping (clean stop, manual takeover, or the
    /// kernel proving itself stable).
    fn reset(&mut self) {
        self.crashes.clear();
        self.attempt = 0;
    }
}

/// Push a `[supervisor] …` banner to both stderr and the kernel-log channel.
///
/// The supervisor has no other observable surface in a headless run, and
/// "why did the kernel stop restarting?" is exactly the symptom a tripped
/// breaker produces — so the reason must land somewhere a developer (or a log
/// file) can see even when no window is open. Mirrors `core::ingest::banner`.
fn log<R: Runtime>(app: &AppHandle<R>, line: impl Into<String>) {
    let line = line.into();
    eprintln!("{line}");
    let _ = app.emit(events::KERNEL_LOG, line);
}

/// Spawn the supervisor. Called once from `setup` (after `mount_events`).
pub fn spawn<R: Runtime>(app: &AppHandle<R>, sidecar: SidecarHandle) {
    let app = app.clone();
    // The supervisor claims the receiver once. If it is already gone (this
    // runs exactly once from setup), there is nothing to supervise.
    let Some(mut rx) = sidecar.take_crash_rx() else {
        eprintln!("[supervisor] crash channel already claimed; supervisor not started");
        return;
    };

    tauri::async_runtime::spawn(async move {
        let mut policy = RecoveryPolicy::new();

        loop {
            // Wait for the kernel to exit. The sidecar drain task sends here
            // the instant the child terminates (millisecond-scale).
            let Some(ev) = rx.recv().await else {
                // Channel closed: the sidecar handle was dropped (teardown).
                return;
            };

            match ev {
                ExitEvent::Clean => {
                    policy.reset();
                }
                ExitEvent::Crash { code, signal } => {
                    handle_crash(&app, &sidecar, &mut policy, &mut rx, code, signal).await;
                }
            }
        }
    });
}

/// Snapshot the TUN state, if the manager is registered. Split from
/// `tun_owns_kernel` so the exact value can be logged when the guard fires.
fn current_tun_state<R: Runtime>(app: &AppHandle<R>) -> Option<crate::core::tun::TunState> {
    app.try_state::<crate::core::tun::TunManager>()
        .map(|mgr| mgr.status().state)
}

/// True while TUN owns the kernel: `Enabling` (elevated child launching) or
/// `On` (elevated child serving). In that window the regular sidecar is
/// *expected* to be absent — the elevated TUN kernel holds the controller
/// (9091) and mixed (7897) ports — so a crash-driven restart must be
/// suppressed or the two kernels would fight for the same sockets.
fn tun_owns_kernel<R: Runtime>(app: &AppHandle<R>) -> bool {
    matches!(
        current_tun_state(app),
        Some(crate::core::tun::TunState::Enabling | crate::core::tun::TunState::On)
    )
}

/// Process one crash through the full recovery lifecycle: record it, apply
/// the breaker, back off, relaunch, and watch for stability. Re-enters itself
/// when the kernel crashes again during the stability window or when a
/// synchronous `start()` failure must be counted as a crash.
async fn handle_crash<R: Runtime>(
    app: &AppHandle<R>,
    sidecar: &SidecarHandle,
    policy: &mut RecoveryPolicy,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<ExitEvent>,
    mut code: Option<i32>,
    mut signal: Option<i32>,
) {
    loop {
        // Unconditional diagnostic: every crash must print the TUN state it
        // observed, so a guard that fails to fire is immediately explainable.
        eprintln!(
            "[supervisor] handle_crash: tun_state={:?} sidecar_state={:?} code={code:?} signal={signal:?}",
            current_tun_state(app),
            sidecar.state(),
        );

        // TUN takeover guard (see `tun_owns_kernel`): the sidecar may have
        // been reaped by the elevated child's `pkill -9`, or a previous
        // supervised restart may have raced TUN startup. Never resurrect the
        // regular sidecar while TUN is enabling/on. The exact state is
        // logged so a guard that fails to fire is diagnosable.
        if let Some(state) = current_tun_state(app) {
            if matches!(
                state,
                crate::core::tun::TunState::Enabling | crate::core::tun::TunState::On
            ) {
                sidecar.set_state(KernelState::Stopped);
                policy.reset();
                eprintln!(
                    "[supervisor] TUN guard fired (state={}); backtrace:\n{}",
                    state.as_str(),
                    std::backtrace::Backtrace::force_capture()
                );
                log(
                    app,
                    format!(
                        "[supervisor] sidecar exited while TUN={} — standing down (no restart)",
                        state.as_str()
                    ),
                );
                return;
            }
        }

        let decision = policy.on_crash(Instant::now());

        let (delay, attempt, consecutive) = match decision {
            Decision::Trip { consecutive } => {
                sidecar.set_state(KernelState::Crashed);
                let reason = format!(
                    "kernel crashed {consecutive} times within {}s (last exit code={code:?}, signal={signal:?})",
                    CRASH_WINDOW.as_secs(),
                );
                let _ = SupervisorEvent::GaveUp {
                    reason: reason.clone(),
                    consecutive_crashes: consecutive as u32,
                }
                .emit(app);
                log(
                    app,
                    format!("[supervisor] circuit breaker tripped — {reason}"),
                );
                return;
            }
            Decision::Retry {
                delay,
                attempt,
                consecutive,
            } => (delay, attempt, consecutive),
        };

        // `Recovering` is a transient state: the kernel is down but a
        // supervised relaunch is already scheduled. The renderer treats it as
        // "transitioning", not an error (see the store's hysteresis).
        sidecar.set_state(KernelState::Recovering);
        let _ = SupervisorEvent::Recovering {
            attempt,
            delay_ms: delay.as_millis() as u32,
            consecutive_crashes: consecutive as u32,
        }
        .emit(app);
        log(
            app,
            format!(
                "[supervisor] kernel crashed — restarting in {}s (attempt {attempt}, {consecutive} crash(es) in window)",
                delay.as_secs(),
            ),
        );

        tokio::time::sleep(delay).await;

        // Manual-takeover guard: the user may have started/stopped during the
        // backoff sleep. Stand down in either case.
        match sidecar.state() {
            KernelState::Running
            | KernelState::Starting
            | KernelState::Stopped
            | KernelState::Stopping => {
                policy.reset();
                return;
            }
            KernelState::Recovering | KernelState::Crashed => {}
        }

        // Re-check TUN ownership *after* the backoff: the user may have
        // enabled TUN while we were sleeping, in which case restarting the
        // sidecar would fight the elevated kernel for 9091/7897. The
        // manual-takeover check above does not cover this because `enable`
        // can be mid-flight (state still `Recovering`/`Crashed`).
        if tun_owns_kernel(app) {
            sidecar.set_state(KernelState::Stopped);
            policy.reset();
            log(
                app,
                "[supervisor] TUN took over during backoff — standing down (no restart)",
            );
            return;
        }

        match sidecar::start(app, sidecar.clone()).await {
            Ok(()) => {
                // Watch for stability: either the kernel survives `STABLE_UPTIME`
                // (reset the policy) or it exits again (re-arm the outer loop).
                let stability = tokio::time::sleep(STABLE_UPTIME);
                tokio::pin!(stability);
                tokio::select! {
                    _ = &mut stability => {
                        policy.reset();
                        let _ = SupervisorEvent::Recovered { attempt }.emit(app);
                        log(app, format!("[supervisor] kernel stable after {attempt} restart(s)"));
                        return;
                    }
                    ev = rx.recv() => {
                        match ev {
                            None => return,
                            Some(ExitEvent::Clean) => {
                                policy.reset();
                                return;
                            }
                            Some(ExitEvent::Crash { code: c, signal: s }) => {
                                // Crashed again during the stability window.
                                // Carry the fresh exit through; falling out of
                                // the `select!` re-enters the outer `loop`,
                                // which re-records / re-decides.
                                code = c;
                                signal = s;
                            }
                        }
                    }
                }
            }
            Err(AppError::AlreadyRunning) => {
                // Another task started the kernel between our state check and
                // the spawn. Treat as a manual takeover.
                policy.reset();
                return;
            }
            Err(e) => {
                // Synchronous `start()` failure — no child spawned, so no
                // future `Terminated` will arrive. Count it as a crash and
                // re-loop so the backoff/breaker still advance (with no phantom
                // exit code to misreport).
                log(app, format!("[supervisor] restart failed: {e}"));
                code = None;
                signal = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> RecoveryPolicy {
        RecoveryPolicy::new()
    }

    /// A policy whose breaker threshold is effectively disabled, so the
    /// backoff schedule can be driven past the (default) trip point and the
    /// 30s cap verified in isolation.
    fn policy_with_max(max_crashes: usize) -> RecoveryPolicy {
        RecoveryPolicy {
            window: CRASH_WINDOW,
            max_crashes,
            base: BACKOFF_BASE,
            max_backoff: BACKOFF_MAX,
            crashes: VecDeque::new(),
            attempt: 0,
        }
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let mut p = policy_with_max(100);
        let t0 = Instant::now();
        let mut delays = Vec::new();
        for i in 0..7 {
            // 100ms apart keeps every crash inside the window, so the policy
            // keeps ratcheting `attempt` (no breaker, no isolated reset).
            match p.on_crash(t0 + Duration::from_millis(100 * i as u64)) {
                Decision::Retry { delay, .. } => delays.push(delay),
                Decision::Trip { .. } => panic!("breaker must be disabled here"),
            }
        }
        assert_eq!(delays[0], Duration::from_secs(1));
        assert_eq!(delays[1], Duration::from_secs(2));
        assert_eq!(delays[2], Duration::from_secs(4));
        assert_eq!(delays[3], Duration::from_secs(8));
        assert_eq!(delays[4], Duration::from_secs(16));
        // 32s and 64s both saturate at the 30s cap.
        assert_eq!(delays[5], Duration::from_secs(30));
        assert_eq!(delays[6], Duration::from_secs(30));
    }

    #[test]
    fn breaker_trips_on_nth_crash_in_window() {
        let mut p = policy();
        let t0 = Instant::now();
        assert!(matches!(p.on_crash(t0), Decision::Retry { .. }));
        assert!(matches!(
            p.on_crash(t0 + Duration::from_millis(100)),
            Decision::Retry { .. }
        ));
        assert!(matches!(
            p.on_crash(t0 + Duration::from_millis(200)),
            Decision::Retry { .. }
        ));
        assert_eq!(
            p.on_crash(t0 + Duration::from_millis(300)),
            Decision::Trip { consecutive: 4 }
        );
    }

    #[test]
    fn slow_crashes_never_trip_and_reset_backoff() {
        let mut p = policy();
        let t0 = Instant::now();
        // Crashes 11s apart are outside the 10s window: each is isolated, so
        // the breaker never accumulates and the backoff stays at attempt 1.
        for i in 0..6 {
            let now = t0 + Duration::from_secs(11 * i as u64);
            assert_eq!(
                p.on_crash(now),
                Decision::Retry {
                    delay: Duration::from_secs(1),
                    attempt: 1,
                    consecutive: 1,
                },
                "crash #{i} should be isolated"
            );
        }
    }

    #[test]
    fn reset_clears_window_and_backoff() {
        let mut p = policy();
        let t0 = Instant::now();
        assert!(matches!(p.on_crash(t0), Decision::Retry { .. }));
        assert!(matches!(
            p.on_crash(t0 + Duration::from_millis(10)),
            Decision::Retry { .. }
        ));
        p.reset();
        assert_eq!(
            p.on_crash(t0 + Duration::from_millis(20)),
            Decision::Retry {
                delay: Duration::from_secs(1),
                attempt: 1,
                consecutive: 1,
            }
        );
    }
}

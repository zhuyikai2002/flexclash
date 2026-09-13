// ============================================================================
// core/speedtest.rs — Rust-native concurrent node latency probing.
//
// WHY THIS LIVES IN RUST
// ----------------------
// Node latency is measured by asking Mihomo to test the node:
//
//     GET /proxies/{name}/delay?url=…&timeout=…
//
// That is the only correct primitive — the probe has to egress *through the
// node under test*, so issuing `http://www.gstatic.com/generate_204` directly
// from here would measure this machine's RTT to gstatic and report it as the
// node's latency. The Rust side therefore does not replace Mihomo's prober; it
// replaces the frontend's *orchestration* of it (the old worker pool of 6 in
// `stores/proxies.ts`, which paid an IPC round-trip per node and funnelled
// hundreds of concurrent `invoke` calls through the webview).
//
// So: same probe target, but the fan-out, the timeout budget, the result
// classification and the delivery back to the UI all move below the IPC line.
//
// CONCURRENCY / RESOURCE SAFETY
// -----------------------------
//   * ONE `reqwest::Client` is built per run and shared by every probe. The
//     pool in `commands/mihomo.rs` builds a fresh client per request, which is
//     fine for a handful of calls but would churn sockets at 64 in flight.
//   * Fanned out with `futures_util::stream::…buffer_unordered(N)` so at most N
//     probes are ever in flight — N defaults to 64 and is clamped to
//     `MAX_CONCURRENCY`, so a 1000-node subscription cannot open 1000 sockets.
//     The queue is lazy: an un-polled node has no client, no socket, no task.
//   * Every node gets a definite terminal status. A batch is always emitted for
//     it, including on cancellation and on internal failure, so the UI can
//     never be left with a spinner that has no owner.
//
// TIMEOUT BUDGET — read before "fixing" the grace constant
// -------------------------------------------------------
// `timeout_ms` (default 5000) is Mihomo's *probe* budget and is passed through
// verbatim as `?timeout=`. The `reqwest` timeout is that budget PLUS a small
// grace (`CLIENT_TIMEOUT_GRACE_MS`). Setting the HTTP timeout equal to the
// probe budget would be a bug: both deadlines start at roughly the same
// instant, so a probe that legitimately finishes at 4.9 s can have its reply
// discarded by a client that gave up at 5.0 s, and the node would be reported
// as a timeout even though Mihomo measured it. The grace makes the HTTP
// timeout a true backstop ("never hang") while Mihomo's verdict — which is the
// accurate one — is what reaches the UI. The whole run is still hard-bounded:
// client timeout ≈ 6.5 s, so nothing can hang indefinitely.
//
// CANCELLATION
// ------------
// Each run registers itself in `SpeedTestRegistry` under its group name and
// holds a monotonic `run_id`. Starting a new run for the same group, or an
// explicit `cancel`, replaces/removes that entry, which makes the older run's
// `run_id` stale. The pool checks staleness as results arrive and breaks out;
// dropping the stream drops the in-flight futures, so the sockets go with it.
// Results from a stale run are never emitted.
// ============================================================================

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use futures_util::stream::StreamExt;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Emitter};

use crate::config::profile::RESERVED_CONTROLLER;
use crate::core::urlenc::percent_encode;
use crate::events;

/// Probe target. A 204-returning endpoint is the clash/mihomo convention:
/// tiny, cacheable, and reachable from essentially every exit node.
pub const DEFAULT_TEST_URL: &str = "http://www.gstatic.com/generate_204";

/// Mihomo's own probe budget, in ms. Passed through as `?timeout=`.
pub const DEFAULT_TIMEOUT_MS: u32 = 5_000;

/// In-flight probe cap. Large enough that a few hundred nodes finish quickly,
/// small enough that the socket table stays boring.
pub const DEFAULT_CONCURRENCY: u32 = 64;

/// Hard ceiling on the user-supplied concurrency. Prevents a bad value from
/// turning "speed test" into a local fd exhaustion exercise.
pub const MAX_CONCURRENCY: usize = 256;

/// Results per emitted batch. Small enough that the first pills paint almost
/// immediately, large enough that a 500-node run does not emit 500 events.
pub const BATCH_SIZE: usize = 8;

/// Added to the probe budget to get the HTTP client's hard timeout. See the
/// module header — do not collapse this to zero.
const CLIENT_TIMEOUT_GRACE_MS: u64 = 1_500;

/// Upper bound for the TCP/TLS handshake to the *local* controller. This is a
/// loopback connect; anything slower means the kernel is wedged.
const CONNECT_TIMEOUT_MS: u64 = 3_000;

// ---------------------------------------------------------------------------
// Wire types (serialised camelCase → consumed by `src/bindings.ts`)
// ---------------------------------------------------------------------------

/// Terminal outcome of one probe. Mirrors the frontend `DelayStatus` union
/// (minus `idle`/`testing`, which are client-side-only states).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ProbeStatus {
    /// Mihomo measured an RTT.
    Ok,
    /// Probe budget exhausted (Mihomo 504/408, or a zero/absent delay).
    Timeout,
    /// The controller could not be reached at all — usually "kernel not running".
    Unreachable,
    /// Anything else: unknown node, malformed body, HTTP 4xx/5xx.
    Error,
}

/// One node's probe result.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NodeProbe {
    pub name: String,
    pub status: ProbeStatus,
    /// RTT in ms; `Some` iff `status == Ok`.
    pub delay_ms: Option<u32>,
    /// Human-readable detail for non-`Ok` results (shown in tooltips).
    pub message: Option<String>,
}

impl NodeProbe {
    fn ok(name: &str, delay_ms: u32) -> Self {
        Self {
            name: name.to_string(),
            status: ProbeStatus::Ok,
            delay_ms: Some(delay_ms),
            message: None,
        }
    }

    fn timeout(name: &str, why: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            status: ProbeStatus::Timeout,
            delay_ms: None,
            message: Some(why.into()),
        }
    }

    fn unreachable(name: &str, why: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            status: ProbeStatus::Unreachable,
            delay_ms: None,
            message: Some(why.into()),
        }
    }

    fn error(name: &str, why: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            status: ProbeStatus::Error,
            delay_ms: None,
            message: Some(why.into()),
        }
    }

    fn is_ok(&self) -> bool {
        self.status == ProbeStatus::Ok
    }
}

/// Incremental progress push — one event per `BATCH_SIZE` results.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DelayBatch {
    pub run_id: u32,
    pub group: String,
    pub results: Vec<NodeProbe>,
    /// Nodes probed so far (including this batch).
    pub done: u32,
    pub total: u32,
}

/// Terminal push — exactly one per run, always emitted, including on
/// cancellation, so the UI can always clear its "testing" affordances.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DelayDone {
    pub run_id: u32,
    pub group: String,
    pub total: u32,
    /// How many were actually probed. `< total` iff `cancelled`.
    pub done: u32,
    pub ok: u32,
    pub failed: u32,
    pub cancelled: bool,
}

// ---------------------------------------------------------------------------
// Run registry (cancellation / staleness)
// ---------------------------------------------------------------------------

/// Tracks which run is authoritative for each group.
///
/// The `Mutex` is `std::sync::Mutex` on purpose: every critical section is a
/// couple of hash-map ops with no `.await` inside, so the async mutex would be
/// pure overhead. A poisoned lock is treated as "no active run" rather than
/// unwrapped — a panic here would take the whole app down for a cosmetic
/// failure, which the acceptance criteria explicitly forbid.
#[derive(Default)]
pub struct SpeedTestRegistry {
    next_id: AtomicU32,
    active: Mutex<HashMap<String, u32>>,
}

impl SpeedTestRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Claim `group` for a new run and return its id. Any previous run for the
    /// same group is implicitly superseded (its id stops matching).
    fn begin(&self, group: &str) -> u32 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        if let Ok(mut m) = self.active.lock() {
            m.insert(group.to_string(), id);
        }
        id
    }

    /// True iff `id` is still the authoritative run for `group`.
    fn is_active(&self, group: &str, id: u32) -> bool {
        match self.active.lock() {
            Ok(m) => m.get(group).copied() == Some(id),
            // Poisoned: report "not active" so the run stops instead of
            // spinning forever on a lock we can never take.
            Err(_) => false,
        }
    }

    /// Release the claim, but only if `id` is still the current one — a newer
    /// run must not have its registration yanked by an older one finishing.
    fn finish(&self, group: &str, id: u32) {
        if let Ok(mut m) = self.active.lock() {
            if m.get(group).copied() == Some(id) {
                m.remove(group);
            }
        }
    }

    /// Cancel the run currently owning `group`, if any.
    pub fn cancel(&self, group: &str) {
        if let Ok(mut m) = self.active.lock() {
            m.remove(group);
        }
    }

    /// Cancel every in-flight run (used on kernel stop / app teardown).
    pub fn cancel_all(&self) {
        if let Ok(mut m) = self.active.lock() {
            m.clear();
        }
    }
}

// ---------------------------------------------------------------------------
// Probe
// ---------------------------------------------------------------------------

fn controller_base() -> String {
    format!("http://{RESERVED_CONTROLLER}")
}

/// Classify a Mihomo `/proxies/{name}/delay` reply.
///
/// Split out from the transport so it can be unit-tested without a live
/// kernel. Mihomo's contract, as observed:
///   * 200 `{"delay":123}`            → measured
///   * 200 `{"delay":0}`              → unusable, treat as timeout
///   * 408 / 504, or a `message`     → probe budget exhausted
///   * 4xx (e.g. 404 unknown proxy)  → caller error
fn classify(name: &str, status: u16, body: &str) -> NodeProbe {
    let delay = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("delay").and_then(|d| d.as_u64()));

    if (200..300).contains(&status) {
        return match delay {
            // Clamp defensively; a real RTT will never approach u32::MAX,
            // but `as` from u64 would silently truncate if it did.
            Some(ms) if ms > 0 => NodeProbe::ok(name, ms.min(u32::MAX as u64) as u32),
            Some(_) => NodeProbe::timeout(name, "delay reported as 0"),
            None => NodeProbe::error(name, format!("unexpected body: {}", truncate(body, 120))),
        };
    }

    // Mihomo answers 504 (and occasionally 408) when the probe budget runs out.
    if status == 408 || status == 504 {
        return NodeProbe::timeout(name, format!("probe budget exhausted (HTTP {status})"));
    }

    NodeProbe::error(name, format!("HTTP {status}: {}", truncate(body, 120)))
}

fn truncate(s: &str, max: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= max {
        return t.to_string();
    }
    let head: String = t.chars().take(max).collect();
    format!("{head}…")
}

/// Probe one node through the kernel's controller endpoint.
///
/// Never returns `Err`: every failure mode is folded into a `NodeProbe` so the
/// caller can emit a batch unconditionally. That is what keeps "a node always
/// reaches a terminal state" true even when the kernel dies mid-run.
async fn probe_one(client: &reqwest::Client, name: &str, url: &str, timeout_ms: u32) -> NodeProbe {
    let endpoint = format!(
        "{}/proxies/{}/delay?url={}&timeout={timeout_ms}",
        controller_base(),
        percent_encode(name),
        percent_encode(url),
    );

    match client.get(&endpoint).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            // A body read failure is not fatal — classify on the status alone
            // and let the missing body surface as "unexpected body".
            let body = resp.text().await.unwrap_or_default();
            classify(name, status, &body)
        }
        Err(e) if e.is_timeout() => NodeProbe::timeout(name, "probe exceeded client budget"),
        Err(e) if e.is_connect() => {
            // Loopback refused → the kernel is not listening.
            NodeProbe::unreachable(name, format!("controller unreachable: {e}"))
        }
        Err(e) => NodeProbe::error(name, e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Pool
// ---------------------------------------------------------------------------

/// Everything the pool needs, bundled so the spawned future's signature stays
/// readable.
struct RunSpec {
    run_id: u32,
    group: String,
    nodes: Vec<String>,
    url: String,
    timeout_ms: u32,
    concurrency: usize,
}

/// Start a run, returning its id immediately.
///
/// The heavy lifting happens on Tauri's async runtime; results stream back as
/// `proxy://delay-batch` and terminate with a single `proxy://delay-done`.
/// Returning at once is deliberate — the caller must never be made to wait for
/// a few hundred probes before it can render anything.
pub fn spawn_run(
    app: AppHandle,
    registry: std::sync::Arc<SpeedTestRegistry>,
    group: String,
    nodes: Vec<String>,
    url: Option<String>,
    timeout_ms: Option<u32>,
    concurrency: Option<u32>,
) -> u32 {
    let spec = RunSpec {
        run_id: 0, // filled from `begin` below
        group,
        nodes,
        url: url.unwrap_or_else(|| DEFAULT_TEST_URL.to_string()),
        // `.max(1)` so a stray 0 can never produce a zero-length deadline that
        // fails every probe instantly.
        timeout_ms: timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).max(1),
        concurrency: (concurrency.unwrap_or(DEFAULT_CONCURRENCY) as usize).clamp(1, MAX_CONCURRENCY),
    };

    let run_id = registry.begin(&spec.group);
    let mut spec = spec;
    spec.run_id = run_id;

    tauri::async_runtime::spawn(run_pool(app, registry, spec));
    run_id
}

async fn run_pool(app: AppHandle, registry: std::sync::Arc<SpeedTestRegistry>, spec: RunSpec) {
    let RunSpec {
        run_id,
        group,
        nodes,
        url,
        timeout_ms,
        concurrency,
    } = spec;

    let total = nodes.len() as u32;

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(
            u64::from(timeout_ms) + CLIENT_TIMEOUT_GRACE_MS,
        ))
        .connect_timeout(Duration::from_millis(CONNECT_TIMEOUT_MS))
        // Keep the idle pool sized to the fan-out so keep-alive actually pays
        // off instead of being torn down between batches.
        .pool_max_idle_per_host(concurrency)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            // Cannot probe anything. Still emit a definite status per node and
            // a terminal event, so no card is left spinning.
            let reason = format!("could not build http client: {e}");
            let results: Vec<NodeProbe> =
                nodes.iter().map(|n| NodeProbe::error(n, reason.clone())).collect();
            emit_batch(&app, run_id, &group, results, total, total);
            emit_done(
                &app,
                DelayDone {
                    run_id,
                    group: group.clone(),
                    total,
                    done: total,
                    ok: 0,
                    failed: total,
                    cancelled: false,
                },
            );
            registry.finish(&group, run_id);
            return;
        }
    };

    // Lazily map each name to a future; `buffer_unordered` polls at most
    // `concurrency` of them, so the rest cost nothing until their turn.
    let probes = nodes.iter().cloned().map(|name| {
        let client = client.clone();
        let url = url.clone();
        async move { probe_one(&client, &name, &url, timeout_ms).await }
    });
    let mut stream = futures_util::stream::iter(probes).buffer_unordered(concurrency);

    let mut batch: Vec<NodeProbe> = Vec::with_capacity(BATCH_SIZE);
    let mut done: u32 = 0;
    let mut ok: u32 = 0;
    let mut failed: u32 = 0;
    let mut cancelled = false;

    while let Some(probe) = stream.next().await {
        // Checked per completion, not per node: cheaper, and a superseded run
        // stops within one probe budget at the very worst.
        if !registry.is_active(&group, run_id) {
            cancelled = true;
            break;
        }

        done += 1;
        if probe.is_ok() {
            ok += 1;
        } else {
            failed += 1;
        }
        batch.push(probe);

        if batch.len() >= BATCH_SIZE {
            emit_batch(&app, run_id, &group, std::mem::take(&mut batch), done, total);
            batch = Vec::with_capacity(BATCH_SIZE);
        }
    }

    // `stream` is dropped here either way: on the normal path it is exhausted,
    // on the cancelled path this is what tears down the in-flight sockets.

    if !batch.is_empty() {
        emit_batch(&app, run_id, &group, batch, done, total);
    }

    registry.finish(&group, run_id);
    emit_done(
        &app,
        DelayDone {
            run_id,
            group,
            total,
            done,
            ok,
            failed,
            cancelled,
        },
    );
}

/// Emit one progress batch. Emission failures are swallowed: a webview that
/// went away must not abort a run, and there is nothing useful to do about it.
fn emit_batch(app: &AppHandle, run_id: u32, group: &str, results: Vec<NodeProbe>, done: u32, total: u32) {
    if results.is_empty() {
        return;
    }
    let _ = app.emit(
        events::PROXY_DELAY_BATCH,
        DelayBatch {
            run_id,
            group: group.to_string(),
            results,
            done,
            total,
        },
    );
}

/// Emit the terminal event. Exactly once per run.
fn emit_done(app: &AppHandle, payload: DelayDone) {
    let _ = app.emit(events::PROXY_DELAY_DONE, payload);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- classify ---------------------------------------------------------

    #[test]
    fn classify_ok_delay() {
        let p = classify("n1", 200, r#"{"delay":187}"#);
        assert_eq!(p.status, ProbeStatus::Ok);
        assert_eq!(p.delay_ms, Some(187));
        assert_eq!(p.name, "n1");
        assert!(p.message.is_none());
    }

    #[test]
    fn classify_zero_delay_is_timeout() {
        // Mihomo reports an unusable node as a zero delay, not an error status.
        let p = classify("n1", 200, r#"{"delay":0}"#);
        assert_eq!(p.status, ProbeStatus::Timeout);
        assert_eq!(p.delay_ms, None);
        assert!(p.message.is_some());
    }

    #[test]
    fn classify_missing_delay_is_error() {
        let p = classify("n1", 200, r#"{"message":"weird"}"#);
        assert_eq!(p.status, ProbeStatus::Error);
        assert!(p.message.unwrap().contains("unexpected body"));
    }

    #[test]
    fn classify_504_is_timeout() {
        let p = classify("n1", 504, r#"{"message":"An error occurred in the delay test"}"#);
        assert_eq!(p.status, ProbeStatus::Timeout);
    }

    #[test]
    fn classify_408_is_timeout() {
        let p = classify("n1", 408, "");
        assert_eq!(p.status, ProbeStatus::Timeout);
    }

    #[test]
    fn classify_404_is_error() {
        let p = classify("n1", 404, r#"{"message":"Proxy not found"}"#);
        assert_eq!(p.status, ProbeStatus::Error);
        assert!(p.message.unwrap().contains("404"));
    }

    #[test]
    fn classify_huge_delay_is_clamped_not_truncated() {
        // u64 -> u32 must saturate, not wrap to a tiny bogus RTT.
        let p = classify("n1", 200, r#"{"delay":4294967296}"#);
        assert_eq!(p.status, ProbeStatus::Ok);
        assert_eq!(p.delay_ms, Some(u32::MAX));
    }

    #[test]
    fn classify_does_not_truncate_short_body() {
        let p = classify("n1", 500, "  boom  ");
        assert_eq!(p.message.unwrap(), "HTTP 500: boom");
    }

    #[test]
    fn truncate_is_char_safe_for_cjk() {
        // 10 CJK chars, limit 4 — must not split a multi-byte char.
        let out = truncate("一二三四五六七八九十", 4);
        assert_eq!(out, "一二三四…");
    }

    // --- registry ---------------------------------------------------------

    #[test]
    fn registry_supersedes_previous_run() {
        let r = SpeedTestRegistry::new();
        let a = r.begin("PROXY");
        assert!(r.is_active("PROXY", a));
        let b = r.begin("PROXY");
        assert!(!r.is_active("PROXY", a), "old run must be stale");
        assert!(r.is_active("PROXY", b));
    }

    #[test]
    fn registry_is_per_group() {
        let r = SpeedTestRegistry::new();
        let a = r.begin("A");
        let b = r.begin("B");
        assert!(r.is_active("A", a));
        assert!(r.is_active("B", b));
        // Cancelling one group must not disturb the other.
        r.cancel("A");
        assert!(!r.is_active("A", a));
        assert!(r.is_active("B", b));
    }

    #[test]
    fn registry_cancel_makes_stale() {
        let r = SpeedTestRegistry::new();
        let a = r.begin("PROXY");
        r.cancel("PROXY");
        assert!(!r.is_active("PROXY", a));
    }

    #[test]
    fn registry_finish_only_clears_own_id() {
        let r = SpeedTestRegistry::new();
        let a = r.begin("PROXY");
        let b = r.begin("PROXY");
        // A stale run finishing must not evict the live one.
        r.finish("PROXY", a);
        assert!(r.is_active("PROXY", b));
        r.finish("PROXY", b);
        assert!(!r.is_active("PROXY", b));
    }

    #[test]
    fn registry_cancel_all_clears_everything() {
        let r = SpeedTestRegistry::new();
        let a = r.begin("A");
        let b = r.begin("B");
        r.cancel_all();
        assert!(!r.is_active("A", a));
        assert!(!r.is_active("B", b));
    }

    #[test]
    fn registry_ids_are_distinct() {
        let r = SpeedTestRegistry::new();
        let ids: Vec<u32> = (0..64).map(|i| r.begin(&format!("g{i}"))).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }

    // --- NodeProbe invariants --------------------------------------------

    #[test]
    fn only_ok_carries_a_delay() {
        assert_eq!(NodeProbe::ok("n", 42).delay_ms, Some(42));
        assert_eq!(NodeProbe::timeout("n", "x").delay_ms, None);
        assert_eq!(NodeProbe::unreachable("n", "x").delay_ms, None);
        assert_eq!(NodeProbe::error("n", "x").delay_ms, None);
        assert!(NodeProbe::ok("n", 1).is_ok());
        assert!(!NodeProbe::error("n", "x").is_ok());
    }

    #[test]
    fn probe_url_shape_is_stable() {
        // Mirrors the format string in `probe_one`; guards against a refactor
        // silently dropping the timeout or the url query.
        let endpoint = format!(
            "{}/proxies/{}/delay?url={}&timeout={}",
            controller_base(),
            percent_encode("香港 01"),
            percent_encode(DEFAULT_TEST_URL),
            DEFAULT_TIMEOUT_MS,
        );
        assert!(endpoint.starts_with("http://127.0.0.1:9091/proxies/"));
        assert!(endpoint.contains("/delay?url=http%3A%2F%2Fwww.gstatic.com%2Fgenerate_204"));
        assert!(endpoint.ends_with("&timeout=5000"));
        assert!(!endpoint.contains(' '), "node name must be encoded");
    }

    #[test]
    fn concurrency_clamp_bounds_a_bad_value() {
        let clamp = |v: u32| (v as usize).clamp(1, MAX_CONCURRENCY);
        assert_eq!(clamp(0), 1);
        assert_eq!(clamp(64), 64);
        assert_eq!(clamp(u32::MAX), MAX_CONCURRENCY);
    }
}

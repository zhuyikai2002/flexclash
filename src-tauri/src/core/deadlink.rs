// ============================================================================
// core/deadlink.rs — autonomous dead-link cleanup (v0.6.x Step 4).
//
// WHAT THIS IS
// ------------
// Step 3 taught Rust to *read* the kernel's failures (`core::log_parse`). Step 2
// taught it to *act* on the connection pool (`core::connections`). This module
// is the bridge: when a node keeps failing, Rust reaps the connections stuck on
// it so the kernel is forced to re-route — with no user in the loop.
//
// THE HIDDEN GAP: LOGS DO NOT NAME THE NODE
// -----------------------------------------
// `LogAnomaly` carries a *destination*, never an egress node. Only
// `ProxySwitch{to}` names a node, and that variant is a normal event, not a
// failure. So "which node is failing?" cannot be answered from the log line
// alone — it has to be JOINed against `GET /connections`, the single place in
// the whole system where a destination sits next to its `chains` (the egress
// path).
//
// That join is the reason for the module's first rule:
//
//   **attribution is best-effort, and a failed attribution means NO ACTION.**
//
// We never guess. A destination that matches no live connection scores nothing
// at all, because a wrong guess here is not a missed cleanup — it is killing
// connections that were working fine.
//
// HONEST GUILT BY ASSOCIATION
// ---------------------------
// One destination can legitimately appear on connections egressing through
// several different nodes (a host contacted from two groups, a shared CDN). We
// deliberately do NOT pick a "most likely" one: every distinct egress node gets
// a hit. The three storm guards below are what make that affordable.
//
// FOUR STORM GUARDS
// -----------------
//   1. Idle costs nothing — an empty queue skips the evaluation entirely, so
//      no `GET /connections` is ever issued when there is nothing to attribute.
//   2. One snapshot per evaluation — a DNS server that emits hundreds of
//      failures a second still costs one GET (at most once a second), not one
//      per line.
//   3. Per-node cooldown — after a cleanup, that node is frozen for
//      `COOLDOWN` regardless of how much more evidence arrives.
//   4. Global budget — at most `GLOBAL_BUDGET` cleanups across *all* nodes in
//      any `BUDGET_WINDOW`, so a systemic outage (the whole subscription is
//      dead) cannot fire one cleanup per node in the same second.
//
// On top of those, `core::connections::kill_by_filter` already caps a single
// call at 100 DELETEs spaced 50 ms apart. A full sweep therefore takes ~5 s of
// wall clock, which is why cleanups are awaited **serially** inside the
// evaluation loop: two concurrent cleanups would each hold their own snapshot
// and could delete the same connection twice.
//
// SAFETY CONTRACT
// ---------------
// Release builds set `panic = "abort"`, so nothing here unwraps, indexes, or
// assumes a monotonic clock (see `elapsed_since`). The policy is pure and
// unit-tested without a kernel; only `run`/`evaluate`/`cleanup` touch I/O.
// ============================================================================

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Emitter, Runtime};
use tauri_specta::Event;
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

use crate::core::connections::{self, ConnectionFilter, ConnectionInfo, ConnectionsSnapshot};
use crate::core::kernel_events::LogAnomaly;
use crate::events;

// -- sliding window ---------------------------------------------------------
/// How far back a node's failures still count toward the threshold.
const WINDOW: Duration = Duration::from_secs(10);
/// Failures inside `WINDOW` needed to trip a cleanup.
const THRESHOLD: usize = 3;
// -- storm guards -----------------------------------------------------------
/// Per-node freeze after a cleanup.
const COOLDOWN: Duration = from_secs(60);
/// Cleanups allowed across *all* nodes per `BUDGET_WINDOW`.
const GLOBAL_BUDGET: usize = 3;
const BUDGET_WINDOW: Duration = from_secs(60);
// -- evaluation -------------------------------------------------------------
/// How often the autopilot drains the queue and re-evaluates.
const EVAL_INTERVAL: Duration = from_secs(1);
// -- bounds -----------------------------------------------------------------
/// Queue capacity. Overflow drops the newest (see `AnomalySink::send`).
const SINK_CAP: usize = 256;
/// Hard cap on tracked nodes, so a pathological config cannot grow the map.
const NODE_CAP: usize = 256;

/// `Duration::from_secs` is not `const`-stable on every toolchain this crate
/// builds with, and these are compile-time constants — a tiny `const fn` keeps
/// the call sites readable without a `lazy_static`.
const fn from_secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

/// Escape hatch: `FLEXCLASH_AUTOKILL=off` (also `0` / `false` / `no`) makes the
/// autopilot observe and report but never delete anything.
///
/// Automatic cleanup is a heuristic built on text matching, so it must be
/// possible to switch off without a rebuild — a false positive here shows up to
/// the user as "my connections keep dropping for no reason".
fn autokill_enabled() -> bool {
    std::env::var("FLEXCLASH_AUTOKILL")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .map(|v| !matches!(v.as_str(), "0" | "off" | "false" | "no"))
        .unwrap_or(true)
}

/// The outcome of a dead-link cleanup, published as a typed event.
///
/// Automatic deletion is otherwise invisible, and "why did my connections
/// drop?" is not answerable after the fact. Every cleanup — including the ones
/// that killed nothing and the ones suppressed by the escape hatch — produces
/// exactly one of these.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct DeadLinkCleanup {
    /// The egress node whose dead links were closed.
    pub node: String,
    /// How many DELETEs the controller accepted.
    pub killed: usize,
    /// How many DELETEs failed (connection already gone, controller hiccup…).
    pub failed: usize,
    /// Why the cleanup ran — or why it did not.
    pub reason: String,
    /// Receipt time, as Unix epoch milliseconds.
    pub at_ms: i64,
}

// ===========================================================================
// Policy — pure, deterministic, unit-tested without a kernel
// ===========================================================================

/// What the autopilot should do after one more failure on `node`.
#[derive(Debug, PartialEq, Eq)]
pub enum DeadLinkDecision {
    /// Not enough evidence yet (or the global budget is spent).
    Wait,
    /// The node is inside its post-cleanup cooldown; the evidence is dropped.
    CoolingDown,
    /// Reap this node's connections.
    Fire { node: String },
}

#[derive(Debug, Default)]
struct NodeWindow {
    /// Failure timestamps inside `WINDOW`, oldest first.
    hits: VecDeque<Instant>,
    /// Set by `note_cleanup`; the node is inert until this passes.
    cooldown_until: Option<Instant>,
}

/// Per-node sliding windows plus the global cleanup budget.
///
/// Pure apart from the injected `now`, exactly like
/// `core::supervisor::RecoveryPolicy`, so every rule below is testable without
/// a live kernel.
#[derive(Debug)]
pub struct DeadLinkPolicy {
    window: Duration,
    threshold: usize,
    cooldown: Duration,
    budget: usize,
    budget_window: Duration,
    nodes: HashMap<String, NodeWindow>,
    /// Timestamps of recent cleanups, for the global budget.
    recent_cleanups: VecDeque<Instant>,
}

impl DeadLinkPolicy {
    fn new() -> Self {
        Self {
            window: WINDOW,
            threshold: THRESHOLD,
            cooldown: COOLDOWN,
            budget: GLOBAL_BUDGET,
            budget_window: BUDGET_WINDOW,
            nodes: HashMap::new(),
            recent_cleanups: VecDeque::new(),
        }
    }

    /// Record one failure on `node` at `now` and decide what to do.
    ///
    /// Evidence arriving during a cooldown is discarded rather than banked:
    /// re-arming the window from it would let a permanently broken node fire a
    /// cleanup every `COOLDOWN` seconds forever.
    pub fn record(&mut self, node: &str, now: Instant) -> DeadLinkDecision {
        let entry = self.nodes.entry(node.to_string()).or_default();

        if let Some(until) = entry.cooldown_until {
            if now < until {
                return DeadLinkDecision::CoolingDown;
            }
            entry.cooldown_until = None;
        }

        while entry
            .hits
            .front()
            .is_some_and(|t| elapsed_since(now, *t) >= self.window)
        {
            entry.hits.pop_front();
        }
        entry.hits.push_back(now);

        if entry.hits.len() < self.threshold {
            return DeadLinkDecision::Wait;
        }

        self.prune_budget(now);
        if self.recent_cleanups.len() >= self.budget {
            // The budget is the last line of defence against a systemic
            // outage firing a cleanup per node in the same second. The hits
            // stay, so the node can still fire once the window rolls.
            return DeadLinkDecision::Wait;
        }

        DeadLinkDecision::Fire {
            node: node.to_string(),
        }
    }

    /// Book a cleanup for `node`: clear its evidence and freeze it.
    ///
    /// Called *before* the DELETEs are issued, so a slow cleanup cannot be
    /// re-entered and a failing one still backs off instead of hammering.
    pub fn note_cleanup(&mut self, node: &str, now: Instant) {
        // `entry`, not `get_mut`: the node must exist afterwards even if
        // nothing was ever recorded for it, or the cooldown would be silently
        // dropped and the node would be eligible again immediately.
        let entry = self.nodes.entry(node.to_string()).or_default();
        entry.hits.clear();
        entry.cooldown_until = Some(now + self.cooldown);
        self.recent_cleanups.push_back(now);
    }

    /// Drop bookkeeping that can no longer affect a decision.
    pub fn prune(&mut self, now: Instant) {
        self.prune_budget(now);
        self.nodes.retain(|_, entry| {
            if entry.cooldown_until.is_some_and(|until| now < until) {
                return true;
            }
            while entry
                .hits
                .front()
                .is_some_and(|t| elapsed_since(now, *t) >= self.window)
            {
                entry.hits.pop_front();
            }
            !entry.hits.is_empty()
        });
        self.enforce_node_cap();
    }

    fn prune_budget(&mut self, now: Instant) {
        while self
            .recent_cleanups
            .front()
            .is_some_and(|t| elapsed_since(now, *t) >= self.budget_window)
        {
            self.recent_cleanups.pop_front();
        }
    }

    /// Evict the least-recently-active nodes when over `NODE_CAP`.
    ///
    /// Losing counters is safe — it can only delay a cleanup, never cause a
    /// wrong one — which is the direction every failure here must lean.
    fn enforce_node_cap(&mut self) {
        if self.nodes.len() <= NODE_CAP {
            return;
        }
        let mut ranked: Vec<(Instant, String)> = self
            .nodes
            .iter()
            .filter_map(|(name, entry)| last_activity(entry).map(|t| (t, name.clone())))
            .collect();
        ranked.sort_by_key(|(t, _)| *t);

        let excess = self.nodes.len() - NODE_CAP;
        for (_, name) in ranked.into_iter().take(excess) {
            self.nodes.remove(&name);
        }
    }
}

/// Newest thing that happened to a node — its last failure, or its cooldown.
fn last_activity(entry: &NodeWindow) -> Option<Instant> {
    entry.hits.back().copied().or(entry.cooldown_until)
}

/// `now - t`, saturating at zero.
///
/// `Instant::duration_since` *panics* when the clock is handed out of order,
/// and under `panic = "abort"` that would kill the process. A stepped or
/// backwards clock must degrade, never abort.
fn elapsed_since(now: Instant, t: Instant) -> Duration {
    now.checked_duration_since(t).unwrap_or_default()
}

// ===========================================================================
// Attribution — the log → node join
// ===========================================================================

/// Does this anomaly count toward a node's failure score?
///
/// `ConnectRefused` is deliberately excluded: a refusal is the *far* end
/// rejecting the connection, which says nothing about the health of the node
/// that carried it. `ProxySwitch` is a normal event, not a failure.
pub fn is_trigger(a: &LogAnomaly) -> bool {
    matches!(
        a,
        LogAnomaly::DnsResolveFailed { .. }
            | LogAnomaly::DialTimeout { .. }
            | LogAnomaly::TlsError { .. }
    )
}

/// The destination an anomaly carries — a host, or an `IP:port`.
fn destination_of(a: &LogAnomaly) -> Option<&str> {
    let raw = match a {
        LogAnomaly::DnsResolveFailed { host, .. } | LogAnomaly::TlsError { host, .. } => host,
        LogAnomaly::DialTimeout { address, .. } | LogAnomaly::ConnectRefused { address } => address,
        LogAnomaly::ProxySwitch { .. } => return None,
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// The egress node of a connection: the last element of its chain.
///
/// `chains.last()` is the convention the whole codebase already uses — the
/// renderer derives its `policy` column the same way — so the autopilot and the
/// UI can never disagree about what a connection's node is.
fn egress_node(conn: &ConnectionInfo) -> Option<&str> {
    conn.chains
        .last()
        .or_else(|| conn.provider_chains.last())
        .map(|s| s.as_str())
        .filter(|s| !s.trim().is_empty())
}

/// Does this connection's destination relate to `dest`?
///
/// Two shapes, because the anomaly text carries two:
///   * `IP:port` — prefix-matched against the connection's own `ip:port`
///     (mihomo's addresses are the dial target, which is the IP, not a name);
///   * a bare host or IP — substring-matched against `host` / `sniffHost`, or
///     compared exactly against `destinationIP`.
fn destination_hits(conn: &ConnectionInfo, dest: &str) -> bool {
    let Some(meta) = conn.metadata.as_ref() else {
        return false;
    };
    if contains_ci(&meta.host, dest) || contains_ci(&meta.sniff_host, dest) {
        return true;
    }
    if dest.contains(':') {
        // NOTE: a bare IPv6 literal also contains ':' and would land here. It
        // then only matches on the host fields, which is the safe direction —
        // over-matching here scores a hit, and a hit cannot kill by itself.
        format!("{}:{}", meta.destination_ip, meta.destination_port).starts_with(dest)
    } else {
        meta.destination_ip == dest
    }
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

/// Map an anomaly onto the egress nodes it plausibly belongs to.
///
/// Returns every *distinct* node carrying a connection to that destination —
/// we do not guess which one failed. An empty result means "no attribution",
/// and the caller must then do nothing.
pub fn attribute(snapshot: &ConnectionsSnapshot, a: &LogAnomaly) -> Vec<String> {
    let Some(dest) = destination_of(a) else {
        return Vec::new();
    };

    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for conn in &snapshot.connections {
        if !destination_hits(conn, dest) {
            continue;
        }
        if let Some(node) = egress_node(conn) {
            if seen.insert(node) {
                out.push(node.to_string());
            }
        }
    }
    out
}

// ===========================================================================
// Sink — bounded, non-blocking, and it can never slow the ingest loop
// ===========================================================================

/// The ingest-side half of the bridge. Managed state, so `core::ingest` can
/// reach it without owning anything.
pub struct AnomalySink {
    tx: mpsc::Sender<LogAnomaly>,
}

impl AnomalySink {
    /// Create the sink and the receiver the autopilot consumes.
    pub fn new() -> (Self, mpsc::Receiver<LogAnomaly>) {
        let (tx, rx) = mpsc::channel(SINK_CAP);
        (Self { tx }, rx)
    }

    /// Hand one anomaly to the autopilot. Never blocks, never fails.
    ///
    /// A full queue drops the **newest**: the policy scores a *pattern*, not an
    /// exact count, so losing duplicates inside a burst is harmless — whereas
    /// blocking is not, because this runs on the `/logs` read loop. Returns
    /// whether the anomaly was accepted (useful only for tests).
    pub fn send(&self, a: LogAnomaly) -> bool {
        self.tx.try_send(a).is_ok()
    }
}

// ===========================================================================
// Autopilot — the async half
// ===========================================================================

/// Spawn the autopilot. Called once from `setup` (after `mount_events`).
pub fn spawn<R: Runtime>(app: &AppHandle<R>, rx: mpsc::Receiver<LogAnomaly>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { run(app, rx).await });
}

async fn run<R: Runtime>(app: AppHandle<R>, mut rx: mpsc::Receiver<LogAnomaly>) {
    let mut policy = DeadLinkPolicy::new();

    let mut ticker = tokio::time::interval(EVAL_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    // `interval`'s first tick completes immediately; consume it so the first
    // evaluation lands one full period in, like every later one.
    ticker.tick().await;

    loop {
        ticker.tick().await;
        evaluate(&app, &mut rx, &mut policy).await;
    }
}

/// One evaluation: drain, attribute, score, clean.
async fn evaluate<R: Runtime>(
    app: &AppHandle<R>,
    rx: &mut mpsc::Receiver<LogAnomaly>,
    policy: &mut DeadLinkPolicy,
) {
    let mut batch: Vec<LogAnomaly> = Vec::new();
    while let Ok(a) = rx.try_recv() {
        batch.push(a);
    }

    let now = Instant::now();
    if batch.is_empty() {
        // Idle: no snapshot is fetched at all, so a quiet kernel costs nothing.
        policy.prune(now);
        return;
    }

    // Exactly one snapshot per evaluation, however long the batch was.
    let snapshot = match connections::fetch_snapshot().await {
        Ok(s) => s,
        Err(_) => {
            // The kernel is not answering. Deliberately NOT scored: an
            // unreachable controller is not evidence against any node, and
            // counting it would fire cleanups during every restart.
            policy.prune(now);
            return;
        }
    };

    let mut fired: Vec<String> = Vec::new();
    for anomaly in &batch {
        if !is_trigger(anomaly) {
            continue;
        }
        for node in attribute(&snapshot, anomaly) {
            if let DeadLinkDecision::Fire { node } = policy.record(&node, now) {
                // One cleanup per node per evaluation: `record` keeps firing
                // while the hits are still over the threshold, and the cooldown
                // is only armed by `note_cleanup` inside `cleanup`.
                if !fired.contains(&node) {
                    fired.push(node);
                }
            }
        }
    }

    // Serial by construction — see the module doc for why this must not fan out.
    for node in fired {
        cleanup(app, policy, &node).await;
    }

    policy.prune(Instant::now());
}

/// Book and perform one node's cleanup, then report it.
async fn cleanup<R: Runtime>(app: &AppHandle<R>, policy: &mut DeadLinkPolicy, node: &str) {
    let now = Instant::now();
    // Arm the cooldown *first*, so a slow or failing cleanup backs off instead
    // of being re-entered on the next evaluation.
    policy.note_cleanup(node, now);

    if !autokill_enabled() {
        let reason = "suppressed by FLEXCLASH_AUTOKILL=off";
        report(app, node, 0, 0, reason);
        banner(app, format!("[deadlink] {node}: cleanup {reason}"));
        return;
    }

    let filter = ConnectionFilter {
        proxy: Some(node.to_string()),
        ..ConnectionFilter::default()
    };

    match connections::kill_by_filter(&filter).await {
        Ok(r) => {
            let reason = format!("{} in {}s", r.killed, WINDOW.as_secs());
            banner(
                app,
                format!(
                    "[deadlink] {node}: killed {} connection(s), {} failed",
                    r.killed, r.failed
                ),
            );
            report(app, node, r.killed, r.failed, &reason);
        }
        Err(e) => {
            // Already cooled down by `note_cleanup`, so the next attempt is at
            // earliest `COOLDOWN` away rather than immediately.
            banner(app, format!("[deadlink] {node}: cleanup failed: {e}"));
        }
    }
}

fn report<R: Runtime>(app: &AppHandle<R>, node: &str, killed: usize, failed: usize, reason: &str) {
    let _ = DeadLinkCleanup {
        node: node.to_string(),
        killed,
        failed,
        reason: reason.to_string(),
        at_ms: chrono::Utc::now().timestamp_millis(),
    }
    .emit(app);
}

/// Push a `[deadlink] …` banner to stderr and the kernel log channel, mirroring
/// `core::ingest::banner` — a headless run has no other observable surface.
fn banner<R: Runtime>(app: &AppHandle<R>, line: impl Into<String>) {
    let line = line.into();
    eprintln!("{line}");
    let _ = app.emit(events::KERNEL_LOG, line);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::connections::ConnectionMetadata;

    fn base() -> Instant {
        Instant::now()
    }

    fn conn(id: &str, chains: &[&str], host: &str, ip: &str, port: &str) -> ConnectionInfo {
        let meta = ConnectionMetadata {
            host: host.to_string(),
            destination_ip: ip.to_string(),
            destination_port: port.to_string(),
            ..ConnectionMetadata::default()
        };
        ConnectionInfo {
            id: id.to_string(),
            metadata: Some(meta),
            upload: 0,
            download: 0,
            start: String::new(),
            chains: chains.iter().map(|s| s.to_string()).collect(),
            rule: String::new(),
            rule_payload: String::new(),
            provider_chains: Vec::new(),
        }
    }

    fn snap(conns: Vec<ConnectionInfo>) -> ConnectionsSnapshot {
        ConnectionsSnapshot {
            connections: conns,
            ..ConnectionsSnapshot::default()
        }
    }

    fn anomaly_dns(host: &str) -> LogAnomaly {
        LogAnomaly::DnsResolveFailed {
            host: host.to_string(),
            detail: "no such host".into(),
        }
    }

    // -- sliding window ------------------------------------------------------

    #[test]
    fn policy_fires_on_nth_hit_in_window() {
        let mut p = DeadLinkPolicy::new();
        let t0 = base();
        assert_eq!(p.record("HK-01", t0), DeadLinkDecision::Wait);
        assert_eq!(p.record("HK-01", t0), DeadLinkDecision::Wait);
        assert_eq!(
            p.record("HK-01", t0),
            DeadLinkDecision::Fire {
                node: "HK-01".into()
            }
        );
    }

    #[test]
    fn policy_ignores_hits_outside_window() {
        let mut p = DeadLinkPolicy::new();
        let t0 = base();
        p.record("HK-01", t0);
        p.record("HK-01", t0);
        // Past the window: the two earlier hits must have expired, so even a
        // fresh pair is not enough.
        let later = t0 + WINDOW + Duration::from_secs(1);
        assert_eq!(p.record("HK-01", later), DeadLinkDecision::Wait);
        assert_eq!(p.record("HK-01", later), DeadLinkDecision::Wait);
        assert_eq!(
            p.record("HK-01", later),
            DeadLinkDecision::Fire {
                node: "HK-01".into()
            }
        );
    }

    #[test]
    fn cooldown_blocks_immediate_refire_and_drops_evidence() {
        let mut p = DeadLinkPolicy::new();
        let t0 = base();
        for _ in 0..THRESHOLD {
            p.record("HK-01", t0);
        }
        p.note_cleanup("HK-01", t0);

        let during = t0 + Duration::from_secs(5);
        // Evidence banked during the cooldown must NOT arm the window again.
        for _ in 0..THRESHOLD {
            assert_eq!(p.record("HK-01", during), DeadLinkDecision::CoolingDown);
        }
        assert_eq!(p.record("HK-01", during), DeadLinkDecision::CoolingDown);

        // After the cooldown the node starts from zero, not from a full window.
        let after = t0 + COOLDOWN + Duration::from_secs(1);
        assert_eq!(p.record("HK-01", after), DeadLinkDecision::Wait);
        assert_eq!(p.record("HK-01", after), DeadLinkDecision::Wait);
        assert_eq!(
            p.record("HK-01", after),
            DeadLinkDecision::Fire {
                node: "HK-01".into()
            }
        );
    }

    #[test]
    fn global_budget_caps_cleanups() {
        let mut p = DeadLinkPolicy::new();
        let t0 = base();
        let mut fired = 0usize;
        for i in 0..(GLOBAL_BUDGET + 2) {
            let node = format!("N{i}");
            for _ in 0..THRESHOLD {
                if let DeadLinkDecision::Fire { .. } = p.record(&node, t0) {
                    fired += 1;
                    p.note_cleanup(&node, t0);
                }
            }
        }
        assert_eq!(fired, GLOBAL_BUDGET);
    }

    #[test]
    fn budget_recovers_after_its_window() {
        let mut p = DeadLinkPolicy::new();
        let t0 = base();
        for i in 0..GLOBAL_BUDGET {
            let node = format!("N{i}");
            for _ in 0..THRESHOLD {
                if let DeadLinkDecision::Fire { .. } = p.record(&node, t0) {
                    p.note_cleanup(&node, t0);
                }
            }
        }
        // Same nodes are frozen; a brand-new one is blocked by the budget.
        let fresh = "NEW";
        for _ in 0..THRESHOLD {
            p.record(fresh, t0);
        }
        assert_eq!(p.record(fresh, t0), DeadLinkDecision::Wait);

        // Once the budget window rolls, the same node can fire.
        let later = t0 + BUDGET_WINDOW + Duration::from_secs(1);
        for _ in 0..THRESHOLD {
            p.record(fresh, later);
        }
        assert_eq!(
            p.record(fresh, later),
            DeadLinkDecision::Fire { node: fresh.into() }
        );
    }

    // -- bounded memory -----------------------------------------------------

    #[test]
    fn prune_drops_idle_nodes() {
        let mut p = DeadLinkPolicy::new();
        let t0 = base();
        p.record("HK-01", t0);
        assert!(p.nodes.contains_key("HK-01"));

        p.prune(t0 + WINDOW + Duration::from_secs(1));
        assert!(!p.nodes.contains_key("HK-01"));
    }

    #[test]
    fn prune_keeps_cooling_nodes() {
        let mut p = DeadLinkPolicy::new();
        let t0 = base();
        p.note_cleanup("HK-01", t0);
        p.prune(t0 + Duration::from_secs(1));
        assert!(p.nodes.contains_key("HK-01"));
    }

    #[test]
    fn node_cap_is_enforced() {
        let mut p = DeadLinkPolicy::new();
        let t0 = base();
        for i in 0..(NODE_CAP + 32) {
            // Newest activity strictly increases, so the eviction is
            // deterministic: the earliest-recorded nodes go first.
            p.record(&format!("N{i}"), t0 + Duration::from_millis(i as u64));
        }
        p.prune(t0 + Duration::from_millis(NODE_CAP as u64));
        assert!(p.nodes.len() <= NODE_CAP);
    }

    // -- attribution --------------------------------------------------------

    #[test]
    fn attribution_matches_by_host() {
        let s = snap(vec![conn(
            "a",
            &["DIRECT", "HK-01"],
            "example.com",
            "1.2.3.4",
            "443",
        )]);
        assert_eq!(attribute(&s, &anomaly_dns("example.com")), vec!["HK-01"]);
    }

    #[test]
    fn attribution_matches_by_address() {
        let s = snap(vec![conn("a", &["HK-02"], "", "1.2.3.4", "443")]);
        let a = LogAnomaly::DialTimeout {
            address: "1.2.3.4:443".into(),
            elapsed_ms: None,
        };
        assert_eq!(attribute(&s, &a), vec!["HK-02"]);
    }

    #[test]
    fn attribution_records_every_distinct_node() {
        // "Honest guilt by association": one destination, two egress nodes.
        let s = snap(vec![
            conn("a", &["HK-01"], "cdn.example.net", "", ""),
            conn("b", &["JP-01"], "cdn.example.net", "", ""),
            conn("c", &["HK-01"], "cdn.example.net", "", ""),
        ]);
        let mut got = attribute(&s, &anomaly_dns("cdn.example.net"));
        got.sort();
        assert_eq!(got, vec!["HK-01", "JP-01"]);
    }

    #[test]
    fn attribution_returns_nothing_without_a_match() {
        let s = snap(vec![conn("a", &["HK-01"], "example.com", "1.2.3.4", "443")]);
        assert!(attribute(&s, &anomaly_dns("elsewhere.test")).is_empty());
        // An empty destination can never be attributed either.
        assert!(attribute(&s, &anomaly_dns("")).is_empty());
        // A connection with no metadata is invisible to attribution.
        let mut bare = conn("b", &["HK-01"], "", "", "");
        bare.metadata = None;
        assert!(attribute(&snap(vec![bare]), &anomaly_dns("example.com")).is_empty());
    }

    #[test]
    fn egress_node_is_last_chain() {
        let c = conn("a", &["GLOBAL", "HK-01"], "", "", "");
        assert_eq!(egress_node(&c), Some("HK-01"));
        // Falls back to the provider chain, and rejects blank entries.
        let mut bare = conn("b", &[], "", "", "");
        bare.provider_chains = vec![" ".into(), "JP-02".into()];
        assert_eq!(egress_node(&bare), Some("JP-02"));
        let empty = conn("c", &[], "", "", "");
        assert_eq!(egress_node(&empty), None);
    }

    // -- trigger set --------------------------------------------------------

    #[test]
    fn trigger_set_covers_the_three_failure_kinds() {
        assert!(is_trigger(&anomaly_dns("x.test")));
        assert!(is_trigger(&LogAnomaly::DialTimeout {
            address: "1.2.3.4:443".into(),
            elapsed_ms: None,
        }));
        assert!(is_trigger(&LogAnomaly::TlsError {
            host: "x.test".into(),
            detail: "x509".into(),
        }));
        // A refusal is the far end rejecting, not the node failing.
        assert!(!is_trigger(&LogAnomaly::ConnectRefused {
            address: "1.2.3.4:443".into(),
        }));
        // A switch is a normal event, not a failure.
        assert!(!is_trigger(&LogAnomaly::ProxySwitch {
            from: None,
            to: "HK-01".into(),
        }));
    }

    // -- sink ---------------------------------------------------------------

    #[test]
    fn sink_drops_the_newest_when_full() {
        let (sink, mut rx) = AnomalySink::new();
        for i in 0..SINK_CAP {
            assert!(sink.send(anomaly_dns(&format!("h{i}.test"))));
        }
        // Full: the send is refused rather than blocking or panicking.
        assert!(!sink.send(anomaly_dns("overflow.test")));

        let mut drained = Vec::new();
        while let Ok(a) = rx.try_recv() {
            drained.push(a);
        }
        assert_eq!(drained.len(), SINK_CAP);
    }

    // -- clock safety -------------------------------------------------------

    #[test]
    fn backwards_clock_never_panics() {
        let later = base();
        let earlier = later - Duration::from_secs(5);
        // `Instant::duration_since` would panic here; the helper must not.
        assert_eq!(elapsed_since(later, earlier), Duration::from_secs(5));
        assert_eq!(elapsed_since(earlier, later), Duration::ZERO);

        let mut p = DeadLinkPolicy::new();
        p.record("HK-01", later);
        p.record("HK-01", earlier);
        p.prune(earlier);
    }
}

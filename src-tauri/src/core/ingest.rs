// ============================================================================
// core/ingest.rs — Rust owns the kernel data plane.
//
// WHY THIS LIVES IN RUST
// ----------------------
// Mihomo pushes its live traffic rate and its logs over two WebSocket
// endpoints on the controller (`127.0.0.1:9091`):
//
//     /traffic   {"up":N,"down":N}               ~1 Hz, one sample per push
//     /logs      {"type":"info","payload":"…"}   once per emitted log line
//
// Both used to be dialled from the *renderer* — `useTrafficStream.ts` opened
// `/traffic` directly, and `services/clash.ts` had an (`/connections`) helper
// that was never even subscribed. That put a background socket inside the
// webview, where it competes with rendering, dies with the window instead of
// with the kernel, and cannot be throttled. Here they are owned by one
// supervisor task that outlives every window, coalesces and batches, and heals
// itself across kernel restarts.
//
// THROTTLING / BATCHING — the two shapes, and why each is what it is
// ------------------------------------------------------------------
// `/traffic` is a *latest-value* stream: only the newest sample is
// interesting, and mihomo may push faster than the UI can paint. So the reader
// keeps a single `Option<TrafficPayload>` that every push overwrites, and a
// `tokio::time::interval(1000ms)` tick emits it. That is coalescing, not
// queueing — fifty pushes inside one tick collapse to one event. There is
// deliberately **no channel here**: the reader and the ticker are two branches
// of the same `select!`, so the "buffer" is a local variable and cannot grow.
//   ⇒ `tokio::time::interval` + overwrite-the-latest, in one `select!`.
//
// `/logs` is an *accumulate-all* stream: every line matters and dropping one
// is a bug, but a per-line IPC event would be hundreds of wakeups a second.
// So the reader appends to a `VecDeque` and a `tokio::time::interval(500ms)`
// tick drains the whole buffer into one `LogBatch`. The queue is bounded; a
// log storm drops the *oldest* lines rather than growing without limit.
//   ⇒ `tokio::time::interval` + bounded drain, in the same `select!`.
//
// (An `mpsc` channel is the usual answer when the producer and consumer are
// separate tasks. Here they are not: folding them into one `select!` loop per
// endpoint removes the channel *and* the unbounded-growth bug class it would
// otherwise reintroduce.)
//
// LIFECYCLE / PANIC SAFETY
// ------------------------
// Release builds set `panic = "abort"`, so any panic in here takes the whole
// process down with it. Nothing in the loops therefore uses
// `unwrap`/`expect`/indexing:
//   * every parse is `Option`-gated and a malformed frame is simply skipped;
//   * the supervisor is *spawned*, and it aborts both endpoint tasks the
//     moment the kernel leaves `Running`, then waits for the next start;
//   * each endpoint task reconnects forever with bounded exponential backoff,
//     so a kernel crash/restart heals with no UI involvement;
//   * `/traffic` carries an idle watchdog — it is a guaranteed ~1 Hz
//     heartbeat, so silence means a half-open socket (a bare FIN is not the
//     only way a loopback connection dies) and forces a reconnect.
// ============================================================================

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, Runtime};
use tauri_specta::Event;
use tokio::net::TcpStream;
use tokio::time::MissedTickBehavior;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use crate::config::profile::RESERVED_CONTROLLER;
use crate::core::kernel_events::{KernelLogLevel, LogBatch, LogPayload, TrafficPayload};
use crate::core::sidecar::{KernelState, SidecarHandle};
use crate::events;

/// The concrete socket this module speaks. With no TLS feature enabled
/// `MaybeTlsStream` is always the plain variant, but it is what `connect_async`
/// hands back, so it is what the sessions are typed over.
type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// `/traffic` flush cadence — only the latest sample survives between ticks.
const TRAFFIC_FLUSH: Duration = Duration::from_millis(1000);
/// `/logs` flush cadence — the whole buffer is drained each tick.
const LOGS_FLUSH: Duration = Duration::from_millis(500);
/// Silence longer than this on `/traffic` means the socket is half-open.
const TRAFFIC_IDLE_TIMEOUT: Duration = Duration::from_secs(8);
/// Reconnect backoff bounds.
const RECONNECT_MIN: Duration = Duration::from_millis(500);
const RECONNECT_MAX: Duration = Duration::from_secs(10);
/// A session that lived at least this long counts as healthy, so the next
/// reconnect restarts the backoff from `RECONNECT_MIN`.
const STABLE_SESSION: Duration = Duration::from_secs(5);
/// How often the supervisor re-checks the kernel lifecycle state.
const KERNEL_POLL: Duration = Duration::from_millis(500);
/// Hard cap on buffered log lines; on overflow the oldest are dropped.
const LOG_BUFFER_CAP: usize = 4096;

/// Spawn the ingest supervisor.
///
/// Called once from `run()`'s setup. Fire-and-forget: it owns a loop for the
/// life of the process and shares nothing but the `AppHandle` and the sidecar
/// handle's state.
pub fn spawn<R: Runtime>(app: &AppHandle<R>, sidecar: SidecarHandle) {
    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        loop {
            // Idle until the kernel is up — there is nothing to connect to
            // before the controller port is bound.
            if sidecar.state() != KernelState::Running {
                tokio::time::sleep(KERNEL_POLL).await;
                continue;
            }

            run_session(&app, &sidecar).await;

            // The session ended because the kernel left `Running`. A short
            // pause keeps a rapid start/stop cycle from spinning the loop.
            tokio::time::sleep(RECONNECT_MIN).await;
        }
    });
}

/// One kernel uptime: open both endpoints and tear them down — abruptly — as
/// soon as the kernel is no longer `Running`.
async fn run_session<R: Runtime>(app: &AppHandle<R>, sidecar: &SidecarHandle) {
    let traffic = tauri::async_runtime::spawn(reconnect_loop(
        app.clone(),
        "/traffic",
        "kernel /traffic",
        traffic_session::<R>,
    ));
    let logs = tauri::async_runtime::spawn(reconnect_loop(
        app.clone(),
        "/logs",
        "kernel /logs",
        logs_session::<R>,
    ));

    while sidecar.state() == KernelState::Running {
        tokio::time::sleep(KERNEL_POLL).await;
    }

    // `abort()` cancels each task at its next await point; dropping the
    // `WebSocketStream` there closes the TCP connection. No close handshake is
    // needed — the peer is the kernel we are walking away from, and if it is
    // the one that died, the write would not land anyway.
    traffic.abort();
    logs.abort();
}

/// Connect `endpoint`, run `session` over the socket, and repeat forever with
/// bounded exponential backoff. Only returns if the task is aborted.
async fn reconnect_loop<R, S, F>(app: AppHandle<R>, endpoint: &str, label: &str, session: S)
where
    R: Runtime,
    S: Fn(AppHandle<R>, WsStream) -> F,
    F: std::future::Future<Output = ()>,
{
    let url = format!("ws://{RESERVED_CONTROLLER}{endpoint}");
    let mut backoff = RECONNECT_MIN;

    loop {
        let started = Instant::now();
        match connect_async(url.as_str()).await {
            Ok((ws, _response)) => {
                banner(&app, format!("[ingest] {label} connected"));
                session(app.clone(), ws).await;
            }
            Err(e) => {
                banner(&app, format!("[ingest] {label} connect failed: {e}"));
            }
        }

        // Reset the backoff only after a session that actually lived. A socket
        // that is accepted and dropped immediately must keep backing off, or a
        // wedged endpoint would be hammered in a tight loop.
        backoff = if started.elapsed() >= STABLE_SESSION {
            RECONNECT_MIN
        } else {
            (backoff * 2).min(RECONNECT_MAX)
        };
        tokio::time::sleep(backoff).await;
    }
}

/// `/traffic` session: coalesce to the latest sample, emit once per second.
async fn traffic_session<R: Runtime>(app: AppHandle<R>, mut ws: WsStream) {
    let mut latest: Option<TrafficPayload> = None;
    let mut last_seen = Instant::now();
    let mut pending_pong: Option<Message> = None;

    let mut ticker = tokio::time::interval(TRAFFIC_FLUSH);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    // `interval`'s first tick completes immediately; consume it so the first
    // emit lands one full period in, exactly like every later one.
    ticker.tick().await;

    loop {
        tokio::select! {
            incoming = ws.next() => match incoming {
                Some(Ok(msg)) => {
                    last_seen = Instant::now();
                    if msg.is_ping() {
                        // Answer keep-alives or the peer may drop us. The send
                        // itself happens below, *outside* the `select!`, so it
                        // cannot overlap the read borrow of `ws`.
                        if let Message::Ping(payload) = msg {
                            pending_pong = Some(Message::Pong(payload));
                        }
                    } else if msg.is_close() {
                        break;
                    } else if let Ok(text) = msg.to_text() {
                        if let Some(sample) = parse_traffic(text) {
                            latest = Some(sample);
                        }
                    }
                }
                // A transport error and a clean EOF both end the session; the
                // reconnect loop picks it up from here.
                Some(Err(_)) | None => break,
            },
            _ = ticker.tick() => {
                // Watchdog: `/traffic` is a guaranteed ~1 Hz heartbeat, so
                // prolonged silence means the socket is half-open. Reconnect
                // rather than wait on a read that may never return.
                if last_seen.elapsed() > TRAFFIC_IDLE_TIMEOUT {
                    break;
                }
                if let Some(sample) = latest.take() {
                    let _ = sample.emit(&app);
                }
            }
        }

        if let Some(pong) = pending_pong.take() {
            // Best-effort; a failed pong surfaces on the next read.
            let _ = ws.send(pong).await;
        }
    }
}

/// `/logs` session: accumulate every line, emit one batch every 500 ms.
async fn logs_session<R: Runtime>(app: AppHandle<R>, mut ws: WsStream) {
    let mut buffer: VecDeque<LogPayload> = VecDeque::new();
    let mut pending_pong: Option<Message> = None;

    let mut ticker = tokio::time::interval(LOGS_FLUSH);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    ticker.tick().await;

    loop {
        tokio::select! {
            incoming = ws.next() => match incoming {
                Some(Ok(msg)) => {
                    if msg.is_ping() {
                        if let Message::Ping(payload) = msg {
                            pending_pong = Some(Message::Pong(payload));
                        }
                    } else if msg.is_close() {
                        break;
                    } else if let Ok(text) = msg.to_text() {
                        if let Some(entry) = parse_log(text) {
                            // Promote connectivity anomalies into a typed event.
                            // These are rare, so they are emitted immediately
                            // rather than riding the 500 ms raw-log batch.
                            if let Some(anomaly) = crate::core::log_parse::parse_anomaly(&entry) {
                                let _ = anomaly.emit(&app);
                            }
                            // Bounded: a log storm drops the oldest line rather
                            // than growing the queue without limit. The kernel
                            // is the only writer, so this is never hit in
                            // practice — it is the belt to the interval's
                            // braces.
                            if buffer.len() >= LOG_BUFFER_CAP {
                                buffer.pop_front();
                            }
                            buffer.push_back(entry);
                        }
                    }
                }
                Some(Err(_)) | None => break,
            },
            _ = ticker.tick() => {
                if !buffer.is_empty() {
                    let _ = LogBatch(buffer.drain(..).collect()).emit(&app);
                }
            }
        }

        if let Some(pong) = pending_pong.take() {
            let _ = ws.send(pong).await;
        }
    }

    // Flush the tail so the final <500 ms of logs are not lost when the
    // session ends.
    if !buffer.is_empty() {
        let _ = LogBatch(buffer.drain(..).collect()).emit(&app);
    }
}

/// Raw `/traffic` frame: `{"up":N,"down":N}` in bytes/second. Signed because
/// mihomo serialises an `int64`; clamped on the way in so a negative or absurd
/// value cannot become a bogus huge `u32`.
#[derive(Deserialize)]
struct RawTraffic {
    #[serde(default)]
    up: i64,
    #[serde(default)]
    down: i64,
}

fn parse_traffic(text: &str) -> Option<TrafficPayload> {
    let raw: RawTraffic = serde_json::from_str(text).ok()?;
    Some(TrafficPayload {
        up: clamp_rate(raw.up),
        down: clamp_rate(raw.down),
    })
}

/// Map an `i64` rate into `u32`, saturating at both ends.
fn clamp_rate(value: i64) -> u32 {
    value.clamp(0, i64::from(u32::MAX)) as u32
}

/// Raw `/logs` frame: `{"type":"info","payload":"…"}`.
#[derive(Deserialize)]
struct RawLog {
    #[serde(rename = "type", default)]
    level: String,
    #[serde(default)]
    payload: String,
}

fn parse_log(text: &str) -> Option<LogPayload> {
    let raw: RawLog = serde_json::from_str(text).ok()?;
    Some(LogPayload {
        level: KernelLogLevel::from_wire(&raw.level),
        message: raw.payload,
        at_ms: chrono::Utc::now().timestamp_millis(),
    })
}

/// Push an `[ingest] …` banner onto the kernel log channel.
///
/// Ingest has no other observable surface, and "the traffic panel is stuck at
/// zero" is exactly the symptom a failed connection produces — so the reason
/// it is not connecting has to go somewhere. This is the cheapest honest
/// place, and it matches the `[sidecar] …` banners already on that channel.
fn banner<R: Runtime>(app: &AppHandle<R>, line: impl Into<String>) {
    let line = line.into();
    eprintln!("{line}");
    let _ = app.emit(events::KERNEL_LOG, line);
}

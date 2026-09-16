// ============================================================================
// core/kernel_events.rs — the typed payloads the kernel data plane pushes to
// the renderer as *events*.
//
// WHY THIS MODULE EXISTS
// ----------------------
// The kernel data plane (Mihomo's `/traffic` and `/logs` streams, plus the
// sidecar's own config-refresh notice) used to reach the renderer untyped:
// a bare JSON object for `/traffic`, a bare string for `/logs`, and — worst of
// all — a *hand-written* interface mirrored in `stores/kernel.ts` for the
// config-refresh notice. Nothing tied those three shapes to the Rust
// definitions, so the mirror could drift silently. That is exactly the failure
// the command registry (`bindings.rs`) was built to remove, and it is the same
// fix here.
//
// Each payload carries `#[derive(tauri_specta::Event)]` and is registered in
// `bindings::builder()` through `collect_events![…]`, which simultaneously:
//
//   * makes it emittable through the typed `.emit(&app)` / `.emit_to(…)` API
//     (and *only* through it — an unregistered payload has no such method);
//   * puts its TypeScript shape in `src/bindings.ts`; and
//   * gives the renderer a typed `events.<name>.listen(…)` with no hand copy,
//     so `stores/kernel.ts` can delete its mirror.
//
// EVENT NAMING
// ------------
// `tauri-specta` derives the wire name from the struct name (`TrafficPayload`
// → raw Tauri event `"TrafficPayload"`, exported as `events.trafficPayload`).
// The renderer never spells that string itself — it goes through the generated
// `events` object — so the name stays an implementation detail. The wire names
// deliberately do NOT follow the older `namespace://verb` convention for the
// same reason: there is now a single typed producer/consumer pair for each.
//
// BIGINT NOTE
// -----------
// `LogPayload::at_ms` is an `i64`, which `dangerously_cast_bigints_to_number`
// exports as TypeScript `number`. Epoch milliseconds are ~2^41 today, far
// inside the 2^53 exact-integer range, so the cast is lossless here. Any future
// field that could exceed 2^53 must be a string instead — see the invariant in
// `bindings.rs`.
// ============================================================================

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;

/// One `/traffic` sample — the kernel's own instantaneous byte rates.
///
/// Mihomo pushes a sample on its `/traffic` WebSocket roughly once per second.
/// The ingest task (`core::ingest`) coalesces those pushes down to the *latest*
/// sample and re-emits it exactly once per second, so the renderer sees a
/// steady 1 Hz tick no matter how chatty the kernel is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type, Event)]
pub struct TrafficPayload {
    /// Egress (upload) bytes per second over the last interval.
    pub up: u32,
    /// Ingress (download) bytes per second over the last interval.
    pub down: u32,
}

/// Mihomo's log severity, mirrored from the `type` field of a `/logs` frame.
///
/// Kept as its own enum (rather than reusing `types/clash.d.ts`'s `LogLevel`)
/// because this one is *generated* and therefore authoritative.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum KernelLogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

impl KernelLogLevel {
    /// Map Mihomo's wire spelling onto the enum, defaulting to `Info` for
    /// anything unrecognised.
    ///
    /// Deliberately total: a severity a future kernel invents must not be able
    /// to fail the ingest loop, and under `panic = "abort"` a failure here
    /// would take the whole process with it.
    pub(crate) fn from_wire(raw: &str) -> Self {
        match raw {
            "debug" => Self::Debug,
            // Mihomo writes "warning"; `warn` is accepted defensively.
            "warning" | "warn" => Self::Warning,
            "error" => Self::Error,
            _ => Self::Info,
        }
    }
}

/// A single structured kernel log line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LogPayload {
    /// Severity as reported by the kernel.
    pub level: KernelLogLevel,
    /// The log line itself (Mihomo's `payload` field).
    pub message: String,
    /// Receipt time, as Unix epoch milliseconds.
    pub at_ms: i64,
}

/// One flush of buffered log lines — the `/logs` ingest pushes a *batch* every
/// 500 ms rather than one event per line.
///
/// `#[serde(transparent)]` is deliberate: the wire shape, and therefore the
/// generated TypeScript, is the bare `LogPayload[]` rather than a single-field
/// wrapper object. `tauri-specta` still sees a *named reference* type, which
/// its `collect_events!` registration requires — an event payload cannot be a
/// bare `Vec` (see `register_event`'s `Can't register event … with
/// non-reference type` guard).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type, Event)]
#[serde(transparent)]
pub struct LogBatch(pub Vec<LogPayload>);

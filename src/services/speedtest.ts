// ============================================================================
// services/speedtest.ts — Renderer client for the Rust-native speed test.
//
// The engine, the concurrency pool and the timeout budget all live in
// `src-tauri/src/core/speedtest.rs`. This module is only the thin IPC face:
//
//   startSpeedTest()  → fire and forget; returns the run id at once
//   cancelSpeedTest() → supersede the run owning a group
//   onDelayBatch()    → incremental results, batched by the engine
//   onDelayDone()     → exactly one terminal event per run, always
//
// No results travel back through the `invoke` return value. A few hundred
// nodes would otherwise have to be marshalled into a single payload, which is
// precisely the "wait for everything, then render" pattern this design exists
// to avoid.
//
// Like `services/tun.ts`, the payload types are declared here rather than
// pulled from the generated `@/bindings`: the speed-test commands are plain
// `#[tauri::command]`s (see the note in `commands/speedtest.rs` for why), so
// there is no generated type to import. Field names are camelCase because the
// Rust DTOs carry `#[serde(rename_all = "camelCase")]`.
// ============================================================================

import { safeInvoke, safeListen, type UnlistenFn } from '@/utils/tauri-bridge'

// ---------------------------------------------------------------------------
// Event names — must match the constants in `src-tauri/src/events.rs`.
// ---------------------------------------------------------------------------

export const DELAY_BATCH_EVENT = 'proxy://delay-batch'
export const DELAY_DONE_EVENT = 'proxy://delay-done'

// ---------------------------------------------------------------------------
// Defaults — must match the `const`s in `src-tauri/src/core/speedtest.rs`.
// Duplicated deliberately: the frontend needs them to seed its own state before
// the first event lands.
// ---------------------------------------------------------------------------

export const DEFAULT_TEST_URL = 'http://www.gstatic.com/generate_204'
export const DEFAULT_TIMEOUT_MS = 5_000
export const DEFAULT_CONCURRENCY = 64

// ---------------------------------------------------------------------------
// Wire types (mirror `core/speedtest.rs`)
// ---------------------------------------------------------------------------

/** Terminal outcome of one probe. */
export type ProbeStatus = 'ok' | 'timeout' | 'unreachable' | 'error'

export interface NodeProbe {
  name: string
  status: ProbeStatus
  /** RTT in ms; non-null iff `status === 'ok'`. */
  delayMs: number | null
  /** Why a non-`ok` probe ended that way. */
  message: string | null
}

/** Incremental progress push — one event per batch of results. */
export interface DelayBatch {
  runId: number
  group: string
  results: NodeProbe[]
  /** Nodes probed so far, including this batch. */
  done: number
  total: number
}

/** Terminal push — exactly one per run, including a cancelled run. */
export interface DelayDone {
  runId: number
  group: string
  total: number
  /** How many were actually probed; `< total` iff `cancelled`. */
  done: number
  ok: number
  failed: number
  cancelled: boolean
}

export interface StartSpeedTestOptions {
  url?: string
  timeoutMs?: number
  concurrency?: number
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/**
 * Kick off a run against `nodes` and return its id.
 *
 * Resolves as soon as the engine has accepted the run — not when the probes
 * finish. Results arrive on {@link DELAY_BATCH_EVENT}.
 *
 * Rejects on a genuinely bad request (e.g. an empty node list, which would
 * otherwise be a run that never emits a terminal event).
 */
export async function startSpeedTest(
  group: string,
  nodes: string[],
  opts: StartSpeedTestOptions = {},
): Promise<number> {
  return await safeInvoke<number>('speed_test_group', {
    group,
    nodes,
    url: opts.url ?? null,
    timeoutMs: opts.timeoutMs ?? null,
    concurrency: opts.concurrency ?? null,
  })
}

/**
 * Cancel the run owning `group`. Idempotent — cancelling an idle group is a
 * successful no-op, because the alternative (throwing) would make every
 * teardown path guard for a condition that is not an error.
 */
export async function cancelSpeedTest(group: string): Promise<void> {
  await safeInvoke<null>('cancel_speed_test', { group })
}

// ---------------------------------------------------------------------------
// Stream subscriptions
// ---------------------------------------------------------------------------

/** Subscribe to incremental results. Returns the unsubscribe handle. */
export function onDelayBatch(
  handler: (batch: DelayBatch) => void,
): Promise<UnlistenFn> {
  return safeListen<DelayBatch>(DELAY_BATCH_EVENT, (e) => handler(e.payload))
}

/** Subscribe to the per-run terminal event. */
export function onDelayDone(
  handler: (done: DelayDone) => void,
): Promise<UnlistenFn> {
  return safeListen<DelayDone>(DELAY_DONE_EVENT, (e) => handler(e.payload))
}

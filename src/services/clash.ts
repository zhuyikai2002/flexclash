// ============================================================================
// clash.ts — Mihomo RESTful client
//
// Responsibilities:
//   - Provide a single axios instance bound to the Mihomo external-controller
//     port (127.0.0.1:9091 by default).
//   - Convert raw errors into human-readable messages the UI can show.
//   - Expose typed wrappers for every endpoint used by the dashboard.
//
// IMPORTANT: This module speaks DIRECTLY to Mihomo. It does NOT go through
// Rust. The only Rust<->frontend interaction for the kernel lifecycle is via
// @tauri-apps/api invoke/listen in `stores/kernel.ts`.
// ============================================================================

import axios, { AxiosError, AxiosInstance } from 'axios'
import type {
  Config,
  ConnectionsResponse,
  MihomoVersion,
  ProxiesResponse,
  Proxy,
  ProxyDelayResponse,
  Rule,
  RulesResponse,
  TrafficSample,
} from '@/types/clash'

export const MIHOMO_BASE_URL = 'http://127.0.0.1:9091'
export const MIHOMO_WS_URL = 'ws://127.0.0.1:9091'

// ---- axios instance -------------------------------------------------------

const client: AxiosInstance = axios.create({
  baseURL: MIHOMO_BASE_URL,
  // Global ceiling. Per-request overrides (see `isAlive`) tighten this
  // to 2000ms so a stuck mihomo never freezes the dashboard.
  timeout: 5_000,
  headers: { 'Content-Type': 'application/json' },
})

/**
 * Translate transport-level failures into messages a human can act on.
 *
 * Categorisation matters for the cold-boot retry loop in the kernel store:
 *   - CORS / 403     → not retried (config is wrong on the Rust side)
 *   - ECONNREFUSED   → retried (mihomo is still starting)
 *   - timeout        → retried once (mihomo is slow to bind the port)
 *   - 5xx            → retried (mihomo is up but erroring)
 *   - other 4xx      → not retried (request is wrong)
 *
 * Mihomo itself uses 401 for secret mismatch and 404 for unknown proxy names.
 */
function classify(err: AxiosError): { retriable: boolean; reason: string } {
  if (err.response) {
    const s = err.response.status
    if (s === 401) return { retriable: false, reason: 'auth (secret mismatch)' }
    if (s === 403) return { retriable: false, reason: 'CORS / forbidden' }
    if (s === 404) return { retriable: false, reason: 'not found' }
    if (s >= 500)  return { retriable: true,  reason: `HTTP ${s} (server error)` }
    return { retriable: false, reason: `HTTP ${s}` }
  }
  // No response = transport error.
  if (err.code === 'ECONNREFUSED' || err.code === 'ERR_NETWORK') {
    return { retriable: true, reason: 'connection refused (kernel not listening yet?)' }
  }
  if (err.code === 'ECONNABORTED' || err.code === 'ETIMEDOUT') {
    return { retriable: true, reason: 'timeout' }
  }
  return { retriable: false, reason: err.code ?? err.message }
}

function toUserError(err: unknown): Error {
  if (axios.isAxiosError(err)) {
    const c = classify(err as AxiosError)
    // eslint-disable-next-line no-console
    console.debug(`[mihomo] ${(err as AxiosError).config?.url} → ${c.reason}`)
    if (c.reason.startsWith('connection refused')) {
      return new Error('mihomo not reachable — is the kernel running?')
    }
    if (c.reason === 'timeout') {
      return new Error('mihomo request timed out')
    }
    if (c.reason === 'CORS / forbidden') {
      return new Error('mihomo CORS rejected the request — check external-controller-cors.allow-origins')
    }
    return new Error(`mihomo ${c.reason}: ${(err as AxiosError).message}`)
  }
  return err instanceof Error ? err : new Error(String(err))
}

client.interceptors.response.use(
  (r) => r,
  (err) => Promise.reject(toUserError(err)),
)

// ============================================================================
// Health
// ============================================================================

/**
 * Lightweight liveness probe. Returns true iff `GET /version` succeeds within
 * `timeoutMs` (default 2000ms). Catches all errors silently so the kernel
 * store can use it as a poll predicate.
 *
 * NOTE: must NOT throw — the cold-boot retry loop depends on a clean
 * true/false signal. Use `getVersion()` directly if you need the payload.
 */
export async function isAlive(timeoutMs = 2_000): Promise<boolean> {
  try {
    await client.get<MihomoVersion>('/version', { timeout: timeoutMs })
    return true
  } catch {
    return false
  }
}

export async function getVersion(timeoutMs = 2_000): Promise<MihomoVersion> {
  const r = await client.get<MihomoVersion>('/version', { timeout: timeoutMs })
  return r.data
}

// ============================================================================
// Configs
// ============================================================================

export async function getConfigs(): Promise<Config> {
  const r = await client.get<Config>('/configs')
  return r.data
}

/** Trigger mihomo to reload config from disk. */
export async function reloadConfigs(force = true): Promise<void> {
  await client.put('/configs', undefined, { params: { force } })
}

/**
 * Hot-reload a specific profile yaml.
 *
 * mihomo's API is `PUT /configs?force=true` with body
 * `{ "path": "C:/.../profiles/<id>/config.yaml" }`. We forward the body
 * instead of the no-body `reloadConfigs()` so the user-selected profile is
 * the one that becomes active.
 */
export async function reloadConfig(path: string, force = true): Promise<void> {
  await client.put('/configs', { path }, { params: { force } })
}

/** Replace mihomo's in-memory config via PATCH. Reserved for M4. */
export async function patchConfigs(patch: Partial<Config>): Promise<void> {
  await client.patch('/configs', patch)
}

/**
 * Outbound mode switcher.  Phase 8 dashboard capsule.
 *
 * mihomo's `mode` is one of `rule` (default) / `global` / `direct`.
 * The PATCH /configs body shape is `{ "mode": "rule" | "global" | "direct" }`.
 * Throws if the kernel is not reachable; the caller is expected to gate
 * the call behind a `kernel.isRunning` check so the user never sees a
 * raw ECONNREFUSED toast.
 */
export async function setMode(
  mode: 'rule' | 'global' | 'direct',
): Promise<void> {
  await client.patch('/configs', { mode })
}

// ============================================================================
// Proxies
// ============================================================================

export async function getProxies(): Promise<ProxiesResponse> {
  const r = await client.get<ProxiesResponse>('/proxies')
  return r.data
}

export async function getProxy(name: string): Promise<Proxy> {
  const r = await client.get<Proxy>(`/proxies/${encodeURIComponent(name)}`)
  return r.data
}

/**
 * Switch the current node of a Selector / URLTest / LoadBalance group.
 *
 * mihomo v1.19.30 returns a 200 OK with an *empty body* on PUT (older versions
 * echoed the updated Proxy). To keep callers honest we always re-fetch the
 * group afterwards and return the authoritative state.
 */
export async function selectProxy(group: string, name: string): Promise<Proxy> {
  await client.put(`/proxies/${encodeURIComponent(group)}`, { name })
  return await getProxy(group)
}

/**
 * Measure RTT to `url` using the given proxy/node. Default URL is the
 * connectivity-check probe used by most Clash forks.
 */
export async function getProxyDelay(
  name: string,
  url = 'http://www.gstatic.com/generate_204',
  timeoutMs = 5_000,
): Promise<number> {
  const r = await client.get<ProxyDelayResponse>(
    `/proxies/${encodeURIComponent(name)}/delay`,
    { params: { url, timeout: timeoutMs } },
  )
  return r.data.delay
}

// ============================================================================
// Connections
// ============================================================================

export async function getConnections(): Promise<ConnectionsResponse> {
  const r = await client.get<ConnectionsResponse>('/connections')
  return r.data
}

export async function closeAllConnections(): Promise<void> {
  await client.delete('/connections')
}

/**
 * Close a single connection. Mihomo 1.18+ supports `DELETE /connections/{id}`
 * natively; if the build rejects the path with 404 we surface the error so
 * the UI can offer a "close all" fallback instead of silently no-oping.
 */
export async function closeConnection(id: string): Promise<void> {
  await client.delete(`/connections/${encodeURIComponent(id)}`)
}

// ============================================================================
// Rules
// ============================================================================

export async function getRules(): Promise<RulesResponse> {
  const r = await client.get<RulesResponse>('/rules')
  return r.data
}

// ============================================================================
// WebSockets — kept here so the store layer doesn't need to know URLs
// ============================================================================

export interface DisposableSocket {
  close(): void
}

/** Subscribe to `/traffic` push messages (≈1/sec). */
export function openTrafficSocket(
  onSample: (s: TrafficSample) => void,
  onError?: (e: Event) => void,
): DisposableSocket {
  const ws = new WebSocket(`${MIHOMO_WS_URL}/traffic`)
  ws.onmessage = (e) => {
    try {
      onSample(JSON.parse(e.data) as TrafficSample)
    } catch {
      /* malformed payload — ignore */
    }
  }
  ws.onerror = onError ?? (() => {})
  return { close: () => ws.close() }
}

/** Subscribe to `/connections` push messages. */
export function openConnectionsSocket(
  onSnapshot: (c: ConnectionsResponse) => void,
  onError?: (e: Event) => void,
): DisposableSocket {
  const ws = new WebSocket(`${MIHOMO_WS_URL}/connections`)
  ws.onmessage = (e) => {
    try {
      onSnapshot(JSON.parse(e.data) as ConnectionsResponse)
    } catch {
      /* ignore */
    }
  }
  ws.onerror = onError ?? (() => {})
  return { close: () => ws.close() }
}

export { client as axios }

// ============================================================================
// Retry helper
// ============================================================================

/**
 * Poll an async predicate until it resolves truthy or the attempt budget
 * is exhausted. Used by `stores/kernel.ts#probeWithBackoff` to ride out
 * mihomo's ~1-2s cold-start window without an "unreachable" flash on
 * every dashboard load.
 *
 *   await pollUntil(
 *     () => isAlive(),
 *     { intervalMs: 500, maxAttempts: 10, label: 'mihomo /version' },
 *   )
 *
 * Returns the resolved value of the LAST attempt (true/false) plus the
 * number of attempts taken — the store can use this to decide whether
 * to surface a "still unreachable" notice to the user.
 */
export interface PollResult<T> {
  ok: boolean
  value: T | null
  attempts: number
  lastError: Error | null
}

export async function pollUntil<T>(
  fn: () => Promise<T | null | undefined | false>,
  opts: { intervalMs: number; maxAttempts: number; label?: string },
): Promise<PollResult<T>> {
  const label = opts.label ?? 'probe'
  let lastError: Error | null = null
  for (let attempt = 1; attempt <= opts.maxAttempts; attempt += 1) {
    try {
      const v = await fn()
      if (v) {
        // eslint-disable-next-line no-console
        console.debug(`[${label}] ok after ${attempt}/${opts.maxAttempts} attempt(s)`)
        return { ok: true, value: v as T, attempts: attempt, lastError: null }
      }
    } catch (e) {
      lastError = e instanceof Error ? e : new Error(String(e))
    }
    if (attempt < opts.maxAttempts) {
      await new Promise((r) => setTimeout(r, opts.intervalMs))
    }
  }
  return { ok: false, value: null, attempts: opts.maxAttempts, lastError }
}

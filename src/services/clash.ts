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
  timeout: 5_000,
  headers: { 'Content-Type': 'application/json' },
})

/**
 * Translate transport-level failures into messages a human can act on.
 * Mihomo itself uses 401 for secret mismatch and 404 for unknown proxy names.
 */
function toUserError(err: unknown): Error {
  if (axios.isAxiosError(err)) {
    const ax = err as AxiosError
    if (ax.code === 'ERR_NETWORK' || ax.code === 'ECONNREFUSED') {
      return new Error('mihomo not reachable — is the kernel running?')
    }
    if (ax.code === 'ECONNABORTED') {
      return new Error('mihomo request timed out')
    }
    if (ax.response?.status === 401) {
      return new Error('mihomo secret mismatch (check config)')
    }
    if (ax.response?.status === 404) {
      return new Error('mihomo: resource not found')
    }
    return new Error(`mihomo HTTP ${ax.response?.status ?? '?'}: ${ax.message}`)
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
 * 2s. Used by the kernel store's reactive `probeStatus`.
 */
export async function isAlive(): Promise<boolean> {
  try {
    await client.get<MihomoVersion>('/version', { timeout: 2_000 })
    return true
  } catch {
    return false
  }
}

export async function getVersion(): Promise<MihomoVersion> {
  const r = await client.get<MihomoVersion>('/version')
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

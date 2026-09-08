// ============================================================================
// clash.ts — Renderer-side Mihomo client (Phase R1: IPC façade).
//
// RESPONSIBILITIES CHANGED (Phase R1):
//   * Every Mihomo REST call is now a Tauri command invocation. The Rust
//     side (`commands/mihomo.rs`) talks to the controller at
//     http://127.0.0.1:9091 and returns serde_json values. Components and
//     stores no longer touch that URL.
//   * Only the push WebSocket streams (/traffic, /connections) stay on the
//     renderer — a stream cannot ride a request/response IPC.
//
// All exported signatures are unchanged from the pre-R1 axios client, so
// stores (`kernel`, `proxies`, `connections`, `profiles`, …) keep their
// parsing/mapping logic untouched — only the transport was swapped.
// ============================================================================

import { safeInvokeOr } from '@/utils/tauri-bridge'
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

// Kept for informational / WS URL purposes and any UI that displays the
// controller address. All REST traffic now flows through Rust commands.
export const MIHOMO_BASE_URL = 'http://127.0.0.1:9091'
export const MIHOMO_WS_URL = 'ws://127.0.0.1:9091'

/** Normalise whatever `invoke` rejects with into a real Error. */
function wrapErr(err: unknown): Error {
  if (err instanceof Error) return err
  return new Error(typeof err === 'string' ? err : String(err))
}

// ============================================================================
// Health
// ============================================================================

/** Lightweight liveness probe (never throws). */
export async function isAlive(timeoutMs = 2_000): Promise<boolean> {
  try {
    await safeInvokeOr('get_mihomo_version', null)
    return true
  } catch {
    return false
  }
}

export async function getVersion(timeoutMs = 2_000): Promise<MihomoVersion> {
  try {
    const v = await safeInvokeOr<unknown>('get_mihomo_version', null)
    return v as unknown as MihomoVersion
  } catch (e) {
    throw wrapErr(e)
  }
}

// ============================================================================
// Configs (GET / PATCH / PUT) — all via Rust façade
// ============================================================================

export async function getConfigs(): Promise<Config> {
  try {
    const v = await safeInvokeOr<unknown>('get_mihomo_configs', null)
    return v as unknown as Config
  } catch (e) {
    throw wrapErr(e)
  }
}

/** PATCH /configs — outbound mode switch, etc. */
export async function patchConfigs(patch: Partial<Config>): Promise<void> {
  await safeInvokeOr<void>('patch_mihomo_config', undefined, { payload: patch })
}

/**
 * PUT /configs?force=true — ask mihomo to reload its current on-disk
 * config (no body variant).
 */
export async function reloadConfigs(force = true): Promise<void> {
  await safeInvokeOr<void>('reload_mihomo_config', undefined, {
    path: null,
    force,
  })
}

/**
 * PUT /configs?force=true with body `{ path }` — hot-load a specific
 * profile yaml as the active config.
 */
export async function reloadConfig(path: string, force = true): Promise<void> {
  await safeInvokeOr<void>('reload_mihomo_config', undefined, {
    path,
    force,
  })
}

/** Outbound mode: 'rule' | 'global' | 'direct'. */
export async function setMode(
  mode: 'rule' | 'global' | 'direct',
): Promise<void> {
  await patchConfigs({ mode } as Partial<Config>)
}

// ============================================================================
// Proxies
// ============================================================================

export async function getProxies(): Promise<ProxiesResponse> {
  try {
    const v = await safeInvokeOr<unknown>('get_mihomo_proxies', null)
    return v as unknown as ProxiesResponse
  } catch (e) {
    throw wrapErr(e)
  }
}

export async function getProxy(name: string): Promise<Proxy> {
  try {
    const v = await safeInvokeOr<unknown>('get_mihomo_proxy', null, { name })
    return v as unknown as Proxy
  } catch (e) {
    throw wrapErr(e)
  }
}

/**
 * Switch the active node of a Selector / URLTest / LoadBalance group.
 * Returns the refreshed group object.
 */
export async function selectProxy(group: string, name: string): Promise<Proxy> {
  try {
    const v = await safeInvokeOr<unknown>('select_mihomo_proxy', null, {
      group,
      proxy: name,
    })
    return v as unknown as Proxy
  } catch (e) {
    throw wrapErr(e)
  }
}

/**
 * Measure RTT to `url` through `name`. Default URL mirrors Clash forks.
 */
export async function getProxyDelay(
  name: string,
  url = 'http://www.gstatic.com/generate_204',
  timeoutMs = 5_000,
): Promise<number> {
  try {
    const v = await safeInvokeOr<ProxyDelayResponse | null>(
      'get_mihomo_proxy_delay',
      null,
      { name, url, timeoutMs },
    )
    return (v?.delay ?? 0) as number
  } catch (e) {
    throw wrapErr(e)
  }
}

// ============================================================================
// Connections
// ============================================================================

export async function getConnections(): Promise<ConnectionsResponse> {
  try {
    const v = await safeInvokeOr<unknown>('get_mihomo_connections', null)
    return v as unknown as ConnectionsResponse
  } catch (e) {
    throw wrapErr(e)
  }
}

export async function closeAllConnections(): Promise<void> {
  await safeInvokeOr<void>('close_all_mihomo_connections', undefined)
}

export async function closeConnection(id: string): Promise<void> {
  await safeInvokeOr<void>('close_mihomo_connection', undefined, { id })
}

// ============================================================================
// Rules
// ============================================================================

export async function getRules(): Promise<RulesResponse> {
  try {
    const v = await safeInvokeOr<unknown>('get_mihomo_rules', null)
    return v as unknown as RulesResponse
  } catch (e) {
    throw wrapErr(e)
  }
}

// ============================================================================
// WebSockets — push streams CANNOT ride IPC; kept renderer-side.
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

// ============================================================================
// Retry helper (pure logic — unchanged)
// ============================================================================

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

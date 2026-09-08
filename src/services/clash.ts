// ============================================================================
// clash.ts — Renderer-side Mihomo client (Phase R2: Specta typed bindings).
//
// All Mihomo REST calls go through the typed client generated from the Rust
// façade by tauri-specta (see `src/bindings.ts`). The Rust commands return
// the raw JSON body as a string; we parse here and hand the object to the
// store — store parsing/mapping logic is unchanged.
//
// Push WebSocket streams (/traffic, /connections) stay renderer-side.
// ============================================================================

import { commands, type AppError } from '@/bindings'
import { isTauri, NotInTauriError } from '@/utils/tauri-bridge'
import type {
  Config,
  ConnectionsResponse,
  MihomoVersion,
  ProxiesResponse,
  Proxy,
  ProxyDelayResponse,
  RulesResponse,
  TrafficSample,
} from '@/types/clash'

// Kept for informational / WS URL purposes. All REST flows via Rust.
export const MIHOMO_BASE_URL = 'http://127.0.0.1:9091'
export const MIHOMO_WS_URL = 'ws://127.0.0.1:9091'

type CmdRes<T> =
  | { status: 'ok'; data: T }
  | { status: 'error'; error: AppError }

function errText(e: AppError): string {
  if (typeof e === 'string') return e
  return JSON.stringify(e)
}

/** Unwrap a typed command result; throw a readable Error on `error`. */
function ok<T>(r: CmdRes<T>): T {
  if (r.status === 'ok') return r.data
  throw new Error(errText(r.error))
}

/** Invoke a command returning a JSON string; parse it to an object. */
async function getJson(
  p: Promise<CmdRes<string>>,
): Promise<unknown> {
  const raw = ok(await p)
  if (!raw) return null
  try {
    return JSON.parse(raw) as unknown
  } catch {
    return null
  }
}

/** Browser-preview guard: throw the same typed error the safe wrapper used. */
function guardTauri(cmd: string): void {
  if (!isTauri()) throw new NotInTauriError(cmd)
}

// ============================================================================
// Health
// ============================================================================

/** Lightweight liveness probe (never throws). */
export async function isAlive(timeoutMs = 2_000): Promise<boolean> {
  try {
    if (!isTauri()) return false
    const r = await commands.getMihomoVersion()
    return r.status === 'ok'
  } catch {
    return false
  }
}

export async function getVersion(timeoutMs = 2_000): Promise<MihomoVersion> {
  guardTauri('get_mihomo_version')
  const v = await getJson(commands.getMihomoVersion())
  return v as unknown as MihomoVersion
}

// ============================================================================
// Configs
// ============================================================================

export async function getConfigs(): Promise<Config> {
  guardTauri('get_mihomo_configs')
  const v = await getJson(commands.getMihomoConfigs())
  return v as unknown as Config
}

/** PATCH /configs — outbound mode switch. Only `mode` is wired today. */
export async function patchConfigs(patch: Partial<Config>): Promise<void> {
  guardTauri('patch_mihomo_config')
  const mode = String((patch as { mode?: string }).mode ?? 'rule')
  const r = await commands.patchMihomoConfig(mode)
  ok(r)
}

/** PUT /configs?force=true — reload current on-disk config. */
export async function reloadConfigs(force = true): Promise<void> {
  guardTauri('reload_mihomo_config')
  const r = await commands.reloadMihomoConfig(null, force)
  ok(r)
}

/** PUT /configs?force=true { path } — hot-load a specific profile yaml. */
export async function reloadConfig(path: string, force = true): Promise<void> {
  guardTauri('reload_mihomo_config')
  const r = await commands.reloadMihomoConfig(path, force)
  ok(r)
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
  guardTauri('get_mihomo_proxies')
  const v = await getJson(commands.getMihomoProxies())
  return v as unknown as ProxiesResponse
}

export async function getProxy(name: string): Promise<Proxy> {
  guardTauri('get_mihomo_proxy')
  const v = await getJson(commands.getMihomoProxy(name))
  return v as unknown as Proxy
}

/** Switch the active node of a Selector group; returns refreshed group. */
export async function selectProxy(group: string, name: string): Promise<Proxy> {
  guardTauri('select_mihomo_proxy')
  const v = await getJson(commands.selectMihomoProxy(group, name))
  return v as unknown as Proxy
}

export async function getProxyDelay(
  name: string,
  url = 'http://www.gstatic.com/generate_204',
  timeoutMs = 5_000,
): Promise<number> {
  guardTauri('get_mihomo_proxy_delay')
  const v = await getJson(commands.getMihomoProxyDelay(name, url, timeoutMs))
  return ((v as ProxyDelayResponse | null)?.delay ?? 0) as number
}

// ============================================================================
// Connections
// ============================================================================

export async function getConnections(): Promise<ConnectionsResponse> {
  guardTauri('get_mihomo_connections')
  const v = await getJson(commands.getMihomoConnections())
  return v as unknown as ConnectionsResponse
}

export async function closeAllConnections(): Promise<void> {
  guardTauri('close_all_mihomo_connections')
  ok(await commands.closeAllMihomoConnections())
}

export async function closeConnection(id: string): Promise<void> {
  guardTauri('close_mihomo_connection')
  ok(await commands.closeMihomoConnection(id))
}

// ============================================================================
// Rules
// ============================================================================

export async function getRules(): Promise<RulesResponse> {
  guardTauri('get_mihomo_rules')
  const v = await getJson(commands.getMihomoRules())
  return v as unknown as RulesResponse
}

// ============================================================================
// WebSockets — push streams cannot ride IPC; kept renderer-side.
// ============================================================================

export interface DisposableSocket {
  close(): void
}

export function openTrafficSocket(
  onSample: (s: TrafficSample) => void,
  onError?: (e: Event) => void,
): DisposableSocket {
  const ws = new WebSocket(`${MIHOMO_WS_URL}/traffic`)
  ws.onmessage = (e) => {
    try { onSample(JSON.parse(e.data) as TrafficSample) } catch { /* ignore */ }
  }
  ws.onerror = onError ?? (() => {})
  return { close: () => ws.close() }
}

export function openConnectionsSocket(
  onSnapshot: (c: ConnectionsResponse) => void,
  onError?: (e: Event) => void,
): DisposableSocket {
  const ws = new WebSocket(`${MIHOMO_WS_URL}/connections`)
  ws.onmessage = (e) => {
    try { onSnapshot(JSON.parse(e.data) as ConnectionsResponse) } catch { /* ignore */ }
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

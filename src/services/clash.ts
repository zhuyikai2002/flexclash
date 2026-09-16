// ============================================================================
// clash.ts — Renderer-side Mihomo client (Phase R2: Specta typed bindings).
//
// All Mihomo REST calls go through the typed client generated from the Rust
// façade by tauri-specta (see `src/bindings.ts`). The Rust commands return
// the raw JSON body as a string; we parse here and hand the object to the
// store — store parsing/mapping logic is unchanged.
//
// There is no WebSocket code here any more. The push streams (`/traffic`,
// `/logs`) are owned by Rust (`core::ingest`) and reach the renderer as typed
// events; the two renderer-side socket helpers that used to live at the bottom
// of this file (`openTrafficSocket`, `openConnectionsSocket`) were dead or
// superseded and have been removed.
// ============================================================================

import { commands } from '@/bindings'
import { guardInTauri, isTauri, unwrap, type CmdResult } from '@/utils/tauri-bridge'
import type {
  Config,
  ConnectionsResponse,
  MihomoVersion,
  ProxiesResponse,
  Proxy,
  RulesResponse,
} from '@/types/clash'

// Kept for informational purposes (the pinned controller endpoint is also
// declared Rust-side as `RESERVED_CONTROLLER`). All REST flows via Rust.
export const MIHOMO_BASE_URL = 'http://127.0.0.1:9091'

/** Invoke a command returning a JSON string; parse it to an object.
 *
 *  `unwrap` (from the shared bridge) turns the typed `AppError` channel into a
 *  thrown `Error`; everything below already runs inside try/catch.
 */
async function getJson(p: Promise<CmdResult<string>>): Promise<unknown> {
  const raw = unwrap(await p)
  if (!raw) return null
  try {
    return JSON.parse(raw) as unknown
  } catch {
    return null
  }
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
  guardInTauri('get_mihomo_version')
  const v = await getJson(commands.getMihomoVersion())
  return v as unknown as MihomoVersion
}

// ============================================================================
// Configs
// ============================================================================

export async function getConfigs(): Promise<Config> {
  guardInTauri('get_mihomo_configs')
  const v = await getJson(commands.getMihomoConfigs())
  return v as unknown as Config
}

/** PATCH /configs — outbound mode switch. Only `mode` is wired today. */
export async function patchConfigs(patch: Partial<Config>): Promise<void> {
  guardInTauri('patch_mihomo_config')
  const mode = String((patch as { mode?: string }).mode ?? 'rule')
  const r = await commands.patchMihomoConfig(mode)
  unwrap(r)
}

/** PUT /configs?force=true — reload current on-disk config. */
export async function reloadConfigs(force = true): Promise<void> {
  guardInTauri('reload_mihomo_config')
  const r = await commands.reloadMihomoConfig(null, force)
  unwrap(r)
}

/** PUT /configs?force=true { path } — hot-load a specific profile yaml. */
export async function reloadConfig(path: string, force = true): Promise<void> {
  guardInTauri('reload_mihomo_config')
  const r = await commands.reloadMihomoConfig(path, force)
  unwrap(r)
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
  guardInTauri('get_mihomo_proxies')
  const v = await getJson(commands.getMihomoProxies())
  return v as unknown as ProxiesResponse
}

export async function getProxy(name: string): Promise<Proxy> {
  guardInTauri('get_mihomo_proxy')
  const v = await getJson(commands.getMihomoProxy(name))
  return v as unknown as Proxy
}

/** Switch the active node of a Selector group; returns refreshed group. */
export async function selectProxy(group: string, name: string): Promise<Proxy> {
  guardInTauri('select_mihomo_proxy')
  const v = await getJson(commands.selectMihomoProxy(group, name))
  // Rust re-GETs the group so this is normally the full object; fall back
  // to a minimal shape so callers never dereference `null.now`.
  return (v ?? { name: group, now: name }) as unknown as Proxy
}

// NOTE: `getProxyDelay` used to live here — a single-node
// `GET /proxies/{name}/delay` wrapper that `stores/proxies.ts` fanned out with
// a 6-wide pool in the renderer. v0.3 moved the whole probe loop below the IPC
// line (`src/services/speedtest.ts` → `core/speedtest.rs`), so there is no
// longer a caller: probing one node at a time from the UI is exactly the
// pattern this replaced. The Rust command `get_mihomo_proxy_delay` is still
// registered — it remains a valid primitive — but nothing in the renderer
// drives it.

// ============================================================================
// Connections
// ============================================================================

export async function getConnections(): Promise<ConnectionsResponse> {
  guardInTauri('get_mihomo_connections')
  const v = await getJson(commands.getMihomoConnections())
  return v as unknown as ConnectionsResponse
}

export async function closeAllConnections(): Promise<void> {
  guardInTauri('close_all_mihomo_connections')
  unwrap(await commands.closeAllMihomoConnections())
}

export async function closeConnection(id: string): Promise<void> {
  guardInTauri('close_mihomo_connection')
  unwrap(await commands.closeMihomoConnection(id))
}

// ============================================================================
// Rules
// ============================================================================

export async function getRules(): Promise<RulesResponse> {
  guardInTauri('get_mihomo_rules')
  const v = await getJson(commands.getMihomoRules())
  return v as unknown as RulesResponse
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

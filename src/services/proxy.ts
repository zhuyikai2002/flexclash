// ============================================================================
// services/proxy.ts — Tauri commands for the Windows system proxy (registry).
//
// All real work happens in Rust. Wire types come from the generated bindings.
// ============================================================================

import { commands, type ProxyStatus, type ProxyToggleResult } from '@/bindings'
import { call, guardInTauri, inTauri } from '@/utils/tauri-bridge'

export const SYSTEM_PROXY_CHANGED_EVENT = 'system-proxy://changed'

/** Enable the system proxy, pointing it at `127.0.0.1:<port>` (mihomo's
 *  mixed-port when omitted). Throws `NotInTauriError` outside the Tauri
 *  runtime — this is a write triggered by a user click. */
export async function enableSystemProxy(port?: number): Promise<ProxyToggleResult> {
  guardInTauri('enable_system_proxy')
  return call(commands.enableSystemProxy(port ?? null))
}

export async function disableSystemProxy(): Promise<ProxyToggleResult> {
  guardInTauri('disable_system_proxy')
  return call(commands.disableSystemProxy())
}

/** Read-only probe; returns `null` outside Tauri so the store can boot
 *  with default state in browser preview. */
export async function getSystemProxyStatus(): Promise<ProxyStatus | null> {
  if (!inTauri('get_system_proxy_status')) return null
  return call(commands.getSystemProxyStatus())
}

// ============================================================================
// services/proxy.ts — Tauri commands for the system proxy (Windows registry).
//
// All real work happens in Rust. This module is just a thin invoke wrapper.
// `safeInvoke` is used instead of `invoke` so the renderer can boot in a
// plain browser preview without flooding DevTools with TypeErrors.
// ============================================================================

import { safeInvoke, safeInvokeOr, type InvokeArgs } from '@/utils/tauri-bridge'

export interface ProxyStatus {
  enabled: boolean
  server: string | null
  override: string | null
}

export interface ProxyToggleResult {
  enabled: boolean
  port: number | null
  detail: string
}

export const SYSTEM_PROXY_CHANGED_EVENT = 'system-proxy://changed'

/** Throws `NotInTauriError` outside the Tauri runtime — intended for
 *  write operations triggered by user clicks.  The store/UI layer is
 *  expected to surface a friendly message. */
export async function enableSystemProxy(port?: number): Promise<ProxyToggleResult> {
  const args: InvokeArgs = { port: port ?? null }
  return await safeInvoke<ProxyToggleResult>('enable_system_proxy', args)
}

export async function disableSystemProxy(): Promise<ProxyToggleResult> {
  return await safeInvoke<ProxyToggleResult>('disable_system_proxy')
}

/** Read-only probe; returns `null` outside Tauri so the store can boot
 *  with default state in browser preview. */
export async function getSystemProxyStatus(): Promise<ProxyStatus | null> {
  return await safeInvokeOr<ProxyStatus | null>('get_system_proxy_status', null)
}

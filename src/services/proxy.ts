// ============================================================================
// services/proxy.ts — Tauri commands for the system proxy (Windows registry).
//
// All real work happens in Rust. This module is just a thin invoke wrapper.
// ============================================================================

import { invoke } from '@tauri-apps/api/core'

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

export async function enableSystemProxy(port?: number): Promise<ProxyToggleResult> {
  return await invoke<ProxyToggleResult>('enable_system_proxy', { port: port ?? null })
}

export async function disableSystemProxy(): Promise<ProxyToggleResult> {
  return await invoke<ProxyToggleResult>('disable_system_proxy')
}

export async function getSystemProxyStatus(): Promise<ProxyStatus | null> {
  return await invoke<ProxyStatus | null>('get_system_proxy_status')
}

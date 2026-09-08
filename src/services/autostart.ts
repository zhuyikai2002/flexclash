// ============================================================================
// services/autostart.ts — Tauri command surface for desktop integration (M7).
//
// All real work happens in Rust (commands/desktop.rs). The plugin's
// `tauri-plugin-autostart` is wrapped here to keep the rest of the
// frontend off the plugin-internal API.  `safeInvoke` is used so the
// store can boot in a plain browser preview without DevTools errors.
// ============================================================================

import { safeInvoke, safeInvokeOr } from '@/utils/tauri-bridge'

export interface AutostartStatus {
  enabled: boolean
  /** True when the binary was launched with `--silent` (autostart at boot). */
  silent: boolean
}

export interface SweepResult {
  deleted: number
  ok: boolean
}

const DEFAULT_STATUS: AutostartStatus = { enabled: false, silent: false }

export async function getAutostartStatus(): Promise<AutostartStatus> {
  return await safeInvokeOr<AutostartStatus>('get_autostart_status', DEFAULT_STATUS)
}

export async function setAutostart(enabled: boolean): Promise<AutostartStatus> {
  return await safeInvoke<AutostartStatus>('set_autostart', { enabled })
}

export async function getSilentFlag(): Promise<boolean> {
  return await safeInvokeOr<boolean>('get_silent_flag', false)
}

export async function sweepResidualRoutes(): Promise<SweepResult> {
  return await safeInvoke<SweepResult>('sweep_residual_routes')
}

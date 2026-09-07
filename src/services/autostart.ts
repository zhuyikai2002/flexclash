// ============================================================================
// services/autostart.ts — Tauri command surface for desktop integration (M7).
//
// All real work happens in Rust (commands/desktop.rs). The plugin's
// `tauri-plugin-autostart` is wrapped here to keep the rest of the
// frontend off the plugin-internal API.
// ============================================================================

import { invoke } from '@tauri-apps/api/core'

export interface AutostartStatus {
  enabled: boolean
  /** True when the binary was launched with `--silent` (autostart at boot). */
  silent: boolean
}

export interface SweepResult {
  deleted: number
  ok: boolean
}

export async function getAutostartStatus(): Promise<AutostartStatus> {
  return await invoke<AutostartStatus>('get_autostart_status')
}

export async function setAutostart(enabled: boolean): Promise<AutostartStatus> {
  return await invoke<AutostartStatus>('set_autostart', { enabled })
}

export async function getSilentFlag(): Promise<boolean> {
  return await invoke<boolean>('get_silent_flag')
}

export async function sweepResidualRoutes(): Promise<SweepResult> {
  return await invoke<SweepResult>('sweep_residual_routes')
}

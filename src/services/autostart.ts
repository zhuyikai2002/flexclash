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

/**
 * Elevated (Task Scheduler) autostart.
 *
 * Distinct from the registry mechanism above: the scheduled task launches
 * the app with a full administrator token, so TUN mode works without a UAC
 * prompt on every toggle. The two are mutually exclusive in the backend --
 * enabling one clears the other, because two logon launches fight over the
 * reserved inbound port.
 */
export interface SilentAutostartStatus {
  /** The logon task is registered in Task Scheduler. */
  enabled: boolean
  /** False where the platform cannot support it; the UI hides the switch. */
  available: boolean
  /** The registry entry is the active mechanism instead. */
  registry_active: boolean
}

const DEFAULT_STATUS: AutostartStatus = { enabled: false, silent: false }

const DEFAULT_SILENT_STATUS: SilentAutostartStatus = {
  enabled: false,
  available: false,
  registry_active: false,
}

export async function getAutostartStatus(): Promise<AutostartStatus> {
  return await safeInvokeOr<AutostartStatus>('get_autostart_status', DEFAULT_STATUS)
}

export async function setAutostart(enabled: boolean): Promise<AutostartStatus> {
  return await safeInvoke<AutostartStatus>('set_autostart', { enabled })
}

export async function getSilentAutostartStatus(): Promise<SilentAutostartStatus> {
  return await safeInvokeOr<SilentAutostartStatus>(
    'get_silent_autostart_status',
    DEFAULT_SILENT_STATUS,
  )
}

/**
 * Enable/disable the elevated logon task.
 *
 * Enabling raises a UAC prompt (creating a `HighestAvailable` task requires
 * administrator rights). Rejects with a message mentioning cancellation
 * when the user dismisses it.
 */
export async function setSilentAutostart(
  enabled: boolean,
): Promise<SilentAutostartStatus> {
  return await safeInvoke<SilentAutostartStatus>('set_silent_autostart', { enabled })
}

export async function getSilentFlag(): Promise<boolean> {
  return await safeInvokeOr<boolean>('get_silent_flag', false)
}

export async function sweepResidualRoutes(): Promise<SweepResult> {
  return await safeInvoke<SweepResult>('sweep_residual_routes')
}

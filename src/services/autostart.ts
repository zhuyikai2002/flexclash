// ============================================================================
// services/autostart.ts — Tauri command surface for desktop integration.
//
// All real work happens in Rust (commands/desktop.rs). The plugin's
// `tauri-plugin-autostart` is wrapped there, so the renderer never touches
// the plugin's own API.
//
// Wire types are the *generated* ones (`src/bindings.ts`), so a change to a
// Rust DTO is a compile error here instead of a silent drift.
//
// Note the two command flavours the generated bindings expose:
//   * `call(commands.x(...))` — the command returns `Result<T, AppError>`, so
//     the error is carried on the typed channel and `call` throws it;
//   * `await commands.x()`     — the command is infallible on the Rust side
//     (`-> T`), so there is nothing to unwrap; it only rejects on a transport
//     failure. `get_autostart_status`, `get_silent_autostart_status`,
//     `get_silent_flag` and `sweep_residual_routes` are all of this kind.
// ============================================================================

import {
  commands,
  type AutostartStatus,
  type SilentAutostartStatus,
  type SweepResult,
} from '@/bindings'
import { call, guardInTauri, inTauri } from '@/utils/tauri-bridge'

const DEFAULT_STATUS: AutostartStatus = { enabled: false, silent: false }

const DEFAULT_SILENT_STATUS: SilentAutostartStatus = {
  enabled: false,
  available: false,
  registry_active: false,
}

export async function getAutostartStatus(): Promise<AutostartStatus> {
  if (!inTauri('get_autostart_status')) return DEFAULT_STATUS
  return await commands.getAutostartStatus()
}

export async function setAutostart(enabled: boolean): Promise<AutostartStatus> {
  guardInTauri('set_autostart')
  return call(commands.setAutostart(enabled))
}

export async function getSilentAutostartStatus(): Promise<SilentAutostartStatus> {
  if (!inTauri('get_silent_autostart_status')) return DEFAULT_SILENT_STATUS
  return await commands.getSilentAutostartStatus()
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
  guardInTauri('set_silent_autostart')
  return call(commands.setSilentAutostart(enabled))
}

export async function getSilentFlag(): Promise<boolean> {
  if (!inTauri('get_silent_flag')) return false
  return await commands.getSilentFlag()
}

export async function sweepResidualRoutes(): Promise<SweepResult> {
  guardInTauri('sweep_residual_routes')
  return await commands.sweepResidualRoutes()
}

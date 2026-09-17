// ============================================================================
// services/deadlink.ts — Tauri command surface for the dead-link autopilot.
//
// All policy lives in Rust (`core/deadlink.rs`); this file is only the typed
// door to its kill switch.
// ============================================================================

import { commands, type AutokillState } from '@/bindings'
import { guardInTauri, inTauri } from '@/utils/tauri-bridge'

/**
 * What the renderer assumes before Rust has answered — and what a plain
 * browser preview keeps forever.
 *
 * `enabled: true` matches the Rust default (`FLEXCLASH_AUTOKILL` unset), so
 * the toggle never lies by showing "off" for an autopilot that is in fact
 * armed. `envLocked: false` is the honest answer for a process we know
 * nothing about: preview mode has no environment to read.
 */
const FALLBACK_STATE: AutokillState = { enabled: true, envLocked: false }

/**
 * Read the current state of the kill switch.
 *
 * Both commands are declared `-> AutokillState` on the Rust side, i.e. the
 * *infallible* shape from `tauri-bridge`'s point of view: there is no typed
 * error channel to unwrap, so `commands.*` is awaited directly rather than
 * through `call()` (which only accepts `Result`-returning commands). The only
 * failure left is a transport one, and that rejects — which the store turns
 * into `lastError` exactly like any other.
 */
export async function getAutokillState(): Promise<AutokillState> {
  if (!inTauri('get_autokill_state')) return { ...FALLBACK_STATE }
  return await commands.getAutokillState()
}

/**
 * Flip the runtime override.
 *
 * Not guarded by `inTauri` on purpose: this is a write, and a write that
 * silently no-ops in preview would leave the UI showing a state nobody set.
 * `guardInTauri` raises instead, which the store turns into `lastError`.
 */
export async function setAutokill(enabled: boolean): Promise<AutokillState> {
  guardInTauri('set_autokill')
  return await commands.setAutokill(enabled)
}

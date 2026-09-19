// ============================================================================
// services/desktop.ts — Tauri command surface for desktop window behaviour.
//
// A mirror of one piece of state that lives in Rust for the duration of the
// session: the close-window preference (Settings → General → On window close).
//
// It is not durable on the Rust side on purpose. The renderer owns
// persistence (localStorage) and replays the value at cold start, which keeps
// one source of truth for "what did the user actually choose".
// ============================================================================

import { commands } from '@/bindings'
import { inTauri } from '@/utils/tauri-bridge'

/** localStorage key. Also read at cold start to replay the choice into Rust. */
export const CLOSE_BEHAVIOR_KEY = 'flexclash.closeBehavior'

/**
 * `minimize` (default) keeps the historical behaviour: the window hides to
 * the tray and the kernel keeps running. `exit` makes a window close final.
 */
export type CloseBehavior = 'minimize' | 'exit'

/**
 * Read the persisted preference. Anything unrecognised — including a first
 * run with no entry at all — resolves to `minimize`, which is what the Rust
 * default also is, so the two sides can never disagree about the fallback.
 */
export function readCloseBehavior(): CloseBehavior {
  if (typeof localStorage === 'undefined') return 'minimize'
  return localStorage.getItem(CLOSE_BEHAVIOR_KEY) === 'exit' ? 'exit' : 'minimize'
}

/**
 * Push the preference to Rust, where the `CloseRequested` hook consumes it.
 *
 * Not guarded by `inTauri` on purpose: this is a write, and a write that
 * silently no-ops would leave the backend on its default while the UI shows
 * "exit". Outside Tauri the call is simply skipped — nothing is being
 * simulated there.
 */
export async function pushCloseBehavior(behavior: CloseBehavior): Promise<void> {
  if (!inTauri('set_close_behavior')) return
  // Declared `-> bool` in Rust, i.e. the infallible shape, so it is awaited
  // directly rather than through `call()` (which only accepts `Result`s).
  await commands.setCloseBehavior(behavior === 'exit')
}

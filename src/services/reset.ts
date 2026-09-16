// ============================================================================
// services/reset.ts — "Reset Application" wrapper.
//
// The heavy lifting (kill mihomo, wipe work dir, rewrite default config,
// emit `app://reset-completed`) lives in the Rust command
// `commands::reset::reset_application`. This file:
//   - routes the call through the typed-command layer,
//   - attaches a listener for `app://reset-completed` so the frontend can
//     clear localStorage + show a "restarting" toast before exiting.
// ============================================================================

import { commands, type ResetReport } from '@/bindings'
import { call, guardInTauri, safeListen, type UnlistenFn } from '@/utils/tauri-bridge'
import { useProxiesStore } from '@/stores/proxies'
import { useRulesStore } from '@/stores/rules'

/** Invoke the destructive reset on the Rust side. */
export async function resetApplication(): Promise<ResetReport> {
  guardInTauri('reset_application')
  return call(commands.resetApplication())
}

/** Subscribe to the post-reset completion event. The frontend
 *  should use this to (1) clear localStorage, (2) show a "Reset
 *  complete" modal, then (3) call `app.exit(0)` from the UI
 *  thread after a short delay. */
export async function onResetCompleted(
  cb: (report: ResetReport) => void,
): Promise<UnlistenFn> {
  return safeListen<ResetReport>('app://reset-completed', (e) => cb(e.payload))
}

/** Drop every piece of client-side state that could survive the reset.
 *
 *  The Rust side has already stopped the kernel, replaced `config.yaml` with
 *  the bundled default, and deleted `profiles/` + `cache.db`. Anything still
 *  rendered after that point is a memory-only leftover — which is exactly the
 *  "my old proxy groups are still there after a reset" symptom.
 *
 *  Call this from the `app://reset-completed` handler, before the restart.
 */
export function clearClientState(): void {
  // 1. localStorage — a full clear, not the old `flexclash.*` prefix sweep.
  //    This webview origin belongs to the app alone, and a reset that leaves
  //    an unrecognised key behind is the very bug being fixed.
  try {
    localStorage.clear()
  } catch {
    /* private mode / storage unavailable — nothing to clear */
  }

  // 2. In-memory stores. `proxies` holds the group/node tree and the delay
  //    measurements fetched from the old kernel; `rules` holds its rule list.
  //    Pinia only synthesises `$reset()` for option stores, so a store that
  //    lacks it must not abort the restart — hence the guard.
  for (const store of [useProxiesStore(), useRulesStore()]) {
    const reset = (store as unknown as { $reset?: () => void }).$reset
    if (typeof reset === 'function') {
      try {
        reset.call(store)
      } catch {
        /* fall back to the process restart, which rebuilds every store */
      }
    }
  }
}

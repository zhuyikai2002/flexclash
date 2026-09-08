// ============================================================================
// services/reset.ts — Phase 8 "Reset Application" wrapper.
//
// The heavy lifting (kill mihomo, wipe work dir, rewrite default config,
// emit `app://reset-completed`) lives in the Rust command
// `commands::reset::reset_application`.  This file is a thin, typed
// wrapper that:
//   - routes through the Tauri IPC chokepoint (`safeInvoke`)
//   - attaches a listener for the `app://reset-completed` event so the
//     frontend can clear localStorage + show a "restarting" toast
//     before invoking `app.exit(0)` from the UI thread.
// ============================================================================

import { safeInvoke, safeListen, type UnlistenFn } from '@/utils/tauri-bridge'

/** Mirrors `commands::reset::ResetReport` (snake_case fields, Rust
 *  → JSON keeps the names as-is). */
export interface ResetReport {
  removed_profiles: number
  removed_history_bytes: number
  swept_routes: boolean
  proxy_disabled: boolean
  kernel_was_running: boolean
  tun_was_on: boolean
  detail: string
}

/** Invoke the destructive reset on the Rust side. */
export async function resetApplication(): Promise<ResetReport> {
  return safeInvoke<ResetReport>('reset_application')
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

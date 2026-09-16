// ============================================================================
// services/updater.ts — Tauri command surface for the in-app updater.
//
// The real work lives in Rust (`commands/updater.rs`): `check_update` reads
// the feed, `download_update` downloads + signature-verifies and parks the
// bytes in managed state, and `install_update` tears the sidecar down and
// launches the installer. Progress streams over the raw `updater://progress`
// event with a *cumulative* `{ downloaded, total }`.
//
// Wire types come from the generated bindings; every call is routed through
// the single `call`/`safeListen` bridge so nothing here touches the raw Tauri
// API (and a plain browser preview fails soft instead of throwing).
// ============================================================================

import { commands, type UpdateInfo } from '@/bindings'
import { call, safeListen, type UnlistenFn } from '@/utils/tauri-bridge'

/** Payload of the `updater://progress` event. `total` is null until the
 *  server reports a content length. */
export interface UpdateProgress {
  downloaded: number
  total: number | null
}

/** Check the feed. `null` means "already on the latest version". */
export function checkUpdate(): Promise<UpdateInfo | null> {
  return call(commands.checkUpdate())
}

/** Download (and verify) the latest update without installing. Resolves with
 *  the update descriptor once the bytes are safely parked on the Rust side. */
export function downloadUpdate(): Promise<UpdateInfo> {
  return call(commands.downloadUpdate())
}

/** Install the already-downloaded update. On Windows this launches the NSIS
 *  installer and exits the app, so it usually never resolves. The Rust
 *  command returns `()` (which specta serialises to `null`); we discard it. */
export async function installUpdate(): Promise<void> {
  await call(commands.installUpdate())
}

/** Subscribe to live download progress. Returns the unlisten function. */
export function listenProgress(handler: (p: UpdateProgress) => void): Promise<UnlistenFn> {
  return safeListen<{ downloaded?: number; total?: number | null }>(
    'updater://progress',
    (e) => {
      handler({
        downloaded: e.payload.downloaded ?? 0,
        total: e.payload.total ?? null,
      })
    },
  )
}

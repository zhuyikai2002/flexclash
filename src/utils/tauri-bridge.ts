// ============================================================================
// utils/tauri-bridge.ts — Single chokepoint for Tauri IPC in the renderer.
//
// Why this file exists
// --------------------
// `@tauri-apps/api/core#invoke` and `@tauri-apps/api/event#listen` look up
// the runtime in `window.__TAURI_INTERNALS__`. That global is injected by
// the Tauri webview at boot, so it is **only present when the app is
// running inside `flexclash.exe` (or `npm run tauri:dev`)**.
//
// In *plain browser preview* (`npm run dev` → http://localhost:5173) the
// global is `undefined`. Every bare `invoke(...)` then throws
// `TypeError: Cannot read properties of undefined (reading 'transformCallback')`
// before our own try/catch can record a useful message, flooding the
// DevTools console and obscuring the real error (if any) from the
// Vue runtime handler.
//
// Strategy
// --------
// We wrap both surfaces once, here, and direct all module-level imports
// at the wrapper. The wrapper:
//   * probes `window.__TAURI_INTERNALS__` / `window.__TAURI__` on every
//     call (cheap, just an `in` check),
//   * logs once per call when the runtime is missing (DEV-only — keeps
//     prod noise at zero),
//   * throws a typed `NotInTauriError` for write operations, so the
//     click handler can surface a friendly "preview mode" hint instead
//     of a stack-trace dump,
//   * returns a no-op unsubscribe for `safeListen` so event subscriptions
//     can be set up in the same `onMounted` code path used in production
//     without guarding every call site.
//
// The prod path inside Tauri is a single `await invoke(...)` — no extra
// branch, no perf cost.  The dev / browser-preview path is the one that
// actually saves a console-spam storm.
// ============================================================================

import { invoke, type InvokeArgs } from '@tauri-apps/api/core'
import { listen, type EventCallback, type UnlistenFn } from '@tauri-apps/api/event'
import { getVersion } from '@tauri-apps/api/app'

/** True iff the renderer is hosted inside a Tauri webview. */
export function isTauri(): boolean {
  if (typeof window === 'undefined') return false
  // Tauri 2.x injects both `__TAURI_INTERNALS__` (the actual IPC bridge)
  // and `__TAURI__` (the legacy compat shim). Either is sufficient.
  return '__TAURI_INTERNALS__' in window || '__TAURI__' in window
}

/** Thrown by `safeInvoke` when the renderer is not in a Tauri context. */
export class NotInTauriError extends Error {
  readonly cmd: string
  constructor(cmd: string) {
    super(
      `[tauri-bridge] invoke('${cmd}') called outside the Tauri runtime. ` +
        `This is expected when previewing the UI in a plain browser ` +
        `(\`npm run dev\`). Use \`npm run tauri:dev\` for full integration.`,
    )
    this.name = 'NotInTauriError'
    this.cmd = cmd
  }
}

const isDev = typeof import.meta !== 'undefined' && Boolean(import.meta.env?.DEV)

/**
 * Invoke a Tauri command. In the Tauri runtime this is a zero-cost
 * pass-through. In a plain browser it throws `NotInTauriError` so the
 * caller can show a "preview mode" message instead of an opaque
 * `TypeError`.
 */
export async function safeInvoke<T>(cmd: string, args?: InvokeArgs): Promise<T> {
  if (!isTauri()) {
    if (isDev) console.debug(`[tauri-bridge] skip ${cmd} — not in Tauri runtime`)
    throw new NotInTauriError(cmd)
  }
  return invoke<T>(cmd, args)
}

/**
 * Variant of `safeInvoke` for read-only probes: instead of throwing,
 * returns the supplied `fallback` when outside Tauri. Useful for
 * `init()` / `refresh()` flows that should silently no-op in browser
 * preview so the UI can still render with default state.
 */
export async function safeInvokeOr<T>(cmd: string, fallback: T, args?: InvokeArgs): Promise<T> {
  if (!isTauri()) {
    if (isDev) console.debug(`[tauri-bridge] skip ${cmd} — using fallback`)
    return fallback
  }
  return invoke<T>(cmd, args)
}

/**
 * Subscribe to a Tauri event. In the Tauri runtime this is a zero-cost
 * pass-through. Outside Tauri, returns a no-op unsubscribe so the call
 * site can still `await` it inside `onMounted` and the component's
 * `dispose()` path can call it safely.
 */
export async function safeListen<T>(
  event: string,
  handler: EventCallback<T>,
): Promise<UnlistenFn> {
  if (!isTauri()) {
    if (isDev) console.debug(`[tauri-bridge] skip listen('${event}') — not in Tauri runtime`)
    return () => {
      /* no-op unlisten */
    }
  }
  return listen<T>(event, handler)
}

/** Re-export so call sites that already imported from the Tauri package
 *  can switch in one line.  (We never *use* these in the bridge; they
 *  exist so `services/*` can `import { safeInvoke as invoke }` and
 *  the diff stays small.) */
export { invoke, listen, type UnlistenFn, type EventCallback, type InvokeArgs }

/**
 * Read the version of the running application bundle — i.e. the `version`
 * field of `src-tauri/tauri.conf.json`, baked into the binary at build
 * time. This is the single source of truth for "客户端版本"; never mirror
 * it with a literal, which silently goes stale the moment you bump the
 * manifest (a hard-coded `0.1.2` survived four releases that way).
 *
 * Returns `null` outside the Tauri runtime (plain browser preview) or if
 * the IPC call fails, so callers can render a neutral placeholder instead
 * of inventing a version.
 */
export async function getAppVersion(): Promise<string | null> {
  if (!isTauri()) {
    if (isDev) console.debug('[tauri-bridge] skip getVersion — not in Tauri runtime')
    return null
  }
  try {
    return await getVersion()
  } catch (e) {
    if (isDev) console.debug('[tauri-bridge] getVersion failed', e)
    return null
  }
}

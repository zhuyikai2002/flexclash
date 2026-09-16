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
// Two surfaces live here, and nothing else in the renderer is allowed to
// touch the raw Tauri API:
//
//   * **Events** — `safeListen` wraps `listen` and returns a no-op
//     unsubscribe outside Tauri, so a component can subscribe in the same
//     `onMounted` path in production and preview alike.
//
//   * **Commands** — `unwrap` / `call` / `guardInTauri` / `inTauri` wrap the
//     *generated* `commands` object from `@/bindings`. They are the only
//     place the two shapes a tauri-specta command can have (`Result<T, E>`
//     vs. infallible `T`) are collapsed into the `T`-or-throw contract the
//     stores use. Services must call `commands.*` through them; a bare
//     `invoke('some_name')` is what this file exists to prevent, because a
//     string command name is checked by nothing.
//
// Both probes are cheap (`in` checks, DEV-only logging), so the prod path
// inside Tauri costs one branch and nothing else.
// ============================================================================

import { listen, type EventCallback, type UnlistenFn } from '@tauri-apps/api/event'
import { getVersion } from '@tauri-apps/api/app'
import type { AppError } from '@/bindings'

/** True iff the renderer is hosted inside a Tauri webview. */
export function isTauri(): boolean {
  if (typeof window === 'undefined') return false
  // Tauri 2.x injects both `__TAURI_INTERNALS__` (the actual IPC bridge)
  // and `__TAURI__` (the legacy compat shim). Either is sufficient.
  return '__TAURI_INTERNALS__' in window || '__TAURI__' in window
}

/** Thrown by `guardInTauri` when a write is attempted outside a Tauri context. */
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

/**
 * Subscribe to a *typed* tauri-specta event — an entry of the generated
 * `events` object (`events.trafficPayload`, `events.logBatch`, …).
 *
 * Why not call `events.x.listen` directly: the generated helper is a thin
 * wrapper over the raw `@tauri-apps/api/event` module, which is `undefined`
 * outside the Tauri runtime, so a direct call throws in a plain browser
 * preview — exactly what `safeListen` exists to prevent. Routing both through
 * here keeps the "only this module touches the raw Tauri API" rule intact, and
 * the payload type is inferred from the event rather than restated.
 */
export async function safeListenEvent<T>(
  event: { listen: (cb: EventCallback<T>) => Promise<UnlistenFn> },
  handler: EventCallback<T>,
): Promise<UnlistenFn> {
  if (!isTauri()) {
    if (isDev) console.debug('[tauri-bridge] skip typed-event listen — not in Tauri runtime')
    return () => {
      /* no-op unlisten */
    }
  }
  return event.listen(handler)
}

// ============================================================================
// Typed-command layer (tauri-specta)
// ============================================================================
//
// `src/bindings.ts` is generated from the Rust command surface and exports a
// `commands` object. Each entry is one of exactly two shapes:
//
//   * a `Result<T, AppError>` command — the generated `typedError` wrapper
//     resolves it to `{ status: 'ok', data } | { status: 'error', error }`,
//     so the failure channel is *typed*. A bare `invoke` rejects with
//     `unknown`, which is why this shape is worth carrying;
//   * an infallible command (`-> T`, no `Result`), which resolves straight
//     to `T` and only rejects on a transport failure.
//
// The helpers below are the single place those shapes are collapsed into the
// `T`-or-throw contract every store already expects, so no service repeats the
// unwrap and none of them hand-rolls a `commands.x()` call or its own copy of
// `ok()` / `errText()`.

/** Result of a tauri-specta command declared `-> Result<T, E>`. */
export type CmdResult<T, E = AppError> =
  | { status: 'ok'; data: T }
  | { status: 'error'; error: E }

/**
 * Render a typed-error payload as a message worth showing.
 *
 * `AppError` serialises to a plain string today (see `specta(type = String)`
 * on the Rust enum), but the guard keeps this honest if it ever grows a
 * structured variant.
 */
export function appErrorText(e: unknown): string {
  return typeof e === 'string' ? e : JSON.stringify(e)
}

/**
 * Unwrap a settled typed result, throwing on the error channel.
 *
 * Throwing rather than returning the discriminated union is deliberate: every
 * existing call site is a `try { … } catch (e) { show(e.message) }`, so the
 * union never had to escape this module.
 */
export function unwrap<T, E = AppError>(r: CmdResult<T, E>): T {
  if (r.status === 'ok') return r.data
  throw new Error(appErrorText(r.error))
}

/** Await a typed command and return its `data`, or throw its error. */
export async function call<T, E = AppError>(p: Promise<CmdResult<T, E>>): Promise<T> {
  return unwrap(await p)
}

/**
 * Browser-preview guard for a *write*: throws `NotInTauriError` outside the
 * Tauri runtime so the click handler can surface the "preview mode" hint.
 */
export function guardInTauri(cmd: string): void {
  if (!isTauri()) throw new NotInTauriError(cmd)
}

/**
 * Browser-preview guard for a *read*: returns `false` (and logs once in DEV)
 * outside the Tauri runtime, so the caller falls back to a default and the UI
 * still renders. Reads compose as
 *
 * ```ts
 * if (!inTauri('list_profiles')) return []
 * ```
 */
export function inTauri(cmd: string): boolean {
  if (isTauri()) return true
  if (isDev) console.debug(`[tauri-bridge] skip ${cmd} — not in Tauri runtime`)
  return false
}

/** `UnlistenFn` is re-exported because every store that subscribes to an
 *  event needs the type, and pulling it from `@tauri-apps/api/event` in a
 *  dozen places would let those modules drift back onto the raw API. */
export type { UnlistenFn }

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

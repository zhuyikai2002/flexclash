// ============================================================================
// stores/notices.ts — the renderer's single error channel (v0.6.x Step 5.2).
//
// WHAT WAS WRONG
// --------------
// Eleven stores each owned a private `lastError: string | null` (or `error`),
// plus three components owned a local `ref` with a hand-rolled expiry timer.
// That is fourteen places inventing the same wheel, and the consequences were
// not theoretical:
//
//   * No provenance. A bare string cannot say whether it came from the kernel,
//     the system proxy or a profile import, so nothing could route it.
//   * No severity. Everything was an "error", including things that were only
//     warnings, so nothing could decide how loudly to speak.
//   * No time. Nothing could tell a fresh failure from a week-old one still
//     sitting in a field nobody ever cleared.
//   * Dead ends. `kernel.lastError` and `history.lastError` were written to
//     and *never read by anything at all* — a failure the user could not see.
//
// WHAT THIS IS
// ------------
// One bounded ring of typed notices. Every store keeps its existing public
// field name (`lastError` / `error`) but reimplements it as a *getter* over
// this ring, so no component had to change to adopt the channel.
//
// COALESCING
// ----------
// A poller that fails every two seconds must not evict the ring in a minute.
// An identical (source, severity, message) triple inside `COALESCE_MS` bumps a
// counter on the existing notice instead of appending, which is also strictly
// more informative: "this failed 40 times" is a different bug report from
// "this failed once".
//
// SEVERITY IS NOT PRESENTATION
// ----------------------------
// `raise()` records and nothing else. What to *do* with a notice is a
// presentation decision made in `ToastHost` — see `GLOBAL_SOURCES` there for
// why most errors are deliberately not toasted.
// ============================================================================

import { defineStore } from 'pinia'

/** Where a notice came from. Keep in sync with `GLOBAL_SOURCES` below. */
export type NoticeSource =
  | 'kernel'
  | 'proxy'
  | 'tun'
  | 'desktop'
  /** The elevated (Task Scheduler) autostart — a separate mechanism in the
   *  same store, so it needs its own channel or the two cards would show
   *  each other's failures. */
  | 'desktopTask'
  | 'proxies'
  | 'connections'
  | 'rules'
  | 'profiles'
  | 'history'
  | 'updater'
  | 'autokill'
  | 'geo'

export type NoticeSeverity = 'info' | 'warn' | 'error'

export interface UiNotice {
  id: number
  source: NoticeSource
  severity: NoticeSeverity
  message: string
  /** First time this notice was raised. */
  at: number
  /** Most recent repeat (equals `at` when `count === 1`). */
  lastAt: number
  /** How many times this exact notice has been raised. */
  count: number
}

/** Ring capacity. Oldest evicted first. */
const RING_CAP = 30

/** Repeats inside this window collapse into `count` rather than new entries. */
const COALESCE_MS = 5000

let _seq = 0

export const useNoticesStore = defineStore('notices', {
  state: () => ({
    items: [] as UiNotice[],
  }),

  getters: {
    /** Newest notice, or `null` when the channel is quiet. */
    latest: (s): UiNotice | null => {
      const n = s.items[s.items.length - 1]
      return n ?? null
    },

    /**
     * Newest notice for one source.
     *
     * This is what every store's `lastError` getter is built on: a store only
     * ever sees its own history, so two failing subsystems cannot overwrite
     * each other's message the way independent fields silently allowed.
     */
    latestFor: (s) => (source: NoticeSource): UiNotice | null => {
      for (let i = s.items.length - 1; i >= 0; i -= 1) {
        const n = s.items[i]
        if (n && n.source === source) return n
      }
      return null
    },

    /** Highest severity currently pending — for a global banner. */
    worst: (s): NoticeSeverity | null => {
      let out: NoticeSeverity | null = null
      for (const n of s.items) {
        if (n.severity === 'error') return 'error'
        if (n.severity === 'warn') out = 'warn'
        else if (out === null) out = 'info'
      }
      return out
    },
  },

  actions: {
    /** Shortcut used by every store's `catch` block. */
    raiseError(source: NoticeSource, e: unknown): UiNotice {
      return this.raise(source, 'error', e instanceof Error ? e.message : String(e))
    },

    /**
     * Record a notice. Returns it (coalesced or new) so a caller can inspect
     * `count` or `id` if it cares.
     */
    raise(source: NoticeSource, severity: NoticeSeverity, message: string): UiNotice {
      const now = Date.now()
      const existing = this.latestFor(source)
      if (
        existing &&
        existing.severity === severity &&
        existing.message === message &&
        now - existing.lastAt < COALESCE_MS
      ) {
        existing.count += 1
        existing.lastAt = now
        return existing
      }

      const notice: UiNotice = {
        id: (_seq += 1),
        source,
        severity,
        message,
        at: now,
        lastAt: now,
        count: 1,
      }
      this.items.push(notice)
      if (this.items.length > RING_CAP) {
        this.items.splice(0, this.items.length - RING_CAP)
      }
      return notice
    },

    /** Drop one source's notices — what "the operation succeeded" means. */
    clearSource(source: NoticeSource): void {
      this.items = this.items.filter((n) => n.source !== source)
    },

    /** Drop everything (diagnostics "clear"). */
    clear(): void {
      this.items = []
    },
  },
})

// ============================================================================
// stores/toast.ts — a tiny global toast bus.
//
// Why a store instead of a per-component toast: LogAnomaly toasts and the
// connection-kill toasts can fire from *anywhere* (an event stream, a virtual
// list row, a proxy-node card), and the renderer had no global surface to show
// them — each feature would otherwise roll its own inline toast (as
// ProfileManager does) and the result would be a pile of competing overlays.
//
// The store is i18n-agnostic on purpose: callers resolve their own strings and
// hand `push()` a concrete `title` / `message`. It only owns the queue, the
// auto-dismiss timers, and the `kind` used for icon/colour.
// ============================================================================

import { defineStore } from 'pinia'

export type ToastKind = 'info' | 'success' | 'error'

export interface ToastItem {
  id: number
  kind: ToastKind
  title: string
  message?: string
}

/** How long a toast stays before it dismisses itself. */
const TTL_MS = 5000

/** Monotonic id, module-scoped so it never resets with a store teardown. */
let _seq = 0

/** Live auto-dismiss timers, keyed by id (not reactive state). */
const _timers = new Map<number, ReturnType<typeof setTimeout>>()

export const useToastStore = defineStore('toast', {
  state: () => ({
    items: [] as ToastItem[],
  }),

  actions: {
    /** Push a toast and arm its auto-dismiss timer. */
    push(kind: ToastKind, title: string, message?: string): number {
      const id = ++_seq
      this.items.push({ id, kind, title, message })
      const timer = setTimeout(() => this.dismiss(id), TTL_MS)
      _timers.set(id, timer)
      return id
    },

    /** Remove a toast (and cancel its timer). */
    dismiss(id: number): void {
      const timer = _timers.get(id)
      if (timer) {
        clearTimeout(timer)
        _timers.delete(id)
      }
      this.items = this.items.filter((t) => t.id !== id)
    },

    /** Drop every toast at once (used on app teardown). */
    clear(): void {
      for (const timer of _timers.values()) clearTimeout(timer)
      _timers.clear()
      this.items = []
    },
  },
})

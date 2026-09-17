// ============================================================================
// stores/appstate.ts — Unified dashboard state projection (Phase R3).
//
// Two Rust-pushed streams feed this store, and it is the renderer's only
// projection of either:
//
//   * `app-state://sync` (`core/watcher.rs`) — once a second: kernel
//     online-ness, system-proxy state, outbound mode.
//   * `TrafficPayload` (`core/ingest.rs`) — once a second: the live up/down
//     rates, straight off mihomo's own `/traffic` stream.
//
// The rates used to be folded into `AppStateSnapshot` as a per-tick diff of
// `/connections` byte totals. That derivation is gone — `/traffic` is the
// authoritative source and nothing should be able to disagree with it — so the
// two streams are deliberately separate and `apply` no longer touches speed.
// ============================================================================

import { defineStore } from 'pinia'
import type { AppStateSnapshot, TrafficPayload } from '@/bindings'
import type { UnlistenFn } from '@/utils/tauri-bridge'
import { useKernelStore } from '@/stores/kernel'
import { useNoticesStore } from '@/stores/notices'

// Listener registry owned by this module; App.vue registers the
// `app-state://sync` + `TrafficPayload` subscriptions and releases them on
// unmount.
let unlistens: UnlistenFn[] = []

interface AppState {
  online: boolean
  proxyActive: boolean
  uploadSpeed: number
  downloadSpeed: number
  /** Outbound mode: rule / global / direct. */
  mode: string
  /**
   * Epoch ms of the last `/traffic` sample, or `null` before the first one.
   *
   * Kept as a timestamp rather than a boolean so a consumer can judge
   * *staleness*: the ingest pushes exactly one sample a second, so a gap means
   * the socket is gone, and a plain "connected" flag would go stale silently.
   */
  lastTrafficAt: number | null
}

export const useAppStateStore = defineStore('appstate', {
  state: (): AppState => ({
    online: false,
    proxyActive: false,
    uploadSpeed: 0,
    downloadSpeed: 0,
    mode: 'rule',
    lastTrafficAt: null,
  }),

  getters: {
    isOnline: (s): boolean => s.online,
    totalSpeed: (s): number => s.uploadSpeed + s.downloadSpeed,
  },

  actions: {
    /** Project one `app-state://sync` tick into local reactive state. */
    apply(snap: AppStateSnapshot): void {
      this.online = snap.kernelOnline
      this.proxyActive = snap.systemProxyActive
      if (snap.currentMode) this.mode = snap.currentMode
      this.crossCheck(snap.kernelOnline)
    },

    /**
     * `online` is no longer a truth to branch on — `kernel.availability` is.
     *
     * Rust's 1 s snapshot and our own view of the world are produced by
     * different paths (a supervisor state machine vs. a REST probe), so they
     * can legitimately disagree for a second or two. That is not worth
     * showing. What IS worth recording is a disagreement that persists,
     * because it means one of the two paths is lying — which is exactly the
     * kind of thing nobody can debug after the fact.
     *
     * Severity is `info`, so this lands in the diagnostics log and never on
     * the user's screen.
     */
    crossCheck(rustSaysOnline: boolean): void {
      const kernel = useKernelStore()
      if (kernel.state === 'unknown') return
      const weSayUp = kernel.availability !== 'down'
      if (rustSaysOnline === weSayUp) return
      useNoticesStore().raise(
        'kernel',
        'info',
        rustSaysOnline
          ? 'Rust reports the kernel online while the frontend sees it down'
          : 'Rust reports the kernel offline while the frontend sees it up',
      )
    },

    /**
     * Project one typed `/traffic` sample. This is the *only* writer of
     * `uploadSpeed` / `downloadSpeed`.
     */
    applyTraffic(sample: TrafficPayload): void {
      this.uploadSpeed = sample.up
      this.downloadSpeed = sample.down
      this.lastTrafficAt = Date.now()
    },

    /** Track a listener so App.vue can release it on teardown. */
    addListener(u: UnlistenFn): void {
      unlistens.push(u)
    },
    /** Drop every registered listener (app unmount). */
    release(): void {
      for (const u of unlistens) u()
      unlistens = []
    },
  },
})

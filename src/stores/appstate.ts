// ============================================================================
// stores/appstate.ts — Unified dashboard state projection (Phase R3).
//
// Rust's `core/watcher.rs` broadcasts one `AppStateSnapshot` per second on
// `app-state://sync`. This store is the single renderer-side projection:
// components read here instead of probing mihomo / the registry themselves.
// ============================================================================

import { defineStore } from 'pinia'
import type { AppStateSnapshot } from '@/bindings'
import type { UnlistenFn } from '@/utils/tauri-bridge'

// Listener registry owned by this module; App.vue registers the single
// `app-state://sync` subscription and releases it on unmount.
let unlistens: UnlistenFn[] = []

interface AppState {
  online: boolean
  proxyActive: boolean
  uploadSpeed: number
  downloadSpeed: number
  /** Outbound mode: rule / global / direct. */
  mode: string
}

export const useAppStateStore = defineStore('appstate', {
  state: (): AppState => ({
    online: false,
    proxyActive: false,
    uploadSpeed: 0,
    downloadSpeed: 0,
    mode: 'rule',
  }),

  getters: {
    isOnline: (s): boolean => s.online,
    totalSpeed: (s): number => s.uploadSpeed + s.downloadSpeed,
  },

  actions: {
    /** Project one snapshot tick into local reactive state. */
    apply(snap: AppStateSnapshot): void {
      this.online = snap.kernelOnline
      this.proxyActive = snap.systemProxyActive
      this.uploadSpeed = snap.uploadSpeed
      this.downloadSpeed = snap.downloadSpeed
      if (snap.currentMode) this.mode = snap.currentMode
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

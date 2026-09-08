// ============================================================================
// stores/proxy.ts — Pinia store for the Windows system proxy switch.
//
// State is the single source of truth, updated by:
//   1. Local toggle action (calls Rust, awaits, then commits result).
//   2. The `system-proxy://changed` event fired by Rust whenever any code
//      path (tray menu, dashboard, future CLI) flips the registry.
// We always re-read `getSystemProxyStatus()` after an event, so we never
// trust the in-flight value blindly — Rust is authoritative.
// ============================================================================

import { defineStore } from 'pinia'
import { safeListen, type UnlistenFn } from '@/utils/tauri-bridge'

import {
  enableSystemProxy,
  disableSystemProxy,
  getSystemProxyStatus,
  SYSTEM_PROXY_CHANGED_EVENT,
  type ProxyStatus,
  type ProxyToggleResult,
} from '@/services/proxy'

interface ProxyState {
  status: ProxyStatus | null
  toggling: boolean
  lastError: string | null
  _unlisteners: UnlistenFn[]
}

export const useProxyStore = defineStore('proxy', {
  state: (): ProxyState => ({
    status: null,
    toggling: false,
    lastError: null,
    _unlisteners: [],
  }),

  getters: {
    enabled: (s): boolean => s.status?.enabled === true,
    /** Effective port the system proxy points to (defaults to 7890 / mihomo mixed-port). */
    port: (s): number | null => {
      if (!s.status?.enabled || !s.status.server) return null
      const m = s.status.server.match(/:(\d+)$/)
      return m ? Number(m[1]) : null
    },
  },

  actions: {
    async init(): Promise<void> {
      await this.refresh()
      await this.subscribeEvents()
    },

    async refresh(): Promise<void> {
      try {
        this.status = await getSystemProxyStatus()
        this.lastError = null
      } catch (e) {
        this.lastError = e instanceof Error ? e.message : String(e)
      }
    },

    async subscribeEvents(): Promise<void> {
      if (this._unlisteners.length > 0) return
      this._unlisteners.push(
        await safeListen<{ enabled: boolean; port: number | null; source: string }>(
          SYSTEM_PROXY_CHANGED_EVENT,
          () => {
            // Re-read from registry (single source of truth).
            void this.refresh()
          },
        ),
      )
    },

    async enable(port = 7890): Promise<ProxyToggleResult> {
      this.toggling = true
      this.lastError = null
      try {
        const res = await enableSystemProxy(port)
        await this.refresh()
        return res
      } catch (e) {
        this.lastError = e instanceof Error ? e.message : String(e)
        throw e
      } finally {
        this.toggling = false
      }
    },

    async disable(): Promise<ProxyToggleResult> {
      this.toggling = true
      this.lastError = null
      try {
        const res = await disableSystemProxy()
        await this.refresh()
        return res
      } catch (e) {
        this.lastError = e instanceof Error ? e.message : String(e)
        throw e
      } finally {
        this.toggling = false
      }
    },

    async toggle(): Promise<void> {
      if (this.enabled) {
        await this.disable()
      } else {
        await this.enable()
      }
    },

    dispose(): void {
      for (const u of this._unlisteners) {
        try { u() } catch { /* noop */ }
      }
      this._unlisteners = []
    },
  },
})

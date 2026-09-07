// ============================================================================
// stores/tun.ts — Pinia store for M9 TUN mode.
//
// Mirrors the structure of stores/desktop.ts / stores/proxy.ts so the
// three "toggle" cards (SystemProxy, AutoStart, Tun) feel uniform to
// the user. The TUN transition is asynchronous (UAC consent + child
// startup can take seconds) so the store models busy + lastError
// alongside the authoritative state field.
// ============================================================================

import { defineStore } from 'pinia'
import {
  enableTun,
  disableTun,
  getTunState,
  sweepTunRoutes,
  type TunState,
  type TunStatus,
  type SweepResultFull,
} from '@/services/tun'

interface TunState_ {
  state: TunState
  enabled: boolean
  device: string
  lastError: string | null
  lastChangedAtMs: number
  lastSweep: SweepResultFull | null
  busy: boolean
  initialised: boolean
  /** Toggled off transiently to silence the user while we re-fetch
   *  after a tun://state-changed event we triggered locally. */
  suppressNextEvent: boolean
}

export const useTunStore = defineStore('tun', {
  state: (): TunState_ => ({
    state: 'off',
    enabled: false,
    device: 'flexclash-tun',
    lastError: null,
    lastChangedAtMs: 0,
    lastSweep: null,
    busy: false,
    initialised: false,
    suppressNextEvent: false,
  }),

  getters: {
    /** Convenience for the UI badge: a single boolean for the toggle. */
    isOn: (s): boolean => s.state === 'on',
    /** True while a transition is in flight (Enabling / Disabling). */
    isTransitioning: (s): boolean =>
      s.state === 'enabling' || s.state === 'disabling',
    /** Friendly label for the current state, localised in zh-CN. */
    stateLabel: (s): string => {
      switch (s.state) {
        case 'off': return '已关闭'
        case 'enabling': return '正在启用…'
        case 'on': return '已启用'
        case 'disabling': return '正在关闭…'
        case 'failed': return '启用失败'
      }
    },
  },

  actions: {
    async init(): Promise<void> {
      try {
        const s = await getTunState()
        this.applyStatus(s)
      } catch (e) {
        this.lastError = e instanceof Error ? e.message : String(e)
      } finally {
        this.initialised = true
      }
    },

    applyStatus(s: TunStatus): void {
      this.state = s.state
      this.enabled = s.enabled
      this.device = s.device
      this.lastError = s.last_error
      this.lastChangedAtMs = s.last_changed_at_ms
      this.lastSweep = s.last_sweep
    },

    async refresh(): Promise<void> {
      const s = await getTunState()
      this.applyStatus(s)
    },

    async setEnabled(enabled: boolean): Promise<void> {
      if (this.busy) return
      this.busy = true
      this.lastError = null
      // Suppress the next event because we already have the local
      // snapshot returned by the invoke() result.
      this.suppressNextEvent = true
      try {
        const s = enabled ? await enableTun() : await disableTun()
        this.applyStatus(s)
      } catch (e) {
        this.lastError = e instanceof Error ? e.message : String(e)
        // Re-fetch authoritative state in case the failure was a
        // partial transition.
        try { await this.refresh() } catch { /* ignore */ }
        throw e
      } finally {
        this.busy = false
      }
    },

    async toggle(): Promise<void> {
      await this.setEnabled(!this.isOn)
    },

    async runSweep(): Promise<SweepResultFull> {
      const r = await sweepTunRoutes()
      this.lastSweep = r
      return r
    },

    /** Called by the App-level event listener for `tun://state-changed`. */
    onStateChanged(s: TunStatus): void {
      if (this.suppressNextEvent) {
        this.suppressNextEvent = false
        // Still apply (server snapshot may differ from the optimistic
        // local one when e.g. UAC was cancelled) but skip the toast.
        this.applyStatus(s)
        return
      }
      const wasOn = this.isOn
      this.applyStatus(s)
      // Surface non-OK transitions as lastError; the UI listens on
      // lastError to render a toast.
      if (s.state === 'failed' && s.last_error) {
        this.lastError = s.last_error
      }
      // Reset suppress flag if the event happened to carry a failure
      // (in that case we DO want to notify).
      if (s.state === 'failed') this.suppressNextEvent = false
      // Touch `wasOn` to keep the lint happy about unused locals.
      void wasOn
    },
  },
})

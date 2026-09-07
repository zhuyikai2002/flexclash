// ============================================================================
// stores/desktop.ts — Pinia store for autostart + silent launch + M9 route sweep.
//
// The single source of truth lives in the Windows registry; Rust reads
// `HKCU\...\Run\FlexClash` and `SilentFlag` managed state. The store
// mirrors the latest known values and exposes toggle / refresh actions.
// ============================================================================

import { defineStore } from 'pinia'
import {
  getAutostartStatus,
  getSilentFlag,
  setAutostart as setAutostartInvoke,
  sweepResidualRoutes,
  type AutostartStatus,
  type SweepResult,
} from '@/services/autostart'

interface DesktopState {
  enabled: boolean
  silent: boolean
  busy: boolean
  lastError: string | null
  /** Last sweep result, surfaced in the UI as a toast or footer note. */
  lastSweep: SweepResult | null
  /** True after the first `init()` has resolved. UI uses this to avoid
   *  flashing a "Disabled" pill before Rust has been queried. */
  initialised: boolean
}

export const useDesktopStore = defineStore('desktop', {
  state: (): DesktopState => ({
    enabled: false,
    silent: false,
    busy: false,
    lastError: null,
    lastSweep: null,
    initialised: false,
  }),

  getters: {
    /** True when the app was launched from the autostart hook. */
    startedInBackground: (s): boolean => s.silent,
  },

  actions: {
    async init(): Promise<void> {
      try {
        const status = await getAutostartStatus()
        this.enabled = status.enabled
        this.silent = status.silent
      } catch (e) {
        this.lastError = e instanceof Error ? e.message : String(e)
      } finally {
        this.initialised = true
      }
    },

    async refresh(): Promise<void> {
      const status = await getAutostartStatus()
      this.enabled = status.enabled
      this.silent = status.silent
    },

    async setAutostart(enabled: boolean): Promise<void> {
      this.busy = true
      this.lastError = null
      try {
        const status = await setAutostartInvoke(enabled)
        this.enabled = status.enabled
        this.silent = status.silent
      } catch (e) {
        this.lastError = e instanceof Error ? e.message : String(e)
        throw e
      } finally {
        this.busy = false
      }
    },

    async toggle(): Promise<void> {
      await this.setAutostart(!this.enabled)
    },

    /** Run the M9 route sweep (no-op in M7). */
    async runSweep(): Promise<SweepResult> {
      const res = await sweepResidualRoutes()
      this.lastSweep = res
      return res
    },
  },
})

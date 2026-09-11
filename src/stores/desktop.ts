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
  getSilentAutostartStatus,
  setAutostart as setAutostartInvoke,
  setSilentAutostart as setSilentAutostartInvoke,
  sweepResidualRoutes,
  type SilentAutostartStatus,
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
  /** Elevated (Task Scheduler) autostart is registered. */
  taskEnabled: boolean
  /** Platform supports the elevated mechanism at all. */
  taskAvailable: boolean
  /** Separate busy flag: enabling raises UAC, so it is a much longer and
   *  more interruptible operation than flipping the registry entry. */
  taskBusy: boolean
  taskError: string | null
}

export const useDesktopStore = defineStore('desktop', {
  state: (): DesktopState => ({
    enabled: false,
    silent: false,
    busy: false,
    lastError: null,
    lastSweep: null,
    initialised: false,
    taskEnabled: false,
    taskAvailable: false,
    taskBusy: false,
    taskError: null,
  }),

  getters: {
    /** True when the app was launched from the autostart hook. */
    startedInBackground: (s): boolean => s.silent,
    /** Exactly one mechanism may be armed at a time (backend invariant). */
    anyAutostart: (s): boolean => s.enabled || s.taskEnabled,
  },

  actions: {
    async init(): Promise<void> {
      try {
        const status = await getAutostartStatus()
        this.enabled = status.enabled
        this.silent = status.silent
      } catch (e) {
        this.lastError = e instanceof Error ? e.message : String(e)
      }
      try {
        this.applySilentStatus(await getSilentAutostartStatus())
      } catch (e) {
        this.taskError = e instanceof Error ? e.message : String(e)
      } finally {
        this.initialised = true
      }
    },

    async refresh(): Promise<void> {
      const status = await getAutostartStatus()
      this.enabled = status.enabled
      this.silent = status.silent
      this.applySilentStatus(await getSilentAutostartStatus())
    },

    applySilentStatus(status: SilentAutostartStatus): void {
      this.taskEnabled = status.enabled
      this.taskAvailable = status.available
      // The backend clears the opposite mechanism, so mirror that here
      // immediately instead of waiting for a refresh: otherwise the two
      // cards can both read "ON" for a frame, which looks like the
      // double-launch bug we are specifically preventing.
      this.enabled = status.registry_active
    },

    async setAutostart(enabled: boolean): Promise<void> {
      this.busy = true
      this.lastError = null
      try {
        const status = await setAutostartInvoke(enabled)
        this.enabled = status.enabled
        this.silent = status.silent
        // Enabling the registry entry disarms the task in the backend.
        if (enabled) {
          this.applySilentStatus(await getSilentAutostartStatus())
        }
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

    async setSilentAutostart(enabled: boolean): Promise<void> {
      this.taskBusy = true
      this.taskError = null
      try {
        this.applySilentStatus(await setSilentAutostartInvoke(enabled))
        // Re-read the registry side too: enabling the task clears it.
        const status = await getAutostartStatus()
        this.enabled = status.enabled
        this.silent = status.silent
      } catch (e) {
        this.taskError = e instanceof Error ? e.message : String(e)
        throw e
      } finally {
        this.taskBusy = false
      }
    },

    async toggleSilentAutostart(): Promise<void> {
      await this.setSilentAutostart(!this.taskEnabled)
    },

    /** Run the M9 route sweep (no-op in M7). */
    async runSweep(): Promise<SweepResult> {
      const res = await sweepResidualRoutes()
      this.lastSweep = res
      return res
    },
  },
})

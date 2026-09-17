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
} from '@/services/autostart'
import type { SilentAutostartStatus, SweepResult } from '@/bindings'
import { useNoticesStore } from '@/stores/notices'

interface DesktopState {
  enabled: boolean
  silent: boolean
  busy: boolean
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
}

export const useDesktopStore = defineStore('desktop', {
  state: (): DesktopState => ({
    enabled: false,
    silent: false,
    busy: false,
    lastSweep: null,
    initialised: false,
    taskEnabled: false,
    taskAvailable: false,
    taskBusy: false,
  }),

  getters: {
    /** Thin proxy over the unified notice channel — stores/notices.ts. */
    lastError: (): string | null =>
      useNoticesStore().latestFor('desktop')?.message ?? null,
    /** Same, for the elevated (Task Scheduler) mechanism. Kept distinct
     *  from `lastError` so the two cards never show each other's message. */
    taskError: (): string | null =>
      useNoticesStore().latestFor('desktopTask')?.message ?? null,
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
        useNoticesStore().raiseError('desktop', e)
      }
      try {
        this.applySilentStatus(await getSilentAutostartStatus())
      } catch (e) {
        useNoticesStore().raiseError('desktopTask', e)
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
      useNoticesStore().clearSource('desktop')
      try {
        const status = await setAutostartInvoke(enabled)
        this.enabled = status.enabled
        this.silent = status.silent
        // Enabling the registry entry disarms the task in the backend.
        if (enabled) {
          this.applySilentStatus(await getSilentAutostartStatus())
        }
      } catch (e) {
        useNoticesStore().raiseError('desktop', e)
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
      useNoticesStore().clearSource('desktopTask')
      try {
        this.applySilentStatus(await setSilentAutostartInvoke(enabled))
        // Re-read the registry side too: enabling the task clears it.
        const status = await getAutostartStatus()
        this.enabled = status.enabled
        this.silent = status.silent
      } catch (e) {
        useNoticesStore().raiseError('desktopTask', e)
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

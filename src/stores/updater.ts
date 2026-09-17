// ============================================================================
// stores/updater.ts — Pinia store for the in-app updater (v0.5.x UX).
//
// Owns the update state machine and the single progress listener, so both the
// settings card and the cold-start dialog read the same authoritative state
// and never re-attach competing listeners.
//
//   idle -> checking -> available -> downloading -> ready -> installing
//                     \-> uptodate                 \-> error (any step)
//
// `ready` is the deliberate "downloaded, waiting for the user" stop: the
// settings button and the startup dialog take different next steps from it.
// ============================================================================

import { defineStore } from 'pinia'
import type { UpdateInfo } from '@/bindings'
import { useNoticesStore } from '@/stores/notices'
import {
  checkUpdate,
  downloadUpdate,
  installUpdate,
  listenProgress,
} from '@/services/updater'

export type UpdaterPhase =
  | 'idle'
  | 'checking'
  | 'uptodate'
  | 'available'
  | 'downloading'
  | 'ready'
  | 'installing'
  | 'error'

interface UpdaterState {
  phase: UpdaterPhase
  info: UpdateInfo | null
  /** Download progress 0..100 (0 while unknown). */
  progress: number
  /** Whether the cold-start "update available" dialog should be open. */
  promptOpen: boolean
  /** Guards the progress listener against double-attach. */
  listening: boolean
}

export const useUpdaterStore = defineStore('updater', {
  state: (): UpdaterState => ({
    phase: 'idle',
    info: null,
    progress: 0,
    promptOpen: false,
    listening: false,
  }),

  getters: {
    /** Thin proxy over the unified notice channel — stores/notices.ts. */
    error: (): string | null => useNoticesStore().latestFor('updater')?.message ?? null,
    isBusy: (s): boolean =>
      s.phase === 'checking' || s.phase === 'downloading' || s.phase === 'installing',
    isReady: (s): boolean => s.phase === 'ready',
    /** `v0.5.0` or empty when nothing is known. */
    versionLabel: (s): string => (s.info ? `v${s.info.version}` : ''),
  },

  actions: {
    /** Attach the live `updater://progress` listener exactly once. */
    async attachProgress(): Promise<void> {
      if (this.listening) return
      this.listening = true
      await listenProgress((p) => {
        this.progress =
          p.total && p.total > 0
            ? Math.min(100, Math.round((p.downloaded / p.total) * 100))
            : 0
      })
    },

    /** Cold start: attach the listener, then check silently. Never nags the
     *  user over a network error — that only surfaces in the settings card. */
    async init(): Promise<void> {
      await this.attachProgress()
      await this.check({ silent: true })
    },

    async check(opts: { silent?: boolean } = {}): Promise<void> {
      if (this.isBusy) return
      this.phase = 'checking'
      useNoticesStore().clearSource('updater')
      try {
        const info = await checkUpdate()
        if (info) {
          this.info = info
          this.phase = 'available'
          if (opts.silent) this.promptOpen = true
        } else {
          this.info = null
          this.phase = 'uptodate'
        }
      } catch (e) {
        this.phase = 'error'
        useNoticesStore().raiseError('updater', e)
      }
    },

    /** Download + verify; resolves once parked on the Rust side (`ready`).
     *  Rejects on failure so the dialog can distinguish "now install" from
     *  "show the error". */
    async download(): Promise<void> {
      if (this.phase === 'downloading' || this.phase === 'installing') return
      this.phase = 'downloading'
      this.progress = 0
      useNoticesStore().clearSource('updater')
      try {
        const info = await downloadUpdate()
        this.info = info
        this.progress = 100
        this.phase = 'ready'
      } catch (e) {
        this.phase = 'error'
        useNoticesStore().raiseError('updater', e)
        throw e
      }
    },

    /** Launch the installer. On Windows this exits the app; returning here
     *  means the platform deferred the swap, so we stay in `ready`. */
    async install(): Promise<void> {
      if (this.phase !== 'ready') return
      this.phase = 'installing'
      useNoticesStore().clearSource('updater')
      try {
        await installUpdate()
        this.phase = 'ready'
      } catch (e) {
        this.phase = 'error'
        useNoticesStore().raiseError('updater', e)
        throw e
      }
    },

    /** "Remind me later": hide the prompt but keep the update available for
     *  the settings card. */
    dismissPrompt(): void {
      this.promptOpen = false
    },
  },
})

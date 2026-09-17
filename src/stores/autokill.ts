// ============================================================================
// stores/autokill.ts — the dead-link autopilot's kill switch (v0.6.x Step 5).
//
// WHY A STORE AND NOT A LOCAL `ref`
// ---------------------------------
// The switch has three possible truths and only one of them is ours:
//
//   * `FLEXCLASH_AUTOKILL=off` — the hard veto, read by Rust from the
//     environment. `envLocked` is its shadow, and the UI must render the
//     toggle *disabled* when it is set: a switch that cannot change the
//     outcome is worse than no switch, because it invites the user to
//     conclude the feature is broken.
//   * the runtime override — what `set()` writes, session-scoped;
//   * the default (on), before either has spoken.
//
// Keeping those in one store means Settings and any future surface read the
// same answer instead of each keeping a private copy that can drift.
//
// The override is deliberately NOT persisted anywhere. A durable "off" is the
// kind of setting a user forgets they set and then reports as "the app
// stopped healing itself".
// ============================================================================

import { defineStore } from 'pinia'
import { getAutokillState, setAutokill } from '@/services/deadlink'

interface AutokillStoreState {
  /** Whether a cleanup will actually delete connections right now. */
  enabled: boolean
  /** `FLEXCLASH_AUTOKILL=off` — the UI must disable its toggle. */
  envLocked: boolean
  /** False until the first successful read, so the UI can avoid flashing. */
  initialised: boolean
  busy: boolean
  lastError: string | null
}

export const useAutokillStore = defineStore('autokill', {
  state: (): AutokillStoreState => ({
    enabled: true,
    envLocked: false,
    initialised: false,
    busy: false,
    lastError: null,
  }),

  getters: {
    /** The toggle is inert while busy or while the environment vetoes. */
    isLocked: (s): boolean => s.busy || s.envLocked,
  },

  actions: {
    /** Read the switch from Rust. Safe to call more than once. */
    async refresh(): Promise<void> {
      this.lastError = null
      try {
        const s = await getAutokillState()
        this.enabled = s.enabled
        this.envLocked = s.envLocked
        this.initialised = true
      } catch (e) {
        this.lastError = e instanceof Error ? e.message : String(e)
      }
    },

    /**
     * Flip the override.
     *
     * The UI is optimistic-free on purpose: `enabled` is written from Rust's
     * reply, never from the requested value. When the environment vetoes, the
     * request still "succeeds" but the answer comes back `enabled: false` —
     * mirroring that honestly is the whole point of `envLocked`.
     */
    async set(enabled: boolean): Promise<void> {
      if (this.busy || this.envLocked) return
      this.busy = true
      this.lastError = null
      try {
        const s = await setAutokill(enabled)
        this.enabled = s.enabled
        this.envLocked = s.envLocked
        this.initialised = true
      } catch (e) {
        this.lastError = e instanceof Error ? e.message : String(e)
      } finally {
        this.busy = false
      }
    },

    async toggle(): Promise<void> {
      await this.set(!this.enabled)
    },
  },
})

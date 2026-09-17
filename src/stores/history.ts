// ============================================================================
// stores/history.ts — Pinia store for the M10 traffic history view.
//
// Mirrors the structure of stores/connections.ts so the new view slots
// in cleanly. Owns:
//   * the current range selector,
//   * the most recent fetch result per range (so tabbing back to 1h
//     after viewing 7d is instant — no round-trip),
//   * a background refresh loop (15s) that re-pulls only the active
//     range. The sampler on the Rust side is the authoritative writer;
//     this loop is just a polling consumer.
// ============================================================================

import { defineStore } from 'pinia'
import {
  getTrafficHistory,
  getHistorySampleCount,
  getHistoryDbPath,
  type HistoryRange,
} from '@/services/history'
import type { TrafficHistory } from '@/bindings'
import { useNoticesStore } from '@/stores/notices'
import { useKernelStore } from '@/stores/kernel'

interface HistoryState_ {
  range: HistoryRange
  /** Cached result per range so tabbing is instant. */
  cache: Partial<Record<HistoryRange, TrafficHistory>>
  loading: boolean
  lastFetchAtMs: number | null
  /** "Currently buffering N rows" line. */
  sampleCount: number | null
  dbPath: string | null
  refreshTimer: ReturnType<typeof setInterval> | null
}

const REFRESH_MS = 15_000

export const useHistoryStore = defineStore('history', {
  state: (): HistoryState_ => ({
    range: '1h',
    cache: {},
    loading: false,
    lastFetchAtMs: null,
    sampleCount: null,
    dbPath: null,
    refreshTimer: null,
  }),

  getters: {
    /** Thin proxy over the unified notice channel — stores/notices.ts. */
    lastError: (): string | null =>
      useNoticesStore().latestFor('history')?.message ?? null,
    current: (s): TrafficHistory | null => s.cache[s.range] ?? null,
    hasData: (s): boolean => (s.cache[s.range]?.buckets?.length ?? 0) > 0,
    /** Sum of upload bytes for the active range. */
    totalUpload: (s): number => s.cache[s.range]?.total_upload ?? 0,
    /** Sum of download bytes for the active range. */
    totalDownload: (s): number => s.cache[s.range]?.total_download ?? 0,
  },

  actions: {
    /** Start the periodic refresh + read DB metadata. Idempotent. */
    async init(): Promise<void> {
      if (this.refreshTimer !== null) return
      try { this.dbPath = await getHistoryDbPath() } catch { /* noop */ }
      try { this.sampleCount = await getHistorySampleCount() } catch { /* noop */ }
      await this.refresh()
      this.refreshTimer = setInterval(() => {
        void this.tick()
      }, REFRESH_MS)
    },

    /** Stop the refresh loop. Called from HMR / tests. */
    dispose(): void {
      if (this.refreshTimer !== null) {
        clearInterval(this.refreshTimer)
        this.refreshTimer = null
      }
    },

    setRange(r: HistoryRange): void {
      if (this.range === r) return
      this.range = r
      // If we have cached data for the new range, no need to fetch
      // immediately. The 15s tick will refresh it within REFRESH_MS.
      if (!this.cache[r]) void this.refresh()
    },

    async refresh(): Promise<void> {
      if (this.loading) return
      this.loading = true
      useNoticesStore().clearSource('history')
      try {
        const h = await getTrafficHistory(this.range)
        this.cache = { ...this.cache, [this.range]: h }
        this.lastFetchAtMs = Date.now()
      } catch (e) {
        useNoticesStore().raiseError('history', e)
      } finally {
        this.loading = false
      }
    },

    async tick(): Promise<void> {
      // Refresh the active range + the live row counter. Errors during
      // a tick are silent — the cached value keeps the chart rendering.
      //
      // Skipped entirely while the kernel is unreachable: this loop used to
      // poll regardless, so a dead controller produced a failure every 15 s
      // for something that was never going to answer.
      if (useKernelStore().availability !== 'up') return
      try {
        const h = await getTrafficHistory(this.range)
        this.cache = { ...this.cache, [this.range]: h }
        this.lastFetchAtMs = Date.now()
      } catch (e) {
        // `warn`, not `error`: the cached series keeps the chart rendering,
        // so this is degraded rather than broken — and it must not toast.
        useNoticesStore().raise('history', 'warn', e instanceof Error ? e.message : String(e))
      }
      try { this.sampleCount = await getHistorySampleCount() } catch { /* ignore */ }
    },
  },
})

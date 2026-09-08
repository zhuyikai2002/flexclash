// ============================================================================
// stores/connections.ts — Pinia store for the M8 connection monitor.
//
// Hard performance contract:
//   * `rows` is a plain array of `markRaw`-ed ConnectionRow objects, so Vue
//     WILL NOT wrap per-row fields in a Proxy. The whole array is replaced
//     on every snapshot (cheap O(n) at 1k rows) and the consumer
//     re-renders only on identity change.
//   * `previous` counters (for speed derivation) live in a plain `Map`,
//     never reactive.
// ============================================================================

import { defineStore } from 'pinia'
import { markRaw } from 'vue'
import {
  deriveSpeeds,
  dropAllConnections,
  dropConnection,
  fetchConnections,
  hashConnections,
  projectConnection,
} from '@/services/connections'
import type { Connection, ConnectionRow } from '@/types/clash'

export type PollIntervalMs = 1000 | 2000 | 5000

interface ConnectionsState {
  /** Flat rows. Each element is `markRaw`-ed to keep Vue from proxying
   *  per-row fields. The whole array is replaced on every snapshot. */
  rows: ConnectionRow[]
  /** Non-reactive per-id byte counters used by `deriveSpeeds`. */
  previousCounters: Map<string, { upload: number; download: number }>
  lastSnapshotAt: number | null
  lastFetchAt: number | null
  totalConnections: number
  uploadTotal: number
  downloadTotal: number
  searchKeyword: string
  selectedPolicyFilter: string
  pollIntervalMs: PollIntervalMs
  isPaused: boolean
  busy: boolean
  lastError: string | null
  /** Hash of the last fully-projected snapshot, used to skip identical frames. */
  lastContentHash: string
  /** True after the first fetch resolves. */
  initialised: boolean
}

export const useConnectionsStore = defineStore('connections', {
  state: (): ConnectionsState => ({
    rows: [],
    previousCounters: new Map(),
    lastSnapshotAt: null,
    lastFetchAt: null,
    totalConnections: 0,
    uploadTotal: 0,
    downloadTotal: 0,
    searchKeyword: '',
    selectedPolicyFilter: '',
    pollIntervalMs: 2000,
    isPaused: false,
    busy: false,
    lastError: null,
    lastContentHash: '',
    initialised: false,
  }),

  getters: {
    /** Filtered rows (search keyword + policy dropdown). Iterates the
     *  source array, never allocates intermediate wrappers. */
    filteredRows(state): ConnectionRow[] {
      const kw = state.searchKeyword.trim().toLowerCase()
      const pol = state.selectedPolicyFilter
      const base = Array.isArray(state.rows) ? state.rows : []
      if (!kw && !pol) return base
      const out: ConnectionRow[] = []
      for (const r of base) {
        if (pol && r.policy !== pol) continue
        if (kw) {
          const hay =
            `${r.host} ${r.dst} ${r.destinationIP} ${r.process} ${r.processPath} ${r.policy} ${r.rule}`.toLowerCase()
          if (!hay.includes(kw)) continue
        }
        out.push(r)
      }
      return out
    },

    /** Distinct policies for the dropdown filter. */
    distinctPolicies(state): string[] {
      const set = new Set<string>()
      const rows = Array.isArray(state.rows) ? state.rows : []
      for (const r of rows) {
        if (r.policy) set.add(r.policy)
      }
      return Array.from(set).sort()
    },
  },

  actions: {
    /** Pull one snapshot, project, derive speeds, swap array. */
    async refresh(): Promise<void> {
      if (this.busy) return
      this.busy = true
      try {
        const snap = await fetchConnections()
        const now = Date.now()
        const dt = this.lastSnapshotAt ? now - this.lastSnapshotAt : 0
        this.lastSnapshotAt = now
        this.lastFetchAt = now

        // 空值安全：Mihomo 在冷启动/异常时可能返回 null / undefined
        // 的 `connections` 字段，严禁假定它总是可迭代。
        const raw = (snap as { connections?: unknown } | null)?.connections
        const connections: Connection[] = Array.isArray(raw)
          ? (raw as Connection[])
          : []

        // Project and derive per-row speed in place.
        const projected: ConnectionRow[] = []
        for (const c of connections) {
          projected.push(projectConnection(c))
        }
        deriveSpeeds(projected, this.previousCounters, dt)
        // Drop counters for vanished connections.
        const live = new Set(projected.map((r) => r.id))
        for (const id of Array.from(this.previousCounters.keys())) {
          if (!live.has(id)) this.previousCounters.delete(id)
        }

        // Hash-based skip: identical frame (no new traffic, no new conns)
        // means no UI re-render. We markRaw the elements so per-row fields
        // are never proxied.
        const h = hashConnections(projected)
        if (h !== this.lastContentHash) {
          this.rows = projected.map((r) => markRaw(r))
          this.lastContentHash = h
        }

        this.totalConnections = connections.length
        this.uploadTotal = snap.uploadTotal || 0
        this.downloadTotal = snap.downloadTotal || 0
        this.lastError = null
        this.initialised = true
      } catch (e) {
        this.lastError = e instanceof Error ? e.message : String(e)
      } finally {
        this.busy = false
      }
    },

    /** Force a refresh ignoring the busy guard. */
    async forceRefresh(): Promise<void> {
      this.busy = false
      await this.refresh()
    },

    setSearchKeyword(kw: string): void {
      this.searchKeyword = kw
    },

    setPolicyFilter(p: string): void {
      this.selectedPolicyFilter = p
    },

    setPollInterval(ms: PollIntervalMs): void {
      this.pollIntervalMs = ms
    },

    setPaused(paused: boolean): void {
      this.isPaused = paused
    },

    /** Optimistically mark a row as closing, then DELETE it. If the API
     *  rejects (older mihomo), re-throw and roll back the flag. */
    async closeOne(id: string): Promise<void> {
      const before = this.rows
      const idx = before.findIndex((r) => r.id === id)
      if (idx >= 0) {
        const next = before.slice()
        next[idx] = markRaw({ ...next[idx], closing: true })
        this.rows = next
      }
      try {
        await dropConnection(id)
        // Re-fetch to converge with server state (the row is gone).
        await this.forceRefresh()
      } catch (e) {
        // Roll back the optimistic flag.
        const cur = this.rows
        const j = cur.findIndex((r) => r.id === id)
        if (j >= 0) {
          const next = cur.slice()
          next[j] = markRaw({ ...next[j], closing: false })
          this.rows = next
        }
        throw e
      }
    },

    async closeAll(): Promise<void> {
      await dropAllConnections()
      // Re-fetch to converge.
      await this.forceRefresh()
    },

    /** Wipe in-memory counters. Call when leaving the tab. */
    reset(): void {
      this.rows = []
      this.previousCounters.clear()
      this.lastSnapshotAt = null
      this.lastContentHash = ''
      this.searchKeyword = ''
      this.selectedPolicyFilter = ''
      this.lastError = null
    },
  },
})

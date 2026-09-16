// ============================================================================
// stores/proxies.ts — Pinia store for proxy groups, node selection, and
// speed-test result aggregation.
//
// Design:
//   - `groups`  = the Selector/URLTest/Fallback/LoadBalance groups (by name).
//   - `byName`  = the raw `proxies` map from mihomo's GET /proxies (every
//                 proxy/group is keyed here, including Direct / Reject).
//   - Per-node delay info is *denormalised* into `groups[name].nodes[name]`
//     so that the UI can render "ms" badges without re-walking the tree.
//
// Speed testing (v0.3 — handed to Rust):
//   - This store no longer probes anything. The old worker pool of 6 lived
//     here and paid one IPC round-trip per node; the fan-out, timeout budget
//     and classification now live in `src-tauri/src/core/speedtest.rs`, which
//     streams results back in batches. See `src/services/speedtest.ts`.
//   - What remains here is *aggregation*: folding each batch into the group's
//     node map. That is the only part that has to be on this side of the IPC
//     line, because it is what the UI renders.
//   - Batches are keyed by a monotonic run id. `runIds[group]` holds the
//     newest id seen for that group and anything older is discarded, which is
//     what makes "test again mid-run" safe. Ids are global and increasing, so
//     `newer id wins` is exactly the right rule per group — and it also
//     closes the race where a batch is delivered before the `invoke` promise
//     that carries the same id resolves.
//
// Sorting (M6):
//   - `sortMode` toggles between `default` (mihomo's order) and
//     `latency_asc` (lowest first, with timeout/unreachable/error nodes
//     sent to the bottom). Sort is computed in a getter so the underlying
//     state is never reordered — the user can flip modes at will without
//     losing their original layout.
// ============================================================================

import { defineStore } from 'pinia'
import {
  getProxies,
  selectProxy as apiSelectProxy,
} from '@/services/clash'
import {
  startSpeedTest,
  onDelayBatch,
  onDelayDone,
  DEFAULT_TEST_URL,
} from '@/services/speedtest'
import type { DelayBatch, DelayDone, ProbeStatus } from '@/bindings'
import type { UnlistenFn } from '@/utils/tauri-bridge'
import type { Proxy, ProxyType } from '@/types/clash'

export type DelayStatus =
  | 'idle'
  | 'testing'
  | 'ok'
  | 'timeout'
  | 'unreachable'
  | 'error'

export interface NodeDelayInfo {
  status: DelayStatus
  /** RTT in ms; null until measured (or unmeasurable). */
  delay: number | null
  /** Unix ms when this measurement was taken. */
  testedAt: number | null
  /**
   * Why a non-`ok` probe ended that way ("probe budget exhausted (HTTP 504)",
   * "controller unreachable: …"). Surfaced as the node's tooltip — the old
   * pool collapsed every failure into a bare status, which made a dead node
   * and a stopped kernel look identical.
   */
  message: string | null
}

export interface ProxyGroupState {
  name: string
  type: ProxyType
  /** Currently-selected child name. */
  now: string | null
  /** Children names, in the order mihomo returned them. */
  all: string[]
  /** Per-child delay info, keyed by child name. */
  nodes: Record<string, NodeDelayInfo>
}

export type SortMode = 'default' | 'latency_asc'

interface ProxiesStoreState {
  groups: Record<string, ProxyGroupState>
  byName: Record<string, Proxy>
  loading: boolean
  error: string | null
  lastFetchAt: number | null
  /** Groups currently being delay-tested; re-assigned as a new Set to trigger reactivity. */
  testingGroups: string[]
  sortMode: SortMode
  /**
   * Newest speed-test run id seen per group. A batch/done whose `runId` is
   * lower than this is from a superseded run and is dropped.
   */
  runIds: Record<string, number>
}

/** Proxy types that are local/binary and cannot be delay-tested. */
const UNTESTABLE_TYPES = new Set<string>([
  'Direct',
  'Reject',
  'Global',
  'Pass',
  'Compatible',
  'Relay',
])

/** Proxy types that constitute a "group" with selectable children. */
const GROUP_TYPES = new Set<ProxyType>([
  'Selector',
  'URLTest',
  'Fallback',
  'LoadBalance',
])

const emptyDelay = (): NodeDelayInfo => ({
  status: 'idle',
  delay: null,
  testedAt: null,
  message: null,
})

/** Rust `ProbeStatus` → frontend `DelayStatus`. The former is a strict subset. */
function probeStatusToDelay(status: ProbeStatus): DelayStatus {
  switch (status) {
    case 'ok':
      return 'ok'
    case 'timeout':
      return 'timeout'
    case 'unreachable':
      return 'unreachable'
    case 'error':
      return 'error'
    default:
      return 'error'
  }
}

/** A node is considered "fast" (rank 0) only when it has a real RTT. */
function delayRank(info: NodeDelayInfo | undefined): number {
  if (!info) return 4
  switch (info.status) {
    case 'ok':         return 0
    case 'testing':    return 1
    case 'idle':       return 2
    case 'unreachable': return 3
    case 'timeout':    return 4
    case 'error':      return 5
    default:           return 6
  }
}

function compareLatency(a: NodeDelayInfo, b: NodeDelayInfo): number {
  const ra = delayRank(a)
  const rb = delayRank(b)
  if (ra !== rb) return ra - rb
  // Within the "ok" bucket, lowest delay first.
  if (ra === 0 && a.delay !== null && b.delay !== null) return a.delay - b.delay
  // Stable tie-breaker on name.
  return 0
}

// ---------------------------------------------------------------------------
// Stream subscription (process-wide, created once)
// ---------------------------------------------------------------------------
// Deliberately NOT per component mount. A run keeps streaming while the user
// is on another tab, and a mount/unmount cycle would drop whatever landed in
// the gap and leave the group stuck on its spinners. Two listeners for the
// app's lifetime is a fixed cost, not a leak; `disposeSpeedTestStream()`
// exists for explicit teardown.
let batchUnlisten: UnlistenFn | null = null
let doneUnlisten: UnlistenFn | null = null
let streamReady: Promise<void> | null = null

/**
 * Tear the stream subscription down. Not called during normal navigation —
 * exposed so a teardown path (or a test harness) can release it.
 */
export function disposeSpeedTestStream(): void {
  batchUnlisten?.()
  doneUnlisten?.()
  batchUnlisten = null
  doneUnlisten = null
  streamReady = null
}

export const useProxiesStore = defineStore('proxies', {
  state: (): ProxiesStoreState => ({
    groups: {},
    byName: {},
    loading: false,
    error: null,
    lastFetchAt: null,
    testingGroups: [],
    sortMode: 'default',
    runIds: {},
  }),

  getters: {
    selectorGroups: (s): ProxyGroupState[] => {
      return Object.values(s.groups)
        .filter((g) => GROUP_TYPES.has(g.type))
        .sort((a, b) => a.name.localeCompare(b.name))
    },
    isGroupTesting: (s) => (name: string) => s.testingGroups.includes(name),

    /**
     * Child names of a group in the order they should be displayed.
     * When `sortMode === 'latency_asc'` the order is
     *   [successful nodes (asc by delay), then currently-testing,
     *    then idle, then unreachable, then timeout, then error].
     * The currently-selected node is always moved to the top so the user
     * can find it after a sort.
     */
    sortedChildren: (s) => (groupName: string): string[] => {
      const g = s.groups[groupName]
      if (!g) return []
      if (s.sortMode === 'default') return g.all

      const head: string[] = []
      const tail: string[] = []
      for (const name of g.all) {
        if (name === g.now) head.push(name)
        else tail.push(name)
      }
      tail.sort((a, b) => {
        const ra = compareLatency(g.nodes[a], g.nodes[b])
        if (ra !== 0) return ra
        return a.localeCompare(b)
      })
      return [...head, ...tail]
    },
  },

  actions: {
    setSortMode(mode: SortMode): void {
      this.sortMode = mode
    },
    toggleSortMode(): void {
      this.sortMode = this.sortMode === 'default' ? 'latency_asc' : 'default'
    },

    /**
     * Pull the full proxy tree from mihomo and rebuild `groups` / `byName`.
     * Preserves any existing per-node delay info so a refresh doesn't wipe
     * the user's recent speed-test results.
     */
    async fetchProxies(): Promise<void> {
      this.loading = true
      this.error = null
      try {
        const r = await getProxies()
        this.byName = r.proxies

        const next: Record<string, ProxyGroupState> = {}
        for (const [name, proxy] of Object.entries(r.proxies)) {
          if (!proxy.all || proxy.all.length === 0) continue
          if (!GROUP_TYPES.has(proxy.type)) continue

          const prevNodes = this.groups[name]?.nodes ?? {}
          const nodes: Record<string, NodeDelayInfo> = {}
          for (const child of proxy.all) {
            // Preserve the previous delay info keyed by child name.
            nodes[child] = prevNodes[child] ?? emptyDelay()
          }
          next[name] = {
            name: proxy.name,
            type: proxy.type,
            now: proxy.now ?? null,
            all: proxy.all,
            nodes,
          }
        }
        this.groups = next
        this.lastFetchAt = Date.now()
      } catch (e) {
        this.error = e instanceof Error ? e.message : String(e)
      } finally {
        this.loading = false
      }
    },

    /**
     * Switch the active node of a Selector / URLTest / LoadBalance group.
     * Throws on failure (e.g. invalid node name) — caller decides UX.
     */
    async selectProxyNode(groupName: string, nodeName: string): Promise<void> {
      try {
        const updated = await apiSelectProxy(groupName, nodeName)
        const target = this.groups[groupName]
        if (target) {
          // Rust now always returns the refreshed group object, but stay
          // defensive: never read `.now` off a null payload.
          target.now = updated?.now ?? nodeName
        }
        if (updated) {
          this.byName[groupName] = updated
        }
      } catch (e) {
        this.error = e instanceof Error ? e.message : String(e)
        throw e
      }
    },

    // -----------------------------------------------------------------------
    // Speed test (Rust engine + streamed batches)
    // -----------------------------------------------------------------------

    /**
     * Create the batch/done subscriptions once. Idempotent and cheap, so every
     * entry point that might need results can call it without coordinating.
     */
    initSpeedTestStream(): void {
      if (streamReady) return
      const self = this
      streamReady = (async () => {
        batchUnlisten = await onDelayBatch((b) => self.applyDelayBatch(b))
        doneUnlisten = await onDelayDone((d) => self.applyDelayDone(d))
      })().catch((e: unknown) => {
        // Never leave a half-registered stream behind: reset both so the next
        // call retries cleanly, and report it rather than failing silently.
        batchUnlisten = null
        doneUnlisten = null
        streamReady = null
        self.error = e instanceof Error ? e.message : String(e)
      })
    },

    /**
     * Fold one streamed batch into the group's node map.
     *
     * Staleness rule: run ids increase globally, so for a given group a lower
     * id is always older. Dropping those is what makes "test again mid-run"
     * safe — the superseded run keeps emitting until it notices, and none of
     * its results may overwrite the newer run's.
     */
    applyDelayBatch(batch: DelayBatch): void {
      const seen = this.runIds[batch.group] ?? 0
      if (batch.runId < seen) return

      const group = this.groups[batch.group]
      if (!group) return

      // Adopt the id before merging. A batch can be delivered *before* the
      // `invoke` promise that returns this same id resolves; adopting here
      // means that ordering is harmless instead of a dropped first batch.
      if (batch.runId > seen) {
        this.runIds = { ...this.runIds, [batch.group]: batch.runId }
      }

      const nodes: Record<string, NodeDelayInfo> = { ...group.nodes }
      for (const r of batch.results) {
        nodes[r.name] = {
          status: probeStatusToDelay(r.status),
          delay: r.delayMs ?? null,
          testedAt: Date.now(),
          message: r.message ?? null,
        }
      }
      // Re-assign the group so Pinia sees a new reference.
      this.groups[batch.group] = { ...group, nodes }
    },

    /**
     * Terminal handler for a run. Always arrives exactly once per run, which
     * is why the "clear the spinners" work happens here rather than being
     * inferred from result counts.
     */
    applyDelayDone(done: DelayDone): void {
      const seen = this.runIds[done.group] ?? 0
      if (done.runId < seen) return
      if (done.runId > seen) {
        this.runIds = { ...this.runIds, [done.group]: done.runId }
      }

      const group = this.groups[done.group]
      if (group) {
        const nodes: Record<string, NodeDelayInfo> = { ...group.nodes }
        let patched = false
        // A cancelled run leaves nodes that were never probed. Give them a
        // definite "never measured" state — otherwise they would spin forever
        // once `testingGroups` is cleared below.
        for (const [name, info] of Object.entries(nodes)) {
          if (info.status === 'testing') {
            nodes[name] = {
              status: 'idle',
              delay: null,
              testedAt: null,
              message: done.cancelled ? 'cancelled' : 'not probed',
            }
            patched = true
          }
        }
        if (patched) this.groups[done.group] = { ...group, nodes }
      }

      this.testingGroups = this.testingGroups.filter((g) => g !== done.group)
    },

    /**
     * Start a speed test over every child of `groupName`.
     *
     * Resolves as soon as the Rust engine accepts the run — results arrive
     * later via `applyDelayBatch`. Calling it again while a run is in flight
     * supersedes that run rather than being ignored, so the user's most recent
     * intent always wins.
     */
    async speedTestGroup(
      groupName: string,
      testUrl: string = DEFAULT_TEST_URL,
    ): Promise<void> {
      const group = this.groups[groupName]
      if (!group) throw new Error(`Unknown group: ${groupName}`)

      // Must be listening before the run starts, or early batches are lost.
      this.initSpeedTestStream()

      const children = [...group.all]
      const probeable = children.filter((c) => {
        const p = this.byName[c]
        return Boolean(p) && !UNTESTABLE_TYPES.has(p.type)
      })

      // Give every child a definite state up front: probeable ones spin, the
      // rest are terminal immediately (mihomo cannot delay-test Direct/Reject).
      const nodes: Record<string, NodeDelayInfo> = { ...group.nodes }
      for (const child of children) {
        nodes[child] = probeable.includes(child)
          ? { status: 'testing', delay: null, testedAt: null, message: null }
          : {
              status: 'unreachable',
              delay: null,
              testedAt: Date.now(),
              message: 'type cannot be delay-tested',
            }
      }
      this.groups[groupName] = { ...group, nodes }

      if (!this.testingGroups.includes(groupName)) {
        this.testingGroups = [...this.testingGroups, groupName]
      }

      if (probeable.length === 0) {
        // No run is created, so no terminal event will ever arrive. Clear the
        // spinner here instead of leaving it up forever.
        this.testingGroups = this.testingGroups.filter((g) => g !== groupName)
        return
      }

      try {
        const runId = await startSpeedTest(groupName, probeable, { url: testUrl })
        // Keep the newest id. `applyDelayBatch` may already have adopted this
        // same id (it can be delivered first) — only ever move forward.
        if (runId > (this.runIds[groupName] ?? 0)) {
          this.runIds = { ...this.runIds, [groupName]: runId }
        }
      } catch (e) {
        // The run never started, so no `done` event is coming: roll the
        // spinners back ourselves.
        this.testingGroups = this.testingGroups.filter((g) => g !== groupName)
        const current = this.groups[groupName]
        if (current) {
          const rolled: Record<string, NodeDelayInfo> = { ...current.nodes }
          for (const child of probeable) {
            rolled[child] = {
              status: 'error',
              delay: null,
              testedAt: Date.now(),
              message: 'speed test failed to start',
            }
          }
          this.groups[groupName] = { ...current, nodes: rolled }
        }
        this.error = e instanceof Error ? e.message : String(e)
        throw e
      }
    },
  },
})

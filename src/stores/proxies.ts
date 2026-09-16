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
//
// Update cost (v0.4 virtual list):
//   - Batches mutate `groups[name].nodes[child]` **in place** instead of
//     replacing the group object. Replacing it re-triggered every card in the
//     group and invalidated the (now virtualised) layout list on each of the
//     ~625 batches a 5,000-node run emits; a leaf write only re-renders the
//     card that reads that leaf.
//   - `latency_asc` order lives in `sortedCache`, rebuilt through
//     `scheduleSort()` — a 400 ms trailing throttle, with an immediate flush
//     at the boundaries of a run (start, terminal event, selection, mode flip)
//     where a lag would be visible. Without it the comparator runs O(N log N)
//     on every batch and the list reorders under the user's cursor.
//   - The comparator sorts a plain snapshot of the delay ranks, so it never
//     touches a reactive proxy while sorting.
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
   * Display order per group, valid only while `sortMode === 'latency_asc'`.
   * Replaced wholesale by `rebuildSort()`; read through `sortedChildren()`.
   * Kept as state (not a getter) so a batch storm cannot re-sort the list —
   * only `scheduleSort()` moves it, at most once per `SORT_THROTTLE_MS`.
   */
  sortedCache: Record<string, string[]>
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

/**
 * Coalescing window for `latency_asc` order rebuilds.
 *
 * The Rust engine streams one batch per `BATCH_SIZE` (8) results, so a
 * 5,000-node group would re-sort ~625 times over a single run — O(N log N)
 * each time, and the rows would visibly thrash under the cursor. Rebuilding at
 * most every 400 ms keeps the cost negligible and makes the reorder read as
 * deliberate batches.
 */
const SORT_THROTTLE_MS = 400

/**
 * Pending trailing throttle. Module-level on purpose: it must not become
 * reactive state, and a store reset may safely leave one pending — the timer
 * only reads current state when it fires.
 */
let sortTimer: ReturnType<typeof setTimeout> | null = null

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
    sortedCache: {},
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
     *
     * The order itself comes from `sortedCache` (rebuilt by `scheduleSort()`),
     * not from sorting here: this is called for every group on every layout
     * pass, and sorting per call is what made a batch storm expensive.
     */
    sortedChildren: (s) => (groupName: string): string[] => {
      const g = s.groups[groupName]
      if (!g) return []
      if (s.sortMode === 'default') return g.all
      const cached = s.sortedCache[groupName]
      // The cache is only valid for the child list it was built from. A length
      // check is O(1) and catches the real hazard — `groups` being replaced
      // wholesale without a rebuild (which `fetchProxies` does flush). Falling
      // back to mihomo's order is always safe, and the next batch or selection
      // rebuilds properly.
      //
      // Invariant worth keeping: `groups` is only ever replaced by
      // `fetchProxies`, which calls `scheduleSort(true)`; anything else that
      // wants to swap it must do the same.
      if (!cached || cached.length !== g.all.length) return g.all
      return cached
    },
  },

  actions: {
    setSortMode(mode: SortMode): void {
      this.sortMode = mode
      this.scheduleSort(true)
    },
    toggleSortMode(): void {
      this.sortMode = this.sortMode === 'default' ? 'latency_asc' : 'default'
      this.scheduleSort(true)
    },

    /**
     * Recompute every group's display order into `sortedCache`.
     *
     * Runs outside any render effect, so reading state here subscribes nothing.
     * The delay ranks are snapshotted into plain objects *before* sorting: the
     * comparator then touches no reactive proxy, which is what keeps an
     * O(N log N) rebuild over thousands of nodes cheap enough to repeat.
     */
    rebuildSort(): void {
      if (this.sortMode !== 'latency_asc') {
        // Nothing reads the cache in `default` mode; keep the same empty object
        // so clearing it does not invalidate the layout list.
        if (Object.keys(this.sortedCache).length > 0) this.sortedCache = {}
        return
      }

      const next: Record<string, string[]> = {}
      for (const [name, g] of Object.entries(this.groups)) {
        const head: string[] = []
        const tail: { name: string; rank: number; delay: number | null }[] = []
        for (const child of g.all) {
          // The selected node leads regardless of its latency, so a sort never
          // hides what the user currently has active.
          if (child === g.now) {
            head.push(child)
            continue
          }
          const info = g.nodes[child]
          tail.push({ name: child, rank: delayRank(info), delay: info?.delay ?? null })
        }
        tail.sort((a, b) => {
          if (a.rank !== b.rank) return a.rank - b.rank
          // Within the "ok" bucket, lowest delay first.
          if (a.rank === 0 && a.delay !== null && b.delay !== null) {
            const d = a.delay - b.delay
            if (d !== 0) return d
          }
          return a.name.localeCompare(b.name)
        })
        next[name] = [...head, ...tail.map((t) => t.name)]
      }
      this.sortedCache = next
    },

    /**
     * Queue an order rebuild.
     *
     * `immediate` is for the boundaries of a run — the start, the terminal
     * event, a selection, a sort-mode flip — where the user is looking at the
     * outcome and a 400 ms lag would read as a bug. Everything else (that is,
     * the batch storm in the middle) is coalesced to a trailing 400 ms edge.
     */
    scheduleSort(immediate = false): void {
      if (immediate) {
        if (sortTimer !== null) {
          clearTimeout(sortTimer)
          sortTimer = null
        }
        this.rebuildSort()
        return
      }
      // mihomo's own order needs no maintenance: the cache is already empty, and
      // scheduling a no-op rebuild per batch window would be pure overhead.
      // `setSortMode`/`toggleSortMode` flush immediately when the mode flips.
      if (this.sortMode !== 'latency_asc') return
      // Trailing throttle: the first call opens the window, and whatever is in
      // state when it closes is what gets sorted. Later results are never lost,
      // only folded into the next rebuild.
      if (sortTimer !== null) return
      sortTimer = setTimeout(() => {
        sortTimer = null
        this.rebuildSort()
      }, SORT_THROTTLE_MS)
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
        // The child lists may have changed wholesale, so any cached order is
        // stale — and in `latency_asc` mode the new group needs one.
        this.scheduleSort(true)
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
        // The selected node leads the order in `latency_asc` mode, so the
        // change has to be reflected now rather than on the next batch.
        this.scheduleSort(true)
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

      // Leaf writes, not `this.groups[name] = { ...group, nodes }`: a new group
      // object re-triggers every card in the group and invalidates the layout
      // list, which is exactly the O(N)-per-batch cost the virtual list exists
      // to remove.
      for (const r of batch.results) {
        group.nodes[r.name] = {
          status: probeStatusToDelay(r.status),
          delay: r.delayMs ?? null,
          testedAt: Date.now(),
          message: r.message ?? null,
        }
      }
      // Coalesced: a run emits one of these per 8 nodes.
      this.scheduleSort()
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
        // A cancelled run leaves nodes that were never probed. Give them a
        // definite "never measured" state — otherwise they would spin forever
        // once `testingGroups` is cleared below. In place, for the same reason
        // as `applyDelayBatch`.
        for (const [name, info] of Object.entries(group.nodes)) {
          if (info.status === 'testing') {
            group.nodes[name] = {
              status: 'idle',
              delay: null,
              testedAt: null,
              message: done.cancelled ? 'cancelled' : 'not probed',
            }
          }
        }
      }

      this.testingGroups = this.testingGroups.filter((g) => g !== done.group)
      // Terminal event: the final order has to be exact, not up to 400 ms stale.
      this.scheduleSort(true)
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
      // In place — see `applyDelayBatch` for why the group object is not replaced.
      for (const child of children) {
        group.nodes[child] = probeable.includes(child)
          ? { status: 'testing', delay: null, testedAt: null, message: null }
          : {
              status: 'unreachable',
              delay: null,
              testedAt: Date.now(),
              message: 'type cannot be delay-tested',
            }
      }

      if (!this.testingGroups.includes(groupName)) {
        this.testingGroups = [...this.testingGroups, groupName]
      }
      // Every node just became `testing`, so the order changes wholesale here:
      // doing that immediately beats a 400 ms flash of the pre-run order.
      this.scheduleSort(true)

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
          for (const child of probeable) {
            current.nodes[child] = {
              status: 'error',
              delay: null,
              testedAt: Date.now(),
              message: 'speed test failed to start',
            }
          }
          this.scheduleSort(true)
        }
        this.error = e instanceof Error ? e.message : String(e)
        throw e
      }
    },
  },
})

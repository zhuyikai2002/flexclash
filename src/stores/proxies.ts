// ============================================================================
// stores/proxies.ts — Pinia store for proxy groups, node selection, and
// concurrent delay testing.
//
// Design:
//   - `groups`  = the Selector/URLTest/Fallback/LoadBalance groups (by name).
//   - `byName`  = the raw `proxies` map from mihomo's GET /proxies (every
//                 proxy/group is keyed here, including Direct / Reject).
//   - Per-node delay info is *denormalised* into `groups[name].nodes[name]`
//     so that the UI can render "ms" badges without re-walking the tree.
//
// Concurrency (M6):
//   - The fixed worker pool size is exposed as `DELAY_CONCURRENCY` (6) so
//     a 50-node subscription doesn't fire 50 parallel delay tests. The
//     pool is also cancellable: when the group is refreshed mid-test, the
//     stale workers drop their result silently.
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
  getProxyDelay,
  selectProxy as apiSelectProxy,
} from '@/services/clash'
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
  /** Monotonic counter per group: latest speed-test run id. */
  runIds: Record<string, number>
}

const DELAY_CONCURRENCY = 6
const DELAY_TIMEOUT_MS = 5_000
const DEFAULT_TEST_URL = 'http://www.gstatic.com/generate_204'

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
})

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
        if (this.groups[groupName]) {
          this.groups[groupName].now = updated.now ?? nodeName
        }
        this.byName[groupName] = updated
      } catch (e) {
        this.error = e instanceof Error ? e.message : String(e)
        throw e
      }
    },

    /**
     * Run delay tests against every child of `groupName` using a small
     * worker pool. Updates each node's delay badge in-place. No-op if a
     * test for this group is already in flight.
     */
    async speedTestGroup(
      groupName: string,
      testUrl: string = DEFAULT_TEST_URL,
    ): Promise<void> {
      const group = this.groups[groupName]
      if (!group) throw new Error(`Unknown group: ${groupName}`)
      if (this.testingGroups.includes(groupName)) return

      this.testingGroups = [...this.testingGroups, groupName]

      // Snapshot the children so the worker queue is stable across the
      // multiple state mutations done by the workers.
      const children = [...group.all]
      for (const child of children) {
        group.nodes[child] = {
          status: 'testing',
          delay: null,
          testedAt: null,
        }
      }

      const queue: string[] = [...children]
      // Tag this run with a monotonic id so we can drop late results
      // when the group is refreshed mid-test.
      const myRun = (this.runIds[groupName] ?? 0) + 1
      this.runIds = { ...this.runIds, [groupName]: myRun }
      const isStale = () => this.runIds[groupName] !== myRun

      const results: Record<string, NodeDelayInfo> = {}

      const worker = async (): Promise<void> => {
        while (queue.length > 0) {
          if (isStale()) return
          const child = queue.shift() as string
          const childProxy = this.byName[child]
          if (!childProxy || UNTESTABLE_TYPES.has(childProxy.type)) {
            results[child] = {
              status: 'unreachable',
              delay: null,
              testedAt: Date.now(),
            }
            continue
          }
          try {
            const delay = await getProxyDelay(child, testUrl, DELAY_TIMEOUT_MS)
            if (isStale()) return
            results[child] = {
              status: delay === 0 ? 'timeout' : 'ok',
              delay: delay === 0 ? null : delay,
              testedAt: Date.now(),
            }
          } catch {
            if (isStale()) return
            results[child] = {
              status: 'error',
              delay: null,
              testedAt: Date.now(),
            }
          }
        }
      }

      const workers = Array.from(
        { length: Math.min(DELAY_CONCURRENCY, children.length) },
        () => worker(),
      )
      await Promise.all(workers)

      // Bail out if a newer run has started.
      if (isStale()) return

      // Commit results. Re-assign to keep Pinia reactive.
      const updatedNodes: Record<string, NodeDelayInfo> = { ...group.nodes }
      for (const [child, info] of Object.entries(results)) {
        updatedNodes[child] = info
      }
      this.groups[groupName] = {
        ...group,
        nodes: updatedNodes,
      }

      this.testingGroups = this.testingGroups.filter((g) => g !== groupName)
    },
  },
})

/**
 * dev/proxy-bench.ts — dev-only smoke target for the virtualised proxy list.
 *
 * WHY THIS EXISTS
 * ---------------
 * The whole contract of the virtual list is "mounted DOM is proportional to the
 * viewport, not to the subscription". That claim is about the real DOM and real
 * frame pacing, so it cannot be asserted with unit tests — and this project has
 * no JS test runner at all. So this harness mounts the **real** `ProxyGroups`
 * against a synthetic multi-thousand-node tree and reports numbers for it.
 *
 * It also replays the two costs the refactor removed, as a control:
 *   1. the old per-batch merge — clone the group's whole node map and replace
 *      the group object, which re-triggered every card in the group;
 *   2. the old per-batch full re-sort of the display order.
 * "Faster" then becomes a measurement instead of an opinion.
 *
 * HOW TO RUN
 * ----------
 *   npm run dev:web
 *   open http://127.0.0.1:5173/dev-proxy-bench.html
 *   await window.__bench.report()          // everything at once
 *
 * This file is dev-only and never shipped: Vite bundles `index.html` alone
 * unless more build inputs are configured, and this page is not one of them.
 */
import { createApp, h, nextTick } from 'vue'
import { createPinia } from 'pinia'

import '@/style.css'
import { i18n } from '@/i18n'
import ProxyGroups from '@/components/ProxyGroups.vue'
import {
  useProxiesStore,
  type NodeDelayInfo,
  type ProxyGroupState,
  type SortMode,
} from '@/stores/proxies'
import { useNoticesStore } from '@/stores/notices'
import type { DelayBatch, NodeProbe, ProbeStatus } from '@/bindings'
import type { ProxyType } from '@/types/clash'

/** Mirrors `HEADER_H` / `NODE_ROW_H` in ProxyGroups.vue so drift shows up as a
 *  non-zero `maxGapErrorPx` instead of silently overlapping rows. Keep in sync. */
const HEADER_EST = 64
const NODE_ROW_EST = 51

const STATUSES: ProbeStatus[] = ['ok', 'ok', 'ok', 'timeout', 'unreachable', 'error']

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))
const raf = () => new Promise((resolve) => requestAnimationFrame(() => resolve(null)))

// ---------------------------------------------------------------------------
// Synthetic tree
// ---------------------------------------------------------------------------

function seedGroups(sizes: number[]): Record<string, ProxyGroupState> {
  const out: Record<string, ProxyGroupState> = {}
  sizes.forEach((size, gi) => {
    const name = `Group-${gi + 1}`
    const all: string[] = []
    const nodes: Record<string, NodeDelayInfo> = {}
    for (let i = 0; i < size; i++) {
      const child = `Node-${gi + 1}-${String(i).padStart(4, '0')}`
      all.push(child)
      nodes[child] = { status: 'idle', delay: null, testedAt: null, message: null }
    }
    // Deliberate duplicate name across groups. Every group now shares one scroll
    // container, so a row keyed on the bare node name would collide — this is the
    // canary for that, and Vue reports the collision as a console warning.
    all.push('Shared-Node')
    nodes['Shared-Node'] = { status: 'idle', delay: null, testedAt: null, message: null }
    const type: ProxyType = gi === 0 ? 'Selector' : 'URLTest'
    out[name] = { name, type, now: all[0], all, nodes }
  })
  return out
}

function probesFor(children: string[], cursor: number, count: number): NodeProbe[] {
  const out: NodeProbe[] = []
  for (let i = 0; i < count; i++) {
    const name = children[(cursor + i) % children.length]
    const status = STATUSES[(cursor + i) % STATUSES.length]
    out.push({
      name,
      status,
      delayMs: status === 'ok' ? 40 + ((cursor + i) * 7) % 480 : null,
      message: status === 'ok' ? null : `synthetic ${status}`,
    })
  }
  return out
}

// ---------------------------------------------------------------------------
// Mount the real view
// ---------------------------------------------------------------------------

const host = document.getElementById('bench')
if (!host) throw new Error('bench host element missing')

/**
 * Rebuild App.vue's layout chain around the view under test:
 *
 *   main (h-full, overflow-y-auto, p-6, shifted by the 68px rail)
 *     └─ div (mx-auto max-w-7xl, space-y-6)
 *          ├─ <ProxyGroups/>
 *          └─ <footer>  ← the thing the pane height must leave room for
 *
 * Without it the pane-height arithmetic behind `PANE_H` cannot be checked at
 * all, and "the footer stays visible" would be an assertion rather than a
 * measurement.
 */
host.style.height = '100vh'
host.style.overflowY = 'auto'
host.style.padding = '24px'
host.style.marginLeft = '68px'
host.style.width = 'calc(100% - 68px)'
host.style.boxSizing = 'border-box'

const inner = document.createElement('div')
inner.style.maxWidth = '1280px'
inner.style.margin = '0 auto'
inner.style.display = 'flex'
inner.style.flexDirection = 'column'
inner.style.gap = '24px'

const groupHost = document.createElement('div')

const footerEl = document.createElement('footer')
footerEl.style.textAlign = 'center'
footerEl.style.fontSize = '12px'
footerEl.style.lineHeight = '1.5'
footerEl.style.paddingTop = '16px'
footerEl.style.color = '#52525b'
footerEl.textContent =
  'Rust owns the mihomo data plane — kernel state, logs and live rates arrive as typed events.'

inner.append(groupHost, footerEl)
host.append(inner)

const pinia = createPinia()
const app = createApp({ render: () => h(ProxyGroups) })
app.use(pinia)
app.use(i18n)
app.mount(groupHost)

const store = useProxiesStore(pinia)

// Capture Vue warnings (duplicate keys, bad props) — the canary above is only
// worth planting if something reads the result.
const consoleNoise: string[] = []
for (const level of ['warn', 'error'] as const) {
  const original = console[level].bind(console)
  console[level] = (...args: unknown[]) => {
    consoleNoise.push(`[${level}] ${args.map((a) => String(a)).join(' ')}`)
    original(...args)
  }
}

function scrollEl(): HTMLElement {
  for (const el of groupHost.querySelectorAll<HTMLElement>('div')) {
    if (getComputedStyle(el).overflowY === 'auto') return el
  }
  throw new Error('virtual list scroll container not found')
}

/** The absolutely positioned wrappers, read back from their own transform. */
function mountedRows(): { start: number; height: number; header: boolean; el: HTMLElement }[] {
  const rows: { start: number; height: number; header: boolean; el: HTMLElement }[] = []
  for (const el of groupHost.querySelectorAll<HTMLElement>('div')) {
    const m = /translateY\((-?[\d.]+)px\)/.exec(el.style.transform)
    if (!m) continue
    rows.push({
      start: Number(m[1]),
      height: el.getBoundingClientRect().height,
      header: el.firstElementChild?.tagName === 'HEADER',
      el,
    })
  }
  return rows.sort((a, b) => a.start - b.start)
}

const mountedCards = () => groupHost.querySelectorAll('div.grid > button').length

function cols(): number {
  const grid = groupHost.querySelector<HTMLElement>('div.grid')
  if (!grid) return 1
  return getComputedStyle(grid).gridTemplateColumns.split(' ').filter(Boolean).length
}

async function reseed(sizes: number[]): Promise<void> {
  store.groups = seedGroups(sizes)
  store.lastFetchAt = Date.now()
  store.loading = false
  // `fetchProxies` on mount threw its way onto the notice channel (no Tauri
  // runtime here); seeding happened afterwards, so clear the stale message.
  useNoticesStore().clearSource('proxies')
  // Mirror what `fetchProxies` does after swapping `groups`: the cached display
  // order belongs to the previous child lists. Without this the bench renders a
  // stale order — which is exactly the footgun the store now guards against.
  store.scheduleSort(true)
  await nextTick()
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/**
 * Does the pane height actually leave the footer visible inside the viewport,
 * with no outer scrollbar? This is the one claim about `PANE_H` that only the
 * surrounding layout can answer.
 */
function layoutMetrics() {
  const doc = document.documentElement
  const paneRect = scrollEl().getBoundingClientRect()
  const footerRect = footerEl.getBoundingClientRect()
  return {
    paneHeight: Math.round(paneRect.height),
    footerTop: Math.round(footerRect.top),
    footerBottom: Math.round(footerRect.bottom),
    viewportHeight: window.innerHeight,
    footerFullyVisible: footerRect.bottom <= window.innerHeight + 1,
    paneClearsFooter: paneRect.bottom <= footerRect.top + 1,
    documentScrollHeight: doc.scrollHeight,
    documentClientHeight: doc.clientHeight,
    outerScrolls: doc.scrollHeight > doc.clientHeight + 1,
  }
}

interface ViewportMetrics {
  groups: number
  nodes: number
  cols: number
  mountedRows: number
  mountedCards: number
  domElements: number
  scrollHeight: number
  clientHeight: number
}

function viewportMetrics(): ViewportMetrics {
  const el = scrollEl()
  const total = Object.values(store.groups).reduce((n, g) => n + g.all.length, 0)
  return {
    groups: Object.keys(store.groups).length,
    nodes: total,
    cols: cols(),
    mountedRows: mountedRows().length,
    mountedCards: mountedCards(),
    domElements: groupHost.querySelectorAll('*').length,
    scrollHeight: el.scrollHeight,
    clientHeight: el.clientHeight,
  }
}


/**
 * The decisive check on the fixed-height assumption: the virtualiser places each
 * row `estimateSize` apart, so if a row's real height differs from its estimate
 * the error accumulates as whitespace or overlap. Anything other than ~0 here
 * means `HEADER_H` / `NODE_ROW_H` need retuning.
 */
function geometryMetrics() {
  const rows = mountedRows()
  let maxGapError = 0
  const headerHeights: number[] = []
  const nodeHeights: number[] = []
  for (let i = 0; i < rows.length; i++) {
    const row = rows[i]
    const bucket = row.header ? headerHeights : nodeHeights
    bucket.push(Math.round(row.height * 10) / 10)
    const next = rows[i + 1]
    if (next) maxGapError = Math.max(maxGapError, Math.abs(next.start - (row.start + row.height)))
  }
  const avg = (xs: number[]) => (xs.length ? Math.round((xs.reduce((a, b) => a + b, 0) / xs.length) * 10) / 10 : 0)
  return {
    headerEstimate: HEADER_EST,
    nodeRowEstimate: NODE_ROW_EST,
    headerActualAvg: avg(headerHeights),
    nodeRowActualAvg: avg(nodeHeights),
    maxGapErrorPx: Math.round(maxGapError * 10) / 10,
    sampledRows: rows.length,
  }
}

async function scrollProbe() {
  const el = scrollEl()
  const max = el.scrollHeight - el.clientHeight
  const out: {
    at: string
    scrollTop: number
    mountedRows: number
    mountedCards: number
    coversViewport: boolean
  }[] = []
  for (const frac of [0, 0.25, 0.5, 0.75, 1]) {
    el.scrollTop = Math.round(max * frac)
    await nextTick()
    await raf()
    await raf()
    const rows = mountedRows()
    const bottom = rows.length
      ? rows[rows.length - 1].start + rows[rows.length - 1].height
      : 0
    const top = rows.length ? rows[0].start : 0
    out.push({
      at: `${Math.round(frac * 100)}%`,
      scrollTop: Math.round(el.scrollTop),
      mountedRows: rows.length,
      mountedCards: mountedCards(),
      coversViewport:
        rows.length > 0 && top <= el.scrollTop + 1 && bottom >= el.scrollTop + el.clientHeight - 1,
    })
  }
  return out
}

async function collapseProbe() {
  const el = scrollEl()
  // A group band is only mounted while it is inside the window, so start from
  // the top — otherwise this silently probes nothing (as it did at first).
  el.scrollTop = 0
  await nextTick()
  await raf()
  const before = viewportMetrics()
  const firstBand = el.querySelector<HTMLElement>('header button')
  if (!firstBand) return { error: 'no group band mounted at scrollTop 0' }
  firstBand.click()
  await nextTick()
  await raf()
  const after = viewportMetrics()
  firstBand.click() // restore
  await nextTick()
  await raf()
  return {
    mountedRowsBefore: before.mountedRows,
    mountedRowsAfter: after.mountedRows,
    domElementsBefore: before.domElements,
    domElementsAfter: after.domElements,
    // The decisive number: collapsing must *remove* the group's rows from the
    // list (scrollHeight drops by one row per chunk) rather than hide them.
    scrollHeightBefore: before.scrollHeight,
    scrollHeightAfter: after.scrollHeight,
  }
}

// ---------------------------------------------------------------------------
// Batch storm: new path vs a faithful copy of the old one
// ---------------------------------------------------------------------------

/**
 * The comparator the store used to run per batch (and, before that, per render):
 * `compareLatency`, i.e. rank then delay then name, with the selection hoisted.
 */
function legacySortOrder(groupName: string): string[] {
  const g = store.groups[groupName]
  const rank = (info: NodeDelayInfo | undefined): number => {
    if (!info) return 4
    switch (info.status) {
      case 'ok': return 0
      case 'testing': return 1
      case 'idle': return 2
      case 'unreachable': return 3
      case 'timeout': return 4
      case 'error': return 5
      default: return 6
    }
  }
  const head: string[] = []
  const tail: string[] = []
  for (const name of g.all) {
    if (name === g.now) head.push(name)
    else tail.push(name)
  }
  tail.sort((a, b) => {
    const ia = g.nodes[a]
    const ib = g.nodes[b]
    const ra = rank(ia)
    const rb = rank(ib)
    if (ra !== rb) return ra - rb
    if (ra === 0 && ia?.delay != null && ib?.delay != null) return ia.delay - ib.delay
    return a.localeCompare(b)
  })
  return [...head, ...tail]
}

/** The merge the store used to do: clone the whole node map, swap the group. */
function legacyMerge(groupName: string, results: NodeProbe[]): void {
  const group = store.groups[groupName]
  const nodes: Record<string, NodeDelayInfo> = { ...group.nodes }
  for (const r of results) {
    nodes[r.name] = {
      status: r.status === 'ok' ? 'ok' : r.status === 'timeout' ? 'timeout'
        : r.status === 'unreachable' ? 'unreachable' : 'error',
      delay: r.delayMs ?? null,
      testedAt: Date.now(),
      message: r.message ?? null,
    }
  }
  store.groups[groupName] = { ...group, nodes }
}

interface StormResult {
  variant: 'new' | 'legacy'
  mode: SortMode
  batches: number
  nodesTouched: number
  mergeMs: number
  sortMs: number
  sortCalls: number
  wallMs: number
}

/**
 * Replay one full run against `groupName` and attribute the cost.
 *
 * Wall time is paced (~10 ms per 8 results) so the store's 400 ms trailing
 * throttle behaves exactly as it does in production; the numbers that matter are
 * `mergeMs`/`sortMs`/`sortCalls`, which are paced-independent.
 */
async function storm(
  variant: 'new' | 'legacy',
  mode: SortMode,
  groupName: string,
): Promise<StormResult> {
  store.setSortMode(mode)
  store.runIds = {}
  const g = store.groups[groupName]
  const perBatch = 8
  const batches = Math.ceil(g.all.length / perBatch)

  let mergeMs = 0
  let sortMs = 0
  let sortCalls = 0

  // Count and time whatever sort work actually happens — for the new path this
  // is `rebuildSort`, for the legacy path the inline comparator below. The store
  // calls `this.rebuildSort()`, so patching the property covers both the direct
  // call and the throttled one.
  const originalRebuild = store.rebuildSort
  const mutable = store as unknown as { rebuildSort: (...a: unknown[]) => unknown }
  mutable.rebuildSort = (...args: unknown[]) => {
    const t0 = performance.now()
    const r = (originalRebuild as (...a: unknown[]) => unknown).apply(store, args)
    sortMs += performance.now() - t0
    sortCalls++
    return r
  }

  const wallStart = performance.now()
  let cursor = 0
  for (let b = 0; b < batches; b++) {
    const results = probesFor(g.all, cursor, perBatch)
    cursor += perBatch
    const t0 = performance.now()
    if (variant === 'new') {
      const batch: DelayBatch = {
        runId: 1,
        group: groupName,
        results,
        done: Math.min(cursor, g.all.length),
        total: g.all.length,
      }
      store.applyDelayBatch(batch)
    } else {
      legacyMerge(groupName, results)
      if (mode === 'latency_asc') {
        const s0 = performance.now()
        legacySortOrder(groupName)
        sortMs += performance.now() - s0
        sortCalls++
      }
    }
    mergeMs += performance.now() - t0
    // Let the trailing throttle fire as it would in a real run.
    if (b % 8 === 7) await sleep(10)
  }
  await sleep(600) // drain the pending trailing edge
  const wallMs = performance.now() - wallStart

  store.rebuildSort = originalRebuild
  return {
    variant,
    mode,
    batches,
    nodesTouched: cursor,
    mergeMs: Math.round(mergeMs * 10) / 10,
    sortMs: Math.round(sortMs * 10) / 10,
    sortCalls,
    wallMs: Math.round(wallMs),
  }
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

async function report() {
  await reseed([2000, 2000, 700, 301])
  const viewport = viewportMetrics()
  const layout = layoutMetrics()
  const geometry = geometryMetrics()
  const scroll = await scrollProbe()
  const collapse = await collapseProbe()

  const perCard = viewport.mountedCards
    ? (viewport.domElements - viewport.mountedRows) / viewport.mountedCards
    : 0
  const equivalentFullDom = Math.round(perCard * viewport.nodes + viewport.mountedRows)

  await reseed([5000, 2])
  const stormResults: StormResult[] = []
  for (const variant of ['legacy', 'new'] as const) {
    for (const mode of ['default', 'latency_asc'] as const) {
      await reseed([5000, 2])
      stormResults.push(await storm(variant, mode, 'Group-1'))
    }
  }

  await reseed([2000, 2000, 700, 301])
  return {
    viewport,
    layout,
    extrapolation: {
      elementsPerCard: Math.round(perCard * 100) / 100,
      equivalentFullPageDom: equivalentFullDom,
      reduction: `${Math.round((1 - viewport.domElements / equivalentFullDom) * 100)}%`,
    },
    geometry,
    scroll,
    collapse,
    storm: stormResults,
    consoleNoise,
  }
}

interface BenchApi {
  store: ReturnType<typeof useProxiesStore>
  reseed: (sizes: number[]) => Promise<void>
  viewportMetrics: () => ViewportMetrics
  layoutMetrics: () => unknown
  geometryMetrics: () => unknown
  scrollProbe: () => Promise<unknown>
  collapseProbe: () => Promise<unknown>
  storm: (variant: 'new' | 'legacy', mode: SortMode, group: string) => Promise<StormResult>
  report: () => Promise<unknown>
  scrollEl: () => HTMLElement
}

declare global {
  interface Window {
    __bench: BenchApi
  }
}

window.__bench = {
  store,
  reseed,
  viewportMetrics,
  layoutMetrics,
  geometryMetrics,
  scrollProbe,
  collapseProbe,
  storm,
  report,
  scrollEl,
}

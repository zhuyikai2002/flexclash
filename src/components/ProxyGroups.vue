<script setup lang="ts">
/**
 * ProxyGroups — the proxy-group tree, virtualised.
 *
 * Why a virtual list: a serious subscription can carry thousands of nodes, and
 * before this the tab mounted one DOM node per card (plus a wrapper per group).
 * Everything below exists to keep the mounted set proportional to the *viewport*
 * rather than to the subscription.
 *
 * The model is a flattened row list, not a nested one:
 *
 *   [ header(A), nodes(A,0..3), nodes(A,4..7), …, header(B), nodes(B,0..3), … ]
 *
 * Each item is one visual row — a group band, or one line of `cols` cards — so a
 * single scroll container can window group headers and grids together. The
 * alternatives do not work here: virtualising whole groups still mounts every
 * card in an expanded group, and a nested virtualiser per group needs its own
 * scroll box, which turns one list into N scrollbars. `lanes` cannot express a
 * header that spans the full width.
 *
 * The three pieces of the contract:
 *   - Row heights are fixed, so the scroll geometry is exact and no measurement
 *     pass is needed (`HEADER_H` / `NODE_ROW_H`).
 *   - `cols` is derived from the same viewport breakpoints the old grid used, and
 *     drives both the chunking here and the grid inside `ProxyNodeRow`.
 *   - Collapsing a group removes its rows from the list instead of hiding them,
 *     so a collapsed group costs no DOM at all.
 *
 * This component deliberately never reads node payloads while rendering. Rows and
 * headers subscribe to their own group (see those components); if the list read
 * `nodes` here, every speed-test batch would re-render the whole view.
 */
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useVirtualizer } from '@tanstack/vue-virtual'
import {
  AlertTriangle, ArrowDownNarrowWide, ArrowDownWideNarrow, Loader2, RefreshCw,
} from 'lucide-vue-next'

import { useProxiesStore } from '@/stores/proxies'
import { useKernelStore } from '@/stores/kernel'
import { safeListen, type UnlistenFn } from '@/utils/tauri-bridge'
import { useI18n } from '@/composables/useI18n'
import ProxyGroupHeader from '@/components/ProxyGroupHeader.vue'
import ProxyNodeRow from '@/components/ProxyNodeRow.vue'

/**
 * Fixed row heights, in px.
 *
 * Both kinds are genuinely fixed: a card truncates its name to one line and the
 * group band is one name line plus one meta line, with `rem`-sized type in a
 * webview at fixed zoom. Constant heights let the virtualiser size the scroll
 * area exactly, which is what keeps a fast drag from jittering the scrollbar —
 * measured heights would cost a second layout pass on every newly seen row.
 *
 * `HEADER_H` is **measured**, not derived: the bench harness (`npm run dev:web`
 * → `/dev-proxy-bench.html`) reports the real rendered height of both kinds and
 * the worst placement error between neighbours. It caught this at 60 vs an actual
 * 64, which would have overlapped every group band by 4px. Re-run it after
 * touching the card or band padding, and keep `maxGapErrorPx` at 0.
 */
const HEADER_H = 64
const NODE_ROW_H = 51

/** Rows kept mounted beyond the viewport, at 1–4 cards each. */
const OVERSCAN = 8

/**
 * Height of the scroll box: the viewport minus the chrome that has to stay
 * visible below it — `main`'s 24px padding top and bottom, this view's toolbar
 * (28px) plus its 12px gap, the 24px gap above the footer, and the footer's 34px.
 * Yielding to the footer is intentional: it keeps the panel from hiding it.
 */
const PANE_H = 'calc(100vh - 9.5rem)'

/** One virtual row. Group bands and card rows share the list. */
type RowItem =
  | { kind: 'header'; key: string; group: string }
  | { kind: 'nodes'; key: string; group: string; names: string[] }

const proxies = useProxiesStore()
const kernel = useKernelStore()
const { t } = useI18n()

const selecting = ref<Record<string, boolean>>({})
const collapsed = ref<Record<string, boolean>>({})
const unlistens: UnlistenFn[] = []

const scrollEl = ref<HTMLElement | null>(null)

// Always pull the freshest proxy tree on mount — a subscription activated
// while the user was on the Profiles tab won't be visible otherwise.
onMounted(() => {
  void proxies.fetchProxies()

  // Subscribe to the Rust speed-test stream before anything can start a run,
  // so batches that land while this view is unmounted are still folded in.
  // Idempotent — every entry point calls it.
  proxies.initSpeedTestStream()

  // React to profile (re)loads / kernel state flips while this view is
  // alive: a toast-triggered reload should also refresh the tree.
  void safeListen('profile://reloaded', () => { void proxies.fetchProxies() })
    .then((u) => unlistens.push(u))
    .catch(() => {})
  void safeListen('profile://list-changed', () => {
    if (kernel.isUp) void proxies.fetchProxies()
  })
    .then((u) => unlistens.push(u))
    .catch(() => {})

  // When the kernel transitions into Running while this tab is already
  // mounted (dashboard auto-start), pull the tree too.
  watch(
    () => kernel.isUp,
    (running) => { if (running) void proxies.fetchProxies() },
  )
})

onUnmounted(() => {
  for (const u of unlistens) u()
  unlistens.length = 0
  syncColsOff()
})

// ---------------------------------------------------------------------------
// Columns — mirrors the `sm: / lg: / xl:` breakpoints the grid used to carry,
// measured against the viewport because that is what those prefixes mean. The
// scroll box is narrower than the viewport (68px rail + padding), so deriving
// this from the container would shift every breakpoint.
// ---------------------------------------------------------------------------
const COLS_BREAKPOINTS = ['(min-width: 640px)', '(min-width: 1024px)', '(min-width: 1280px)']
const cols = ref(1)
const mediaQueryLists: MediaQueryList[] = []

function syncCols(): void {
  let n = 1
  for (const mql of mediaQueryLists) if (mql.matches) n++
  cols.value = n
}

function syncColsOn(): void {
  mediaQueryLists.length = 0
  for (const query of COLS_BREAKPOINTS) {
    const mql = window.matchMedia(query)
    mediaQueryLists.push(mql)
    mql.addEventListener('change', syncCols)
  }
  syncCols()
}

function syncColsOff(): void {
  for (const mql of mediaQueryLists) mql.removeEventListener('change', syncCols)
  mediaQueryLists.length = 0
}

syncColsOn()

// ---------------------------------------------------------------------------
// The flattened row list. Depends on structure only — group identity, display
// order, collapse state, column count — so a speed-test batch never rebuilds it.
// ---------------------------------------------------------------------------
const flatItems = computed<RowItem[]>(() => {
  const out: RowItem[] = []
  for (const g of proxies.selectorGroups) {
    out.push({ kind: 'header', key: `h:${g.name}`, group: g.name })
    if (collapsed.value[g.name]) continue
    // `sortedChildren` is the display order: mihomo's, or the coalesced
    // latency order maintained by the store.
    const names = proxies.sortedChildren(g.name)
    for (let i = 0; i < names.length; i += cols.value) {
      out.push({
        kind: 'nodes',
        // The group name is part of the key on purpose: with every group in one
        // container, a bare node name would collide across groups.
        key: `r:${g.name}:${i}`,
        group: g.name,
        names: names.slice(i, i + cols.value),
      })
    }
  }
  return out
})

const virtualizer = useVirtualizer({
  get count() { return flatItems.value.length },
  getScrollElement: () => scrollEl.value,
  estimateSize: (index) => (flatItems.value[index]?.kind === 'header' ? HEADER_H : NODE_ROW_H),
  getItemKey: (index) => flatItems.value[index]?.key ?? index,
  overscan: OVERSCAN,
})

/** Virtual rows paired with their item, so the template can narrow on `kind`. */
const visibleRows = computed(() =>
  virtualizer.value.getVirtualItems().map((v) => ({
    v,
    item: flatItems.value[v.index] as RowItem,
  })),
)

const totalSize = computed(() => virtualizer.value.getTotalSize())

// ---------------------------------------------------------------------------
// Interaction
// ---------------------------------------------------------------------------
async function handleSelect(group: string, node: string) {
  const key = `${group}::${node}`
  if (selecting.value[key]) return
  selecting.value = { ...selecting.value, [key]: true }
  try { await proxies.selectProxyNode(group, node) } finally {
    const next = { ...selecting.value }
    delete next[key]
    selecting.value = next
  }
}

async function handleSpeedTest(group: string) {
  try { await proxies.speedTestGroup(group) } catch { /* ignored */ }
}

function toggleCollapse(group: string) {
  collapsed.value = { ...collapsed.value, [group]: !collapsed.value[group] }
}
</script>

<template>
  <section class="flex flex-col">
    <!-- Panel head: stays put while the list scrolls, so the controls remain
         reachable no matter where the user is in a long subscription. -->
    <header class="mb-3 flex items-center justify-between">
      <h2 class="text-sm font-semibold text-zinc-200">
        {{ t('proxies.title') }}
        <span class="ml-2 text-[11px] font-mono text-zinc-500">
          {{ proxies.selectorGroups.length }} groups
        </span>
      </h2>
      <div class="flex items-center gap-2">
        <button
          @click="proxies.toggleSortMode()"
          class="inline-flex items-center gap-1.5 rounded-lg border border-white/5 bg-white/[0.04] px-2 py-1 text-[11px] text-zinc-300 hover:bg-white/[0.08] hover:border-white/10 transition-colors"
        >
          <ArrowDownNarrowWide v-if="proxies.sortMode === 'default'" class="h-3 w-3" />
          <ArrowDownWideNarrow v-else class="h-3 w-3 text-emerald-400" />
          {{ proxies.sortMode === 'default' ? t('proxies.select') : 'latency asc' }}
        </button>
        <button
          @click="proxies.fetchProxies()"
          class="inline-flex items-center gap-1 rounded-lg border border-white/5 bg-white/[0.04] px-2 py-1 text-[11px] text-zinc-300 hover:bg-white/[0.08] transition-colors"
        >
          <RefreshCw class="h-3 w-3" />
          {{ t('common.refresh') }}
        </button>
      </div>
    </header>

    <div
      v-if="proxies.loading && !proxies.lastFetchAt"
      class="rounded-2xl border border-white/5 bg-white/[0.04] p-6 text-zinc-500 text-sm flex items-center gap-2"
    >
      <Loader2 class="w-4 h-4 animate-spin" /> {{ t('common.loading') }}
    </div>

    <div
      v-else-if="proxies.selectorGroups.length === 0"
      class="rounded-2xl border border-dashed border-white/10 bg-white/[0.02] p-6 text-center"
    >
      <div class="text-zinc-400 text-sm">{{ t('proxies.empty') }}</div>
      <div class="text-zinc-600 text-xs mt-1 font-mono">
        Add <code>proxy-groups</code> to your mihomo config.
      </div>
    </div>

    <div
      v-else
      class="overflow-hidden rounded-2xl border border-white/5 bg-white/[0.04] backdrop-blur-md"
    >
      <div
        ref="scrollEl"
        class="overflow-auto overscroll-contain"
        :style="{ height: PANE_H, minHeight: '280px' }"
      >
        <div class="relative w-full" :style="{ height: `${totalSize}px` }">
          <div
            v-for="row in visibleRows"
            :key="row.item.key"
            class="absolute top-0 left-0 w-full"
            :style="{ transform: `translateY(${row.v.start}px)` }"
          >
            <ProxyGroupHeader
              v-if="row.item.kind === 'header'"
              :group="row.item.group"
              :collapsed="collapsed[row.item.group] === true"
              :first="row.v.index === 0"
              @toggle="toggleCollapse(row.item.group)"
              @test="handleSpeedTest(row.item.group)"
            />
            <ProxyNodeRow
              v-else
              :group="row.item.group"
              :names="row.item.names"
              :cols="cols"
              :selecting="selecting"
              @select="handleSelect(row.item.group, $event)"
            />
          </div>
        </div>
      </div>
    </div>

    <div
      v-if="proxies.stale && !proxies.error"
      class="mt-3 flex items-center gap-2 rounded-lg border border-amber-500/20 bg-amber-500/[0.07] p-3 text-xs text-amber-300/90"
    >
      <AlertTriangle class="h-3.5 w-3.5 shrink-0" />
      {{ t('proxies.stale_hint') }}
    </div>

    <div
      v-if="proxies.error"
      class="mt-3 rounded-lg border border-rose-500/20 bg-rose-500/10 p-3 text-xs text-rose-300 font-mono"
    >
      {{ proxies.error }}
    </div>
  </section>
</template>

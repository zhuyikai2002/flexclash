<script setup lang="ts">
// ============================================================================
// ConnectionsView.vue — M8 main panel: toolbar + virtual list.
//
// Performance contract:
//   * Fixed row height (40px) for O(1) virtualisation math.
//   * `useVirtualizer` from @tanstack/vue-virtual handles DOM recycling.
//   * Filtered rows are computed in the store; the template iterates the
//     pre-filtered array directly.
//   * The polling composable is created here so its lifecycle is bound to
//     the component instance. The parent's `tab === 'connections'` flag
//     drives `enabled`.
// ============================================================================

import { computed, ref, toRef, watch } from 'vue'
import { useVirtualizer } from '@tanstack/vue-virtual'
import {
  Filter,
  Loader2,
  Pause,
  Play,
  RefreshCw,
  Search,
  Trash2,
  XCircle,
} from 'lucide-vue-next'

import { useConnectionsStore, type PollIntervalMs } from '@/stores/connections'
import { useKernelStore } from '@/stores/kernel'
import { useConnectionMonitor } from '@/composables/useConnectionMonitor'
import ConnectionRow from '@/components/ConnectionRow.vue'
import { formatRate } from '@/composables/useTrafficStream'

const props = defineProps<{
  /** Caller must pass `true` only when the connections tab is the active tab. */
  active: boolean
}>()

const store = useConnectionsStore()
const kernel = useKernelStore()

// --- Toolbar state --------------------------------------------------------
const keyword = ref('')
watch(keyword, (v) => store.setSearchKeyword(v))

const policyFilter = ref('')
watch(policyFilter, (v) => store.setPolicyFilter(v))

const intervalChoice = ref<PollIntervalMs>(store.pollIntervalMs)
watch(intervalChoice, (v) => store.setPollInterval(v))

const intervalOptions: { label: string; value: PollIntervalMs }[] = [
  { label: '1s', value: 1000 },
  { label: '2s', value: 2000 },
  { label: '5s', value: 5000 },
]

// --- Confirm close-all modal ---------------------------------------------
const confirmOpen = ref(false)
const closing = ref(false)

function askCloseAll() {
  confirmOpen.value = true
}
async function doCloseAll() {
  if (closing.value) return
  closing.value = true
  try {
    await store.closeAll()
  } catch (e) {
    console.error('[connections] closeAll failed', e)
  } finally {
    closing.value = false
    confirmOpen.value = false
  }
}

// --- Manual refresh ------------------------------------------------------
async function manualRefresh() {
  await store.forceRefresh()
}

// --- Toggle pause --------------------------------------------------------
function togglePause() {
  store.setPaused(!store.isPaused)
}

// --- Polling lifecycle ---------------------------------------------------
useConnectionMonitor({ enabled: toRef(props, 'active') })

// --- Virtual list --------------------------------------------------------
const scrollEl = ref<HTMLElement | null>(null)
const filteredRows = computed(() => store.filteredRows)
const total = computed(() => filteredRows.value.length)

const virtualizer = useVirtualizer({
  get count() { return total.value },
  getScrollElement: () => scrollEl.value,
  estimateSize: () => 40,
  overscan: 8,
})

const virtualRows = computed(() => virtualizer.value.getVirtualItems())
const totalSize = computed(() => virtualizer.value.getTotalSize())

// --- Aggregates for the header ------------------------------------------
const totalUpText = computed(() => formatRate(
  filteredRows.value.reduce((s, r) => s + r.uploadSpeed, 0),
))
const totalDownText = computed(() => formatRate(
  filteredRows.value.reduce((s, r) => s + r.downloadSpeed, 0),
))

function onCloseRow(id: string) {
  store.closeOne(id).catch((e) => {
    console.error('[connections] closeOne failed', e)
  })
}
</script>

<template>
  <section class="bg-zinc-900 border border-zinc-800 rounded-xl overflow-hidden">
    <!-- Header -->
    <header class="flex items-center justify-between px-5 py-3 border-b border-zinc-800">
      <div class="flex items-center gap-2">
        <h2 class="text-sm font-semibold text-zinc-200">Connections</h2>
        <span class="rounded bg-zinc-800 px-1.5 py-0.5 text-[10px] text-zinc-300 font-mono">
          {{ store.totalConnections }} total · {{ total }} shown
        </span>
        <span
          v-if="store.lastError"
          class="inline-flex items-center gap-1 rounded bg-rose-950/60 border border-rose-900 px-2 py-0.5 text-[10px] text-rose-300"
          :title="store.lastError"
        >
          <XCircle class="h-3 w-3" />
          {{ store.lastError }}
        </span>
      </div>
      <div class="flex items-center gap-1 text-[11px] text-zinc-400 font-mono">
        <span class="text-emerald-400">↑ {{ totalUpText }}</span>
        <span class="text-sky-400">↓ {{ totalDownText }}</span>
      </div>
    </header>

    <!-- Toolbar -->
    <div class="flex flex-wrap items-center gap-2 px-5 py-3 border-b border-zinc-800 bg-zinc-900/40">
      <div class="relative flex-1 min-w-[200px] max-w-md">
        <Search class="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-zinc-500" />
        <input
          v-model="keyword"
          type="text"
          placeholder="Filter by host, IP, process, rule…"
          class="w-full pl-7 pr-2 py-1.5 rounded-md bg-zinc-950 border border-zinc-800 text-xs text-zinc-100 placeholder-zinc-500 focus:outline-none focus:ring-1 focus:ring-indigo-500 focus:border-indigo-500"
        />
      </div>

      <div class="flex items-center gap-1.5">
        <Filter class="h-3.5 w-3.5 text-zinc-500" />
        <select
          v-model="policyFilter"
          class="rounded-md bg-zinc-950 border border-zinc-800 text-xs text-zinc-100 px-2 py-1.5 focus:outline-none focus:ring-1 focus:ring-indigo-500"
        >
          <option value="">All policies</option>
          <option v-for="p in store.distinctPolicies" :key="p" :value="p">{{ p }}</option>
        </select>
      </div>

      <div class="flex items-center rounded-md border border-zinc-800 overflow-hidden">
        <button
          v-for="opt in intervalOptions"
          :key="opt.value"
          type="button"
          class="px-2.5 py-1.5 text-xs font-mono transition-colors"
          :class="intervalChoice === opt.value
            ? 'bg-indigo-600 text-white'
            : 'bg-zinc-950 text-zinc-400 hover:bg-zinc-800'"
          :disabled="store.isPaused"
          @click="intervalChoice = opt.value"
        >
          {{ opt.label }}
        </button>
        <button
          type="button"
          class="px-2 py-1.5 text-xs transition-colors border-l border-zinc-800"
          :class="store.isPaused
            ? 'bg-amber-600 text-white'
            : 'bg-zinc-950 text-zinc-400 hover:bg-zinc-800'"
          :title="store.isPaused ? 'Resume polling' : 'Pause polling'"
          @click="togglePause"
        >
          <Pause v-if="!store.isPaused" class="h-3.5 w-3.5" />
          <Play v-else class="h-3.5 w-3.5" />
        </button>
      </div>

      <button
        type="button"
        :disabled="!kernel.isRunning"
        class="inline-flex items-center gap-1.5 rounded-md bg-zinc-800 hover:bg-zinc-700 disabled:opacity-40 text-zinc-100 px-2.5 py-1.5 text-xs transition-colors"
        title="Refresh now"
        @click="manualRefresh"
      >
        <RefreshCw class="h-3.5 w-3.5" :class="store.busy ? 'animate-spin' : ''" />
        Refresh
      </button>

      <button
        type="button"
        :disabled="!store.rows.length || closing"
        class="inline-flex items-center gap-1.5 rounded-md bg-rose-700 hover:bg-rose-600 disabled:opacity-40 text-white px-2.5 py-1.5 text-xs transition-colors"
        title="Close all connections"
        @click="askCloseAll"
      >
        <Trash2 class="h-3.5 w-3.5" />
        Close all
      </button>
    </div>

    <!-- Column header -->
    <div class="flex items-center gap-3 px-3 py-1.5 bg-zinc-950/60 border-b border-zinc-800 text-[10px] text-zinc-500 uppercase tracking-wider font-semibold">
      <div class="flex-1 min-w-0">Host / IP</div>
      <div class="w-28 shrink-0">Process</div>
      <div class="w-24 shrink-0">Net · Type</div>
      <div class="w-56 shrink-0">Rule</div>
      <div class="w-40 shrink-0">Chain</div>
      <div class="w-44 shrink-0">Rate</div>
      <div class="w-20 shrink-0 text-right">Total</div>
      <div class="w-24 shrink-0">Policy</div>
      <div class="w-8 shrink-0"></div>
    </div>

    <!-- Virtual scroller -->
    <div
      ref="scrollEl"
      class="overflow-auto"
      style="height: 480px"
    >
      <div
        v-if="!kernel.isRunning"
        class="flex items-center justify-center h-full text-zinc-500 text-sm"
      >
        Kernel is not running — start the Mihomo sidecar to see live connections.
      </div>
      <div
        v-else-if="total === 0 && !store.lastError"
        class="flex items-center justify-center h-full text-zinc-500 text-sm"
      >
        <Loader2 class="h-4 w-4 animate-spin mr-2" />
        Waiting for first snapshot…
      </div>
      <div
        v-else-if="total === 0 && store.lastError"
        class="flex items-center justify-center h-full text-rose-300 text-sm px-6 text-center"
      >
        {{ store.lastError }}
      </div>
      <div
        v-else
        :style="{ height: `${totalSize}px`, position: 'relative', width: '100%' }"
      >
        <div
          v-for="vrow in virtualRows"
          :key="vrow.index"
          :style="{
            position: 'absolute',
            top: 0,
            left: 0,
            width: '100%',
            transform: `translateY(${vrow.start}px)`,
          }"
        >
          <ConnectionRow
            :row="filteredRows[vrow.index]"
            @close="onCloseRow"
          />
        </div>
      </div>
    </div>

    <!-- Close-all confirm modal -->
    <Teleport to="body">
      <div
        v-if="confirmOpen"
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
        @click.self="confirmOpen = false"
      >
        <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-5 max-w-sm w-full mx-4 shadow-2xl">
          <h3 class="text-base font-semibold text-zinc-100">Close all connections?</h3>
          <p class="mt-2 text-sm text-zinc-400">
            This will drop <span class="font-mono text-zinc-200">{{ store.totalConnections }}</span>
            active connection(s). Open TCP sockets will reset, which may interrupt downloads
            and cause some apps to reconnect.
          </p>
          <div class="mt-4 flex justify-end gap-2">
            <button
              type="button"
              class="rounded-md bg-zinc-800 hover:bg-zinc-700 text-zinc-200 px-3 py-1.5 text-sm transition-colors"
              @click="confirmOpen = false"
            >
              Cancel
            </button>
            <button
              type="button"
              :disabled="closing"
              class="inline-flex items-center gap-1.5 rounded-md bg-rose-600 hover:bg-rose-500 disabled:opacity-60 text-white px-3 py-1.5 text-sm transition-colors"
              @click="doCloseAll"
            >
              <Loader2 v-if="closing" class="h-3.5 w-3.5 animate-spin" />
              <Trash2 v-else class="h-3.5 w-3.5" />
              Yes, close all
            </button>
          </div>
        </div>
      </div>
    </Teleport>
  </section>
</template>

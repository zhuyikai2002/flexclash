<script setup lang="ts">
/**
 * ConnectionsView.vue — M8 main panel: toolbar + virtual list.
 * (Style refactor only; virtualization math untouched.)
 */
import { computed, ref, toRef, watch } from 'vue'
import { useVirtualizer } from '@tanstack/vue-virtual'
import {
  Filter, Loader2, Pause, Play, RefreshCw, Search, Trash2, XCircle,
} from 'lucide-vue-next'

import { useConnectionsStore, type PollIntervalMs } from '@/stores/connections'
import { useKernelStore } from '@/stores/kernel'
import { useConnectionMonitor } from '@/composables/useConnectionMonitor'
import ConnectionRow from '@/components/ConnectionRow.vue'
import { formatRate } from '@/composables/useTrafficStream'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{ active: boolean }>()

const store = useConnectionsStore()
const kernel = useKernelStore()
const { t } = useI18n()

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

const confirmOpen = ref(false)
const closing = ref(false)
function askCloseAll() { confirmOpen.value = true }
async function doCloseAll() {
  if (closing.value) return
  closing.value = true
  try { await store.closeAll() } catch (e) { console.error('[connections] closeAll failed', e) } finally {
    closing.value = false
    confirmOpen.value = false
  }
}

async function manualRefresh() { await store.forceRefresh() }
function togglePause() { store.setPaused(!store.isPaused) }

useConnectionMonitor({ enabled: toRef(props, 'active') })

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

const totalUpText = computed(() => formatRate(
  filteredRows.value.reduce((s, r) => s + r.uploadSpeed, 0),
))
const totalDownText = computed(() => formatRate(
  filteredRows.value.reduce((s, r) => s + r.downloadSpeed, 0),
))

function onCloseRow(id: string) {
  store.closeOne(id).catch((e) => { console.error('[connections] closeOne failed', e) })
}
</script>

<template>
  <section class="overflow-hidden rounded-2xl border border-white/5 bg-white/[0.04] backdrop-blur-md">
    <header class="flex items-center justify-between px-5 py-3 border-b border-white/5">
      <div class="flex items-center gap-2">
        <h2 class="text-sm font-semibold text-zinc-100">{{ t('connections.title') }}</h2>
        <span class="rounded-md bg-indigo-500/20 px-1.5 py-0.5 text-[10px] font-mono text-indigo-200">
          {{ store.totalConnections }} total · {{ total }} shown
        </span>
        <span
          v-if="store.lastError"
          class="inline-flex items-center gap-1 rounded-md bg-rose-500/10 border border-rose-500/20 px-2 py-0.5 text-[10px] text-rose-300"
          :title="store.lastError"
        >
          <XCircle class="h-3 w-3" />
          {{ store.lastError }}
        </span>
      </div>
      <div class="flex items-center gap-3 text-[11px] font-mono">
        <span class="text-emerald-400">↑ {{ totalUpText }}</span>
        <span class="text-indigo-400">↓ {{ totalDownText }}</span>
      </div>
    </header>

    <div class="flex flex-wrap items-center gap-2 px-5 py-3 border-b border-white/5 bg-white/[0.02]">
      <div class="relative flex-1 min-w-[200px] max-w-md">
        <Search class="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-zinc-500" />
        <input
          v-model="keyword"
          type="text"
          :placeholder="t('stats.rules.search_placeholder')"
          class="w-full pl-7 pr-2 py-1.5 rounded-lg bg-zinc-950/40 border border-white/5 text-xs text-zinc-100 placeholder-zinc-500 focus:outline-none focus:ring-1 focus:ring-indigo-500/50 focus:border-indigo-400/30 transition-colors"
        />
      </div>

      <div class="flex items-center gap-1.5">
        <Filter class="h-3.5 w-3.5 text-zinc-500" />
        <select
          v-model="policyFilter"
          class="rounded-lg bg-zinc-950/40 border border-white/5 text-xs text-zinc-100 px-2 py-1.5 focus:outline-none focus:ring-1 focus:ring-indigo-500/50"
        >
          <option value="">{{ t('stats.rules.all_types') }}</option>
          <option v-for="p in store.distinctPolicies" :key="p" :value="p">{{ p }}</option>
        </select>
      </div>

      <div class="flex items-center rounded-lg border border-white/5 overflow-hidden">
        <button
          v-for="opt in intervalOptions"
          :key="opt.value"
          type="button"
          class="px-2.5 py-1.5 text-xs font-mono transition-colors"
          :class="intervalChoice === opt.value
            ? 'bg-indigo-500/20 text-indigo-200'
            : 'text-zinc-400 hover:bg-white/5'"
          :disabled="store.isPaused"
          @click="intervalChoice = opt.value"
        >
          {{ opt.label }}
        </button>
        <button
          type="button"
          class="px-2 py-1.5 text-xs transition-colors border-l border-white/5"
          :class="store.isPaused
            ? 'bg-amber-500/20 text-amber-200'
            : 'text-zinc-400 hover:bg-white/5'"
          :title="store.isPaused ? t('common.start') : t('common.stop')"
          @click="togglePause"
        >
          <Pause v-if="!store.isPaused" class="h-3.5 w-3.5" />
          <Play v-else class="h-3.5 w-3.5" />
        </button>
      </div>

      <button
        type="button"
        :disabled="!kernel.isRunning"
        class="inline-flex items-center gap-1.5 rounded-lg border border-white/5 bg-white/[0.04] hover:bg-white/[0.08] disabled:opacity-40 text-zinc-100 px-2.5 py-1.5 text-xs transition-colors"
        @click="manualRefresh"
      >
        <RefreshCw class="h-3.5 w-3.5" :class="store.busy ? 'animate-spin' : ''" />
        {{ t('common.refresh') }}
      </button>

      <button
        type="button"
        :disabled="!store.rows.length || closing"
        class="inline-flex items-center gap-1.5 rounded-lg bg-rose-500/80 hover:bg-rose-500 disabled:opacity-40 text-white px-2.5 py-1.5 text-xs transition-colors"
        @click="askCloseAll"
      >
        <Trash2 class="h-3.5 w-3.5" />
        {{ t('connections.disconnect_all') }}
      </button>
    </div>

    <div class="flex items-center gap-3 px-3 py-1.5 bg-white/[0.02] border-b border-white/5 text-[10px] text-zinc-500 uppercase tracking-wider font-semibold">
      <div class="flex-1 min-w-0">{{ t('connections.columns.host') }}</div>
      <div class="w-28 shrink-0">{{ t('connections.columns.process') }}</div>
      <div class="w-24 shrink-0">{{ t('connections.columns.network') }} · {{ t('connections.columns.type') }}</div>
      <div class="w-56 shrink-0">{{ t('connections.columns.rule') }}</div>
      <div class="w-40 shrink-0">Chain</div>
      <div class="w-44 shrink-0">{{ t('connections.columns.time') }} / {{ t('dashboard.traffic.upload') }} / {{ t('dashboard.traffic.download') }}</div>
      <div class="w-20 shrink-0 text-right">Sum</div>
      <div class="w-24 shrink-0">{{ t('connections.columns.rule') }}</div>
      <div class="w-8 shrink-0"></div>
    </div>

    <div ref="scrollEl" class="overflow-auto" style="height: 480px">
      <div
        v-if="!kernel.isRunning"
        class="flex items-center justify-center h-full text-zinc-500 text-sm px-6 text-center"
      >
        Start the kernel to see live connections.
      </div>
      <div
        v-else-if="total === 0 && !store.lastError"
        class="flex items-center justify-center h-full text-zinc-500 text-sm"
      >
        <Loader2 class="h-4 w-4 animate-spin mr-2" />
        {{ t('common.loading') }}
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
            top: 0, left: 0, width: '100%',
            transform: `translateY(${vrow.start}px)`,
          }"
        >
          <ConnectionRow :row="filteredRows[vrow.index]" @close="onCloseRow" />
        </div>
      </div>
    </div>

    <Teleport to="body">
      <div
        v-if="confirmOpen"
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
        @click.self="confirmOpen = false"
      >
        <div class="rounded-2xl border border-white/10 bg-zinc-900/95 backdrop-blur-xl p-5 max-w-sm w-full mx-4 shadow-2xl">
          <h3 class="text-base font-semibold text-zinc-100">
            {{ t('connections.disconnect_confirm_title') }}
          </h3>
          <p class="mt-2 text-sm text-zinc-400">
            {{ t('connections.disconnect_confirm_body', { n: store.totalConnections }) }}
          </p>
          <div class="mt-4 flex justify-end gap-2">
            <button
              type="button"
              class="rounded-lg border border-white/5 bg-white/[0.04] hover:bg-white/[0.08] text-zinc-200 px-3 py-1.5 text-sm transition-colors"
              @click="confirmOpen = false"
            >
              {{ t('common.cancel') }}
            </button>
            <button
              type="button"
              :disabled="closing"
              class="inline-flex items-center gap-1.5 rounded-lg bg-rose-500 hover:bg-rose-400 disabled:opacity-60 text-white px-3 py-1.5 text-sm transition-colors"
              @click="doCloseAll"
            >
              <Loader2 v-if="closing" class="h-3.5 w-3.5 animate-spin" />
              <Trash2 v-else class="h-3.5 w-3.5" />
              {{ t('connections.disconnect_confirm_ok') }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>
  </section>
</template>

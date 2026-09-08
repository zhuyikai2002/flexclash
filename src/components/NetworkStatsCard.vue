<script setup lang="ts">
/**
 * NetworkStatsCard.vue — Phase 8 dashboard micro-card.
 *
 *   ┌─────────────────────────────────────────────┐
 *   │ 网络连接                       查看连接 →   │
 *   │   ┌────┐  ┌──────────────┐  ┌─────────────┐ │
 *   │   │ 42 │  │  ⬆ 12.3 MB   │  │  ⬇ 87.1 MB  │ │
 *   │   │活跃│  │    累计上行   │  │   累计下行   │ │
 *   │   └────┘  └──────────────┘  └─────────────┘ │
 *   └─────────────────────────────────────────────┘
 *
 * Pure presentation: reads from the existing `useConnectionsStore`
 * (populated by M7's WebSocket subscription).  No new IPC surface.
 */
import { computed } from 'vue'
import { Plug, ArrowUp, ArrowDown, ChevronRight, Hash } from 'lucide-vue-next'
import { useI18n } from '@/composables/useI18n'
import { useConnectionsStore } from '@/stores/connections'
import { formatBytes } from '@/utils/format'

const { t } = useI18n()
const conns = useConnectionsStore()

const activeCount = computed(() => conns.totalConnections)
const totalUp = computed(() => conns.uploadTotal)
const totalDown = computed(() => conns.downloadTotal)

const emit = defineEmits<{ openConnections: [] }>()
</script>

<template>
  <section
    class="rounded-2xl border border-white/5 bg-white/[0.04] backdrop-blur-md p-5 space-y-4"
  >
    <header class="flex items-center justify-between">
      <h2 class="text-sm font-semibold text-zinc-100">
        {{ t('dashboard.network_stats.title') }}
      </h2>
      <button
        type="button"
        @click="emit('openConnections')"
        class="inline-flex items-center gap-1 text-[11px] text-sky-400 hover:text-sky-300 transition-colors"
      >
        {{ t('dashboard.network_stats.view_connections') }}
        <ChevronRight class="w-3 h-3" />
      </button>
    </header>

    <div class="grid grid-cols-3 gap-3">
      <!-- Active connections -->
      <div class="rounded-xl border border-white/5 bg-white/[0.03] p-4 flex flex-col gap-1.5">
        <div class="flex items-center gap-1.5 text-[10px] uppercase tracking-wider text-zinc-500 font-semibold">
          <Plug class="w-3 h-3 text-sky-400" />
          {{ t('dashboard.network_stats.active_connections') }}
        </div>
        <div class="text-2xl font-mono text-zinc-100 tabular-nums leading-none">
          {{ activeCount.toLocaleString() }}
        </div>
        <div class="text-[10px] text-zinc-500 inline-flex items-center gap-1">
          <Hash class="w-2.5 h-2.5" />
          live
        </div>
      </div>

      <!-- Total upload -->
      <div class="rounded-xl border border-white/5 bg-white/[0.03] p-4 flex flex-col gap-1.5">
        <div class="flex items-center gap-1.5 text-[10px] uppercase tracking-wider text-zinc-500 font-semibold">
          <ArrowUp class="w-3 h-3 text-indigo-400" />
          {{ t('dashboard.network_stats.total_upload') }}
        </div>
        <div class="text-2xl font-mono text-zinc-100 tabular-nums leading-none">
          {{ formatBytes(totalUp) }}
        </div>
        <div class="text-[10px] text-zinc-500">cumulative</div>
      </div>

      <!-- Total download -->
      <div class="rounded-xl border border-white/5 bg-white/[0.03] p-4 flex flex-col gap-1.5">
        <div class="flex items-center gap-1.5 text-[10px] uppercase tracking-wider text-zinc-500 font-semibold">
          <ArrowDown class="w-3 h-3 text-emerald-400" />
          {{ t('dashboard.network_stats.total_download') }}
        </div>
        <div class="text-2xl font-mono text-zinc-100 tabular-nums leading-none">
          {{ formatBytes(totalDown) }}
        </div>
        <div class="text-[10px] text-zinc-500">cumulative</div>
      </div>
    </div>
  </section>
</template>

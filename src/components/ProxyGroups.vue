<script setup lang="ts">
/**
 * ProxyGroups — adaptive grid of node cards, grouped by selector type.
 *
 * Phase 4 redesign:
 *   - Each group is a section header + responsive grid (1→2→3→4 cols)
 *   - Each node is a small card:
 *       * left:  node name (truncate, tooltip)
 *       * right: latency pill (rounded-full, color-graded)
 *       * selected: sky-500/50 ring + bg + checkmark
 *       * hover:   lift -0.5, transition 200ms
 *   - Speed test is per-group, not per-node
 *   - Sort / refresh controls sit in the section header (sticky)
 */
import { onMounted, ref } from 'vue'
import {
  CheckCircle2, Loader2, Zap,
  ArrowDownNarrowWide, ArrowDownWideNarrow, RefreshCw, ChevronDown, ChevronRight,
} from 'lucide-vue-next'
import { useProxiesStore, type DelayStatus } from '@/stores/proxies'
import { useI18n } from '@/composables/useI18n'

const proxies = useProxiesStore()
const { t } = useI18n()
const selecting = ref<Record<string, boolean>>({})
const collapsed = ref<Record<string, boolean>>({})

onMounted(() => {
  if (!proxies.lastFetchAt) {
    void proxies.fetchProxies()
  }
})

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

function latencyText(delay: number | null, status: DelayStatus): string {
  if (status === 'testing') return '…'
  if (status === 'unreachable') return t('proxies.latency.failed')
  if (status === 'timeout') return t('proxies.latency.timeout')
  if (status === 'error') return 'err'
  if (status === 'ok' && delay !== null) return t('proxies.latency.ms', { n: delay })
  return '—'
}

function latencyPillClass(delay: number | null, status: DelayStatus): string {
  if (status === 'testing') return 'bg-amber-500/15 text-amber-300 ring-1 ring-amber-400/20'
  if (status === 'ok' && delay !== null) {
    if (delay < 200) return 'bg-emerald-500/15 text-emerald-300 ring-1 ring-emerald-400/20'
    if (delay < 500) return 'bg-yellow-500/15 text-yellow-300 ring-1 ring-yellow-400/20'
    return 'bg-orange-500/15 text-orange-300 ring-1 ring-orange-400/20'
  }
  if (status === 'timeout' || status === 'error') return 'bg-rose-500/15 text-rose-300 ring-1 ring-rose-400/20'
  if (status === 'unreachable') return 'bg-zinc-500/15 text-zinc-500 ring-1 ring-white/5'
  return 'bg-white/5 text-zinc-500 ring-1 ring-white/5'
}

function groupTypeLabel(type: string): string {
  const known: Record<string, string> = {
    Selector: 'Selector', Fallback: 'Fallback', URLTest: 'URL Test',
    LoadBalance: 'Load Balance', Relay: 'Relay', DIRECT: 'Direct', REJECT: 'Reject',
  }
  return known[type] ?? type
}
</script>

<template>
  <section class="space-y-4">
    <header class="flex items-center justify-between">
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

    <div v-else class="space-y-4">
      <article
        v-for="group in proxies.selectorGroups"
        :key="group.name"
        class="rounded-2xl border border-white/5 bg-white/[0.04] backdrop-blur-md overflow-hidden"
      >
        <!-- Group header (sticky-like: solid bg, sharp border) -->
        <header
          class="flex items-center justify-between gap-2 px-4 py-3 border-b border-white/5 bg-white/[0.02]"
        >
          <button
            class="flex items-center gap-2 min-w-0 text-left"
            @click="toggleCollapse(group.name)"
          >
            <ChevronDown
              v-if="!collapsed[group.name]"
              class="h-3.5 w-3.5 text-zinc-500 shrink-0 transition-transform"
            />
            <ChevronRight
              v-else
              class="h-3.5 w-3.5 text-zinc-500 shrink-0 transition-transform"
            />
            <div class="min-w-0">
              <div class="text-sm font-semibold text-zinc-100 truncate flex items-center gap-1.5">
                {{ group.name }}
                <span
                  v-if="group.now"
                  class="inline-flex items-center gap-1 rounded-md bg-sky-500/15 px-1.5 py-0.5 text-[10px] font-mono text-sky-200 truncate max-w-[180px]"
                >
                  <CheckCircle2 class="h-2.5 w-2.5 shrink-0" />
                  <span class="truncate">{{ group.now }}</span>
                </span>
              </div>
              <div class="text-[10px] uppercase tracking-wider text-zinc-500 font-mono">
                {{ groupTypeLabel(group.type) }} · {{ group.all.length }} nodes
              </div>
            </div>
          </button>
          <button
            @click="handleSpeedTest(group.name)"
            :disabled="proxies.isGroupTesting(group.name)"
            class="inline-flex items-center gap-1.5 rounded-lg bg-indigo-500/20 px-2.5 py-1 text-[11px] text-indigo-200 hover:bg-indigo-500/30 disabled:opacity-50 transition-colors"
          >
            <Loader2 v-if="proxies.isGroupTesting(group.name)" class="w-3 h-3 animate-spin" />
            <Zap v-else class="w-3 h-3" />
            {{ t('proxies.test') }}
          </button>
        </header>

        <!-- Node grid (1→2→3→4 cols responsive) -->
        <div
          v-show="!collapsed[group.name]"
          class="p-3 grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-2.5"
        >
          <button
            v-for="node in proxies.sortedChildren(group.name)"
            :key="node"
            @click="handleSelect(group.name, node)"
            :disabled="
              proxies.isGroupTesting(group.name) ||
              selecting[`${group.name}::${node}`] === true
            "
            :title="node"
            :class="[
              'group relative flex items-center gap-2 rounded-xl border px-3 py-2.5 text-left transition-all duration-200',
              'hover:-translate-y-0.5 hover:shadow-md active:translate-y-0 active:scale-[0.99] disabled:opacity-50 disabled:hover:translate-y-0',
              group.now === node
                ? 'border-sky-400/50 bg-sky-500/10 ring-1 ring-sky-400/30 text-zinc-50 shadow-md shadow-sky-500/10'
                : 'border-white/5 bg-zinc-950/30 text-zinc-200 hover:border-white/15 hover:bg-white/[0.05]'
            ]"
          >
            <CheckCircle2
              v-if="group.now === node"
              class="h-3.5 w-3.5 text-sky-300 shrink-0"
            />
            <span
              v-else
              class="h-3.5 w-3.5 shrink-0"
              aria-hidden="true"
            ></span>
            <span class="text-[13px] font-medium truncate flex-1 leading-tight">
              {{ node }}
            </span>
            <Loader2
              v-if="selecting[`${group.name}::${node}`] === true"
              class="h-3 w-3 animate-spin text-sky-300 shrink-0"
            />
            <span
              v-else
              :class="[
                'inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-mono font-medium shrink-0',
                latencyPillClass(group.nodes[node]?.delay ?? null, group.nodes[node]?.status ?? 'idle')
              ]"
            >
              <Loader2
                v-if="group.nodes[node]?.status === 'testing'"
                class="h-2.5 w-2.5 animate-spin"
              />
              {{ latencyText(group.nodes[node]?.delay ?? null, group.nodes[node]?.status ?? 'idle') }}
            </span>
          </button>
        </div>
      </article>
    </div>

    <div
      v-if="proxies.error"
      class="rounded-lg border border-rose-500/20 bg-rose-500/10 p-3 text-xs text-rose-300 font-mono"
    >
      {{ proxies.error }}
    </div>
  </section>
</template>

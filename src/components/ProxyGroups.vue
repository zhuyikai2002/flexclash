<script setup lang="ts">
/**
 * ProxyGroups — selector groups + speed test + sort toggle.
 *
 * Visual:
 *   - Group card: glass surface (white/4)
 *   - Node button: hover lift (`scale-[1.01]`), accent ring on selected
 *   - Latency pill: small rounded-full badge color-coded by threshold
 */
import { onMounted, ref } from 'vue'
import {
  CheckCircle2, Loader2, Zap,
  ArrowDownNarrowWide, ArrowDownWideNarrow, RefreshCw,
} from 'lucide-vue-next'
import { useProxiesStore, type DelayStatus } from '@/stores/proxies'
import { useI18n } from '@/composables/useI18n'

const proxies = useProxiesStore()
const { t } = useI18n()
const selecting = ref<Record<string, boolean>>({})

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

function latencyText(delay: number | null, status: DelayStatus): string {
  if (status === 'testing') return '…'
  if (status === 'unreachable') return t('proxies.latency.failed')
  if (status === 'timeout') return t('proxies.latency.timeout')
  if (status === 'error') return 'err'
  if (status === 'ok' && delay !== null) return t('proxies.latency.ms', { n: delay })
  return '—'
}

function latencyPillClass(delay: number | null, status: DelayStatus): string {
  if (status === 'testing') return 'bg-amber-500/15 text-amber-300'
  if (status === 'ok' && delay !== null) {
    if (delay < 200) return 'bg-emerald-500/15 text-emerald-300'
    if (delay < 500) return 'bg-yellow-500/15 text-yellow-300'
    return 'bg-orange-500/15 text-orange-300'
  }
  if (status === 'timeout' || status === 'error') return 'bg-rose-500/15 text-rose-300'
  if (status === 'unreachable') return 'bg-zinc-500/15 text-zinc-500'
  return 'bg-white/5 text-zinc-500'
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
  <section class="space-y-3">
    <div class="flex items-center justify-between">
      <h2 class="text-sm font-semibold text-zinc-200">
        {{ t('proxies.title') }}
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
    </div>

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

    <div v-else class="space-y-3">
      <article
        v-for="group in proxies.selectorGroups"
        :key="group.name"
        class="rounded-2xl border border-white/5 bg-white/[0.04] p-4 backdrop-blur-md"
      >
        <div class="mb-3 flex items-center justify-between gap-2">
          <div class="min-w-0">
            <div class="text-sm font-semibold text-zinc-100 truncate">
              {{ group.name }}
            </div>
            <div class="text-[10px] uppercase tracking-wider text-zinc-500 font-mono">
              {{ groupTypeLabel(group.type) }}
            </div>
          </div>
          <button
            @click="handleSpeedTest(group.name)"
            :disabled="proxies.isGroupTesting(group.name)"
            class="inline-flex items-center gap-1.5 rounded-lg bg-indigo-500/20 px-2.5 py-1 text-[11px] text-indigo-200 hover:bg-indigo-500/30 disabled:opacity-50 transition-colors"
          >
            <Loader2 v-if="proxies.isGroupTesting(group.name)" class="w-3 h-3 animate-spin" />
            <Zap v-else class="w-3 h-3" />
            {{ t('proxies.test') }}
          </button>
        </div>

        <div class="grid grid-cols-2 sm:grid-cols-3 gap-2">
          <button
            v-for="node in proxies.sortedChildren(group.name)"
            :key="node"
            @click="handleSelect(group.name, node)"
            :disabled="
              proxies.isGroupTesting(group.name) ||
              selecting[`${group.name}::${node}`] === true
            "
            :class="[
              'group relative flex flex-col items-start gap-1.5 rounded-xl border px-3 py-2.5 text-left transition-all duration-150 disabled:opacity-50',
              'hover:scale-[1.01] active:scale-[0.99]',
              group.now === node
                ? 'border-indigo-400/40 bg-indigo-500/15 ring-1 ring-indigo-400/30 text-indigo-100 shadow-md shadow-indigo-500/10'
                : 'border-white/5 bg-zinc-950/30 text-zinc-200 hover:border-white/15 hover:bg-white/[0.04]'
            ]"
          >
            <div class="flex w-full min-w-0 items-center gap-1.5">
              <CheckCircle2
                v-if="group.now === node"
                class="h-3.5 w-3.5 text-indigo-300 shrink-0"
              />
              <span class="text-sm font-medium truncate flex-1">{{ node }}</span>
              <Loader2
                v-if="selecting[`${group.name}::${node}`] === true"
                class="h-3 w-3 animate-spin text-indigo-300 shrink-0"
              />
            </div>
            <div class="flex items-center gap-1">
              <Loader2
                v-if="group.nodes[node]?.status === 'testing'"
                class="h-2.5 w-2.5 animate-spin text-amber-300"
              />
              <span
                :class="[
                  'inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-mono font-medium',
                  latencyPillClass(group.nodes[node]?.delay ?? null, group.nodes[node]?.status ?? 'idle')
                ]"
              >
                {{ latencyText(group.nodes[node]?.delay ?? null, group.nodes[node]?.status ?? 'idle') }}
              </span>
            </div>
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

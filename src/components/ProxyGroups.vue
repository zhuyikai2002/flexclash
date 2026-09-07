<script setup lang="ts">
// ============================================================================
// ProxyGroups — selector groups + speed test + sort toggle.
//
// M6 changes:
//   - Each group header now exposes a sort toggle (Default ⇄ Latency asc).
//   - Node list uses `sortedChildren` so latency mode reorders on the fly
//     without re-fetching from mihomo.
//   - During a test, every node button shows a spinner in place of the
//     delay badge (skeleton behaviour).
// ============================================================================

import { onMounted, ref } from 'vue'
import {
  CheckCircle2,
  Loader2,
  Zap,
  ArrowDownNarrowWide,
  ArrowDownWideNarrow,
} from 'lucide-vue-next'
import { useProxiesStore, type DelayStatus } from '@/stores/proxies'

const proxies = useProxiesStore()
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
  try {
    await proxies.selectProxyNode(group, node)
  } catch {
    // store.error already set
  } finally {
    const next = { ...selecting.value }
    delete next[key]
    selecting.value = next
  }
}

async function handleSpeedTest(group: string) {
  try {
    await proxies.speedTestGroup(group)
  } catch {
    // ignored
  }
}

function delayText(delay: number | null, status: DelayStatus): string {
  if (status === 'testing') return '…'
  if (status === 'idle') return '—'
  if (status === 'unreachable') return 'N/A'
  if (status === 'timeout') return 'timeout'
  if (status === 'error') return 'err'
  if (status === 'ok' && delay !== null) return `${delay} ms`
  return '—'
}

function delayColorClass(delay: number | null, status: DelayStatus): string {
  if (status === 'testing') return 'text-amber-400'
  if (status === 'ok' && delay !== null) {
    if (delay < 200) return 'text-emerald-400'
    if (delay < 500) return 'text-yellow-400'
    return 'text-orange-400'
  }
  if (status === 'timeout' || status === 'error') return 'text-rose-400'
  if (status === 'unreachable') return 'text-zinc-500'
  return 'text-zinc-600'
}
</script>

<template>
  <section class="space-y-3">
    <div class="flex items-center justify-between">
      <h2 class="text-sm font-semibold text-zinc-300 uppercase tracking-wider">
        Proxy groups
      </h2>
      <div class="flex items-center gap-2">
        <button
          @click="proxies.toggleSortMode()"
          :title="
            proxies.sortMode === 'default'
              ? 'Sort by latency'
              : 'Restore default order'
          "
          class="inline-flex items-center gap-1 rounded-md border border-zinc-700 bg-zinc-800/60 px-2 py-0.5 text-[11px] text-zinc-300 hover:bg-zinc-700"
        >
          <ArrowDownNarrowWide v-if="proxies.sortMode === 'default'" class="h-3 w-3" />
          <ArrowDownWideNarrow v-else class="h-3 w-3 text-emerald-400" />
          {{ proxies.sortMode === 'default' ? 'Default order' : 'Latency asc' }}
        </button>
        <button
          @click="proxies.fetchProxies()"
          class="text-xs text-zinc-400 hover:text-zinc-200"
        >
          refresh
        </button>
      </div>
    </div>

    <div
      v-if="proxies.loading && !proxies.lastFetchAt"
      class="text-zinc-500 text-sm flex items-center gap-2"
    >
      <Loader2 class="w-4 h-4 animate-spin" /> loading…
    </div>

    <div
      v-else-if="proxies.selectorGroups.length === 0"
      class="bg-zinc-900/40 border border-dashed border-zinc-800 rounded-xl p-6 text-center"
    >
      <div class="text-zinc-500 text-sm">
        No selector groups.
      </div>
      <div class="text-zinc-600 text-xs mt-1 font-mono">
        Add <code>proxy-groups</code> to your mihomo config.
      </div>
    </div>

    <div v-else class="space-y-3">
      <article
        v-for="group in proxies.selectorGroups"
        :key="group.name"
        class="bg-zinc-900 border border-zinc-800 rounded-xl p-4"
      >
        <!-- Group header -->
        <div class="flex items-center justify-between mb-3 gap-2">
          <div class="min-w-0">
            <div class="text-sm font-semibold text-zinc-100 truncate">
              {{ group.name }}
            </div>
            <div class="text-[10px] text-zinc-500 uppercase tracking-wider font-mono">
              {{ group.type }}
            </div>
          </div>
          <button
            @click="handleSpeedTest(group.name)"
            :disabled="proxies.isGroupTesting(group.name)"
            class="inline-flex items-center gap-1.5 text-xs px-2.5 py-1 rounded-md bg-indigo-600 hover:bg-indigo-500 disabled:bg-indigo-900 disabled:text-indigo-400 text-white transition-colors shrink-0"
          >
            <Loader2
              v-if="proxies.isGroupTesting(group.name)"
              class="w-3 h-3 animate-spin"
            />
            <Zap v-else class="w-3 h-3" />
            Speed test
          </button>
        </div>

        <!-- Node list -->
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
              'group flex flex-col items-start px-3 py-2 rounded-lg text-left transition-colors disabled:opacity-50',
              group.now === node
                ? 'bg-indigo-600/20 border border-indigo-500/60 text-indigo-100'
                : 'bg-zinc-950/40 border border-zinc-800 hover:border-zinc-700 text-zinc-200',
            ]"
          >
            <div class="flex items-center gap-1.5 w-full min-w-0">
              <CheckCircle2
                v-if="group.now === node"
                class="w-3.5 h-3.5 text-indigo-400 shrink-0"
              />
              <span class="text-sm font-medium truncate flex-1">{{ node }}</span>
              <Loader2
                v-if="selecting[`${group.name}::${node}`] === true"
                class="w-3 h-3 animate-spin text-indigo-300 shrink-0"
              />
            </div>
            <div
              :class="[
                'text-[10px] font-mono mt-0.5 flex items-center gap-1',
                delayColorClass(
                  group.nodes[node]?.delay ?? null,
                  group.nodes[node]?.status ?? 'idle',
                ),
              ]"
            >
              <Loader2
                v-if="group.nodes[node]?.status === 'testing'"
                class="h-2.5 w-2.5 animate-spin"
              />
              <span>
                {{
                  delayText(
                    group.nodes[node]?.delay ?? null,
                    group.nodes[node]?.status ?? 'idle',
                  )
                }}
              </span>
            </div>
          </button>
        </div>
      </article>
    </div>

    <div
      v-if="proxies.error"
      class="bg-rose-950/40 border border-rose-800/60 text-rose-200 rounded-lg p-3 text-xs font-mono"
    >
      {{ proxies.error }}
    </div>
  </section>
</template>

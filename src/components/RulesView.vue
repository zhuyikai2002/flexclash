<script setup lang="ts">
// ============================================================================
// RulesView — M10 `/rules` panel. Reads `GET /rules` straight from
// mihomo (no Rust proxy). Filter by rule type and free-text payload.
// ============================================================================

import { computed, onMounted, ref } from 'vue'
import { Loader2, RefreshCw, Search, X, ListFilter, Filter } from 'lucide-vue-next'
import { useRulesStore } from '@/stores/rules'
import { useKernelStore } from '@/stores/kernel'
import { RULE_TYPE_GROUPS } from '@/services/rules'

const rules = useRulesStore()
const kernel = useKernelStore()
const showTypeMenu = ref(false)

onMounted(() => {
  if (kernel.isRunning && !rules.lastFetchAt) {
    void rules.fetch()
  }
})

const typeLabel = computed(() => {
  if (rules.typeFilter === 'all') return 'All types'
  // First group that contains the active filter, or just the type name.
  for (const g of RULE_TYPE_GROUPS) {
    if (g.types.includes(rules.typeFilter as never)) return rules.typeFilter
  }
  return rules.typeFilter
})

function typeBadgeColor(type: string): string {
  // Colour-code a few common categories for quick scanning.
  if (type.startsWith('Domain'))   return 'text-emerald-300 bg-emerald-950/40 border-emerald-900/60'
  if (type.startsWith('IP') || type === 'GEOIP') return 'text-sky-300 bg-sky-950/40 border-sky-900/60'
  if (type.includes('Port'))        return 'text-amber-300 bg-amber-950/40 border-amber-900/60'
  if (type.startsWith('Process') || type === 'UID') return 'text-rose-300 bg-rose-950/40 border-rose-900/60'
  if (['AND','OR','NOT','MATCH','RuleSet'].includes(type)) return 'text-purple-300 bg-purple-950/40 border-purple-900/60'
  return 'text-zinc-300 bg-zinc-800/60 border-zinc-700'
}
</script>

<template>
  <section class="space-y-3">
    <div class="flex items-center justify-between gap-2 flex-wrap">
      <div class="flex items-center gap-2 min-w-0">
        <h2 class="text-sm font-semibold text-zinc-300 uppercase tracking-wider">
          Rules
        </h2>
        <span
          v-if="rules.rules.length"
          class="rounded bg-zinc-800/60 px-1.5 py-0.5 text-[10px] text-zinc-300 font-mono"
        >{{ rules.filtered.length }} / {{ rules.rules.length }}</span>
      </div>
      <div class="flex items-center gap-2">
        <div class="relative">
          <button
            @click="showTypeMenu = !showTypeMenu"
            class="inline-flex items-center gap-1.5 rounded-md border border-zinc-700 bg-zinc-800/60 px-2.5 py-1 text-[11px] text-zinc-200 hover:bg-zinc-700"
          >
            <Filter class="h-3 w-3" />
            {{ typeLabel }}
          </button>
          <div
            v-if="showTypeMenu"
            class="absolute right-0 mt-1 z-20 w-56 max-h-72 overflow-auto rounded-md border border-zinc-700 bg-zinc-900 shadow-xl p-1"
          >
            <button
              @click="rules.setTypeFilter('all'); showTypeMenu = false"
              :class="[
                'block w-full text-left px-2 py-1 rounded text-[12px]',
                rules.typeFilter === 'all' ? 'bg-indigo-600/20 text-indigo-200' : 'text-zinc-300 hover:bg-zinc-800',
              ]"
            >All types</button>
            <div
              v-for="g in RULE_TYPE_GROUPS"
              :key="g.label"
              class="mt-1 first:mt-0"
            >
              <div class="px-2 py-0.5 text-[10px] uppercase tracking-wider text-zinc-500">
                {{ g.label }}
              </div>
              <button
                v-for="t in g.types"
                :key="t"
                @click="rules.setTypeFilter(t); showTypeMenu = false"
                :class="[
                  'block w-full text-left px-2 py-1 rounded text-[12px] font-mono',
                  rules.typeFilter === t ? 'bg-indigo-600/20 text-indigo-200' : 'text-zinc-300 hover:bg-zinc-800',
                ]"
              >{{ t }}</button>
            </div>
          </div>
        </div>
        <button
          @click="rules.fetch()"
          class="inline-flex items-center gap-1.5 rounded-md border border-zinc-700 bg-zinc-800/60 px-2.5 py-1 text-[11px] text-zinc-200 hover:bg-zinc-700"
        >
          <RefreshCw v-if="!rules.loading" class="h-3 w-3" />
          <Loader2 v-else class="h-3 w-3 animate-spin" />
          refresh
        </button>
      </div>
    </div>

    <div class="relative">
      <Search class="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-zinc-500" />
      <input
        :value="rules.search"
        @input="(e) => rules.setSearch((e.target as HTMLInputElement).value)"
        placeholder="filter by type / payload / proxy…"
        class="w-full pl-8 pr-8 py-1.5 rounded-md border border-zinc-700 bg-zinc-900/60 text-sm text-zinc-200 placeholder-zinc-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
      />
      <button
        v-if="rules.isFiltered"
        @click="rules.clearFilters()"
        class="absolute right-2 top-1/2 -translate-y-1/2 text-zinc-500 hover:text-zinc-300"
        title="clear filters"
      ><X class="h-3.5 w-3.5" /></button>
    </div>

    <div
      v-if="!kernel.isRunning"
      class="bg-zinc-900/40 border border-dashed border-zinc-800 rounded-xl p-6 text-center text-sm text-zinc-500"
    >
      Start the kernel to view rules.
    </div>

    <div
      v-else-if="rules.loading && !rules.rules.length"
      class="text-zinc-500 text-sm flex items-center gap-2"
    >
      <Loader2 class="w-4 h-4 animate-spin" /> loading rules…
    </div>

    <div
      v-else-if="!rules.filtered.length"
      class="bg-zinc-900/40 border border-dashed border-zinc-800 rounded-xl p-6 text-center text-sm text-zinc-500"
    >
      <ListFilter class="w-5 h-5 mx-auto mb-1 text-zinc-600" />
      no rules match the current filter.
    </div>

    <div
      v-else
      class="bg-zinc-900 border border-zinc-800 rounded-xl overflow-hidden"
    >
      <table class="w-full text-sm">
        <thead class="bg-zinc-900/60 text-[10px] uppercase tracking-wider text-zinc-500">
          <tr>
            <th class="text-left font-semibold px-3 py-2 w-32">Type</th>
            <th class="text-left font-semibold px-3 py-2">Payload</th>
            <th class="text-left font-semibold px-3 py-2 w-40">Proxy</th>
          </tr>
        </thead>
        <tbody class="font-mono text-[12px]">
          <tr
            v-for="(r, i) in rules.filtered"
            :key="`${r.type}-${r.payload}-${i}`"
            class="border-t border-zinc-800/80 hover:bg-zinc-800/40"
          >
            <td class="px-3 py-1.5">
              <span
                :class="['inline-block rounded border px-1.5 py-0.5', typeBadgeColor(r.type)]"
              >{{ r.type }}</span>
            </td>
            <td class="px-3 py-1.5 text-zinc-200 break-all">{{ r.payload }}</td>
            <td class="px-3 py-1.5 text-indigo-300">{{ r.proxy }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <div
      v-if="rules.lastError"
      class="bg-rose-950/40 border border-rose-800/60 text-rose-200 rounded-lg p-3 text-xs font-mono"
    >
      {{ rules.lastError }}
    </div>
  </section>
</template>

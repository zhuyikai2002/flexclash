<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { Loader2, RefreshCw, Search, X, ListFilter, Filter } from 'lucide-vue-next'
import { useRulesStore } from '@/stores/rules'
import { useKernelStore } from '@/stores/kernel'
import { useI18n } from '@/composables/useI18n'
import { RULE_TYPE_GROUPS } from '@/services/rules'

const rules = useRulesStore()
const kernel = useKernelStore()
const { t } = useI18n()
const showTypeMenu = ref(false)

onMounted(() => {
  if (kernel.isRunning && !rules.lastFetchAt) {
    void rules.fetch()
  }
})

const typeLabel = computed(() => {
  if (rules.typeFilter === 'all') return t('stats.rules.all_types')
  for (const g of RULE_TYPE_GROUPS) {
    if (g.types.includes(rules.typeFilter as never)) return rules.typeFilter
  }
  return rules.typeFilter
})

function typeBadgeColor(type: string): string {
  if (type.startsWith('Domain'))   return 'text-emerald-300 bg-emerald-500/10 border-emerald-500/30'
  if (type.startsWith('IP') || type === 'GEOIP') return 'text-sky-300 bg-sky-500/10 border-sky-500/30'
  if (type.includes('Port'))        return 'text-amber-300 bg-amber-500/10 border-amber-500/30'
  if (type.startsWith('Process') || type === 'UID') return 'text-rose-300 bg-rose-500/10 border-rose-500/30'
  if (['AND','OR','NOT','MATCH','RuleSet'].includes(type)) return 'text-purple-300 bg-purple-500/10 border-purple-500/30'
  return 'text-zinc-300 bg-white/5 border-white/10'
}
</script>

<template>
  <section class="space-y-3">
    <div class="flex items-center justify-between gap-2 flex-wrap">
      <div class="flex items-center gap-2 min-w-0">
        <h2 class="text-sm font-semibold text-zinc-200">
          {{ t('stats.rules.title') }}
        </h2>
        <span
          v-if="rules.rules.length"
          class="rounded-md bg-indigo-500/20 px-1.5 py-0.5 text-[10px] font-mono text-indigo-200"
        >{{ rules.filtered.length }} / {{ rules.rules.length }}</span>
      </div>
      <div class="flex items-center gap-2">
        <div class="relative">
          <button
            @click="showTypeMenu = !showTypeMenu"
            class="inline-flex items-center gap-1.5 rounded-lg border border-white/5 bg-white/[0.04] px-2.5 py-1 text-[11px] text-zinc-200 hover:bg-white/[0.08] hover:border-white/10 transition-colors"
          >
            <Filter class="h-3 w-3" />
            {{ typeLabel }}
          </button>
          <div
            v-if="showTypeMenu"
            class="absolute right-0 mt-1 z-20 w-56 max-h-72 overflow-auto rounded-xl border border-white/10 bg-zinc-900/95 shadow-2xl backdrop-blur-xl p-1"
          >
            <button
              @click="rules.setTypeFilter('all'); showTypeMenu = false"
              :class="[
                'block w-full text-left px-2 py-1 rounded text-[12px] transition-colors',
                rules.typeFilter === 'all' ? 'bg-indigo-500/20 text-indigo-200' : 'text-zinc-300 hover:bg-white/5',
              ]"
            >{{ t('stats.rules.all_types') }}</button>
            <div
              v-for="g in RULE_TYPE_GROUPS"
              :key="g.label"
              class="mt-1 first:mt-0"
            >
              <div class="px-2 py-0.5 text-[10px] uppercase tracking-wider text-zinc-500">
                {{ g.label }}
              </div>
              <button
                v-for="tp in g.types"
                :key="tp"
                @click="rules.setTypeFilter(tp); showTypeMenu = false"
                :class="[
                  'block w-full text-left px-2 py-1 rounded text-[12px] font-mono transition-colors',
                  rules.typeFilter === tp ? 'bg-indigo-500/20 text-indigo-200' : 'text-zinc-300 hover:bg-white/5',
                ]"
              >{{ tp }}</button>
            </div>
          </div>
        </div>
        <button
          @click="rules.fetch()"
          class="inline-flex items-center gap-1.5 rounded-lg border border-white/5 bg-white/[0.04] px-2.5 py-1 text-[11px] text-zinc-200 hover:bg-white/[0.08] transition-colors"
        >
          <RefreshCw v-if="!rules.loading" class="h-3 w-3" />
          <Loader2 v-else class="h-3 w-3 animate-spin" />
          {{ t('common.refresh') }}
        </button>
      </div>
    </div>

    <div class="relative">
      <Search class="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-zinc-500" />
      <input
        :value="rules.search"
        @input="(e) => rules.setSearch((e.target as HTMLInputElement).value)"
        :placeholder="t('stats.rules.search_placeholder')"
        class="w-full rounded-lg border border-white/5 bg-white/[0.04] pl-8 pr-8 py-1.5 text-sm text-zinc-200 placeholder-zinc-500 focus:outline-none focus:ring-1 focus:ring-indigo-500/50 focus:border-indigo-400/30 transition-colors"
      />
      <button
        v-if="rules.isFiltered"
        @click="rules.clearFilters()"
        class="absolute right-2 top-1/2 -translate-y-1/2 text-zinc-500 hover:text-zinc-300"
        :title="t('common.clear')"
      ><X class="h-3.5 w-3.5" /></button>
    </div>

    <div
      v-if="!kernel.isRunning"
      class="rounded-2xl border border-dashed border-white/10 bg-white/[0.02] p-6 text-center text-sm text-zinc-500"
    >
      {{ t('common.loading') }}
    </div>

    <div
      v-else-if="rules.loading && !rules.rules.length"
      class="rounded-2xl border border-white/5 bg-white/[0.04] p-6 text-zinc-500 text-sm flex items-center gap-2"
    >
      <Loader2 class="w-4 h-4 animate-spin" /> {{ t('common.loading') }}
    </div>

    <div
      v-else-if="!rules.filtered.length"
      class="rounded-2xl border border-dashed border-white/10 bg-white/[0.02] p-6 text-center text-sm text-zinc-500"
    >
      <ListFilter class="w-5 h-5 mx-auto mb-1 text-zinc-600" />
      {{ t('stats.rules.no_results') }}
    </div>

    <div
      v-else
      class="overflow-hidden rounded-2xl border border-white/5 bg-white/[0.04] backdrop-blur-md"
    >
      <table class="w-full text-sm">
        <thead class="bg-white/[0.02] text-[10px] uppercase tracking-wider text-zinc-500">
          <tr>
            <th class="text-left font-semibold px-3 py-2 w-32">{{ t('connections.columns.type') }}</th>
            <th class="text-left font-semibold px-3 py-2">{{ t('connections.columns.host') }}</th>
            <th class="text-left font-semibold px-3 py-2 w-40">{{ t('connections.columns.rule') }}</th>
          </tr>
        </thead>
        <tbody class="font-mono text-[12px]">
          <tr
            v-for="(r, i) in rules.filtered"
            :key="`${r.type}-${r.payload}-${i}`"
            class="border-t border-white/5 hover:bg-white/[0.04] transition-colors"
          >
            <td class="px-3 py-1.5">
              <span
                :class="['inline-block rounded-md border px-1.5 py-0.5', typeBadgeColor(r.type)]"
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
      class="rounded-lg border border-rose-500/20 bg-rose-500/10 p-3 text-xs text-rose-300 font-mono"
    >
      {{ rules.lastError }}
    </div>
  </section>
</template>

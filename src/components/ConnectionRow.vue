<script setup lang="ts">
// ============================================================================
// ConnectionRow.vue — single row in the connections virtual list.
// Height is fixed at 40px (set in ConnectionsView's estimateSize).
// All fields are read from a `markRaw`-ed ConnectionRow so the per-row
// reactivity cost is zero.
// ============================================================================

import { computed } from 'vue'
import { Loader2, X, Globe, Cpu, Network, Shield } from 'lucide-vue-next'

import { formatRate } from '@/composables/useTrafficStream'
import type { ConnectionRow } from '@/types/clash'

const props = defineProps<{
  row: ConnectionRow
  /** Width of the column for the policy pill. */
  hostWidth?: string
}>()

const emit = defineEmits<{
  (e: 'close', id: string): void
}>()

const upText = computed(() => formatRate(props.row.uploadSpeed))
const downText = computed(() => formatRate(props.row.downloadSpeed))
const upClass = computed(() => (props.row.uploadSpeed > 0 ? 'text-emerald-400' : 'text-zinc-600'))
const downClass = computed(() => (props.row.downloadSpeed > 0 ? 'text-sky-400' : 'text-zinc-600'))

const totalBytes = computed(() => props.row.upload + props.row.download)
const totalText = computed(() => {
  const b = totalBytes.value
  if (b < 1024) return `${b} B`
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`
  return `${(b / 1024 / 1024).toFixed(2)} MB`
})

const policy = computed(() => props.row.policy || '—')
const network = computed(() => (props.row.network || '').toUpperCase())
const type = computed(() => props.row.type || 'Unknown')

const targetText = computed(() => {
  if (props.row.host) return props.row.host
  if (props.row.dst) return props.row.dst
  if (props.row.destinationIP) return `${props.row.destinationIP}:${props.row.destinationPort}`
  return '—'
})

const processText = computed(() => props.row.process || 'system')

const chainText = computed(() => {
  if (props.row.chains.length === 0) return '—'
  return props.row.chains.join(' › ')
})

function close() {
  emit('close', props.row.id)
}
</script>

<template>
  <div
    class="flex items-center gap-3 px-3 text-xs text-zinc-200 border-b border-zinc-800/60 hover:bg-zinc-900/60 transition-colors"
    :class="row.closing ? 'opacity-50' : ''"
    style="height: 40px"
    :data-conn-id="row.id"
  >
    <!-- 1. Host / IP -->
    <div class="flex-1 min-w-0 flex items-center gap-1.5">
      <Globe class="h-3 w-3 text-zinc-500 shrink-0" />
      <span class="truncate font-mono" :title="row.dst">{{ targetText }}</span>
    </div>

    <!-- 2. Process -->
    <div class="w-28 shrink-0 flex items-center gap-1.5 text-zinc-400">
      <Cpu class="h-3 w-3 text-zinc-500 shrink-0" />
      <span class="truncate" :title="row.processPath || processText">{{ processText }}</span>
    </div>

    <!-- 3. Network / Type -->
    <div class="w-24 shrink-0 flex items-center gap-1 text-zinc-500 font-mono">
      <Network class="h-3 w-3 shrink-0" />
      <span>{{ network }} · {{ type }}</span>
    </div>

    <!-- 4. Rule -->
    <div class="w-56 shrink-0 truncate text-zinc-400 font-mono" :title="row.rule">
      {{ row.rule || '—' }}
    </div>

    <!-- 5. Chain -->
    <div class="w-40 shrink-0 truncate text-zinc-400 font-mono" :title="chainText">
      {{ chainText }}
    </div>

    <!-- 6. Speeds -->
    <div class="w-44 shrink-0 flex flex-col font-mono leading-tight">
      <span :class="['flex items-center gap-1', upClass]">↑ {{ upText }}</span>
      <span :class="['flex items-center gap-1', downClass]">↓ {{ downText }}</span>
    </div>

    <!-- 7. Total -->
    <div class="w-20 shrink-0 text-right font-mono text-zinc-300">
      {{ totalText }}
    </div>

    <!-- 8. Policy pill -->
    <div class="w-24 shrink-0 flex items-center gap-1">
      <Shield class="h-3 w-3 text-zinc-500 shrink-0" />
      <span
        class="rounded-full bg-indigo-950/60 border border-indigo-800 px-2 py-0.5 text-[10px] text-indigo-200 truncate"
        :title="policy"
      >{{ policy }}</span>
    </div>

    <!-- 9. Action -->
    <div class="w-8 shrink-0 flex justify-end">
      <button
        v-if="!row.closing"
        type="button"
        class="rounded p-1 text-zinc-500 hover:text-rose-300 hover:bg-rose-950/40"
        title="Close this connection"
        @click="close"
      >
        <X class="h-3.5 w-3.5" />
      </button>
      <Loader2 v-else class="h-3.5 w-3.5 animate-spin text-zinc-500" />
    </div>
  </div>
</template>

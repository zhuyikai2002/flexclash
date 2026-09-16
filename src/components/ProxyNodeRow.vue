<script setup lang="ts">
/**
 * ProxyNodeRow — one horizontal band of node cards: the unit the virtual list
 * windows on.
 *
 * The list operates on rows (not on individual cards) so that a group header
 * can sit between them in a single scroll container, while the visual design
 * stays a responsive grid. `cols` comes from the owning list's breakpoint
 * watcher and drives both the chunking there and this grid — one source of
 * truth, so the two can never disagree.
 *
 * The delay map is read here, per key, rather than passed as a payload from the
 * list: a batch that updates four nodes then re-renders at most the rows those
 * four live in, and nothing above them.
 *
 * Height is fixed by design (one truncated name line, `py-2.5`), with the
 * inter-row gap carried as bottom padding so the row's height is the whole
 * pitch. The owning list relies on that to size the scroll area without
 * measuring anything.
 */
import { computed } from 'vue'
import { CheckCircle2, Loader2 } from 'lucide-vue-next'

import { useProxiesStore, type DelayStatus, type NodeDelayInfo } from '@/stores/proxies'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{
  /** Owning group name. */
  group: string
  /** Child names for this row, already in display order. */
  names: string[]
  /** Columns to lay the row out in. */
  cols: number
  /** `${group}::${node}` → a selection for that node is in flight. */
  selecting: Record<string, boolean>
}>()

const emit = defineEmits<{ select: [node: string] }>()

const proxies = useProxiesStore()
const { t } = useI18n()

const meta = computed(() => proxies.groups[props.group])
const testing = computed(() => proxies.isGroupTesting(props.group))

function delayOf(node: string): NodeDelayInfo | undefined {
  return meta.value?.nodes[node]
}

function latencyText(delay: number | null, status: DelayStatus): string {
  if (status === 'testing') return '…'
  if (status === 'unreachable') return t('proxies.latency.failed')
  if (status === 'timeout') return t('proxies.latency.timeout')
  if (status === 'error') return 'err'
  if (status === 'ok' && delay !== null) return t('proxies.latency.ms', { n: delay })
  return '—'
}

/**
 * Tooltip for a node card: its name, plus why the last probe ended the way it
 * did. The Rust engine reports a specific reason per failure, so surfacing it
 * here is what distinguishes "this node is dead" from "the kernel is down" —
 * both of which used to render as the same grey pill.
 */
function nodeTitle(name: string, info?: NodeDelayInfo): string {
  const reason = info?.message
  return reason ? `${name}\n${reason}` : name
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
</script>

<template>
  <div class="px-3 pb-2.5">
    <div
      class="grid gap-2.5"
      :style="{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }"
    >
      <button
        v-for="node in names"
        :key="node"
        @click="emit('select', node)"
        :disabled="testing || selecting[`${group}::${node}`] === true"
        :title="nodeTitle(node, delayOf(node))"
        :class="[
          'group relative flex items-center gap-2 rounded-xl border px-3 py-2.5 text-left transition-[transform,box-shadow,background-color,border-color] duration-200',
          'hover:-translate-y-0.5 hover:shadow-md active:translate-y-0 active:scale-[0.99] disabled:opacity-50 disabled:hover:translate-y-0',
          meta?.now === node
            ? 'border-sky-400/50 bg-sky-500/10 ring-1 ring-sky-400/30 text-zinc-50 shadow-md shadow-sky-500/10'
            : 'border-white/5 bg-zinc-950/30 text-zinc-200 hover:border-white/15 hover:bg-white/[0.05]'
        ]"
      >
        <CheckCircle2
          v-if="meta?.now === node"
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
          v-if="selecting[`${group}::${node}`] === true"
          class="h-3 w-3 animate-spin text-sky-300 shrink-0"
        />
        <span
          v-else
          :class="[
            'inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-mono font-medium shrink-0',
            latencyPillClass(delayOf(node)?.delay ?? null, delayOf(node)?.status ?? 'idle')
          ]"
        >
          <Loader2
            v-if="delayOf(node)?.status === 'testing'"
            class="h-2.5 w-2.5 animate-spin"
          />
          {{ latencyText(delayOf(node)?.delay ?? null, delayOf(node)?.status ?? 'idle') }}
        </span>
      </button>
    </div>
  </div>
</template>

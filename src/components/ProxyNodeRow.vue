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
import { computed, ref } from 'vue'
import { CheckCircle2, Loader2, Zap } from 'lucide-vue-next'

import { useProxiesStore, type DelayStatus, type NodeDelayInfo } from '@/stores/proxies'
import { useConnectionsStore } from '@/stores/connections'
import { useToastStore } from '@/stores/toast'
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
const conns = useConnectionsStore()
const toast = useToastStore()
const { t } = useI18n()

const meta = computed(() => proxies.groups[props.group])
const testing = computed(() => proxies.isGroupTesting(props.group))

/** `${node}` → a "kill dead links" run for that node is in flight. */
const killing = ref<Record<string, boolean>>({})

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

/** The card's select click is a no-op while the group is testing or the node's
 *  own selection is in flight — the kill button below stays independent. */
function onCardClick(node: string) {
  if (testing.value || props.selecting[`${props.group}::${node}`] === true) return
  emit('select', node)
}

/** Kill every connection currently egressing through this node. */
async function killNode(node: string) {
  if (killing.value[node]) return
  killing.value = { ...killing.value, [node]: true }
  try {
    const r = await conns.closeByProxy(node)
    if (r.killed > 0) toast.push('success', t('connections.killed_n', { n: r.killed }))
    else toast.push('info', t('connections.kill_none'))
  } catch (e) {
    toast.push('error', t('connections.kill_failed', { msg: e instanceof Error ? e.message : String(e) }))
  } finally {
    const next = { ...killing.value }
    delete next[node]
    killing.value = next
  }
}
</script>

<template>
  <div class="px-3 pb-2.5">
    <div
      class="grid gap-2.5"
      :style="{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }"
    >
      <div
        v-for="node in names"
        :key="node"
        role="button"
        tabindex="0"
        @click="onCardClick(node)"
        @keydown.enter.prevent="onCardClick(node)"
        @keydown.space.prevent="onCardClick(node)"
        :title="nodeTitle(node, delayOf(node))"
        :class="[
          'group relative flex items-center gap-2 rounded-xl border px-3 py-2.5 text-left transition-[transform,box-shadow,background-color,border-color] duration-200',
          'hover:-translate-y-0.5 hover:shadow-md active:translate-y-0 active:scale-[0.99]',
          (testing || selecting[`${group}::${node}`] === true) ? 'opacity-50' : '',
          meta?.now === node
            ? 'border-sky-400/50 bg-sky-500/10 ring-1 ring-sky-400/30 text-zinc-50 shadow-md shadow-sky-500/10'
            : 'glass-row text-zinc-200'
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
            'inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-mono font-medium shrink-0 tabular-nums justify-center transition-colors duration-300 min-w-[4rem]',
            latencyPillClass(delayOf(node)?.delay ?? null, delayOf(node)?.status ?? 'idle')
          ]"
        >
          <Loader2
            v-if="delayOf(node)?.status === 'testing'"
            class="h-2.5 w-2.5 animate-spin"
          />
          {{ latencyText(delayOf(node)?.delay ?? null, delayOf(node)?.status ?? 'idle') }}
        </span>
        <button
          type="button"
          class="shrink-0 rounded-md p-1 text-amber-300/70 opacity-0 group-hover:opacity-100 focus:opacity-100 hover:bg-amber-500/20 hover:text-amber-200 transition-opacity duration-200"
          :title="t('connections.kill_by_proxy')"
          @click.stop="killNode(node)"
          @keydown.stop
        >
          <Loader2 v-if="killing[node] === true" class="h-3.5 w-3.5 animate-spin" />
          <Zap v-else class="h-3.5 w-3.5" />
        </button>
      </div>
    </div>
  </div>
</template>

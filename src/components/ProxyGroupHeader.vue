<script setup lang="ts">
/**
 * ProxyGroupHeader — the band that introduces one proxy group inside the
 * virtual list.
 *
 * It subscribes to the store itself rather than receiving the group object as a
 * prop. That is deliberate: the virtual list's parent must never read node
 * payloads while rendering, or every speed-test batch would re-render the whole
 * list. Reading `groups[name]` here scopes the subscription to this header, and
 * the only fields it touches — `type`, `all`, `now` — change on a fetch or a
 * selection, never on a batch.
 *
 * Height is fixed by design (one name line + one meta line, `py-3`): the owning
 * list relies on it to compute scroll geometry without measuring.
 */
import { computed } from 'vue'
import { CheckCircle2, ChevronDown, ChevronRight, Loader2, Zap } from 'lucide-vue-next'

import { useProxiesStore } from '@/stores/proxies'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{
  /** Group name, as mihomo reports it. */
  group: string
  collapsed: boolean
  /** True for the first group, which needs no separator on its top edge. */
  first: boolean
}>()

const emit = defineEmits<{ toggle: []; test: [] }>()

const proxies = useProxiesStore()
const { t } = useI18n()

const meta = computed(() => proxies.groups[props.group])
const typeLabel = computed(() => (meta.value ? groupTypeLabel(meta.value.type) : ''))
const count = computed(() => meta.value?.all.length ?? 0)
const now = computed(() => meta.value?.now ?? null)
const testing = computed(() => proxies.isGroupTesting(props.group))

function groupTypeLabel(type: string): string {
  const known: Record<string, string> = {
    Selector: 'Selector', Fallback: 'Fallback', URLTest: 'URL Test',
    LoadBalance: 'Load Balance', Relay: 'Relay', DIRECT: 'Direct', REJECT: 'Reject',
  }
  return known[type] ?? type
}
</script>

<template>
  <header
    class="flex items-center justify-between gap-2 px-4 py-3 border-b border-white/5 bg-white/[0.03]"
    :class="first ? '' : 'border-t'"
  >
    <button
      class="flex items-center gap-2 min-w-0 text-left"
      @click="emit('toggle')"
    >
      <ChevronDown
        v-if="!collapsed"
        class="h-3.5 w-3.5 text-zinc-500 shrink-0 transition-transform"
      />
      <ChevronRight
        v-else
        class="h-3.5 w-3.5 text-zinc-500 shrink-0 transition-transform"
      />
      <div class="min-w-0">
        <div class="text-sm font-semibold text-zinc-100 truncate flex items-center gap-1.5">
          {{ group }}
          <span
            v-if="now"
            class="inline-flex items-center gap-1 rounded-md bg-sky-500/15 px-1.5 py-0.5 text-[10px] font-mono text-sky-200 truncate max-w-[180px]"
          >
            <CheckCircle2 class="h-2.5 w-2.5 shrink-0" />
            <span class="truncate">{{ now }}</span>
          </span>
        </div>
        <div class="text-[10px] uppercase tracking-wider text-zinc-500 font-mono tabular-nums">
          {{ typeLabel }} · {{ count }} nodes
        </div>
      </div>
    </button>
    <button
      @click="emit('test')"
      :disabled="testing"
      class="inline-flex items-center gap-1.5 rounded-lg bg-indigo-500/20 px-2.5 py-1 text-[11px] text-indigo-200 hover:bg-indigo-500/30 disabled:opacity-50 transition-colors shrink-0"
    >
      <Loader2 v-if="testing" class="w-3 h-3 animate-spin" />
      <Zap v-else class="w-3 h-3" />
      {{ t('proxies.test') }}
    </button>
  </header>
</template>

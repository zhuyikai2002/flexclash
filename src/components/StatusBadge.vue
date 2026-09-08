<script setup lang="ts">
/**
 * StatusBadge — pulsing indicator for the kernel state.
 *
 *   ● running   — solid emerald with ping halo
 *   ◐ starting  — amber, slow pulse (no ping)
 *   ○ stopped   — zinc
 *   ✕ crashed   — solid rose
 *
 * `compact` mode (used by Sidebar) drops the text label and shows a
 * smaller dot with a tooltip.
 */
import { computed } from 'vue'
import { useI18n } from '@/composables/useI18n'
import type { KernelState } from '@/types/clash'

const props = withDefaults(
  defineProps<{ state: KernelState; compact?: boolean }>(),
  { compact: false },
)
const { t } = useI18n()

const variant = computed(() => {
  switch (props.state) {
    case 'running':   return { dot: 'bg-emerald-400', ping: 'bg-emerald-400', ring: 'shadow-emerald-500/30', label: 'dashboard.state.running' }
    case 'starting':
    case 'stopping':  return { dot: 'bg-amber-400 animate-pulse', ping: '', ring: '', label: props.state === 'starting' ? 'dashboard.state.starting' : 'dashboard.state.stopping' }
    case 'crashed':   return { dot: 'bg-rose-500', ping: '', ring: 'shadow-rose-500/30', label: 'dashboard.state.crashed' }
    case 'stopped':   return { dot: 'bg-zinc-500', ping: '', ring: '', label: 'dashboard.state.stopped' }
    default:          return { dot: 'bg-zinc-700', ping: '', ring: '', label: 'dashboard.state.unknown' }
  }
})

const labelText = computed(() => {
  if (variant.value.label === 'dashboard.state.starting') return t('dashboard.state.starting')
  if (variant.value.label === 'dashboard.state.stopping') return t('dashboard.state.stopping')
  return t(variant.value.label)
})
</script>

<template>
  <!-- Compact: tooltip-only dot.  Sidebar shows this in its 68px rail. -->
  <div
    v-if="compact"
    :title="labelText"
    :aria-label="labelText"
    :class="['relative flex h-3.5 w-3.5 items-center justify-center rounded-full', variant.ring && 'shadow-sm']"
  >
    <span
      v-if="variant.ping"
      :class="['absolute inline-flex h-full w-full rounded-full opacity-75 animate-ping', variant.ping]"
    />
    <span :class="['relative inline-flex h-2 w-2 rounded-full', variant.dot]" />
  </div>

  <!-- Full: pill with text label (legacy, used inside modals / cards). -->
  <div
    v-else
    :class="['inline-flex items-center gap-2 rounded-full border border-white/5 bg-white/[0.04] px-3 py-1', variant.ring && 'shadow-sm']"
  >
    <span class="relative flex h-2.5 w-2.5">
      <span
        v-if="variant.ping"
        :class="['absolute inline-flex h-full w-full rounded-full opacity-75 animate-ping', variant.ping]"
      />
      <span :class="['relative inline-flex h-2.5 w-2.5 rounded-full', variant.dot]" />
    </span>
    <span class="font-mono text-[11px] uppercase tracking-wider text-zinc-200">
      {{ labelText }}
    </span>
  </div>
</template>

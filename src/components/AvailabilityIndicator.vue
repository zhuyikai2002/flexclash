<script setup lang="ts">
/**
 * AvailabilityIndicator.vue — promotes the kernel's reachability
 * (`up` / `degraded` / `down`, from `useKernelStore().availability`) to a
 * first-class, glanceable visual instead of something buried in logs.
 *
 *   - up:       emerald dot + a faint breathing pulse (everything is fine).
 *   - degraded: amber dot, no pulse — the process is alive but our own REST
 *               probe cannot reach the controller (e.g. mid supervisor-restart).
 *   - down:     red dot — Rust reports the kernel process is not running.
 *
 * `compact` (default) renders just a status dot for the 68px sidebar rail.
 * Expanded renders a labelled pill. Either way the dot is a button that opens
 * a minimal Popover (teleported to <body>) with the live probe status,
 * latency, and the last error surfaced through the unified notice channel.
 */
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useKernelStore } from '@/stores/kernel'
import { useI18n } from '@/composables/useI18n'

const props = withDefaults(defineProps<{ compact?: boolean }>(), { compact: false })

const kernel = useKernelStore()
const { t } = useI18n()

const open = ref(false)
const btnRef = ref<HTMLElement | null>(null)
const pos = ref({ top: 0, left: 0 })

// Emerald for up, amber for the "alive but unreachable" degraded state, red
// for a genuinely down kernel.
const dotClass = computed(() => {
  switch (kernel.availability) {
    case 'up': return 'bg-emerald-400'
    case 'degraded': return 'bg-amber-400'
    default: return 'bg-red-500'
  }
})

// Only `up` breathes — a calm pulse reads as "healthy", whereas a pulsing
// amber/red would read as "alarming" and fight the colour semantics.
const showPulse = computed(() => kernel.availability === 'up')

const label = computed(() => {
  switch (kernel.availability) {
    case 'up': return t('kernel.availability.up')
    case 'degraded': return t('kernel.availability.degraded')
    default: return t('kernel.availability.down')
  }
})

const probeLabel = computed(() => {
  switch (kernel.probeStatus) {
    case 'healthy': return t('kernel.probe.healthy')
    case 'probing': return t('kernel.probe.probing')
    case 'error': return t('kernel.probe.error')
    default: return t('kernel.probe.idle')
  }
})

function toggle() {
  // Position the flyout to the right of the rail, vertically centred on the
  // dot, only when opening — so it tracks the live button location.
  if (!open.value && btnRef.value) {
    const r = btnRef.value.getBoundingClientRect()
    pos.value = {
      top: Math.max(8, r.top + r.height / 2 - 48),
      left: r.right + 12,
    }
  }
  open.value = !open.value
}

function close() { open.value = false }

function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape' && open.value) close()
}
onMounted(() => window.addEventListener('keydown', onKey))
onUnmounted(() => window.removeEventListener('keydown', onKey))
</script>

<template>
  <div class="relative inline-flex">
    <!-- Compact: a bare, clickable status dot (sidebar rail). -->
    <button
      v-if="props.compact"
      ref="btnRef"
      type="button"
      :title="label"
      :aria-label="label"
      class="relative flex h-6 w-6 items-center justify-center rounded-full outline-none focus-visible:ring-2 focus-visible:ring-white/30"
      @click="toggle"
    >
      <span
        v-if="showPulse"
        class="absolute inline-flex h-2.5 w-2.5 rounded-full bg-emerald-400 opacity-75 animate-ping"
      ></span>
      <span class="relative inline-flex h-2.5 w-2.5 rounded-full" :class="dotClass"></span>
    </button>

    <!-- Expanded: a labelled pill (dashboard header, etc.). -->
    <button
      v-else
      ref="btnRef"
      type="button"
      :title="label"
      :aria-label="label"
      class="inline-flex items-center gap-2 rounded-full border border-white/10 bg-white/[0.04] px-2.5 py-1 text-[11px] text-zinc-300 outline-none transition-colors hover:bg-white/[0.08] focus-visible:ring-2 focus-visible:ring-white/30"
      @click="toggle"
    >
      <span class="relative inline-flex h-2 w-2">
        <span
          v-if="showPulse"
          class="absolute inline-flex h-2 w-2 rounded-full bg-emerald-400 opacity-75 animate-ping"
        ></span>
        <span class="relative inline-flex h-2 w-2 rounded-full" :class="dotClass"></span>
      </span>
      <span>{{ label }}</span>
    </button>

    <Teleport to="body">
      <template v-if="open">
        <!-- Click-away catcher; sits below the card so the card stays usable. -->
        <div class="fixed inset-0 z-[95]" @click="close"></div>
        <div
          class="fixed z-[96] w-60 rounded-xl border border-white/10 bg-zinc-900/95 p-3 text-xs shadow-2xl backdrop-blur-xl"
          :style="{ top: `${pos.top}px`, left: `${pos.left}px` }"
          @click.stop
        >
          <div class="mb-2 flex items-center gap-2">
            <span class="relative inline-flex h-2 w-2">
              <span
                v-if="showPulse"
                class="absolute inline-flex h-2 w-2 rounded-full bg-emerald-400 opacity-75 animate-ping"
              ></span>
              <span class="relative inline-flex h-2 w-2 rounded-full" :class="dotClass"></span>
            </span>
            <span class="font-medium text-zinc-100">{{ label }}</span>
          </div>
          <dl class="space-y-1.5 text-zinc-400">
            <div class="flex items-center justify-between gap-3">
              <dt>{{ t('kernel.popover.probe') }}</dt>
              <dd class="text-zinc-200">{{ probeLabel }}</dd>
            </div>
            <div class="flex items-center justify-between gap-3">
              <dt>{{ t('kernel.popover.latency') }}</dt>
              <dd class="font-mono tabular-nums text-zinc-200">
                {{ kernel.probeLatencyMs != null ? `${kernel.probeLatencyMs} ms` : '—' }}
              </dd>
            </div>
            <div v-if="kernel.lastError" class="flex items-start justify-between gap-3">
              <dt class="shrink-0 pt-0.5">{{ t('kernel.popover.lastError') }}</dt>
              <dd class="break-all text-right text-rose-300">{{ kernel.lastError }}</dd>
            </div>
          </dl>
        </div>
      </template>
    </Teleport>
  </div>
</template>

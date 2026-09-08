<script setup lang="ts">
/**
 * OutboundModeSwitcher.vue — Phase 8 dashboard capsule.
 *
 *   ┌────────────────────────────────────────────────┐
 *   │ 出站模式                                       │
 *   │ ┌──────┐  ┌──────┐  ┌──────┐                  │
 *   │ │ 规则 │  │ 全局 │  │ 直连 │   ← segmented    │
 *   │ └──────┘  └──────┘  └──────┘                  │
 *   │ 决定未被规则命中的流量如何处理                   │
 *   └────────────────────────────────────────────────┘
 *
 * Design notes (Phase 8 polish):
 *  - The pill slides with a CSS spring (`transition-all duration-200 ease-out`).
 *  - We COMMIT the local state optimistically the moment the user
 *    clicks — no "switching…" / loading chip.  The pill glides to
 *    its new home while the PATCH is in flight.
 *  - On failure (network error / kernel off) we snap the state
 *    back and surface a single rose-line error.  The pill slides
 *    back the same way.
 *  - Talks DIRECTLY to mihomo (PATCH /configs) — Rust does not own
 *    the data path.  Gated by `kernel.isRunning` so a cold start
 *    can never emit a raw ECONNREFUSED toast.
 */
import { computed, onMounted, ref, watch } from 'vue'
import { setMode, getConfigs } from '@/services/clash'
import { useI18n } from '@/composables/useI18n'
import { useKernelStore } from '@/stores/kernel'
import { useAppStateStore } from '@/stores/appstate'

type Mode = 'rule' | 'global' | 'direct'

const { t } = useI18n()
const kernel = useKernelStore()
const appstate = useAppStateStore()

const current = ref<Mode>('rule')
const lastError = ref<string | null>(null)
let errorTimer: ReturnType<typeof setTimeout> | null = null

function flashError(msg: string) {
  lastError.value = msg
  if (errorTimer) clearTimeout(errorTimer)
  errorTimer = setTimeout(() => {
    lastError.value = null
    errorTimer = null
  }, 3000)
}

const modes: Array<{ id: Mode; labelKey: string; color: string; icon: string }> = [
  { id: 'rule',   labelKey: 'dashboard.outbound_mode.rule',   color: 'sky',     icon: '⚖' },
  { id: 'global', labelKey: 'dashboard.outbound_mode.global', color: 'amber',   icon: '🌐' },
  { id: 'direct', labelKey: 'dashboard.outbound_mode.direct', color: 'emerald', icon: '↳' },
]

const activeIndex = computed(() => modes.findIndex((m) => m.id === current.value))

async function refresh() {
  if (!kernel.isRunning) return
  try {
    const cfg = await getConfigs()
    if (cfg.mode === 'rule' || cfg.mode === 'global' || cfg.mode === 'direct') {
      current.value = cfg.mode
    }
  } catch {
    /* cold start: ignore */
  }
}

onMounted(refresh)
// Keep the capsule in sync when the kernel transitions Stopped → Running.
watch(() => kernel.isRunning, (running) => {
  if (running) void refresh()
})
// Phase R3: the Rust state watcher is authoritative for outbound mode —
// any external change (another client, config reload) reflects here within
// one second, without polling.
watch(
  () => appstate.mode,
  (m) => {
    if (m === 'rule' || m === 'global' || m === 'direct') {
      if (m !== current.value) current.value = m
    }
  },
)

async function pick(m: Mode) {
  if (m === current.value) return
  if (!kernel.isRunning) {
    flashError('kernel not running')
    return
  }
  // Snapshot for rollback — but the pill glides forward first.
  const prev = current.value
  current.value = m
  try {
    await setMode(m)
  } catch (e) {
    // Roll back: the same `transition-all duration-200 ease-out`
    // carries the pill home, so this looks like a deliberate
    // "unselection", not a glitch.
    current.value = prev
    flashError(t('dashboard.outbound_mode.switch_failed', {
      msg: e instanceof Error ? e.message : String(e),
    }))
  }
}
</script>

<template>
  <section
    class="rounded-2xl border border-white/5 bg-white/[0.04] backdrop-blur-md p-5 space-y-3"
  >
    <header class="flex items-center justify-between">
      <h2 class="text-sm font-semibold text-zinc-100">
        {{ t('dashboard.outbound_mode.title') }}
      </h2>
    </header>

    <!-- Segmented capsule.  The sliding indicator is a sibling of the
         buttons and inherits the same `transition-all duration-200
         ease-out` from the buttons themselves — that is the spring
         you see as the pill glides. -->
    <div
      class="relative grid grid-cols-3 gap-1 rounded-xl bg-white/[0.03] border border-white/5 p-1"
    >
      <button
        v-for="(m, i) in modes"
        :key="m.id"
        type="button"
        :disabled="!kernel.isRunning"
        :aria-pressed="current === m.id"
        :class="[
          'relative z-10 rounded-lg px-3 py-2 text-sm font-medium transition-colors duration-200',
          'disabled:opacity-50 disabled:cursor-not-allowed',
          current === m.id
            ? 'text-zinc-100'
            : 'text-zinc-400 hover:text-zinc-200'
        ]"
        @click="pick(m.id)"
      >
        <span class="mr-1.5 text-base leading-none">{{ m.icon }}</span>
        {{ t(m.labelKey) }}
      </button>
      <!-- Sliding indicator: GPU-composited left/width transition. -->
      <span
        class="absolute top-1 bottom-1 rounded-lg bg-white/10 ring-1 ring-white/10 shadow-sm shadow-sky-500/10 transition-all duration-200 ease-out pointer-events-none"
        :style="{
          left: `calc(${(activeIndex * 100) / 3}% + 4px)`,
          width: `calc(${100 / 3}% - 8px)`,
        }"
      />
    </div>

    <p class="text-[11px] text-zinc-500 leading-relaxed">
      {{ t('dashboard.outbound_mode.hint') }}
    </p>

    <p
      v-if="lastError"
      class="text-[11px] text-rose-300/90 font-mono"
    >
      {{ lastError }}
    </p>
  </section>
</template>

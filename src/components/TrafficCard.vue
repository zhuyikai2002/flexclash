<script setup lang="ts">
/**
 * TrafficCard — Hero-style live download / upload rate panel.
 *
 * Phase 4 redesign (modern grid):
 *   - Two side-by-side "gauge" cards, each dominated by a HUGE mono number
 *     (text-5xl / font-semibold) so the eye locks on the rate instantly.
 *   - Subtle gradient line + corner indicator for direction (down=emerald,
 *     up=indigo) so the panel reads correctly at a glance.
 *   - Tiny "live" pulse / "reconnect" CTA in the top-right.
 *   - Footer: total over the visible window + manual refresh.
 */
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { ArrowDown, ArrowUp, RefreshCw, Wifi, WifiOff } from 'lucide-vue-next'
import { formatRate, useTrafficStream } from '@/composables/useTrafficStream'
import { useI18n } from '@/composables/useI18n'

const { t } = useI18n()
const { up, down, connected, lastUpdateAt, error, reconnect } = useTrafficStream()

const tick = ref(0)
let tickTimer: ReturnType<typeof setInterval> | null = null

onMounted(() => {
  tickTimer = setInterval(() => { tick.value++ }, 1_000)
})
onUnmounted(() => {
  if (tickTimer !== null) {
    clearInterval(tickTimer)
    tickTimer = null
  }
})

const upText = computed(() => formatRate(up.value))
const downText = computed(() => formatRate(down.value))

// Split "1.42 MB/s" into { num: "1.42", unit: "MB/s" } so the unit can be
// rendered in a softer color without breaking the tabular-nums alignment.
const upParts = computed(() => splitRate(upText.value))
const downParts = computed(() => splitRate(downText.value))

function splitRate(s: string): { num: string; unit: string } {
  const m = s.match(/^([\d.]+)\s*(.*)$/)
  return m ? { num: m[1], unit: m[2] } : { num: s, unit: '' }
}

const sinceText = computed(() => {
  void tick.value
  if (!lastUpdateAt.value) return t('common.never')
  const ago = Math.floor((Date.now() - lastUpdateAt.value) / 1000)
  if (ago < 5) return t('time.just_now')
  if (ago < 60) return t('time.seconds_ago', { n: ago })
  return t('time.minutes_ago', { n: Math.floor(ago / 60) })
})
</script>

<template>
  <section
    class="rounded-2xl border border-white/5 bg-white/[0.04] p-5 backdrop-blur-md"
  >
    <header class="mb-4 flex items-center justify-between">
      <div class="flex items-center gap-2">
        <h3 class="text-sm font-semibold text-zinc-100">
          {{ t('dashboard.traffic.title') }}
        </h3>
        <span
          v-if="connected"
          class="inline-flex items-center gap-1.5 text-[10px] font-mono text-emerald-400"
        >
          <span class="relative flex h-1.5 w-1.5">
            <span class="absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75 animate-ping" />
            <span class="relative inline-flex h-1.5 w-1.5 rounded-full bg-emerald-500" />
          </span>
          live
        </span>
      </div>
      <button
        v-if="!connected"
        @click="reconnect"
        class="inline-flex items-center gap-1.5 rounded-full bg-amber-500/10 px-2.5 py-0.5 text-[10px] text-amber-400 hover:bg-amber-500/20 transition-colors"
      >
        <WifiOff class="h-3 w-3" />
        reconnect
      </button>
    </header>

    <div class="grid grid-cols-2 gap-3">
      <!-- Download -->
      <div
        class="group relative overflow-hidden rounded-xl border border-white/5 bg-gradient-to-br from-emerald-500/[0.04] via-transparent to-transparent p-5"
      >
        <div class="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-emerald-400/60 to-transparent" />
        <div class="absolute top-3 right-3 text-emerald-400/60">
          <ArrowDown class="h-3.5 w-3.5" />
        </div>
        <div class="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wider text-emerald-400/80">
          {{ t('dashboard.traffic.download') }}
        </div>
        <div class="mt-2 flex items-baseline gap-1">
          <span class="text-4xl font-semibold font-mono tabular-nums text-zinc-50 leading-none">
            {{ downParts.num }}
          </span>
          <span class="text-sm font-mono text-zinc-500">
            {{ downParts.unit }}
          </span>
        </div>
        <!-- Mini "flow" indicator: a thin gradient line that shimmers left → right. -->
        <div class="mt-4 h-[2px] w-full overflow-hidden rounded-full bg-emerald-500/10">
          <div
            class="h-full w-1/3 rounded-full bg-gradient-to-r from-emerald-400/0 via-emerald-400/80 to-emerald-400/0"
            :class="connected ? 'animate-pulse' : ''"
          ></div>
        </div>
      </div>

      <!-- Upload -->
      <div
        class="group relative overflow-hidden rounded-xl border border-white/5 bg-gradient-to-br from-indigo-500/[0.04] via-transparent to-transparent p-5"
      >
        <div class="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-indigo-400/60 to-transparent" />
        <div class="absolute top-3 right-3 text-indigo-400/60">
          <ArrowUp class="h-3.5 w-3.5" />
        </div>
        <div class="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wider text-indigo-400/80">
          {{ t('dashboard.traffic.upload') }}
        </div>
        <div class="mt-2 flex items-baseline gap-1">
          <span class="text-4xl font-semibold font-mono tabular-nums text-zinc-50 leading-none">
            {{ upParts.num }}
          </span>
          <span class="text-sm font-mono text-zinc-500">
            {{ upParts.unit }}
          </span>
        </div>
        <div class="mt-4 h-[2px] w-full overflow-hidden rounded-full bg-indigo-500/10">
          <div
            class="h-full w-1/3 rounded-full bg-gradient-to-r from-indigo-400/0 via-indigo-400/80 to-indigo-400/0"
            :class="connected ? 'animate-pulse' : ''"
          ></div>
        </div>
      </div>
    </div>

    <footer
      class="mt-4 flex items-center justify-between text-[11px] font-mono text-zinc-500"
    >
      <span>{{ sinceText }}</span>
      <span class="inline-flex items-center gap-2">
        <span v-if="error" class="text-rose-400">{{ error }}</span>
        <button
          v-else
          @click="reconnect"
          class="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-zinc-500 hover:bg-white/5 hover:text-zinc-300 transition-colors"
          :title="t('common.refresh')"
        >
          <RefreshCw class="h-3 w-3" />
        </button>
      </span>
    </footer>
  </section>
</template>

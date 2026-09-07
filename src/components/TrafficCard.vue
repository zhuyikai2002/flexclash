<script setup lang="ts">
/**
 * TrafficCard — Live upload/download rates.
 *
 * Visual language: dashboard-style numeric panel.
 *   - Hero numbers in mono / tabular-nums so digits don't dance
 *   - Tiny up/down arrows for direction
 *   - Subtle gradient bars under each number (currently static;
 *     could be wired to a sparkline later)
 *   - Connection status: live (emerald) / reconnect (amber)
 *   - Last sample: "5s ago" relative time
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

// Translate numeric rate to "1.42 MB/s" / "423 KB/s" — keep the unit
// separate so we can render the suffix in a softer color.
const upParts = computed(() => splitRate(upText.value))
const downParts = computed(() => splitRate(downText.value))

function splitRate(s: string): { num: string; unit: string } {
  // "1.42 MB/s" → { num: "1.42", unit: "MB/s" }
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
    <header class="mb-5 flex items-center justify-between">
      <div class="text-[11px] font-semibold uppercase tracking-wider text-zinc-400">
        {{ t('dashboard.traffic.title') }}
      </div>
      <div v-if="connected" class="inline-flex items-center gap-1.5 text-[11px] text-emerald-400">
        <span class="relative flex h-2 w-2">
          <span class="absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75 animate-ping" />
          <span class="relative inline-flex h-2 w-2 rounded-full bg-emerald-500" />
        </span>
        live
      </div>
      <button
        v-else
        @click="reconnect"
        class="inline-flex items-center gap-1.5 rounded-full bg-amber-500/10 px-2.5 py-0.5 text-[11px] text-amber-400 hover:bg-amber-500/20 transition-colors"
      >
        <WifiOff class="h-3 w-3" />
        reconnect
      </button>
    </header>

    <div class="grid grid-cols-2 gap-3">
      <!-- Download -->
      <div
        class="group relative overflow-hidden rounded-xl border border-white/5 bg-zinc-950/30 p-4"
      >
        <div class="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-emerald-400/40 to-transparent" />
        <div class="flex items-center gap-2 text-[11px] uppercase tracking-wider text-zinc-400">
          <ArrowDown class="h-3 w-3 text-emerald-400" />
          {{ t('dashboard.traffic.download') }}
        </div>
        <div class="mt-1.5 flex items-baseline gap-1">
          <span class="text-3xl font-semibold font-mono tabular-nums text-zinc-50">
            {{ downParts.num }}
          </span>
          <span class="text-xs font-mono text-zinc-500">
            {{ downParts.unit }}
          </span>
        </div>
      </div>

      <!-- Upload -->
      <div
        class="group relative overflow-hidden rounded-xl border border-white/5 bg-zinc-950/30 p-4"
      >
        <div class="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-indigo-400/40 to-transparent" />
        <div class="flex items-center gap-2 text-[11px] uppercase tracking-wider text-zinc-400">
          <ArrowUp class="h-3 w-3 text-indigo-400" />
          {{ t('dashboard.traffic.upload') }}
        </div>
        <div class="mt-1.5 flex items-baseline gap-1">
          <span class="text-3xl font-semibold font-mono tabular-nums text-zinc-50">
            {{ upParts.num }}
          </span>
          <span class="text-xs font-mono text-zinc-500">
            {{ upParts.unit }}
          </span>
        </div>
      </div>
    </div>

    <footer
      class="mt-4 flex items-center justify-between text-[11px] font-mono text-zinc-500"
    >
      <span>last sample: {{ sinceText }}</span>
      <span class="inline-flex items-center gap-2">
        <span v-if="error" class="text-rose-400">{{ error }}</span>
        <button
          v-else
          @click="reconnect"
          class="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-zinc-500 hover:bg-white/5 hover:text-zinc-300 transition-colors"
          :title="t('common.refresh')"
        >
          <RefreshCw class="h-3 w-3" />
          {{ t('common.refresh') }}
        </button>
      </span>
    </footer>
  </section>
</template>

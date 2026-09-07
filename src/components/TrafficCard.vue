<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { ArrowDown, ArrowUp, RefreshCw, Wifi, WifiOff } from 'lucide-vue-next'
import { formatRate, useTrafficStream } from '@/composables/useTrafficStream'

const { up, down, connected, lastUpdateAt, error, reconnect } = useTrafficStream()

// Live "seconds since last sample" — re-render every 1s without depending on a sample.
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

// Touch `tick` so Vue re-evaluates this computed every second.
const sinceText = computed(() => {
  void tick.value
  if (!lastUpdateAt.value) return '—'
  const ago = Math.floor((Date.now() - lastUpdateAt.value) / 1000)
  if (ago < 1) return 'just now'
  if (ago < 60) return `${ago}s ago`
  return `${Math.floor(ago / 60)}m ${ago % 60}s ago`
})
</script>

<template>
  <section class="bg-zinc-900 border border-zinc-800 rounded-xl p-5">
    <header class="flex items-center justify-between">
      <div class="text-[10px] text-zinc-500 uppercase tracking-widest font-semibold">
        Real-time traffic
      </div>
      <div class="flex items-center gap-2 text-xs">
        <span
          v-if="connected"
          class="inline-flex items-center gap-1 text-emerald-400"
        >
          <Wifi class="w-3 h-3" />
          live
        </span>
        <button
          v-else
          @click="reconnect"
          class="inline-flex items-center gap-1 text-amber-400 hover:text-amber-300"
        >
          <WifiOff class="w-3 h-3" />
          reconnect
        </button>
      </div>
    </header>

    <div class="mt-4 grid grid-cols-2 gap-3">
      <!-- Download -->
      <div class="flex items-center gap-3 px-3 py-3 bg-zinc-950/40 rounded-lg">
        <div
          class="w-10 h-10 rounded-md bg-emerald-500/15 text-emerald-400 flex items-center justify-center"
        >
          <ArrowDown class="w-5 h-5" />
        </div>
        <div>
          <div class="text-[10px] text-zinc-500 uppercase tracking-wider">Download</div>
          <div class="text-lg font-semibold font-mono leading-tight">
            {{ downText }}
          </div>
        </div>
      </div>

      <!-- Upload -->
      <div class="flex items-center gap-3 px-3 py-3 bg-zinc-950/40 rounded-lg">
        <div
          class="w-10 h-10 rounded-md bg-indigo-500/15 text-indigo-400 flex items-center justify-center"
        >
          <ArrowUp class="w-5 h-5" />
        </div>
        <div>
          <div class="text-[10px] text-zinc-500 uppercase tracking-wider">Upload</div>
          <div class="text-lg font-semibold font-mono leading-tight">
            {{ upText }}
          </div>
        </div>
      </div>
    </div>

    <footer
      class="mt-3 text-[10px] text-zinc-600 flex items-center justify-between font-mono"
    >
      <span>last sample: {{ sinceText }}</span>
      <span v-if="error" class="text-rose-400">{{ error }}</span>
      <button
        v-else
        @click="reconnect"
        class="text-zinc-500 hover:text-zinc-300 inline-flex items-center gap-1"
        :title="'Force reconnect'"
      >
        <RefreshCw class="w-3 h-3" /> refresh
      </button>
    </footer>
  </section>
</template>

<script setup lang="ts">
// ============================================================================
// SystemProxyToggle — Dashboard card with a single big switch.
// Fits FlexClash's dark theme.
// ============================================================================

import { onMounted, computed } from 'vue'
import { ShieldCheck, ShieldOff, Loader2 } from 'lucide-vue-next'

import { useProxyStore } from '@/stores/proxy'

const store = useProxyStore()

onMounted(async () => {
  if (!store.status) {
    await store.refresh()
  }
})

const enabled = computed(() => store.enabled)
const port = computed(() => store.port)
const busy = computed(() => store.toggling)
const errMsg = computed(() => store.lastError)

async function flip() {
  try {
    await store.toggle()
  } catch (e) {
    console.error('[proxy] toggle failed', e)
  }
}
</script>

<template>
  <section class="bg-zinc-900 border border-zinc-800 rounded-xl p-5">
    <header class="mb-3 flex items-center justify-between">
      <h2 class="text-sm font-semibold text-zinc-200">
        System proxy
      </h2>
      <span
        v-if="enabled"
        class="inline-flex items-center gap-1 rounded-full bg-emerald-900/50 px-2 py-0.5 text-[11px] font-medium text-emerald-300"
      >
        <ShieldCheck class="h-3 w-3" />
        ON
      </span>
      <span
        v-else
        class="inline-flex items-center gap-1 rounded-full bg-zinc-800 px-2 py-0.5 text-[11px] font-medium text-zinc-400"
      >
        <ShieldOff class="h-3 w-3" />
        OFF
      </span>
    </header>

    <div class="flex items-center gap-4">
      <button
        type="button"
        role="switch"
        :aria-checked="enabled"
        :disabled="busy"
        class="relative inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2 focus:ring-offset-zinc-900 disabled:opacity-60 disabled:cursor-wait"
        :class="enabled ? 'bg-emerald-500' : 'bg-zinc-700'"
        @click="flip"
      >
        <span class="sr-only">Toggle system proxy</span>
        <span
          class="pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow transition duration-200 ease-in-out flex items-center justify-center"
          :class="enabled ? 'translate-x-5' : 'translate-x-0'"
        >
          <Loader2 v-if="busy" class="h-3 w-3 animate-spin text-zinc-500" />
        </span>
      </button>

      <div class="text-sm leading-snug">
        <div v-if="enabled" class="text-zinc-200">
          Forwarding
          <code class="rounded bg-black/40 px-1.5 py-0.5 font-mono text-[11px] text-zinc-100">
            127.0.0.1:{{ port ?? 7890 }}
          </code>
          for all WinINET clients.
        </div>
        <div v-else class="text-zinc-400">
          Direct connection — no system-wide proxy in effect.
        </div>
      </div>
    </div>

    <p
      v-if="errMsg"
      class="mt-3 rounded bg-rose-950/60 px-2 py-1 text-xs text-rose-300 border border-rose-900"
    >
      {{ errMsg }}
    </p>

    <p class="mt-3 text-[11px] text-zinc-500">
      Closing FlexClash automatically restores your previous proxy state.
    </p>
  </section>
</template>

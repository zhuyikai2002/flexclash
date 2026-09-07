<script setup lang="ts">
// ============================================================================
// AutoStartToggle — Dashboard card controlling the Windows autostart entry.
//
// Mirrors the visual language of SystemProxyToggle:
//   * Dark card on zinc-900/800
//   * ON/OFF pill with lucide icons
//   * iOS-style switch with busy spinner
//   * Red error banner on failure
// ============================================================================

import { onMounted, computed } from 'vue'
import {
  Power,
  PowerOff,
  Loader2,
  AlertCircle,
  CheckCircle2,
} from 'lucide-vue-next'

import { useDesktopStore } from '@/stores/desktop'

const store = useDesktopStore()

onMounted(async () => {
  if (!store.initialised) {
    await store.init()
  }
})

const enabled = computed(() => store.enabled)
const busy = computed(() => store.busy)
const errMsg = computed(() => store.lastError)
const silent = computed(() => store.silent)

async function flip() {
  try {
    await store.toggle()
  } catch (e) {
    console.error('[autostart] toggle failed', e)
  }
}
</script>

<template>
  <section class="bg-zinc-900 border border-zinc-800 rounded-xl p-5">
    <header class="mb-3 flex items-center justify-between">
      <h2 class="text-sm font-semibold text-zinc-200">
        Launch at Windows startup
      </h2>
      <span
        v-if="enabled"
        class="inline-flex items-center gap-1 rounded-full bg-emerald-900/50 px-2 py-0.5 text-[11px] font-medium text-emerald-300"
      >
        <CheckCircle2 class="h-3 w-3" />
        ON
      </span>
      <span
        v-else
        class="inline-flex items-center gap-1 rounded-full bg-zinc-800 px-2 py-0.5 text-[11px] font-medium text-zinc-400"
      >
        <PowerOff class="h-3 w-3" />
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
        <span class="sr-only">Toggle autostart</span>
        <span
          class="pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow transition duration-200 ease-in-out flex items-center justify-center"
          :class="enabled ? 'translate-x-5' : 'translate-x-0'"
        >
          <Loader2 v-if="busy" class="h-3 w-3 animate-spin text-zinc-500" />
        </span>
      </button>

      <div class="text-sm leading-snug">
        <div v-if="enabled" class="text-zinc-200">
          <Power class="inline h-3.5 w-3.5 mr-1 align-text-bottom text-emerald-400" />
          FlexClash starts automatically in the tray
          <span class="text-zinc-400">(silent mode)</span>.
        </div>
        <div v-else class="text-zinc-400">
          Not started automatically. Launch from the Start menu or shortcut.
        </div>
      </div>
    </div>

    <p
      v-if="silent"
      class="mt-3 inline-flex items-center gap-1.5 rounded bg-indigo-950/50 border border-indigo-900 px-2 py-1 text-[11px] text-indigo-200"
    >
      <AlertCircle class="h-3 w-3" />
      This session was launched silently by the autostart hook.
    </p>

    <p
      v-if="errMsg"
      class="mt-3 rounded bg-rose-950/60 px-2 py-1 text-xs text-rose-300 border border-rose-900"
    >
      {{ errMsg }}
    </p>

    <p class="mt-3 text-[11px] text-zinc-500">
      Registry entry: <code class="font-mono">HKCU\Software\Microsoft\Windows\CurrentVersion\Run\FlexClash</code>.
      Closing the app keeps the entry; disabling the toggle removes it.
    </p>
  </section>
</template>

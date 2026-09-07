<script setup lang="ts">
// ============================================================================
// TunModeToggle — Dashboard card for M9 TUN mode.
//
// Visual language matches SystemProxyToggle / AutoStartToggle.
// Adds two M9-only affordances:
//   * State badge (off / enabling / on / disabling / failed) so the
//     user can see the elevated mihomo's status at a glance.
//   * "Sweep now" button that triggers the residual-route cleanup
//     without toggling TUN itself.
// ============================================================================

import { onMounted, computed } from 'vue'
import {
  Shield,
  ShieldOff,
  Loader2,
  AlertCircle,
  CheckCircle2,
  Network,
  Trash2,
} from 'lucide-vue-next'

import { useTunStore } from '@/stores/tun'

const store = useTunStore()

onMounted(async () => {
  if (!store.initialised) {
    await store.init()
  }
})

const isOn = computed(() => store.isOn)
const busy = computed(() => store.busy || store.isTransitioning)
const errMsg = computed(() => store.lastError)
const stateLabel = computed(() => store.stateLabel)
const state = computed(() => store.state)

async function flip() {
  try {
    await store.toggle()
  } catch (e) {
    console.error('[tun] toggle failed', e)
  }
}

async function sweep() {
  try {
    await store.runSweep()
  } catch (e) {
    console.error('[tun] sweep failed', e)
  }
}
</script>

<template>
  <section class="bg-zinc-900 border border-zinc-800 rounded-xl p-5">
    <header class="mb-3 flex items-center justify-between">
      <h2 class="text-sm font-semibold text-zinc-200">
        TUN mode (transparent proxy)
      </h2>
      <span
        v-if="isOn"
        class="inline-flex items-center gap-1 rounded-full bg-emerald-900/50 px-2 py-0.5 text-[11px] font-medium text-emerald-300"
      >
        <CheckCircle2 class="h-3 w-3" />
        ON
      </span>
      <span
        v-else-if="state === 'failed'"
        class="inline-flex items-center gap-1 rounded-full bg-rose-900/60 px-2 py-0.5 text-[11px] font-medium text-rose-200"
      >
        <AlertCircle class="h-3 w-3" />
        FAILED
      </span>
      <span
        v-else-if="busy"
        class="inline-flex items-center gap-1 rounded-full bg-indigo-900/50 px-2 py-0.5 text-[11px] font-medium text-indigo-200"
      >
        <Loader2 class="h-3 w-3 animate-spin" />
        {{ stateLabel }}
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
        :aria-checked="isOn"
        :disabled="busy"
        class="relative inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2 focus:ring-offset-zinc-900 disabled:opacity-60 disabled:cursor-wait"
        :class="isOn ? 'bg-emerald-500' : 'bg-zinc-700'"
        @click="flip"
      >
        <span class="sr-only">Toggle TUN mode</span>
        <span
          class="pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow transition duration-200 ease-in-out flex items-center justify-center"
          :class="isOn ? 'translate-x-5' : 'translate-x-0'"
        >
          <Loader2 v-if="busy" class="h-3 w-3 animate-spin text-zinc-500" />
        </span>
      </button>

      <div class="text-sm leading-snug">
        <div v-if="isOn" class="text-zinc-200">
          <Shield class="inline h-3.5 w-3.5 mr-1 align-text-bottom text-emerald-400" />
          All traffic flows through the Wintun device
          <code class="font-mono text-zinc-400">{{ store.device }}</code>.
        </div>
        <div v-else-if="state === 'failed'" class="text-rose-200">
          TUN failed to start. See error below.
        </div>
        <div v-else class="text-zinc-400">
          <Network class="inline h-3.5 w-3.5 mr-1 align-text-bottom" />
          TUN is off. Only apps using the system proxy are routed.
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
      Enabling TUN triggers a UAC prompt to relaunch Mihomo elevated.
      The TUN device
      <code class="font-mono">{{ store.device }}</code>
      is created on success; routes and the device are removed on
      disable or app exit.
    </p>

    <div class="mt-3 flex items-center gap-2">
      <button
        type="button"
        class="inline-flex items-center gap-1.5 rounded border border-zinc-700 bg-zinc-800/60 px-2.5 py-1 text-[11px] text-zinc-200 hover:bg-zinc-800 disabled:opacity-50"
        :disabled="busy"
        @click="sweep"
      >
        <Trash2 class="h-3 w-3" />
        Clean residual routes
      </button>
      <span
        v-if="store.lastSweep"
        class="text-[10.5px] text-zinc-500"
      >
        last sweep: {{ store.lastSweep.deleted_routes }} routes,
        {{ store.lastSweep.deleted_adapters }} adapter(s) —
        {{ store.lastSweep.message }}
      </span>
    </div>
  </section>
</template>

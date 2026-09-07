<script setup lang="ts">
/**
 * TunModeToggle — Dashboard card for M9 TUN mode.
 *
 * Custom (not ToggleCard) because it has two extra affordances:
 *   - state pill (off / enabling / on / disabling / failed)
 *   - "sweep" button for residual-route cleanup
 */
import { onMounted, computed } from 'vue'
import {
  Shield, ShieldOff, Loader2, AlertCircle, CheckCircle2,
  Network, Trash2,
} from 'lucide-vue-next'
import { useTunStore } from '@/stores/tun'
import { useI18n } from '@/composables/useI18n'

const store = useTunStore()
const { t } = useI18n()

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
  try { await store.toggle() } catch (e) { console.error('[tun] toggle failed', e) }
}
async function sweep() {
  try { await store.runSweep() } catch (e) { console.error('[tun] sweep failed', e) }
}
</script>

<template>
  <section
    class="rounded-2xl border border-white/5 bg-white/[0.04] p-5 backdrop-blur-md"
  >
    <header class="mb-3 flex items-center justify-between">
      <h2 class="text-sm font-semibold text-zinc-100">
        {{ t('dashboard.toggles.tun.title') }}
      </h2>
      <span
        v-if="isOn"
        class="inline-flex items-center gap-1 rounded-full bg-emerald-500/15 px-2 py-0.5 text-[11px] font-medium text-emerald-300"
      >
        <CheckCircle2 class="h-3 w-3" /> ON
      </span>
      <span
        v-else-if="state === 'failed'"
        class="inline-flex items-center gap-1 rounded-full bg-rose-500/15 px-2 py-0.5 text-[11px] font-medium text-rose-300"
      >
        <AlertCircle class="h-3 w-3" /> FAILED
      </span>
      <span
        v-else-if="busy"
        class="inline-flex items-center gap-1 rounded-full bg-amber-500/15 px-2 py-0.5 text-[11px] font-medium text-amber-300"
      >
        <Loader2 class="h-3 w-3 animate-spin" /> {{ stateLabel }}
      </span>
      <span
        v-else
        class="inline-flex items-center gap-1 rounded-full bg-white/5 px-2 py-0.5 text-[11px] font-medium text-zinc-400"
      >
        <ShieldOff class="h-3 w-3" /> OFF
      </span>
    </header>

    <p class="text-xs leading-relaxed text-zinc-400">
      {{ t('dashboard.toggles.tun.description') }}
    </p>

    <div class="mt-4 flex items-center gap-3">
      <button
        type="button"
        role="switch"
        :aria-checked="isOn"
        :disabled="busy"
        :class="[
          'relative inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus:ring-2 focus:ring-indigo-500/50 disabled:opacity-60 disabled:cursor-wait',
          isOn ? 'bg-emerald-500' : 'bg-zinc-700'
        ]"
        @click="flip"
      >
        <span class="sr-only">Toggle TUN</span>
        <span
          :class="[
            'pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow transition duration-200 ease-in-out',
            isOn ? 'translate-x-5' : 'translate-x-0'
          ]"
        >
          <span class="flex h-full w-full items-center justify-center">
            <Loader2 v-if="busy" class="h-3 w-3 animate-spin text-zinc-500" />
          </span>
        </span>
      </button>

      <p v-if="isOn" class="text-xs text-zinc-300">
        <Shield class="inline h-3.5 w-3.5 mr-1 align-text-bottom text-emerald-400" />
        Wintun <code class="font-mono text-zinc-400">{{ store.device }}</code> active
      </p>
      <p v-else class="text-xs text-zinc-500">
        <Network class="inline h-3.5 w-3.5 mr-1 align-text-bottom" />
        Only apps using the system proxy are routed
      </p>
    </div>

    <p
      v-if="errMsg"
      class="mt-3 rounded-lg bg-rose-500/10 border border-rose-500/20 px-2.5 py-1.5 text-xs text-rose-300"
    >
      {{ errMsg }}
    </p>

    <div class="mt-4 flex items-center gap-2">
      <button
        type="button"
        class="inline-flex items-center gap-1.5 rounded-lg border border-white/5 bg-white/[0.04] px-2.5 py-1 text-[11px] text-zinc-300 hover:bg-white/[0.08] hover:border-white/10 disabled:opacity-50 transition-colors"
        :disabled="busy"
        @click="sweep"
      >
        <Trash2 class="h-3 w-3" />
        Clean residual routes
      </button>
      <span v-if="store.lastSweep" class="text-[10.5px] text-zinc-500 font-mono">
        last sweep: {{ store.lastSweep.deleted_routes }} routes,
        {{ store.lastSweep.deleted_adapters }} adapter(s)
      </span>
    </div>
  </section>
</template>

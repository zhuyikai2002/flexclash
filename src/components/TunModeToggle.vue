<script setup lang="ts">
/**
 * TunModeToggle — TUN on/off card with state pill + sweep button.
 *
 * Uses the new ToggleCard visual language (icon chip, brand halo, full-
 * card click target) but extends it with two M9-specific affordances:
 *   - a state pill (off / enabling / on / disabling / failed) that
 *     reflects the full M9 state machine, not just a boolean
 *   - a "sweep" action button to clean residual routes after a hard
 *     crash / ungraceful shutdown
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

const accent = computed<'emerald' | 'amber' | 'sky'>(() => {
  if (state.value === 'failed') return 'amber'
  if (isOn.value) return 'emerald'
  return 'sky'
})
</script>

<template>
  <button
    type="button"
    role="switch"
    :aria-checked="isOn"
    :disabled="busy"
    :class="[
      'group relative w-full overflow-hidden rounded-2xl border p-4 text-left transition-all duration-200',
      'focus:outline-none focus-visible:ring-2 focus-visible:ring-offset-2 focus-visible:ring-offset-zinc-950 active:scale-[0.99]',
      isOn
        ? 'border-white/15 bg-white/[0.05] focus-visible:ring-emerald-400/50 shadow-lg shadow-emerald-500/5'
        : 'border-white/5 bg-white/[0.04] hover:border-white/10 hover:bg-white/[0.06] focus-visible:ring-sky-400/50',
      busy && 'cursor-wait',
    ]"
    @click="flip"
  >
    <!-- Brand halo when ON -->
    <div
      v-if="isOn"
      class="pointer-events-none absolute inset-0 bg-gradient-to-br from-emerald-500/15 via-emerald-500/5 to-transparent opacity-60"
    ></div>
    <div
      v-else-if="state === 'failed'"
      class="pointer-events-none absolute inset-0 bg-gradient-to-br from-amber-500/15 via-amber-500/5 to-transparent opacity-60"
    ></div>

    <div class="relative flex items-center gap-3">
      <!-- Icon chip -->
      <div
        :class="[
          'flex h-10 w-10 shrink-0 items-center justify-center rounded-xl transition-all',
          isOn
            ? 'bg-emerald-500/15 text-emerald-300 ring-1 ring-emerald-400/20'
            : state === 'failed'
              ? 'bg-amber-500/15 text-amber-300 ring-1 ring-amber-400/20'
              : 'bg-white/[0.04] text-zinc-500 ring-1 ring-white/5',
        ]"
      >
        <Loader2 v-if="busy" class="h-4.5 w-4.5 animate-spin text-zinc-400" />
        <Shield v-else-if="isOn" class="h-4.5 w-4.5 text-emerald-300" />
        <ShieldOff v-else class="h-4.5 w-4.5 text-zinc-500" />
      </div>

      <!-- Title + description -->
      <div class="min-w-0 flex-1">
        <div class="flex items-center gap-2">
          <h3 class="text-sm font-semibold text-zinc-100 truncate">
            {{ t('dashboard.toggles.tun.title') }}
          </h3>
          <span
            v-if="isOn"
            class="inline-flex items-center gap-1 rounded-full bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-300"
          >
            <CheckCircle2 class="h-2.5 w-2.5" />
            {{ t('common.on') }}
          </span>
          <span
            v-else-if="state === 'failed'"
            class="inline-flex items-center gap-1 rounded-full bg-rose-500/15 px-1.5 py-0.5 text-[10px] font-medium text-rose-300"
          >
            <AlertCircle class="h-2.5 w-2.5" />
            FAILED
          </span>
          <span
            v-else-if="busy"
            class="inline-flex items-center gap-1 rounded-full bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-300"
          >
            <Loader2 class="h-2.5 w-2.5 animate-spin" />
            {{ stateLabel }}
          </span>
          <span
            v-else
            class="inline-flex items-center gap-1 rounded-full bg-white/5 px-1.5 py-0.5 text-[10px] font-medium text-zinc-500"
          >
            <ShieldOff class="h-2.5 w-2.5" />
            {{ t('common.off') }}
          </span>
        </div>
        <p class="mt-0.5 text-xs leading-relaxed text-zinc-400 line-clamp-2">
          {{ t('dashboard.toggles.tun.description') }}
        </p>
        <p
          v-if="isOn"
          class="mt-1 text-[11px] text-zinc-300"
        >
          <Shield class="inline h-3 w-3 mr-1 align-text-bottom text-emerald-400" />
          Wintun <code class="font-mono text-zinc-400">{{ store.device }}</code> active
        </p>
        <p
          v-else
          class="mt-1 text-[11px] text-zinc-500"
        >
          <Network class="inline h-3 w-3 mr-1 align-text-bottom" />
          Only apps using the system proxy are routed
        </p>
      </div>

      <!-- iOS-style switch -->
      <span
        :class="[
          'relative inline-flex h-6 w-11 shrink-0 items-center rounded-full border-2 border-transparent transition-colors duration-200',
          isOn ? accent.split(' ')[0] === 'emerald' ? 'bg-emerald-500' : 'bg-amber-500' : 'bg-zinc-700',
        ]"
      >
        <span
          :class="[
            'pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow transition duration-200',
            isOn ? 'translate-x-5' : 'translate-x-0',
          ]"
        ></span>
      </span>
    </div>

    <p
      v-if="errMsg"
      class="relative mt-3 rounded-lg bg-rose-500/10 border border-rose-500/20 px-2.5 py-1.5 text-[11px] text-rose-300"
    >
      {{ errMsg }}
    </p>

    <!-- Sweep action -->
    <div class="relative mt-3 flex items-center gap-2">
      <button
        type="button"
        class="inline-flex items-center gap-1.5 rounded-lg border border-white/5 bg-white/[0.04] px-2.5 py-1 text-[11px] text-zinc-300 hover:bg-white/[0.08] hover:border-white/10 disabled:opacity-50 transition-colors"
        :disabled="busy"
        @click.stop="sweep"
      >
        <Trash2 class="h-3 w-3" />
        Clean residual routes
      </button>
      <span v-if="store.lastSweep" class="text-[10.5px] text-zinc-500 font-mono">
        last sweep: {{ store.lastSweep.deleted_routes }} routes,
        {{ store.lastSweep.deleted_adapters }} adapter(s)
      </span>
    </div>
  </button>
</template>

<script setup lang="ts">
/**
 * AutokillToggle — the UI half of the dead-link autopilot's escape hatch.
 *
 * Deliberately NOT built on `ToggleCard.vue`: that component's only inert
 * state is `busy` (spinner + "wait"), which is the wrong signal for "the
 * environment has vetoed this and no click will ever change it". A locked
 * switch has to look locked, not merely busy.
 *
 * The store is the single source of truth; this component only renders it and
 * forwards clicks, so Settings and any future surface cannot disagree.
 */
import { computed, onMounted } from 'vue'
import { useI18n } from '@/composables/useI18n'
import { Loader2, Lock, Zap } from 'lucide-vue-next'
import { useAutokillStore } from '@/stores/autokill'

const { t } = useI18n()
const store = useAutokillStore()

onMounted(() => {
  void store.refresh()
})

/** Busy is transient; locked is permanent. Both must refuse clicks. */
const disabled = computed(() => store.isLocked)

/** The hint line explains *which* switch is in charge right now. */
const hint = computed(() => {
  if (store.envLocked) return t('settings.diagnostics.autokill_env_locked')
  if (store.busy) return ''
  return t('settings.diagnostics.autokill_hint')
})
</script>

<template>
  <button
    type="button"
    role="switch"
    :aria-checked="store.enabled"
    :disabled="disabled"
    :class="[
      'group relative w-full overflow-hidden rounded-xl border p-4 text-left transition-all duration-200',
      'focus:outline-none focus-visible:ring-2 focus-visible:ring-offset-2 focus-visible:ring-offset-zinc-950',
      store.envLocked
        ? 'cursor-not-allowed border-white/5 bg-white/[0.02] opacity-70'
        : 'border-white/5 bg-white/[0.04] hover:border-white/10 hover:bg-white/[0.06] active:scale-[0.995]',
    ]"
    @click="store.toggle()"
  >
    <div
      class="pointer-events-none absolute inset-0 bg-gradient-to-br from-emerald-500/10 via-emerald-500/5 to-transparent transition-opacity duration-200"
      :class="store.enabled && !store.envLocked ? 'opacity-100' : 'opacity-0'"
    ></div>

    <div class="relative flex items-center gap-3">
      <!-- Icon chip -->
      <div
        :class="[
          'flex h-10 w-10 shrink-0 items-center justify-center rounded-xl transition-all',
          store.envLocked
            ? 'bg-white/[0.03] text-zinc-600 ring-1 ring-white/5'
            : store.enabled
              ? 'bg-emerald-500/15 text-emerald-300 ring-1 ring-emerald-400/20'
              : 'bg-white/[0.04] text-zinc-500 ring-1 ring-white/5',
        ]"
      >
        <Loader2 v-if="store.busy" class="h-4.5 w-4.5 animate-spin text-zinc-400" />
        <Lock v-else-if="store.envLocked" class="h-4.5 w-4.5" />
        <Zap v-else class="h-4.5 w-4.5" />
      </div>

      <!-- Title + description -->
      <div class="min-w-0 flex-1">
        <div class="flex items-center gap-2">
          <h3 class="text-sm font-semibold text-zinc-100 truncate">
            {{ t('settings.diagnostics.autokill_title') }}
          </h3>
          <span
            v-if="store.envLocked"
            class="inline-flex items-center gap-1 rounded-full bg-zinc-500/15 px-1.5 py-0.5 text-[10px] font-medium text-zinc-400"
          >
            <Lock class="h-2.5 w-2.5" />
            {{ t('settings.diagnostics.autokill_locked_badge') }}
          </span>
          <span
            v-else-if="store.enabled"
            class="inline-flex items-center gap-1 rounded-full bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-300"
          >
            ON
          </span>
          <span
            v-else
            class="inline-flex items-center gap-1 rounded-full bg-white/5 px-1.5 py-0.5 text-[10px] font-medium text-zinc-500"
          >
            OFF
          </span>
        </div>
        <p class="mt-0.5 text-xs leading-relaxed text-zinc-400">
          {{ t('settings.diagnostics.autokill_desc') }}
        </p>
      </div>

      <!-- iOS-style switch -->
      <span
        :class="[
          'relative inline-flex h-6 w-11 shrink-0 items-center rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out',
          store.enabled && !store.envLocked ? 'bg-emerald-500' : 'bg-zinc-700',
          store.envLocked && 'opacity-60',
        ]"
      >
        <span
          :class="[
            'pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow transition duration-200 ease-in-out',
            store.enabled ? 'translate-x-5' : 'translate-x-0',
          ]"
        ></span>
      </span>
    </div>

    <!-- Footer: error wins, then the reason the switch is inert -->
    <p
      v-if="store.lastError"
      class="relative mt-3 rounded-lg bg-rose-500/10 border border-rose-500/20 px-2.5 py-1.5 text-[11px] text-rose-300"
    >
      {{ store.lastError }}
    </p>
    <p
      v-else-if="hint"
      :class="[
        'relative mt-3 text-[10.5px] font-mono',
        store.envLocked ? 'text-zinc-500' : 'text-zinc-600',
      ]"
    >
      {{ hint }}
    </p>
  </button>
</template>

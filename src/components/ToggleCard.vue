<script setup lang="ts">
/**
 * ToggleCard — generic on/off card used by system-proxy / autostart / tun.
 *
 * Visual:
 *   - Rounded-2xl glass card
 *   - Header: icon chip + title + state pill
 *   - Body: iOS-style switch + description + detail
 *   - Footer: error / hint
 */
import { computed } from 'vue'
import { Check, Loader2, Power } from 'lucide-vue-next'

const props = defineProps<{
  enabled: boolean
  busy: boolean
  title: string
  description: string
  detail?: string
  error?: string | null
  hint?: string
  accent?: 'indigo' | 'emerald' | 'amber'
}>()

const emit = defineEmits<{ toggle: [] }>()

const accentRing = computed(() => {
  switch (props.accent) {
    case 'emerald': return 'bg-emerald-500'
    case 'amber':   return 'bg-amber-500'
    default:        return 'bg-indigo-500'
  }
})
</script>

<template>
  <section
    class="rounded-2xl border border-white/5 bg-white/[0.04] p-5 backdrop-blur-md"
  >
    <header class="mb-3 flex items-center justify-between">
      <h2 class="text-sm font-semibold text-zinc-100">
        {{ title }}
      </h2>
      <span
        v-if="enabled"
        class="inline-flex items-center gap-1 rounded-full bg-emerald-500/15 px-2 py-0.5 text-[11px] font-medium text-emerald-300"
      >
        <Check class="h-3 w-3" />
        ON
      </span>
      <span
        v-else
        class="inline-flex items-center gap-1 rounded-full bg-white/5 px-2 py-0.5 text-[11px] font-medium text-zinc-400"
      >
        <Power class="h-3 w-3" />
        OFF
      </span>
    </header>

    <p class="text-xs leading-relaxed text-zinc-400">
      {{ description }}
    </p>

    <div class="mt-4 flex items-center gap-3">
      <button
        type="button"
        role="switch"
        :aria-checked="enabled"
        :disabled="busy"
        :class="[
          'relative inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus:ring-2 focus:ring-indigo-500/50 disabled:opacity-60 disabled:cursor-wait',
          enabled ? accentRing : 'bg-zinc-700'
        ]"
        @click="emit('toggle')"
      >
        <span class="sr-only">Toggle</span>
        <span
          :class="[
            'pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow transition duration-200 ease-in-out',
            enabled ? 'translate-x-5' : 'translate-x-0'
          ]"
        >
          <span class="flex h-full w-full items-center justify-center">
            <Loader2 v-if="busy" class="h-3 w-3 animate-spin text-zinc-500" />
          </span>
        </span>
      </button>

      <p v-if="detail" class="text-xs text-zinc-300 font-mono">
        {{ detail }}
      </p>
    </div>

    <p
      v-if="error"
      class="mt-3 rounded-lg bg-rose-500/10 border border-rose-500/20 px-2.5 py-1.5 text-xs text-rose-300"
    >
      {{ error }}
    </p>
    <p
      v-else-if="hint"
      class="mt-3 text-[11px] text-zinc-500"
    >
      {{ hint }}
    </p>
  </section>
</template>

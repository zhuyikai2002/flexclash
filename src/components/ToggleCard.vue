<script setup lang="ts">
/**
 * ToggleCard — premium on/off control card.
 *
 * Design goals (Phase 4):
 *   - The whole card is clickable (large hit target, Fitts's law)
 *   - ON state has a brand gradient halo (sky → indigo) at low opacity
 *   - Icon sits in a colored chip that mirrors the `accent` palette
 *   - iOS-style switch on the right is the visual anchor
 *
 * Accent palette:
 *   - emerald  → proxy / "on" success states
 *   - indigo   → autostart / neutral-blue system actions
 *   - amber    → warning-ish / TUN transition
 *   - sky      → live indicators
 */
import { computed } from 'vue'
import { Check, Loader2, Power } from 'lucide-vue-next'
import type { Component } from 'vue'

const props = withDefaults(
  defineProps<{
    enabled: boolean
    busy: boolean
    title: string
    description: string
    icon?: Component
    detail?: string
    error?: string | null
    hint?: string
    accent?: 'indigo' | 'emerald' | 'amber' | 'sky'
  }>(),
  { accent: 'indigo' },
)

const emit = defineEmits<{ toggle: [] }>()

const palette = computed(() => {
  switch (props.accent) {
    case 'emerald':
      return {
        switchOn:  'bg-emerald-500',
        chip:      'bg-emerald-500/15 text-emerald-300 ring-1 ring-emerald-400/20',
        halo:      'from-emerald-500/15 via-emerald-500/5 to-transparent',
        iconColor: 'text-emerald-300',
        badgeOn:   'bg-emerald-500/15 text-emerald-300',
      }
    case 'amber':
      return {
        switchOn:  'bg-amber-500',
        chip:      'bg-amber-500/15 text-amber-300 ring-1 ring-amber-400/20',
        halo:      'from-amber-500/15 via-amber-500/5 to-transparent',
        iconColor: 'text-amber-300',
        badgeOn:   'bg-amber-500/15 text-amber-300',
      }
    case 'sky':
      return {
        switchOn:  'bg-sky-500',
        chip:      'bg-sky-500/15 text-sky-300 ring-1 ring-sky-400/20',
        halo:      'from-sky-500/15 via-sky-500/5 to-transparent',
        iconColor: 'text-sky-300',
        badgeOn:   'bg-sky-500/15 text-sky-300',
      }
    default:
      return {
        switchOn:  'bg-indigo-500',
        chip:      'bg-indigo-500/15 text-indigo-300 ring-1 ring-indigo-400/20',
        halo:      'from-indigo-500/15 via-indigo-500/5 to-transparent',
        iconColor: 'text-indigo-300',
        badgeOn:   'bg-indigo-500/15 text-indigo-300',
      }
  }
})
</script>

<template>
  <button
    type="button"
    role="switch"
    :aria-checked="enabled"
    :disabled="busy"
    :class="[
      'group relative w-full overflow-hidden rounded-2xl border p-4 text-left transition-all duration-200',
      'focus:outline-none focus-visible:ring-2 focus-visible:ring-offset-2 focus-visible:ring-offset-zinc-950',
      'active:scale-[0.99]',
      enabled
        ? 'border-white/15 bg-white/[0.05] focus-visible:ring-sky-400/50 shadow-lg shadow-sky-500/5'
        : 'border-white/5 bg-white/[0.04] hover:border-white/10 hover:bg-white/[0.06] focus-visible:ring-indigo-400/50',
      busy && 'cursor-wait',
    ]"
    @click="emit('toggle')"
  >
    <!-- Brand-tinted halo when ON -->
    <div
      v-if="enabled"
      :class="['pointer-events-none absolute inset-0 bg-gradient-to-br opacity-60', palette.halo]"
    ></div>

    <div class="relative flex items-center gap-3">
      <!-- Icon chip -->
      <div
        :class="[
          'flex h-10 w-10 shrink-0 items-center justify-center rounded-xl transition-all',
          enabled ? palette.chip : 'bg-white/[0.04] text-zinc-500 ring-1 ring-white/5',
        ]"
      >
        <Loader2 v-if="busy" class="h-4.5 w-4.5 animate-spin text-zinc-400" />
        <component v-else-if="icon" :is="icon" class="h-4.5 w-4.5" :class="enabled ? palette.iconColor : 'text-zinc-500'" />
        <Power v-else class="h-4.5 w-4.5" :class="enabled ? palette.iconColor : 'text-zinc-500'" />
      </div>

      <!-- Title + description -->
      <div class="min-w-0 flex-1">
        <div class="flex items-center gap-2">
          <h3 class="text-sm font-semibold text-zinc-100 truncate">
            {{ title }}
          </h3>
          <span
            v-if="enabled"
            :class="['inline-flex items-center gap-1 rounded-full px-1.5 py-0.5 text-[10px] font-medium', palette.badgeOn]"
          >
            <Check class="h-2.5 w-2.5" />
            ON
          </span>
          <span
            v-else-if="!busy"
            class="inline-flex items-center gap-1 rounded-full bg-white/5 px-1.5 py-0.5 text-[10px] font-medium text-zinc-500"
          >
            <Power class="h-2.5 w-2.5" />
            OFF
          </span>
          <span
            v-else
            class="inline-flex items-center gap-1 rounded-full bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-300"
          >
            <Loader2 class="h-2.5 w-2.5 animate-spin" />
            …
          </span>
        </div>
        <p class="mt-0.5 text-xs leading-relaxed text-zinc-400 line-clamp-2">
          {{ description }}
        </p>
        <p
          v-if="detail"
          class="mt-1 text-[11px] text-zinc-300 font-mono truncate"
        >
          {{ detail }}
        </p>
      </div>

      <!-- iOS-style switch (visual anchor; the whole card is the click target) -->
      <span
        :class="[
          'relative inline-flex h-6 w-11 shrink-0 items-center rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out',
          enabled ? palette.switchOn : 'bg-zinc-700',
        ]"
      >
        <span
          :class="[
            'pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow transition duration-200 ease-in-out',
            enabled ? 'translate-x-5' : 'translate-x-0',
          ]"
        ></span>
      </span>
    </div>

    <!-- Footer: error takes priority, otherwise hint -->
    <p
      v-if="error"
      class="relative mt-3 rounded-lg bg-rose-500/10 border border-rose-500/20 px-2.5 py-1.5 text-[11px] text-rose-300"
    >
      {{ error }}
    </p>
    <p
      v-else-if="hint"
      class="relative mt-3 text-[10.5px] text-zinc-500 font-mono truncate"
    >
      {{ hint }}
    </p>
  </button>
</template>

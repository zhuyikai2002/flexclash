<script setup lang="ts">
/**
 * Sidebar.vue — slim 68px left navigation rail.
 *
 *   ┌────┐
 *   │ 🐱 │  ← static neon brand logo (top)
 *   ├────┤
 *   │ D  │  ← dashboard
 *   │ P  │  ← proxies
 *   │ C  │  ← connections
 *   │ S  │  ← profiles
 *   │ T  │  ← stats
 *   │ ─  │  ← divider
 *   │ ⚙  │  ← settings (config destination, separate from data views)
 *   │    │
 *   │ 🌐 │  ← language switcher
 *   │ ●  │  ← kernel status indicator
 *   └────┘
 *
 * Layout reference: "Clash Party" sidebar — wide enough for an icon +
 * a small badge, narrow enough to free up screen real-estate for the
 * main content grid.
 */
import { computed } from 'vue'
import {
  LayoutDashboard, Network, Plug, Layers, BarChart3, Settings as SettingsIcon,
} from 'lucide-vue-next'
import { useI18n } from '@/composables/useI18n'
import LanguageSwitcher from './LanguageSwitcher.vue'
import StatusBadge from './StatusBadge.vue'
import type { KernelState } from '@/types/clash'

type TabId = 'dashboard' | 'proxies' | 'connections' | 'profiles' | 'stats' | 'settings'

const props = defineProps<{
  modelValue: TabId
  kernelState: KernelState
  connCount: number
  profileCount: number
}>()

const emit = defineEmits<{ 'update:modelValue': [v: TabId] }>()

const { t } = useI18n()

// Primary data-view tabs: top of the rail.  These are the screens
// the user "looks at" (live traffic, node list, connection dump).
const primaryTabs = computed(() => [
  { id: 'dashboard'   as TabId, icon: LayoutDashboard, label: t('nav.dashboard'),   badge: 0 },
  { id: 'proxies'     as TabId, icon: Network,         label: t('nav.proxies'),     badge: 0 },
  { id: 'connections' as TabId, icon: Plug,            label: t('nav.connections'), badge: props.connCount },
  { id: 'profiles'    as TabId, icon: Layers,          label: t('nav.profiles'),    badge: props.profileCount },
  { id: 'stats'       as TabId, icon: BarChart3,       label: t('nav.stats'),       badge: 0 },
])

// Settings tab: bottom of the rail, separated by a divider.  This
// is a "config destination" rather than a live data view, so it
// gets its own visual group.
const settingsTab = computed(() => ({
  id: 'settings' as TabId,
  icon: SettingsIcon,
  label: t('nav.settings'),
  badge: 0,
}))

function pick(id: TabId) {
  emit('update:modelValue', id)
}
</script>

<template>
  <div class="flex h-full w-full flex-col items-center py-4">
    <!-- Top: brand logo (static inline SVG, no mask dependency) -->
    <div
      class="relative flex h-10 w-10 items-center justify-center rounded-xl bg-sky-500/10 border border-sky-500/20 shadow-[0_0_15px_rgba(56,189,248,0.2)]"
      :title="t('app.name')"
      aria-hidden="true"
    >
      <svg
        viewBox="0 0 32 32"
        class="h-6 w-6 text-sky-400"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <path
          d="M 8 7 L 11 14 C 13 13.5, 19 13.5, 21 14 L 24 7 C 26 10, 26 15, 25 19 C 24 25, 20 27, 16 27 C 12 27, 8 25, 7 19 C 6 15, 6 10, 8 7 Z"
          fill="rgba(255,255,255,0.06)"
        />
        <circle cx="12" cy="18" r="1.5" fill="currentColor" />
        <circle cx="20" cy="18" r="1.5" fill="currentColor" />
      </svg>
    </div>

    <!-- Middle: vertical capsule nav (data views) -->
    <nav class="mt-6 flex flex-col items-center gap-1">
      <button
        v-for="tab in primaryTabs"
        :key="tab.id"
        type="button"
        :title="tab.label"
        :aria-label="tab.label"
        :aria-current="modelValue === tab.id ? 'page' : undefined"
        :class="[
          'group relative flex h-10 w-10 items-center justify-center rounded-xl transition-all duration-150',
          modelValue === tab.id
            ? 'bg-white/10 text-zinc-100 ring-1 ring-white/10 shadow-md shadow-sky-500/10'
            : 'text-zinc-500 hover:text-zinc-200 hover:bg-white/[0.04]'
        ]"
        @click="pick(tab.id)"
      >
        <component :is="tab.icon" class="h-4 w-4" />
        <!-- Badge: a tiny pill in the top-right corner when count > 0 -->
        <span
          v-if="tab.badge > 0"
          class="absolute -top-0.5 -right-0.5 min-w-[14px] h-[14px] px-1 rounded-full bg-indigo-500 text-white text-[9px] font-mono font-medium flex items-center justify-center ring-2 ring-zinc-950"
        >
          {{ tab.badge > 99 ? '99+' : tab.badge }}
        </span>
        <!-- Active indicator: a thin gradient bar on the left edge -->
        <span
          v-if="modelValue === tab.id"
          class="absolute -left-3 top-1/2 -translate-y-1/2 h-5 w-[3px] rounded-r-full bg-gradient-to-b from-sky-400 to-indigo-400"
        ></span>
      </button>
    </nav>

    <!-- Spacer pushes the bottom group to the floor of the rail. -->
    <div class="flex-1"></div>

    <!-- Divider above the settings tab — signals "config destination". -->
    <div class="my-3 h-px w-6 bg-white/10"></div>

    <!-- Settings tab (single icon, separated by a divider). -->
    <button
      type="button"
      :title="settingsTab.label"
      :aria-label="settingsTab.label"
      :aria-current="modelValue === settingsTab.id ? 'page' : undefined"
      :class="[
        'group relative flex h-10 w-10 items-center justify-center rounded-xl transition-all duration-150',
        modelValue === settingsTab.id
          ? 'bg-white/10 text-zinc-100 ring-1 ring-white/10 shadow-md shadow-sky-500/10'
          : 'text-zinc-500 hover:text-zinc-200 hover:bg-white/[0.04]'
      ]"
      @click="pick(settingsTab.id)"
    >
      <component :is="settingsTab.icon" class="h-4 w-4" />
      <span
        v-if="modelValue === settingsTab.id"
        class="absolute -left-3 top-1/2 -translate-y-1/2 h-5 w-[3px] rounded-r-full bg-gradient-to-b from-sky-400 to-indigo-400"
      ></span>
    </button>

    <!-- Bottom: language + status (light-mode toggle is locked off). -->
    <div class="mt-3 flex flex-col items-center gap-2.5">
      <LanguageSwitcher compact />
      <StatusBadge :state="kernelState" compact />
    </div>
  </div>
</template>

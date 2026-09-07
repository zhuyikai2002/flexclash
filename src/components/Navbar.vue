<script setup lang="ts">
/**
 * Navbar — segmented control (capsule) top navigation.
 *
 * - 4 tabs: dashboard / connections / profiles / stats
 * - Active item has a sliding white background
 * - Optional badge per tab (counts)
 * - LanguageSwitcher + StatusBadge on the right
 * - Brand logo = SVG cat (src/assets/navbar-logo.svg) used as a
 *   `mask-image`, so a sky-500 → indigo-400 gradient is "cut into"
 *   the cat silhouette.  This mirrors the gradient of the tray-active
 *   icon and the main app-icon.svg at a small scale, keeping the
 *   visual identity consistent across window title bar / tray /
 *   navbar.
 */
import { computed } from 'vue'
import { LayoutDashboard, Activity, Layers, BarChart3 } from 'lucide-vue-next'
import { useI18n } from '@/composables/useI18n'
import LanguageSwitcher from './LanguageSwitcher.vue'
import StatusBadge from './StatusBadge.vue'
import type { KernelState } from '@/types/clash'
import navbarLogoUrl from '@/assets/navbar-logo.svg?url'

const props = defineProps<{
  modelValue: 'dashboard' | 'connections' | 'profiles' | 'stats'
  kernelState: KernelState
  connCount: number
  profileCount: number
}>()

const emit = defineEmits<{ 'update:modelValue': [v: typeof props.modelValue] }>()

const { t } = useI18n()

const tabs = computed(() => [
  { id: 'dashboard'   as const, icon: LayoutDashboard, label: t('nav.dashboard'),   badge: 0 },
  { id: 'connections' as const, icon: Activity,        label: t('nav.connections'), badge: props.connCount },
  { id: 'profiles'    as const, icon: Layers,          label: t('nav.profiles'),    badge: props.profileCount },
  { id: 'stats'       as const, icon: BarChart3,       label: t('nav.stats'),       badge: 0 },
])
</script>

<template>
  <header class="flex flex-col gap-4">
    <!-- Top row: brand + state + language -->
    <div class="flex items-center justify-between">
      <div class="flex items-center gap-3">
        <div
          class="brand-logo h-9 w-9 rounded-lg shadow-lg shadow-indigo-500/30 ring-1 ring-white/10"
          :style="{ '--brand-mask': `url('${navbarLogoUrl}')` }"
          aria-hidden="true"
        ></div>
        <div>
          <h1 class="text-base font-semibold tracking-tight text-zinc-100">
            {{ t('app.name') }}
          </h1>
          <p class="text-[11px] text-zinc-500 leading-tight">
            {{ t('app.tagline') }}
          </p>
        </div>
      </div>
      <div class="flex items-center gap-2">
        <StatusBadge :state="kernelState" />
        <LanguageSwitcher />
      </div>
    </div>

    <!-- Capsule / segmented nav -->
    <nav
      class="inline-flex items-center gap-0.5 self-start rounded-2xl border border-white/5 bg-white/[0.03] p-1 backdrop-blur-md"
    >
      <button
        v-for="tab in tabs"
        :key="tab.id"
        type="button"
        :class="[
          'inline-flex items-center gap-1.5 rounded-xl px-3.5 py-1.5 text-sm transition-all',
          modelValue === tab.id
            ? 'bg-white/10 text-zinc-100 shadow-sm'
            : 'text-zinc-400 hover:text-zinc-200 hover:bg-white/[0.04]'
        ]"
        @click="emit('update:modelValue', tab.id)"
      >
        <component :is="tab.icon" class="h-3.5 w-3.5" />
        <span>{{ tab.label }}</span>
        <span
          v-if="tab.badge"
          class="ml-1 rounded-md bg-indigo-500/20 px-1.5 py-0.5 text-[10px] font-medium text-indigo-300"
        >
          {{ tab.badge }}
        </span>
      </button>
    </nav>
  </header>
</template>

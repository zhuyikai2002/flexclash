<script setup lang="ts">
/**
 * ThemeToggle.vue — sun / moon switch in the 68px sidebar rail.
 *
 * Sits between LanguageSwitcher and StatusBadge.  Always renders
 * a single circle (the unused icon is rotated 90° + scaled to 0
 * and faded out, so the icon flip is a single 300ms transition
 * without an extra wrapper element).
 *
 * The actual state lives in `useTheme()` — both this button and
 * the eventual "Theme" row in SettingsView read / write the same
 * `theme` ref, so they stay in lockstep.
 */
import { Sun, Moon } from 'lucide-vue-next'
import { useI18n } from '@/composables/useI18n'
import { useTheme } from '@/stores/theme'

const { t } = useI18n()
const { theme, toggleTheme } = useTheme()
</script>

<template>
  <button
    type="button"
    :title="theme === 'dark' ? t('nav.theme_light') : t('nav.theme_dark')"
    :aria-label="theme === 'dark' ? t('nav.theme_light') : t('nav.theme_dark')"
    :aria-pressed="theme === 'light'"
    class="relative flex h-9 w-9 items-center justify-center rounded-xl text-zinc-500 hover:text-zinc-200 hover:bg-white/[0.06] dark:text-zinc-500 dark:hover:text-zinc-200 dark:hover:bg-white/[0.06] light:text-zinc-500 light:hover:text-zinc-700 light:hover:bg-zinc-200/60 transition-colors"
    @click="toggleTheme"
  >
    <!-- Sun (visible in dark mode — clicking goes to light) -->
    <Sun
      :class="[
        'absolute h-4 w-4 transition-all duration-300 ease-out',
        theme === 'dark'
          ? 'rotate-0 scale-100 opacity-100'
          : 'rotate-90 scale-0 opacity-0',
      ]"
    />
    <!-- Moon (visible in light mode) -->
    <Moon
      :class="[
        'absolute h-4 w-4 transition-all duration-300 ease-out',
        theme === 'light'
          ? 'rotate-0 scale-100 opacity-100'
          : '-rotate-90 scale-0 opacity-0',
      ]"
    />
  </button>
</template>

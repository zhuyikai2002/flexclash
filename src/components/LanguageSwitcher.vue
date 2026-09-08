<script setup lang="ts">
/**
 * LanguageSwitcher — compact dropdown for switching between zh-CN and en-US.
 * Persists the choice via i18n.setLocale() which writes localStorage.
 *
 *   compact: icon-only button (no text label) — used inside the 68px sidebar.
 *   default: full pill with Globe + current locale name.
 */
import { computed, onBeforeUnmount, ref } from 'vue'
import { Globe, Check, ChevronDown } from 'lucide-vue-next'
import { useI18n } from '@/composables/useI18n'
import { LOCALE_LABELS, type Locale } from '@/i18n'

const props = withDefaults(
  defineProps<{ compact?: boolean }>(),
  { compact: false },
)
const { t, locale, setLocale } = useI18n()
const open = ref(false)
const root = ref<HTMLElement | null>(null)

const current = computed(() => LOCALE_LABELS[locale.value])
const options: readonly Locale[] = ['zh-CN', 'en-US']

function pick(next: Locale) {
  setLocale(next)
  open.value = false
}

function onDocClick(e: MouseEvent) {
  if (!root.value) return
  if (!root.value.contains(e.target as Node)) open.value = false
}
window.addEventListener('click', onDocClick)
onBeforeUnmount(() => window.removeEventListener('click', onDocClick))

function labelFor(opt: Locale): string {
  return LOCALE_LABELS[opt]
}
</script>

<template>
  <div ref="root" class="relative">
    <button
      v-if="!props.compact"
      type="button"
      :aria-label="t('nav.language')"
      class="inline-flex items-center gap-1.5 rounded-full border border-white/5 bg-white/[0.04] px-3 py-1.5 text-xs text-zinc-300 hover:bg-white/[0.08] hover:border-white/10 transition-colors"
      @click.stop="open = !open"
    >
      <Globe class="h-3.5 w-3.5 text-zinc-400" />
      <span class="font-medium">{{ current }}</span>
      <ChevronDown
        :class="['h-3 w-3 text-zinc-500 transition-transform', open && 'rotate-180']"
      />
    </button>

    <button
      v-else
      type="button"
      :aria-label="t('nav.language')"
      :title="t('nav.language') + ' — ' + current"
      class="flex h-9 w-9 items-center justify-center rounded-xl text-zinc-500 hover:text-zinc-200 hover:bg-white/[0.04] transition-colors"
      @click.stop="open = !open"
    >
      <Globe class="h-4 w-4" />
    </button>

    <Transition
      enter-active-class="transition duration-120 ease-out"
      enter-from-class="opacity-0 -translate-y-1"
      enter-to-class="opacity-100 translate-y-0"
      leave-active-class="transition duration-100 ease-in"
      leave-from-class="opacity-100"
      leave-to-class="opacity-0"
    >
      <div
        v-if="open"
        class="absolute z-30 mt-1.5 w-36 overflow-hidden rounded-xl border border-white/10 bg-zinc-900/95 p-1 shadow-2xl backdrop-blur-xl"
        :class="props.compact ? 'left-full ml-2 top-0' : 'right-0'"
      >
        <button
          v-for="opt in options"
          :key="opt"
          type="button"
          class="flex w-full items-center justify-between rounded-lg px-2.5 py-1.5 text-xs text-zinc-200 hover:bg-white/10 transition-colors"
          :class="locale === opt && 'bg-white/10'"
          @click="pick(opt)"
        >
          <span>{{ labelFor(opt) }}</span>
          <Check v-if="locale === opt" class="h-3.5 w-3.5 text-indigo-400" />
        </button>
      </div>
    </Transition>
  </div>
</template>

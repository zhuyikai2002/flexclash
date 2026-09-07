/**
 * i18n module — vue-i18n@11 wiring.
 *
 * - Default locale: zh-CN (per product spec).
 * - Persisted to localStorage under STORAGE_KEY.
 * - Composition API only (`useI18n()` in components, no Options API).
 * - 8 namespaces loaded synchronously (small JSON files, no lazy load).
 */
import { createI18n } from 'vue-i18n'
import zhCN from './locales/zh-CN.json'
import enUS from './locales/en-US.json'

export const SUPPORTED_LOCALES = ['zh-CN', 'en-US'] as const
export type Locale = (typeof SUPPORTED_LOCALES)[number]

export const LOCALE_LABELS: Record<Locale, string> = {
  'zh-CN': '简体中文',
  'en-US': 'English',
}

export const STORAGE_KEY = 'flexclash.locale'

function detectLocale(): Locale {
  try {
    const stored = localStorage.getItem(STORAGE_KEY)
    if (stored && (SUPPORTED_LOCALES as readonly string[]).includes(stored)) {
      return stored as Locale
    }
  } catch {
    // localStorage unavailable (e.g. SSR); fall through.
  }
  // Browser language hint as last resort.
  if (typeof navigator !== 'undefined' && navigator.language?.startsWith('en')) {
    return 'en-US'
  }
  return 'zh-CN'
}

export const i18n = createI18n({
  legacy: false,
  globalInjection: true,
  locale: detectLocale(),
  fallbackLocale: 'zh-CN',
  messages: {
    'zh-CN': zhCN,
    'en-US': enUS,
  },
  // Pluralization: 中文用 _one / _other 也是 0/1 规则即可，英文需要 _one / _other
  // 简化方案：直接传数字给 message 即可，调用方用 t('time.seconds_ago', { n: 5 })
  missingWarn: false,
  fallbackWarn: false,
})

export function setLocale(locale: Locale): void {
  i18n.global.locale.value = locale
  try {
    localStorage.setItem(STORAGE_KEY, locale)
  } catch {
    // ignore — best effort
  }
  // Notify listeners (e.g. document <html lang=...>)
  if (typeof document !== 'undefined') {
    document.documentElement.lang = locale
  }
}

export function getLocale(): Locale {
  return i18n.global.locale.value as Locale
}

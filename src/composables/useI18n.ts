/**
 * useI18n — thin reactive wrapper around vue-i18n.
 *
 * - Exposes current locale as a `ref` so other Pinia stores / composables
 *   can `watch` it (e.g. tooltips that need to recompute their text).
 * - `setLocale()` is the only mutation entry point; it also writes
 *   localStorage via the i18n module.
 */
import { ref, watch } from 'vue'
import { useI18n as useVueI18n } from 'vue-i18n'
import {
  getLocale,
  setLocale,
  SUPPORTED_LOCALES,
  type Locale,
} from '../i18n'
import { setTrayLanguage } from '@/services/desktop'

const localeRef = ref<Locale>(getLocale())

// Keep the ref in sync with the i18n global (so any other code path
// that changes locale also refreshes the ref).
watch(
  () => getLocale(),
  (v) => {
    localeRef.value = v
  },
)

export function useI18n() {
  const i18n = useVueI18n()
  return {
    ...i18n,
    locale: localeRef,
    /** Frozen list of locales the app currently ships translations
     *  for.  Exposed for UI pickers (e.g. the Settings → Language
     *  button group) so they don't hard-code the locale codes. */
    supportedLocales: SUPPORTED_LOCALES,
    setLocale: (next: Locale) => {
      setLocale(next)
      // The tray menu is built by Rust and cannot read the i18n bundle, so
      // the backend is told which locale is now active. Routed through this
      // composable because it is the only mutation entry point — every switch
      // (settings, sidebar picker) lands here, so none can forget to sync.
      // Fire-and-forget on purpose: a tray left in the previous language is
      // cosmetic, and it must never block the UI switch itself.
      void setTrayLanguage(next)
    },
  }
}

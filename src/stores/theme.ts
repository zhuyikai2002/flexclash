/**
 * theme.ts — Dark / Light theme management.
 *
 *  - Default: 'dark' (per product spec — preserves the existing
 *    "premium black glass" feel for first-run users on Win11).
 *  - Persisted to localStorage under `flexclash.theme`.
 *  - Reflected on `<html>` as `class="dark"` / `class="light"`,
 *    which Tailwind picks up via `darkMode: 'class'`.
 *  - Applies on boot BEFORE the Vue app mounts (see `applyTheme()`)
 *    so users never see a flash of the wrong palette.
 */
import { ref, watch } from 'vue'

export type Theme = 'dark' | 'light'

export const THEME_STORAGE_KEY = 'flexclash.theme'
const DEFAULT_THEME: Theme = 'dark'
const VALID_THEMES: ReadonlyArray<Theme> = ['dark', 'light']

function isTheme(v: unknown): v is Theme {
  return typeof v === 'string' && (VALID_THEMES as readonly string[]).includes(v)
}

function readInitial(): Theme {
  try {
    const stored = localStorage.getItem(THEME_STORAGE_KEY)
    if (isTheme(stored)) return stored
  } catch {
    // localStorage unavailable (e.g. during SSR); fall through.
  }
  return DEFAULT_THEME
}

/** Reactive singleton — both the store and the floating toggle
 *  watch this ref, so every consumer stays in sync. */
export const theme = ref<Theme>(readInitial())

/** Write the current theme onto the `<html>` element and
 *  `localStorage`.  Called on every change AND on first boot. */
function applyTheme(next: Theme) {
  if (typeof document === 'undefined') return
  const root = document.documentElement
  // Order matters: clear both first, then set exactly one so we
  // never leave a stale class behind after a hot reload.
  root.classList.remove('dark', 'light')
  root.classList.add(next)
  root.style.colorScheme = next
  try {
    localStorage.setItem(THEME_STORAGE_KEY, next)
  } catch {
    // best effort
  }
}

applyTheme(theme.value)

watch(theme, (next) => {
  applyTheme(next)
})

export function useTheme() {
  return {
    theme,
    setTheme: (next: Theme) => {
      theme.value = next
    },
    toggleTheme: () => {
      theme.value = theme.value === 'dark' ? 'light' : 'dark'
    },
  }
}

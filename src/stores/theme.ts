/**
 * theme.ts — Dark-only lock (revert from Phase 8 light-mode experiment).
 *
 * The light variant shipped a frosted-white shell, but the inner
 * component tokens (Tailwind `text-zinc-100` etc.) were never
 * converted to `light:` variants across the codebase, so users saw
 * near-invisible text. Until every component is re-themed end to
 * end, we hard-lock the app to `class="dark"` and never let the
 * user toggle. The single source of truth is the inline script in
 * `index.html`; this file just exposes the read-only `theme` ref
 * so any future code that needs to branch on it (e.g. an about
 * page) can read the same value the DOM is using.
 */
import { ref } from 'vue'

export type Theme = 'dark' | 'light'

/** Hard-locked constant — the only theme the app currently ships. */
export const ACTIVE_THEME: Theme = 'dark'

/** Reactive singleton kept in sync for any code that wants to read it. */
export const theme = ref<Theme>(ACTIVE_THEME)

export const THEME_STORAGE_KEY = 'flexclash.theme'

/** Always returns the locked dark theme.  Kept as a function so call
 *  sites that read it from a watcher don't need to change. */
export function useTheme() {
  return {
    theme,
    setTheme: (_next: Theme) => {
      // No-op: light mode is intentionally disabled until the rest
      // of the component tree ships proper `light:` token variants.
    },
    toggleTheme: () => {
      // No-op: see above.
    },
  }
}

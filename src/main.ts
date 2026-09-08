import { createApp } from 'vue'
import { createPinia } from 'pinia'
import { i18n } from './i18n'
import './style.css'
import App from './App.vue'

// ---------------------------------------------------------------------------
// Desktop keyboard hardening (Phase 9.9)
// ---------------------------------------------------------------------------
// Block browser / web-debugging shortcuts so the packaged app behaves like a
// native desktop product and never exposes DevTools, view-source, reload or
// zoom.  Regular text-editing keys (Backspace, Enter, Ctrl+C/V/A, etc.) are
// deliberately NOT intercepted, so the subscribe/rename/URL inputs keep
// working normally.
// ---------------------------------------------------------------------------
window.addEventListener(
  'keydown',
  (e: KeyboardEvent) => {
    // F12 — DevTools
    if (e.key === 'F12') {
      e.preventDefault()
      return
    }

    // F5 — reload
    if (e.key === 'F5') {
      e.preventDefault()
      return
    }

    if (e.ctrlKey || e.metaKey) {
      // Ctrl+R reload / Ctrl+U view-source / Ctrl+P print
      if (['r', 'R', 'u', 'U', 'p', 'P'].includes(e.key)) {
        e.preventDefault()
        return
      }

      // Ctrl+Shift+I / J / C — DevTools / console / inspect
      if (e.shiftKey && ['i', 'I', 'j', 'J', 'c', 'C'].includes(e.key)) {
        e.preventDefault()
        return
      }

      // Ctrl + +/-/0 (and Shift variants) — zoom / reset zoom
      if (['+', '-', '=', '_', '0', ')'].includes(e.key)) {
        e.preventDefault()
        return
      }
    }
  },
  { capture: true },
)

const app = createApp(App)
app.use(createPinia())
app.use(i18n)
app.mount('#app')

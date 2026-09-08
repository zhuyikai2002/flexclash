import { createApp } from 'vue'
import { createPinia } from 'pinia'
import { i18n } from './i18n'
// IMPORTANT: import theme.ts so it runs its top-level
// `applyTheme(theme.value)` BEFORE the Vue app mounts.  Without
// this the first frame would briefly show the wrong palette.
import './stores/theme'
import './style.css'
import App from './App.vue'

const app = createApp(App)
app.use(createPinia())
app.use(i18n)
app.mount('#app')

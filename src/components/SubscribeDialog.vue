<script setup lang="ts">
/**
 * SubscribeDialog — Add a profile (URL / file / paste YAML).
 *
 * 2025 enhancement: CLIPBOARD INTELLIGENCE.
 *  When the dialog opens we read `navigator.clipboard.readText()` and
 *  branch on the content shape:
 *    - matches `^https?://…`        → switch to the URL tab and pre-fill
 *    - looks like a Clash YAML
 *      (contains `proxies:` /
 *      `port:` / `mixed-port:` /
 *      `rules:`)                    → switch to the paste tab and
 *                                     pre-fill the textarea
 *    - anything else                 → stay on the URL tab (default)
 *
 *  Browser security note: `navigator.clipboard.readText()` requires
 *  either focus on the document OR the user to have explicitly granted
 *  permission.  Tauri WebView2 grants read-access to the active web
 *  page in the same way Chromium does — it works.  We still wrap the
 *  call in try/catch so a denied permission does not break the dialog.
 */
import { ref, watch } from 'vue'
import { X, Link, FileCode2, ClipboardPaste, Loader2 } from 'lucide-vue-next'
import { open } from '@tauri-apps/plugin-dialog'
import { readText } from '@tauri-apps/plugin-clipboard-manager'
import { useProfilesStore } from '@/stores/profiles'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ (e: 'close'): void }>()

const store = useProfilesStore()
const { t } = useI18n()

type Tab = 'url' | 'file' | 'paste'
const tab = ref<Tab>('url')

const url = ref('')
const name = ref('')
const filePath = ref('')
const pastedName = ref('')
const pastedYaml = ref('')

const busy = ref(false)
const error = ref<string | null>(null)
const hint = ref<string | null>(null) // clipboard auto-detect message

function reset() {
  url.value = ''; name.value = ''; filePath.value = ''
  pastedName.value = ''; pastedYaml.value = ''
  error.value = null; busy.value = false; hint.value = null
}
function close() { reset(); emit('close') }

// ============================================================================
// Clipboard auto-detect
// ============================================================================
// We use `tauri-plugin-clipboard-manager`'s `readText()` instead of
// `navigator.clipboard.readText()`.  The browser API requires a
// Chromium-level permission grant (which pops the "allow clipboard"
// dialog) and is blocked in some WebView2 sandboxes.  The Tauri
// plugin calls `arboard` / Win32 `GetClipboardData` directly and
// needs no JS-side prompt at all.
//
// Failure is silent: an empty clipboard, a non-text format, or a
// transient OS error all return `null` and we fall through to the
// URL tab without any intrusive UI.
// ============================================================================
async function readClipboard(): Promise<string | null> {
  try {
    return await readText()
  } catch {
    return null
  }
}

function isUrl(s: string): boolean {
  return /^https?:\/\/\S+$/i.test(s.trim())
}

function isClashYaml(s: string): boolean {
  // Heuristic: real Clash configs always include at least one of the
  // canonical top-level keys.  We accept `proxies:`, `proxy-groups:`,
  // `rules:`, `mixed-port:`, `port:`, `tun:` or `dns:` — any of these
  // inside the first 4 KB is enough to call it a config snippet.
  const head = s.slice(0, 4096)
  return /(^|\n)\s*(proxies|proxy-groups|rules|mixed-port|port|tun|dns)\s*[:{]/m.test(head)
}

async function probeClipboard() {
  hint.value = null
  const text = await readClipboard()
  if (!text) return
  const trimmed = text.trim()
  if (isUrl(trimmed)) {
    tab.value = 'url'
    if (!url.value) url.value = trimmed
    hint.value = t('profiles.clipboard_detected_url')
  } else if (isClashYaml(trimmed)) {
    tab.value = 'paste'
    if (!pastedYaml.value) pastedYaml.value = text
    hint.value = t('profiles.clipboard_detected_yaml')
  }
}

// Run the probe every time the dialog opens. We `watch(props.open)`
// rather than `onMounted` so re-opens re-read fresh clipboard content.
watch(
  () => props.open,
  (open) => { if (open) void probeClipboard() },
  { immediate: true },
)

async function pickFile() {
  try {
    const sel = await open({
      multiple: false, directory: false,
      filters: [
        { name: 'YAML / text', extensions: ['yaml', 'yml', 'txt', 'conf'] },
        { name: 'All files',  extensions: ['*'] },
      ],
    })
    if (typeof sel === 'string') filePath.value = sel
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  }
}

async function submit() {
  busy.value = true; error.value = null
  try {
    if (tab.value === 'url') {
      if (!url.value.trim()) throw new Error('URL is required')
      await store.addFromUrl(url.value.trim(), name.value.trim() || undefined)
    } else if (tab.value === 'file') {
      if (!filePath.value.trim()) throw new Error('File path is required')
      await store.addFromFile(filePath.value.trim(), name.value.trim() || undefined)
    } else {
      if (!pastedYaml.value.trim()) throw new Error('YAML is empty')
      await store.pasteYaml(
        pastedName.value.trim() || 'Pasted profile',
        pastedYaml.value,
      )
    }
    close()
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = false
  }
}

const tabs = [
  { id: 'url'   as const, label: t('profiles.add_url'),  icon: Link },
  { id: 'file'  as const, label: t('profiles.add_file'), icon: FileCode2 },
  { id: 'paste' as const, label: 'Paste YAML',           icon: ClipboardPaste },
]
</script>

<template>
  <Teleport to="body">
    <div
      v-if="props.open"
      class="fixed inset-0 z-40 flex items-center justify-center bg-black/60 backdrop-blur-sm p-4"
      @click.self="close"
    >
      <div class="w-full max-w-xl rounded-2xl border border-white/10 bg-zinc-900/95 backdrop-blur-xl p-5 shadow-2xl">
        <header class="mb-4 flex items-center justify-between">
          <h2 class="text-base font-semibold tracking-tight text-zinc-100">
            {{ t('common.import') }} profile
          </h2>
          <button
            class="rounded-lg p-1 text-zinc-400 hover:bg-white/5 hover:text-zinc-200 transition-colors"
            @click="close"
          >
            <X class="h-4 w-4" />
          </button>
        </header>

        <nav class="mb-4 inline-flex items-center gap-0.5 rounded-xl border border-white/5 bg-white/[0.04] p-1 text-xs w-full">
          <button
            v-for="t in tabs"
            :key="t.id"
            class="flex flex-1 items-center justify-center gap-1.5 rounded-lg px-2 py-1.5 transition-colors"
            :class="tab === t.id
              ? 'bg-white/10 text-zinc-100'
              : 'text-zinc-400 hover:text-zinc-200 hover:bg-white/[0.04]'"
            @click="tab = t.id"
          >
            <component :is="t.icon" class="h-3.5 w-3.5" />
            {{ t.label }}
          </button>
        </nav>

        <!-- Auto-detect feedback (only visible when we found something
             in the clipboard that we acted on). -->
        <p
          v-if="hint"
          class="mb-3 rounded-lg border border-sky-500/20 bg-sky-500/10 px-3 py-1.5 text-[11px] text-sky-200"
        >
          {{ hint }}
        </p>

        <form class="space-y-3" @submit.prevent="submit">
          <div v-if="tab === 'url'" class="space-y-2">
            <label class="block">
              <span class="text-xs text-zinc-400">{{ t('profiles.subscription.url') }}</span>
              <input
                v-model="url"
                type="url"
                :placeholder="t('profiles.subscription.url_placeholder')"
                class="mt-1 block w-full rounded-lg border border-white/5 bg-white/[0.04] px-2.5 py-1.5 text-sm text-zinc-100 placeholder:text-zinc-500 focus:border-indigo-400/30 focus:outline-none focus:ring-1 focus:ring-indigo-500/50 transition-colors"
                required
              />
            </label>
            <label class="block">
              <span class="text-xs text-zinc-400">Display name (optional)</span>
              <input
                v-model="name"
                type="text"
                placeholder="My provider"
                class="mt-1 block w-full rounded-lg border border-white/5 bg-white/[0.04] px-2.5 py-1.5 text-sm text-zinc-100 placeholder:text-zinc-500 focus:border-indigo-400/30 focus:outline-none focus:ring-1 focus:ring-indigo-500/50 transition-colors"
              />
            </label>
            <p class="text-[11px] text-zinc-500">
              User-Agent: <code class="text-zinc-300 font-mono">mihomo/1.19.30</code>
            </p>
          </div>

          <div v-else-if="tab === 'file'" class="space-y-2">
            <label class="block">
              <span class="text-xs text-zinc-400">File path</span>
              <div class="mt-1 flex gap-2">
                <input
                  v-model="filePath"
                  type="text"
                  placeholder="C:\path\to\config.yaml"
                  class="block w-full rounded-lg border border-white/5 bg-white/[0.04] px-2.5 py-1.5 text-sm text-zinc-100 placeholder:text-zinc-500 focus:border-indigo-400/30 focus:outline-none focus:ring-1 focus:ring-indigo-500/50 transition-colors"
                />
                <button
                  type="button"
                  class="shrink-0 rounded-lg border border-white/5 bg-white/[0.04] px-3 py-1.5 text-xs text-zinc-200 hover:bg-white/[0.08] hover:border-white/10 transition-colors"
                  @click="pickFile"
                >
                  Browse…
                </button>
              </div>
            </label>
            <label class="block">
              <span class="text-xs text-zinc-400">Display name (optional)</span>
              <input
                v-model="name"
                type="text"
                placeholder="My profile"
                class="mt-1 block w-full rounded-lg border border-white/5 bg-white/[0.04] px-2.5 py-1.5 text-sm text-zinc-100 placeholder:text-zinc-500 focus:border-indigo-400/30 focus:outline-none focus:ring-1 focus:ring-indigo-500/50 transition-colors"
              />
            </label>
          </div>

          <div v-else class="space-y-2">
            <label class="block">
              <span class="text-xs text-zinc-400">Display name</span>
              <input
                v-model="pastedName"
                type="text"
                placeholder="Pasted profile"
                class="mt-1 block w-full rounded-lg border border-white/5 bg-white/[0.04] px-2.5 py-1.5 text-sm text-zinc-100 placeholder:text-zinc-500 focus:border-indigo-400/30 focus:outline-none focus:ring-1 focus:ring-indigo-500/50 transition-colors"
              />
            </label>
            <label class="block">
              <span class="text-xs text-zinc-400">YAML</span>
              <textarea
                v-model="pastedYaml"
                rows="10"
                placeholder="mixed-port: 7890&#10;proxies:&#10;  - { name: 'ss1', type: ss, server: 1.2.3.4, port: 8388 }"
                class="mt-1 block w-full rounded-lg border border-white/5 bg-white/[0.04] px-2.5 py-1.5 font-mono text-xs text-zinc-100 placeholder:text-zinc-500 focus:border-indigo-400/30 focus:outline-none focus:ring-1 focus:ring-indigo-500/50 transition-colors"
              />
            </label>
          </div>

          <p v-if="error" class="rounded-lg border border-rose-500/20 bg-rose-500/10 px-3 py-2 text-xs text-rose-300">
            {{ error }}
          </p>

          <footer class="flex items-center justify-end gap-2 pt-2">
            <button
              type="button"
              class="rounded-lg border border-white/5 bg-white/[0.04] px-3 py-1.5 text-xs text-zinc-300 hover:bg-white/[0.08] hover:border-white/10 transition-colors"
              @click="close"
            >
              {{ t('common.cancel') }}
            </button>
            <button
              type="submit"
              class="inline-flex items-center gap-1.5 rounded-lg bg-indigo-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-indigo-400 disabled:opacity-60 transition-colors"
              :disabled="busy"
            >
              <Loader2 v-if="busy" class="h-3.5 w-3.5 animate-spin" />
              {{ busy ? t('common.loading') : t('common.import') }}
            </button>
          </footer>
        </form>
      </div>
    </div>
  </Teleport>
</template>

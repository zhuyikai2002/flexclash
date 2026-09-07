<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { X, Link, FileCode2, ClipboardPaste, Loader2 } from 'lucide-vue-next'
import { open } from '@tauri-apps/plugin-dialog'
import { useProfilesStore } from '@/stores/profiles'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ (e: 'close'): void }>()

const store = useProfilesStore()

type Tab = 'url' | 'file' | 'paste'
const tab = ref<Tab>('url')

const url = ref('')
const name = ref('')
const filePath = ref('')
const pastedName = ref('')
const pastedYaml = ref('')

const busy = ref(false)
const error = ref<string | null>(null)

onMounted(() => { /* nothing to load */ })

function reset() {
  url.value = ''
  name.value = ''
  filePath.value = ''
  pastedName.value = ''
  pastedYaml.value = ''
  error.value = null
  busy.value = false
}

function close() {
  reset()
  emit('close')
}

async function pickFile() {
  try {
    const sel = await open({
      multiple: false,
      directory: false,
      filters: [
        { name: 'YAML / text', extensions: ['yaml', 'yml', 'txt', 'conf'] },
        { name: 'All files', extensions: ['*'] },
      ],
    })
    if (typeof sel === 'string') filePath.value = sel
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  }
}

async function submit() {
  busy.value = true
  error.value = null
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
</script>

<template>
  <Teleport to="body">
    <div
      v-if="props.open"
      class="fixed inset-0 z-40 flex items-center justify-center bg-slate-950/70 p-4"
      @click.self="close"
    >
      <div class="w-full max-w-xl rounded-lg border border-slate-700 bg-slate-900 p-5 shadow-xl">
        <header class="mb-4 flex items-center justify-between">
          <h2 class="text-lg font-semibold text-slate-100">Add a profile</h2>
          <button
            class="rounded p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-200"
            @click="close"
          >
            <X class="h-4 w-4" />
          </button>
        </header>

        <nav class="mb-4 flex gap-1 rounded-md bg-slate-800/60 p-1 text-xs">
          <button
            v-for="t in [
              { id: 'url',   label: 'Subscription URL', icon: Link },
              { id: 'file',  label: 'From file',        icon: FileCode2 },
              { id: 'paste', label: 'Paste YAML',       icon: ClipboardPaste },
            ] as const"
            :key="t.id"
            class="flex flex-1 items-center justify-center gap-1.5 rounded px-2 py-1.5 transition"
            :class="tab === t.id
              ? 'bg-sky-600 text-white'
              : 'text-slate-300 hover:bg-slate-700/60'"
            @click="tab = t.id"
          >
            <component :is="t.icon" class="h-3.5 w-3.5" />
            {{ t.label }}
          </button>
        </nav>

        <form class="space-y-3" @submit.prevent="submit">
          <div v-if="tab === 'url'" class="space-y-2">
            <label class="block">
              <span class="text-xs text-slate-400">Subscription URL</span>
              <input
                v-model="url"
                type="url"
                placeholder="https://example.com/link?clash=1"
                class="mt-1 block w-full rounded-md border border-slate-700 bg-slate-800 px-2.5 py-1.5 text-sm text-slate-100 placeholder:text-slate-500 focus:border-sky-500 focus:outline-none"
                required
              />
            </label>
            <label class="block">
              <span class="text-xs text-slate-400">Display name (optional)</span>
              <input
                v-model="name"
                type="text"
                placeholder="My provider"
                class="mt-1 block w-full rounded-md border border-slate-700 bg-slate-800 px-2.5 py-1.5 text-sm text-slate-100 placeholder:text-slate-500 focus:border-sky-500 focus:outline-none"
              />
            </label>
            <p class="text-[11px] text-slate-500">
              User-Agent is set to <code class="text-slate-300">mihomo/1.19.30</code> automatically.
            </p>
          </div>

          <div v-else-if="tab === 'file'" class="space-y-2">
            <label class="block">
              <span class="text-xs text-slate-400">File path</span>
              <div class="mt-1 flex gap-2">
                <input
                  v-model="filePath"
                  type="text"
                  placeholder="C:\path\to\config.yaml"
                  class="block w-full rounded-md border border-slate-700 bg-slate-800 px-2.5 py-1.5 text-sm text-slate-100 placeholder:text-slate-500 focus:border-sky-500 focus:outline-none"
                />
                <button
                  type="button"
                  class="shrink-0 rounded-md border border-slate-700 bg-slate-800 px-3 py-1.5 text-xs text-slate-200 hover:bg-slate-700"
                  @click="pickFile"
                >
                  Browse…
                </button>
              </div>
            </label>
            <label class="block">
              <span class="text-xs text-slate-400">Display name (optional)</span>
              <input
                v-model="name"
                type="text"
                placeholder="My profile"
                class="mt-1 block w-full rounded-md border border-slate-700 bg-slate-800 px-2.5 py-1.5 text-sm text-slate-100 placeholder:text-slate-500 focus:border-sky-500 focus:outline-none"
              />
            </label>
          </div>

          <div v-else class="space-y-2">
            <label class="block">
              <span class="text-xs text-slate-400">Display name</span>
              <input
                v-model="pastedName"
                type="text"
                placeholder="Pasted profile"
                class="mt-1 block w-full rounded-md border border-slate-700 bg-slate-800 px-2.5 py-1.5 text-sm text-slate-100 placeholder:text-slate-500 focus:border-sky-500 focus:outline-none"
              />
            </label>
            <label class="block">
              <span class="text-xs text-slate-400">YAML</span>
              <textarea
                v-model="pastedYaml"
                rows="10"
                placeholder="mixed-port: 7890&#10;proxies:&#10;  - { name: 'ss1', type: ss, server: 1.2.3.4, port: 8388 }"
                class="mt-1 block w-full rounded-md border border-slate-700 bg-slate-800 px-2.5 py-1.5 font-mono text-xs text-slate-100 placeholder:text-slate-500 focus:border-sky-500 focus:outline-none"
              />
            </label>
          </div>

          <p v-if="error" class="rounded-md border border-rose-700/50 bg-rose-900/20 px-3 py-2 text-xs text-rose-200">
            {{ error }}
          </p>

          <footer class="flex items-center justify-end gap-2 pt-2">
            <button
              type="button"
              class="rounded-md border border-slate-700 px-3 py-1.5 text-xs text-slate-300 hover:bg-slate-800"
              @click="close"
            >
              Cancel
            </button>
            <button
              type="submit"
              class="inline-flex items-center gap-1.5 rounded-md bg-sky-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-sky-500 disabled:opacity-60"
              :disabled="busy"
            >
              <Loader2 v-if="busy" class="h-3.5 w-3.5 animate-spin" />
              {{ busy ? 'Importing…' : 'Import' }}
            </button>
          </footer>
        </form>
      </div>
    </div>
  </Teleport>
</template>

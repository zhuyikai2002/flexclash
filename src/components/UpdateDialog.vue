<script setup lang="ts">
// ============================================================================
// UpdateDialog.vue — cold-start "update available" prompt (v0.5.x UX).
//
// Driven entirely by the updater store, so it renders one of three states
// with zero local orchestration:
//   available   -> changelog (update.body, Markdown) + [下次再说] [立即更新]
//   downloading -> live progress bar (store.progress)
//   ready/installing -> "installing and restarting…" spinner
//   error       -> the failure message + a close affordance
//
// "Update Now" awaits download() then install(); on Windows install() exits
// the app, so the spinner is usually the last thing the user sees.
// ============================================================================

import { computed, nextTick, onBeforeUnmount, watch } from 'vue'
import { Download, Loader2, Sparkles, X } from 'lucide-vue-next'
import DOMPurify from 'dompurify'
import { marked } from 'marked'
import { useI18n } from '@/composables/useI18n'
import { useUpdaterStore } from '@/stores/updater'

const { t } = useI18n()
const updater = useUpdaterStore()

const open = computed(() => updater.promptOpen)
const busy = computed(() => updater.isBusy)

/** Render the release notes as sanitised HTML. `update.body` is the
 *  `latest.json > notes` field — Markdown we author in release_notes.md — so
 *  it is converted with marked and then sanitised against the (unsigned,
 *  transport-visible) field. */
function renderBody(md: string | null | undefined): string {
  if (!md) return ''
  const raw = marked.parse(md, { async: false, gfm: true, breaks: false }) as string
  return DOMPurify.sanitize(raw)
}
const bodyHtml = computed(() => renderBody(updater.info?.body))
const currentLabel = computed(() =>
  updater.info?.currentVersion ? `v${updater.info.currentVersion}` : '—',
)

function close() {
  if (busy.value) return
  updater.dismissPrompt()
}

async function onUpdateNow() {
  try {
    await updater.download()
    await updater.install()
  } catch {
    // store.error is already set; the dialog switches to the error view.
  }
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape' && open.value && !busy.value) {
    e.stopPropagation()
    close()
  }
}

watch(
  open,
  async (v) => {
    if (v) {
      document.body.style.overflow = 'hidden'
      await nextTick()
    } else {
      document.body.style.overflow = ''
    }
  },
)

onBeforeUnmount(() => {
  document.body.style.overflow = ''
})
</script>

<template>
  <Teleport to="body">
    <Transition
      enter-active-class="transition duration-150"
      enter-from-class="opacity-0"
      enter-to-class="opacity-100"
      leave-active-class="transition duration-100"
      leave-from-class="opacity-100"
      leave-to-class="opacity-0"
    >
      <div
        v-if="open"
        class="fixed inset-0 z-[100] flex items-center justify-center p-4"
        @keydown="onKeydown"
      >
        <!-- Backdrop -->
        <div
          class="glass-scrim absolute inset-0"
          aria-hidden="true"
          @click="close"
        />

        <!-- Dialog -->
        <div
          role="dialog"
          aria-modal="true"
          aria-label="update available"
          class="glass-popover relative w-full max-w-lg rounded-2xl p-5 space-y-4"
          @click.stop
        >
          <!-- Header -->
          <header class="flex items-start gap-3">
            <div
              class="w-9 h-9 rounded-lg bg-sky-500/15 ring-1 ring-sky-400/30 flex items-center justify-center shrink-0"
            >
              <Sparkles class="w-4 h-4 text-sky-300" />
            </div>
            <div class="flex-1 min-w-0">
              <h2 class="text-sm font-semibold text-zinc-100">
                {{ t('updater.new_version', { version: updater.versionLabel }) }}
              </h2>
              <p class="text-[11px] text-zinc-500 mt-0.5">
                {{ t('updater.current_version', { version: currentLabel }) }}
              </p>
            </div>
            <button
              type="button"
              :disabled="busy"
              @click="close"
              class="text-zinc-500 hover:text-zinc-300 disabled:opacity-50 transition-colors"
              aria-label="Close"
            >
              <X class="w-4 h-4" />
            </button>
          </header>

          <!-- Changelog -->
          <div
            v-if="updater.phase === 'available'"
            class="update-body max-h-[45vh] overflow-y-auto pr-1 text-[12px] text-zinc-300 leading-relaxed"
            v-html="bodyHtml"
          />

          <!-- Progress -->
          <div v-else-if="updater.phase === 'downloading'" class="space-y-3 py-1">
            <div class="flex items-center justify-between text-[11px] text-zinc-400">
              <span class="inline-flex items-center gap-2">
                <Loader2 class="w-3.5 h-3.5 animate-spin text-sky-300" />
                {{ t('updater.downloading') }}
              </span>
              <span class="tabular-nums text-zinc-200">{{ updater.progress }}%</span>
            </div>
            <div class="h-2 rounded-full bg-white/10 overflow-hidden">
              <div
                class="h-full bg-sky-500 transition-all duration-200 ease-out"
                :style="{ width: `${updater.progress}%` }"
              />
            </div>
          </div>

          <!-- Installing / restarting -->
          <div
            v-else-if="updater.phase === 'ready' || updater.phase === 'installing'"
            class="flex items-center gap-2 text-[12px] text-zinc-300 py-1"
          >
            <Loader2 class="w-4 h-4 animate-spin text-sky-300" />
            {{ t('updater.installing') }}
          </div>

          <!-- Error -->
          <div v-else-if="updater.phase === 'error'" class="text-[12px] text-rose-300 leading-relaxed">
            {{ updater.error }}
          </div>

          <!-- Footer -->
          <footer
            v-if="updater.phase === 'available'"
            class="flex items-center justify-end gap-2 pt-1"
          >
            <button
              type="button"
              @click="close"
              class="rounded-lg border border-white/10 bg-white/[0.04] hover:bg-white/[0.08] text-zinc-100 px-3 py-1.5 text-sm font-medium transition-colors"
            >
              {{ t('updater.later') }}
            </button>
            <button
              type="button"
              @click="onUpdateNow"
              class="inline-flex items-center gap-2 rounded-lg px-3 py-1.5 text-sm font-medium bg-sky-500 hover:bg-sky-400 text-white shadow-sm shadow-sky-500/30 transition-colors"
            >
              <Download class="w-3.5 h-3.5" />
              {{ t('updater.update_now') }}
            </button>
          </footer>
          <footer v-else-if="updater.phase === 'error'" class="flex justify-end pt-1">
            <button
              type="button"
              @click="close"
              class="rounded-lg border border-white/10 bg-white/[0.04] hover:bg-white/[0.08] text-zinc-100 px-3 py-1.5 text-sm font-medium transition-colors"
            >
              {{ t('updater.close') }}
            </button>
          </footer>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
/* Minimal, dependency-free styling for the sanitised Markdown in update.body.
   :deep() reaches the v-html children, which carry no scoped data-attribute. */
.update-body :deep(h1),
.update-body :deep(h2),
.update-body :deep(h3),
.update-body :deep(h4) {
  color: rgb(244 244 245); /* zinc-100 */
  font-weight: 600;
  margin: 0.75em 0 0.4em;
  line-height: 1.3;
}
.update-body :deep(h1) { font-size: 1.1rem; }
.update-body :deep(h2) { font-size: 1rem; }
.update-body :deep(h3) { font-size: 0.9rem; }
.update-body :deep(h4) { font-size: 0.85rem; }
.update-body :deep(h1:first-child),
.update-body :deep(h2:first-child),
.update-body :deep(h3:first-child) {
  margin-top: 0;
}
.update-body :deep(p) {
  margin: 0.4em 0;
}
.update-body :deep(ul),
.update-body :deep(ol) {
  padding-left: 1.25rem;
  margin: 0.4em 0;
}
.update-body :deep(li) {
  margin: 0.2em 0;
}
.update-body :deep(ul > li) {
  list-style: disc;
}
.update-body :deep(ol > li) {
  list-style: decimal;
}
.update-body :deep(code) {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 0.85em;
  background: rgb(255 255 255 / 0.08);
  border-radius: 0.25rem;
  padding: 0.1em 0.35em;
}
.update-body :deep(pre) {
  background: rgb(0 0 0 / 0.3);
  border: 1px solid rgb(255 255 255 / 0.1);
  border-radius: 0.5rem;
  padding: 0.6rem 0.75rem;
  overflow-x: auto;
  margin: 0.5em 0;
}
.update-body :deep(pre code) {
  background: transparent;
  padding: 0;
}
.update-body :deep(table) {
  border-collapse: collapse;
  width: 100%;
  margin: 0.5em 0;
  font-size: 0.85em;
}
.update-body :deep(th),
.update-body :deep(td) {
  border: 1px solid rgb(255 255 255 / 0.12);
  padding: 0.35em 0.6em;
  text-align: left;
}
.update-body :deep(th) {
  background: rgb(255 255 255 / 0.05);
  font-weight: 600;
  color: rgb(228 228 231);
}
.update-body :deep(a) {
  color: rgb(125 211 252); /* sky-300 */
  text-decoration: underline;
}
.update-body :deep(strong) {
  color: rgb(228 228 231);
  font-weight: 600;
}
</style>

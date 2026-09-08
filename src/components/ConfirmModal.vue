<script setup lang="ts">
/**
 * ConfirmModal.vue — generic dangerous-action confirm dialog.
 *
 * Phase 8 use case: "Reset Application" requires the user to type
 * the literal `RESET` before the confirm button is enabled.  This
 * component handles the boilerplate (overlay, focus trap, escape key,
 * body-scroll lock) so the caller only specifies the danger label
 * and the action callback.
 *
 * Usage:
 *   <ConfirmModal
 *     v-model:open="show"
 *     :title="t('settings.danger_zone.confirm_title')"
 *     :body="t('settings.danger_zone.confirm_body')"
 *     :require-text="'RESET'"
 *     :confirm-label="t('settings.danger_zone.confirm_action')"
 *     :busy="resetting"
 *     @confirm="onReset"
 *   />
 */
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { AlertTriangle, X, Loader2 } from 'lucide-vue-next'

const props = withDefaults(defineProps<{
  /** v-model:open */
  open: boolean
  title: string
  /** Multi-line body text.  \n is honoured. */
  body: string
  /** If non-empty, user must type this exact string into the input
   *  before the confirm button is enabled.  Case-sensitive. */
  requireText?: string
  confirmLabel: string
  cancelLabel?: string
  /** Disables the confirm button and shows a spinner. */
  busy?: boolean
  /** Visual emphasis for the confirm button.  Defaults to `danger`. */
  tone?: 'danger' | 'warning' | 'primary'
}>(), {
  requireText: '',
  busy: false,
  tone: 'danger',
  cancelLabel: 'Cancel',
})

const emit = defineEmits<{
  'update:open': [v: boolean]
  confirm: []
  cancel: []
}>()

const typed = ref('')
const inputEl = ref<HTMLInputElement | null>(null)

const canConfirm = computed(() => {
  if (props.busy) return false
  if (!props.requireText) return true
  return typed.value === props.requireText
})

const toneClasses = computed(() => {
  switch (props.tone) {
    case 'danger':
      return 'bg-rose-500 hover:bg-rose-400 disabled:bg-rose-900 disabled:text-rose-400 text-white shadow-rose-500/30'
    case 'warning':
      return 'bg-amber-500 hover:bg-amber-400 disabled:bg-amber-900 disabled:text-amber-400 text-zinc-900 shadow-amber-500/30'
    case 'primary':
    default:
      return 'bg-sky-500 hover:bg-sky-400 disabled:bg-sky-900 disabled:text-sky-400 text-white shadow-sky-500/30'
  }
})

function close() {
  if (props.busy) return
  emit('cancel')
  emit('update:open', false)
}

function onConfirm() {
  if (!canConfirm.value) return
  emit('confirm')
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape' && props.open && !props.busy) {
    e.stopPropagation()
    close()
  }
  if (e.key === 'Enter' && props.open && canConfirm.value) {
    e.preventDefault()
    onConfirm()
  }
}

// Focus the input when the dialog opens (especially the confirm-typing one).
watch(() => props.open, async (open) => {
  if (open) {
    typed.value = ''
    // Body scroll lock
    document.body.style.overflow = 'hidden'
    await nextTick()
    inputEl.value?.focus()
  } else {
    document.body.style.overflow = ''
  }
})

onBeforeUnmount(() => {
  document.body.style.overflow = ''
})

defineExpose({ close })
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
          class="absolute inset-0 bg-zinc-950/70 backdrop-blur-sm"
          aria-hidden="true"
          @click="close"
        />

        <!-- Dialog -->
        <div
          role="dialog"
          aria-modal="true"
          :aria-labelledby="'confirm-title'"
          class="relative w-full max-w-md rounded-2xl border border-white/10 bg-zinc-900/95 backdrop-blur-xl shadow-2xl shadow-black/50 p-5 space-y-4"
          @click.stop
        >
          <!-- Header -->
          <header class="flex items-start gap-3">
            <div
              :class="[
                'w-9 h-9 rounded-lg flex items-center justify-center shrink-0',
                tone === 'danger' ? 'bg-rose-500/15 ring-1 ring-rose-400/30' :
                tone === 'warning' ? 'bg-amber-500/15 ring-1 ring-amber-400/30' :
                'bg-sky-500/15 ring-1 ring-sky-400/30'
              ]"
            >
              <AlertTriangle
                :class="[
                  'w-4 h-4',
                  tone === 'danger' ? 'text-rose-300' :
                  tone === 'warning' ? 'text-amber-300' :
                  'text-sky-300'
                ]"
              />
            </div>
            <div class="flex-1 min-w-0">
              <h2 id="confirm-title" class="text-sm font-semibold text-zinc-100">
                {{ title }}
              </h2>
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

          <!-- Body -->
          <p class="text-[12px] text-zinc-400 leading-relaxed whitespace-pre-line pl-12">
            {{ body }}
          </p>

          <!-- Type-to-confirm input (optional) -->
          <div v-if="requireText" class="pl-12 space-y-1.5">
            <label class="block text-[10px] uppercase tracking-wider text-zinc-500 font-semibold">
              {{ requireText === 'RESET' ? 'Type RESET to confirm' : `Type "${requireText}" to confirm` }}
            </label>
            <input
              ref="inputEl"
              v-model="typed"
              type="text"
              :placeholder="requireText"
              :disabled="busy"
              autocomplete="off"
              spellcheck="false"
              class="w-full rounded-lg border border-white/10 bg-white/[0.04] focus:border-rose-400/50 focus:ring-1 focus:ring-rose-400/30 outline-none px-3 py-2 text-sm font-mono text-zinc-100 placeholder:text-zinc-600 transition-colors"
              @keydown.enter.prevent="onConfirm"
            />
          </div>

          <!-- Footer -->
          <footer class="flex items-center justify-end gap-2 pt-1">
            <button
              type="button"
              :disabled="busy"
              @click="close"
              class="rounded-lg border border-white/10 bg-white/[0.04] hover:bg-white/[0.08] disabled:opacity-50 text-zinc-100 px-3 py-1.5 text-sm font-medium transition-colors"
            >
              {{ cancelLabel }}
            </button>
            <button
              type="button"
              :disabled="!canConfirm"
              @click="onConfirm"
              :class="[
                'inline-flex items-center gap-2 rounded-lg px-3 py-1.5 text-sm font-medium transition-colors shadow-sm disabled:cursor-not-allowed',
                toneClasses
              ]"
            >
              <Loader2 v-if="busy" class="w-3.5 h-3.5 animate-spin" />
              {{ confirmLabel }}
            </button>
          </footer>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

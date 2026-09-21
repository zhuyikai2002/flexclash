<script setup lang="ts">
// ============================================================================
// ToastHost.vue — renders the global toast stack (top-right, Teleported).
//
// Mounted once in App.vue. It is the only consumer of `useToastStore` that
// *renders*; producers just call `toast.push(...)` from anywhere.
//
// It is also the one place the unified notice channel (Step 5.2) is projected
// onto the screen — see `TOASTED_SOURCES` for why that set is deliberately
// small rather than "every error goes here".
// ============================================================================
import { onUnmounted, watch } from 'vue'
import { AlertTriangle, CheckCircle2, Info, X } from 'lucide-vue-next'
import { useToastStore, type ToastKind } from '@/stores/toast'
import { useNoticesStore, type NoticeSource } from '@/stores/notices'

const toast = useToastStore()
const notices = useNoticesStore()

/**
 * Notice sources with **no inline error surface of their own**.
 *
 * Everything else on the channel is already rendered where it happened — next
 * to the TUN switch, under the profile list, inside the update dialog — and a
 * second copy of the same sentence in the top-right corner is noise, not
 * information. These two are the exceptions: `kernel.lastError` and
 * `history.lastError` were written to for months and read by *nothing at all*,
 * which is how a dead controller could fail silently. They get a toast now.
 */
const TOASTED_SOURCES: readonly NoticeSource[] = ['kernel', 'history']

/** Highest notice id already toasted, so a re-render cannot re-toast. */
let lastToastedId = 0

const stopWatching = watch(
  () => notices.latest,
  (n) => {
    if (!n || n.id <= lastToastedId) return
    lastToastedId = n.id
    // `warn` deliberately stays off-screen: it means "degraded but working"
    // (e.g. a background history tick that failed while the chart kept its
    // cached series). It belongs in the diagnostics log, not on the user's face.
    if (n.severity !== 'error') return
    if (!TOASTED_SOURCES.includes(n.source)) return
    toast.push('error', sourceTitle(n.source), n.message)
  },
)

onUnmounted(stopWatching)

function sourceTitle(source: NoticeSource): string {
  return source === 'history' ? 'Traffic history' : 'Kernel'
}

function iconFor(kind: ToastKind) {
  switch (kind) {
    case 'success': return CheckCircle2
    case 'error': return AlertTriangle
    default: return Info
  }
}

function toneClass(kind: ToastKind): string {
  switch (kind) {
    case 'success':
      return 'border-emerald-500/30 bg-emerald-950/90 text-emerald-100'
    case 'error':
      return 'border-rose-500/30 bg-rose-950/90 text-rose-100'
    default:
      return 'text-zinc-100'
  }
}

function iconClass(kind: ToastKind): string {
  switch (kind) {
    case 'success':
      return 'text-emerald-400'
    case 'error':
      return 'text-rose-400'
    default:
      return 'text-sky-400'
  }
}
</script>

<template>
  <Teleport to="body">
    <div
      class="fixed top-4 right-4 z-[100] flex w-80 max-w-[calc(100vw-2rem)] flex-col gap-2 pointer-events-none"
    >
      <TransitionGroup name="toast">
        <div
          v-for="item in toast.items"
          :key="item.id"
          class="glass-popover pointer-events-auto flex items-start gap-2.5 rounded-xl px-3.5 py-3"
          :class="toneClass(item.kind)"
        >
          <component :is="iconFor(item.kind)" class="h-4 w-4 shrink-0 mt-0.5" :class="iconClass(item.kind)" />
          <div class="flex-1 min-w-0">
            <div class="text-sm font-medium leading-tight">{{ item.title }}</div>
            <div v-if="item.message" class="mt-0.5 text-xs opacity-80 break-words font-mono">
              {{ item.message }}
            </div>
          </div>
          <button
            type="button"
            class="shrink-0 rounded p-0.5 opacity-60 hover:opacity-100 transition-opacity"
            @click="toast.dismiss(item.id)"
          >
            <X class="h-3.5 w-3.5" />
          </button>
        </div>
      </TransitionGroup>
    </div>
  </Teleport>
</template>

<style scoped>
.toast-enter-active,
.toast-leave-active {
  transition: opacity 0.18s ease, transform 0.18s ease;
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateX(12px);
}
/* Slide the surviving cards into their new slots with an elegant curve
   instead of snapping. The leaving card is pulled out of flow (absolute) so
   its siblings can glide up beneath it. */
.toast-move {
  transition: transform 0.25s cubic-bezier(0.22, 1, 0.36, 1);
}
.toast-leave-active {
  position: absolute;
  width: 100%;
}
</style>

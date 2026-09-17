<script setup lang="ts">
// ============================================================================
// ToastHost.vue — renders the global toast stack (top-right, Teleported).
//
// Mounted once in App.vue. It is the only consumer of `useToastStore` that
// *renders*; producers just call `toast.push(...)` from anywhere.
// ============================================================================
import { AlertTriangle, CheckCircle2, Info, X } from 'lucide-vue-next'
import { useToastStore, type ToastKind } from '@/stores/toast'

const toast = useToastStore()

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
      return 'border-white/10 bg-zinc-900/95 text-zinc-100'
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
          class="pointer-events-auto flex items-start gap-2.5 rounded-xl border px-3.5 py-3 shadow-2xl backdrop-blur-md"
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
</style>

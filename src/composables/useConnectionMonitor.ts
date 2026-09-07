// ============================================================================
// useConnectionMonitor.ts — M8: on-demand polling lifecycle.
//
// Conditions for polling (ALL must be true):
//   1. `enabled` ref is true  (App.vue flips this when tab === 'connections')
//   2. `document.visibilityState === 'visible'`  (Page Visibility API)
//   3. The kernel is actually running
//   4. The user has not pressed pause
//
// When any condition flips false, the interval is torn down immediately.
// When the user manually pauses, we stop polling but keep the watcher so
// resume is instant.
// ============================================================================

import { onUnmounted, ref, watch, type Ref } from 'vue'

import { useConnectionsStore, type PollIntervalMs } from '@/stores/connections'
import { useKernelStore } from '@/stores/kernel'

interface UseConnectionMonitorOptions {
  /** Caller-controlled enable flag (e.g. current tab === 'connections'). */
  enabled: Ref<boolean>
  /** Override default poll interval at construction time. */
  initialIntervalMs?: PollIntervalMs
}

export function useConnectionMonitor(opts: UseConnectionMonitorOptions) {
  const store = useConnectionsStore()
  const kernel = useKernelStore()

  if (opts.initialIntervalMs) {
    store.setPollInterval(opts.initialIntervalMs)
  }

  const documentVisible = ref(
    typeof document === 'undefined'
      ? true
      : document.visibilityState === 'visible',
  )
  const onVisibilityChange = () => {
    documentVisible.value = document.visibilityState === 'visible'
  }
  if (typeof document !== 'undefined') {
    document.addEventListener('visibilitychange', onVisibilityChange)
  }

  let intervalId: ReturnType<typeof setInterval> | null = null
  let lastInterval = -1

  function clearIntervalNow(): void {
    if (intervalId !== null) {
      clearInterval(intervalId)
      intervalId = null
    }
  }

  function maybeStart(): void {
    clearIntervalNow()
    if (intervalId !== null) return
    if (!opts.enabled.value) return
    if (!documentVisible.value) return
    if (!kernel.isRunning) return
    if (store.isPaused) return

    const ms = store.pollIntervalMs
    lastInterval = ms
    // Fire one tick immediately so the UI is not empty on first activation.
    void store.refresh()
    intervalId = setInterval(() => {
      // Guard against the user changing the interval while a tick is queued.
      if (store.pollIntervalMs !== lastInterval) {
        clearIntervalNow()
        maybeStart()
        return
      }
      void store.refresh()
    }, ms)
  }

  function stop(): void {
    clearIntervalNow()
  }

  // React to any condition that should (re)start polling.
  watch(
    [opts.enabled, documentVisible, () => kernel.isRunning, () => store.isPaused, () => store.pollIntervalMs],
    () => maybeStart(),
    { immediate: true },
  )

  onUnmounted(() => {
    stop()
    if (typeof document !== 'undefined') {
      document.removeEventListener('visibilitychange', onVisibilityChange)
    }
  })

  return {
    documentVisible,
    stop,
    start: maybeStart,
  }
}

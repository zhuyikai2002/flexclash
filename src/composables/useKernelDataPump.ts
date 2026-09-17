// ============================================================================
// useKernelDataPump.ts — one orchestrator for every kernel-backed poll
// (v0.6.x Step 5.3).
//
// WHY THIS EXISTS
// ---------------
// Before it, each data view owned its own timer and its own idea of when the
// kernel was reachable:
//
//   * `useConnectionMonitor` watched `[tabActive, documentVisible,
//     kernel.isRunning, isPaused, pollIntervalMs]` and drove one interval;
//   * `stores/history.ts` ran a private 15 s `setInterval` started inside
//     `init()`, with no kernel gate at all;
//   * `ProxyGroups` / `RulesView` fired a one-shot fetch from a `watch` on
//     `kernel.isRunning`.
//
// Three timers, three sets of conditions, three chances to get "when do we
// stop?" wrong. And all of them answered a question nobody had defined: when
// the kernel goes away, is the data on screen *empty* or merely *old*? They
// all implicitly said "empty", which is why a vanished controller looked
// exactly like an idle network.
//
// WHAT THIS IS
// ------------
// One `watch` over one derived signal (`kernel.availability`) driving N tasks.
// A task that stops because the kernel went away is told, via `onKernelDown`,
// to mark its store **stale** rather than empty — the last known rows stay on
// screen with an honest "these may be out of date" note, and the first tick
// after recovery is a forced refresh so they are replaced rather than
// appended to.
//
// Overlap is impossible by construction: a task whose previous poll has not
// settled skips its tick instead of stacking requests.
// ============================================================================

import { computed, onUnmounted, ref, watch, type ComputedRef } from 'vue'
import { useKernelStore } from '@/stores/kernel'

export interface PumpTask {
  /** Diagnostic name; also the key for stale bookkeeping. */
  name: string
  /** One poll. Rejections are the caller's business — the pump never throws. */
  fetch: () => Promise<unknown>
  /**
   * Interval in ms, re-read on every (re)start so a user changing the poll
   * rate takes effect without a remount.
   */
  intervalMs: () => number
  /** Extra gate beyond "kernel is up": tab active, not paused, … */
  enabled?: () => boolean
}

export interface UseKernelDataPumpOptions {
  tasks: PumpTask[]
  /**
   * Called exactly once per up → not-up transition, i.e. on the *edge*, not
   * on every watcher re-run. This is where stores mark themselves stale.
   */
  onKernelDown?: () => void
  /**
   * Called exactly once per not-up → up transition. The natural thing to do
   * here is force a refresh so recovered data replaces stale data instead of
   * sitting next to it.
   */
  onKernelUp?: () => void
}

interface Slot {
  timer: ReturnType<typeof setInterval> | null
  interval: number
  inFlight: boolean
}

export function useKernelDataPump(opts: UseKernelDataPumpOptions) {
  const kernel = useKernelStore()

  const documentVisible = ref(
    typeof document === 'undefined' ? true : document.visibilityState === 'visible',
  )
  const onVisibilityChange = () => {
    documentVisible.value =
      typeof document === 'undefined' || document.visibilityState === 'visible'
  }
  if (typeof document !== 'undefined') {
    document.addEventListener('visibilitychange', onVisibilityChange)
  }

  const slots: Slot[] = opts.tasks.map(() => ({
    timer: null,
    interval: 0,
    inFlight: false,
  }))

  /** Tracks the up/not-up edge so the callbacks fire once, not per re-run. */
  let wasUp = false

  /** Per-task enable flags, read through a computed so `watch` tracks them. */
  const gates: ComputedRef<boolean[]> = computed(() =>
    opts.tasks.map((t) => (t.enabled ? t.enabled() : true)),
  )

  function stopSlot(i: number): void {
    const slot = slots[i]
    if (slot && slot.timer !== null) {
      clearInterval(slot.timer)
      slot.timer = null
      slot.interval = 0
    }
  }

  function startSlot(i: number): void {
    const task = opts.tasks[i]
    const slot = slots[i]
    if (!task || !slot) return

    const ms = task.intervalMs()
    // Already running at this interval: leave it alone, or every unrelated
    // watcher tick would restart the timer and reset the phase.
    if (slot.timer !== null && slot.interval === ms) return
    stopSlot(i)

    slot.interval = ms
    // One immediate poll so a freshly opened tab is never empty.
    void run(i)
    slot.timer = setInterval(() => void run(i), ms)
  }

  async function run(i: number): Promise<void> {
    const task = opts.tasks[i]
    const slot = slots[i]
    if (!task || !slot || slot.inFlight) return
    slot.inFlight = true
    try {
      await task.fetch()
    } catch {
      // The store raises on the notice channel; the pump only has to survive.
    } finally {
      slot.inFlight = false
    }
  }

  function sync(): void {
    const up = kernel.availability === 'up'

    if (up !== wasUp) {
      wasUp = up
      if (up) opts.onKernelUp?.()
      else opts.onKernelDown?.()
    }

    for (let i = 0; i < opts.tasks.length; i += 1) {
      const gate = gates.value[i] ?? true
      if (up && documentVisible.value && gate) startSlot(i)
      else stopSlot(i)
    }
  }

  watch(
    [() => kernel.availability, documentVisible, gates],
    () => sync(),
    { immediate: true, deep: true },
  )

  function stop(): void {
    for (let i = 0; i < slots.length; i += 1) stopSlot(i)
  }

  onUnmounted(() => {
    stop()
    if (typeof document !== 'undefined') {
      document.removeEventListener('visibilitychange', onVisibilityChange)
    }
  })

  return { documentVisible, stop, start: sync }
}

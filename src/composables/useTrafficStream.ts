// ============================================================================
// useTrafficStream.ts — reactive lifecycle wrapper around Mihomo `/traffic`
// WebSocket. Lives inside the Vue tree (uses inject/let/closure), so it MUST
// be created from a setup() context.
//
// Features:
//   - Auto-connect when the kernel is running; auto-disconnect when not.
//   - Reconnect with exponential-ish backoff after unexpected closes.
//   - Proper teardown on component unmount (no leaked sockets).
//   - Exposes the last sample's `up` / `down` and a connected flag.
//   - Bundles a human-readable rate formatter.
//
// IMPORTANT: this opens a raw WebSocket straight to Mihomo on 127.0.0.1:9091.
// The Rust side is not involved in the data path.
// ============================================================================

import { onUnmounted, ref, watch } from 'vue'
import { MIHOMO_WS_URL } from '@/services/clash'
import { useKernelStore } from '@/stores/kernel'
import type { TrafficSample } from '@/types/clash'

export interface TrafficStreamOptions {
  /** Reconnect after unexpected closes. Default: true. */
  autoReconnect?: boolean
  /** Base delay between reconnect attempts (ms). Default: 2000. */
  reconnectDelayMs?: number
  /** Cap on consecutive reconnect attempts. Default: Infinity. */
  maxReconnectAttempts?: number
}

const STREAM_KEY = '/traffic'

/** Format bytes-per-second into B/s, KB/s, MB/s, GB/s (1 decimal). */
export function formatRate(bytesPerSec: number): string {
  if (!Number.isFinite(bytesPerSec) || bytesPerSec <= 0) return '0 B/s'
  const KB = 1024
  const MB = KB * 1024
  const GB = MB * 1024
  if (bytesPerSec < KB) return `${bytesPerSec.toFixed(0)} B/s`
  if (bytesPerSec < MB) return `${(bytesPerSec / KB).toFixed(1)} KB/s`
  if (bytesPerSec < GB) return `${(bytesPerSec / MB).toFixed(1)} MB/s`
  return `${(bytesPerSec / GB).toFixed(2)} GB/s`
}

export function useTrafficStream(opts: TrafficStreamOptions = {}) {
  const {
    autoReconnect = true,
    reconnectDelayMs = 2_000,
    maxReconnectAttempts = Number.POSITIVE_INFINITY,
  } = opts

  const up = ref(0)
  const down = ref(0)
  const connected = ref(false)
  const lastUpdateAt = ref<number | null>(null)
  const errorMsg = ref<string | null>(null)
  const reconnectAttempt = ref(0)

  let ws: WebSocket | null = null
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null
  /** Set to true when the consumer (or kernel-stopped) intentionally closes. */
  let manualClose = false

  const kernel = useKernelStore()

  function clearReconnect(): void {
    if (reconnectTimer !== null) {
      clearTimeout(reconnectTimer)
      reconnectTimer = null
    }
  }

  function scheduleReconnect(): void {
    if (!autoReconnect) return
    if (reconnectAttempt.value >= maxReconnectAttempts) return
    clearReconnect()
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null
      reconnectAttempt.value += 1
      connect()
    }, reconnectDelayMs)
  }

  function connect(): void {
    if (ws) return // already connecting/open
    manualClose = false
    clearReconnect()

    try {
      ws = new WebSocket(`${MIHOMO_WS_URL}${STREAM_KEY}`)
    } catch (e) {
      errorMsg.value = e instanceof Error ? e.message : String(e)
      scheduleReconnect()
      return
    }

    ws.onopen = () => {
      connected.value = true
      errorMsg.value = null
      reconnectAttempt.value = 0
    }

    ws.onmessage = (ev) => {
      try {
        const sample = JSON.parse(ev.data) as TrafficSample
        if (typeof sample.up === 'number') up.value = sample.up
        if (typeof sample.down === 'number') down.value = sample.down
        lastUpdateAt.value = Date.now()
      } catch {
        /* malformed payload — ignore */
      }
    }

    ws.onerror = () => {
      errorMsg.value = 'WebSocket error'
    }

    ws.onclose = () => {
      connected.value = false
      ws = null
      if (!manualClose) scheduleReconnect()
    }
  }

  function disconnect(): void {
    manualClose = true
    clearReconnect()
    if (ws) {
      try { ws.close() } catch { /* noop */ }
      ws = null
    }
    connected.value = false
  }

  function reconnectNow(): void {
    disconnect()
    reconnectAttempt.value = 0
    manualClose = false
    connect()
  }

  // React to kernel lifecycle — stream only makes sense while running.
  watch(
    () => kernel.isRunning,
    (running) => {
      if (running) connect()
      else disconnect()
    },
    { immediate: true },
  )

  // Hard teardown on component unmount.
  onUnmounted(() => {
    disconnect()
  })

  return {
    up,
    down,
    connected,
    lastUpdateAt,
    error: errorMsg,
    reconnectAttempt,
    reconnect: reconnectNow,
    disconnect,
  }
}

// ============================================================================
// stores/kernel.ts — Pinia store for the Mihomo sidecar lifecycle.
//
// Responsibilities:
//   - Mirror the Rust-side `KernelState` enum onto the frontend.
//   - Subscribe to `kernel://*` events emitted by sidecar.rs.
//   - Drive the direct axios probe (does NOT go through Rust).
//
// State machine:
//   backend `KernelState` is single-writer on the Rust side.
//   The store here is read-mostly; it only writes when an event arrives or a
//   local command (start/stop/restart) resolves.
//
// Type contract (per spec):
//   `KernelStoreState` describes the *public* reactive state exposed by
//   this store. The string-union type `KernelState` is the single source
//   of truth for the kernel's lifecycle position and is imported from
//   `@/types/clash` (where the frontend-wide types live).
//   Internal-only fields (event teardown list, latest error) are kept
//   outside the interface so the public contract is tight and small.
// ============================================================================

import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

import { getVersion, MIHOMO_BASE_URL, pollUntil, isAlive } from '@/services/clash'
import type { KernelState } from '@/types/clash'

interface ConfigRefreshNotice {
  fromPort: number | null
  toPort: number
}

/**
 * Public reactive state of the kernel store.
 *
 * The string-union field `state` is the only place that mirrors the
 * Rust `KernelState` enum; everything else is presentation / health
 * data derived from REST probes.
 */
export interface KernelStoreState {
  state: KernelState
  version: string | null
  endpoint: string
  probeStatus: 'idle' | 'probing' | 'healthy' | 'error'
  probeLatencyMs: number | null
  recentLogs: string[]
  configRefresh: ConfigRefreshNotice | null
}

const LOG_CAP = 120

// ---------------------------------------------------------------------------
// Internal (non-public) state.  Kept module-scoped because nothing in the
// UI cares about them, and they avoid cluttering the public interface.
// `_unlisteners` is a plain `let` because Tauri unlisten handles are
// imperative, not reactive.  `lastErrorRef` IS reactive so the dashboard
// toast / banner can render it via a getter on the store.
// ---------------------------------------------------------------------------
let _unlisteners: UnlistenFn[] = []
const lastErrorRef = ref<string | null>(null)

export const useKernelStore = defineStore('kernel', {
  state: (): KernelStoreState => ({
    state: 'unknown',
    version: null,
    endpoint: MIHOMO_BASE_URL,
    probeStatus: 'idle',
    probeLatencyMs: null,
    recentLogs: [],
    configRefresh: null,
  }),

  getters: {
    isRunning: (s): boolean => s.state === 'running',
    isTransitioning: (s): boolean =>
      s.state === 'starting' || s.state === 'stopping',
    /** Latest error message, or `null` when no error is pending. */
    lastError: (): string | null => lastErrorRef.value,
  },

  actions: {
    /** Called once on app mount. Subscribes events + syncs initial state. */
    async init(): Promise<void> {
      await this.subscribeEvents()
      try {
        this.state = await invoke<KernelState>('get_kernel_state')
      } catch (e) {
        lastErrorRef.value = String(e)
      }
      // If the kernel is already running (post-restart, autostart) we still
      // need to poll until mihomo actually binds 9091 — otherwise the first
      // proxies / traffic calls would race the bind and fail noisily. The
      // backoff window is 5s (10 × 500ms), enough for cold start.
      if (this.state === 'running') {
        await this.probeWithBackoff()
      } else {
        // Even if Rust says "stopped", mihomo may already be binding; try
        // a single cheap probe and let the user click "start" if it fails.
        const v = await isAlive(1500)
        if (v) {
          this.state = 'running'
          await this.probeWithBackoff()
        }
      }
    },

    async subscribeEvents(): Promise<void> {
      // Idempotent: if already subscribed, do nothing.
      if (_unlisteners.length > 0) return

      _unlisteners.push(
        await listen<KernelState>('kernel://state', (e) => {
          this.state = e.payload
          if (e.payload === 'running') {
            void this.probe()
          } else if (e.payload === 'stopped' || e.payload === 'crashed') {
            this.probeStatus = 'idle'
            this.probeLatencyMs = null
          }
        }),
      )

      _unlisteners.push(
        await listen<string>('kernel://log', (e) => {
          this.recentLogs.push(e.payload)
          if (this.recentLogs.length > LOG_CAP) {
            this.recentLogs.splice(0, this.recentLogs.length - LOG_CAP)
          }
        }),
      )

      _unlisteners.push(
        await listen<ConfigRefreshPayload>('kernel://config-refreshed', (e) => {
          if (e.payload.kind === 'port_changed') {
            this.configRefresh = {
              fromPort: e.payload.from_port ?? null,
              toPort: e.payload.to_port,
            }
          }
        }),
      )

      _unlisteners.push(
        await listen<{ code: number | null; signal: number | null }>(
          'kernel://terminated',
          (e) => {
            this.recentLogs.push(
              `[terminated] code=${e.payload.code ?? 'null'} signal=${e.payload.signal ?? 'null'}`,
            )
          },
        ),
      )
    },

    /** Probe the frontend API. Updates version + latency on success. */
    async probe(): Promise<void> {
      this.probeStatus = 'probing'
      const t0 = performance.now()
      try {
        const ver = await getVersion()
        this.version = ver.version
        this.probeLatencyMs = Math.round(performance.now() - t0)
        this.probeStatus = 'healthy'
        lastErrorRef.value = null
      } catch (e) {
        this.probeStatus = 'error'
        this.probeLatencyMs = null
        lastErrorRef.value = e instanceof Error ? e.message : String(e)
      }
    },

    /**
     * Poll `GET /version` every 500ms up to 10 times (5s backoff window).
     * Designed to ride out mihomo's cold-start: even after Rust flips
     * `state → running`, the TCP port 9091 may not be bound for another
     * 500-1500ms (especially on first launch when the MMDB geoip file
     * is being downloaded).
     *
     * On success: mark `healthy` and set `version`.
     * On total failure: mark `error` and surface the most recent
     * transport-level reason (CORS / refused / timeout) so the user
     * knows whether the problem is theirs or the kernel's.
     */
    async probeWithBackoff(): Promise<void> {
      this.probeStatus = 'probing'
      const t0 = performance.now()
      const result = await pollUntil(
        () => isAlive(2000).then((ok) => (ok ? true : null)),
        { intervalMs: 500, maxAttempts: 10, label: 'mihomo/version' },
      )
      if (result.ok) {
        try {
          const ver = await getVersion(2000)
          this.version = ver.version
          this.probeLatencyMs = Math.round(performance.now() - t0)
          this.probeStatus = 'healthy'
          lastErrorRef.value = null
        } catch (e) {
          // Shouldn't happen — `isAlive` just passed — but be defensive.
          this.probeStatus = 'error'
          this.probeLatencyMs = null
          lastErrorRef.value = e instanceof Error ? e.message : String(e)
        }
      } else {
        this.probeStatus = 'error'
        this.probeLatencyMs = null
        const reason = result.lastError?.message ?? 'mihomo unreachable after 5s'
        lastErrorRef.value = `${reason} (tried ${result.attempts} times)`
        // eslint-disable-next-line no-console
        console.warn(`[kernel] probeWithBackoff failed: ${lastErrorRef.value}`)
      }
    },

    async start(): Promise<void> {
      lastErrorRef.value = null
      try {
        this.state = await invoke<KernelState>('start_kernel')
        // start() itself only flips state to Starting; Running arrives via event.
      } catch (e) {
        lastErrorRef.value = String(e)
        throw e
      }
    },

    async stop(): Promise<void> {
      lastErrorRef.value = null
      try {
        this.state = await invoke<KernelState>('stop_kernel')
      } catch (e) {
        lastErrorRef.value = String(e)
        throw e
      }
    },

    async restart(): Promise<void> {
      lastErrorRef.value = null
      this.version = null
      this.probeLatencyMs = null
      try {
        this.state = await invoke<KernelState>('restart_kernel')
        // Mihomo needs ~1-2s to rebind 9091. Use the full backoff window
        // instead of a fixed 700ms sleep so MMDB download / Wintun init
        // don't trip a false "unreachable".
        await this.probeWithBackoff()
      } catch (e) {
        lastErrorRef.value = String(e)
        throw e
      }
    },

    /** Tear down all Tauri event listeners (used by HMR / tests). */
    dispose(): void {
      for (const u of _unlisteners) {
        try { u() } catch { /* noop */ }
      }
      _unlisteners = []
    },
  },
})

// Mirror of `ConfigRefresh` enum from sidecar.rs (Phase 2 only needs PortChanged).
// `to_port` is always present on the Rust side (u16, non-Option);
// `from_port` is Option<u16> → serialises as `null` when absent.
interface ConfigRefreshPayload {
  kind: 'created' | 'port_changed' | 'unchanged'
  from_port: number | null
  to_port: number
}

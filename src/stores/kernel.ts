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
// ============================================================================

import { defineStore } from 'pinia'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

import { getVersion, MIHOMO_BASE_URL } from '@/services/clash'

export type KernelStateStr =
  | 'stopped'
  | 'starting'
  | 'running'
  | 'stopping'
  | 'crashed'
  | 'unknown'

export type ProbeStatus = 'idle' | 'probing' | 'alive' | 'unreachable'

interface ConfigRefreshNotice {
  fromPort: number | null
  toPort: number
}

interface KernelState {
  state: KernelStateStr
  version: string | null
  endpoint: string
  probeStatus: ProbeStatus
  probeLatencyMs: number | null
  lastError: string | null
  recentLogs: string[]
  configRefresh: ConfigRefreshNotice | null
  // Listeners kept so the store could be torn down if needed (e.g. HMR).
  _unlisteners: UnlistenFn[]
}

const LOG_CAP = 120

export const useKernelStore = defineStore('kernel', {
  state: (): KernelState => ({
    state: 'unknown',
    version: null,
    endpoint: MIHOMO_BASE_URL,
    probeStatus: 'idle',
    probeLatencyMs: null,
    lastError: null,
    recentLogs: [],
    configRefresh: null,
    _unlisteners: [],
  }),

  getters: {
    isRunning: (s): boolean => s.state === 'running',
    isTransitioning: (s): boolean =>
      s.state === 'starting' || s.state === 'stopping',
  },

  actions: {
    /** Called once on app mount. Subscribes events + syncs initial state. */
    async init(): Promise<void> {
      await this.subscribeEvents()
      try {
        this.state = await invoke<KernelStateStr>('get_kernel_state')
        if (this.state === 'running') {
          await this.probe()
        }
      } catch (e) {
        this.lastError = String(e)
      }
    },

    async subscribeEvents(): Promise<void> {
      // Idempotent: if already subscribed, do nothing.
      if (this._unlisteners.length > 0) return

      this._unlisteners.push(
        await listen<KernelStateStr>('kernel://state', (e) => {
          this.state = e.payload
          if (e.payload === 'running') {
            void this.probe()
          } else if (e.payload === 'stopped' || e.payload === 'crashed') {
            this.probeStatus = 'idle'
            this.probeLatencyMs = null
          }
        }),
      )

      this._unlisteners.push(
        await listen<string>('kernel://log', (e) => {
          this.recentLogs.push(e.payload)
          if (this.recentLogs.length > LOG_CAP) {
            this.recentLogs.splice(0, this.recentLogs.length - LOG_CAP)
          }
        }),
      )

      this._unlisteners.push(
        await listen<ConfigRefreshPayload>('kernel://config-refreshed', (e) => {
          if (e.payload.kind === 'port_changed') {
            this.configRefresh = {
              fromPort: e.payload.from_port ?? null,
              toPort: e.payload.to_port,
            }
          }
        }),
      )

      this._unlisteners.push(
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
        this.probeStatus = 'alive'
        this.lastError = null
      } catch (e) {
        this.probeStatus = 'unreachable'
        this.probeLatencyMs = null
        this.lastError = e instanceof Error ? e.message : String(e)
      }
    },

    async start(): Promise<void> {
      this.lastError = null
      try {
        this.state = await invoke<KernelStateStr>('start_kernel')
        // start() itself only flips state to Starting; Running arrives via event.
      } catch (e) {
        this.lastError = String(e)
        throw e
      }
    },

    async stop(): Promise<void> {
      this.lastError = null
      try {
        this.state = await invoke<KernelStateStr>('stop_kernel')
      } catch (e) {
        this.lastError = String(e)
        throw e
      }
    },

    async restart(): Promise<void> {
      this.lastError = null
      this.version = null
      this.probeLatencyMs = null
      try {
        this.state = await invoke<KernelStateStr>('restart_kernel')
        // Mihomo needs a moment to listen again after restart.
        await new Promise((r) => setTimeout(r, 700))
        await this.probe()
      } catch (e) {
        this.lastError = String(e)
        throw e
      }
    },

    /** Tear down all Tauri event listeners (used by HMR / tests). */
    dispose(): void {
      for (const u of this._unlisteners) {
        try { u() } catch { /* noop */ }
      }
      this._unlisteners = []
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

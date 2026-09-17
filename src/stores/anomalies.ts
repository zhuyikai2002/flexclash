// ============================================================================
// stores/anomalies.ts — subscribe to the typed `log-anomaly` event stream.
//
// The Rust `core::log_parse` promotes a handful of connectivity signatures out
// of the raw `/logs` text into a strongly-typed `LogAnomaly`. This store is the
// single renderer-side consumer:
//
//   * it keeps a bounded `records` ring (the newest 50) for a future
//     "异常诊断面板" (settings / diagnostics);
//   * it fans the *fatal* subset out to the global toast bus. DNS resolve
//     failures and TLS errors are the ones the user actually needs to know
//     about *now*; dial timeouts / refused / proxy switches are high-volume
//     and stay in the records only.
//
// Anti-flood: a burst of failures (a dead DNS server can emit hundreds of
// `no such host` lines a second) must not spray the screen with toasts. The
// same `kind + host` is only allowed one toast per `THROTTLE_MS` window.
// ============================================================================

import { defineStore } from 'pinia'
import { events, type LogAnomaly } from '@/bindings'
import { safeListenEvent, type UnlistenFn } from '@/utils/tauri-bridge'
import { useToastStore } from '@/stores/toast'
import { i18n } from '@/i18n'

/** One recorded anomaly, newest-first invariant broken by `push` (append). */
export interface AnomalyRecord {
  id: number
  at: number
  anomaly: LogAnomaly
}

/** Ring capacity for the diagnostics panel. */
const RECORD_CAP = 50

/** Same `kind + host` may toast at most once in this window (ms). */
const THROTTLE_MS = 4000

/** Process-wide subscription, created once (mirrors the kernel store pattern). */
let _unlisten: UnlistenFn | null = null
let _seq = 0

/** `kind:host` → last toast timestamp (module-scoped, not reactive). */
const _lastToastAt = new Map<string, number>()

/** The subset of anomalies that deserve an immediate toast. */
function isFatal(a: LogAnomaly): boolean {
  return a.kind === 'dns_resolve_failed' || a.kind === 'tls_error'
}

/** The host used both for display and for the throttle key. */
function hostOf(a: LogAnomaly): string {
  return a.kind === 'dns_resolve_failed' || a.kind === 'tls_error' ? a.host : ''
}

/** Resolve the localized title for a fatal anomaly. */
function titleOf(a: LogAnomaly): string {
  if (a.kind === 'dns_resolve_failed') return i18n.global.t('anomalies.toast.dns')
  if (a.kind === 'tls_error') return i18n.global.t('anomalies.toast.tls')
  return ''
}

/** Resolve the localized detail line for a fatal anomaly. */
function detailOf(a: LogAnomaly): string {
  if (a.kind === 'dns_resolve_failed') {
    return a.host ? `${a.host} · ${a.detail || 'unknown'}` : (a.detail || '')
  }
  if (a.kind === 'tls_error') {
    return a.host ? `${a.host} · ${a.detail || 'tls'}` : (a.detail || '')
  }
  return ''
}

export const useAnomaliesStore = defineStore('anomalies', {
  state: () => ({
    /** Newest 50 anomalies, oldest dropped first. */
    records: [] as AnomalyRecord[],
  }),

  actions: {
    /** Subscribe to `events.logAnomaly`. Idempotent. */
    init(): void {
      if (_unlisten) return
      safeListenEvent(events.logAnomaly, (e) => this.ingest(e.payload)).then((u) => {
        _unlisten = u
      })
    },

    /** Record the anomaly, and toast it if it is fatal and not throttled. */
    ingest(a: LogAnomaly): void {
      this.records.push({ id: ++_seq, at: Date.now(), anomaly: a })
      if (this.records.length > RECORD_CAP) {
        this.records.splice(0, this.records.length - RECORD_CAP)
      }

      if (!isFatal(a)) return

      const key = `${a.kind}:${hostOf(a)}`
      const now = Date.now()
      const last = _lastToastAt.get(key) ?? 0
      if (now - last < THROTTLE_MS) return

      _lastToastAt.set(key, now)
      useToastStore().push('error', titleOf(a), detailOf(a))
    },

    /** Tear down the event subscription. */
    dispose(): void {
      _unlisten?.()
      _unlisten = null
    },
  },
})

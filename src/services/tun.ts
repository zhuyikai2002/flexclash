// ============================================================================
// services/tun.ts — Tauri command surface for M9 TUN mode.
//
// All real work happens in Rust (commands/tun.rs → core::tun::TunManager).
// The store + UI are kept off the raw invoke() calls so swap-outs stay
// local.  `safeInvoke` is used so browser preview doesn't throw.
// ============================================================================

import { safeInvoke, safeInvokeOr } from '@/utils/tauri-bridge'

export type TunState = 'off' | 'enabling' | 'on' | 'disabling' | 'failed'

export interface TunStatus {
  state: TunState
  enabled: boolean
  device: string
  last_error: string | null
  last_changed_at_ms: number
  last_sweep: SweepResultFull | null
}

export interface SweepResultFull {
  deleted_routes: number
  deleted_adapters: number
  ok: boolean
  message: string
}

const DEFAULT_STATUS: TunStatus = {
  state: 'off',
  enabled: false,
  device: 'flexclash-tun',
  last_error: null,
  last_changed_at_ms: 0,
  last_sweep: null,
}

export async function getTunState(): Promise<TunStatus> {
  return await safeInvokeOr<TunStatus>('get_tun_state', DEFAULT_STATUS)
}

export async function enableTun(): Promise<TunStatus> {
  return await safeInvoke<TunStatus>('enable_tun')
}

export async function disableTun(): Promise<TunStatus> {
  return await safeInvoke<TunStatus>('disable_tun')
}

export async function sweepTunRoutes(): Promise<SweepResultFull> {
  return await safeInvoke<SweepResultFull>('sweep_tun_routes')
}

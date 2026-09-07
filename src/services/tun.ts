// ============================================================================
// services/tun.ts — Tauri command surface for M9 TUN mode.
//
// All real work happens in Rust (commands/tun.rs → core::tun::TunManager).
// The store + UI are kept off the raw invoke() calls so swap-outs stay
// local.
// ============================================================================

import { invoke } from '@tauri-apps/api/core'

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

export async function getTunState(): Promise<TunStatus> {
  return await invoke<TunStatus>('get_tun_state')
}

export async function enableTun(): Promise<TunStatus> {
  return await invoke<TunStatus>('enable_tun')
}

export async function disableTun(): Promise<TunStatus> {
  return await invoke<TunStatus>('disable_tun')
}

export async function sweepTunRoutes(): Promise<SweepResultFull> {
  return await invoke<SweepResultFull>('sweep_tun_routes')
}

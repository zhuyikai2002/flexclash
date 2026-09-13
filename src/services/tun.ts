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

/**
 * The two switches from Settings -> "TUN Advanced". They are stamped onto
 * the `tun:` block of the active config at enable time — see
 * `src-tauri/src/config/profile.rs::toggle_tun_block`.
 */
export interface TunAdvancedOptions {
  strictRoute: boolean
  dnsHijack: boolean
}

/** Defaults mirror `TUN_ADVANCED_DEFAULTS` in `@/stores/settings`. */
const TUN_ADVANCED_FALLBACK: TunAdvancedOptions = {
  strictRoute: false,
  dnsHijack: true,
}

export async function getTunState(): Promise<TunStatus> {
  return await safeInvokeOr<TunStatus>('get_tun_state', DEFAULT_STATUS)
}

/**
 * Enable TUN, stamping the advanced switches onto the `tun:` block.
 *
 * Argument names are snake_case to match
 * `#[tauri::command(rename_all = "snake_case")]` on the Rust side — Tauri 2
 * defaults to camelCase, and the explicit annotation is what keeps the two
 * ends honest rather than relying on a convention nobody can see.
 *
 * `advanced` is optional so an existing caller that does not care about the
 * switches keeps working; the backend then applies the same documented
 * defaults via `Option<bool>`.
 */
export async function enableTun(advanced?: TunAdvancedOptions): Promise<TunStatus> {
  const adv = advanced ?? TUN_ADVANCED_FALLBACK
  return await safeInvoke<TunStatus>('enable_tun', {
    strict_route: adv.strictRoute,
    dns_hijack: adv.dnsHijack,
  })
}

/**
 * Re-stamp the switches while TUN is *already* running, then ask the
 * elevated mihomo to hot-reload so they take effect immediately. When TUN is
 * off this is a cheap no-op on the backend — the switches are already
 * persisted and will be applied by the next {@link enableTun}.
 */
export async function applyTunAdvanced(advanced: TunAdvancedOptions): Promise<TunStatus> {
  return await safeInvoke<TunStatus>('apply_tun_advanced', {
    strict_route: advanced.strictRoute,
    dns_hijack: advanced.dnsHijack,
  })
}

export async function disableTun(): Promise<TunStatus> {
  return await safeInvoke<TunStatus>('disable_tun')
}

export async function sweepTunRoutes(): Promise<SweepResultFull> {
  return await safeInvoke<SweepResultFull>('sweep_tun_routes')
}

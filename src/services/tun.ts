// ============================================================================
// services/tun.ts — Tauri command surface for TUN mode.
//
// All real work happens in Rust (commands/tun.rs → core::tun::TunManager).
// Wire types come from the generated bindings.
// ============================================================================

import { commands, type SweepResultFull, type TunStatus } from '@/bindings'
import { call, guardInTauri, inTauri } from '@/utils/tauri-bridge'

const DEFAULT_STATUS: TunStatus = {
  state: 'off',
  enabled: false,
  device: 'flexclash-tun',
  last_error: null,
  last_changed_at_ms: 0,
  last_sweep: null,
}

/**
 * The two switches from Settings -> "TUN Advanced", in the renderer's
 * camelCase spelling. They are stamped onto the `tun:` block of the active
 * config at enable time — see
 * `src-tauri/src/config/profile.rs::toggle_tun_block`.
 *
 * Deliberately a frontend-side input type, not an IPC mirror: the Rust side
 * takes these as two separate `Option<bool>` arguments rather than a struct.
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
  if (!inTauri('get_tun_state')) return DEFAULT_STATUS
  return call(commands.getTunState())
}

/**
 * Enable TUN, stamping the advanced switches onto the `tun:` block.
 *
 * NOTE ON ARGUMENT NAMES: `commands.enableTun` sends `{ strictRoute,
 * dnsHijack }`. That is not a style choice — Tauri 2 defaults command
 * arguments to camelCase, and tauri-specta hard-codes the same convention for
 * the payload keys it generates regardless of any `rename_all` on the Rust
 * command. This module previously hand-wrote `{ strict_route, dns_hijack }`,
 * which the Rust side read as `None` (both switches silently reset to their
 * defaults on every enable). Routing through the generated command is what
 * makes that class of bug impossible.
 *
 * `advanced` is optional so a caller that does not care about the switches
 * keeps working; the backend then applies the same documented defaults.
 */
export async function enableTun(advanced?: TunAdvancedOptions): Promise<TunStatus> {
  guardInTauri('enable_tun')
  const adv = advanced ?? TUN_ADVANCED_FALLBACK
  return call(commands.enableTun(adv.strictRoute, adv.dnsHijack))
}

/**
 * Re-stamp the switches while TUN is *already* running, then ask the
 * elevated mihomo to hot-reload so they take effect immediately. When TUN is
 * off this is a cheap no-op on the backend — the switches are already
 * persisted and will be applied by the next {@link enableTun}.
 */
export async function applyTunAdvanced(advanced: TunAdvancedOptions): Promise<TunStatus> {
  guardInTauri('apply_tun_advanced')
  return call(commands.applyTunAdvanced(advanced.strictRoute, advanced.dnsHijack))
}

export async function disableTun(): Promise<TunStatus> {
  guardInTauri('disable_tun')
  return call(commands.disableTun())
}

export async function sweepTunRoutes(): Promise<SweepResultFull> {
  guardInTauri('sweep_tun_routes')
  return call(commands.sweepTunRoutes())
}

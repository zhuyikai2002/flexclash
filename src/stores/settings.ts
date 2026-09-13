// ============================================================================
// stores/settings.ts — persisted application preferences (renderer-side).
//
// Scope
// -----
// The single owner of user-facing *preferences*. This is deliberately the
// opposite of `stores/tun.ts`, which projects runtime state that Rust
// already owns: everything in here is authored by the user, survives a
// restart, and is *handed down* to the backend as an argument when an
// action needs it (e.g. `enable_tun({ strictRoute, dnsHijack })`).
//
// Persistence
// -----------
// `localStorage`, one namespaced key. Reads are defensive:
//   * no `localStorage` (SSR / plain-browser preview)  -> defaults,
//   * missing key                                      -> defaults,
//   * partial / hand-edited / corrupt JSON             -> merged per field,
// so a bad record can never leave the settings screen unable to render.
// The key is wiped by `reset_application` (it calls `localStorage.clear()`),
// which is exactly the semantics we want for "factory reset".
// ============================================================================

import { defineStore } from 'pinia'

/** Namespaced storage key for the whole settings record. */
export const SETTINGS_STORAGE_KEY = 'flexclash.settings'

/** The two user-facing switches in Settings -> "TUN Advanced". */
export interface TunAdvancedSettings {
  /**
   * `tun.strict-route` — refuse any traffic that would escape the tunnel
   * instead of silently leaking it out of the physical interface.
   */
  strictRoute: boolean
  /**
   * `tun.dns-hijack` — capture :53 (UDP *and* TCP) so DNS cannot leak
   * around the tunnel.
   */
  dnsHijack: boolean
}

/**
 * Documented defaults, mirrored by Rust's `config::profile::TunAdvanced`
 * — keep the two in sync.
 *
 * `dnsHijack` defaults **on**: a TUN that resolves through the host
 * resolver hands the ISP every domain it visits, which defeats the point
 * of the tunnel. `strictRoute` defaults **off**: it is the more
 * opinionated of the two and can break LAN / nested-container setups, so
 * it is opt-in.
 */
export const TUN_ADVANCED_DEFAULTS: TunAdvancedSettings = {
  strictRoute: false,
  dnsHijack: true,
}

interface SettingsState {
  tunAdvanced: TunAdvancedSettings
}

/** Narrow unknown JSON into the documented shape, field by field. */
function readPersisted(): TunAdvancedSettings {
  const read = (raw: string | null): TunAdvancedSettings => {
    if (!raw) return { ...TUN_ADVANCED_DEFAULTS }
    try {
      const parsed = JSON.parse(raw) as { tunAdvanced?: Partial<TunAdvancedSettings> } | null
      const t = parsed?.tunAdvanced
      return {
        strictRoute:
          typeof t?.strictRoute === 'boolean'
            ? t.strictRoute
            : TUN_ADVANCED_DEFAULTS.strictRoute,
        dnsHijack:
          typeof t?.dnsHijack === 'boolean' ? t.dnsHijack : TUN_ADVANCED_DEFAULTS.dnsHijack,
      }
    } catch {
      // Corrupt payload: fall back rather than throw during store setup.
      return { ...TUN_ADVANCED_DEFAULTS }
    }
  }

  if (typeof localStorage === 'undefined') return { ...TUN_ADVANCED_DEFAULTS }
  try {
    return read(localStorage.getItem(SETTINGS_STORAGE_KEY))
  } catch {
    // Some privacy modes throw on read as well as write.
    return { ...TUN_ADVANCED_DEFAULTS }
  }
}

export const useSettingsStore = defineStore('settings', {
  state: (): SettingsState => ({ tunAdvanced: readPersisted() }),

  getters: {
    /** Snapshot suitable for the `enable_tun` / `apply_tun_advanced` IPC
     *  arguments. Kept as a getter so call sites cannot drift from the
     *  store's own shape. */
    tunAdvancedSnapshot: (s): TunAdvancedSettings => ({ ...s.tunAdvanced }),
  },

  actions: {
    setStrictRoute(v: boolean): void {
      this.tunAdvanced.strictRoute = v
      this.persist()
    },

    setDnsHijack(v: boolean): void {
      this.tunAdvanced.dnsHijack = v
      this.persist()
    },

    toggleStrictRoute(): void {
      this.setStrictRoute(!this.tunAdvanced.strictRoute)
    },

    toggleDnsHijack(): void {
      this.setDnsHijack(!this.tunAdvanced.dnsHijack)
    },

    /** Write the record back to `localStorage`. Best-effort: a quota /
     *  private-mode failure still leaves the in-memory value usable for
     *  the current session. */
    persist(): void {
      if (typeof localStorage === 'undefined') return
      try {
        localStorage.setItem(
          SETTINGS_STORAGE_KEY,
          JSON.stringify({ tunAdvanced: this.tunAdvanced }),
        )
      } catch {
        /* not persistable in this environment — keep the in-memory value */
      }
    },
  },
})

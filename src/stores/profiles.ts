// ============================================================================
// stores/profiles.ts — Pinia store for profile / subscription management.
//
// Listens to `profile://list-changed` and `profile://reloaded` events emitted
// from the Rust side. Mutations are performed via `services/profile.ts` and
// then either awaited locally for the result, or refreshed from disk after
// the event to keep the index in sync.
// ============================================================================

import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  deleteProfile as delProfile,
  getActiveProfile as fetchActive,
  importProfileUrl as importUrl,
  importProfileFile as importFile,
  listProfiles as fetchList,
  saveProfile as saveYaml,
  setActiveProfile as activate,
  updateSubscription as updateOne,
  EVT_PROFILE_LIST_CHANGED,
  EVT_PROFILE_RELOADED,
} from '@/services/profile'
import { reloadConfig as clashReload } from '@/services/clash'
import type { ProfileMeta, ReloadResult } from '@/types/clash'

export const useProfilesStore = defineStore('profiles', () => {
  const profiles = ref<ProfileMeta[]>([])
  const activeId = ref<string | null>(null)
  const loading = ref(false)
  const lastError = ref<string | null>(null)
  const unlistens: UnlistenFn[] = []
  /** Per-id "is currently being updated" map (M6). */
  const updating = ref<Record<string, boolean>>({})

  // ---- computed -----------------------------------------------------------
  const active = computed<ProfileMeta | null>(
    () => profiles.value.find((p) => p.id === activeId.value) ?? null,
  )
  const hasProfiles = computed(() => profiles.value.length > 0)

  // ---- actions ------------------------------------------------------------
  async function refresh(): Promise<void> {
    loading.value = true
    lastError.value = null
    try {
      profiles.value = await fetchList()
      const a = await fetchActive()
      activeId.value = a?.id ?? null
    } catch (e) {
      lastError.value = e instanceof Error ? e.message : String(e)
    } finally {
      loading.value = false
    }
  }

  async function addFromUrl(url: string, name?: string): Promise<ProfileMeta> {
    const display = name && name.trim() ? name.trim() : deriveName(url)
    const meta = await importUrl(url, display)
    await refresh()
    return meta
  }

  async function addFromFile(path: string, name?: string): Promise<ProfileMeta> {
    const display = name && name.trim() ? name.trim() : path.split(/[\\/]/).pop() ?? 'Profile'
    const meta = await importFile(path, display)
    await refresh()
    return meta
  }

  async function pasteYaml(name: string, yaml: string): Promise<ProfileMeta> {
    const meta = await saveYaml(null, name, yaml)
    await refresh()
    return meta
  }

  async function remove(id: string): Promise<void> {
    await delProfile(id)
    if (activeId.value === id) activeId.value = null
    await refresh()
  }

  /**
   * Re-fetch an existing subscription (M6). Caller passes the id; the
   * profile must have a non-empty `url`. Returns the reload result so the
   * UI can surface "reloaded / staged / failed" outcomes.
   */
  async function updateProfile(id: string): Promise<ReloadResult> {
    updating.value = { ...updating.value, [id]: true }
    try {
      const result = await updateOne(id)
      // event listener will refresh the list; explicit refresh here covers
      // the rare case the event arrives before this returns.
      await refresh()
      return result
    } catch (e) {
      lastError.value = e instanceof Error ? e.message : String(e)
      throw e
    } finally {
      const next = { ...updating.value }
      delete next[id]
      updating.value = next
    }
  }

  /**
   * Activate a profile.
   *   1) Rust activates on disk (copy yaml -> config.yaml) and tries
   *      `PUT /configs?force=true` (or kernel restart fallback).
   *   2) For belt-and-suspenders, if the Rust call reported `reloaded` we
   *      still re-issue the reload from the frontend — this is harmless
   *      (mihomo reloads twice, second is a no-op) and keeps the code
   *      resilient if the Rust call raced with the sidecar shutting down.
   */
  async function activateProfile(id: string): Promise<ReloadResult> {
    const result = await activate(id)
    activeId.value = id
    // Best-effort frontend-side reload (matches `clash.ts reloadConfig(path)`)
    if (result.status === 'reloaded') {
      const p = profiles.value.find((x) => x.id === id)
      if (p) {
        try {
          await clashReload(p.file_path)
        } catch {
          /* swallow — Rust already succeeded, the 2nd reload is defensive */
        }
      }
    }
    await refresh()
    return result
  }

  // ---- event subscription -------------------------------------------------
  function attach(): void {
    if (unlistens.length) return
    void listen<ProfileMeta | string>(EVT_PROFILE_LIST_CHANGED, () => {
      void refresh()
    })
      .then((u) => unlistens.push(u))
      .catch(() => {})
    void listen<ProfileMeta>(EVT_PROFILE_RELOADED, (e) => {
      const p = e.payload
      if (p && typeof p === 'object' && 'id' in p) {
        activeId.value = p.id
      }
      void refresh()
    })
      .then((u) => unlistens.push(u))
      .catch(() => {})
  }

  function dispose(): void {
    for (const u of unlistens) u()
    unlistens.length = 0
  }

  // ---- utils --------------------------------------------------------------
  function deriveName(url: string): string {
    try {
      const u = new URL(url)
      // Use the first path segment or hostname as a stable, human-friendly name.
      const seg = u.pathname.split('/').filter(Boolean)[0]
      return seg || u.hostname || 'Subscription'
    } catch {
      return 'Subscription'
    }
  }

  return {
    profiles,
    activeId,
    active,
    hasProfiles,
    loading,
    lastError,
    updating,
    refresh,
    addFromUrl,
    addFromFile,
    pasteYaml,
    remove,
    updateProfile,
    activateProfile,
    attach,
    dispose,
  }
})

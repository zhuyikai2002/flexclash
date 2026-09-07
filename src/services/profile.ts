// ============================================================================
// profile.ts — Thin Tauri invoke wrapper for the `commands::profile` surface.
// The store is the *only* place these calls are made from the UI; this module
// exists to keep the store readable and to centralise the channel names.
// ============================================================================

import { invoke } from '@tauri-apps/api/core'
import type { ProfileMeta, ReloadResult } from '@/types/clash'

export async function listProfiles(): Promise<ProfileMeta[]> {
  return await invoke<ProfileMeta[]>('list_profiles')
}

export async function getActiveProfile(): Promise<ProfileMeta | null> {
  return await invoke<ProfileMeta | null>('get_active_profile')
}

export async function getProfileContent(id: string): Promise<string> {
  return await invoke<string>('get_profile_content', { id })
}

export async function saveProfile(
  id: string | null,
  name: string,
  content: string,
): Promise<ProfileMeta> {
  return await invoke<ProfileMeta>('save_profile', { id, name, content })
}

export async function deleteProfile(id: string): Promise<void> {
  await invoke('delete_profile', { id })
}

export async function importProfileUrl(url: string, name: string): Promise<ProfileMeta> {
  return await invoke<ProfileMeta>('import_profile_url', { url, name })
}

export async function importProfileFile(path: string, name: string): Promise<ProfileMeta> {
  return await invoke<ProfileMeta>('import_profile_file', { path, name })
}

/**
 * Re-fetch an existing subscription (M6) by id. The profile must have a
 * non-empty `url`. If it is the active one, mihomo is hot-reloaded.
 */
export async function updateSubscription(id: string): Promise<ReloadResult> {
  return await invoke<ReloadResult>('update_subscription', { id })
}

export async function setActiveProfile(id: string): Promise<ReloadResult> {
  return await invoke<ReloadResult>('set_active_profile', { id })
}

// Tauri event names — mirror `events.rs`.
export const EVT_PROFILE_LIST_CHANGED = 'profile://list-changed'
export const EVT_PROFILE_RELOADED = 'profile://reloaded'

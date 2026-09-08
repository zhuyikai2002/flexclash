// ============================================================================
// profile.ts — Thin Tauri invoke wrapper for the `commands::profile` surface.
// The store is the *only* place these calls are made from the UI; this module
// exists to keep the store readable and to centralise the channel names.
//
// `safeInvoke` is used so the renderer can boot in a plain browser
// preview without DevTools errors.
// ============================================================================

import { safeInvoke, safeInvokeOr } from '@/utils/tauri-bridge'
import type { ProfileMeta, ReloadResult } from '@/types/clash'

export async function listProfiles(): Promise<ProfileMeta[]> {
  return await safeInvokeOr<ProfileMeta[]>('list_profiles', [])
}

export async function getActiveProfile(): Promise<ProfileMeta | null> {
  return await safeInvokeOr<ProfileMeta | null>('get_active_profile', null)
}

export async function getProfileContent(id: string): Promise<string> {
  return await safeInvoke<string>('get_profile_content', { id })
}

export async function saveProfile(
  id: string | null,
  name: string,
  content: string,
): Promise<ProfileMeta> {
  return await safeInvoke<ProfileMeta>('save_profile', { id, name, content })
}

export async function deleteProfile(id: string): Promise<void> {
  await safeInvoke('delete_profile', { id })
}

export async function importProfileUrl(url: string, name: string): Promise<ProfileMeta> {
  return await safeInvoke<ProfileMeta>('import_profile_url', { url, name })
}

export async function importProfileFile(path: string, name: string): Promise<ProfileMeta> {
  return await safeInvoke<ProfileMeta>('import_profile_file', { path, name })
}

/**
 * Re-fetch an existing subscription (M6) by id. The profile must have a
 * non-empty `url`. If it is the active one, mihomo is hot-reloaded.
 */
export async function updateSubscription(id: string): Promise<ReloadResult> {
  return await safeInvoke<ReloadResult>('update_subscription', { id })
}

export async function setActiveProfile(id: string): Promise<ReloadResult> {
  return await safeInvoke<ReloadResult>('set_active_profile', { id })
}

/**
 * Patch a profile's display name (UTF-8).  Empty names are rejected
 * by the Rust side.  Emits `profile://list-changed`.
 */
export async function renameProfile(id: string, name: string): Promise<ProfileMeta> {
  return await safeInvoke<ProfileMeta>('rename_profile', { id, name })
}

/**
 * Hand the on-disk yaml to whatever is registered for `.yaml`
 * (Notepad, VS Code, …).  Windows-only path; no-op on other OSes.
 */
export async function openProfileInEditor(id: string): Promise<void> {
  await safeInvoke('open_profile_in_editor', { id })
}

/**
 * Pop the parent folder in Explorer with the file selected.
 * Windows-only path; no-op on other OSes.
 */
export async function revealProfileFile(id: string): Promise<void> {
  await safeInvoke('reveal_profile_file', { id })
}

// Tauri event names — mirror `events.rs`.
export const EVT_PROFILE_LIST_CHANGED = 'profile://list-changed'
export const EVT_PROFILE_RELOADED = 'profile://reloaded'

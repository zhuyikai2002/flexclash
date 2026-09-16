// ============================================================================
// profile.ts — Tauri invoke surface for the `commands::profile` module.
// The store is the *only* caller from the UI; this module keeps it readable
// and centralises the event-channel names.
//
// Wire types come from the generated bindings (`src/bindings.ts`).
// ============================================================================

import { commands, type ProfileMeta, type ReloadResult } from '@/bindings'
import { call, guardInTauri, inTauri } from '@/utils/tauri-bridge'

export async function listProfiles(): Promise<ProfileMeta[]> {
  if (!inTauri('list_profiles')) return []
  return call(commands.listProfiles())
}

export async function getActiveProfile(): Promise<ProfileMeta | null> {
  if (!inTauri('get_active_profile')) return null
  return call(commands.getActiveProfile())
}

export async function getProfileContent(id: string): Promise<string> {
  guardInTauri('get_profile_content')
  return call(commands.getProfileContent(id))
}

export async function saveProfile(
  id: string | null,
  name: string,
  content: string,
): Promise<ProfileMeta> {
  guardInTauri('save_profile')
  return call(commands.saveProfile(id, name, content))
}

export async function deleteProfile(id: string): Promise<void> {
  guardInTauri('delete_profile')
  await call(commands.deleteProfile(id))
}

export async function importProfileUrl(url: string, name: string): Promise<ProfileMeta> {
  guardInTauri('import_profile_url')
  return call(commands.importProfileUrl(url, name))
}

export async function importProfileFile(path: string, name: string): Promise<ProfileMeta> {
  guardInTauri('import_profile_file')
  return call(commands.importProfileFile(path, name))
}

/**
 * Re-fetch an existing subscription by id. The profile must have a
 * non-empty `url`. If it is the active one, mihomo is hot-reloaded.
 */
export async function updateSubscription(id: string): Promise<ReloadResult> {
  guardInTauri('update_subscription')
  return call(commands.updateSubscription(id))
}

export async function setActiveProfile(id: string): Promise<ReloadResult> {
  guardInTauri('set_active_profile')
  return call(commands.setActiveProfile(id))
}

/**
 * Patch a profile's display name (UTF-8).  Empty names are rejected
 * by the Rust side.  Emits `profile://list-changed`.
 */
export async function renameProfile(id: string, name: string): Promise<ProfileMeta> {
  guardInTauri('rename_profile')
  return call(commands.renameProfile(id, name))
}

/**
 * Hand the on-disk yaml to whatever is registered for `.yaml`
 * (Notepad, VS Code, …).  Windows-only path; no-op on other OSes.
 */
export async function openProfileInEditor(id: string): Promise<void> {
  guardInTauri('open_profile_in_editor')
  await call(commands.openProfileInEditor(id))
}

/**
 * Pop the parent folder in Explorer with the file selected.
 * Windows-only path; no-op on other OSes.
 */
export async function revealProfileFile(id: string): Promise<void> {
  guardInTauri('reveal_profile_file')
  await call(commands.revealProfileFile(id))
}

// Tauri event names — mirror `events.rs`.
export const EVT_PROFILE_LIST_CHANGED = 'profile://list-changed'
export const EVT_PROFILE_RELOADED = 'profile://reloaded'

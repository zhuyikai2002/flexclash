// ============================================================================
// commands/profile.rs — Tauri command surface for profile / subscription
// management. Every entry point here is callable from the Vue frontend via
// `@tauri-apps/api/core::invoke`.
// ============================================================================

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::config::profile as profile_ops;
use crate::config::profile::ProfileStorage;
use crate::config::subscription as sub_ops;
use crate::core::sidecar::{self, SidecarHandle};
use crate::error::AppError;

type CmdResult<T> = Result<T, AppError>;

// ============================================================================
// Wire types (extra result shapes exposed to the frontend)
// ============================================================================

#[derive(Debug, Serialize)]
pub struct ReloadResult {
    pub status: ReloadStatus,
    pub detail: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReloadStatus {
    /// mihomo was running and reload succeeded.
    Reloaded,
    /// mihomo is not running; the active yaml has been staged for next start.
    Staged,
    /// mihomo is running; reload failed AND restart fallback also failed.
    Failed,
}

// ============================================================================
// Commands
// ============================================================================

#[tauri::command]
pub fn list_profiles<R: Runtime>(app: AppHandle<R>) -> CmdResult<Vec<profile_ops::ProfileMeta>> {
    let storage = storage_for(&app)?;
    profile_ops::list_profiles(&storage)
}

#[tauri::command]
pub fn get_active_profile<R: Runtime>(
    app: AppHandle<R>,
) -> CmdResult<Option<profile_ops::ProfileMeta>> {
    let storage = storage_for(&app)?;
    profile_ops::get_active_profile(&storage)
}

#[tauri::command]
pub fn get_profile_content<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> CmdResult<String> {
    let storage = storage_for(&app)?;
    let path = storage.profile_yaml(&id);
    std::fs::read_to_string(&path)
        .map_err(|e| AppError::Io(format!("read {}: {e}", path.display())))
}

#[tauri::command]
pub fn save_profile<R: Runtime>(
    app: AppHandle<R>,
    id: Option<String>,
    name: String,
    content: String,
) -> CmdResult<profile_ops::ProfileMeta> {
    let storage = storage_for(&app)?;
    let meta = profile_ops::save_profile(&storage, id, name, content, String::new())?;
    let _ = app.emit(
        crate::events::PROFILE_LIST_CHANGED,
        &meta,
    );
    Ok(meta)
}

#[tauri::command]
pub fn delete_profile<R: Runtime>(app: AppHandle<R>, id: String) -> CmdResult<()> {
    let storage = storage_for(&app)?;
    profile_ops::delete_profile(&storage, &id)?;
    let _ = app.emit(crate::events::PROFILE_LIST_CHANGED, &id);
    Ok(())
}

#[tauri::command]
pub async fn import_profile_url<R: Runtime>(
    app: AppHandle<R>,
    url: String,
    name: String,
) -> CmdResult<profile_ops::ProfileMeta> {
    let _ = app.emit(
        crate::events::KERNEL_LOG,
        format!("[subscription] fetching {url}"),
    );

    let (body, user_info) = sub_ops::fetch_subscription(&url, None).await.map_err(|e| {
        let msg = format!("[subscription] fetch failed: {e}");
        let _ = app.emit(crate::events::KERNEL_LOG, &msg);
        e
    })?;

    let _ = app.emit(
        crate::events::KERNEL_LOG,
        format!(
            "[subscription] ok ({} bytes), used={:?}/total={:?}, expire={:?}",
            body.len(),
            user_info.download.or(user_info.upload),
            user_info.total,
            user_info.expire_at
        ),
    );

    let storage = storage_for(&app)?;
    let mut meta =
        profile_ops::save_profile(&storage, None, name.clone(), body, url.clone())?;

    // Attach subscription usage info, if reported.
    if user_info.total.is_some() || user_info.download.is_some() {
        meta.used_bytes = user_info.download;
        meta.remaining_bytes = match (user_info.total, user_info.download) {
            (Some(t), Some(d)) => Some(t.saturating_sub(d)),
            _ => None,
        };
        meta.total_bytes = user_info.total;
        meta.expire_at = user_info.expire_at;
        // Re-upsert so the index file sees the updated meta.
        let _ = storage.upsert_meta(meta.clone())?;
    }

    let _ = app.emit(crate::events::PROFILE_LIST_CHANGED, &meta);
    Ok(meta)
}

#[tauri::command]
pub fn import_profile_file<R: Runtime>(
    app: AppHandle<R>,
    path: String,
    name: String,
) -> CmdResult<profile_ops::ProfileMeta> {
    let content = std::fs::read_to_string(&path)
        .map_err(|e| AppError::Io(format!("read {}: {e}", path)))?;
    let storage = storage_for(&app)?;
    let meta = profile_ops::save_profile(&storage, None, name, content, String::new())?;
    let _ = app.emit(crate::events::PROFILE_LIST_CHANGED, &meta);
    Ok(meta)
}

/// Re-fetch an existing subscription profile (M6).
///
/// Fetches the original `url` (must be non-empty), sanitises the yaml,
/// preserves the profile `id` so a subsequent `set_active_profile` is
/// unaffected, and refreshes the `subscription-userinfo` quota fields.
/// If the updated profile is the active one we additionally hot-reload
/// mihomo so the new node list takes effect immediately.
#[tauri::command]
pub async fn update_subscription<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> CmdResult<ReloadResult> {
    let storage = storage_for(&app)?;
    let idx = storage.read_index()?;
    let Some(mut meta) = idx.profiles.iter().find(|p| p.id == id).cloned() else {
        return Err(AppError::Config(format!("profile id not found: {id}")));
    };
    if meta.url.trim().is_empty() {
        return Err(AppError::Config(
            "profile has no source URL — cannot update".into(),
        ));
    }
    let url = meta.url.clone();

    let _ = app.emit(
        crate::events::KERNEL_LOG,
        format!("[subscription] updating '{name}' from {url}", name = meta.name),
    );

    // 1) Fetch.
    let (body, user_info) = sub_ops::fetch_subscription(&url, None)
        .await
        .map_err(|e| AppError::Subscription(e.to_string()))?;

    // 2) Persist. Pass `id` to overwrite the existing profile on disk.
    let mut updated =
        profile_ops::save_profile(&storage, Some(id.clone()), meta.name.clone(), body, url.clone())?;

    // 3) Update quota fields from the new header.
    if user_info.total.is_some() || user_info.download.is_some() {
        updated.used_bytes = user_info.download;
        updated.remaining_bytes = match (user_info.total, user_info.download) {
            (Some(t), Some(d)) => Some(t.saturating_sub(d)),
            _ => None,
        };
        updated.total_bytes = user_info.total;
        updated.expire_at = user_info.expire_at;
        let _ = storage.upsert_meta(updated.clone())?;
    }

    // 4) If this is the active profile, hot-reload mihomo.
    let is_active = idx.active_id.as_deref() == Some(id.as_str());
    if is_active {
        // Re-activate so the on-disk active path is updated; this also
        // returns the (now-updated) meta. Then ask mihomo to reload.
        let reactivated = profile_ops::activate_profile(&storage, &id)?;
        meta = reactivated;
        let sidecar = app.state::<SidecarHandle>();
        if sidecar.state() == sidecar::KernelState::Running {
            let rel = reload_via_controller(&meta.file_path).await;
            match rel {
                Ok(()) => {
                    let _ = app.emit(
                        crate::events::KERNEL_LOG,
                        format!("[subscription] hot-reload of '{}' ok", meta.name),
                    );
                    let _ = app.emit(crate::events::PROFILE_RELOADED, &meta);
                    let _ = app.emit(crate::events::PROFILE_LIST_CHANGED, &updated);
                    return Ok(ReloadResult {
                        status: ReloadStatus::Reloaded,
                        detail: "updated + reloaded".into(),
                    });
                }
                Err(e) => {
                    let msg = format!("[subscription] reload after update failed ({e}); restarting");
                    let _ = app.emit(crate::events::KERNEL_LOG, &msg);
                    let handle = (*sidecar).clone();
                    if sidecar::restart(&app, handle).await.is_ok() {
                        let _ = app.emit(crate::events::PROFILE_RELOADED, &meta);
                        let _ = app.emit(crate::events::PROFILE_LIST_CHANGED, &updated);
                        return Ok(ReloadResult {
                            status: ReloadStatus::Reloaded,
                            detail: "updated + restart fallback".into(),
                        });
                    }
                }
            }
        }
    }

    let _ = app.emit(crate::events::PROFILE_LIST_CHANGED, &updated);
    Ok(ReloadResult {
        status: if is_active { ReloadStatus::Staged } else { ReloadStatus::Reloaded },
        detail: if is_active {
            "subscription updated (kernel not running)".into()
        } else {
            "subscription updated".into()
        },
    })
}

#[tauri::command]
pub async fn set_active_profile<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> CmdResult<ReloadResult> {
    let storage = storage_for(&app)?;

    // 1) Copy the profile yaml over the active config path.
    let meta = profile_ops::activate_profile(&storage, &id)?;
    let _ = app.emit(crate::events::PROFILE_LIST_CHANGED, &meta);

    // 2) If the kernel is running, ask mihomo to reload from disk.
    let sidecar = app.state::<SidecarHandle>();
    let is_running = sidecar.state() == sidecar::KernelState::Running;
    if !is_running {
        let _ = app.emit(
            crate::events::KERNEL_LOG,
            format!(
                "[profile] activated '{}' ({}), but kernel is not running — staged for next start",
                meta.name, meta.file_path
            ),
        );
        return Ok(ReloadResult {
            status: ReloadStatus::Staged,
            detail: "kernel not running".into(),
        });
    }

    // 3) Try hot reload via the mihomo external-controller.
    let rel = reload_via_controller(&meta.file_path).await;
    match rel {
        Ok(()) => {
            let _ = app.emit(
                crate::events::KERNEL_LOG,
                format!("[profile] mihomo reloaded '{}' via /configs", meta.name),
            );
            let _ = app.emit(crate::events::PROFILE_RELOADED, &meta);
            Ok(ReloadResult {
                status: ReloadStatus::Reloaded,
                detail: "PUT /configs?force=true ok".into(),
            })
        }
        Err(e) => {
            // 4) Fallback: restart the kernel.
            let msg = format!("[profile] reload via API failed ({e}); restarting kernel");
            let _ = app.emit(crate::events::KERNEL_LOG, &msg);
            // Deref `tauri::State<SidecarHandle>` to the inner handle and clone.
            let handle = (*sidecar).clone();
            match sidecar::restart(&app, handle).await {
                Ok(()) => {
                    let _ = app.emit(crate::events::PROFILE_RELOADED, &meta);
                    Ok(ReloadResult {
                        status: ReloadStatus::Reloaded,
                        detail: "fallback: restart succeeded".into(),
                    })
                }
                Err(e2) => {
                    let msg = format!("[profile] restart fallback failed: {e2}");
                    let _ = app.emit(crate::events::KERNEL_LOG, &msg);
                    Ok(ReloadResult {
                        status: ReloadStatus::Failed,
                        detail: format!("reload: {e}; restart: {e2}"),
                    })
                }
            }
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn storage_for<R: Runtime>(app: &AppHandle<R>) -> CmdResult<ProfileStorage> {
    let work_dir = sidecar::work_dir_for(app)?;
    Ok(ProfileStorage::new(&work_dir))
}

/// `PUT http://127.0.0.1:9091/configs?force=true` with body `{"path":"..."}`.
async fn reload_via_controller(file_path: &str) -> Result<(), String> {
    let url = format!("http://{}/configs?force=true", profile_ops::RESERVED_CONTROLLER);
    let body = serde_json::json!({ "path": file_path }).to_string();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| format!("build http client: {e}"))?;
    let resp = client
        .put(&url)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("PUT {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("PUT {} -> HTTP {}", url, resp.status()));
    }
    Ok(())
}

// ============================================================================
// File-system utilities
// ============================================================================

/// `rename_profile` — Patch a profile's display `name` in the index file.
/// Used by the UI's card-level "rename" action. Empty / whitespace-only
/// names are rejected with `AppError::Config`.  The yaml file is left
/// alone (its file name remains the id-derived directory), but the
/// `ProfileIndex` is rewritten so the list reflects the new label.
#[tauri::command]
pub fn rename_profile<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    name: String,
) -> CmdResult<profile_ops::ProfileMeta> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::Config("profile name cannot be empty".into()));
    }
    if trimmed.len() > 80 {
        return Err(AppError::Config("profile name too long (>80 chars)".into()));
    }
    let storage = storage_for(&app)?;
    let mut idx = storage.read_index()?;
    let Some(slot) = idx.profiles.iter_mut().find(|p| p.id == id) else {
        return Err(AppError::Config(format!("profile id not found: {id}")));
    };
    slot.name = trimmed.to_string();
    slot.updated_at = chrono::Utc::now();
    storage.write_index(&idx)?;
    let updated = idx
        .profiles
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .ok_or_else(|| AppError::Config("profile disappeared during rename".into()))?;
    let _ = app.emit(crate::events::PROFILE_LIST_CHANGED, &updated);
    Ok(updated)
}

/// `open_profile_in_editor` — Hand the on-disk yaml to whatever the
/// user has registered for `.yaml` (Notepad, VS Code, etc.).  We use
/// `ShellExecuteW` with the `edit` verb when the association supports
/// it, falling back to the `open` verb.  No-op stub on non-Windows.
#[tauri::command]
pub fn open_profile_in_editor<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> CmdResult<()> {
    let storage = storage_for(&app)?;
    let path = storage.profile_yaml(&id);
    if !path.exists() {
        return Err(AppError::Config(format!(
            "profile yaml missing on disk: {}",
            path.display()
        )));
    }
    let _ = app; // unused on Windows
    open_path_external(&path, true)?;
    Ok(())
}

/// `reveal_profile_file` — Open the profile's containing folder in
/// Explorer with the file already selected. Implemented via the
/// Win32 `SHOpenFolderAndSelectItems` shell API, which is exactly
/// what `explorer.exe /select,"path"` does under the hood.  Stub on
/// non-Windows: prints the parent dir to stderr.
#[tauri::command]
pub fn reveal_profile_file<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> CmdResult<()> {
    let storage = storage_for(&app)?;
    let path = storage.profile_yaml(&id);
    if !path.exists() {
        return Err(AppError::Config(format!(
            "profile yaml missing on disk: {}",
            path.display()
        )));
    }
    let _ = app;
    reveal_in_explorer(&path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Platform shims
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn open_path_external(path: &std::path::Path, prefer_edit_verb: bool) -> CmdResult<()> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    // Hide the transient console for `cmd /C start` (no black flash).
    let create_no_window: u32 = 0x0800_0000;

    // Prefer `cmd /c start "" "<file>"` for the `open` verb — Windows
    // resolves the file association (Notepad / VS Code / whatever the
    // user installed for `.yaml`).  For the `edit` verb we fall back
    // to the default `open` because most editors don't register an
    // "edit" verb; the `open` verb has the same end-user result.
    //
    // The empty "" after `start` is the window title slot, not a path
    // — `start` interprets the first quoted arg as a title if it is
    // always a literal, but since we want a path there we pass "" to
    // explicitly skip the title.  Without this Windows can mangle
    // paths that start with "/".
    let verb = if prefer_edit_verb { "edit" } else { "open" };
    if verb == "open" {
        let path_arg = path.as_os_str().to_string_lossy().into_owned();
        let status = Command::new("cmd").creation_flags(create_no_window)
            .args(["/C", "start", "", &path_arg])
            .status()
            .map_err(|e| AppError::Shell(format!("cmd /C start {}: {e}", path.display())))?;
        if !status.success() {
            return Err(AppError::Shell(format!(
                "cmd /C start {} exited with {}",
                path.display(),
                status
            )));
        }
    } else {
        // Fallback: run the same `open` path.
        let path_arg = path.as_os_str().to_string_lossy().into_owned();
        let status = Command::new("cmd").creation_flags(create_no_window)
            .args(["/C", "start", "", &path_arg])
            .status()
            .map_err(|e| AppError::Shell(format!("cmd /C start {}: {e}", path.display())))?;
        if !status.success() {
            return Err(AppError::Shell(format!(
                "cmd /C start {} exited with {}",
                path.display(),
                status
            )));
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn open_path_external(path: &std::path::Path, _prefer_edit_verb: bool) -> CmdResult<()> {
    eprintln!("[profile] open_path_external stub on non-Windows: {}", path.display());
    Ok(())
}

#[cfg(target_os = "windows")]
fn reveal_in_explorer(path: &std::path::Path) -> CmdResult<()> {
    use std::process::Command;

    // `explorer.exe /select,"<path>"` opens the parent folder in a
    // new window with the file already highlighted.  This is the
    // exact same call `SHOpenFolderAndSelectItems` makes under the
    // hood, but it doesn't need the COM-style PIDL plumbing or any
    // extra `windows` crate features.  The `/n` switch keeps the
    // call from accidentally opening an existing Explorer window.
    let arg = format!("/select,{}", path.display());
    let status = Command::new("explorer.exe")
        .arg(&arg)
        .status()
        .map_err(|e| AppError::Shell(format!("explorer.exe {arg}: {e}")))?;
    if !status.success() {
        // explorer.exe often returns 1 even on success; treat any
        // "the process ran" as success, only bail on OS-level error.
        eprintln!(
            "[profile] explorer.exe {} returned {} (usually benign)",
            arg, status
        );
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn reveal_in_explorer(path: &std::path::Path) -> CmdResult<()> {
    eprintln!("[profile] reveal_in_explorer stub on non-Windows: {}", path.display());
    Ok(())
}

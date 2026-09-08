// ============================================================================
// commands/reset.rs — Phase 8 "Reset Application" command.
//
// Destructive, but scoped.  After this command returns the on-disk
// state of FlexClash is exactly what a fresh install would have:
//
//   work_dir/
//     config.yaml             → bundled default_mihomo.yaml
//     index.json              → { "active_id": null, "profiles": [] }
//     profiles/               → DELETED (all subscription caches gone)
//     history.db*              → DELETED (SQLite file + WAL/SHM)
//
// The mihomo child, the TUN device, and the system proxy are all
// torn down first.  The actual app restart is signalled via the
// `app://reset-completed` event so the frontend can:
//   1. clear its localStorage (locale, closeBehavior, kernel cache)
//   2. show a "Reset complete — restarting" toast
//   3. invoke `app.exit(0)` 1s later
// ============================================================================

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};

use crate::config::profile::ProfileStorage;
use crate::core::sidecar::{self, SidecarHandle, EXPECTED_CONTROLLER_PORT};
use crate::core::tun::TunManager;
use crate::error::Result;
use crate::events::APP_RESET_COMPLETED;
use crate::proxy;

type CmdResult<T> = Result<T>;

#[derive(Debug, Clone, Serialize)]
pub struct ResetReport {
    /// Items removed (for the success modal).
    pub removed_profiles: usize,
    pub removed_history_bytes: u64,
    pub swept_routes: bool,
    pub proxy_disabled: bool,
    pub kernel_was_running: bool,
    pub tun_was_on: bool,
    pub detail: String,
}

#[tauri::command]
pub async fn reset_application<R: Runtime>(
    app: AppHandle<R>,
    handle: State<'_, SidecarHandle>,
    tun: State<'_, TunManager>,
) -> CmdResult<ResetReport> {
    let mut report = ResetReport {
        removed_profiles: 0,
        removed_history_bytes: 0,
        swept_routes: false,
        proxy_disabled: false,
        kernel_was_running: handle.state() == crate::core::sidecar::KernelState::Running,
        tun_was_on: tun.status().state == crate::core::tun::TunState::On,
        detail: String::new(),
    };

    // ----- 1. Disable system proxy (best effort) -------------------------
    match proxy::disable_system_proxy() {
        Ok(()) => {
            report.proxy_disabled = true;
            let _ = app.emit(
                crate::events::SYSTEM_PROXY_CHANGED,
                serde_json::json!({"enabled": false, "port": Option::<u16>::None, "source": "reset"}),
            );
        }
        Err(e) => {
            eprintln!("[reset] disable_system_proxy failed: {e}");
        }
    }

    // ----- 2. Stop TUN (best effort, rolls back config + sweeps routes) --
    if report.tun_was_on {
        let work_dir = handle.work_dir();
        let storage = ProfileStorage::new(&work_dir);
        if let Err(e) = tun.disable(&app, &storage) {
            eprintln!("[reset] tun.disable failed: {e}");
        } else {
            report.swept_routes = true;
        }
    }

    // ----- 3. Stop the mihomo sidecar (best effort) ----------------------
    if report.kernel_was_running {
        if let Err(e) = sidecar::stop(handle.inner().clone()) {
            eprintln!("[reset] sidecar.stop failed: {e}");
        }
        // Give the OS a moment to release the controller port.
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    }

    // Belt-and-suspenders: hard-kill any orphaned mihomo*.exe the kernel
    // child might have left behind (e.g. previously crashed on restart).
    sidecar::hard_cleanup();

    // ----- 4. Wipe the work dir contents ----------------------------------
    let work_dir = sidecar::work_dir_for(&app)?;
    let storage = ProfileStorage::new(&work_dir);

    // 4a. profiles/
    if storage.profiles_dir().exists() {
        // Count before delete so the report can show "n profiles removed".
        if let Ok(idx) = storage.read_index() {
            report.removed_profiles = idx.profiles.len();
        }
        if let Err(e) = std::fs::remove_dir_all(storage.profiles_dir()) {
            eprintln!("[reset] remove profiles/: {e}");
        }
    }
    // Re-create the empty profiles dir so subsequent commands don't NPE.
    let _ = storage.ensure_dirs();

    // 4b. history.db + WAL/SHM (SQLite)
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let p = work_dir.join(format!("history{suffix}"));
        if p.exists() {
            if let Ok(meta) = std::fs::metadata(&p) {
                report.removed_history_bytes =
                    report.removed_history_bytes.saturating_add(meta.len());
            }
            if let Err(e) = std::fs::remove_file(&p) {
                eprintln!("[reset] remove {}: {e}", p.display());
            }
        }
    }

    // 4c. log files (if any)
    if let Ok(rd) = std::fs::read_dir(&work_dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("mihomo") && (name.ends_with(".log") || name.ends_with(".txt"))
                {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }

    // 4d. active config → bundled default
    let bundled = include_str!("../../resources/default_mihomo.yaml");
    let config_path = work_dir.join("config.yaml");
    if let Err(e) = std::fs::write(&config_path, bundled) {
        eprintln!("[reset] rewrite config.yaml: {e}");
    }

    // 4e. profile index → empty
    let _ = storage.write_index(&crate::config::profile::ProfileIndex::default());

    // ----- 5. Announce completion ----------------------------------------
    let controller = format!("http://127.0.0.1:{EXPECTED_CONTROLLER_PORT}");
    report.detail = format!(
        "Reset complete: {} profile(s) removed, {:.1} KB of history wiped, default config at {}, controller URL {controller}",
        report.removed_profiles,
        report.removed_history_bytes as f64 / 1024.0,
        config_path.display(),
    );
    eprintln!("[reset] {}", report.detail);
    let _ = app.emit(APP_RESET_COMPLETED, &report);

    Ok(report)
}

// ============================================================================
// commands/reset.rs — Phase 8 "Reset Application" command.
//
// Destructive, but scoped.  After this command returns the on-disk
// state of FlexClash is exactly what a fresh install would have:
//
//   <app_local_data>/com.flexclash.app/
//     history.db*             → DELETED (SQLite file + WAL/SHM)
//     mihomo/
//       config.yaml           → bundled default_mihomo.yaml
//       index.json            → { "active_id": null, "profiles": [] }
//       profiles/             → DELETED (all subscription caches gone)
//       cache.db*             → DELETED (selected group + fake-ip state)
//       proxies/ rules/       → DELETED (cached proxy/rule providers)
//       dashboard/            → DELETED (downloaded web UI)
//       *.log *.txt           → DELETED (kernel logs, incl. runtime.log)
//
// Two layout details are load-bearing and were both wrong before:
//
//   1. `history.db` does **not** live in the mihomo work dir. The store
//      resolves it against `app_local_data_dir()` itself, i.e. one level
//      up. The wipe used to run `work_dir.join("history.db*")`, which
//      matched nothing, so traffic history survived every reset.
//   2. `cache.db` is what makes a reset *look* ineffective. mihomo writes
//      the selected node of every group plus the fake-ip pool there (the
//      active config sets `profile.store-selected` / `store-fake-ip`), so
//      leaving it behind means the old group selections come straight back
//      on the next start. It was never touched.
//
// Both are resolved through their owning module (`store::db_path_for`) so
// the paths cannot drift apart again.
//
// The mihomo child, the TUN device, and the system proxy are all torn
// down first.  The actual app restart is signalled via the
// `app://reset-completed` event so the frontend can:
//   1. clear its localStorage + in-memory stores (locale, theme,
//      closeBehavior, cached proxies/groups)
//   2. show a "Reset complete — restarting" toast
//   3. invoke `app.exit(0)` 1s later
//
// Note on "runtime route temp files": `core::route_guard` shells out to
// `route` / `netsh` directly and never writes a temp file, so there is
// nothing of that kind to clean up here.
// ============================================================================

use std::path::Path;

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

/// Delete `path` if it is a file, returning the bytes reclaimed.
/// Missing / non-file paths are a no-op — every wipe below is best-effort
/// and must never abort the reset halfway through.
fn remove_file_if_present(path: &Path) -> u64 {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => {
            if let Err(e) = std::fs::remove_file(path) {
                eprintln!("[reset] remove {}: {e}", path.display());
                return 0;
            }
            meta.len()
        }
        _ => 0,
    }
}

/// Recursive byte count, used only to report how much a cache dir freed.
fn dir_size(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in rd.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        total += if meta.is_dir() {
            dir_size(&entry.path())
        } else {
            meta.len()
        };
    }
    total
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

    // Resolve the canonical work dir ONCE, from the Tauri path API — not from
    // `SidecarHandle::work_dir()`, which can only guess before the kernel has
    // started. This is the directory mihomo runs against (`-d`), so every
    // wipe below is relative to it.
    let work_dir = sidecar::work_dir_for(&app)?;
    let storage = ProfileStorage::new(&work_dir);

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
    // child might have left behind (e.g. previously crashed on restart), and
    // any TUN kernel this process no longer has a PID for.
    //
    // ⚠️ This is the one deliberately broad step, and it is *not* scoped to
    // our own kernel: `taskkill /IM mihomo*.exe` also matches a second
    // Clash-family client's kernel (Clash Party, etc.). It is kept because
    // after an app restart the elevated TUN kernel is untracked and this is
    // the only reliable way to reap it. See the reset notes in the repo for
    // the open question about scoping it.
    sidecar::hard_cleanup();

    // ----- 4. Wipe the work dir contents ----------------------------------

    // 4a. profiles/ — every subscription cache. Count before delete so the
    //     report can say "n profiles removed".
    if storage.profiles_dir().is_dir() {
        if let Ok(idx) = storage.read_index() {
            report.removed_profiles = idx.profiles.len();
        }
        if let Err(e) = std::fs::remove_dir_all(storage.profiles_dir()) {
            eprintln!("[reset] remove profiles/: {e}");
        }
    }
    // Re-create the empty profiles dir so subsequent commands don't NPE.
    let _ = storage.ensure_dirs();

    // 4b. mihomo's own runtime state — the reason a "reset" used to change
    //     nothing the user could see.
    let mut removed_cache_bytes: u64 = 0;
    // cache.db holds the selected node of every proxy group plus the fake-ip
    // pool. Survivors here come back as "my old group selections are still
    // there after a reset".
    for suffix in ["cache.db", "cache.db-shm", "cache.db-wal"] {
        removed_cache_bytes += remove_file_if_present(&work_dir.join(suffix));
    }
    // Provider / rule caches: a cached subscription payload (proxies/) and a
    // cached rule set (rules/) are both stale the moment the user resets.
    for name in ["proxies", "rules", "dashboard"] {
        let dir = work_dir.join(name);
        if dir.is_dir() {
            removed_cache_bytes += dir_size(&dir);
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                eprintln!("[reset] remove {name}/: {e}");
            }
        }
    }

    // 4c. Kernel logs. The previous filter only matched `mihomo*`, which
    //     missed `runtime.log` — the file the app itself appends every
    //     kernel/init line to (see `SidecarHandle::push_log`).
    let mut removed_log_bytes: u64 = 0;
    if let Ok(rd) = std::fs::read_dir(&work_dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".log") || lower.ends_with(".txt") {
                removed_log_bytes += remove_file_if_present(&path);
            }
        }
    }

    // 4d. Traffic-history SQLite — resolved through its owner so the path
    //     cannot drift away from where `store::open` actually writes it.
    match crate::store::db_path_for(&app) {
        Ok(db) => {
            let stem = db.to_string_lossy().into_owned();
            for suffix in ["", "-wal", "-shm", "-journal"] {
                report.removed_history_bytes = report
                    .removed_history_bytes
                    .saturating_add(remove_file_if_present(Path::new(&format!("{stem}{suffix}"))));
            }
        }
        Err(e) => eprintln!("[reset] history db path: {e}"),
    }

    // 4e. active config → bundled default
    let bundled = include_str!("../../resources/default_mihomo.yaml");
    let config_path = work_dir.join("config.yaml");
    if let Err(e) = std::fs::write(&config_path, bundled) {
        eprintln!("[reset] rewrite config.yaml: {e}");
    }

    // 4f. profile index → empty
    let _ = storage.write_index(&crate::config::profile::ProfileIndex::default());

    // ----- 5. Announce completion ----------------------------------------
    let controller = format!("http://127.0.0.1:{EXPECTED_CONTROLLER_PORT}");
    report.detail = format!(
        "Reset complete: {} profile(s) removed, {:.1} KB of history wiped, \
         {:.1} KB of kernel cache (cache.db / providers / rules / dashboard) removed, \
         {:.1} KB of logs removed, default config at {}, controller URL {controller}",
        report.removed_profiles,
        report.removed_history_bytes as f64 / 1024.0,
        removed_cache_bytes as f64 / 1024.0,
        removed_log_bytes as f64 / 1024.0,
        config_path.display(),
    );
    eprintln!("[reset] {}", report.detail);
    let _ = app.emit(APP_RESET_COMPLETED, &report);

    Ok(report)
}

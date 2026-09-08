//! FlexClash backend library crate.
//!
//! Wires plugins, managed state and `#[tauri::command]` handlers.
//! Mobile entry point is delegated to `main.rs`.

mod commands;
pub mod config;
pub mod core;
pub mod error;
pub mod events;
pub mod proxy;
pub mod store;
pub mod tray;

use crate::core::shutdown::ExitFlag;
use crate::core::sidecar::SidecarHandle;
use crate::core::startup::{self, SilentFlag};
use crate::core::tun::TunManager;
use crate::store::queries::HistoryDb;
use tauri::{Emitter, Manager, WindowEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Parse silent flag *before* the Tauri builder so the managed state
    // is correct on first read from any setup hook.
    let silent = startup::parse_silent_flag();
    let silent_flag = SilentFlag::default();
    silent_flag.set(silent);

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            // `tauri-plugin-autostart` requires a launcher for init. We use
            // MacosLauncher::LaunchAgent as the generic default; on Windows
            // the plugin ignores it and writes the registry entry instead.
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--silent"]),
        ))
        .manage(SidecarHandle::new())
        .manage(ExitFlag::default())
        .manage(silent_flag)
        .manage(TunManager::new())
        // -----------------------------------------------------------------
        // WINDOW-LEVEL DRAG-AND-DROP INTERCEPT
        // -----------------------------------------------------------------
        // We catch `WindowEvent::DragDrop` on the main thread of the
        // Tauri runtime — this sits *below* the WebView2 child
        // window, so the JS layer never sees a DOM event and the
        // well-known WebView2 "no drop zone" issues go away.  We then
        // re-broadcast three plain Tauri events the renderer can
        // `listen()` on.  Filtering by extension lives in Rust so the
        // frontend never even has to check.
        // -----------------------------------------------------------------
        .on_window_event(|window, event| {
            if let WindowEvent::DragDrop(drag) = event {
                match drag {
                    tauri::DragDropEvent::Enter { paths, .. } => {
                        // Show the overlay only if at least one of the
                        // currently-hovered files is something we
                        // would actually import.
                        let any_yaml = paths.iter().any(|p| is_yaml_path(p));
                        if any_yaml {
                            let _ = window.emit(events::NATIVE_FILE_DRAG_ENTER, ());
                        }
                    }
                    tauri::DragDropEvent::Over { .. } => {
                        // No-op: `enter` already flipped the overlay.
                    }
                    tauri::DragDropEvent::Drop { paths, .. } => {
                        let yaml_paths: Vec<String> = paths
                            .iter()
                            .filter(|p| is_yaml_path(p))
                            .map(|p| p.to_string_lossy().to_string())
                            .collect();
                        if !yaml_paths.is_empty() {
                            println!(
                                "[DragDrop] hit {} yaml file(s): {:?}",
                                yaml_paths.len(),
                                yaml_paths
                            );
                            let _ = window.emit(events::NATIVE_FILE_DROP, &yaml_paths);
                        }
                        // Always clear the overlay on drop, even if no
                        // file was accepted (e.g. user dropped a .png).
                        let _ = window.emit(events::NATIVE_FILE_DRAG_LEAVE, ());
                    }
                    tauri::DragDropEvent::Leave => {
                        let _ = window.emit(events::NATIVE_FILE_DRAG_LEAVE, ());
                    }
                    _ => {}
                }
            }
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            crate::core::shutdown::install(&handle);
            // M9: hand the AppHandle to the elevate module so the
            // TUN manager can resolve the mihomo work dir at any time.
            crate::core::elevate::install(handle.clone());

            // M10: open the history DB at
            //   %LOCALAPPDATA%\com.flexclash.app\history.db
            // and register it as managed state so the Tauri commands
            // can borrow `State<'_, HistoryDb>`. Then spawn the
            // sampler + pruner background tasks.
            match crate::store::open(&handle) {
                Ok(db) => {
                    // Register BEFORE spawning so a `get_traffic_history`
                    // call from the UI finds the state even if the
                    // sampler hasn't written its first row yet.
                    handle.manage(db.clone());
                    match crate::store::spawn_sampler(&handle, db) {
                        Ok(_h) => eprintln!("[startup] history sampler started"),
                        Err(e) => eprintln!("[startup] history sampler spawn failed: {e}"),
                    }
                }
                Err(e) => {
                    // Don't brick the app on a bad path — fall back to
                    // an in-memory DB and log loudly. Data is lost on
                    // restart but the UI still works.
                    eprintln!("[startup] history db open failed: {e}, using in-memory fallback");
                    if let Ok(memdb) = HistoryDb::open_in_memory() {
                        handle.manage(memdb.clone());
                        let _ = crate::store::spawn_sampler(&handle, memdb);
                    }
                }
            }

            // M7: apply native backdrop (Mica on Win11, Acrylic on Win10).
            #[cfg(target_os = "windows")]
            if let Some(win) = handle.get_webview_window("main") {
                crate::core::startup::apply_native_backdrop(&win);

                // UIPI: relax drag-and-drop / OLE data-transfer message
                // filters on the top-level HWND so that a non-elevated
                // Explorer (which is the common case for end users) can
                // still drop files onto us when we are running elevated
                // (TUN mode / dev as admin).  This is a no-op when the
                // process integrity level is already the same as the
                // source.
                if let Ok(hwnd) = win.hwnd() {
                    crate::core::uipi::relax_drag_drop_for_window(hwnd.0 as isize);
                }

                // Silent boot: keep window hidden, only tray is visible.
                if silent {
                    eprintln!("[startup] --silent detected, hiding main window");
                    let _ = win.hide();
                }
            }

            // M7: defensive route sweep (M9 will fill in real work).
            let sweep = crate::core::startup::sweep_residual_routes();
            if !sweep.ok {
                eprintln!("[startup] route sweep reported failure (ok=false)");
            }

            #[cfg(target_os = "windows")]
            {
                let handle = app.handle();
                if let Err(e) = crate::tray::install(&handle) {
                    eprintln!("[startup] tray install failed: {e}");
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::kernel::start_kernel,
            commands::kernel::stop_kernel,
            commands::kernel::restart_kernel,
            commands::kernel::get_kernel_state,
            commands::profile::list_profiles,
            commands::profile::get_active_profile,
            commands::profile::get_profile_content,
            commands::profile::save_profile,
            commands::profile::delete_profile,
            commands::profile::import_profile_url,
            commands::profile::import_profile_file,
            commands::profile::update_subscription,
            commands::profile::set_active_profile,
            commands::profile::rename_profile,
            commands::profile::open_profile_in_editor,
            commands::profile::reveal_profile_file,
            commands::proxy::enable_system_proxy,
            commands::proxy::disable_system_proxy,
            commands::proxy::get_system_proxy_status,
            commands::desktop::get_autostart_status,
            commands::desktop::set_autostart,
            commands::desktop::get_silent_flag,
            commands::desktop::sweep_residual_routes,
            commands::tun::get_tun_state,
            commands::tun::enable_tun,
            commands::tun::disable_tun,
            commands::tun::sweep_tun_routes,
            // M10: traffic history (SQLite-backed)
            commands::history::get_traffic_history,
            commands::history::get_history_db_path,
            commands::history::get_history_sample_count,
            // Phase 8: outbound mode switcher + application reset
            commands::reset::reset_application,
        ])
        .run(tauri::generate_context!())
        .expect("error while running FlexClash");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Case-insensitive `.yaml` / `.yml` extension check used by the
/// drag-drop router above.  Lives in module scope so we can call it
/// from the `on_window_event` closure without capturing state.
fn is_yaml_path(p: &std::path::Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("yaml") | Some("yml"),
    )
}

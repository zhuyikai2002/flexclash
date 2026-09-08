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
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Parse silent flag *before* the Tauri builder so the managed state
    // is correct on first read from any setup hook.
    let silent = startup::parse_silent_flag();
    let silent_flag = SilentFlag::default();
    silent_flag.set(silent);

    // Phase R2: export typed IPC bindings (commands → src/bindings.ts).
    // Debug/dev builds rewrite the file on every start so the frontend
    // types never drift; release keeps the last generated copy.
    {
        use tauri_specta::{collect_commands, Builder};

        let _ = std::fs::write(
            concat!(env!("CARGO_MANIFEST_DIR"), "/.specta-ran.log"),
            "started",
        );

        let specta = Builder::<tauri::Wry>::new()
            .disable_serde_phases()
            .commands(collect_commands![
                commands::mihomo::get_mihomo_version,
                commands::mihomo::get_mihomo_configs,
                commands::mihomo::patch_mihomo_config,
                commands::mihomo::reload_mihomo_config,
                commands::mihomo::get_mihomo_proxies,
                commands::mihomo::get_mihomo_proxy,
                commands::mihomo::select_mihomo_proxy,
                commands::mihomo::get_mihomo_proxy_delay,
                commands::mihomo::get_mihomo_connections,
                commands::mihomo::close_mihomo_connection,
                commands::mihomo::close_all_mihomo_connections,
                commands::mihomo::get_mihomo_rules,
                commands::updater::check_update,
                commands::updater::install_update,
            ])
            .typ::<crate::core::watcher::AppStateSnapshot>()
            .typ::<crate::commands::updater::UpdateInfo>();

        // Export TypeScript bindings to the frontend source tree.
        let out_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/bindings.ts");
        let handle = std::thread::spawn(move || {
            specta.export(specta_typescript::Typescript::default(), out_path)
        });
        let _ = std::fs::write(
            concat!(env!("CARGO_MANIFEST_DIR"), "/.specta-export-err.log"),
            format!("{:?}", handle.join()),
        );
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
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
        .setup(move |app| {
            let handle = app.handle().clone();
            crate::core::shutdown::install(&handle);
            // M9: hand the AppHandle to the elevate module so the
            // TUN manager can resolve the mihomo work dir at any time.
            crate::core::elevate::install(handle.clone());

            // Phase R3: spawn the once-per-second state broadcaster.
            crate::core::watcher::spawn_state_watcher(
                &handle,
                app.state::<SidecarHandle>().inner().clone(),
            );

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

                // Phase 9.9: 彻底禁用 WebView2 DevTools + 浏览器加速键。
                //
                // `tauri.conf.json` 的 `devtools: false` 只会调用
                // `SetAreDevToolsEnabled(false)`，但 Ctrl+Shift+I / F12 /
                // Ctrl+R / F5 这类浏览器加速键在 WebView2 浏览器进程层
                // 处理，Settings 开关未必拦得住。这里用
                // `AcceleratorKeyPressed` 事件在加速键分发前精确拦截，
                // 同时保留 Ctrl+C/V/A 等编辑快捷键。
                use windows_core::Interface;
                let _ = win.with_webview(|webview| {
                    use webview2_com::AcceleratorKeyPressedEventHandler;
                    use webview2_com::Microsoft::Web::WebView2::Win32::{
                        ICoreWebView2Settings3, COREWEBVIEW2_KEY_EVENT_KIND,
                        COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
                        COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN,
                    };
                    let controller = webview.controller();

                    // 1. 引擎层开关（DevTools + 浏览器加速键）。
                    if let Ok(core) = unsafe { controller.CoreWebView2() } {
                        if let Ok(settings) = unsafe { core.Settings() } {
                            let _ = unsafe { settings.SetAreDevToolsEnabled(false) };
                            if let Ok(settings3) = settings.cast::<ICoreWebView2Settings3>() {
                                let _ =
                                    unsafe { settings3.SetAreBrowserAcceleratorKeysEnabled(false) };
                            }
                        }
                    }

                    // 2. 最终防线：AcceleratorKeyPressed 精确拦截。
                    let handler = AcceleratorKeyPressedEventHandler::create(Box::new(
                        move |_, args| {
                            let Some(args) = args else { return Ok(()) };
                            let mut kind = COREWEBVIEW2_KEY_EVENT_KIND(0);
                            unsafe { args.KeyEventKind(&mut kind)?; }
                            // 只处理按下事件，忽略抬起/重复。
                            if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                                && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
                            {
                                return Ok(());
                            }
                            let mut vk: u32 = 0;
                            unsafe { args.VirtualKey(&mut vk)?; }

                            // 修饰键状态（Ctrl = 0x11, Shift = 0x10）。
                            use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
                            // GetKeyState 返回 i16，最高位（0x8000）置位表示按下，
                            // 对 i16 而言即负数。
                            let ctrl = unsafe { GetKeyState(0x11) } < 0;
                            let shift = unsafe { GetKeyState(0x10) } < 0;

                            let block = match vk {
                                // F12 (DevTools) / F5 (reload)：无条件拦截。
                                0x7B | 0x74 => true,
                                // I / J：Ctrl+Shift+I/J (DevTools) 或 Ctrl+I/J。
                                0x49 | 0x4A => ctrl,
                                // R / U / P：Ctrl+R / Ctrl+U / Ctrl+P。
                                0x52 | 0x55 | 0x50 => ctrl,
                                // C：仅拦截 Ctrl+Shift+C (DevTools inspect)，
                                // 保留 Ctrl+C (复制)。
                                0x43 => ctrl && shift,
                                _ => false,
                            };
                            if block {
                                unsafe { args.SetHandled(true)?; }
                            }
                            Ok(())
                        },
                    ));
                    let mut token: i64 = 0;
                    if unsafe {
                        controller.add_AcceleratorKeyPressed(&handler, &mut token)
                    }
                    .is_ok()
                    {
                        eprintln!(
                            "[startup] WebView2 DevTools + accelerator keys hard-blocked"
                        );
                    }
                });

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
            // Phase R1: Mihomo REST façade (frontend no longer hits 9091 directly)
            commands::mihomo::get_mihomo_version,
            commands::mihomo::get_mihomo_configs,
            commands::mihomo::patch_mihomo_config,
            commands::mihomo::reload_mihomo_config,
            commands::mihomo::get_mihomo_proxies,
            commands::mihomo::get_mihomo_proxy,
            commands::mihomo::select_mihomo_proxy,
            commands::mihomo::get_mihomo_proxy_delay,
            commands::mihomo::get_mihomo_connections,
            commands::mihomo::close_mihomo_connection,
            commands::mihomo::close_all_mihomo_connections,
            commands::mihomo::get_mihomo_rules,
            commands::updater::check_update,
            commands::updater::install_update,
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

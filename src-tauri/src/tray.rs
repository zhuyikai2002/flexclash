// ============================================================================
// tray.rs — System tray integration (Tauri 2.x).
//
// Provides:
//   - A right-click context menu:
//       * Show / hide main window
//       * Toggle system proxy (check menu item, reflects current state)
//       * Quit (triggers the same CloseRequested path so shutdown runs)
//   - Left-click: toggle window visibility (hide-to-tray UX).
//
// State coordination:
//   The proxy check state is updated in two places:
//     1. When the proxy is toggled from the tray menu (immediate).
//     2. When the user toggles from the dashboard (via the `system_proxy_changed`
//        event we emit in `commands::proxy`).
//   The single source of truth is the registry; both code paths emit
//   `SYSTEM_PROXY_CHANGED` so listeners re-read the registry and update UI.
// ============================================================================

use std::sync::Arc;
use std::sync::Mutex;

use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Listener, Manager, Wry};

use crate::core::shutdown::ExitFlag;
use crate::error::AppError;
use crate::events::{KERNEL_LOG, SYSTEM_PROXY_CHANGED};
use crate::proxy;

const ID_SHOW:    &str = "tray_show";
const ID_TOGGLE:  &str = "tray_toggle_proxy";
const ID_QUIT:    &str = "tray_quit";

/// Container for handles the tray code needs to access from event callbacks.
/// Tray APIs only exist on the desktop Wry runtime, so the concrete type is
/// fixed here (Phase 1 ships Windows only).
pub struct TrayHandles {
    pub toggle_item: Arc<Mutex<Option<CheckMenuItem<Wry>>>>,
    pub show_item:   Arc<Mutex<Option<MenuItem<Wry>>>>,
}

impl TrayHandles {
    pub fn new() -> Self {
        Self {
            toggle_item: Arc::new(Mutex::new(None)),
            show_item:   Arc::new(Mutex::new(None)),
        }
    }
}

/// Build and install the tray icon + context menu. Phase 1 desktop-only.
#[cfg(target_os = "windows")]
pub fn install(app: &AppHandle<Wry>) -> Result<(), AppError> {
    install_impl(app)
}

#[cfg(not(target_os = "windows"))]
pub fn install<R: tauri::Runtime>(_app: &AppHandle<R>) -> Result<(), AppError> {
    Ok(())
}

fn install_impl(app: &AppHandle<Wry>) -> Result<(), AppError> {
    // Build the menu.
    let show_item = MenuItem::with_id(app, ID_SHOW, "Show panel", true, None::<&str>)
        .map_err(|e| AppError::Tray(format!("build show item: {e}")))?;

    // Initial proxy state from registry.
    let initial_checked = proxy::query_system_proxy_status()
        .map(|s| s.enabled)
        .unwrap_or(false);
    let toggle_item = CheckMenuItem::with_id(
        app,
        ID_TOGGLE,
        "System proxy",
        true,
        initial_checked,
        None::<&str>,
    )
    .map_err(|e| AppError::Tray(format!("build toggle item: {e}")))?;

    let quit_item = MenuItem::with_id(app, ID_QUIT, "Quit", true, None::<&str>)
        .map_err(|e| AppError::Tray(format!("build quit item: {e}")))?;

    let sep = PredefinedMenuItem::separator(app)
        .map_err(|e| AppError::Tray(format!("build separator: {e}")))?;

    let menu = Menu::with_items(app, &[
        &show_item,
        &sep,
        &toggle_item,
        &sep,
        &quit_item,
    ])
    .map_err(|e| AppError::Tray(format!("build menu: {e}")))?;

    // Stash handles so we can update the check state later.
    let handles = TrayHandles::new();
    {
        let mut g = handles.toggle_item.lock().expect("tray handles poisoned");
        *g = Some(toggle_item.clone());
    }
    {
        let mut g = handles.show_item.lock().expect("tray handles poisoned");
        *g = Some(show_item.clone());
    }
    app.manage(handles);

    // Build the icon. We re-use the app's bundle icon.
    let _tray = TrayIconBuilder::with_id("main")
        .tooltip("FlexClash")
        .icon(app.default_window_icon().cloned().ok_or_else(|| {
            AppError::Tray("no default window icon configured".into())
        })?)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event: MenuEvent| {
            handle_menu_event(app, event);
        })
        .on_tray_icon_event(|tray, event| {
            handle_tray_event(tray.app_handle().clone(), event);
        })
        .build(app)
        .map_err(|e| AppError::Tray(format!("build tray icon: {e}")))?;

    // Also subscribe to SYSTEM_PROXY_CHANGED so external toggles (e.g. the
    // dashboard switch) keep the menu item in sync.
    subscribe_proxy_state(app);

    let _ = app.emit(KERNEL_LOG, "[tray] installed");
    Ok(())
}

fn handle_menu_event(app: &AppHandle<Wry>, event: MenuEvent) {
    match event.id().as_ref() {
        ID_SHOW => toggle_window_visibility(app),
        ID_TOGGLE => {
            // Flip the proxy state. We do NOT pre-flip the menu item —
            // we wait for the `SYSTEM_PROXY_CHANGED` event to echo the
            // new state back. This keeps the menu and the registry in
            // sync via a single source of truth.
            let currently_on = proxy::query_system_proxy_status()
                .map(|s| s.enabled)
                .unwrap_or(false);
            let r = if currently_on {
                proxy::disable_system_proxy()
            } else {
                proxy::set_system_proxy(7890)
            };
            if let Err(e) = r {
                let _ = app.emit(KERNEL_LOG, format!("[tray] proxy toggle failed: {e}"));
                return;
            }
            let new_state = !currently_on;
            let _ = app.emit(
                SYSTEM_PROXY_CHANGED,
                serde_json::json!({
                    "enabled": new_state,
                    "port": if new_state { Some(7890u16) } else { None },
                    "source": "tray",
                }),
            );
        }
        ID_QUIT => {
            // Real-exit path: arm the global flag so the close hook runs
            // proxy restore + sidecar kill, then trigger the close.
            if let Some(flag) = app.try_state::<ExitFlag>() {
                flag.request();
            }
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.close();
            } else {
                app.exit(0);
            }
        }
        _ => {}
    }
}

fn handle_tray_event(app: AppHandle<Wry>, event: TrayIconEvent) {
    if let TrayIconEvent::Click { button, button_state, .. } = event {
        if button == MouseButton::Left && button_state == MouseButtonState::Up {
            toggle_window_visibility(&app);
        }
    }
}

fn toggle_window_visibility(app: &AppHandle<Wry>) {
    if let Some(win) = app.get_webview_window("main") {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            let _ = win.show();
            let _ = win.set_focus();
            let _ = win.unminimize();
        }
    }
}

fn subscribe_proxy_state(app: &AppHandle<Wry>) {
    let app_for_task = app.clone();
    tauri::async_runtime::spawn(async move {
        let app_for_cb = app_for_task.clone();
        let _ = app_for_task.listen_any(SYSTEM_PROXY_CHANGED, move |event| {
            #[derive(serde::Deserialize)]
            struct P { enabled: bool }
            if let Ok(p) = serde_json::from_str::<P>(event.payload()) {
                if let Some(state) = app_for_cb.try_state::<TrayHandles>() {
                    if let Some(item) = state.toggle_item.lock().expect("poisoned").as_ref() {
                        let _ = item.set_checked(p.enabled);
                    }
                }
            }
        });
    });
}

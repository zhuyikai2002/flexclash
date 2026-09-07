// ============================================================================
// tray.rs — System tray integration (Tauri 2.x).
//
// Provides:
//   - A right-click context menu:
//       * Show / hide main window
//       * Toggle system proxy (check menu item, reflects current state)
//       * Quit (triggers the same CloseRequested path so shutdown runs)
//   - Left-click: toggle window visibility (hide-to-tray UX).
//   - DYNAMIC ICON: the tray icon swaps between `tray-active.png`
//     (neon cat) and `tray-idle.png` (graphite cat) depending on
//     whether the system proxy AND/OR TUN mode is currently active.
//     Tooltip text updates to match.
//
// State coordination:
//   The proxy check state is updated in two places:
//     1. When the proxy is toggled from the tray menu (immediate).
//     2. When the user toggles from the dashboard (via the
//        `SYSTEM_PROXY_CHANGED` event we emit in `commands::proxy`).
//   The single source of truth is the registry; both code paths emit
//   `SYSTEM_PROXY_CHANGED` so listeners re-read the registry and
//   update UI.
//   The tray icon also subscribes to `TUN_STATE_CHANGED` so a TUN
//   flip swaps the icon on its own.
// ============================================================================

use std::sync::Arc;
use std::sync::Mutex;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Listener, Manager, Runtime, Wry};

use crate::core::shutdown::ExitFlag;
use crate::error::AppError;
use crate::events::{KERNEL_LOG, SYSTEM_PROXY_CHANGED, TUN_STATE_CHANGED};
use crate::proxy;

const ID_SHOW:    &str = "tray_show";
const ID_TOGGLE:  &str = "tray_toggle_proxy";
const ID_QUIT:    &str = "tray_quit";

// --- Icons (embedded at compile time so the bundle doesn't depend on
//     the working directory of the launcher).  tray-active.png 888 B,
//     tray-idle.png 590 B — negligible.  Generated from
//     src-tauri/icons/tray-{active,idle}.svg via
//     `node tools/svg-to-png.cjs` (uses @resvg/resvg-js). -----------------
const ICON_ACTIVE_PNG: &[u8] = include_bytes!("../icons/tray-active.png");
const ICON_IDLE_PNG:   &[u8] = include_bytes!("../icons/tray-idle.png");

const TOOLTIP_ACTIVE: &str = "FlexClash - 代理已连接";
const TOOLTIP_IDLE:   &str = "FlexClash - 直连模式";

/// Container for handles the tray code needs to access from event
/// callbacks. Tray APIs only exist on the desktop Wry runtime, so the
/// concrete type is fixed here (Phase 1 ships Windows only).
#[derive(Default)]
pub struct TrayHandles {
    pub toggle_item: Arc<Mutex<Option<CheckMenuItem<Wry>>>>,
    pub show_item:   Arc<Mutex<Option<MenuItem<Wry>>>>,
    /// The TrayIcon itself, so we can call `set_icon` / `set_tooltip`
    /// at runtime. Built once in `install()` and stashed here.
    pub tray_icon:   Arc<Mutex<Option<TrayIcon<Wry>>>>,
}

impl TrayHandles {
    /// Kept for API consistency with the rest of the codebase
    /// (`TunManager::new()`, `SidecarHandle::new()`, ...).
    pub fn new() -> Self { Self::default() }
}

// ============================================================================
// update_tray_icon — public sync helper, called from every state-change
// site.  Decision matrix:
//
//     proxy enabled?   TUN On?   icon
//     ----------------------------------
//     false            false     idle
//     true             false     active
//     false            true      active
//     true             true      active
//
// We re-load both icons from the embedded bytes every time, which is
// cheap (the bytes are already in .rodata) and means we never need to
// worry about handle / path lifetimes.
//
// On non-Windows builds this is a no-op (the tray is only installed
// on Windows) so commands can call it unconditionally.
// ============================================================================
pub fn update_tray_icon<R: Runtime>(app: &AppHandle<R>) -> Result<(), AppError> {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        update_tray_icon_impl(app)
    }
}

#[cfg(target_os = "windows")]
fn update_tray_icon_impl<R: Runtime>(app: &AppHandle<R>) -> Result<(), AppError> {
    let proxy_on = proxy::query_system_proxy_status()
        .map(|s| s.enabled)
        .unwrap_or(false);
    let tun_on = app
        .try_state::<crate::core::tun::TunManager>()
        .map(|m| m.snapshot().state == crate::core::tun::TunState::On)
        .unwrap_or(false);
    let active = proxy_on || tun_on;

    let (png, tooltip) = if active {
        (ICON_ACTIVE_PNG, TOOLTIP_ACTIVE)
    } else {
        (ICON_IDLE_PNG, TOOLTIP_IDLE)
    };

    let handles = app
        .try_state::<TrayHandles>()
        .ok_or_else(|| AppError::Tray("TrayHandles not managed".into()))?;
    let icon: Image<'_> = Image::from_bytes(png)
        .map_err(|e| AppError::Tray(format!("decode tray png: {e}")))?;

    let g = handles.tray_icon.lock().expect("tray handles poisoned");
    if let Some(tray) = g.as_ref() {
        tray.set_icon(Some(icon))
            .map_err(|e| AppError::Tray(format!("set tray icon: {e}")))?;
        tray.set_tooltip(Some(tooltip))
            .map_err(|e| AppError::Tray(format!("set tray tooltip: {e}")))?;
    }
    // Note: when both `proxy_on` and `tun_on` are false but the
    // kernel is running, we still show the idle icon — the user
    // metric is "is traffic going through FlexClash?", and that
    // requires either proxy or TUN.  The kernel can be started
    // before either of those is flipped, so this is the right
    // behaviour.
    Ok(())
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

    // Build the icon. We re-use the app's bundle icon for the *initial*
    // render, then immediately call `update_tray_icon` which swaps in
    // the correct active/idle variant.  This avoids a one-frame flash
    // of the wrong icon on first boot.
    let tray = TrayIconBuilder::with_id("main")
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

    // Stash the TrayIcon handle so update_tray_icon can mutate it.
    {
        let state = app.state::<TrayHandles>();
        let mut g = state.tray_icon.lock().expect("tray handles poisoned");
        *g = Some(tray);
    }

    // Subscribe to SYSTEM_PROXY_CHANGED + TUN_STATE_CHANGED so the
    // icon stays in sync with whatever flipped the underlying state.
    subscribe_proxy_state(app);
    subscribe_tun_state(app);

    // First paint: read current state and pick the right icon now.
    if let Err(e) = update_tray_icon(app) {
        let _ = app.emit(KERNEL_LOG, format!("[tray] initial icon update failed: {e}"));
    }

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
                // Swap the icon / tooltip.
                if let Err(e) = update_tray_icon(&app_for_cb) {
                    let _ = app_for_cb.emit(
                        KERNEL_LOG,
                        format!("[tray] icon update on proxy change failed: {e}"),
                    );
                }
            }
        });
    });
}

fn subscribe_tun_state(app: &AppHandle<Wry>) {
    let app_for_task = app.clone();
    tauri::async_runtime::spawn(async move {
        let app_for_cb = app_for_task.clone();
        let _ = app_for_task.listen_any(TUN_STATE_CHANGED, move |_event| {
            // The payload is the full TunStatus; we don't need to parse
            // it because `update_tray_icon` re-reads the canonical state
            // from the TunManager. Just re-paint the icon.
            if let Err(e) = update_tray_icon(&app_for_cb) {
                let _ = app_for_cb.emit(
                    KERNEL_LOG,
                    format!("[tray] icon update on tun change failed: {e}"),
                );
            }
        });
    });
}

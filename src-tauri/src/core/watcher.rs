// ============================================================================
// core/watcher.rs — Unified background state watcher (Phase R3).
//
// A single long-lived task pushes a compact `AppStateSnapshot` to the
// renderer once per second on `app-state://sync`. The frontend *projects*
// the snapshot into its stores; it no longer needs background timers /
// probes for kernel health, system-proxy state, or outbound mode.
//
// Collection strategy (all cheap, all local):
//   * kernel_online        — from the sidecar lifecycle state (no probe).
//   * system_proxy_active  — platform registry/gsettings read (µs-scale).
//   * current_mode         — GET /configs `mode` (only while online).
//
// SCOPE — what this deliberately does NOT do any more
// --------------------------------------------------
// Until Phase 2 this also derived `upload_speed` / `download_speed` by
// diffing `/connections` byte totals *once per second*, which meant a second
// HTTP poll per tick competing with the WebSocket ingest and a second,
// disagreeing notion of "current speed". The kernel's own `/traffic` stream is
// the authoritative source for those rates, and `core::ingest` now owns it, so
// the derivation is gone: one source, no duplicate poll, one fewer field pair
// to keep in step.
// ============================================================================

use std::sync::OnceLock;
use std::time::Duration;

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Emitter, Runtime};

use crate::config::profile::RESERVED_CONTROLLER;
use crate::core::sidecar::{KernelState, SidecarHandle};

/// Event emitted once per second with the full app state snapshot.
pub const APP_STATE_SYNC: &str = "app-state://sync";

const TICK: Duration = Duration::from_secs(1);
const HTTP_TIMEOUT: Duration = Duration::from_millis(800);

fn http() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            // Local controller call: bypass any system / environment proxy.
            .no_proxy()
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("build reqwest client for state watcher")
    })
}

/// Lightweight snapshot broadcast to the renderer each tick.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppStateSnapshot {
    /// Mihomo sidecar is up (Running state — no network probe needed).
    pub kernel_online: bool,
    /// System proxy currently active (platform query).
    pub system_proxy_active: bool,
    /// Outbound mode: rule / global / direct.
    pub current_mode: String,
}

async fn fetch_mode() -> Option<String> {
    let url = format!("http://{RESERVED_CONTROLLER}/configs");
    let resp = http().get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: serde_json::Value = resp.json().await.ok()?;
    v.get("mode").and_then(|x| x.as_str()).map(String::from)
}

/// Spawn the once-per-second state broadcaster. `sidecar` drives the
/// online flag; `app` is used to emit events to every webview.
pub fn spawn_state_watcher<R: Runtime>(app: &AppHandle<R>, sidecar: SidecarHandle) {
    let app = app.clone();
    let sidecar = sidecar.clone();

    tauri::async_runtime::spawn(async move {
        let mut mode = String::from("rule");

        loop {
            tokio::time::sleep(TICK).await;

            // `effective_state` folds in TUN ownership: while the elevated
            // TUN kernel serves the controller it counts as online even though
            // the regular sidecar handle is `Stopped`.
            let online =
                crate::core::sidecar::effective_state(&app, &sidecar) == KernelState::Running;
            let proxy_active = crate::proxy::query_system_proxy_status()
                .map(|s| s.enabled)
                .unwrap_or(false);

            if online {
                if let Some(m) = fetch_mode().await {
                    mode = m;
                }
            }

            let snap = AppStateSnapshot {
                kernel_online: online,
                system_proxy_active: proxy_active,
                current_mode: mode.clone(),
            };
            let _ = app.emit(APP_STATE_SYNC, snap);
        }
    });
}

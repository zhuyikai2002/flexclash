// ============================================================================
// core/watcher.rs — Unified background state watcher (Phase R3).
//
// A single long-lived task pushes a compact `AppStateSnapshot` to the
// renderer once per second on `app-state://sync`. The frontend *projects*
// the snapshot into its stores; it no longer needs background timers /
// probes for kernel health, system-proxy state, outbound mode, or speed.
//
// Collection strategy (all cheap, all local):
//   * kernel_online        — from the sidecar lifecycle state (no probe).
//   * system_proxy_active  — platform registry/gsettings read (µs-scale).
//   * current_mode         — GET /configs `mode` (only while online).
//   * upload/download_speed — diff of /connections byte totals per tick
//                             (only while online; 0 when offline).
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
    /// Instantaneous egress bytes/sec (derived from /connections totals).
    pub upload_speed: u32,
    /// Instantaneous ingress bytes/sec.
    pub download_speed: u32,
    /// Outbound mode: rule / global / direct.
    pub current_mode: String,
}

async fn fetch_totals() -> Option<(u64, u64)> {
    let url = format!("http://{RESERVED_CONTROLLER}/connections");
    let resp = http().get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: serde_json::Value = resp.json().await.ok()?;
    Some((
        v.get("uploadTotal").and_then(|x| x.as_u64()).unwrap_or(0),
        v.get("downloadTotal").and_then(|x| x.as_u64()).unwrap_or(0),
    ))
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

/// Delta of a monotonic counter vs the previous sample, clamped ≥ 0.
fn delta(cur: u64, prev: &mut u64) -> u32 {
    let d = cur.saturating_sub(*prev);
    *prev = cur;
    d.min(u32::MAX as u64) as u32
}

/// Spawn the once-per-second state broadcaster. `sidecar` drives the
/// online flag; `app` is used to emit events to every webview.
pub fn spawn_state_watcher<R: Runtime>(app: &AppHandle<R>, sidecar: SidecarHandle) {
    let app = app.clone();
    let sidecar = sidecar.clone();

    tauri::async_runtime::spawn(async move {
        let mut last_up: u64 = 0;
        let mut last_down: u64 = 0;
        let mut mode = String::from("rule");

        loop {
            tokio::time::sleep(TICK).await;

            let online = sidecar.state() == KernelState::Running;
            let proxy_active = crate::proxy::query_system_proxy_status()
                .map(|s| s.enabled)
                .unwrap_or(false);

            let (up_speed, down_speed) = if online {
                match fetch_totals().await {
                    Some((up, down)) => {
                        let ups = delta(up, &mut last_up);
                        let downs = delta(down, &mut last_down);
                        (ups, downs)
                    }
                    None => {
                        // First tick after start or a transient fetch miss.
                        (0u32, 0u32)
                    }
                }
            } else {
                // Kernel down — reset baselines so re-connect doesn't
                // report one giant delta on the next tick.
                last_up = 0;
                last_down = 0;
                (0u32, 0u32)
            };

            if online {
                if let Some(m) = fetch_mode().await {
                    mode = m;
                }
            }

            let snap = AppStateSnapshot {
                kernel_online: online,
                system_proxy_active: proxy_active,
                upload_speed: up_speed,
                download_speed: down_speed,
                current_mode: mode.clone(),
            };
            let _ = app.emit(APP_STATE_SYNC, snap);
        }
    });
}

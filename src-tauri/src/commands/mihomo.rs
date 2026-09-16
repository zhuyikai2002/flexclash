// ============================================================================
// commands/mihomo.rs — Rust-side Mihomo REST façade.
//
// Phase R1: every frontend REST query/control of the Mihomo external
// controller is funneled through these Tauri commands. Components no
// longer talk to http://127.0.0.1:9091/... directly.
//
// NOTE: push streams (/traffic, /connections WebSocket) cannot ride a
// Tauri IPC request, so they stay client-side — but all request/response
// surfaces live here.
//
// Errors are flattened to `String` so the renderer can `safeInvoke` and
// surface them without importing axios or parsing HRESULTs.
// ============================================================================

use serde_json::Value;

use crate::config::profile::RESERVED_CONTROLLER;
use crate::core::connections::{ConnectionFilter, KillReport};
use crate::core::urlenc::percent_encode;
use crate::error::AppError;

type CmdResult<T> = Result<T, AppError>;

const DEFAULT_TIMEOUT_MS: u64 = 5_000;

/// Address of the kernel's REST controller.
///
/// The app forces `RESERVED_CONTROLLER` into every profile it writes, so in the
/// running application this is simply that constant. It is read from the
/// environment first so the head-less geo-data smoke harness
/// (`src/bin/geodata-smoke.rs`) can drive a kernel it started on a scratch port;
/// a release build never sets `FLEXCLASH_CONTROLLER`, so nothing changes there.
fn base_url() -> String {
    let addr = std::env::var("FLEXCLASH_CONTROLLER")
        .ok()
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| RESERVED_CONTROLLER.to_string());
    format!("http://{addr}")
}

/// Fire one request against the Mihomo controller and return the JSON body.
/// A non-2xx response or a transport error is converted to a readable
/// `Err(String)` (via AppError) the frontend shows verbatim.
async fn mihomo_request(
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
    timeout_ms: u64,
) -> Result<Value, AppError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .build()
        .map_err(|e| AppError::Mihomo(format!("build http client: {e}")))?;

    let url = format!("{}{}", base_url(), path);
    let mut req = client.request(method, &url);
    if let Some(b) = body {
        req = req
            .header("Content-Type", "application/json")
            .body(b.to_string());
    }
    let resp = req.send().await.map_err(|e| {
        if e.is_connect() {
            AppError::Mihomo("mihomo not reachable — is the kernel running?".into())
        } else {
            AppError::Mihomo(format!("request {url}: {e}"))
        }
    })?;

    let status = resp.status();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Mihomo(format!("read {url}: {e}")))?;

    if !status.is_success() {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        return Err(AppError::Mihomo(format!(
            "mihomo HTTP {status} on {path}: {}",
            if text.trim().is_empty() {
                "(empty body)"
            } else {
                text.trim()
            }
        )));
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|e| AppError::Mihomo(format!("parse {url}: {e}")))
}

// ---------------------------------------------------------------------------
// Version / health
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn get_mihomo_version() -> CmdResult<String> {
    mihomo_request(reqwest::Method::GET, "/version", None, 2_000)
        .await
        .map(|v| v.to_string())
}

// ---------------------------------------------------------------------------
// Configs (GET / PATCH / PUT)
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn get_mihomo_configs() -> CmdResult<String> {
    mihomo_request(reqwest::Method::GET, "/configs", None, DEFAULT_TIMEOUT_MS)
        .await
        .map(|v| v.to_string())
}

/// PATCH /configs — outbound mode switch (`{"mode": "rule"}`), etc.
#[tauri::command]
#[specta::specta]
pub async fn patch_mihomo_config(mode: String) -> CmdResult<()> {
    mihomo_request(
        reqwest::Method::PATCH,
        "/configs",
        Some(serde_json::json!({ "mode": mode })),
        DEFAULT_TIMEOUT_MS,
    )
    .await
    .map(|_| ())
}

/// PUT /configs — hot-reload. `path: None` reloads the current config;
/// `Some(path)` loads that yaml file. `force` mirrors `?force=true`.
#[tauri::command]
#[specta::specta]
pub async fn reload_mihomo_config(path: Option<String>, force: Option<bool>) -> CmdResult<()> {
    let force = force.unwrap_or(true);
    let (query, body) = match &path {
        Some(p) => (
            format!("?force={force}"),
            Some(serde_json::json!({ "path": p })),
        ),
        None => (format!("?force={force}"), None),
    };
    mihomo_request(
        reqwest::Method::PUT,
        &format!("/configs{query}"),
        body,
        DEFAULT_TIMEOUT_MS,
    )
    .await
    .map(|_| ())
}

// ---------------------------------------------------------------------------
// Geo databases (v0.4.x refresh pipeline)
// ---------------------------------------------------------------------------

/// A full `ApplyConfig` re-parses every provider and rebuilds every listener,
/// so it is given far more room than a plain control call.
const CONFIG_RELOAD_TIMEOUT_MS: u64 = 30_000;

/// `PUT /configs?force=…` with `{"path": <file>}`.
///
/// This is the route mihomo uses to re-read a config *from disk*: its handler
/// JSON-decodes `{path, payload}`, parses the file at `path` and calls
/// `executor.ApplyConfig`. `path` must resolve under the user's home directory
/// (`constant.Path.IsSafePath`), which the mihomo work dir does.
///
/// Note there is no partial-update route for `geox-url`: `PATCH /configs`
/// accepts a fixed schema of ports / mode / tun / log-level and nothing else.
/// Repointing the geo sources therefore requires handing over a whole config.
pub(crate) async fn put_config_file(file: &std::path::Path, force: bool) -> Result<(), AppError> {
    let body = serde_json::json!({ "path": file.to_string_lossy() });
    mihomo_request(
        reqwest::Method::PUT,
        &format!("/configs?force={force}"),
        Some(body),
        CONFIG_RELOAD_TIMEOUT_MS,
    )
    .await
    .map(|_| ())
}

/// `POST /configs/geo` — make the kernel re-fetch the geo databases from
/// whatever `geox-url` currently points at, and drop its parsed-matcher
/// caches.
///
/// This is the *only* thing that clears those caches. mihomo exposes no other
/// REST entry point for it, and its own updater registers the clear
/// (`defer ClearGeoIPCache()` / `ClearGeoSiteCache()`) on the branch where it
/// actually downloaded something — which is precisely why the refreshed bytes
/// must be served from somewhere *other* than mihomo's own database path.
///
/// Answers `204 No Content` on success, or `500` plus a JSON error body.
pub(crate) async fn trigger_mihomo_geo_update(timeout_ms: u64) -> Result<(), AppError> {
    mihomo_request(reqwest::Method::POST, "/configs/geo", None, timeout_ms)
        .await
        .map(|_| ())
}

/// Is a kernel actually listening?
///
/// Consulted before committing to a geo-database transfer. There is no point
/// downloading ~21 MB when there is no kernel to apply it to, and a scheduled
/// check that hit a stopped kernel would otherwise repeat the transfer on every
/// retry. Kept short so a missing kernel costs a moment, not a timeout.
const PROBE_TIMEOUT_MS: u64 = 2_000;

pub(crate) async fn probe_controller() -> Result<(), AppError> {
    mihomo_request(reqwest::Method::GET, "/version", None, PROBE_TIMEOUT_MS)
        .await
        .map(|_| ())
}

// ---------------------------------------------------------------------------
// Proxies
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn get_mihomo_proxies() -> CmdResult<String> {
    mihomo_request(reqwest::Method::GET, "/proxies", None, DEFAULT_TIMEOUT_MS)
        .await
        .map(|v| v.to_string())
}

/// GET /proxies/{name} — single proxy / group read.
#[tauri::command]
#[specta::specta]
pub async fn get_mihomo_proxy(name: String) -> CmdResult<String> {
    let path = format!("/proxies/{}", percent_encode(&name));
    mihomo_request(reqwest::Method::GET, &path, None, DEFAULT_TIMEOUT_MS)
        .await
        .map(|v| v.to_string())
}

/// PUT /proxies/{group} — switch the active child of a Selector group.
/// Mihomo answers the PUT with 204 (empty body) on success, so we then
/// re-GET the group and return its authoritative JSON — the renderer never
/// sees a bare `null` payload it would crash on. A failed re-GET is NOT
/// fatal: the switch already happened, so we degrade to `null` and the
/// frontend falls back to optimistic state.
#[tauri::command]
#[specta::specta]
pub async fn select_mihomo_proxy(group: String, proxy: String) -> CmdResult<String> {
    // Tight 1.5 s budget: a hung controller must not wedge the UI.
    const SELECT_TIMEOUT_MS: u64 = 1_500;
    let path = format!("/proxies/{}", percent_encode(&group));
    // 1) Issue the switch; propagate transport/HTTP errors, ignore body.
    mihomo_request(
        reqwest::Method::PUT,
        &path,
        Some(serde_json::json!({ "name": proxy })),
        SELECT_TIMEOUT_MS,
    )
    .await?;
    // 2) Best-effort re-GET of the refreshed group. If this races or
    //    times out the switch already took effect — return null so the
    //    renderer uses its optimistic fallback instead of erroring.
    match mihomo_request(reqwest::Method::GET, &path, None, SELECT_TIMEOUT_MS).await {
        Ok(v) => Ok(v.to_string()),
        Err(_) => Ok(Value::Null.to_string()),
    }
}

/// GET /proxies/{name}/delay?url=…&timeout=…
#[tauri::command]
#[specta::specta]
pub async fn get_mihomo_proxy_delay(
    name: String,
    url: Option<String>,
    timeout_ms: Option<u32>,
) -> CmdResult<String> {
    let u = url.unwrap_or_else(|| "http://www.gstatic.com/generate_204".into());
    let t = timeout_ms.unwrap_or(5_000) as u64;
    let path = format!(
        "/proxies/{}/delay?url={}&timeout={}",
        percent_encode(&name),
        percent_encode(&u),
        t
    );
    mihomo_request(reqwest::Method::GET, &path, None, t)
        .await
        .map(|v| v.to_string())
}

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn get_mihomo_connections() -> CmdResult<String> {
    mihomo_request(
        reqwest::Method::GET,
        "/connections",
        None,
        DEFAULT_TIMEOUT_MS,
    )
    .await
    .map(|v| v.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn kill_connection(id: String) -> CmdResult<()> {
    crate::core::connections::kill_one(&id).await
}

/// Kill every connection matching a typed `ConnectionFilter` — fetch the
/// snapshot, filter by metadata, then DELETE the survivors one by one with rate
/// limiting. The filter is AND-combined and an empty filter is rejected (it
/// would otherwise mean "close the whole pool").
#[tauri::command]
#[specta::specta]
pub async fn kill_connections_by(filter: ConnectionFilter) -> CmdResult<KillReport> {
    crate::core::connections::kill_by_filter(&filter).await
}

#[tauri::command]
#[specta::specta]
pub async fn close_all_mihomo_connections() -> CmdResult<()> {
    mihomo_request(
        reqwest::Method::DELETE,
        "/connections",
        None,
        DEFAULT_TIMEOUT_MS,
    )
    .await
    .map(|_| ())
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn get_mihomo_rules() -> CmdResult<String> {
    mihomo_request(reqwest::Method::GET, "/rules", None, DEFAULT_TIMEOUT_MS)
        .await
        .map(|v| v.to_string())
}

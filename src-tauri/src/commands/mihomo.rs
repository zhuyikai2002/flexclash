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
use tauri::Runtime;

use crate::config::profile::RESERVED_CONTROLLER;
use crate::error::AppError;

type CmdResult<T> = Result<T, AppError>;

const DEFAULT_TIMEOUT_MS: u64 = 5_000;

fn base_url() -> String {
    format!("http://{RESERVED_CONTROLLER}")
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
            if text.trim().is_empty() { "(empty body)" } else { text.trim() }
        )));
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| AppError::Mihomo(format!("parse {url}: {e}")))
}

// ---------------------------------------------------------------------------
// Version / health
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_mihomo_version<R: Runtime>(
    _app: tauri::AppHandle<R>,
) -> CmdResult<Value> {
    mihomo_request(reqwest::Method::GET, "/version", None, 2_000).await
}

// ---------------------------------------------------------------------------
// Configs (GET / PATCH / PUT)
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_mihomo_configs<R: Runtime>(
    _app: tauri::AppHandle<R>,
) -> CmdResult<Value> {
    mihomo_request(reqwest::Method::GET, "/configs", None, DEFAULT_TIMEOUT_MS).await
}

/// PATCH /configs — outbound mode switch (`{"mode": "rule"}`), etc.
#[tauri::command]
pub async fn patch_mihomo_config<R: Runtime>(
    _app: tauri::AppHandle<R>,
    payload: Value,
) -> CmdResult<()> {
    mihomo_request(reqwest::Method::PATCH, "/configs", Some(payload), DEFAULT_TIMEOUT_MS)
        .await
        .map(|_| ())
}

/// PUT /configs — hot-reload. `path: None` reloads the current config;
/// `Some(path)` loads that yaml file. `force` mirrors `?force=true`.
#[tauri::command]
pub async fn reload_mihomo_config<R: Runtime>(
    _app: tauri::AppHandle<R>,
    path: Option<String>,
    force: Option<bool>,
) -> CmdResult<()> {
    let force = force.unwrap_or(true);
    let (query, body) = match &path {
        Some(p) => (format!("?force={force}"), Some(serde_json::json!({ "path": p }))),
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
// Proxies
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_mihomo_proxies<R: Runtime>(
    _app: tauri::AppHandle<R>,
) -> CmdResult<Value> {
    mihomo_request(reqwest::Method::GET, "/proxies", None, DEFAULT_TIMEOUT_MS).await
}

/// GET /proxies/{name} — single proxy / group read.
#[tauri::command]
pub async fn get_mihomo_proxy<R: Runtime>(
    _app: tauri::AppHandle<R>,
    name: String,
) -> CmdResult<Value> {
    let path = format!("/proxies/{}", percent_encode(&name));
    mihomo_request(reqwest::Method::GET, &path, None, DEFAULT_TIMEOUT_MS).await
}

/// PUT /proxies/{group} — switch the active child of a Selector group.
/// Returns the refreshed group object (mihomo echoes it on 200).
#[tauri::command]
pub async fn select_mihomo_proxy<R: Runtime>(
    _app: tauri::AppHandle<R>,
    group: String,
    proxy: String,
) -> CmdResult<Value> {
    let path = format!(
        "/proxies/{}",
        percent_encode(&group)
    );
    mihomo_request(
        reqwest::Method::PUT,
        &path,
        Some(serde_json::json!({ "name": proxy })),
        DEFAULT_TIMEOUT_MS,
    )
    .await
}

/// GET /proxies/{name}/delay?url=…&timeout=…
#[tauri::command]
pub async fn get_mihomo_proxy_delay<R: Runtime>(
    _app: tauri::AppHandle<R>,
    name: String,
    url: Option<String>,
    timeout_ms: Option<u64>,
) -> CmdResult<Value> {
    let u = url.unwrap_or_else(|| "http://www.gstatic.com/generate_204".into());
    let t = timeout_ms.unwrap_or(5_000);
    let path = format!(
        "/proxies/{}/delay?url={}&timeout={}",
        percent_encode(&name),
        percent_encode(&u),
        t
    );
    mihomo_request(reqwest::Method::GET, &path, None, t).await
}

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_mihomo_connections<R: Runtime>(
    _app: tauri::AppHandle<R>,
) -> CmdResult<Value> {
    mihomo_request(reqwest::Method::GET, "/connections", None, DEFAULT_TIMEOUT_MS).await
}

#[tauri::command]
pub async fn close_mihomo_connection<R: Runtime>(
    _app: tauri::AppHandle<R>,
    id: String,
) -> CmdResult<()> {
    let path = format!("/connections/{}", percent_encode(&id));
    mihomo_request(reqwest::Method::DELETE, &path, None, DEFAULT_TIMEOUT_MS)
        .await
        .map(|_| ())
}

#[tauri::command]
pub async fn close_all_mihomo_connections<R: Runtime>(
    _app: tauri::AppHandle<R>,
) -> CmdResult<()> {
    mihomo_request(reqwest::Method::DELETE, "/connections", None, DEFAULT_TIMEOUT_MS)
        .await
        .map(|_| ())
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_mihomo_rules<R: Runtime>(
    _app: tauri::AppHandle<R>,
) -> CmdResult<Value> {
    mihomo_request(reqwest::Method::GET, "/rules", None, DEFAULT_TIMEOUT_MS).await
}

/// Minimal RFC-3986 path/query segment encoding (keeps letters/digits and
/// the unreserved set, percent-encodes everything else).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

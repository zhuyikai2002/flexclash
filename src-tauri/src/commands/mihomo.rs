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
use specta::specta;


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

#[specta]
#[tauri::command]
pub async fn get_mihomo_version(
    
) -> CmdResult<String> {
    mihomo_request(reqwest::Method::GET, "/version", None, 2_000).await.map(|v| v.to_string())
}

// ---------------------------------------------------------------------------
// Configs (GET / PATCH / PUT)
// ---------------------------------------------------------------------------

#[specta]
#[tauri::command]
pub async fn get_mihomo_configs(
    
) -> CmdResult<String> {
    mihomo_request(reqwest::Method::GET, "/configs", None, DEFAULT_TIMEOUT_MS).await.map(|v| v.to_string())
}

/// PATCH /configs — outbound mode switch (`{"mode": "rule"}`), etc.
#[specta]
#[tauri::command]
pub async fn patch_mihomo_config(
    
    mode: String,
) -> CmdResult<()> {
    mihomo_request(reqwest::Method::PATCH, "/configs", Some(serde_json::json!({ "mode": mode })), DEFAULT_TIMEOUT_MS)
        .await
        .map(|_| ())
}

/// PUT /configs — hot-reload. `path: None` reloads the current config;
/// `Some(path)` loads that yaml file. `force` mirrors `?force=true`.
#[specta]
#[tauri::command]
pub async fn reload_mihomo_config(
    
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

#[specta]
#[tauri::command]
pub async fn get_mihomo_proxies(
    
) -> CmdResult<String> {
    mihomo_request(reqwest::Method::GET, "/proxies", None, DEFAULT_TIMEOUT_MS).await.map(|v| v.to_string())
}

/// GET /proxies/{name} — single proxy / group read.
#[specta]
#[tauri::command]
pub async fn get_mihomo_proxy(
    
    name: String,
) -> CmdResult<String> {
    let path = format!("/proxies/{}", percent_encode(&name));
    mihomo_request(reqwest::Method::GET, &path, None, DEFAULT_TIMEOUT_MS).await.map(|v| v.to_string())
}

/// PUT /proxies/{group} — switch the active child of a Selector group.
/// Returns the refreshed group object (mihomo echoes it on 200).
#[specta]
#[tauri::command]
pub async fn select_mihomo_proxy(
    
    group: String,
    proxy: String,
) -> CmdResult<String> {
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
    .map(|v| v.to_string())
}

/// GET /proxies/{name}/delay?url=…&timeout=…
#[specta]
#[tauri::command]
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
    mihomo_request(reqwest::Method::GET, &path, None, t).await.map(|v| v.to_string())
}

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

#[specta]
#[tauri::command]
pub async fn get_mihomo_connections(
    
) -> CmdResult<String> {
    mihomo_request(reqwest::Method::GET, "/connections", None, DEFAULT_TIMEOUT_MS).await.map(|v| v.to_string())
}

#[specta]
#[tauri::command]
pub async fn close_mihomo_connection(
    
    id: String,
) -> CmdResult<()> {
    let path = format!("/connections/{}", percent_encode(&id));
    mihomo_request(reqwest::Method::DELETE, &path, None, DEFAULT_TIMEOUT_MS)
        .await
        .map(|_| ())
}

#[specta]
#[tauri::command]
pub async fn close_all_mihomo_connections(
    
) -> CmdResult<()> {
    mihomo_request(reqwest::Method::DELETE, "/connections", None, DEFAULT_TIMEOUT_MS)
        .await
        .map(|_| ())
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

#[specta]
#[tauri::command]
pub async fn get_mihomo_rules(
    
) -> CmdResult<String> {
    mihomo_request(reqwest::Method::GET, "/rules", None, DEFAULT_TIMEOUT_MS).await.map(|v| v.to_string())
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

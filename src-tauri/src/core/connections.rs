// ============================================================================
// core/connections.rs — conditional connection-pool management.
//
// WHY THIS MODULE EXISTS
// ----------------------
// Mihomo exposes two connection-killing verbs on the controller, and no more:
//
//     GET    /connections            -> the live Snapshot (typed metadata)
//     DELETE /connections/{id}       -> close one connection by UUID
//     DELETE /connections            -> close *every* connection
//
// There is deliberately no "close connections matching X" endpoint. That is the
// gap this module fills: fetch the snapshot, filter it by a typed predicate
// over `TrackerInfo.metadata`, and DELETE the survivors one by one.
//
// The predicate (`ConnectionFilter`) is a *flat* AND of optional fields. It is
// intentionally NOT a composable query DSL — an empty filter matches nothing
// (see `is_empty` below) and is rejected before any DELETE, so a malformed or
// over-broad request can never nuke the whole pool. "Kill everything" stays an
// explicit, separate verb (`close_all_mihomo_connections`).
//
// LAYERING
// --------
// `core/` must never depend on `commands/`, so the HTTP client lives here (it
// mirrors the controller dialect already used by `commands/mihomo.rs` and
// `core/ingest.rs`) and the Tauri commands in `commands/mihomo.rs` are thin
// wrappers. The matching logic (`filter_ids`) is pure and unit-tested without
// any kernel.
//
// RATE LIMITING
// -------------
// A condition that matches hundreds of connections would otherwise fire the
// same number of DELETEs back-to-back and hammer the controller. Each DELETE is
// therefore spaced by `DELETE_SPACING` and a single call stops after
// `MAX_KILLS_PER_CALL`; `KillReport.matched` still reports the full count so the
// caller can see it was truncated (`matched > killed`).
// ============================================================================

use std::time::Duration;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use specta::Type;

use crate::config::profile::RESERVED_CONTROLLER;
use crate::core::urlenc::percent_encode;
use crate::error::AppError;

type Result<T> = std::result::Result<T, AppError>;

const DEFAULT_TIMEOUT_MS: u64 = 5_000;
/// Spacing between consecutive DELETEs so a broad match cannot storm the
/// controller.
const DELETE_SPACING: Duration = Duration::from_millis(50);
/// Hard cap on DELETEs issued by a single `kill_by_filter` call.
const MAX_KILLS_PER_CALL: usize = 100;

// ---------------------------------------------------------------------------
// Typed snapshot (mirrors mihomo `hub/route/connections.go` + `TrackerInfo`)
// ---------------------------------------------------------------------------

/// `GET /connections` response body.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ConnectionsSnapshot {
    #[serde(default, rename = "downloadTotal")]
    pub download_total: u64,
    #[serde(default, rename = "uploadTotal")]
    pub upload_total: u64,
    #[serde(default)]
    pub connections: Vec<ConnectionInfo>,
}

/// One live connection (`tunnel/statistic/tracker.go::TrackerInfo`).
#[derive(Clone, Debug, Deserialize)]
pub struct ConnectionInfo {
    pub id: String,
    /// Present for nearly every connection, but mihomo leaves it `null` for a
    /// bare handful (e.g. some early bootstrap sockets), so it is optional and
    /// a filter over metadata-only fields simply cannot match such a row.
    #[serde(default)]
    pub metadata: Option<ConnectionMetadata>,
    #[serde(default)]
    pub upload: u64,
    #[serde(default)]
    pub download: u64,
    #[serde(default)]
    pub start: String,
    #[serde(default)]
    pub chains: Vec<String>,
    #[serde(default)]
    pub rule: String,
    #[serde(default, rename = "rulePayload")]
    pub rule_payload: String,
    #[serde(default, rename = "providerChains")]
    pub provider_chains: Vec<String>,
}

/// `constant/metadata.go::Metadata` — the strongly-typed filter source.
///
/// Field renames are explicit because mihomo's JSON keys are not uniformly
/// camelCase (`sourceIP`, `destinationPort`, `processPath`, `sniffHost` …).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ConnectionMetadata {
    #[serde(default)]
    pub network: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default, rename = "sourceIP")]
    pub source_ip: String,
    #[serde(default, rename = "destinationIP")]
    pub destination_ip: String,
    #[serde(default, rename = "sourcePort")]
    pub source_port: String,
    #[serde(default, rename = "destinationPort")]
    pub destination_port: String,
    #[serde(default)]
    pub host: String,
    #[serde(default, rename = "sniffHost")]
    pub sniff_host: String,
    #[serde(default)]
    pub process: String,
    #[serde(default, rename = "processPath")]
    pub process_path: String,
    #[serde(default, rename = "dnsMode")]
    pub dns_mode: String,
    #[serde(default, rename = "remoteDestination")]
    pub remote_destination: String,
    #[serde(default, rename = "inboundName")]
    pub inbound_name: String,
}

// ---------------------------------------------------------------------------
// Filter
// ---------------------------------------------------------------------------

/// A flat, AND-combined predicate over a connection's metadata.
///
/// Every `Some` field must match; a `None` field is a wildcard. All fields
/// match as case-insensitive substrings except `destination`, which matches the
/// destination IP exactly, or the `IP:port` form as a prefix.
#[derive(Clone, Debug, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionFilter {
    /// Match `metadata.host` or `metadata.sniffHost`.
    pub host: Option<String>,
    /// Match `metadata.process` or `metadata.processPath`.
    pub process: Option<String>,
    /// Match the matched rule (`rule` or `rulePayload`).
    pub rule: Option<String>,
    /// Match any element of `chains` or `providerChains` (egress proxy).
    pub proxy: Option<String>,
    /// Match the destination IP (exact) or `IP:port` (prefix).
    pub destination: Option<String>,
}

impl ConnectionFilter {
    /// An empty filter matches nothing and must never be turned into a kill
    /// operation: it is the foot-gun that would otherwise mean "close the whole
    /// pool" through an accidentally-`{}`-shaped request.
    pub fn is_empty(&self) -> bool {
        self.host.is_none()
            && self.process.is_none()
            && self.rule.is_none()
            && self.proxy.is_none()
            && self.destination.is_none()
    }
}

/// Case-insensitive substring test.
fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

/// Does `conn` satisfy every specified field of `filter`?
fn matches(conn: &ConnectionInfo, filter: &ConnectionFilter) -> bool {
    let meta = conn.metadata.as_ref();

    if let Some(needle) = &filter.host {
        let hit = meta
            .map(|m| contains_ci(&m.host, needle) || contains_ci(&m.sniff_host, needle))
            .unwrap_or(false);
        if !hit {
            return false;
        }
    }
    if let Some(needle) = &filter.process {
        let hit = meta
            .map(|m| contains_ci(&m.process, needle) || contains_ci(&m.process_path, needle))
            .unwrap_or(false);
        if !hit {
            return false;
        }
    }
    if let Some(needle) = &filter.rule {
        if !(contains_ci(&conn.rule, needle) || contains_ci(&conn.rule_payload, needle)) {
            return false;
        }
    }
    if let Some(needle) = &filter.proxy {
        let hit = conn
            .chains
            .iter()
            .chain(conn.provider_chains.iter())
            .any(|chain| contains_ci(chain, needle));
        if !hit {
            return false;
        }
    }
    if let Some(needle) = &filter.destination {
        let hit = meta
            .map(|m| destination_matches(&m.destination_ip, &m.destination_port, needle))
            .unwrap_or(false);
        if !hit {
            return false;
        }
    }

    true
}

/// `IP` matches exactly; `IP:port` matches as a prefix of `ip:port`.
fn destination_matches(ip: &str, port: &str, needle: &str) -> bool {
    if needle.contains(':') {
        format!("{ip}:{port}").starts_with(needle)
    } else {
        ip == needle
    }
}

/// Project the snapshot down to the UUIDs that satisfy `filter`.
pub fn filter_ids(snapshot: &ConnectionsSnapshot, filter: &ConnectionFilter) -> Vec<String> {
    snapshot
        .connections
        .iter()
        .filter(|c| matches(c, filter))
        .map(|c| c.id.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Kill report
// ---------------------------------------------------------------------------

/// The outcome of a conditional kill.
#[derive(Clone, Debug, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KillReport {
    /// How many connections matched the filter (before any DELETE).
    pub matched: usize,
    /// How many DELETEs the controller accepted.
    pub killed: usize,
    /// How many DELETEs failed (connection already gone, controller hiccup…).
    pub failed: usize,
    /// The UUIDs that were successfully closed.
    pub killed_ids: Vec<String>,
}

// ---------------------------------------------------------------------------
// Controller HTTP
// ---------------------------------------------------------------------------

/// Address of the kernel's REST controller, honouring the same env override the
/// `commands/mihomo.rs` façade uses so a scratch kernel can be driven in tests.
fn base_url() -> String {
    let addr = std::env::var("FLEXCLASH_CONTROLLER")
        .ok()
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| RESERVED_CONTROLLER.to_string());
    format!("http://{addr}")
}

/// Fire one request and return the raw body bytes (empty on 204).
async fn request(method: reqwest::Method, path: &str, timeout_ms: u64) -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .build()
        .map_err(|e| AppError::Mihomo(format!("build http client: {e}")))?;

    let url = format!("{}{}", base_url(), path);
    let resp = client.request(method, &url).send().await.map_err(|e| {
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
    Ok(bytes.to_vec())
}

async fn get_json<T: DeserializeOwned>(path: &str, timeout_ms: u64) -> Result<T> {
    let bytes = request(reqwest::Method::GET, path, timeout_ms).await?;
    serde_json::from_slice(&bytes).map_err(|e| AppError::Mihomo(format!("parse {path}: {e}")))
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

/// `GET /connections`, parsed into the typed snapshot.
pub async fn fetch_snapshot() -> Result<ConnectionsSnapshot> {
    get_json("/connections", DEFAULT_TIMEOUT_MS).await
}

/// `DELETE /connections/{id}` — close exactly one connection.
pub async fn kill_one(id: &str) -> Result<()> {
    let path = format!("/connections/{}", percent_encode(id));
    request(reqwest::Method::DELETE, &path, DEFAULT_TIMEOUT_MS)
        .await
        .map(|_| ())
}

/// `GET /connections` → filter → rate-limited `DELETE /connections/{id}` loop.
pub async fn kill_by_filter(filter: &ConnectionFilter) -> Result<KillReport> {
    if filter.is_empty() {
        return Err(AppError::Mihomo(
            "refusing to kill: empty filter would match the whole pool".into(),
        ));
    }

    let snapshot = fetch_snapshot().await?;
    let ids = filter_ids(&snapshot, filter);

    let mut report = KillReport {
        matched: ids.len(),
        ..KillReport::default()
    };

    for id in ids.into_iter().take(MAX_KILLS_PER_CALL) {
        match kill_one(&id).await {
            Ok(()) => {
                report.killed += 1;
                report.killed_ids.push(id);
            }
            Err(_) => report.failed += 1,
        }
        // Space the DELETEs so a broad match cannot storm the controller.
        tokio::time::sleep(DELETE_SPACING).await;
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(id: &str, metadata: ConnectionMetadata) -> ConnectionInfo {
        ConnectionInfo {
            id: id.to_string(),
            metadata: Some(metadata),
            upload: 0,
            download: 0,
            start: String::new(),
            chains: Vec::new(),
            rule: String::new(),
            rule_payload: String::new(),
            provider_chains: Vec::new(),
        }
    }

    fn meta_with(host: &str, process: &str, ip: &str, port: &str) -> ConnectionMetadata {
        ConnectionMetadata {
            host: host.to_string(),
            process: process.to_string(),
            destination_ip: ip.to_string(),
            destination_port: port.to_string(),
            ..ConnectionMetadata::default()
        }
    }

    #[test]
    fn empty_filter_is_empty() {
        assert!(ConnectionFilter::default().is_empty());
        let f = ConnectionFilter {
            host: Some("x".into()),
            ..ConnectionFilter::default()
        };
        assert!(!f.is_empty());
    }

    #[test]
    fn host_matches_case_insensitive_substring() {
        let s = ConnectionsSnapshot {
            connections: vec![conn("a", meta_with("Example.COM", "", "", ""))],
            ..ConnectionsSnapshot::default()
        };
        let f = ConnectionFilter {
            host: Some("example".into()),
            ..ConnectionFilter::default()
        };
        assert_eq!(filter_ids(&s, &f), vec!["a"]);
    }

    #[test]
    fn host_also_matches_sniff_host() {
        let mut m = meta_with("", "", "", "");
        m.sniff_host = "cdn.example.net".into();
        let s = ConnectionsSnapshot {
            connections: vec![conn("a", m)],
            ..ConnectionsSnapshot::default()
        };
        let f = ConnectionFilter {
            host: Some("example.net".into()),
            ..ConnectionFilter::default()
        };
        assert_eq!(filter_ids(&s, &f), vec!["a"]);
    }

    #[test]
    fn proxy_matches_chain_element() {
        let mut c = conn("a", ConnectionMetadata::default());
        c.chains = vec!["DIRECT".into(), "HK-01".into()];
        let s = ConnectionsSnapshot {
            connections: vec![c],
            ..ConnectionsSnapshot::default()
        };
        let f = ConnectionFilter {
            proxy: Some("hk-01".into()),
            ..ConnectionFilter::default()
        };
        assert_eq!(filter_ids(&s, &f), vec!["a"]);
    }

    #[test]
    fn destination_exact_ip_and_port_prefix() {
        let s = ConnectionsSnapshot {
            connections: vec![conn("a", meta_with("", "", "1.2.3.4", "443"))],
            ..ConnectionsSnapshot::default()
        };
        let ip_only = ConnectionFilter {
            destination: Some("1.2.3.4".into()),
            ..ConnectionFilter::default()
        };
        assert_eq!(filter_ids(&s, &ip_only), vec!["a"]);
        let ip_port = ConnectionFilter {
            destination: Some("1.2.3.4:44".into()),
            ..ConnectionFilter::default()
        };
        assert_eq!(filter_ids(&s, &ip_port), vec!["a"]);
        let wrong = ConnectionFilter {
            destination: Some("1.2.3.5".into()),
            ..ConnectionFilter::default()
        };
        assert!(filter_ids(&s, &wrong).is_empty());
    }

    #[test]
    fn and_semantics_require_all_fields() {
        let s = ConnectionsSnapshot {
            connections: vec![
                conn("a", meta_with("example.com", "chrome", "1.2.3.4", "443")),
                conn("b", meta_with("example.com", "firefox", "1.2.3.4", "443")),
            ],
            ..ConnectionsSnapshot::default()
        };
        let f = ConnectionFilter {
            host: Some("example.com".into()),
            process: Some("chrome".into()),
            ..ConnectionFilter::default()
        };
        assert_eq!(filter_ids(&s, &f), vec!["a"]);
    }

    #[test]
    fn missing_metadata_never_matches_metadata_filters() {
        let mut c = conn("a", ConnectionMetadata::default());
        c.metadata = None;
        let s = ConnectionsSnapshot {
            connections: vec![c],
            ..ConnectionsSnapshot::default()
        };
        let f = ConnectionFilter {
            host: Some("example.com".into()),
            ..ConnectionFilter::default()
        };
        assert!(filter_ids(&s, &f).is_empty());
    }
}

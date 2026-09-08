// ============================================================================
// subscription.rs — HTTP fetch for clash-meta subscriptions + header parsing.
// ============================================================================

use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use serde::Serialize;

use crate::error::AppError;

/// Default User-Agent. Many providers (机场) key off this header to pick
/// the payload format:
///   * a plain browser UA  → webpage / Base64 node list
///   * a clash / mihomo UA → the full YAML with `proxy-groups`
/// We advertise `clash.meta` (plus an app suffix) so providers that
/// recognise the meta line hand down the standard subscription YAML.
pub const DEFAULT_USER_AGENT: &str = "clash.meta/v1.18.0 FlexClash/0.1.0";

/// Standard clash-meta / mihomo `subscription-userinfo` response header.
/// Shape (per spec): `upload=NN; download=NN; total=NN; expire=UNIX_TS`
#[derive(Debug, Clone, Default, Serialize)]
pub struct SubscriptionUserInfo {
    pub upload: Option<u64>,
    pub download: Option<u64>,
    pub total: Option<u64>,
    pub expire_at: Option<DateTime<Utc>>,
}

/// Fetch the subscription body from `url`. Returns the raw yaml content and
/// the parsed `subscription-userinfo` (if any).
pub async fn fetch_subscription(
    url: &str,
    user_agent: Option<&str>,
) -> Result<(String, SubscriptionUserInfo), AppError> {
    let ua = user_agent.unwrap_or(DEFAULT_USER_AGENT);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .user_agent(ua)
        .build()
        .map_err(|e| AppError::Subscription(format!("build http client: {e}")))?;

    let resp = client
        .get(url)
        .header("Accept", "application/yaml, text/yaml, text/plain;q=0.9, */*;q=0.5")
        .send()
        .await
        .map_err(|e| AppError::Subscription(format!("GET {url}: {e}")))?;

    if !resp.status().is_success() {
        return Err(AppError::Subscription(format!(
            "subscription HTTP {} for {url}",
            resp.status()
        )));
    }

    let user_info = parse_user_info_header(
        resp.headers()
            .get("subscription-userinfo")
            .and_then(|h| h.to_str().ok()),
    );

    let body = resp
        .text()
        .await
        .map_err(|e| AppError::Subscription(format!("read body: {e}")))?;

    if body.trim().is_empty() {
        return Err(AppError::Subscription("empty subscription body".into()));
    }

    // Format sniff: some providers return a Base64 node list or a webpage
    // instead of YAML when the UA isn't recognised.  A real Clash/mihomo
    // subscription always carries `proxies:` (and usually `proxy-groups:`).
    // We reject only the unambiguous non-YAML shapes so a legitimately
    // minimal YAML (e.g. a bare `proxies:` file) is never mis-filed.
    sniff_subscription(&body)?;

    Ok((body, user_info))
}

/// Reject subscription bodies that are clearly NOT a Clash/mihomo YAML.
/// Returns `Ok` for anything that contains a `proxies:` map, and also
/// tolerates other opaque-but-parseable bodies; only the two common
/// wrong-format payloads (Base64 node dump, HTML login/error page) get a
/// human-readable error so the UI can suggest subscription conversion.
fn sniff_subscription(body: &str) -> Result<(), AppError> {
    let head_has_yaml = body.contains("proxies:") || body.contains("proxy-groups:");
    if head_has_yaml {
        return Ok(());
    }

    // HTML (e.g. a login wall or provider error page).
    let trimmed = body.trim_start();
    if trimmed.starts_with("<!DOCTYPE") || trimmed.starts_with("<html") || trimmed.starts_with("<head") {
        return Err(AppError::Subscription(
            "subscription returned an HTML page instead of YAML — the link may be expired or require login".into(),
        ));
    }

    // Base64 node dump: mostly [A-Za-z0-9+/=], whitespace, length >= 48.
    let compact: String = body
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let looks_base64 = compact.len() >= 48
        && compact.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=')
        });
    if looks_base64 {
        return Err(AppError::Subscription(
            "subscription returned a Base64 node list instead of YAML — enable the provider's Clash/meta subscription, or run it through a subscription-converter".into(),
        ));
    }

    Ok(())
}

/// Parse a `subscription-userinfo` header value into structured fields.
/// Returns `SubscriptionUserInfo::default()` for any malformed/missing input.
pub fn parse_user_info_header(raw: Option<&str>) -> SubscriptionUserInfo {
    let Some(raw) = raw else { return SubscriptionUserInfo::default() };
    let mut out = SubscriptionUserInfo::default();
    for part in raw.split(';') {
        let part = part.trim();
        if part.is_empty() { continue; }
        let Some((k, v)) = part.split_once('=') else { continue; };
        let k = k.trim();
        let v = v.trim();
        match k {
            "upload" => out.upload = v.parse().ok(),
            "download" => out.download = v.parse().ok(),
            "total" => out.total = v.parse().ok(),
            "expire" => {
                if let Ok(ts) = v.parse::<i64>() {
                    out.expire_at = Some(Utc.timestamp_opt(ts, 0).single().unwrap_or_else(Utc::now));
                }
            }
            _ => { /* ignore unknown keys */ }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_user_info_full() {
        let h = "upload=1024; download=2048; total=10240; expire=1893456000";
        let u = parse_user_info_header(Some(h));
        assert_eq!(u.upload, Some(1024));
        assert_eq!(u.download, Some(2048));
        assert_eq!(u.total, Some(10240));
        assert!(u.expire_at.is_some());
    }

    #[test]
    fn parse_user_info_partial() {
        let h = "download=10";
        let u = parse_user_info_header(Some(h));
        assert_eq!(u.download, Some(10));
        assert!(u.upload.is_none());
        assert!(u.total.is_none());
    }

    #[test]
    fn parse_user_info_missing() {
        assert!(parse_user_info_header(None).total.is_none());
    }

    #[test]
    fn sniff_accepts_yaml_with_proxies() {
        let yaml = "proxies:\n  - { name: ss1, type: ss, server: 1.2.3.4, port: 8388 }\nproxy-groups:\n  - name: PROXY\n    type: select\n";
        assert!(sniff_subscription(yaml).is_ok());
    }

    #[test]
    fn sniff_rejects_base64_node_dump() {
        // A long run of base64 chars with line wraps is the classic
        // "converter needed" payload from providers.
        let b64 = "c3M6Ly9leGFtcGxlLmNvbTo4Mzg4OmFiY2RlZmdoaWprbG1ub3BxcnN0dXZ3eHl6MTIzNDU2"
            .repeat(8)
            .chars()
            .enumerate()
            .map(|(i, c)| if i % 64 == 63 { '\n' } else { c })
            .collect::<String>();
        assert!(sniff_subscription(&b64).is_err());
    }

    #[test]
    fn sniff_rejects_html_page() {
        let html = "<!DOCTYPE html><html><head><title>Login</title></head><body>sign in required</body></html>";
        assert!(sniff_subscription(html).is_err());
    }

    #[test]
    fn sniff_tolerates_unknown_plain_body() {
        // Non-YAML, non-base64 plain text is passed through (the UI will
        // surface parse errors later if mihomo rejects it).
        assert!(sniff_subscription("just some text\n").is_ok());
    }
}

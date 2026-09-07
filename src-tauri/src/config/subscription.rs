// ============================================================================
// subscription.rs — HTTP fetch for clash-meta subscriptions + header parsing.
// ============================================================================

use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use serde::Serialize;

use crate::error::AppError;

/// Default User-Agent. Many providers (机场) whitelist this UA and reject
/// arbitrary `curl` / `wget` traffic.
pub const DEFAULT_USER_AGENT: &str = "mihomo/1.19.30";

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

    Ok((body, user_info))
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
}

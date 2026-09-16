// ============================================================================
// core/log_parse.rs — defensive, strongly-typed anomaly extraction from the
// raw `/logs` text stream.
//
// WHY THIS MODULE EXISTS
// ----------------------
// Mihomo's `/logs?format=structured` returns `Fields` hardcoded to an empty
// array, so there is no JSON structure to parse — the fine-grained signal
// (DNS failures, dial timeouts, TLS errors, proxy switches) exists *only* as
// substrings inside the plain-text `payload`. This module scans that text and
// promotes the handful of lines that carry a real connectivity signature into
// a typed `LogAnomaly` event, while every other line keeps flowing through the
// existing raw `LogBatch` untouched.
//
// SAFETY CONTRACT
// ---------------
// Under `panic = "abort"` a panic here takes the whole process down, so this is
// written to be total: it never `unwrap`s, never indexes, and never allocates
// unboundedly. A line with no known signature returns `None`; a future-version
// line that merely *changed* its wording does the same. Nothing in here can
// fail the ingest loop.
//
// SIGNATURE TABLE
// ---------------
// The substrings below are pinned to mihomo v1.19.30's text output (Go `net` /
// `tls` error strings plus its `[DNS]` / `[TCP]` banners). They are matched
// case-insensitively. The `lookup ` marker is checked *before* the dial-timeout
// marker so that "dial tcp: lookup example.com … i/o timeout" (a DNS timeout)
// is classified as DNS, not as a TCP dial timeout.
// ============================================================================

use crate::core::kernel_events::{LogAnomaly, LogPayload};

/// Parse a defensive, strongly-typed anomaly from one `/logs` line.
///
/// Returns `None` for any line that carries no known connectivity signature
/// (including empty lines), so the caller simply lets it continue as raw log.
pub fn parse_anomaly(log: &LogPayload) -> Option<LogAnomaly> {
    let msg = log.message.trim();
    if msg.is_empty() {
        return None;
    }
    let lower = msg.to_ascii_lowercase();

    if lower.contains("connection refused") {
        return Some(LogAnomaly::ConnectRefused {
            address: extract_dial_addr(msg),
        });
    }
    // DNS *before* dial-timeout: a "dial tcp: lookup … i/o timeout" is a DNS
    // timeout, not a TCP one.
    if lower.contains("no such host")
        || lower.contains("nxdomain")
        || lower.contains("resolve failed")
        || lower.contains("lookup ")
    {
        return Some(LogAnomaly::DnsResolveFailed {
            host: extract_dns_host(msg),
            detail: dns_reason(&lower),
        });
    }
    if lower.contains("i/o timeout") || (lower.contains("dial tcp") && lower.contains("timeout")) {
        return Some(LogAnomaly::DialTimeout {
            address: extract_dial_addr(msg),
            elapsed_ms: None,
        });
    }
    if lower.contains("tls:") || lower.contains("x509") || lower.contains("certificate") {
        return Some(LogAnomaly::TlsError {
            host: extract_dns_host(msg),
            detail: tls_reason(&lower),
        });
    }
    if lower.contains("switched to") {
        return Some(LogAnomaly::ProxySwitch {
            from: None,
            to: extract_switched_to(msg),
        });
    }

    None
}

// -- reason tokens (short, stable, never the full raw line) -----------------

fn dns_reason(lower: &str) -> String {
    if lower.contains("nxdomain") {
        "nxdomain".into()
    } else if lower.contains("no such host") {
        "no such host".into()
    } else if lower.contains("i/o timeout") || lower.contains("timeout") {
        "timeout".into()
    } else if lower.contains("resolve failed") {
        "resolve failed".into()
    } else {
        String::new()
    }
}

fn tls_reason(lower: &str) -> String {
    if lower.contains("x509") {
        "x509".into()
    } else if lower.contains("certificate") {
        "certificate".into()
    } else {
        "tls".into()
    }
}

// -- best-effort field extraction -------------------------------------------

/// Extract the `IP:port` from `dial tcp ADDR: …` / `dial ADDR …`.
fn extract_dial_addr(msg: &str) -> String {
    for needle in ["dial tcp ", "dial "] {
        if let Some(idx) = msg.find(needle) {
            let rest = &msg[idx + needle.len()..];
            // `ADDR` ends at the first `": "` (the separator before the Go error
            // string); fall back to the first whitespace run.
            let cut = rest
                .find(": ")
                .or_else(|| rest.find(' '))
                .unwrap_or(rest.len());
            return rest[..cut].trim().to_string();
        }
    }
    String::new()
}

/// Extract the hostname from `lookup HOST…` / `resolve failed: HOST…`.
fn extract_dns_host(msg: &str) -> String {
    if let Some(idx) = msg.find("lookup ") {
        let rest = &msg[idx + "lookup ".len()..];
        let cut = rest.find([':', ' ', ',']).unwrap_or(rest.len());
        return rest[..cut].trim().to_string();
    }
    if let Some(idx) = msg.find("resolve failed") {
        let rest = &msg[idx + "resolve failed".len()..];
        return rest
            .trim_start_matches([':', ' '])
            .split([':', ' ', ','])
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
    }
    String::new()
}

/// Extract the node name from `switched to NAME`.
fn extract_switched_to(msg: &str) -> String {
    if let Some(idx) = msg.find("switched to ") {
        let rest = &msg[idx + "switched to ".len()..];
        let cut = rest.find([' ', ',', ':', '\n', '\r']).unwrap_or(rest.len());
        return rest[..cut].trim().to_string();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::kernel_events::KernelLogLevel;

    fn line(message: &str) -> LogPayload {
        LogPayload {
            level: KernelLogLevel::Info,
            message: message.to_string(),
            at_ms: 0,
        }
    }

    #[test]
    fn dns_no_such_host() {
        assert_eq!(
            parse_anomaly(&line("dial tcp: lookup example.com: no such host")),
            Some(LogAnomaly::DnsResolveFailed {
                host: "example.com".into(),
                detail: "no such host".into(),
            })
        );
    }

    #[test]
    fn dns_nxdomain() {
        assert_eq!(
            parse_anomaly(&line("[DNS] resolve example.org: NXDOMAIN")),
            Some(LogAnomaly::DnsResolveFailed {
                host: String::new(),
                detail: "nxdomain".into(),
            })
        );
    }

    #[test]
    fn dns_timeout_is_dns_not_dial() {
        // "lookup … i/o timeout" must be classified as DNS, not TCP dial.
        assert_eq!(
            parse_anomaly(&line(
                "dial tcp: lookup example.net on 8.8.8.8:53: i/o timeout"
            )),
            Some(LogAnomaly::DnsResolveFailed {
                host: "example.net".into(),
                detail: "timeout".into(),
            })
        );
    }

    #[test]
    fn dial_timeout() {
        assert_eq!(
            parse_anomaly(&line("dial tcp 1.2.3.4:443: i/o timeout")),
            Some(LogAnomaly::DialTimeout {
                address: "1.2.3.4:443".into(),
                elapsed_ms: None,
            })
        );
    }

    #[test]
    fn connect_refused() {
        assert_eq!(
            parse_anomaly(&line("dial tcp 10.0.0.1:22: connect: connection refused")),
            Some(LogAnomaly::ConnectRefused {
                address: "10.0.0.1:22".into(),
            })
        );
    }

    #[test]
    fn tls_certificate_error() {
        assert_eq!(
            parse_anomaly(&line(
                "tls: failed to verify certificate: x509: certificate signed by unknown authority"
            )),
            Some(LogAnomaly::TlsError {
                host: String::new(),
                detail: "x509".into(),
            })
        );
    }

    #[test]
    fn proxy_switch() {
        assert_eq!(
            parse_anomaly(&line("[URLTest] switching: switched to HK-01")),
            Some(LogAnomaly::ProxySwitch {
                from: None,
                to: "HK-01".into(),
            })
        );
    }

    #[test]
    fn unknown_line_returns_none() {
        assert_eq!(parse_anomaly(&line("[TCP] connected 1.2.3.4:443")), None);
        assert_eq!(parse_anomaly(&line("level=info msg=started")), None);
    }

    #[test]
    fn empty_line_returns_none() {
        assert_eq!(parse_anomaly(&line("   ")), None);
    }
}

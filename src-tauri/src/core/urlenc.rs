// ============================================================================
// core/urlenc.rs — Minimal RFC-3986 segment encoding.
//
// Two call sites need it: the Mihomo REST façade (`commands/mihomo.rs`) builds
// `/proxies/{name}` paths, and the native speed-test engine
// (`core/speedtest.rs`) builds the same paths concurrently. Keeping one copy
// here means a node name containing `#`, `?`, `/` or a space is escaped
// identically on both paths — a divergence would make a node testable from one
// code path and "not found" from the other, which is a miserable bug to chase.
// ============================================================================

/// Percent-encode a single path/query segment.
///
/// Keeps the RFC-3986 unreserved set (`A-Z a-z 0-9 - _ . ~`) and encodes
/// everything else as `%XX` over the **UTF-8 bytes**, so non-ASCII node names
/// (Chinese airport node labels are common) round-trip correctly.
pub(crate) fn percent_encode(s: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_unreserved_untouched() {
        assert_eq!(percent_encode("Direct"), "Direct");
        assert_eq!(percent_encode("node-1_a.b~c"), "node-1_a.b~c");
    }

    #[test]
    fn encodes_reserved_and_reserved_adjacent() {
        assert_eq!(percent_encode("HK#1"), "HK%231");
        assert_eq!(percent_encode("a/b"), "a%2Fb");
        assert_eq!(percent_encode("a b"), "a%20b");
        assert_eq!(percent_encode("a?b=c"), "a%3Fb%3Dc");
    }

    #[test]
    fn encodes_utf8_bytes_not_chars() {
        // "香港" -> 6 UTF-8 bytes, each percent-encoded.
        assert_eq!(percent_encode("香港"), "%E9%A6%99%E6%B8%AF");
    }

    #[test]
    fn empty_is_empty() {
        assert_eq!(percent_encode(""), "");
    }
}

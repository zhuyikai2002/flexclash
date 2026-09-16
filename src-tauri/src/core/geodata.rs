// ============================================================================
// core/geodata.rs — Geo-Data (GeoIP.dat / GeoSite.dat) refresh policy.
//
// This module is the *policy* half of the v0.4.x geo-data pipeline: which
// upstream to ask, whether a refresh is warranted at all, and how to degrade
// when a source is down. It deliberately performs **no I/O** — every function
// below is pure, so the decision logic is unit-testable without a network and
// CI stays deterministic. The transferring half lands in a later step.
//
// WHY A POLICY LAYER RATHER THAN A PLAIN DOWNLOADER
// ------------------------------------------------
// mihomo caches *parsed* geo matchers in permanent `singleflight` groups
// (`component/geodata/utils.go`) and exposes no REST entry point to clear that
// cache. Only its own updater calls `ClearGeoIPCache()` / `ClearGeoSiteCache()`
// / `mmdb.ReloadIP()`, from inside `component/updater/update_geo.go`, and only
// on the branch where it actually downloaded something. The practical
// consequence, verified against the source rather than assumed:
//
//     download ourselves -> write the file -> PUT /configs?force=true
//
// does NOT take effect. Rule construction calls `LoadGeoIPMatcher("CN")` again,
// the singleflight group answers from cache, and the kernel keeps matching
// against the old database. `POST /configs/geo` does not rescue it either: if
// the file on disk already carries the new hash, mihomo returns early *before*
// registering its `defer ClearGeoIPCache()`.
//
// So the transfer has to be performed by mihomo, from a source we control.
// That is why this module decides *what* and *when*, and a later step feeds
// mihomo the verified bytes over loopback.
//
// STICKY FAILOVER
// ---------------
// Sources are NOT interchangeable. The MetaCubeX lineage and the
// v2ray-rules-dat (Loyalsoldier) lineage ship different content, so crossing
// lineages silently changes routing verdicts. Degradation is therefore *sticky
// and monotonic within a cycle*: we only ever step to a lower-priority source,
// never back up, and the primary is re-probed only when a fresh cycle begins.
// See `next_source` and `cycle_start_source`.
//
// CHECKSUM DISCIPLINE
// -------------------
// The `.sha256sum` sidecar (a few dozen bytes) decides *whether* to act; the
// digest of the file that actually landed on disk decides whether we are done.
// The sidecar is preferred over HTTP validators because the primary source is
// a GitHub release asset whose 302 carries no ETag or Last-Modified at all,
// and because jsDelivr's ETag is generated per edge node rather than from the
// content (measured 2026-09-16: cdn.jsdelivr and fastly.jsdelivr agree, while
// testingcf returns a different value for byte-identical files). A content
// digest cannot drift that way; an ETag can.
// ============================================================================

use crate::error::{AppError, Result};

/// Number of hex digits in a SHA-256 digest.
const SHA256_HEX_LEN: usize = 64;

/// The upstream lineage a source belongs to.
///
/// Two sources of the same lineage serve byte-identical data — verified
/// 2026-09-16 across the GitHub release asset, `raw.githubusercontent.com` and
/// all three jsDelivr nodes. Crossing lineages does not, so the caller records
/// a lineage change in the update result instead of applying it silently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeoLineage {
    /// `MetaCubeX/meta-rules-dat` — the default, and every mirror we ship.
    MetaCubeX,
    /// `Loyalsoldier/v2ray-rules-dat` — different content; see module docs.
    V2rayRulesDat,
}

/// One upstream that can supply the geo databases.
#[derive(Clone, Copy, Debug)]
pub struct GeoSource {
    /// Stable id: safe to persist in state and to render in the UI.
    pub id: &'static str,
    /// Which lineage this source serves.
    pub lineage: GeoLineage,
    /// `geoip.dat` — GeoIP in `.dat` form, which requires `geodata-mode: true`.
    pub geoip: &'static str,
    /// `geosite.dat` — always `.dat`, independent of `geodata-mode`.
    pub geosite: &'static str,
    /// Sidecar for `geoip`, containing `<sha256>  geoip.dat`.
    pub geoip_sum: &'static str,
    /// Sidecar for `geosite`, containing `<sha256>  geosite.dat`.
    pub geosite_sum: &'static str,
}

/// The managed artefacts. The on-disk names are the ones mihomo resolves
/// inside its `-d` directory (`constant/path.go`); the match there is
/// case-insensitive and falls back to exactly these spellings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeoFile {
    Geoip,
    Geosite,
}

impl GeoFile {
    /// The file name mihomo expects in its `-d` directory.
    pub fn asset_name(self) -> &'static str {
        match self {
            GeoFile::Geoip => "GeoIP.dat",
            GeoFile::Geosite => "GeoSite.dat",
        }
    }
}

impl GeoSource {
    /// The managed artefacts paired with their own sidecars.
    ///
    /// Exposed as one accessor so callers never hand-pair a database with the
    /// wrong checksum URL — a mismatch that would compare a GeoIP digest
    /// against GeoSite bytes and look like a corrupt download.
    pub fn files(&self) -> [(GeoFile, &'static str, &'static str); 2] {
        [
            (GeoFile::Geoip, self.geoip, self.geoip_sum),
            (GeoFile::Geosite, self.geosite, self.geosite_sum),
        ]
    }
}

/// Index of the source every cycle starts from when the upstream is healthy.
pub const PRIMARY_SOURCE_INDEX: usize = 0;

/// Upstreams in preference order. Index 0 is the primary.
///
/// Every entry is MetaCubeX lineage, so a downgrade changes *where* the bytes
/// come from but not *what* they mean. The Loyalsoldier lineage is modelled by
/// `GeoLineage` but deliberately not wired in yet: it ships different content
/// and its `.sha256sum` sidecar has not been verified, and adding a source
/// whose checksum source is unconfirmed would defeat the whole point of the
/// digest gate. Adding it later is a data change in this table, nothing more.
pub const SOURCES: [GeoSource; 4] = [
    GeoSource {
        id: "gh-release",
        lineage: GeoLineage::MetaCubeX,
        geoip: "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.dat",
        geosite: "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geosite.dat",
        geoip_sum:
            "https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/release/geoip.dat.sha256sum",
        geosite_sum:
            "https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/release/geosite.dat.sha256sum",
    },
    GeoSource {
        id: "jsdelivr-cdn",
        lineage: GeoLineage::MetaCubeX,
        geoip: "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geoip.dat",
        geosite: "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geosite.dat",
        geoip_sum: "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geoip.dat.sha256sum",
        geosite_sum:
            "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geosite.dat.sha256sum",
    },
    GeoSource {
        id: "jsdelivr-fastly",
        lineage: GeoLineage::MetaCubeX,
        geoip: "https://fastly.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geoip.dat",
        geosite: "https://fastly.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geosite.dat",
        geoip_sum:
            "https://fastly.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geoip.dat.sha256sum",
        geosite_sum:
            "https://fastly.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geosite.dat.sha256sum",
    },
    GeoSource {
        id: "jsdelivr-testingcf",
        lineage: GeoLineage::MetaCubeX,
        geoip: "https://testingcf.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geoip.dat",
        geosite: "https://testingcf.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geosite.dat",
        geoip_sum:
            "https://testingcf.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geoip.dat.sha256sum",
        geosite_sum:
            "https://testingcf.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geosite.dat.sha256sum",
    },
];

/// Look up a source by index, or `None` when the index is out of range.
pub fn source(index: usize) -> Option<&'static GeoSource> {
    SOURCES.get(index)
}

// ---------------------------------------------------------------------------
// Checksum parsing
// ---------------------------------------------------------------------------

/// A parsed `.sha256sum` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sha256Sum {
    /// Exactly 64 hex digits, lowercased so comparisons are case-stable.
    pub sha256: String,
    /// The name the digest belongs to, when the sidecar records one.
    pub filename: Option<String>,
}

/// Parse a `<sha256>  <name>` sidecar.
///
/// Tolerant about the *framing* and strict about the *digest*: a leading BOM,
/// CRLF endings, blank leading lines, extra whitespace and GNU's binary-mode
/// `*` marker are all accepted, but a field that is not exactly 64 hex digits
/// is an error. That asymmetry is the point — the framing varies between
/// publishers, whereas a malformed digest means we are looking at something
/// that is not a checksum at all (an HTML error page being the classic case),
/// and silently treating it as "changed" would trigger a pointless 21 MB
/// download on every single check.
///
/// Only the first non-blank line is read: each sidecar here describes exactly
/// one artefact.
pub fn parse_sha256sum(raw: &str) -> Result<Sha256Sum> {
    let first = raw
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or_else(|| AppError::Geo("checksum file is empty".into()))?;

    let mut fields = first.split_whitespace();
    let digest = fields
        .next()
        .ok_or_else(|| AppError::Geo("checksum file has no digest field".into()))?;

    if digest.len() != SHA256_HEX_LEN || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::Geo(format!(
            "checksum field is not a SHA-256 digest: {digest:?}"
        )));
    }

    let filename = fields
        .next()
        .map(|name| name.trim_start_matches('*'))
        .filter(|name| !name.is_empty())
        .map(str::to_owned);

    Ok(Sha256Sum {
        sha256: digest.to_ascii_lowercase(),
        filename,
    })
}

// ---------------------------------------------------------------------------
// Update decision
// ---------------------------------------------------------------------------

/// Whether the local database needs to be refreshed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateDecision {
    /// The remote digest equals the one we already applied — nothing to do.
    UpToDate,
    /// The remote digest differs — a refresh is warranted.
    NeedsUpdate,
    /// We have no applied digest on record, so neither answer is justified.
    /// The caller resolves this by hashing the file on disk, which avoids
    /// pulling ~21 MB on a first run whose database is already current.
    Unknown,
}

/// Compare the published digest against the one we last applied.
///
/// This is the *before* half of the checksum discipline: it runs against a few
/// dozen bytes of sidecar, so an unchanged upstream costs no database traffic
/// at all. The comparison is case-insensitive because uppercase hex is legal
/// in a `.sha256sum` even though publishers almost never use it.
pub fn decide_if_update_needed(
    remote_sha256: &str,
    applied_sha256: Option<&str>,
) -> UpdateDecision {
    let remote = remote_sha256.trim();
    match applied_sha256.map(str::trim) {
        None | Some("") => UpdateDecision::Unknown,
        Some(applied) if applied.eq_ignore_ascii_case(remote) => UpdateDecision::UpToDate,
        Some(_) => UpdateDecision::NeedsUpdate,
    }
}

// ---------------------------------------------------------------------------
// Sticky failover
// ---------------------------------------------------------------------------

/// The source to use for the `attempt`-th try of the current cycle.
///
/// `attempt == 0` returns `current` unchanged, which is what makes failover
/// sticky: the caller starts every cycle from the source it is currently
/// pinned to rather than from the primary, so a degraded cycle cannot ping-pong
/// back onto a source that just failed it.
///
/// Advancing is strictly forward and never wraps, so `attempt` can only ever
/// move *away* from the primary. `None` means every source at or after
/// `current` has been tried — the cycle is exhausted and the caller should
/// report failure and leave the existing database alone.
///
/// Returns `None` for an out-of-range `current` rather than panicking:
/// `current` comes from persisted state, and persisted state can be corrupt.
pub fn next_source(current: usize, attempt: u32) -> Option<usize> {
    if current >= SOURCES.len() {
        return None;
    }
    let index = current.checked_add(attempt as usize)?;
    (index < SOURCES.len()).then_some(index)
}

/// The source a fresh cycle should start from.
///
/// This is where the anti-ping-pong rule actually lives, and why it is a
/// separate function instead of being folded into `next_source`: a cycle is
/// only allowed to return to the primary once the primary has been *proven*
/// good. Passing `primary_ok == false` keeps the previously degraded pin, so a
/// source that failed twice in a row is not retried on a hunch. An out-of-range
/// pin (corrupt state) falls back to the primary rather than to `None`, since
/// a fresh cycle has no prior attempt to be consistent with.
pub fn cycle_start_source(sticky: usize, primary_ok: bool) -> usize {
    if primary_ok || sticky >= SOURCES.len() {
        PRIMARY_SOURCE_INDEX
    } else {
        sticky
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;

    const DIGEST_A: &str = "e8fba6c888d7bf13b4cfcab2b4d3cf7c355a9a2f4d94ee34c9ccfc00730bb389";
    const DIGEST_B: &str = "45325fee1555c8bf04115100694ce8429b88c9bb3b3548abcfd236a1c8ea146f";

    // -- parse_sha256sum : accepted framing --------------------------------

    #[test]
    fn parses_two_space_form() {
        let got = parse_sha256sum(&format!("{DIGEST_A}  geosite.dat")).unwrap();
        assert_eq!(got.sha256, DIGEST_A);
        assert_eq!(got.filename.as_deref(), Some("geosite.dat"));
    }

    #[test]
    fn parses_single_space_form() {
        // What the MetaCubeX sidecars actually contain.
        let got = parse_sha256sum(&format!("{DIGEST_A} geosite.dat")).unwrap();
        assert_eq!(got.sha256, DIGEST_A);
        assert_eq!(got.filename.as_deref(), Some("geosite.dat"));
    }

    #[test]
    fn strips_gnu_binary_marker() {
        let got = parse_sha256sum(&format!("{DIGEST_A} *geosite.dat")).unwrap();
        assert_eq!(got.filename.as_deref(), Some("geosite.dat"));
    }

    #[test]
    fn accepts_crlf_and_blank_leading_lines() {
        let raw = format!("\r\n\r\n{DIGEST_A}  geosite.dat\r\n");
        let got = parse_sha256sum(&raw).unwrap();
        assert_eq!(got.sha256, DIGEST_A);
        assert_eq!(got.filename.as_deref(), Some("geosite.dat"));
    }

    #[test]
    fn accepts_leading_bom() {
        let raw = format!("\u{feff}{DIGEST_A}  geosite.dat");
        let got = parse_sha256sum(&raw).unwrap();
        assert_eq!(got.sha256, DIGEST_A);
    }

    #[test]
    fn tolerates_surrounding_whitespace() {
        let raw = format!("   \t{DIGEST_A}\t\tgeosite.dat   ");
        let got = parse_sha256sum(&raw).unwrap();
        assert_eq!(got.filename.as_deref(), Some("geosite.dat"));
    }

    #[test]
    fn lowercases_uppercase_digest() {
        let got = parse_sha256sum(&format!("{}  geosite.dat", DIGEST_A.to_uppercase())).unwrap();
        assert_eq!(got.sha256, DIGEST_A);
    }

    #[test]
    fn reads_only_the_first_non_blank_line() {
        // A multi-entry sidecar must not be mistaken for a multi-line digest.
        let raw = format!("{DIGEST_A}  geosite.dat\n{DIGEST_B}  geoip.dat\n");
        let got = parse_sha256sum(&raw).unwrap();
        assert_eq!(got.sha256, DIGEST_A);
        assert_eq!(got.filename.as_deref(), Some("geosite.dat"));
    }

    #[test]
    fn digest_without_filename_is_accepted() {
        let got = parse_sha256sum(DIGEST_A).unwrap();
        assert_eq!(got.sha256, DIGEST_A);
        assert_eq!(got.filename, None);
    }

    // -- parse_sha256sum : rejected input ----------------------------------

    #[test]
    fn rejects_empty_input() {
        assert!(matches!(parse_sha256sum(""), Err(AppError::Geo(_))));
        assert!(parse_sha256sum("   \n\t\n").is_err());
        assert!(parse_sha256sum("\u{feff}").is_err());
    }

    #[test]
    fn rejects_wrong_digest_length() {
        let short = &DIGEST_A[..SHA256_HEX_LEN - 1];
        let long = format!("{DIGEST_A}0");
        assert!(
            parse_sha256sum(short).is_err(),
            "63 hex digits must be rejected"
        );
        assert!(
            parse_sha256sum(&long).is_err(),
            "65 hex digits must be rejected"
        );
    }

    #[test]
    fn rejects_non_hex_digest() {
        // An HTML error page is the realistic way this happens.
        assert!(parse_sha256sum("<html><body>404 Not Found</body></html>").is_err());
        let with_g = format!("g{}", &DIGEST_A[1..]);
        assert!(parse_sha256sum(&with_g).is_err());
    }

    // -- decide_if_update_needed ------------------------------------------

    #[test]
    fn identical_digest_early_exits() {
        assert_eq!(
            decide_if_update_needed(DIGEST_A, Some(DIGEST_A)),
            UpdateDecision::UpToDate
        );
    }

    #[test]
    fn differing_digest_needs_update() {
        assert_eq!(
            decide_if_update_needed(DIGEST_A, Some(DIGEST_B)),
            UpdateDecision::NeedsUpdate
        );
    }

    #[test]
    fn absent_applied_digest_is_unknown_not_stale() {
        // Must not collapse to NeedsUpdate: a first run whose on-disk database
        // is already current should not download 21 MB to learn nothing.
        assert_eq!(
            decide_if_update_needed(DIGEST_A, None),
            UpdateDecision::Unknown
        );
        assert_eq!(
            decide_if_update_needed(DIGEST_A, Some("")),
            UpdateDecision::Unknown
        );
        assert_eq!(
            decide_if_update_needed(DIGEST_A, Some("   ")),
            UpdateDecision::Unknown
        );
    }

    #[test]
    fn comparison_ignores_case_and_padding() {
        assert_eq!(
            decide_if_update_needed(DIGEST_A, Some(&DIGEST_A.to_uppercase())),
            UpdateDecision::UpToDate
        );
        assert_eq!(
            decide_if_update_needed(&format!("  {DIGEST_A}  "), Some(DIGEST_A)),
            UpdateDecision::UpToDate
        );
    }

    // -- next_source : sticky and monotonic --------------------------------

    #[test]
    fn first_attempt_stays_on_the_current_source() {
        // The heart of stickiness: attempt 0 never re-consults the primary.
        for current in 0..SOURCES.len() {
            assert_eq!(next_source(current, 0), Some(current));
        }
    }

    #[test]
    fn attempts_walk_forward_one_source_at_a_time() {
        assert_eq!(next_source(0, 1), Some(1));
        assert_eq!(next_source(0, 2), Some(2));
        assert_eq!(next_source(1, 2), Some(3));
    }

    #[test]
    fn exhausting_every_source_returns_none() {
        let last = SOURCES.len() - 1;
        // From the primary, the attempt that lands past the end is exhausted.
        assert_eq!(next_source(0, SOURCES.len() as u32), None);
        // From the last source, the very next attempt is exhausted.
        assert_eq!(next_source(last, 0), Some(last));
        assert_eq!(next_source(last, 1), None);
    }

    #[test]
    fn never_wraps_back_to_the_primary() {
        // A forward-only walk cannot return 0 once it has left it.
        for current in 1..SOURCES.len() {
            for attempt in 0..SOURCES.len() as u32 {
                if let Some(next) = next_source(current, attempt) {
                    assert!(
                        next >= current,
                        "failover moved backwards: {current} -> {next} at attempt {attempt}"
                    );
                }
            }
        }
    }

    #[test]
    fn out_of_range_or_absurd_input_never_panics() {
        assert_eq!(next_source(SOURCES.len(), 0), None);
        assert_eq!(next_source(usize::MAX, 0), None);
        assert_eq!(next_source(usize::MAX, 1), None);
        assert_eq!(next_source(0, u32::MAX), None);
    }

    // -- cycle_start_source ------------------------------------------------

    #[test]
    fn healthy_primary_pulls_the_pin_back() {
        assert_eq!(cycle_start_source(0, true), PRIMARY_SOURCE_INDEX);
        assert_eq!(cycle_start_source(3, true), PRIMARY_SOURCE_INDEX);
    }

    #[test]
    fn degraded_primary_keeps_the_previous_pin() {
        // Sticky across cycles: a mirror that worked is not abandoned on a hunch.
        assert_eq!(cycle_start_source(2, false), 2);
        assert_eq!(
            cycle_start_source(SOURCES.len() - 1, false),
            SOURCES.len() - 1
        );
    }

    #[test]
    fn corrupt_pin_falls_back_to_the_primary() {
        assert_eq!(
            cycle_start_source(SOURCES.len(), false),
            PRIMARY_SOURCE_INDEX
        );
        assert_eq!(cycle_start_source(usize::MAX, false), PRIMARY_SOURCE_INDEX);
    }

    // -- source table integrity -------------------------------------------

    #[test]
    fn table_is_well_formed() {
        assert!(!SOURCES.is_empty());
        assert_eq!(SOURCES[PRIMARY_SOURCE_INDEX].id, "gh-release");
        for (i, s) in SOURCES.iter().enumerate() {
            assert!(
                s.geoip.starts_with("https://"),
                "source {i} geoip not HTTPS"
            );
            assert!(
                s.geosite.starts_with("https://"),
                "source {i} geosite not HTTPS"
            );
            assert!(
                s.geoip_sum.starts_with("https://"),
                "source {i} geoip_sum not HTTPS"
            );
            assert!(
                s.geosite_sum.starts_with("https://"),
                "source {i} geosite_sum not HTTPS"
            );
            for (j, other) in SOURCES.iter().enumerate() {
                if i != j {
                    assert_ne!(s.id, other.id, "duplicate source id {}", s.id);
                }
            }
        }
    }

    #[test]
    fn every_shipped_source_is_meta_cubex_lineage() {
        // Wire in a different lineage and this test is the tripwire: it forces
        // the routing-behaviour change to be a deliberate decision.
        assert!(SOURCES.iter().all(|s| s.lineage == GeoLineage::MetaCubeX));
    }

    #[test]
    fn files_pair_each_artefact_with_its_own_sidecar() {
        for s in &SOURCES {
            let files = s.files();
            assert_eq!(files.len(), 2);
            assert_eq!(files[0].0, GeoFile::Geoip);
            assert_eq!(files[1].0, GeoFile::Geosite);
            // Guard against pasting the geoip URL into the geosite slot.
            for (kind, url, sum) in files {
                let stem = match kind {
                    GeoFile::Geoip => "geoip.dat",
                    GeoFile::Geosite => "geosite.dat",
                };
                assert!(url.ends_with(stem), "{} url does not end in {stem}", s.id);
                // Only the suffix is asserted, never `sum == url + ".sha256sum"`:
                // the primary's sidecar lives on the `release` branch while its
                // database is a release *asset*, so the two are deliberately not
                // on adjacent URLs.
                assert!(
                    sum.ends_with(&format!("{stem}.sha256sum")),
                    "{} sidecar does not describe {stem}",
                    s.id
                );
                assert_ne!(url, sum, "{} sidecar must not be the database url", s.id);
            }
        }
    }

    #[test]
    fn asset_names_match_what_mihomo_resolves() {
        assert_eq!(GeoFile::Geoip.asset_name(), "GeoIP.dat");
        assert_eq!(GeoFile::Geosite.asset_name(), "GeoSite.dat");
    }

    #[test]
    fn source_lookup_is_bounds_checked() {
        assert!(source(0).is_some());
        assert!(source(SOURCES.len() - 1).is_some());
        assert!(source(SOURCES.len()).is_none());
    }
}

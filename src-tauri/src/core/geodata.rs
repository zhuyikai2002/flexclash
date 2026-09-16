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

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Where every artefact of the refresh lives.
///
/// `kernel_*` is the directory mihomo was started with (`-d`): it resolves
/// `GeoIP.dat` / `GeoSite.dat` directly inside it, so those two paths are
/// mihomo's, not ours. `staging_*` is a directory we own, and the distinction
/// is load-bearing rather than cosmetic — see `mount_config` below.
pub struct GeoPaths {
    work_dir: PathBuf,
}

impl GeoPaths {
    pub fn new(work_dir: &Path) -> Self {
        Self {
            work_dir: work_dir.to_path_buf(),
        }
    }

    /// `<work_dir>/geodata.json` — the persisted status.
    pub fn status_file(&self) -> PathBuf {
        self.work_dir.join("geodata.json")
    }

    /// `<work_dir>/geodata-mount.yaml` — the transient config handed to
    /// mihomo's `PUT /configs` while the staging server is up. Written only
    /// for the duration of a refresh and deleted afterwards.
    pub fn mount_config(&self) -> PathBuf {
        self.work_dir.join("geodata-mount.yaml")
    }

    /// `<work_dir>/config.yaml` — the live profile mihomo is running.
    pub fn active_config(&self) -> PathBuf {
        self.work_dir.join("config.yaml")
    }

    /// Our own copy of a verified database.
    ///
    /// Deliberately **not** the file mihomo reads. `UpdateGeoIp` compares the
    /// hash of the file already on disk with the hash of what it downloaded
    /// and returns early — *before* registering its `defer ClearGeoIPCache()`
    /// — when they match. Landing the new bytes at mihomo's own path first
    /// would therefore make the cache clear never run, and the kernel would
    /// keep matching against the stale parsed matcher while every status
    /// indicator said "updated". Keeping the master copy elsewhere guarantees
    /// the hash mihomo computes differs from the one on disk.
    pub fn staging_file(&self, file: GeoFile) -> PathBuf {
        self.work_dir
            .join("geodata-staging")
            .join(file.asset_name())
    }

    /// The path mihomo resolves inside its `-d` directory.
    pub fn kernel_file(&self, file: GeoFile) -> PathBuf {
        self.work_dir.join(file.asset_name())
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        let staging = self.work_dir.join("geodata-staging");
        std::fs::create_dir_all(&staging)
            .map_err(|e| AppError::Io(format!("create {}: {e}", staging.display())))
    }
}

impl GeoFile {
    /// Both managed artefacts, in a stable order.
    pub fn all() -> [GeoFile; 2] {
        [GeoFile::Geoip, GeoFile::Geosite]
    }
}

// ---------------------------------------------------------------------------
// Hashing / atomic writes
// ---------------------------------------------------------------------------

/// Lowercase hex SHA-256 of `bytes`.
///
/// Lowercase because that is what every published `.sha256sum` uses and what
/// [`decide_if_update_needed`] normalises to.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        // Two lowercase hex digits per byte; `write!` into a String cannot
        // fail, but the `fmt::Write` result is still discarded explicitly.
        let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{byte:02x}"));
    }
    out
}

/// SHA-256 of a file, or `None` when it cannot be read.
///
/// Reads the whole file: the pair is ~21 MB, which is the same order as what
/// mihomo itself holds while validating, and streaming would buy nothing.
pub fn sha256_file(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|bytes| sha256_hex(&bytes))
}

/// Write `bytes` to `path` via a sibling temp file plus a rename.
///
/// `fs::rename` is atomic on the same volume, so a reader either sees the
/// whole old file or the whole new one — never a truncated database. Mihomo's
/// own `safeWrite` is a plain `os.WriteFile`; ours is deliberately stronger.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::Io(format!("create {}: {e}", parent.display())))?;
    }
    let tmp = path.with_extension(format!(
        "{}.fc-tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("tmp")
    ));
    std::fs::write(&tmp, bytes)
        .map_err(|e| AppError::Io(format!("write {}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        // Best effort: a leftover temp file is noise, not a failure to report.
        let _ = std::fs::remove_file(&tmp);
        AppError::Io(format!(
            "rename {} -> {}: {e}",
            tmp.display(),
            path.display()
        ))
    })
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// The digests we have actually applied, per artefact.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GeoDigests {
    pub geoip: Option<String>,
    pub geosite: Option<String>,
}

impl GeoDigests {
    /// The digest recorded for one artefact.
    pub fn get(&self, file: GeoFile) -> Option<&str> {
        match file {
            GeoFile::Geoip => self.geoip.as_deref(),
            GeoFile::Geosite => self.geosite.as_deref(),
        }
    }

    /// Record the digest now in force for one artefact.
    pub fn set(&mut self, file: GeoFile, digest: String) {
        match file {
            GeoFile::Geoip => self.geoip = Some(digest),
            GeoFile::Geosite => self.geosite = Some(digest),
        }
    }
}

/// Everything the UI and the next cycle need to know about the pipeline.
#[derive(Clone, Debug, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GeoDataStatus {
    /// When we last looked upstream, successfully or not.
    pub last_check_at: Option<DateTime<Utc>>,
    /// When the databases were last actually replaced.
    pub last_update_at: Option<DateTime<Utc>>,
    /// The `id` of the source that last served us, for display.
    pub active_source: Option<String>,
    /// That source's index — the sticky pin the next cycle starts from.
    pub source_index: u32,
    /// Digests of the databases currently in force.
    pub applied_sha256: GeoDigests,
    /// The last failure, cleared by the next success. Doubles as the
    /// "was the previous cycle clean?" input to [`cycle_start_source`].
    pub last_error: Option<String>,
}

pub fn read_status(path: &Path) -> GeoDataStatus {
    // A missing or unreadable status file is not an error: it is exactly the
    // first run. `Default` is the honest answer.
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn write_status(path: &Path, status: &GeoDataStatus) -> Result<()> {
    let body = serde_json::to_string_pretty(status)
        .map_err(|e| AppError::Geo(format!("serialise geodata status: {e}")))?;
    atomic_write(path, body.as_bytes())
}

/// What one refresh cycle did — returned to the caller of `refresh_geodata`
/// and rendered in the settings card.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GeoRefreshReport {
    /// Whether at least one database was actually replaced.
    pub updated: bool,
    /// The `id` of the source that served this cycle.
    pub source: Option<String>,
    pub geoip_sha256: Option<String>,
    pub geosite_sha256: Option<String>,
    /// Bytes the kernel pulled from the staging server (`0` when it fetched
    /// nothing — see the enabled-flags note on `patch_mount_config`).
    pub bytes_served: u64,
    /// How many `GET`s the staging server answered with a file.
    pub requests: u64,
    /// One-line summary for the UI.
    pub detail: String,
}

// ---------------------------------------------------------------------------
// Mount config
// ---------------------------------------------------------------------------

/// The two rules that keep our own loopback fetch off the proxy.
///
/// Load-bearing, not defensive noise. mihomo fetches geo data through
/// `component/http`, whose dialer hands the destination to `inner.HandleTcp`
/// — i.e. through the **tunnel's rule matcher** — before falling back to a
/// direct dial. A profile whose rules have no loopback entry lets
/// `127.0.0.1:<port>` fall through to its catch-all `MATCH,PROXY`, and the
/// fetch is then sent to a proxy node instead of to us. Remote nodes fail on
/// their own loopback; a node that happens to run on this very machine would
/// silently relay the request. Prepending a DIRECT rule removes the ambiguity.
///
/// Identical to what the bundled default already ships, and applied only to
/// the transient mount config — the profile on disk is never rewritten.
const LOOPBACK_BYPASS_RULES: [&str; 2] = [
    "IP-CIDR,127.0.0.0/8,DIRECT,no-resolve",
    "IP-CIDR6,::1/128,DIRECT,no-resolve",
];

/// Produce the config mihomo should run *while the staging server is up*:
/// the active profile with `geox-url` repointed at loopback.
///
/// A `None` URL leaves that entry exactly as the profile had it. That matters
/// because the kernel fetches **every** enabled geo artefact on one trigger:
/// pointing `geoip` at a server that does not serve `/GeoIP.dat` would turn a
/// geosite-only refresh into a `500`. Only entries we can actually serve —
/// plus `mmdb` / `asn`, which we deliberately never repoint: in `geodata-mode`
/// the MMDB path is unused, ASN only runs when the profile carries an ASN
/// rule, and neither is something we have verified or staged.
pub fn patch_mount_config(
    active_yaml: &str,
    geoip_url: Option<&str>,
    geosite_url: Option<&str>,
) -> Result<String> {
    let mut root: serde_yaml::Value = serde_yaml::from_str(active_yaml)
        .map_err(|e| AppError::Config(format!("active config is not valid YAML: {e}")))?;
    let mapping = root
        .as_mapping_mut()
        .ok_or_else(|| AppError::Config("active config root is not a YAML mapping".into()))?;

    let mut geox = mapping
        .get("geox-url")
        .and_then(|v| v.as_mapping())
        .cloned()
        .unwrap_or_default();
    for (key, url) in [("geoip", geoip_url), ("geosite", geosite_url)] {
        if let Some(url) = url {
            geox.insert(
                serde_yaml::Value::String(key.into()),
                serde_yaml::Value::String(url.into()),
            );
        }
    }
    mapping.insert(
        serde_yaml::Value::String("geox-url".into()),
        serde_yaml::Value::Mapping(geox),
    );

    ensure_loopback_bypass(mapping);

    serde_yaml::to_string(&root)
        .map_err(|e| AppError::Config(format!("serialise mount config: {e}")))
}

fn ensure_loopback_bypass(mapping: &mut serde_yaml::Mapping) {
    let Some(serde_yaml::Value::Sequence(rules)) = mapping.get_mut("rules") else {
        // No `rules:` sequence to prepend to. Nothing sensible to do — mihomo
        // will reject the config anyway if it has no rules at all.
        return;
    };
    let already_covered = rules.iter().any(|rule| {
        rule.as_str()
            .is_some_and(|line| line.contains("127.0.0.0/8") && line.contains("DIRECT"))
    });
    if already_covered {
        return;
    }
    let mut prefixed: Vec<serde_yaml::Value> = LOOPBACK_BYPASS_RULES
        .iter()
        .map(|line| serde_yaml::Value::String((*line).into()))
        .collect();
    prefixed.append(rules);
    *rules = prefixed;
}

// ---------------------------------------------------------------------------
// Transfer
// ---------------------------------------------------------------------------

/// One artefact that needs downloading, with the digest it must hash to.
#[derive(Clone, Debug)]
pub struct PlannedFile {
    pub sha256: String,
    pub url: String,
}

/// What a cycle intends to do.
#[derive(Clone, Debug)]
pub struct UpdatePlan {
    pub source_index: usize,
    pub geoip: Option<PlannedFile>,
    pub geosite: Option<PlannedFile>,
}

impl UpdatePlan {
    pub fn file(&self, file: GeoFile) -> Option<&PlannedFile> {
        match file {
            GeoFile::Geoip => self.geoip.as_ref(),
            GeoFile::Geosite => self.geosite.as_ref(),
        }
    }

    /// True when both databases already match upstream.
    pub fn is_noop(&self) -> bool {
        self.geoip.is_none() && self.geosite.is_none()
    }
}

/// A client with sane bounds for a 21 MB pair.
///
/// The timeout is per request rather than per transfer: the sidecars are
/// bytes and the databases are megabytes, and both must finish inside it.
pub fn geo_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| AppError::Geo(format!("build geo http client: {e}")))
}

const HTTP_TIMEOUT: Duration = Duration::from_secs(90);

/// A cap on what we will accept from a source. The real files are ~21 MB
/// combined; anything past this is a misbehaving endpoint, not a database.
const MAX_DOWNLOAD_BYTES: usize = 256 * 1024 * 1024;

async fn http_get_text(client: &reqwest::Client, url: &str) -> Result<String> {
    let bytes = http_get_bytes(client, url).await?;
    String::from_utf8(bytes).map_err(|e| AppError::Geo(format!("{url} is not UTF-8: {e}")))
}

async fn http_get_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Geo(format!("GET {url}: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(AppError::Geo(format!("GET {url}: HTTP {status}")));
    }
    let mut resp = resp;
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let chunk = resp
            .chunk()
            .await
            .map_err(|e| AppError::Geo(format!("read {url}: {e}")))?;
        let Some(chunk) = chunk else { break };
        if buf.len() + chunk.len() > MAX_DOWNLOAD_BYTES {
            return Err(AppError::Geo(format!(
                "{url} exceeded {MAX_DOWNLOAD_BYTES} bytes"
            )));
        }
        buf.extend_from_slice(&chunk);
    }
    if buf.is_empty() {
        return Err(AppError::Geo(format!("{url} returned an empty body")));
    }
    Ok(buf)
}

/// Ask one source what needs doing, without downloading any database.
///
/// Two cheap sidecar reads decide everything. Any failure — transport, HTTP
/// status, an unparseable checksum — fails the *source*, which is what makes
/// the caller's failover meaningful: we only move on when this upstream
/// cannot answer, never when it simply has nothing new.
pub async fn plan_update(
    client: &reqwest::Client,
    index: usize,
    status: &GeoDataStatus,
    paths: &GeoPaths,
) -> Result<UpdatePlan> {
    let src =
        source(index).ok_or_else(|| AppError::Geo(format!("source index {index} out of range")))?;
    let mut plan = UpdatePlan {
        source_index: index,
        geoip: None,
        geosite: None,
    };

    for file in GeoFile::all() {
        let Some((_, db_url, sum_url)) = src.files().into_iter().find(|(k, _, _)| *k == file)
        else {
            return Err(AppError::Geo(format!(
                "source {} has no entry for {file:?}",
                src.id
            )));
        };
        let raw = http_get_text(client, sum_url).await?;
        let published = parse_sha256sum(&raw)?;
        let applied = status.applied_sha256.get(file);

        match decide_if_update_needed(&published.sha256, applied) {
            UpdateDecision::UpToDate => continue,
            UpdateDecision::Unknown => {
                // No applied digest on record — almost always a first run on a
                // machine whose database mihomo already downloaded. Resolve it
                // against the file itself rather than pulling 21 MB to learn
                // that nothing changed.
                if sha256_file(&paths.kernel_file(file))
                    .is_some_and(|disk| disk.eq_ignore_ascii_case(&published.sha256))
                {
                    continue;
                }
            }
            UpdateDecision::NeedsUpdate => {}
        }

        let planned = PlannedFile {
            sha256: published.sha256,
            url: db_url.to_string(),
        };
        match file {
            GeoFile::Geoip => plan.geoip = Some(planned),
            GeoFile::Geosite => plan.geosite = Some(planned),
        }
    }

    Ok(plan)
}

/// Download a database and refuse it unless it hashes to `expected`.
///
/// Verification happens before anything is written anywhere, so a truncated
/// or substituted response cannot reach the staging directory at all.
pub async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    expected_sha256: &str,
) -> Result<Vec<u8>> {
    let bytes = http_get_bytes(client, url).await?;
    let got = sha256_hex(&bytes);
    if !got.eq_ignore_ascii_case(expected_sha256) {
        return Err(AppError::Geo(format!(
            "{url} failed verification: expected {expected_sha256}, got {got}"
        )));
    }
    Ok(bytes)
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

#[cfg(test)]
mod transfer_tests {
    use super::*;

    /// Same fixture digest the policy tests use.
    const DIGEST_A: &str = "e8fba6c888d7bf13b4cfcab2b4d3cf7c355a9a2f4d94ee34c9ccfc00730bb389";

    const ACTIVE: &str = "mixed-port: 7897\n\
                          mode: rule\n\
                          proxies:\n  - name: a\n    type: ss\n\
                          rules:\n  - MATCH,PROXY\n";

    fn mount(yaml: &str) -> serde_yaml::Value {
        serde_yaml::from_str(yaml).unwrap()
    }

    // -- sha256 ------------------------------------------------------------

    #[test]
    fn sha256_matches_the_published_test_vector() {
        // FIPS 180-2 / the canonical "abc" vector.
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // Lowercase, always — the digest is compared against `.sha256sum`
        // files and against `applied_sha256` from our own JSON.
        assert!(sha256_hex(b"abc").chars().all(|c| !c.is_ascii_uppercase()));
    }

    #[test]
    fn sha256_file_is_none_for_a_missing_path() {
        let missing = std::env::temp_dir().join("flexclash-no-such-geo-file.dat");
        assert_eq!(sha256_file(&missing), None);
    }

    // -- atomic_write ------------------------------------------------------

    #[test]
    fn atomic_write_replaces_content_and_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("fc-geo-at-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("GeoIP.dat");

        atomic_write(&target, b"first").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"first");

        // A second write must replace, not append.
        atomic_write(&target, b"second").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"second");

        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "GeoIP.dat")
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -- status ------------------------------------------------------------

    #[test]
    fn status_defaults_when_the_file_is_absent_or_corrupt() {
        let missing = std::env::temp_dir().join("flexclash-no-such-geodata.json");
        let status = read_status(&missing);
        assert!(status.last_check_at.is_none());
        assert_eq!(status.source_index, 0);
        assert_eq!(status.applied_sha256, GeoDigests::default());

        let dir = std::env::temp_dir().join(format!("fc-geo-st-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let corrupt = dir.join("geodata.json");
        std::fs::write(&corrupt, b"not json at all").unwrap();
        // A corrupt status file degrades to a first run rather than failing
        // the refresh outright.
        assert_eq!(read_status(&corrupt).source_index, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("fc-geo-rt-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("geodata.json");

        let mut status = GeoDataStatus {
            last_check_at: Some(Utc::now()),
            active_source: Some("jsdelivr-cdn".into()),
            source_index: 1,
            ..Default::default()
        };
        status
            .applied_sha256
            .set(GeoFile::Geosite, DIGEST_A.to_string());
        write_status(&path, &status).unwrap();

        let back = read_status(&path);
        assert_eq!(back.active_source.as_deref(), Some("jsdelivr-cdn"));
        assert_eq!(back.source_index, 1);
        assert_eq!(back.applied_sha256.geosite.as_deref(), Some(DIGEST_A));
        assert!(back.applied_sha256.geoip.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn digests_are_keyed_by_file() {
        let mut d = GeoDigests::default();
        d.set(GeoFile::Geoip, "aa".into());
        assert_eq!(d.get(GeoFile::Geoip), Some("aa"));
        assert_eq!(d.get(GeoFile::Geosite), None);
        d.set(GeoFile::Geosite, "bb".into());
        assert_eq!(d.geoip.as_deref(), Some("aa"));
        assert_eq!(d.geosite.as_deref(), Some("bb"));
    }

    // -- patch_mount_config ------------------------------------------------

    #[test]
    fn mount_config_points_geox_url_at_the_staging_server() {
        let out = patch_mount_config(
            ACTIVE,
            Some("http://127.0.0.1:5001/GeoIP.dat"),
            Some("http://127.0.0.1:5001/GeoSite.dat"),
        )
        .unwrap();
        let doc = mount(&out);
        let geox = doc
            .as_mapping()
            .unwrap()
            .get("geox-url")
            .and_then(|v| v.as_mapping())
            .expect("geox-url must be written");
        let get = |k: &str| {
            geox.get(serde_yaml::Value::String(k.into()))
                .and_then(|v| v.as_str())
        };
        assert_eq!(get("geoip"), Some("http://127.0.0.1:5001/GeoIP.dat"));
        assert_eq!(get("geosite"), Some("http://127.0.0.1:5001/GeoSite.dat"));
        // `mmdb` / `asn` are deliberately left alone — we have neither
        // verified nor staged them.
        assert_eq!(get("mmdb"), None);
        assert_eq!(get("asn"), None);
    }

    #[test]
    fn mount_config_preserves_the_rest_of_the_profile() {
        let out = patch_mount_config(
            ACTIVE,
            Some("http://127.0.0.1:1/a"),
            Some("http://127.0.0.1:1/b"),
        )
        .unwrap();
        assert!(out.contains("mixed-port: 7897"), "{out}");
        assert!(out.contains("name: a"), "proxies lost:\n{out}");
        assert!(out.contains("MATCH,PROXY"), "rules lost:\n{out}");
    }

    #[test]
    fn mount_config_prepends_the_loopback_bypass_when_absent() {
        let out = patch_mount_config(
            ACTIVE,
            Some("http://127.0.0.1:1/a"),
            Some("http://127.0.0.1:1/b"),
        )
        .unwrap();
        let doc = mount(&out);
        let rules = doc
            .as_mapping()
            .unwrap()
            .get("rules")
            .and_then(|v| v.as_sequence())
            .expect("rules preserved");
        // Our two bypass rules come first so they win over any catch-all.
        assert_eq!(
            rules[0].as_str(),
            Some("IP-CIDR,127.0.0.0/8,DIRECT,no-resolve")
        );
        assert_eq!(
            rules[1].as_str(),
            Some("IP-CIDR6,::1/128,DIRECT,no-resolve")
        );
        assert_eq!(rules[2].as_str(), Some("MATCH,PROXY"));
        assert_eq!(rules.len(), 3);
    }

    #[test]
    fn mount_config_does_not_duplicate_an_existing_loopback_rule() {
        let yaml = "mixed-port: 7897\n\
                    rules:\n  - IP-CIDR,127.0.0.0/8,DIRECT,no-resolve\n  - MATCH,PROXY\n";
        let out = patch_mount_config(
            yaml,
            Some("http://127.0.0.1:1/a"),
            Some("http://127.0.0.1:1/b"),
        )
        .unwrap();
        let doc = mount(&out);
        let rules = doc
            .as_mapping()
            .unwrap()
            .get("rules")
            .and_then(|v| v.as_sequence())
            .unwrap();
        // Unchanged: the profile already routes loopback direct.
        assert_eq!(rules.len(), 2);
        assert_eq!(
            rules[0].as_str(),
            Some("IP-CIDR,127.0.0.0/8,DIRECT,no-resolve")
        );
    }

    #[test]
    fn mount_config_without_a_rules_block_is_left_alone() {
        // No `rules:` sequence to prepend to — must not invent one, and must
        // not panic.
        let out = patch_mount_config(
            "mixed-port: 7897\n",
            Some("http://127.0.0.1:1/a"),
            Some("http://127.0.0.1:1/b"),
        )
        .unwrap();
        let doc = mount(&out);
        assert!(doc.as_mapping().unwrap().get("geox-url").is_some());
        assert!(doc.as_mapping().unwrap().get("rules").is_none());
    }

    #[test]
    fn mount_config_rejects_a_non_mapping_profile() {
        assert!(patch_mount_config("- just\n- a\n- list\n", Some("a"), Some("b")).is_err());
        assert!(patch_mount_config("", Some("a"), Some("b")).is_err());
    }

    #[test]
    fn mount_config_leaves_unserved_entries_alone() {
        // A geosite-only refresh must not repoint `geoip` at a server that
        // cannot serve it: the kernel fetches every enabled artefact on one
        // trigger, so a 404 there fails the whole cycle.
        let out = patch_mount_config(ACTIVE, None, Some("http://127.0.0.1:7/b")).unwrap();
        let doc = mount(&out);
        let geox = doc
            .as_mapping()
            .unwrap()
            .get("geox-url")
            .and_then(|v| v.as_mapping())
            .unwrap();
        let get = |k: &str| {
            geox.get(serde_yaml::Value::String(k.into()))
                .and_then(|v| v.as_str())
        };
        assert_eq!(get("geoip"), None, "geoip must be left as-is:\n{out}");
        assert_eq!(get("geosite"), Some("http://127.0.0.1:7/b"));
    }

    #[test]
    fn mount_config_replaces_preexisting_geox_url_values() {
        let yaml = "mixed-port: 7897\n\
                    geox-url:\n  geoip: https://example.invalid/geoip.dat\n  geosite: https://example.invalid/geosite.dat\n\
                    rules:\n  - MATCH,PROXY\n";
        let out = patch_mount_config(
            yaml,
            Some("http://127.0.0.1:9/a"),
            Some("http://127.0.0.1:9/b"),
        )
        .unwrap();
        let doc = mount(&out);
        let geox = doc
            .as_mapping()
            .unwrap()
            .get("geox-url")
            .and_then(|v| v.as_mapping())
            .unwrap();
        let get = |k: &str| {
            geox.get(serde_yaml::Value::String(k.into()))
                .and_then(|v| v.as_str())
        };
        assert_eq!(get("geoip"), Some("http://127.0.0.1:9/a"));
        assert_eq!(get("geosite"), Some("http://127.0.0.1:9/b"));
        assert!(
            !out.contains("example.invalid"),
            "stale url survived:\n{out}"
        );
    }

    // -- paths -------------------------------------------------------------

    #[test]
    fn staging_and_kernel_paths_never_collide() {
        let work = std::path::Path::new("C:/tmp/whatever");
        let paths = GeoPaths::new(work);
        for file in GeoFile::all() {
            let staging = paths.staging_file(file);
            let kernel = paths.kernel_file(file);
            assert_ne!(staging, kernel, "staging must not shadow mihomo's own path");
            assert!(staging.ends_with(file.asset_name()));
            assert!(kernel.ends_with(file.asset_name()));
            // The kernel path is a direct child of the work dir, which is
            // where mihomo resolves its databases.
            assert_eq!(kernel.parent(), Some(work));
            assert_ne!(staging.parent(), Some(work));
        }
    }
}

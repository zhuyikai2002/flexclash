// ============================================================================
// profile.rs — Profile CRUD on disk + YAML sanitisation.
// ============================================================================

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use uuid::Uuid;

use crate::error::AppError;
use crate::events::KERNEL_LOG;

// ----------------------------------------------------------------------------
// Wire types (serialised both to disk and to the frontend via Tauri)
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileMeta {
    /// Stable id (UUID v4 string) used in directory names and as the
    /// identifier in Tauri commands.
    pub id: String,
    /// Human-friendly name shown in the UI.
    pub name: String,
    /// Source URL (for subscriptions). Empty for pasted/imported-from-file.
    #[serde(default)]
    pub url: String,
    /// Node count (populated when the yaml is sanitised; `0` if unknown).
    #[serde(default)]
    pub node_count: u32,
    /// Last-write time (UTC, ISO-8601).
    pub updated_at: DateTime<Utc>,
    /// Absolute path to the on-disk yaml.
    pub file_path: String,
    /// Bytes used (download), if the subscription header reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_bytes: Option<u64>,
    /// Bytes remaining (total - used), if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_bytes: Option<u64>,
    /// Total quota, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    /// Expire timestamp (UTC), if the header reported one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expire_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileIndex {
    /// The id of the currently active profile, if any.
    pub active_id: Option<String>,
    /// All known profiles (newest first).
    pub profiles: Vec<ProfileMeta>,
}

// ----------------------------------------------------------------------------
// Reserved / sanitised fields
// ----------------------------------------------------------------------------

/// The address the frontend expects to find Mihomo's external-controller on.
/// MUST stay in sync with `src/services/clash.ts` `MIHOMO_BASE_URL`.
pub const RESERVED_CONTROLLER: &str = "127.0.0.1:9091";
pub const RESERVED_LOG_LEVEL: &str = "info";
pub const RESERVED_MIXED_PORT: u16 = 7897;
pub const RESERVED_ALLOW_LAN: bool = false;
pub const RESERVED_MODE: &str = "rule";

const RESERVED_CORS_ORIGINS: &[&str] = &["tauri://localhost", "http://localhost:5173"];
const RESERVED_CORS_METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];

// ----------------------------------------------------------------------------
// Storage
// ----------------------------------------------------------------------------

/// Owns the on-disk layout for profiles.
pub struct ProfileStorage {
    root: PathBuf,
    profiles_dir: PathBuf,
    active_config: PathBuf,
    index_file: PathBuf,
}

impl ProfileStorage {
    pub fn new(work_dir: &Path) -> Self {
        let profiles_dir = work_dir.join("profiles");
        let active_config = work_dir.join("config.yaml");
        let index_file = work_dir.join("index.json");
        Self {
            root: work_dir.to_path_buf(),
            profiles_dir,
            active_config,
            index_file,
        }
    }

    pub fn profiles_dir(&self) -> &Path { &self.profiles_dir }
    pub fn active_config(&self) -> &Path { &self.active_config }
    pub fn index_file(&self) -> &Path { &self.index_file }
    pub fn root(&self) -> &Path { &self.root }

    /// `<profiles_dir>/<id>/config.yaml`
    pub fn profile_yaml(&self, id: &str) -> PathBuf {
        self.profiles_dir.join(id).join("config.yaml")
    }

    /// Create `profiles/` if missing. Idempotent.
    pub fn ensure_dirs(&self) -> Result<(), AppError> {
        fs::create_dir_all(&self.profiles_dir).map_err(|e| {
            AppError::Io(format!("create profiles dir {}: {e}", self.profiles_dir.display()))
        })?;
        Ok(())
    }

    pub fn read_index(&self) -> Result<ProfileIndex, AppError> {
        if !self.index_file.exists() {
            return Ok(ProfileIndex::default());
        }
        let raw = fs::read_to_string(&self.index_file).map_err(|e| {
            AppError::Io(format!("read index.json: {e}"))
        })?;
        if raw.trim().is_empty() {
            return Ok(ProfileIndex::default());
        }
        let idx: ProfileIndex = serde_json::from_str(&raw).map_err(|e| {
            AppError::Config(format!("index.json not valid JSON: {e}"))
        })?;
        Ok(idx)
    }

    pub fn write_index(&self, idx: &ProfileIndex) -> Result<(), AppError> {
        let tmp = self.index_file.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(idx)
            .map_err(|e| AppError::Config(format!("serialise index: {e}")))?;
        fs::write(&tmp, body).map_err(|e| AppError::Io(format!("write tmp index: {e}")))?;
        fs::rename(&tmp, &self.index_file)
            .map_err(|e| AppError::Io(format!("rename index: {e}")))?;
        Ok(())
    }

    /// Patch the index in place, returning the new state.
    pub fn upsert_meta(&self, meta: ProfileMeta) -> Result<ProfileIndex, AppError> {
        let mut idx = self.read_index()?;
        if let Some(slot) = idx.profiles.iter_mut().find(|p| p.id == meta.id) {
            *slot = meta;
        } else {
            idx.profiles.insert(0, meta);
        }
        self.write_index(&idx)?;
        Ok(idx)
    }

    pub fn remove_meta(&self, id: &str) -> Result<ProfileIndex, AppError> {
        let mut idx = self.read_index()?;
        idx.profiles.retain(|p| p.id != id);
        if idx.active_id.as_deref() == Some(id) {
            idx.active_id = None;
        }
        self.write_index(&idx)?;
        Ok(idx)
    }
}

// ----------------------------------------------------------------------------
// Public CRUD
// ----------------------------------------------------------------------------

/// Sanitise a raw YAML and persist it as a new profile. Returns the resulting
/// `ProfileMeta`. Pass an existing `id` to overwrite.
pub fn save_profile(
    storage: &ProfileStorage,
    id: Option<String>,
    name: String,
    raw_yaml: String,
    url: String,
) -> Result<ProfileMeta, AppError> {
    storage.ensure_dirs()?;
    let id = id.unwrap_or_else(|| Uuid::new_v4().to_string());

    let sanitized = patch_and_sanitize_yaml(&raw_yaml)?;
    let node_count = count_nodes(&sanitized);

    let yaml_path = storage.profile_yaml(&id);
    if let Some(parent) = yaml_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            AppError::Io(format!("create profile dir {}: {e}", parent.display()))
        })?;
    }
    fs::write(&yaml_path, sanitized.as_bytes())
        .map_err(|e| AppError::Io(format!("write profile yaml: {e}")))?;

    let meta = ProfileMeta {
        id: id.clone(),
        name: if name.trim().is_empty() { "Untitled".into() } else { name },
        url,
        node_count,
        updated_at: Utc::now(),
        file_path: yaml_path.to_string_lossy().into_owned(),
        used_bytes: None,
        remaining_bytes: None,
        total_bytes: None,
        expire_at: None,
    };

    storage.upsert_meta(meta.clone())?;
    Ok(meta)
}

pub fn delete_profile(storage: &ProfileStorage, id: &str) -> Result<(), AppError> {
    let dir = storage.profiles_dir().join(id);
    if dir.exists() {
        fs::remove_dir_all(&dir)
            .map_err(|e| AppError::Io(format!("remove profile dir: {e}")))?;
    }
    let _ = storage.remove_meta(id);
    Ok(())
}

pub fn list_profiles(storage: &ProfileStorage) -> Result<Vec<ProfileMeta>, AppError> {
    Ok(storage.read_index()?.profiles)
}

pub fn get_active_profile(storage: &ProfileStorage) -> Result<Option<ProfileMeta>, AppError> {
    let idx = storage.read_index()?;
    let Some(active_id) = idx.active_id else { return Ok(None) };
    Ok(idx.profiles.into_iter().find(|p| p.id == active_id))
}

/// Mark a profile active in the index AND copy its yaml to the active path
/// (the file mihomo is started with). The Tauri command is responsible for
/// then calling `reloadConfig` on the running mihomo.
pub fn activate_profile(
    storage: &ProfileStorage,
    id: &str,
) -> Result<ProfileMeta, AppError> {
    let mut idx = storage.read_index()?;
    let Some(meta) = idx.profiles.iter().find(|p| p.id == id).cloned() else {
        return Err(AppError::Config(format!("profile id not found: {id}")));
    };
    let src = PathBuf::from(&meta.file_path);
    if !src.exists() {
        return Err(AppError::Config(format!(
            "profile yaml missing on disk: {}",
            src.display()
        )));
    }
    // Re-sanitise on activation so the fix lands immediately instead of on
    // the next restart: profiles written by older builds still carry a
    // subscription's inbound port, and mihomo would otherwise keep starting
    // on 7890 (or fail outright when another client already holds it).
    //
    // Best effort — a yaml we cannot parse is still copied through
    // untouched so mihomo itself reports the real error.
    if let Ok(raw) = fs::read_to_string(&src) {
        if let Ok(refreshed) = patch_and_sanitize_yaml(&raw) {
            if refreshed != raw {
                let _ = fs::write(&src, refreshed.as_bytes());
            }
        }
    }

    fs::copy(&src, storage.active_config()).map_err(|e| {
        AppError::Io(format!("copy profile -> active config: {e}"))
    })?;
    idx.active_id = Some(id.to_string());
    storage.write_index(&idx)?;
    Ok(meta)
}

// ----------------------------------------------------------------------------
// YAML sanitisation
// ----------------------------------------------------------------------------

/// Parse `raw_yaml`, force-overwrite a small set of FlexClash-reserved
/// top-level fields, and serialise the result back to YAML.
///
/// **Why we need this:** External subscriptions (机场) sometimes ship with
/// their own `external-controller` bound to `:9090` (or some other port), and
/// mihomo cannot be running two external-controllers on the same port. If we
/// were to load such a profile blindly, the Tauri frontend would lose contact
/// with mihomo. We therefore *unconditionally* overwrite the reserved fields,
/// regardless of what the subscription set.
///
/// Reserved fields (top-level keys):
///   - `external-controller`        : `127.0.0.1:9091`
///   - `external-controller-cors`   : tauri://localhost + http://localhost:5173
///   - `secret`                     : "" (no auth)
///   - `mixed-port`                 : 7897 (always; `port`/`socks-port` dropped)
///   - `allow-lan`                  : false
///   - `mode`                       : rule
///   - `log-level`                  : info
///   - `ipv6`                       : false
pub fn patch_and_sanitize_yaml(raw_yaml: &str) -> Result<String, AppError> {
    // Parse as generic Value so we don't care about structure.
    let mut root: serde_yaml::Value = serde_yaml::from_str(raw_yaml)
        .map_err(|e| AppError::Config(format!("profile yaml is not valid YAML: {e}")))?;

    let mapping = root
        .as_mapping_mut()
        .ok_or_else(|| AppError::Config("profile root is not a YAML mapping".into()))?;

    // ---- FORCE overwrite (these MUST be controlled by us) -----------------
    insert_str(mapping, "external-controller", RESERVED_CONTROLLER);
    insert_bool(mapping, "allow-lan", RESERVED_ALLOW_LAN);
    insert_str(mapping, "mode", RESERVED_MODE);
    insert_str(mapping, "log-level", RESERVED_LOG_LEVEL);
    insert_bool(mapping, "ipv6", false);
    insert_str(mapping, "secret", "");

    // CORS must be in place so the Tauri webview can call the API.
    inject_cors(mapping);

    // ---- FORCE the inbound port ------------------------------------------
    // `mixed-port` is the one inbound port FlexClash owns end to end: the
    // system proxy, the tray toggle and the frontend all resolve to
    // `RESERVED_MIXED_PORT` (7897). A subscription that ships
    // `mixed-port: 7890` — or the legacy `port:` / `socks-port:` pair on
    // 7890 / 7891 / 7892 — collides with any other Clash-family client
    // already running on the box and silently breaks the proxy. Unlike the
    // other reserved fields this one is therefore *always* rewritten, never
    // "fill in if absent".
    //
    // `mixed-port` carries both HTTP and SOCKS, so dropping the legacy pair
    // is functionally equivalent and removes two further collision points.
    drop_legacy_inbound_ports(mapping);
    insert_u64(mapping, "mixed-port", u64::from(RESERVED_MIXED_PORT));

    serde_yaml::to_string(&root)
        .map_err(|e| AppError::Config(format!("serialise sanitised yaml: {e}")))
}

fn insert_str(m: &mut serde_yaml::Mapping, k: &str, v: &str) {
    m.insert(serde_yaml::Value::String(k.into()), serde_yaml::Value::String(v.into()));
}
fn insert_bool(m: &mut serde_yaml::Mapping, k: &str, v: bool) {
    m.insert(serde_yaml::Value::String(k.into()), serde_yaml::Value::Bool(v));
}
fn insert_u64(m: &mut serde_yaml::Mapping, k: &str, v: u64) {
    m.insert(serde_yaml::Value::String(k.into()), serde_yaml::Value::Number(v.into()));
}

/// Remove the legacy `port:` / `socks-port:` inbound ports.
///
/// Rebuilt as "filter, then re-insert" rather than using a direct removal
/// because `serde_yaml::Mapping`'s lookup helpers are keyed by `Value`, and
/// this keeps the helper free of any `Index`-trait import. Key order is
/// preserved for everything that survives.
fn drop_legacy_inbound_ports(m: &mut serde_yaml::Mapping) {
    let kept: Vec<(serde_yaml::Value, serde_yaml::Value)> = m
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), Some("port" | "socks-port")))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if kept.len() == m.len() {
        return; // nothing to drop — avoid churning the mapping
    }
    m.clear();
    for (k, v) in kept {
        m.insert(k, v);
    }
}

fn inject_cors(m: &mut serde_yaml::Mapping) {
    let mut cors = serde_yaml::Mapping::new();
    let origins: serde_yaml::Sequence = RESERVED_CORS_ORIGINS
        .iter()
        .map(|s| serde_yaml::Value::String((*s).into()))
        .collect();
    let methods: serde_yaml::Sequence = RESERVED_CORS_METHODS
        .iter()
        .map(|s| serde_yaml::Value::String((*s).into()))
        .collect();
    cors.insert(
        serde_yaml::Value::String("allow-origins".into()),
        serde_yaml::Value::Sequence(origins),
    );
    cors.insert(
        serde_yaml::Value::String("allow-methods".into()),
        serde_yaml::Value::Sequence(methods),
    );
    cors.insert(
        serde_yaml::Value::String("allow-private-network".into()),
        serde_yaml::Value::Bool(true),
    );
    m.insert(
        serde_yaml::Value::String("external-controller-cors".into()),
        serde_yaml::Value::Mapping(cors),
    );
}

/// Best-effort node count from a sanitised yaml. Counts entries in
/// `proxies:` and flattens `proxy-groups:` children.
fn count_nodes(yaml: &str) -> u32 {
    let Ok(v) = serde_yaml::from_str::<serde_yaml::Value>(yaml) else { return 0 };
    let Some(map) = v.as_mapping() else { return 0 };
    let mut count = 0u32;
    if let Some(serde_yaml::Value::Sequence(seq)) = map.get("proxies") {
        count += seq.len() as u32;
    }
    if let Some(serde_yaml::Value::Sequence(groups)) = map.get("proxy-groups") {
        for g in groups {
            if let Some(serde_yaml::Value::Sequence(children)) =
                g.as_mapping().and_then(|m| m.get("proxies"))
            {
                count += children.len() as u32;
            }
        }
    }
    count
}

// ----------------------------------------------------------------------------
// TUN config injection (M9)
// ----------------------------------------------------------------------------

/// TUN-specific keys we stamp onto the active config when the user enables
/// transparent proxy. These values are the locked-in Mihomo v1.19.30
/// schema; the two *user-facing* switches live in [`TunAdvanced`].
pub const TUN_DEVICE: &str = "flexclash-tun";
pub const TUN_STACK: &str = "mixed";
pub const TUN_AUTO_DETECT_INTERFACE: bool = true;
/// `tun.dns-hijack` targets used when the user leaves "DNS Hijack" on.
///
/// Both entries are load-bearing. Mihomo treats a scheme-less target as
/// `udp://`, so the previous single-entry `["0.0.0.0:53"]` captured **only**
/// UDP: a resolver answering over TCP — which is exactly what a censoring
/// resolver falls back to — still reached the host's own resolver, i.e. a
/// silent DNS leak that `dns-hijack: true` purported to close. `any:53`
/// binds every local address (not just `0.0.0.0`) and the explicit
/// `tcp://` entry closes the TCP hole.
pub const TUN_DNS_HIJACK: &[&str] = &["any:53", "tcp://any:53"];
/// Loopback we want to skip in auto-route (otherwise the local API is
/// captured by TUN and loops back through mihomo).
pub const TUN_AUTO_ROUTE_EXCLUDE: &[&str] = &["127.0.0.0/8"];

/// The two user-facing switches from Settings -> "TUN Advanced".
///
/// Authored by the renderer (`src/stores/settings.ts`), threaded through
/// `enable_tun` / `apply_tun_advanced` down to [`toggle_tun_block`]. The
/// [`Default`] impl is the canonical fallback for any caller that does not
/// supply them, and must stay in sync with `TUN_ADVANCED_DEFAULTS` on the
/// frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TunAdvanced {
    /// `tun.strict-route` — reject traffic that would escape the tunnel
    /// instead of leaking it out of the physical interface.
    pub strict_route: bool,
    /// `tun.dns-hijack` — capture :53 over both UDP and TCP.
    pub dns_hijack: bool,
}

impl Default for TunAdvanced {
    fn default() -> Self {
        Self {
            // Off by default: the more opinionated of the two, and it
            // breaks LAN / nested-virtualisation setups.
            strict_route: false,
            // On by default: without it the host resolver sees every
            // domain the user visits, which defeats the tunnel.
            dns_hijack: true,
        }
    }
}

/// DNS block stamped onto the active config when TUN is enabled and the
/// config has no `dns:` block of its own.
///
/// **Why this is not optional.** Enabling `tun:` installs a `dns-hijack`,
/// which makes mihomo the resolver for the whole host. But `dns-hijack`
/// only *delivers* queries — with no `dns:` block mihomo runs with its DNS
/// server disabled, so every hijacked query dies inside mihomo. The
/// observable symptoms are exactly "TUN is on but nothing is routed":
/// no fake-ip is handed out, `nslookup` returns the real address, and
/// every domain-based rule in `rules:` stops matching.
///
/// The bundled `default_mihomo.yaml` shipped a `tun:` block but **no**
/// `dns:` block, so a fresh install (or any profile copied from a
/// subscription that omits `dns:`) landed in precisely that state.
///
/// A config that already carries its own `dns:` block is left untouched:
/// a subscription's `respect-rules` / `nameserver-policy` / rule-set
/// driven split resolution is strictly better informed than ours. We only
/// fill a hole — and on disable we remove the block again **only if it is
/// still byte-for-byte the one we inserted**, so toggling TUN round-trips
/// back to the original file.
pub const TUN_DNS_ENHANCED_MODE: &str = "fake-ip";
pub const TUN_DNS_FAKE_IP_RANGE: &str = "198.18.0.1/16";
pub const TUN_DNS_NAMESERVERS: &[&str] = &["223.5.5.5", "119.29.29.29"];

/// Result of `inject_tun_config` returned to the caller. The frontend never
/// sees this — it is consumed by `core::tun::TunManager`. `changed: true`
/// means a write to disk happened, `false` means the yaml was already in
/// the desired state.
#[derive(Debug, Clone, Serialize)]
pub struct TunPatchOutcome {
    pub changed: bool,
    pub enabled: bool,
    pub config_path: String,
}

/// Toggle the `tun:` block on the active config and persist it.
///
/// * `enable = true`  → insert (or replace) the `tun:` block with our
///   locked-in schema, with `strict-route` / `dns-hijack` stamped from
///   `advanced`. If a `tun:` block already exists, only the fields we own
///   are overwritten; user overrides such as `inet4-address` are preserved.
/// * `enable = false` → remove the `tun:` block entirely (`advanced` is
///   then irrelevant and only carried for call-site symmetry).
///
/// Reserved fields (external-controller, secret, mode, etc.) are *not*
/// touched here — call `patch_and_sanitize_yaml` separately if the
/// profile was just loaded.
pub fn inject_tun_config(
    storage: &ProfileStorage,
    enable: bool,
    advanced: TunAdvanced,
) -> Result<TunPatchOutcome, AppError> {
    let path = storage.active_config();
    let raw = if path.exists() {
        fs::read_to_string(path)?
    } else {
        // No active config yet: fall back to the bundled default. The
        // caller (TunManager) will sanitise it before activation.
        include_str!("../../resources/default_mihomo.yaml").to_string()
    };

    let patched = toggle_tun_block(&raw, enable, advanced)?;

    let changed = patched != raw;
    if changed {
        // Atomic write via tmp+rename to keep mihomo from reading a
        // half-written file if it does a hot-reload scan.
        let tmp = path.with_extension("yaml.tmp");
        fs::write(&tmp, patched.as_bytes())?;
        fs::rename(&tmp, path)?;
    }
    Ok(TunPatchOutcome {
        changed,
        enabled: enable,
        config_path: path.to_string_lossy().into_owned(),
    })
}

/// Pure function: toggle the `tun:` block in `yaml`. Exposed for unit
/// testing in `tests/tun_yaml.rs`.
pub fn toggle_tun_block(
    yaml: &str,
    enable: bool,
    advanced: TunAdvanced,
) -> Result<String, AppError> {
    let mut root: serde_yaml::Value = serde_yaml::from_str(yaml)
        .map_err(|e| AppError::Config(format!("active config not valid YAML: {e}")))?;
    let mapping = root
        .as_mapping_mut()
        .ok_or_else(|| AppError::Config("active config root is not a mapping".into()))?;

    if enable {
        // Preserve any user-overrides already inside `tun:` (e.g. a
        // `inet4-address: 10.0.0.1/30`).
        let existing_inner = mapping
            .get("tun")
            .and_then(|v| v.as_mapping())
            .cloned()
            .unwrap_or_default();

        let mut tun = existing_inner;
        insert_bool_map(&mut tun, "enable", true);
        insert_str_map(&mut tun, "stack", TUN_STACK);
        insert_str_map(&mut tun, "device", TUN_DEVICE);
        insert_bool_map(&mut tun, "auto-route", true);
        insert_bool_map(&mut tun, "auto-detect-interface", TUN_AUTO_DETECT_INTERFACE);
        // Documented explicitly in both directions rather than omitted when
        // off: a profile may ship its own `strict-route: true`, and "the
        // switch is off" has to mean false, not "leave whatever was there".
        insert_bool_map(&mut tun, "strict-route", advanced.strict_route);

        // dns-hijack: ["any:53", "tcp://any:53"] when on, `[]` when off.
        // The empty sequence is the explicit "do not capture :53" — writing
        // it (instead of deleting the key) keeps the outcome deterministic
        // even for a profile that shipped its own list.
        let hijack: serde_yaml::Sequence = if advanced.dns_hijack {
            TUN_DNS_HIJACK
                .iter()
                .map(|s| serde_yaml::Value::String((*s).into()))
                .collect()
        } else {
            serde_yaml::Sequence::new()
        };
        tun.insert(
            serde_yaml::Value::String("dns-hijack".into()),
            serde_yaml::Value::Sequence(hijack),
        );

        // auto-route-exclude: keep API + future self-conn off TUN.
        let exclude: serde_yaml::Sequence = TUN_AUTO_ROUTE_EXCLUDE
            .iter()
            .map(|s| serde_yaml::Value::String((*s).into()))
            .collect();
        tun.insert(
            serde_yaml::Value::String("auto-route-exclude".into()),
            serde_yaml::Value::Sequence(exclude),
        );

        mapping.insert(
            serde_yaml::Value::String("tun".into()),
            serde_yaml::Value::Mapping(tun),
        );
        // `dns-hijack` with no DNS server behind it is a black hole — fill
        // the hole when the profile left one (see the TUN_DNS_* docs above).
        inject_tun_dns(mapping, true);
    } else {
        mapping.remove("tun");
        inject_tun_dns(mapping, false);
    }

    serde_yaml::to_string(&root)
        .map_err(|e| AppError::Config(format!("serialise tun-patched yaml: {e}")))
}

fn insert_str_map(m: &mut serde_yaml::Mapping, k: &str, v: &str) {
    m.insert(serde_yaml::Value::String(k.into()), serde_yaml::Value::String(v.into()));
}
fn insert_bool_map(m: &mut serde_yaml::Mapping, k: &str, v: bool) {
    m.insert(serde_yaml::Value::String(k.into()), serde_yaml::Value::Bool(v));
}

/// The exact `dns:` mapping we stamp when the active config has none.
/// Built by one function so the "is the on-disk block still ours?" check on
/// disable compares against an identical structure rather than a re-listing
/// that can silently drift out of sync.
fn locked_dns_block() -> serde_yaml::Mapping {
    let mut dns = serde_yaml::Mapping::new();
    insert_bool_map(&mut dns, "enable", true);
    insert_str_map(&mut dns, "enhanced-mode", TUN_DNS_ENHANCED_MODE);
    insert_str_map(&mut dns, "fake-ip-range", TUN_DNS_FAKE_IP_RANGE);
    let ns: serde_yaml::Sequence = TUN_DNS_NAMESERVERS
        .iter()
        .map(|s| serde_yaml::Value::String((*s).into()))
        .collect();
    dns.insert(
        serde_yaml::Value::String("nameserver".into()),
        serde_yaml::Value::Sequence(ns),
    );
    dns
}

/// Add (enable) or remove (disable) the locked-in DNS block.
///
/// Asymmetric on purpose:
/// * `enable` **only fills a hole** — a profile that brings its own `dns:`
///   is never rewritten, because its `nameserver-policy` / rule-set split
///   resolution is better than anything we could invent.
/// * `disable` **only removes our own block** — if the on-disk mapping is
///   not byte-for-byte what `locked_dns_block()` produces, the user (or a
///   re-activated subscription) owns it and we leave it alone.
fn inject_tun_dns(m: &mut serde_yaml::Mapping, enable: bool) {
    if enable {
        if m.get("dns").is_none() {
            m.insert(
                serde_yaml::Value::String("dns".into()),
                serde_yaml::Value::Mapping(locked_dns_block()),
            );
        }
        return;
    }
    let ours = serde_yaml::Value::Mapping(locked_dns_block());
    let is_ours = m.get("dns").map(|v| *v == ours).unwrap_or(false);
    if is_ours {
        m.remove("dns");
    }
}

#[cfg(test)]
mod tun_yaml_tests {
    use super::*;

    /// Default switches: strict-route off, dns-hijack on (UDP **and** TCP).
    #[test]
    fn inject_tun_adds_block_when_missing() {
        let yaml = "mixed-port: 7897\nexternal-controller: 127.0.0.1:9091\n";
        let out = toggle_tun_block(yaml, true, TunAdvanced::default()).unwrap();
        assert!(out.contains("tun:"));
        assert!(out.contains("device: flexclash-tun"));
        assert!(out.contains("stack: mixed"));
        assert!(out.contains("auto-route: true"));
        assert!(out.contains("auto-detect-interface: true"));
        // The default is deliberately OFF — see `TunAdvanced::default()`.
        assert!(out.contains("strict-route: false"));
        // Both families are hijacked; a lone `0.0.0.0:53` only covered UDP.
        assert!(out.contains("- any:53"));
        assert!(out.contains("- tcp://any:53"));
        assert!(out.contains("enable: true"));
        // Reserved fields untouched.
        assert!(out.contains("external-controller: 127.0.0.1:9091"));
    }

    /// The two switches must actually reach the yaml, in both directions.
    #[test]
    fn inject_tun_honours_the_advanced_switches() {
        let yaml = "mixed-port: 7897\n";

        let on = toggle_tun_block(
            yaml,
            true,
            TunAdvanced { strict_route: true, dns_hijack: true },
        )
        .unwrap();
        assert!(on.contains("strict-route: true"), "{on}");
        assert!(on.contains("- any:53"), "{on}");
        assert!(on.contains("- tcp://any:53"), "{on}");

        let off = toggle_tun_block(
            yaml,
            true,
            TunAdvanced { strict_route: false, dns_hijack: false },
        )
        .unwrap();
        assert!(off.contains("strict-route: false"), "{off}");
        let doc: serde_yaml::Value = serde_yaml::from_str(&off).unwrap();
        let hijack = doc
            .as_mapping()
            .unwrap()
            .get("tun")
            .and_then(|t| t.as_mapping())
            .and_then(|t| t.get("dns-hijack"))
            .and_then(|v| v.as_sequence())
            .expect("dns-hijack must still be present while the switch is off");
        assert!(hijack.is_empty(), "dns-hijack must be empty when off: {hijack:?}");
    }

    /// Turning strict-route off has to *override* a profile that turned it
    /// on, otherwise the switch would silently lie about the live config.
    #[test]
    fn inject_tun_overrides_a_profile_strict_route() {
        let yaml = "mixed-port: 7897\ntun:\n  enable: false\n  strict-route: true\n";
        let out = toggle_tun_block(yaml, true, TunAdvanced::default()).unwrap();
        assert!(out.contains("strict-route: false"), "{out}");
    }

    #[test]
    fn inject_tun_toggle_off_removes_block() {
        let with = toggle_tun_block("mixed-port: 7897\n", true, TunAdvanced::default()).unwrap();
        let without = toggle_tun_block(&with, false, TunAdvanced::default()).unwrap();
        assert!(!without.contains("tun:"));
        assert!(!without.contains("flexclash-tun"));
    }

    #[test]
    fn inject_tun_round_trip_is_idempotent() {
        let yaml = "mixed-port: 7897\nexternal-controller: 127.0.0.1:9091\n";
        let once = toggle_tun_block(yaml, true, TunAdvanced::default()).unwrap();
        let twice = toggle_tun_block(&once, true, TunAdvanced::default()).unwrap();
        assert_eq!(once, twice, "toggling enable twice must not mutate the yaml");
    }

    #[test]
    fn inject_tun_preserves_user_overrides() {
        let mut yaml = String::from("mixed-port: 7897\n");
        yaml.push_str("tun:\n  enable: false\n  inet4-address: 10.0.0.1/30\n");
        let out = toggle_tun_block(&yaml, true, TunAdvanced::default()).unwrap();
        // Our lock-in keys should be on.
        assert!(out.contains("device: flexclash-tun"));
        // User override preserved.
        assert!(out.contains("inet4-address: 10.0.0.1/30"));
        // And enable is true.
        assert!(out.contains("enable: true"));
    }

    #[test]
    fn inject_tun_fills_a_missing_dns_block() {
        let yaml = "mixed-port: 7897\nexternal-controller: 127.0.0.1:9091\n";
        let out = toggle_tun_block(yaml, true, TunAdvanced::default()).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        let dns = doc
            .as_mapping()
            .unwrap()
            .get("dns")
            .expect("tun enable must fill a missing dns block")
            .as_mapping()
            .unwrap()
            .clone();
        let get = |k: &str| dns.get(serde_yaml::Value::String(k.into())).cloned();
        assert_eq!(get("enable").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(get("enhanced-mode").and_then(|v| v.as_str().map(String::from)), Some("fake-ip".into()));
        assert_eq!(get("fake-ip-range").and_then(|v| v.as_str().map(String::from)), Some("198.18.0.1/16".into()));
        let ns = get("nameserver").unwrap();
        let ns = ns.as_sequence().unwrap();
        assert_eq!(ns.len(), 2);
        assert_eq!(ns[0].as_str(), Some("223.5.5.5"));
        // Reserved fields still untouched.
        assert!(out.contains("external-controller: 127.0.0.1:9091"));
    }

    #[test]
    fn inject_tun_never_clobbers_a_profile_dns_block() {
        let yaml = "mixed-port: 7897\n\
                    dns:\n  enable: true\n  enhanced-mode: fake-ip\n  respect-rules: true\n\
                    \x20 nameserver:\n    - https://1.1.1.1/dns-query\n";
        let out = toggle_tun_block(yaml, true, TunAdvanced::default()).unwrap();
        assert!(out.contains("respect-rules: true"), "profile dns block was rewritten:\n{out}");
        assert!(out.contains("https://1.1.1.1/dns-query"), "profile nameserver lost:\n{out}");
        assert!(!out.contains("119.29.29.29"), "our nameserver leaked into a profile-owned dns block:\n{out}");
    }

    #[test]
    fn inject_tun_removes_only_its_own_dns_block() {
        // Ours goes away again on disable...
        let on = toggle_tun_block("mixed-port: 7897\n", true, TunAdvanced::default()).unwrap();
        let off = toggle_tun_block(&on, false, TunAdvanced::default()).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&off).unwrap();
        assert!(doc.as_mapping().unwrap().get("dns").is_none(), "our dns block survived disable:\n{off}");

        // ...but a profile-owned one must survive the round trip.
        let with_own = "mixed-port: 7897\ndns:\n  enable: true\n  nameserver:\n    - 223.5.5.5\n";
        let on2 = toggle_tun_block(with_own, true, TunAdvanced::default()).unwrap();
        let off2 = toggle_tun_block(&on2, false, TunAdvanced::default()).unwrap();
        assert!(off2.contains("nameserver:"), "profile-owned dns removed by disable:\n{off2}");
    }

    /// Regression guard for the actual defect: the bundled default is what a
    /// fresh install — and `reset_application` — writes to the active config.
    /// If it loses its `dns:` block, TUN enables but nothing resolves.
    #[test]
    fn bundled_default_config_carries_a_fake_ip_dns_block() {
        let bundled = include_str!("../../resources/default_mihomo.yaml");
        let doc: serde_yaml::Value = serde_yaml::from_str(bundled).unwrap();
        let dns = doc
            .as_mapping()
            .unwrap()
            .get("dns")
            .expect("default_mihomo.yaml must ship a dns: block")
            .as_mapping()
            .unwrap()
            .clone();
        let get = |k: &str| dns.get(serde_yaml::Value::String(k.into())).cloned();
        assert_eq!(get("enable").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            get("enhanced-mode").and_then(|v| v.as_str().map(String::from)),
            Some("fake-ip".into())
        );
        assert!(get("nameserver").is_some(), "default dns block has no nameserver");
        // And it must not be pointing at a fake-ip range that bypasses the
        // documented 198.18.0.0/15 test expectation.
        assert_eq!(
            get("fake-ip-range").and_then(|v| v.as_str().map(String::from)),
            Some("198.18.0.1/16".into())
        );
    }
}

// ----------------------------------------------------------------------------
// Misc
// ----------------------------------------------------------------------------

/// Best-effort append to the in-app kernel log channel. Used by commands
/// to surface subscription fetch progress.
pub fn log<S: Into<String>>(app: &tauri::AppHandle, line: S) {
    let _ = app.emit(KERNEL_LOG, line.into());
}

#[cfg(test)]
mod inbound_port_tests {
    use super::*;

    #[test]
    fn subscription_mixed_port_is_rewritten_to_reserved() {
        let raw = "mixed-port: 7890\nmode: rule\nproxies:\n  - name: a\n    type: ss\n";
        let out = patch_and_sanitize_yaml(raw).unwrap();
        assert!(out.contains("mixed-port: 7897"), "got:\n{out}");
        assert!(!out.contains("7890"), "stale 7890 survived:\n{out}");
        // the rewrite must not cost us any nodes
        assert!(out.contains("name: a"), "nodes lost:\n{out}");
    }

    #[test]
    fn legacy_port_and_socks_port_are_dropped() {
        let raw = "port: 7890\nsocks-port: 7891\nmixed-port: 7897\n";
        let out = patch_and_sanitize_yaml(raw).unwrap();
        assert!(out.contains("mixed-port: 7897"), "got:\n{out}");
        let doc: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        let m = doc.as_mapping().unwrap();
        let key = |k: &str| serde_yaml::Value::String(k.to_string());
        assert!(m.get(&key("port")).is_none(), "port survived:\n{out}");
        assert!(
            m.get(&key("socks-port")).is_none(),
            "socks-port survived:\n{out}"
        );
    }

    #[test]
    fn missing_inbound_port_gets_filled() {
        let out = patch_and_sanitize_yaml("mode: rule\n").unwrap();
        assert!(out.contains("mixed-port: 7897"), "got:\n{out}");
    }
}

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
pub const RESERVED_MIXED_PORT: u16 = 7890;
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
///   - `mixed-port`                 : 7890 (only if absent)
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

    // ---- fill-in-only if absent (don't trample subscription defaults) ----
    if !mapping.contains_key("mixed-port") && !mapping.contains_key("port")
        && !mapping.contains_key("socks-port") {
        insert_u64(mapping, "mixed-port", u64::from(RESERVED_MIXED_PORT));
    }

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
/// schema. Any change must be reflected in `verify-m9` (T2 unit test).
pub const TUN_DEVICE: &str = "flexclash-tun";
pub const TUN_STACK: &str = "mixed";
pub const TUN_AUTO_DETECT_INTERFACE: bool = true;
pub const TUN_STRICT_ROUTE: bool = true;
/// DNS hijack: rewrite all UDP/53 queries to local. Mihomo 1.19.30 expects
/// this list as plain `["0.0.0.0:53"]` (no scheme).
pub const TUN_DNS_HIJACK: &[&str] = &["0.0.0.0:53"];
/// Loopback we want to skip in auto-route (otherwise the local API is
/// captured by TUN and loops back through mihomo).
pub const TUN_AUTO_ROUTE_EXCLUDE: &[&str] = &["127.0.0.0/8"];

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
///   locked-in schema. If a `tun:` block already exists, only `enable`
///   and the device/stack/etc. fields are overwritten; user overrides
///   such as `inet4-address` are preserved.
/// * `enable = false` → remove the `tun:` block entirely.
///
/// Reserved fields (external-controller, secret, mode, etc.) are *not*
/// touched here — call `patch_and_sanitize_yaml` separately if the
/// profile was just loaded.
pub fn inject_tun_config(
    storage: &ProfileStorage,
    enable: bool,
) -> Result<TunPatchOutcome, AppError> {
    let path = storage.active_config();
    let raw = if path.exists() {
        fs::read_to_string(path)?
    } else {
        // No active config yet: fall back to the bundled default. The
        // caller (TunManager) will sanitise it before activation.
        include_str!("../../resources/default_mihomo.yaml").to_string()
    };

    let patched = toggle_tun_block(&raw, enable)?;

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
pub fn toggle_tun_block(yaml: &str, enable: bool) -> Result<String, AppError> {
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
        insert_bool_map(&mut tun, "strict-route", TUN_STRICT_ROUTE);

        // dns-hijack: ["0.0.0.0:53"]
        let hijack: serde_yaml::Sequence = TUN_DNS_HIJACK
            .iter()
            .map(|s| serde_yaml::Value::String((*s).into()))
            .collect();
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
    } else {
        mapping.remove("tun");
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

#[cfg(test)]
mod tun_yaml_tests {
    use super::*;

    #[test]
    fn inject_tun_adds_block_when_missing() {
        let yaml = "mixed-port: 7890\nexternal-controller: 127.0.0.1:9091\n";
        let out = toggle_tun_block(yaml, true).unwrap();
        assert!(out.contains("tun:"));
        assert!(out.contains("device: flexclash-tun"));
        assert!(out.contains("stack: mixed"));
        assert!(out.contains("auto-route: true"));
        assert!(out.contains("auto-detect-interface: true"));
        assert!(out.contains("strict-route: true"));
        assert!(out.contains("- 0.0.0.0:53"));
        assert!(out.contains("enable: true"));
        // Reserved fields untouched.
        assert!(out.contains("external-controller: 127.0.0.1:9091"));
    }

    #[test]
    fn inject_tun_toggle_off_removes_block() {
        let with = toggle_tun_block("mixed-port: 7890\n", true).unwrap();
        let without = toggle_tun_block(&with, false).unwrap();
        assert!(!without.contains("tun:"));
        assert!(!without.contains("flexclash-tun"));
    }

    #[test]
    fn inject_tun_round_trip_is_idempotent() {
        let yaml = "mixed-port: 7890\nexternal-controller: 127.0.0.1:9091\n";
        let once = toggle_tun_block(yaml, true).unwrap();
        let twice = toggle_tun_block(&once, true).unwrap();
        assert_eq!(once, twice, "toggling enable twice must not mutate the yaml");
    }

    #[test]
    fn inject_tun_preserves_user_overrides() {
        let mut yaml = String::from("mixed-port: 7890\n");
        yaml.push_str("tun:\n  enable: false\n  inet4-address: 10.0.0.1/30\n");
        let out = toggle_tun_block(&yaml, true).unwrap();
        // Our lock-in keys should be on.
        assert!(out.contains("device: flexclash-tun"));
        // User override preserved.
        assert!(out.contains("inet4-address: 10.0.0.1/30"));
        // And enable is true.
        assert!(out.contains("enable: true"));
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

// ============================================================================
// config/overrides.rs — subscription override pipeline.
//
// (The module is `overrides`, not `override`, because `override` is a Rust
// reserved keyword. The files it reads keep their natural spelling:
// `overrides/override.yaml`.)
//
// WHAT THIS IS FOR
// ----------------
// An airport subscription is a YAML document you do not control. Every time it
// is refreshed, anything you hand-tuned by editing the profile is thrown away —
// and the fix ("edit the file again, then remember to redo it next refresh") is
// not a fix. This module lets a user keep their edits in a *separate* file that
// is deep-merged into the subscription text **before it is written to disk**,
// so a refresh re-applies them automatically.
//
// LAYOUT (under `app_local_data_dir()`, i.e. %LOCALAPPDATA%\com.flexclash.app)
//
//     overrides/
//         override.yaml              <- global; applies to every profile
//         profiles/<name>.yaml       <- optional; that profile only
//
// Precedence is lowest-first: the global file is merged, then the per-profile
// one, so the per-profile file wins on conflicts. Both layers are optional and
// a missing file is not an event.
//
// The second layer is keyed by the profile's **display name**, not its id, and
// that is deliberate: `import_profile_url` mints the id at save time, so an
// id-keyed file could not exist for a profile the user has not imported yet.
// The name is the only handle both import and refresh have up front.
//
// MERGE SEMANTICS
// ---------------
// Mappings merge recursively. Sequences are policy-driven per top-level key:
//
//     rules          append         (the common case is "add my own rules")
//     proxy-groups   merge by name  (edit one group, keep the rest)
//     proxies        merge by name  (add a node, never drop the airport's)
//     anything else  replace
//
// `proxies` is in that table for a safety reason rather than a convenience one:
// with `replace`, a user who adds one self-hosted node would silently delete
// every node the airport sent, and the `proxy-groups` that reference them would
// then dangle. Merge-by-name is the only non-destructive semantic.
//
// FAILURE POLICY — the red line
// -----------------------------
// A broken override must never cost the user their subscription. If any layer
// fails to parse, fails to merge (type conflict), or the result fails to
// serialise, the **entire chain is discarded** and the pristine subscription
// text is returned unchanged. The only side effect is a warning.
//
// Whole-chain rather than per-layer on purpose: it makes the outcome binary —
// either your overrides applied or they did not, and the log says which file
// broke it. Salvaging a partially-merged document would mean silently shipping
// a config that is neither the airport's nor the user's.
//
// This module never returns `Err`. Every failure is folded into
// `OverrideOutcome::warnings`, because a caller that has to handle an error is
// a caller that might propagate it, and propagating it is exactly what would
// break the subscription update.
// ============================================================================

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_yaml::{Mapping, Value};
use tauri::{AppHandle, Manager, Runtime};

use crate::error::AppError;

/// Directory name under the app-local data dir.
pub const OVERRIDES_DIRNAME: &str = "overrides";
/// The global override file.
pub const GLOBAL_OVERRIDE_FILENAME: &str = "override.yaml";
/// Sub-directory holding the per-profile overrides.
pub const PROFILES_SUBDIR: &str = "profiles";

/// Resolve `<app_local_data_dir>/overrides`.
///
/// Mirrors `store::db_path_for` — one convention for "where does this app keep
/// its own data", rather than each feature inventing its own base.
pub fn dir_for<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, AppError> {
    let base = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Path(format!("app_local_data_dir: {e}")))?;
    Ok(base.join(OVERRIDES_DIRNAME))
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

/// Outcome of running the pipeline. Always carries usable YAML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideOutcome {
    /// What the caller should persist. Equal to the input when nothing applied
    /// or when the chain was discarded.
    pub yaml: String,
    /// Layers that were merged, in order (display paths).
    pub applied: Vec<String>,
    /// Non-fatal problems, in the order encountered. Non-empty with
    /// `applied.is_empty()` means the chain was discarded.
    pub warnings: Vec<String>,
}

impl OverrideOutcome {
    /// Pass the input through untouched, with no warnings.
    fn passthrough(yaml: &str) -> Self {
        Self {
            yaml: yaml.to_string(),
            applied: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// True iff an override file existed but was not used.
    pub fn degraded(&self) -> bool {
        self.applied.is_empty() && !self.warnings.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// Turn a profile display name into a filename.
///
/// Keeps Unicode alphanumerics (airport names are frequently Chinese, and
/// `\u{4e2d}\u{6587}` survives on NTFS), folds every other character to `-`, collapses runs, trims, and
/// truncates. Path separators, `..`, `:` and control characters therefore cannot
/// survive — a profile named `../../evil` cannot escape the overrides directory.
fn sanitize_profile_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_dash = false;
    for ch in name.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    let cut: String = trimmed.chars().take(64).collect();
    let cut = cut.trim_end_matches('-').to_string();
    if cut.is_empty() {
        return "profile".to_string();
    }
    // Windows reserves these device names in *any* directory and with *any*
    // extension, so `CON.yaml` cannot be created. Prefix rather than mangle the
    // name beyond recognition.
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED
        .iter()
        .any(|r| r.eq_ignore_ascii_case(&cut))
    {
        return format!("_{cut}");
    }
    cut
}

/// The candidate layers, lowest precedence first. Missing files are filtered
/// out here so the caller never has to think about them.
pub fn layers(dir: &Path, profile_name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let global = dir.join(GLOBAL_OVERRIDE_FILENAME);
    if global.is_file() {
        out.push(global);
    }
    let per_profile = dir
        .join(PROFILES_SUBDIR)
        .join(format!("{}.yaml", sanitize_profile_name(profile_name)));
    if per_profile.is_file() {
        out.push(per_profile);
    }
    out
}

// ---------------------------------------------------------------------------
// Merge
// ---------------------------------------------------------------------------

/// How a top-level sequence combines with the subscription's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeqPolicy {
    /// Overlay's list wins outright.
    Replace,
    /// Subscription's entries, then the overlay's.
    Append,
    /// Match entries on a mapping key; overlay entries replace same-key
    /// entries in place and new ones are appended.
    MergeByKey(&'static str),
}

fn seq_policy_for(key: &str) -> SeqPolicy {
    match key {
        "rules" => SeqPolicy::Append,
        "proxy-groups" => SeqPolicy::MergeByKey("name"),
        "proxies" => SeqPolicy::MergeByKey("name"),
        _ => SeqPolicy::Replace,
    }
}

/// True iff the two values are the same YAML kind.
///
/// Numbers do not distinguish int from float: `1` vs `1.5` is a legitimate
/// correction, not a mistake worth discarding an override over.
///
/// Written as an explicit `match` rather than `matches!` with a long `|` chain:
/// the alternation is easier to read, and it keeps the "tagged values pass
/// through" rule visibly ahead of the kind check.
fn compatible(base: &Value, overlay: &Value) -> bool {
    match (base, overlay) {
        // Tagged values are opaque to us; let them through.
        (Value::Tagged(_), _) | (_, Value::Tagged(_)) => true,
        (Value::Mapping(_), Value::Mapping(_))
        | (Value::Sequence(_), Value::Sequence(_))
        | (Value::String(_), Value::String(_))
        | (Value::Bool(_), Value::Bool(_))
        | (Value::Number(_), Value::Number(_))
        | (Value::Null, Value::Null) => true,
        _ => false,
    }
}

/// `key_path` is only used to make conflict messages locatable (`dns.nameserver`).
fn describe_conflict(key_path: &str, base: &Value, overlay: &Value) -> String {
    format!(
        "type conflict at `{key_path}`: airport sent {} but the override has {}",
        kind_name(base),
        kind_name(overlay)
    )
}

fn kind_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Sequence(_) => "a list",
        Value::Mapping(_) => "a mapping",
        Value::Tagged(_) => "a tagged value",
    }
}

/// Purely functional deep merge: never mutates either input.
///
/// Returns the conflict message instead of an error type because the only
/// handler is "discard the chain and log it".
fn merge_values(
    base: &Value,
    overlay: &Value,
    key_path: &str,
    warnings: &mut Vec<String>,
) -> Result<Value, String> {
    if !compatible(base, overlay) {
        return Err(describe_conflict(key_path, base, overlay));
    }

    match (base, overlay) {
        (Value::Mapping(b), Value::Mapping(o)) => {
            let mut merged = b.clone();
            for (k, ov) in o.iter() {
                let child_path = match k.as_str() {
                    // Non-string keys are legal YAML but not meaningful in a
                    // Clash config; treat the overlay as authoritative.
                    None => {
                        merged.insert(k.clone(), ov.clone());
                        continue;
                    }
                    Some(s) => {
                        if key_path.is_empty() {
                            s.to_string()
                        } else {
                            format!("{key_path}.{s}")
                        }
                    }
                };
                match b.get(k) {
                    None => {
                        merged.insert(k.clone(), ov.clone());
                    }
                    Some(bv) => {
                        let value = merge_values(bv, ov, &child_path, warnings)?;
                        // `Mapping::insert` keeps an existing key's position, so
                        // the subscription's key order survives the merge.
                        merged.insert(k.clone(), value);
                    }
                }
            }
            Ok(Value::Mapping(merged))
        }
        (Value::Sequence(b), Value::Sequence(o)) => {
            let policy = if key_path.contains('.') {
                // Only top-level keys carry a policy; anything nested replaces,
                // which avoids surprising merges deep inside e.g. a proxy entry.
                SeqPolicy::Replace
            } else {
                seq_policy_for(key_path)
            };
            Ok(Value::Sequence(merge_sequences(b, o, policy, key_path, warnings)?))
        }
        // Everything else: same kind, overlay wins.
        _ => Ok(overlay.clone()),
    }
}

fn merge_sequences(
    base: &[Value],
    overlay: &[Value],
    policy: SeqPolicy,
    key_path: &str,
    warnings: &mut Vec<String>,
) -> Result<Vec<Value>, String> {
    match policy {
        SeqPolicy::Replace => Ok(overlay.to_vec()),
        SeqPolicy::Append => {
            let mut out = base.to_vec();
            out.extend(overlay.iter().cloned());
            Ok(out)
        }
        SeqPolicy::MergeByKey(key) => {
            // Index the subscription's entries by their identity key. Entries
            // without one cannot be matched; they stay put and are never
            // removed, so nothing the airport sent can disappear.
            let mut positions: HashMap<String, usize> = HashMap::new();
            for (i, entry) in base.iter().enumerate() {
                if let Some(id) = entry.get(key).and_then(|v| v.as_str()) {
                    positions.insert(id.to_string(), i);
                }
            }

            let mut out = base.to_vec();
            for ov in overlay {
                let Some(id) = ov.get(key).and_then(|v| v.as_str()) else {
                    // Not matchable — append rather than reject. mihomo will
                    // report the malformed entry far better than we can.
                    out.push(ov.clone());
                    continue;
                };
                match positions.get(id).copied() {
                    Some(i) => {
                        out[i] =
                            merge_values(&out[i], ov, &format!("{key_path}[{id}]"), warnings)?;
                    }
                    None => {
                        positions.insert(id.to_string(), out.len());
                        out.push(ov.clone());
                    }
                }
            }
            Ok(out)
        }
    }
}

/// Parse one override file into a root mapping.
fn load_layer(path: &Path) -> Result<Mapping, String> {
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("{}: cannot read ({e})", path.display()))?;
    if raw.trim().is_empty() {
        // An empty file is almost certainly an unfinished edit, not an
        // instruction to wipe the config.
        return Err(format!("{}: file is empty", path.display()));
    }
    let value: Value = serde_yaml::from_str(&raw)
        .map_err(|e| format!("{}: invalid YAML ({e})", path.display()))?;
    match value {
        Value::Mapping(m) => Ok(m),
        other => Err(format!(
            "{}: root must be a mapping, found {}",
            path.display(),
            kind_name(&other)
        )),
    }
}

/// Merge the whole chain. `Err` means "discard everything and use the input".
fn merge_chain(
    raw_yaml: &str,
    layers: &[PathBuf],
    warnings: &mut Vec<String>,
) -> Result<(String, Vec<String>), String> {
    let mut merged: Value = serde_yaml::from_str(raw_yaml)
        .map_err(|e| format!("subscription is not valid YAML ({e})"))?;
    if !matches!(merged, Value::Mapping(_)) {
        return Err("subscription root is not a mapping".to_string());
    }

    let mut applied = Vec::new();
    for path in layers {
        let overlay = load_layer(path)?;
        merged = merge_values(
            &merged,
            &Value::Mapping(overlay),
            "",
            warnings,
        )?;
        applied.push(path.display().to_string());
    }

    let text = serde_yaml::to_string(&merged)
        .map_err(|e| format!("serialising the merged config failed ({e})"))?;
    Ok((text, applied))
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Deep-merge this profile's override layers into `raw_yaml`.
///
/// Never fails. On any problem the input is returned verbatim and
/// [`OverrideOutcome::warnings`] explains what went wrong, so the caller can log
/// it and carry on with the subscription update.
///
/// `profile_name` is the display name used to locate the per-profile layer —
/// see the module header for why it is not the id.
pub fn apply(dir: &Path, profile_name: &str, raw_yaml: &str) -> OverrideOutcome {
    let layers = layers(dir, profile_name);
    if layers.is_empty() {
        // The overwhelmingly common case — no filesystem touched beyond two
        // `is_file` checks, and no YAML re-serialisation of a large config.
        return OverrideOutcome::passthrough(raw_yaml);
    }

    let mut warnings = Vec::new();
    match merge_chain(raw_yaml, &layers, &mut warnings) {
        Ok((yaml, applied)) => OverrideOutcome {
            yaml,
            applied,
            warnings,
        },
        Err(fatal) => {
            warnings.push(format!(
                "{fatal} — overrides skipped, the subscription config is used as-is"
            ));
            OverrideOutcome {
                yaml: raw_yaml.to_string(),
                applied: Vec::new(),
                warnings,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const AIRPORT: &str = r#"
mixed-port: 7890
proxies:
  - { name: "HK-01", type: ss, server: 1.2.3.4, port: 8388, cipher: aes-128-gcm, password: x }
  - { name: "JP-01", type: ss, server: 5.6.7.8, port: 8388, cipher: aes-128-gcm, password: x }
proxy-groups:
  - name: PROXY
    type: select
    proxies: [HK-01, JP-01]
rules:
  - MATCH,DIRECT
"#;

    // --- pure merge -------------------------------------------------------

    fn merge(base: &str, overlay: &str) -> Result<String, String> {
        let b: Value = serde_yaml::from_str(base).unwrap();
        let o: Value = serde_yaml::from_str(overlay).unwrap();
        let mut w = Vec::new();
        let merged = merge_values(&b, &o, "", &mut w)?;
        Ok(serde_yaml::to_string(&merged).unwrap())
    }

    #[test]
    fn nested_mappings_are_merged_not_replaced() {
        let out = merge(
            "dns:\n  enable: true\n  nameserver: [1.1.1.1]\n",
            "dns:\n  enhanced-mode: fake-ip\n",
        )
        .unwrap();
        // The airport's keys survive alongside the override's.
        assert!(out.contains("enable: true"), "{out}");
        assert!(out.contains("1.1.1.1"), "{out}");
        assert!(out.contains("enhanced-mode: fake-ip"), "{out}");
    }

    #[test]
    fn scalar_of_the_same_kind_is_replaced() {
        let out = merge("mode: rule\n", "mode: global\n").unwrap();
        assert!(out.contains("mode: global"), "{out}");
        assert!(!out.contains("mode: rule"), "{out}");
    }

    #[test]
    fn rules_are_appended_not_replaced() {
        let out = merge(
            AIRPORT,
            "rules:\n  - DOMAIN-SUFFIX,example.com,PROXY\n",
        )
        .unwrap();
        assert!(out.contains("MATCH,DIRECT"), "airport rule kept: {out}");
        assert!(out.contains("DOMAIN-SUFFIX,example.com,PROXY"), "{out}");
        // Airport rule must come first: mihomo is first-match-wins, so an
        // appended rule cannot silently shadow the subscription's own rules.
        let airport_at = out.find("MATCH,DIRECT").unwrap();
        let ours_at = out.find("DOMAIN-SUFFIX").unwrap();
        assert!(airport_at < ours_at, "append order wrong: {out}");
    }

    #[test]
    fn proxy_group_is_edited_in_place_by_name() {
        let out = merge(
            AIRPORT,
            "proxy-groups:\n  - name: PROXY\n    type: url-test\n    url: http://x/generate_204\n",
        )
        .unwrap();
        let groups: Value = serde_yaml::from_str(&out).unwrap();
        let list = groups.get("proxy-groups").unwrap().as_sequence().unwrap();
        // Still one group — edited, not duplicated and not appended.
        assert_eq!(list.len(), 1, "{out}");
        assert_eq!(list[0].get("type").unwrap().as_str(), Some("url-test"));
        // Deep-merged, so the airport's `proxies:` list survived inside it.
        assert!(list[0].get("proxies").is_some(), "{out}");
    }

    #[test]
    fn new_proxy_group_is_appended() {
        let out = merge(
            AIRPORT,
            "proxy-groups:\n  - name: MINE\n    type: select\n    proxies: [HK-01]\n",
        )
        .unwrap();
        let groups: Value = serde_yaml::from_str(&out).unwrap();
        let list = groups.get("proxy-groups").unwrap().as_sequence().unwrap();
        assert_eq!(list.len(), 2, "{out}");
        assert!(out.contains("MINE"), "{out}");
        assert!(out.contains("PROXY"), "{out}");
    }

    #[test]
    fn proxies_merge_by_name_so_an_added_node_never_drops_the_airports() {
        let out = merge(
            AIRPORT,
            "proxies:\n  - { name: \"Self\", type: ss, server: 9.9.9.9, port: 1, cipher: aes-128-gcm, password: y }\n",
        )
        .unwrap();
        let root: Value = serde_yaml::from_str(&out).unwrap();
        let list = root.get("proxies").unwrap().as_sequence().unwrap();
        assert_eq!(list.len(), 3, "2 airport nodes + 1 added: {out}");
        assert!(out.contains("HK-01") && out.contains("Self"), "{out}");
    }

    #[test]
    fn type_conflict_is_reported() {
        // `dns` is a mapping in the override but a scalar in the subscription.
        let err = merge("dns: off\n", "dns:\n  enable: true\n").unwrap_err();
        assert!(err.contains("type conflict"), "{err}");
        assert!(err.contains("dns"), "{err}");
    }

    #[test]
    fn conflict_message_names_the_nested_path() {
        let err = merge(
            "dns:\n  nameserver: [1.1.1.1]\n",
            "dns:\n  nameserver: fast\n",
        )
        .unwrap_err();
        assert!(err.contains("dns.nameserver"), "{err}");
    }

    #[test]
    fn tagged_values_are_tolerated() {
        // serde_yaml surfaces `!include foo` as Tagged; it must not be a
        // conflict, whatever it merges against.
        let out = merge("a: 1\n", "b: !include other.yaml\n").unwrap();
        assert!(out.contains("other.yaml"), "{out}");
    }

    // --- filename sanitisation -------------------------------------------

    #[test]
    fn sanitize_keeps_unicode_letters() {
        assert_eq!(sanitize_profile_name("机场 一号"), "机场-一号");
    }

    #[test]
    fn sanitize_blocks_path_traversal() {
        assert_eq!(sanitize_profile_name("../../evil"), "evil");
        assert_eq!(sanitize_profile_name("a/b\\c"), "a-b-c");
        assert_eq!(sanitize_profile_name(".."), "profile");
        assert_eq!(sanitize_profile_name(""), "profile");
    }

    #[test]
    fn sanitize_folds_runs_and_trims() {
        assert_eq!(sanitize_profile_name("  a ... b  "), "a-b");
        assert!(!sanitize_profile_name("a\u{0}b").contains('\u{0}'));
    }

    #[test]
    fn sanitize_escapes_windows_device_names() {
        assert_eq!(sanitize_profile_name("CON"), "_CON");
        assert_eq!(sanitize_profile_name("nul"), "_nul");
    }

    #[test]
    fn sanitize_truncates() {
        let long = "a".repeat(200);
        let out = sanitize_profile_name(&long);
        assert_eq!(out.len(), 64);
    }

    // --- filesystem-backed --------------------------------------------------

    /// A scratch dir that cleans itself up. Not `tempfile` — the crate is not a
    /// dependency and a UUID suffix is enough to keep parallel tests apart.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "flexclash-override-test-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn write(&self, rel: &str, body: &str) -> PathBuf {
            let p = self.0.join(rel);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&p, body).unwrap();
            p
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn no_override_files_is_a_clean_passthrough() {
        let s = Scratch::new();
        let out = apply(&s.0, "Airport", AIRPORT);
        assert_eq!(out.yaml, AIRPORT);
        assert!(out.applied.is_empty());
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert!(!out.degraded());
    }

    #[test]
    fn global_and_per_profile_layers_both_apply_per_profile_wins() {
        let s = Scratch::new();
        s.write(GLOBAL_OVERRIDE_FILENAME, "dns:\n  enable: true\n  nameserver: [1.1.1.1]\n");
        s.write("profiles/Airport.yaml", "dns:\n  nameserver: [9.9.9.9]\n");

        let out = apply(&s.0, "Airport", AIRPORT);
        assert_eq!(out.applied.len(), 2, "{:?}", out.applied);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);

        let root: Value = serde_yaml::from_str(&out.yaml).unwrap();
        let dns = root.get("dns").unwrap();
        assert_eq!(dns.get("enable").unwrap().as_bool(), Some(true));
        // Per-profile layer is merged last, so it overrides the global one.
        let ns = dns.get("nameserver").unwrap().as_sequence().unwrap();
        assert_eq!(ns.len(), 1);
        assert_eq!(ns[0].as_str(), Some("9.9.9.9"));
    }

    #[test]
    fn per_profile_layer_does_not_leak_to_other_profiles() {
        let s = Scratch::new();
        s.write("profiles/Airport.yaml", "dns:\n  enable: false\n");
        let out = apply(&s.0, "Other", AIRPORT);
        assert!(out.applied.is_empty());
        assert_eq!(out.yaml, AIRPORT);
    }

    /// The red line: a syntactically broken override must cost nothing but a
    /// warning, and the subscription text must be preserved byte-for-byte.
    #[test]
    fn invalid_yaml_override_degrades_to_the_original_config() {
        let s = Scratch::new();
        s.write(GLOBAL_OVERRIDE_FILENAME, "dns: [unclosed\n  bad: : :\n");
        let out = apply(&s.0, "Airport", AIRPORT);
        assert_eq!(out.yaml, AIRPORT, "must fall back to the airport config");
        assert!(out.applied.is_empty());
        assert!(out.degraded());
        assert!(
            out.warnings.iter().any(|w| w.contains("invalid YAML")),
            "{:?}",
            out.warnings
        );
    }

    #[test]
    fn type_conflict_degrades_the_whole_chain() {
        let s = Scratch::new();
        // Valid global layer…
        s.write(GLOBAL_OVERRIDE_FILENAME, "log-level: warning\n");
        // …but the per-profile layer conflicts with the subscription.
        s.write("profiles/Airport.yaml", "proxy-groups: not-a-list\n");

        let out = apply(&s.0, "Airport", AIRPORT);
        assert_eq!(out.yaml, AIRPORT, "whole chain must be discarded");
        assert!(out.applied.is_empty(), "nothing may be reported as applied");
        assert!(out.degraded());
        assert!(
            out.warnings.iter().any(|w| w.contains("type conflict")),
            "{:?}",
            out.warnings
        );
    }

    #[test]
    fn empty_override_file_degrades_rather_than_wiping_the_config() {
        let s = Scratch::new();
        s.write(GLOBAL_OVERRIDE_FILENAME, "   \n");
        let out = apply(&s.0, "Airport", AIRPORT);
        assert_eq!(out.yaml, AIRPORT);
        assert!(out.degraded());
        assert!(out.warnings.iter().any(|w| w.contains("empty")), "{:?}", out.warnings);
    }

    #[test]
    fn non_mapping_override_root_degrades() {
        let s = Scratch::new();
        s.write(GLOBAL_OVERRIDE_FILENAME, "- just\n- a list\n");
        let out = apply(&s.0, "Airport", AIRPORT);
        assert_eq!(out.yaml, AIRPORT);
        assert!(out.degraded());
        assert!(
            out.warnings.iter().any(|w| w.contains("root must be a mapping")),
            "{:?}",
            out.warnings
        );
    }

    /// A degraded run must leave a message naming the offending file, or the
    /// user has no way to find the typo.
    #[test]
    fn warnings_name_the_offending_file() {
        let s = Scratch::new();
        s.write(GLOBAL_OVERRIDE_FILENAME, "dns: [unclosed\n");
        let out = apply(&s.0, "Airport", AIRPORT);
        assert!(
            out.warnings.iter().any(|w| w.contains(GLOBAL_OVERRIDE_FILENAME)),
            "{:?}",
            out.warnings
        );
    }

    /// End-to-end: the merged document must still be parseable and must keep
    /// the airport's nodes.
    #[test]
    fn merged_output_is_valid_yaml_and_keeps_the_providers_nodes() {
        let s = Scratch::new();
        s.write(
            GLOBAL_OVERRIDE_FILENAME,
            "rules:\n  - GEOIP,CN,DIRECT\ndns:\n  enable: true\n  enhanced-mode: fake-ip\ntun:\n  enable: true\n  stack: mixed\nproxy-groups:\n  - name: PROXY\n    type: url-test\n    url: http://www.gstatic.com/generate_204\n",
        );
        let out = apply(&s.0, "Airport", AIRPORT);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(out.applied.len(), 1);

        let root: Value = serde_yaml::from_str(&out.yaml).expect("output must parse");
        assert_eq!(root.get("proxies").unwrap().as_sequence().unwrap().len(), 2);
        assert_eq!(
            root.get("dns").unwrap().get("enhanced-mode").unwrap().as_str(),
            Some("fake-ip")
        );
        assert_eq!(
            root.get("tun").unwrap().get("stack").unwrap().as_str(),
            Some("mixed")
        );
        let rules = root.get("rules").unwrap().as_sequence().unwrap();
        assert_eq!(rules.len(), 2, "MATCH,DIRECT + GEOIP,CN,DIRECT");
        let group = &root.get("proxy-groups").unwrap().as_sequence().unwrap()[0];
        assert_eq!(group.get("type").unwrap().as_str(), Some("url-test"));
        assert!(group.get("proxies").is_some(), "airport children kept");
    }
}

//! config — Profile (yaml) management + subscription import.
//!
//! Layout under the app-local data dir:
//!
//! ```text
//! <app_local_data>/com.flexclash.app/mihomo/
//!     config.yaml                 <- what the running mihomo reads
//!     profiles/
//!         <id>/config.yaml        <- one sub-dir per profile
//!         <id>/...
//!     index.json                  <- ProfileIndex: { active_id, profiles: [ProfileMeta] }
//! ```
//!
//! The `<id>/config.yaml` files are sanitised on save — `external-controller`
//! and a few other FlexClash-critical fields are force-overwritten so a hostile
//! subscription cannot sever the frontend ↔ mihomo API channel.
//!
//! Overrides live one level *up*, as a sibling of `mihomo/` rather than inside
//! it, because they are user-authored input rather than mihomo runtime state:
//!
//! ```text
//! <app_local_data>/com.flexclash.app/
//!     mihomo/                     <- mihomo's own data (wiped by a reset)
//!     overrides/                  <- user hand-written (never wiped by a reset)
//!         override.yaml
//!         profiles/<name>.yaml
//! ```
//!
//! See [`overrides`] for the merge semantics and the degradation policy.

pub mod overrides;
pub mod profile;
pub mod subscription;

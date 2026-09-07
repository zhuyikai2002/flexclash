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

pub mod profile;
pub mod subscription;

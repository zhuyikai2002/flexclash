// ============================================================================
// commands/geodata.rs — the geo-data refresh orchestrator (v0.4.x).
//
// This is the only place the three halves of the pipeline meet: the policy +
// transfer helpers in `core::geodata`, the throwaway loopback server in
// `core::geo_staging`, and the kernel's REST surface in `commands::mihomo`.
//
// WHY THE SEQUENCE IS WHAT IT IS
// ------------------------------
// The kernel caches *parsed* geo matchers in permanent `singleflight` groups
// (`component/geodata/utils.go`) and exposes no REST entry point to clear
// them. Only its own updater calls `ClearGeoIPCache()` / `ClearGeoSiteCache()`
// and it registers those as a `defer` on the one branch of `UpdateGeoIp` /
// `UpdateGeoSite` that actually downloaded something. Two consequences drive
// every step below, and both are easy to get wrong in a way that looks like
// success:
//
//  1. `if oldHash.Equal(hash) { return nil }` — the early return — sits
//     *before* that `defer`. If the bytes on disk already equal what the
//     kernel downloads, it returns without clearing the cache, the rules keep
//     matching against the stale parsed matcher, and nothing anywhere reports
//     a problem. So the verified master copy must NOT live at the path the
//     kernel reads (`GeoPaths::staging_file`), and a cycle that finds them
//     already equal makes the file briefly absent instead.
//
//  2. The only way to make the kernel download is `POST /configs/geo`, and the
//     URL it fetches is whatever `geox-url` currently holds. That value lives
//     in the in-memory config, so repointing it needs `PUT /configs` with a
//     whole config — there is no partial-update route for it.
//
// Hence: verify -> stage -> serve on loopback -> mount -> trigger -> restore.
// The mount is deliberately transient, and the profile on disk is never
// touched.
//
// ONE MORE TRAP: NOT EVERY PROFILE FETCHES
// ----------------------------------------
// `InitGeoIP` / `InitGeoSite` / `InitASN` — which is what makes `POST
// /configs/geo` do anything at all — are called from *rule construction*
// (`rules/common/geoip.go` etc). A profile with no GEOIP/GEOSITE rule
// therefore answers the trigger with a cheerful `204` and fetches nothing.
// That is not a failure (nothing consumes those databases), but it does mean
// the on-disk copy would otherwise stay stale, so the verified bytes are
// written locally afterwards. Step 8 explains why that cannot leave a stale
// matcher behind.
// ============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tauri_specta::Event;

use crate::commands::mihomo;
use crate::core::geo_staging::{StagedFile, StagingServer};
use crate::core::geodata::{
    self, cycle_start_source, next_source, GeoDataStatus, GeoFile, GeoPaths, GeoRefreshReport,
    UpdatePlan, SOURCES,
};
use crate::core::kernel_events::GeoDataUpdatedPayload;
use crate::core::sidecar;
use crate::error::AppError;

type CmdResult<T> = Result<T, AppError>;

/// How long the kernel is given to fetch the pair and adopt it. Generous on
/// purpose: one trigger downloads ~21 MB and re-parses it, and the kernel's
/// own per-request budget is 20 s.
const TRIGGER_TIMEOUT_MS: u64 = 180_000;

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Current state of the refresh pipeline. Cheap: one small JSON read.
#[tauri::command]
#[specta::specta]
pub async fn get_geodata_status(app: tauri::AppHandle) -> CmdResult<GeoDataStatus> {
    let work_dir = sidecar::work_dir_for(&app)?;
    Ok(geodata::read_status(
        &GeoPaths::new(&work_dir).status_file(),
    ))
}

/// Run one refresh cycle now.
///
/// The scheduler will call the same routine in a later step; exposing it as a
/// command is what lets the user force a check without waiting for the tick,
/// and lets the pipeline be exercised end to end without a timer in the way.
#[tauri::command]
#[specta::specta]
pub async fn refresh_geodata(app: tauri::AppHandle) -> CmdResult<GeoRefreshReport> {
    refresh(&app).await
}

// ---------------------------------------------------------------------------
// The cycle
// ---------------------------------------------------------------------------

/// Called once per cycle with `(status, bytes_served, requests, updated)`.
///
/// The cycle is written against this sink rather than an `AppHandle` so it can
/// be driven end to end by the head-less smoke harness
/// (`src/bin/geodata-smoke.rs`), which has no renderer to emit to. The command
/// layer's only contribution is the work dir and the event publication.
pub type EventSink<'a> = &'a (dyn Fn(&GeoDataStatus, u64, u64, bool) + Send + Sync);

async fn refresh(app: &tauri::AppHandle) -> CmdResult<GeoRefreshReport> {
    let work_dir = sidecar::work_dir_for(app)?;
    let sink = event_sink(app);
    run_refresh(&work_dir, &sink).await
}

/// The sink every in-app caller of [`run_refresh`] wants: publish the typed
/// event to the renderer and swallow the result.
///
/// A failed emit is not a refresh failure — a head-less or pre-window run may
/// legitimately have no listener.
fn event_sink(
    app: &tauri::AppHandle,
) -> impl Fn(&GeoDataStatus, u64, u64, bool) + Send + Sync + '_ {
    move |status, bytes_served, requests, updated| {
        emit(app, status, bytes_served, requests, updated);
    }
}

/// One cycle against an explicit work dir.
///
/// Split out of [`refresh`] purely so it is reachable without a live
/// `AppHandle`; the sequence below is the whole pipeline and is described in
/// the module header.
pub async fn run_refresh(work_dir: &Path, on_event: EventSink<'_>) -> CmdResult<GeoRefreshReport> {
    let paths = GeoPaths::new(work_dir);
    paths.ensure_dirs()?;

    let status_path = paths.status_file();
    let mut status = geodata::read_status(&status_path);
    let checked_at = Utc::now();
    let client = geodata::geo_client()?;

    // ---- 1. decide, degrading forward only ------------------------------
    // A cycle starts from the pin it is already on unless the previous one
    // was clean, in which case the primary is re-probed. Within the cycle the
    // walk is strictly forward, so a source that just failed is not retried
    // on a hunch.
    let start = cycle_start_source(status.source_index as usize, status.last_error.is_none());
    let mut plan: Option<UpdatePlan> = None;
    let mut failures: Vec<String> = Vec::new();
    for attempt in 0..SOURCES.len() as u32 {
        let Some(index) = next_source(start, attempt) else {
            break;
        };
        match geodata::plan_update(&client, index, &status, &paths).await {
            Ok(found) => {
                plan = Some(found);
                break;
            }
            Err(e) => failures.push(format!("{}: {e}", SOURCES[index].id)),
        }
    }

    let Some(plan) = plan else {
        let detail = if failures.is_empty() {
            "no geo source is configured".to_string()
        } else {
            failures.join("; ")
        };
        return Err(record_failure(
            &status_path,
            status,
            checked_at,
            detail,
            on_event,
        ));
    };

    let source_id = SOURCES[plan.source_index].id.to_string();

    // ---- 2. nothing to fetch --------------------------------------------
    if plan.is_noop() {
        status.last_check_at = Some(checked_at);
        status.active_source = Some(source_id.clone());
        status.source_index = plan.source_index as u32;
        status.last_error = None;
        geodata::write_status(&status_path, &status)?;
        on_event(&status, 0, 0, false);
        return Ok(GeoRefreshReport {
            updated: false,
            source: Some(source_id),
            geoip_sha256: status.applied_sha256.geoip.clone(),
            geosite_sha256: status.applied_sha256.geosite.clone(),
            bytes_served: 0,
            requests: 0,
            detail: "geo databases are already current".into(),
        });
    }

    // ---- 2b. is there a kernel to apply this to? ------------------------
    // Deliberately after the no-op check, so a check that finds nothing still
    // succeeds with no kernel running. From here on the cycle is committed to
    // moving ~21 MB, and there is no point doing that for a kernel that is not
    // there to adopt it — the scheduled caller would just repeat the transfer
    // on its next probe.
    mihomo::probe_controller().await?;

    // ---- 3. download, verify, stage -------------------------------------
    let mut staged: Vec<StagedFile> = Vec::new();
    // Kept so step 8 can repair the kernel's copy without re-reading 21 MB.
    let mut verified: Vec<(GeoFile, String, Arc<Vec<u8>>)> = Vec::new();

    for file in GeoFile::all() {
        match plan.file(file) {
            Some(planned) => {
                let raw =
                    geodata::download_verified(&client, &planned.url, &planned.sha256).await?;
                geodata::atomic_write(&paths.staging_file(file), &raw)?;
                let bytes = Arc::new(raw);
                verified.push((file, planned.sha256.clone(), Arc::clone(&bytes)));
                staged.push(StagedFile {
                    name: file.asset_name().to_string(),
                    bytes,
                });
            }
            None => {
                // Not being updated — but the kernel fetches *every* enabled
                // geo artefact on one trigger, so answering nothing for this
                // name would 404 and fail the whole cycle. Hand back what is
                // already on disk; the hashes match, so the kernel no-ops.
                if let Ok(raw) = std::fs::read(paths.kernel_file(file)) {
                    staged.push(StagedFile {
                        name: file.asset_name().to_string(),
                        bytes: Arc::new(raw),
                    });
                }
            }
        }
    }
    if staged.is_empty() {
        return Err(record_failure(
            &status_path,
            status,
            checked_at,
            "nothing could be staged for the kernel".into(),
            on_event,
        ));
    }

    // Which names the server actually has, decided before `staged` is moved.
    let serves_geoip = staged.iter().any(|f| f.name == GeoFile::Geoip.asset_name());
    let serves_geosite = staged
        .iter()
        .any(|f| f.name == GeoFile::Geosite.asset_name());

    // ---- 4. stand up the loopback server --------------------------------
    let server = StagingServer::start(staged).await?;

    // ---- 5. write the transient mount config ----------------------------
    let active_path = paths.active_config();
    let active_yaml = std::fs::read_to_string(&active_path)
        .map_err(|e| AppError::Io(format!("read {}: {e}", active_path.display())))?;
    let geoip_url = server.url_for(GeoFile::Geoip.asset_name());
    let geosite_url = server.url_for(GeoFile::Geosite.asset_name());
    let mount_yaml = geodata::patch_mount_config(
        &active_yaml,
        serves_geoip.then_some(geoip_url.as_str()),
        serves_geosite.then_some(geosite_url.as_str()),
    )?;
    geodata::atomic_write(&paths.mount_config(), mount_yaml.as_bytes())?;

    // ---- 6. park anything the kernel would mistake for "already current" -
    // Only files whose on-disk copy already equals the verified bytes are at
    // risk of the early return swallowing the cache clear; in the ordinary
    // case (disk holds the old database) the mismatch is what we want.
    let mut parked: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (file, digest, _) in &verified {
        let dest = paths.kernel_file(*file);
        if geodata::sha256_file(&dest).as_deref() == Some(digest.as_str()) {
            let aside = dest.with_extension("dat.fc-aside");
            if std::fs::rename(&dest, &aside).is_ok() {
                parked.push((aside, dest));
            }
        }
    }

    // ---- 7. mount -> trigger -> restore ---------------------------------
    let triggered = async {
        mihomo::put_config_file(&paths.mount_config(), true).await?;
        mihomo::trigger_mihomo_geo_update(TRIGGER_TIMEOUT_MS).await
    }
    .await;

    // The mount config is only ever meant to be live for the duration of the
    // fetch, so the profile goes back however the trigger ended.
    let restored = mihomo::put_config_file(&active_path, true).await;

    // Un-park. If the kernel never wrote the file, the parked copy is the
    // only one left and must not be lost.
    for (aside, dest) in &parked {
        if dest.exists() {
            let _ = std::fs::remove_file(aside);
        } else {
            let _ = std::fs::rename(aside, dest);
        }
    }
    let _ = std::fs::remove_file(paths.mount_config());

    let (requests, bytes_served) = server.shutdown().await;

    if let Err(e) = triggered {
        return Err(record_failure(
            &status_path,
            status,
            checked_at,
            e.to_string(),
            on_event,
        ));
    }

    // ---- 8. make sure the on-disk copy is the verified one ---------------
    // See the module note: a profile with no GEOIP/GEOSITE rule never causes
    // the kernel to fetch, so the trigger can succeed while the database on
    // disk is untouched. Writing our bytes keeps the next start correct, and
    // it cannot leave a stale matcher behind — no rule ever loaded one.
    let mut repaired = 0usize;
    for (file, digest, bytes) in &verified {
        let dest = paths.kernel_file(*file);
        if geodata::sha256_file(&dest).as_deref() == Some(digest.as_str()) {
            continue;
        }
        geodata::atomic_write(&dest, bytes.as_slice())?;
        repaired += 1;
    }

    // ---- 9. persist + announce ------------------------------------------
    for (file, digest, _) in &verified {
        status.applied_sha256.set(*file, digest.clone());
    }
    status.last_check_at = Some(checked_at);
    status.last_update_at = Some(Utc::now());
    status.active_source = Some(source_id.clone());
    status.source_index = plan.source_index as u32;
    status.last_error = None;
    geodata::write_status(&status_path, &status)?;

    on_event(&status, bytes_served, requests, true);

    let names: Vec<&str> = verified.iter().map(|(f, _, _)| f.asset_name()).collect();
    let mut detail = format!(
        "updated {} from {source_id} — {requests} staging request(s), {bytes_served} bytes",
        names.join(" + ")
    );
    if repaired > 0 {
        detail.push_str(&format!(
            "; {repaired} written locally (no geo rule consumes them yet)"
        ));
    }
    if restored.is_err() {
        detail.push_str("; profile reload failed — restart the kernel to drop the staging URL");
    }

    Ok(GeoRefreshReport {
        updated: true,
        source: Some(source_id),
        geoip_sha256: status.applied_sha256.geoip.clone(),
        geosite_sha256: status.applied_sha256.geosite.clone(),
        bytes_served,
        requests,
        detail,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Persist the failure, tell the UI, and hand back the error to propagate.
fn record_failure(
    status_path: &Path,
    mut status: GeoDataStatus,
    checked_at: DateTime<Utc>,
    detail: String,
    on_event: EventSink<'_>,
) -> AppError {
    status.last_check_at = Some(checked_at);
    status.last_error = Some(detail.clone());
    let _ = geodata::write_status(status_path, &status);
    on_event(&status, 0, 0, false);
    AppError::Geo(format!("geo refresh failed: {detail}"))
}

fn emit(
    app: &tauri::AppHandle,
    status: &GeoDataStatus,
    bytes_served: u64,
    requests: u64,
    updated: bool,
) {
    let payload = GeoDataUpdatedPayload {
        updated,
        source: status.active_source.clone(),
        geoip_sha256: status.applied_sha256.geoip.clone(),
        geosite_sha256: status.applied_sha256.geosite.clone(),
        bytes_served,
        requests,
        at_ms: Utc::now().timestamp_millis(),
    };
    // The renderer may legitimately have no listener (headless runs); a
    // failed emit is not worth failing the refresh over.
    let _ = payload.emit(app);
}

// ---------------------------------------------------------------------------
// Scheduler (Step 4)
// ---------------------------------------------------------------------------

/// Grace period before the first check. Startup is the busiest moment in the
/// process — window, sidecar, first config load — and a geo refresh is never
/// urgent enough to compete with it, so it waits its turn.
const FIRST_RUN_DELAY: Duration = Duration::from_secs(30);

/// Cadence once the first check has run.
const RUN_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Bounded catch-up. A check that failed for a transient reason — the kernel
/// was not up yet, one source blipped — should not cost a whole day, so it is
/// retried a few times before the daily cadence takes over. Bounded on purpose:
/// an unbounded retry against a genuinely dead upstream is a hot loop.
const RETRY_DELAY: Duration = Duration::from_secs(5 * 60);
const MAX_RETRIES: u32 = 3;

/// Start the silent refresher. Runs for the life of the process.
///
/// Every failure is consumed inside: reported on stderr, persisted into the
/// status file and published as the `GeoDataUpdatedPayload` event, both of
/// which the UI already reads. Nothing here may panic — a background task that
/// dies takes the feature with it and nobody notices.
pub fn spawn_scheduler(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_RUN_DELAY).await;

        // `interval`'s first tick completes immediately; burn it so the ticks
        // acted on are a full period apart.
        let mut ticker = tokio::time::interval(RUN_INTERVAL);
        ticker.tick().await;

        loop {
            if let Err(e) = check_with_catch_up(&app).await {
                eprintln!("[geodata] giving up until the next daily check: {e}");
            }
            ticker.tick().await;
        }
    });
}

/// One check plus its bounded retries.
///
/// `Ok` means a cycle ran, including one that found nothing to do. `Err` is
/// returned only after every retry is spent.
async fn check_with_catch_up(app: &tauri::AppHandle) -> Result<(), AppError> {
    let mut last: Option<AppError> = None;
    for attempt in 0..=MAX_RETRIES {
        if attempt > 0 {
            eprintln!(
                "[geodata] retry {attempt}/{MAX_RETRIES} in {}s",
                RETRY_DELAY.as_secs()
            );
            tokio::time::sleep(RETRY_DELAY).await;
        }
        match check_once(app).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                eprintln!("[geodata] check failed: {e}");
                last = Some(e);
            }
        }
    }
    last.ok_or_else(|| AppError::Geo("scheduler retries exhausted".into()))
        .map(|_| ())
}

/// One cycle, with the outcome reduced to `Ok`/`Err` and all reporting done.
async fn check_once(app: &tauri::AppHandle) -> Result<(), AppError> {
    let work_dir = sidecar::work_dir_for(app)?;
    let sink = event_sink(app);
    let report = run_refresh(&work_dir, &sink).await?;
    if report.updated {
        let source = report.source.as_deref().map_or("an unknown source", |s| s);
        eprintln!(
            "[geodata] updated from {source} — {} request(s), {} bytes",
            report.requests, report.bytes_served
        );
    }
    Ok(())
}

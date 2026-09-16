// ============================================================================
// geodata-smoke — head-less end-to-end harness for the geo-data pipeline.
//
//     cargo run --bin geodata-smoke --features geodata-smoke -- <work_dir>
//
// What it is for: the refresh cycle's whole point is that the *kernel* fetches
// the databases from a loopback server we stand up, because only the kernel's
// own updater clears its parsed-matcher caches. Nothing in a unit test can
// prove that a real kernel does the fetch, so this drives one real cycle and
// reports what happened.
//
// Preconditions, all arranged by the caller (see `scripts/` or the CI/runbook
// notes rather than this file):
//
//   * `<work_dir>/config.yaml` is the config the kernel is *running*, and it
//     carries at least one GEOIP / GEOSITE rule. Without such a rule the kernel
//     never marks the databases "enabled", and `POST /configs/geo` then answers
//     204 while fetching absolutely nothing — a success that looks like a
//     no-op. The orchestrator's module note calls this out.
//   * `<work_dir>/geodata.json` exists with an `appliedSha256` that does NOT
//     match upstream, otherwise `plan_update` correctly decides there is
//     nothing to do and the cycle short-circuits before any transfer.
//   * The kernel is already up on the FlexClash controller address
//     (`127.0.0.1:9091`, empty secret).
//
// It prints the cycle's `GeoRefreshReport` as JSON. `requests > 0` is the
// positive proof the kernel really pulled from the staging server; `updated:
// true` plus digests proves it adopted them.
//
// `required-features` keeps this out of ordinary builds (see Cargo.toml). A
// `[[bin]]` target — not a test — is required because tauri-build's comctl32 v6
// manifest is linked only into bin targets; a test executable touching the
// command graph would fail to *load* with STATUS_ENTRYPOINT_NOT_FOUND.
// ============================================================================

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(work_dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: geodata-smoke <work_dir>");
        return ExitCode::FAILURE;
    };

    // A current-thread runtime is enough: the cycle is one linear sequence of
    // awaits, and the staging server is a handful of spawned tasks on the same
    // reactor.
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: could not build the async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };

    // No renderer to emit to; the sink is deliberately a no-op.
    let report = rt.block_on(flexclash_lib::run_refresh(&work_dir, &|_, _, _, _| {}));

    match report {
        Ok(report) => {
            match serde_json::to_string_pretty(&report) {
                Ok(json) => println!("{json}"),
                Err(e) => println!("<unserialisable report: {e}>"),
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("refresh failed: {e}");
            ExitCode::FAILURE
        }
    }
}

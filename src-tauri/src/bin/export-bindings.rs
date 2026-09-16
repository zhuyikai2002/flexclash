// ============================================================================
// export-bindings — regenerate `src/bindings.ts` from the Rust IPC surface.
//
//     cargo run --bin export-bindings --features export-bindings
//
// This exists as a standalone binary for two reasons:
//
//   1. Decoupling. Generation used to happen inside `run()` guarded by
//      `#[cfg(debug_assertions)]`, so `src/bindings.ts` could only be refreshed
//      by starting the GUI. That is unusable headless (CI, a fresh clone on a
//      box with no display) and it silently coupled the type surface to
//      whether anyone had launched the app.
//
//   2. Loading correctly. Reaching the command graph from a `#[cfg(test)]`
//      helper makes rustc keep the whole Tauri/WebView2 stack in the test
//      executable, which then fails to *start* (STATUS_ENTRYPOINT_NOT_FOUND)
//      because test targets do not receive tauri-build's comctl32 v6 manifest.
//      A `[[bin]]` target does, so using one sidesteps the failure entirely.
//
// `required-features = ["export-bindings"]` (see Cargo.toml) keeps this out of
// `cargo build` and `tauri build`, so it costs nothing at release time.
// ============================================================================

use std::process::ExitCode;

fn main() -> ExitCode {
    match flexclash_lib::bindings::export_to_disk() {
        Ok(path) => {
            println!("wrote {path}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

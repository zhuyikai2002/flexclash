// ============================================================================
// bindings.rs — the renderer-facing IPC surface, declared exactly once.
//
// WHY THIS MODULE EXISTS
// ----------------------
// Before this, the crate kept the command list in *two* places that could
// drift apart silently:
//
//   * `tauri::generate_handler![…]` in `lib.rs` — 50 commands the app can
//     actually dispatch;
//   * a `tauri_specta` `collect_commands![…]` in the same function — 14 of
//     those, which is what `src/bindings.ts` was generated from.
//
// The other 36 commands were therefore invisible to the type generator. A
// command could be added, registered, and called from a `safeInvoke<T>('name')`
// string in the renderer with no type checking anywhere.
//
// `Builder::invoke_handler()` returns a dispatcher built from the same list
// that produces the TypeScript, so the two cannot disagree: adding a command
// here registers it *and* types it, and forgetting to add it here makes it
// uncallable. That is the whole point.
//
// HOW THE BINDINGS GET GENERATED
// ------------------------------
// Deliberately NOT from `run()`. The previous implementation exported on app
// startup, which meant `src/bindings.ts` could only be refreshed by launching
// the GUI — so a command added on a machine without a display could not be
// typechecked at all. Instead `src/bin/export-bindings.rs` drives
// `export_to_disk()` (see the `export-bindings` feature in Cargo.toml).
//
// That separate binary is also why the old `#[cfg(test)]` approach was
// abandoned: a test binary does not receive tauri-build's comctl32 v6
// manifest, so the moment test code reached this command graph the test
// executable failed to *load* (`STATUS_ENTRYPOINT_NOT_FOUND`) with no test
// output at all. A `[[bin]]` target does get that manifest, because
// tauri-build links the resource with `cargo:rustc-link-arg-bins`.
// ============================================================================

use tauri_specta::{collect_commands, collect_events, Builder};

/// Where the generated TypeScript lands.
///
/// Resolved from `CARGO_MANIFEST_DIR` rather than the process CWD so it is
/// correct regardless of where cargo or the binary was invoked from.
pub const BINDINGS_TS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/bindings.ts");

/// The single command registry.
///
/// Fed to `tauri::Builder::invoke_handler()` by `run()` **and** to
/// `export_to_disk()` below. Commands generic over `R: Runtime` need their
/// runtime spelled out (`::<tauri::Wry>`) — `collect_commands!` strips the
/// generics before handing the list to `generate_handler!`, which cannot
/// accept them, while keeping them for specta. `src/bindings.ts` is generated
/// from the generic instantiation, and the renderer only ever talks to `Wry`.
pub fn builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        // NOTE: `disable_serde_phases()` is deliberately NOT called, even
        // though the previous (14-command) setup did.
        //
        // Unified mode forces one shape for both directions of every type.
        // That is only safe when serde treats a type the same way both ways,
        // and several here do not: e.g. `ProfileMeta::url` and `node_count`
        // carry `#[serde(default)]`, so a payload may legitimately omit them
        // while our own writes always include them. Collapsing those into one
        // shape would generate a type that overstates what the wire guarantees.
        //
        // Phase-aware export (the default) describes each direction honestly,
        // and the renderer only ever consumes the serialise side, so it sees
        // the fields as optional -- which is what they are.
        .commands(collect_commands![
            // -- kernel lifecycle -------------------------------------------
            crate::commands::kernel::start_kernel::<tauri::Wry>,
            crate::commands::kernel::stop_kernel,
            crate::commands::kernel::restart_kernel::<tauri::Wry>,
            crate::commands::kernel::get_kernel_state,
            // -- Mihomo REST facade ----------------------------------------
            // The renderer never talks to 127.0.0.1:9091 directly; every call
            // goes through these, which is what lets the controller port stay
            // an implementation detail.
            crate::commands::mihomo::get_mihomo_version,
            crate::commands::mihomo::get_mihomo_configs,
            crate::commands::mihomo::patch_mihomo_config,
            crate::commands::mihomo::reload_mihomo_config,
            crate::commands::mihomo::get_mihomo_proxies,
            crate::commands::mihomo::get_mihomo_proxy,
            crate::commands::mihomo::select_mihomo_proxy,
            crate::commands::mihomo::get_mihomo_proxy_delay,
            crate::commands::mihomo::get_mihomo_connections,
            crate::commands::mihomo::close_mihomo_connection,
            crate::commands::mihomo::close_all_mihomo_connections,
            crate::commands::mihomo::get_mihomo_rules,
            // -- Rust-native speed test ------------------------------------
            // Results stream over `proxy://delay-batch` / `proxy://delay-done`;
            // only the start/cancel controls are commands.
            crate::commands::speedtest::speed_test_group,
            crate::commands::speedtest::cancel_speed_test,
            // -- updater ---------------------------------------------------
            crate::commands::updater::check_update,
            crate::commands::updater::install_update,
            // -- profiles --------------------------------------------------
            crate::commands::profile::list_profiles::<tauri::Wry>,
            crate::commands::profile::get_active_profile::<tauri::Wry>,
            crate::commands::profile::get_profile_content::<tauri::Wry>,
            crate::commands::profile::save_profile::<tauri::Wry>,
            crate::commands::profile::delete_profile::<tauri::Wry>,
            crate::commands::profile::import_profile_url::<tauri::Wry>,
            crate::commands::profile::import_profile_file::<tauri::Wry>,
            crate::commands::profile::update_subscription::<tauri::Wry>,
            crate::commands::profile::set_active_profile::<tauri::Wry>,
            crate::commands::profile::rename_profile::<tauri::Wry>,
            crate::commands::profile::open_profile_in_editor::<tauri::Wry>,
            crate::commands::profile::reveal_profile_file::<tauri::Wry>,
            // -- system proxy ----------------------------------------------
            crate::commands::proxy::enable_system_proxy::<tauri::Wry>,
            crate::commands::proxy::disable_system_proxy::<tauri::Wry>,
            crate::commands::proxy::get_system_proxy_status,
            // -- desktop integration ---------------------------------------
            crate::commands::desktop::get_autostart_status::<tauri::Wry>,
            crate::commands::desktop::set_autostart::<tauri::Wry>,
            crate::commands::desktop::get_silent_autostart_status::<tauri::Wry>,
            crate::commands::desktop::set_silent_autostart::<tauri::Wry>,
            crate::commands::desktop::get_silent_flag::<tauri::Wry>,
            crate::commands::desktop::sweep_residual_routes,
            // -- TUN -------------------------------------------------------
            crate::commands::tun::get_tun_state::<tauri::Wry>,
            crate::commands::tun::enable_tun::<tauri::Wry>,
            crate::commands::tun::apply_tun_advanced::<tauri::Wry>,
            crate::commands::tun::disable_tun::<tauri::Wry>,
            crate::commands::tun::sweep_tun_routes,
            // -- traffic history (SQLite-backed) ---------------------------
            crate::commands::history::get_traffic_history,
            crate::commands::history::get_history_db_path,
            crate::commands::history::get_history_sample_count,
            // -- application reset -----------------------------------------
            crate::commands::reset::reset_application::<tauri::Wry>,
        ])
        // BigInt-style Rust integers (`u64` / `i64` / `usize` / …) are emitted
        // as TypeScript `number` rather than being rejected.
        //
        // `specta-typescript` forbids them by default to avoid silent precision
        // loss, and that default is right for a transport that can carry a
        // real `bigint`. Tauri's IPC cannot: the payload is serialised by
        // `serde_json`, sent as text, and revived with `JSON.parse`, which
        // yields a JS `number` (f64) and nothing else. Emitting `bigint` here
        // would generate a type the runtime can never actually hand over -- a
        // declaration that lies about the wire. `number` is the honest one.
        //
        // Every field this remaps today stays far inside the 2^53 exact-integer
        // range, so the cast is also lossless in practice:
        //   * `ProfileMeta.{used,remaining,total}_bytes`, `SubscriptionUserInfo.*`
        //     and `ResetReport.removed_history_bytes` -- byte counts, worst case
        //     a few TB (~2^42); 2^53 bytes is 8 PiB.
        //   * `HistoryPoint.{ts,upload,download}`, `TrafficHistory.{bucket_ms,
        //     total_*}`, `TunStatus.last_changed_at_ms` -- epoch milliseconds
        //     (~2^41) and per-bucket byte totals (~2^45).
        //   * `ResetReport.removed_profiles`, `get_history_sample_count` -- small
        //     counts.
        //
        // INVARIANT for new IPC fields: if a future value can exceed 2^53, do
        // NOT rely on this -- change the field to a string on both sides (or
        // annotate it `#[specta(type = String)]` with a matching `#[serde(with)]`)
        // so the transport is lossless, and add a targeted override here.
        .dangerously_cast_bigints_to_number()
        // Types that cross the IPC boundary but appear in no command
        // signature: they arrive in the renderer as *event payloads*.
        // Registering them as plain types puts them in `bindings.ts` so the
        // listeners can be typed without a hand-written mirror.
        .typ::<crate::core::watcher::AppStateSnapshot>()
        .typ::<crate::core::speedtest::DelayBatch>()
        .typ::<crate::core::speedtest::DelayDone>()
        // Typed events — the kernel data plane. Each payload carries
        // `#[derive(tauri_specta::Event)]` (see `core::kernel_events` for the
        // `/traffic` + `/logs` shapes and `core::sidecar` for the
        // config-refresh notice). Registering it here does two things at once:
        // it becomes emittable solely through the typed `.emit()` API, and its
        // shape lands in `bindings.ts` under `events`, so the renderer's
        // listeners are typed with no mirror to drift. `mount_events()` in
        // `lib.rs` installs the registry these names resolve through.
        .events(collect_events![
            crate::core::kernel_events::TrafficPayload,
            crate::core::kernel_events::LogBatch,
            crate::core::sidecar::ConfigRefreshPayload,
        ])
}

/// Regenerate `src/bindings.ts`. Returns the path written.
pub fn export_to_disk() -> Result<String, String> {
    use specta_typescript::Typescript;

    let path = std::path::Path::new(BINDINGS_TS);
    let exporter = Typescript::default().header(
        "// Generated from the Rust IPC surface by `cargo run --bin export-bindings \
         --features export-bindings`.\n\
         // Do NOT edit by hand: the next export overwrites it, and CI fails when the\n\
         // committed file differs from a fresh export.",
    );

    builder().export(exporter, path).map_err(|e| {
        format!(
            "failed to export TypeScript bindings to {}: {e}",
            path.display()
        )
    })?;

    Ok(BINDINGS_TS.to_string())
}

// NOTE: there is deliberately no `#[cfg(test)]` test that touches `builder()`.
// Referring to it from a test target is what caused the original
// `STATUS_ENTRYPOINT_NOT_FOUND` failure: rustc's dead-code elimination keeps
// the whole command graph alive once anything in the test binary references
// it, which drags user32 / gdi32 / shell32 / comctl32 / webview2 into the test
// executable -- and a test binary never receives tauri-build's comctl32 v6
// manifest. The generated output is guarded by the CI drift check instead
// (`export-bindings` then `git diff --exit-code src/bindings.ts`), which
// exercises the real binary rather than the test harness.

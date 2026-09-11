// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Must run before the WebView2 runtime spins up its renderer.
    disable_webview_sandbox_if_needed();
    flexclash_lib::run()
}

/// Launch the WebView2 renderer without its Chromium sandbox.
///
/// **Why:** on some machines the sandbox cannot initialise at all —
/// network-filter / security drivers (Radmin VPN is one known cause) are a
/// common trigger. The renderer then dies during startup: the window comes
/// up, but no JavaScript ever runs, so `start_kernel` is never invoked and
/// the UI just sits there dead. Measured on the release build — with the
/// sandbox the kernel never starts; with `--no-sandbox` it starts every
/// time. The flag is both necessary and sufficient.
///
/// **Trade-off:** this removes one layer of defence-in-depth around the
/// renderer. Acceptable here because the webview only ever loads the
/// bundled `dist/` assets — it is not a general-purpose browser and never
/// renders remote, untrusted content.
///
/// **Escape hatch:** set `FLEXCLASH_WEBVIEW_SANDBOX=1` to keep the sandbox.
#[cfg(windows)]
fn disable_webview_sandbox_if_needed() {
    const ARGS: &str = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";
    const KEEP_SANDBOX: &str = "FLEXCLASH_WEBVIEW_SANDBOX";

    if std::env::var_os(KEEP_SANDBOX).is_some() {
        return;
    }

    let existing = std::env::var(ARGS).unwrap_or_default();
    if existing.split_whitespace().any(|a| a == "--no-sandbox") {
        return; // already requested — e.g. by a dev wrapper script
    }

    let merged = if existing.is_empty() {
        "--no-sandbox".into()
    } else {
        format!("{existing} --no-sandbox")
    };
    std::env::set_var(ARGS, merged);
}

#[cfg(not(windows))]
fn disable_webview_sandbox_if_needed() {}

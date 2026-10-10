// ============================================================================
// core/kernel_version.rs — read the bundled mihomo's own version, offline.
//
// WHY NOT THE REST CONTROLLER
// --------------------------
// `GET /version` on 127.0.0.1:9091 is the obvious source, but it only answers
// while a kernel is actually running. The version string is needed at times
// when no kernel is up at all: the About panel, the subscribe dialog's
// `User-Agent` hint, a bug report filed before the user ever started the
// proxy. Asking the binary itself (`mihomo -v`) always works, costs one
// process spawn, and cannot disagree with the file on disk.
//
// The two sources agree by construction — the REST handler reports the same
// build tag — but only the binary is authoritative when they cannot both be
// consulted.
//
// PARSING IS A PURE FUNCTION
// -------------------------
// `parse_version_output` is deliberately free of I/O so the awkward cases are
// unit-tested instead of discovered on a user's machine. Real output looks like
//
//     Mihomo Meta v1.19.32 windows amd64 with go1.26.8 Wed Sep 30 17:00:18 UTC 2026
//     Use tags: with_gvisor
//
// and older builds printed `mihomo v1.19.x ...`, so we scan for the first
// `v`-prefixed dotted-numeric token rather than matching the whole line.
// ============================================================================

use std::path::Path;
use std::time::Duration;

use tauri::{AppHandle, Runtime};

use crate::error::{AppError, Result};

/// Spawn budget for `mihomo -v`.
///
/// The call is a version print: it opens no sockets, reads no config and
/// touches no device, so it returns in single-digit milliseconds. Two seconds
/// is ~3 orders of magnitude of headroom while still guaranteeing the IPC
/// caller is released even if the binary wedges (a corrupted sidecar on a
/// locked-down AV scanner is not hypothetical on Windows).
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Extract a bare `vX.Y.Z` version from `mihomo -v` output.
///
/// Returns the token **including** its leading `v` (`"v1.19.32"`), which is
/// the form both the GitHub release tag and the `User-Agent` line want.
///
/// Scans whitespace-separated tokens left to right and returns the first one
/// shaped like a version. Scanning by shape rather than by prefix is what keeps
/// this working across the `mihomo v1.19.x` -> `Mihomo Meta v1.19.x` banner
/// change; the leading `Mihomo Meta` words simply fail the shape test and the
/// scan moves on.
pub fn parse_version_output(stdout: &str) -> Option<String> {
    stdout
        .split_whitespace()
        .map(|tok| tok.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-'))
        .find(|tok| is_version_token(tok))
        .map(str::to_string)
}

/// True when `tok` is `v` followed by at least `MAJOR.MINOR` — the shape every
/// mihomo release tag has had.
///
/// The pre-release/build suffix is stripped *before* the numeric test, so
/// `v1.20.0-rc1` is recognised as the version `v1.20.0` carrying a suffix
/// rather than rejected for its third component not being a bare number. The
/// token itself is still returned unmodified, so the caller renders exactly
/// what the binary printed.
///
/// Two numeric components are the floor because `v1` alone has never been a
/// mihomo tag, and accepting it would let ordinary prose (`Use tags: v1`)
/// through.
fn is_version_token(tok: &str) -> bool {
    let Some(rest) = tok.strip_prefix('v') else {
        return false;
    };
    let core = rest.split(['-', '+']).next().unwrap_or_default();
    let mut components = 0;
    for part in core.split('.') {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        components += 1;
    }
    components >= 2
}

/// Run `mihomo -v` and return the parsed version.
///
/// Blocking: this spawns a process and waits on it. Callers must run it off
/// the async runtime (`spawn_blocking`) for exactly the reason
/// `commands::tun::off_runtime` exists — a synchronous wait inside an async
/// body occupies a tokio worker, and Tauri dispatches every IPC call onto that
/// same runtime, so one blocked worker makes the whole app look frozen.
///
/// The executable is located with the same six-tier fallback chain TUN uses
/// (`elevate::resolve_mihomo_binary`), so this reports the version of the exact
/// binary that would be launched, in dev and in an installed bundle alike.
pub fn probe_kernel_version<R: Runtime>(app: &AppHandle<R>) -> Result<String> {
    let work_dir = crate::core::sidecar::work_dir_for(app)?;
    let binary = crate::core::elevate::resolve_mihomo_binary_pub(app, &work_dir)?;
    probe_with(&binary)
}

/// Same as [`probe_kernel_version`] but against an explicit path. Split out so
/// the timeout/parse pipeline is testable without a Tauri app handle.
fn probe_with(binary: &Path) -> Result<String> {
    let exe = binary.to_string_lossy().into_owned();
    let out = crate::core::proc::run_with_timeout(&exe, &["-v"], PROBE_TIMEOUT)
        .map_err(|e| AppError::Shell(format!("mihomo -v: {e}")))?;

    if out.timed_out {
        return Err(AppError::Shell(format!(
            "mihomo -v did not answer within {}s",
            PROBE_TIMEOUT.as_secs()
        )));
    }

    // mihomo prints its banner on stdout; a wrapper that redirects it would
    // show up on stderr, so both streams are searched before giving up.
    parse_version_output(&out.stdout)
        .or_else(|| parse_version_output(&out.stderr))
        .ok_or_else(|| {
            AppError::Shell(format!(
                "could not find a version in `mihomo -v` output (code={:?}, stdout={:?}, stderr={:?})",
                out.code,
                out.stdout.trim(),
                out.stderr.trim()
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_current_banner_shape() {
        // Captured verbatim from mihomo v1.19.32 on Windows.
        let out = "Mihomo Meta v1.19.32 windows amd64 with go1.26.8 Wed Sep 30 17:00:18 UTC 2026\nUse tags: with_gvisor\n";
        assert_eq!(parse_version_output(out).as_deref(), Some("v1.19.32"));
    }

    #[test]
    fn parses_the_legacy_banner_shape() {
        assert_eq!(
            parse_version_output("mihomo v1.19.0 linux amd64").as_deref(),
            Some("v1.19.0")
        );
    }

    #[test]
    fn finds_the_version_on_a_later_line() {
        let out = "Use tags: with_gvisor\nmihomo v1.18.9 darwin arm64\n";
        assert_eq!(parse_version_output(out).as_deref(), Some("v1.18.9"));
    }

    #[test]
    fn keeps_a_prerelease_suffix_intact() {
        assert_eq!(
            parse_version_output("Mihomo Meta v1.20.0-rc1 linux amd64").as_deref(),
            Some("v1.20.0-rc1")
        );
    }

    #[test]
    fn ignores_go_and_build_noise() {
        // `go1.26.8` and `Wed Sep 30 ...` must not win over the real tag.
        let out = "Mihomo Meta v1.19.32 linux amd64 with go1.26.8 Wed Sep 30 17:00:18 UTC 2026";
        assert_eq!(parse_version_output(out).as_deref(), Some("v1.19.32"));
    }

    #[test]
    fn rejects_prose_that_merely_starts_with_v() {
        assert_eq!(parse_version_output("Use tags: v1"), None);
        assert_eq!(parse_version_output("Use tags: with_gvisor"), None);
        assert_eq!(parse_version_output("v"), None);
        assert_eq!(parse_version_output("v1."), None);
        assert_eq!(parse_version_output(""), None);
    }

    #[test]
    fn survives_an_empty_or_binary_probe() {
        assert_eq!(parse_version_output("\0\0\0"), None);
    }
}

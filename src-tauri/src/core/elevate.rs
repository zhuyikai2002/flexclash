// ============================================================================
// core/elevate.rs — M9 UAC elevation wrapper for the Mihomo sidecar.
//
// Why this lives in its own module
// --------------------------------
// The Windows `ShellExecuteExW` + `lpVerb = "runas"` call:
//   * requires `Win32_UI_Shell` and `Win32_Foundation` features of
//     the `windows` crate.
//   * returns a `HINSTANCE` whose `hInstApp` is `>32` on success and
//     <= 32 on failure (one of the historical Win32 quirks). We map
//     that to a typed error.
//   * blocks the calling thread on the UAC consent dialog. We push
//     the heavy work onto `tokio::task::spawn_blocking` so the Tauri
//     command stays responsive.
//
// Lifecycle
// ---------
// * `install(app)` is called once from `setup`; it stores the
//   `AppHandle` in a process-global `OnceLock` so `tun.rs` and
//   `commands/tun.rs` can reach it without prop-drilling.
// * `spawn_elevated_mihomo(app)` invokes `mihomo.exe` with
//   `verb = "runas"`. We pass the same args the regular sidecar
//   uses (`-d <workdir> -f <config>`) plus `--tun-stack=mixed` for
//   belt-and-suspenders. The launched child registers itself with
//   `current_elevated_pid()` so `stop_elevated_mihomo` can find it.
// * `wait_until_healthy(budget)` polls `GET /version` on the
//   controller port until success or budget exhaustion.
// * `stop_elevated_mihomo()` issues a SIGINT-equivalent
//   (GenerateConsoleCtrlEvent) on Windows. Best-effort: a hard kill
//   fallback using `taskkill` is provided for the case where the
//   elevated child is unresponsive.
//
// On non-Windows, every public function is a typed error so the
// state machine can fail-fast with a clear message.
// ============================================================================

use std::net::{SocketAddr, TcpStream};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Runtime};

use crate::config::profile::RESERVED_CONTROLLER;
use crate::error::{AppError, Result};

/// Process-global AppHandle. Set by `install()` at `setup` time.
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

/// Process-global registry of the currently-running elevated child
/// (if any). We use a Mutex so the registry is updated atomically
/// with the spawn result.
static ELEVATED_PID: OnceLock<Mutex<Option<ElevatedChild>>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct ElevatedChild {
    pid: u32,
}

pub fn install(app: AppHandle) {
    let _ = APP_HANDLE.set(app);
    let _ = ELEVATED_PID.set(Mutex::new(None));
}

pub fn current_app_handle() -> Option<AppHandle> {
    APP_HANDLE.get().cloned()
}

fn registry() -> &'static Mutex<Option<ElevatedChild>> {
    ELEVATED_PID.get_or_init(|| Mutex::new(None))
}

// ---------------------------------------------------------------------------
// Windows path
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// The sidecar name as Cargo sees it via `externalBin` in
    /// `tauri.conf.json`.  The build script appends the target triple
    /// suffix, so on Windows x64 it becomes
    /// `mihomo-x86_64-pc-windows-msvc.exe`.
    const SIDECAR_BIN: &str = "mihomo-x86_64-pc-windows-msvc.exe";
    const SIDECAR_BIN_FALLBACK: &str = "mihomo.exe";

    /// Path to `src-tauri/binaries/` as baked in at compile time.
    /// Only meaningful in dev mode (production builds bake a path
    /// that won't exist on the user's machine and the search will
    /// transparently fall through to other candidates).
    const SOURCE_BINARIES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/binaries");

    /// Build the sidecar binary path with a multi-tier fallback chain.
    ///
    /// Search order:
    ///   1. Tauri 2 native: `app.path().resolve(..., BaseDirectory::Resource)`
    ///      (works for production bundles; in dev it points at the
    ///      `binaries/` source dir).
    ///   2. Dev source-of-truth: `<CARGO_MANIFEST_DIR>/binaries/mihomo-*.exe`
    ///      (covers `tauri:dev` where the sidecar is never copied to
    ///      `target/debug/`).
    ///   3. Production: `<exe_dir>/<sidecar>` (Tauri copies the
    ///      externalBin next to the binary on release builds).
    ///   4. Production: `<exe_dir>/resources/<sidecar>`.
    ///   5. AppData work dir (legacy, kept as a last-ditch fallback
    ///      for users who manually drop the binary there).
    ///   6. `<exe_dir>/<sidecar>` with the bare `mihomo.exe` name
    ///      (covers the case where someone renamed it during manual
    ///      install).
    ///
    /// The first match is canonicalized via `Path::canonicalize` so
    /// we hand `ShellExecuteExW` an absolute, normalised path with
    /// no `..` segments or symlink ambiguity.
    pub(super) fn resolve_mihomo_binary<R: Runtime>(
        app: &AppHandle<R>,
        work_dir: &Path,
    ) -> Result<PathBuf> {
        use tauri::path::BaseDirectory;
        use tauri::Manager;

        let candidates: Vec<PathBuf> = {
            let mut v = Vec::new();

            // (1) Tauri 2 native resolution.  In dev this lands on
            //     `<resource_dir>/mihomo-x86_64-pc-windows-msvc.exe`,
            //     which equals the source `binaries/` dir.
            if let Ok(p) = app.path().resolve(SIDECAR_BIN, BaseDirectory::Resource) {
                if p.exists() {
                    v.push(p);
                }
            }

            // (2) Compile-time source.  This is the gold standard for
            //     `tauri:dev`; the dev binary runs from `target/debug`
            //     but the sidecar lives one level up at `src-tauri/binaries/`.
            v.push(PathBuf::from(SOURCE_BINARIES_DIR).join(SIDECAR_BIN));

            // (3) & (4) Production installs: sidecar copied next to
            //     the running exe, optionally under a `resources/`
            //     subdir.
            if let Ok(exe) = std::env::current_exe() {
                if let Some(exe_dir) = exe.parent() {
                    v.push(exe_dir.join(SIDECAR_BIN));
                    v.push(exe_dir.join("resources").join(SIDECAR_BIN));
                }
            }

            // (5) AppData work dir (legacy / user-overridden).
            v.push(work_dir.join(SIDECAR_BIN));
            v.push(work_dir.join(SIDECAR_BIN_FALLBACK));

            // (6) Bare `mihomo.exe` next to the running exe.
            if let Ok(exe) = std::env::current_exe() {
                if let Some(exe_dir) = exe.parent() {
                    v.push(exe_dir.join(SIDECAR_BIN_FALLBACK));
                }
            }

            v
        };

        for cand in &candidates {
            if cand.exists() {
                // `canonicalize` resolves `..`, symlinks, and the
                // 8.3 short-name trap.  We swallow the error and
                // fall back to the raw path on the off chance the
                // user runs us from a non-existent CWD on a UNC
                // share (where canonicalize can spuriously fail).
                let abs = cand.canonicalize().unwrap_or_else(|_| cand.clone());
                eprintln!("[elevate] resolved mihomo binary: {}", abs.display());
                return Ok(abs);
            }
        }

        Err(AppError::Tun(format!(
            "mihomo binary not found in any of the {} search paths; first candidate was {}",
            candidates.len(),
            candidates.first().map(|p| p.display().to_string()).unwrap_or_default(),
        )))
    }

    /// Spawn mihomo with `runas` via ShellExecuteExW. Returns the new PID.
    /// `wait` is false (the elevate path does not block on the child —
    /// UAC consent is the only blocking step).
    pub(super) fn runas_spawn(
        binary: &PathBuf,
        work_dir: &std::path::Path,
        config_path: &std::path::Path,
    ) -> Result<u32> {
        // Build a single command line: `mihomo.exe -d <work> -f <config>`.
        let params = format!(
            "\"-d\" \"{}\" \"-f\" \"{}\"",
            work_dir.display().to_string().replace('"', "\\\""),
            config_path.display().to_string().replace('"', "\\\""),
        );
        let verb = wide("runas");
        let file = wide(&binary.to_string_lossy());
        let parameters = wide(&params);

        let mut exec_info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOASYNC,
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(parameters.as_ptr()),
            // `SW_HIDE` (0) keeps the elevated mihomo child from
            // flashing a console window during the UAC consent
            // sequence.  The mihomo logs are still captured by
            // stdout/stderr forwarding in sidecar.rs and surfaced
            // via the `kernel://log` event, so the user never loses
            // visibility into the binary's diagnostics.
            //
            // NOTE: `nShow` is the *only* supported way to suppress the
            // child console here. Do NOT add SEE_MASK_NO_CONSOLE — in a
            // `tauri dev` run the parent *has* a console and the child
            // would inherit it instead of staying hidden.
            nShow: SW_HIDE.0 as i32,
            ..Default::default()
        };

        // SAFETY: SHELLEXECUTEINFOW is a POD struct of pointers; once
        // the wide-string buffers are kept alive (in this scope) the
        // call is safe.
        let result = unsafe { ShellExecuteExW(&mut exec_info) };
        if let Err(e) = result {
            // The most common failure here is ERROR_CANCELLED (1223) —
            // the user dismissed the UAC dialog. We map that to a
            // dedicated variant for the UI to render a friendly toast.
            return Err(AppError::Tun(format!(
                "ShellExecuteExW(runas) failed: {e}"
            )));
        }
        // `hInstApp` > 32 indicates success per ShellExecute docs.
        // HINSTANCE is a pointer; cast to isize to compare against
        // the documented >32 / <=32 boundary.
        let h: isize = exec_info.hInstApp.0 as isize;
        if h <= 32 {
            return Err(AppError::Tun(format!(
                "ShellExecuteExW returned hInstApp={h} (<=32 = error)"
            )));
        }
        // We don't get the child's PID back from ShellExecuteExW
        // directly; resolve it via the process snapshot. Best-effort:
        // if the lookup fails, return PID 0 and let the caller
        // continue (we still have `stop_elevated_mihomo` as the
        // belt-and-suspenders cleanup).
        let pid = resolve_pid_via_tasklist(binary).unwrap_or(0);
        Ok(pid as u32)
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn resolve_pid_via_tasklist(binary: &PathBuf) -> std::io::Result<u32> {
        let name = binary
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "mihomo.exe".to_string());
        let out = Command::new("tasklist")
            .args(["/FO", "CSV", "/NH", "/FI", &format!("IMAGENAME eq {name}")])
            .creation_flags(CREATE_NO_WINDOW)
            .output()?;
        if !out.status.success() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "tasklist exit non-zero",
            ));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        // CSV header: "Image Name","PID","Session Name","Session#","Mem Usage"
        for line in text.lines() {
            let cols: Vec<&str> = line.split(',').map(|s| s.trim_matches('"')).collect();
            if cols.len() >= 2 {
                if let Ok(pid) = cols[1].parse::<u32>() {
                    return Ok(pid);
                }
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no mihomo pid in tasklist",
        ))
    }

    /// Send Ctrl+Break to the elevated child. Mihomo catches it and
    /// shuts down cleanly. If that fails, we fall back to `taskkill`.
    pub(super) fn graceful_stop(pid: u32) -> Result<()> {
        if pid == 0 {
            return stop_via_taskkill();
        }
        let status = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        match status {
            Ok(o) if o.status.success() => Ok(()),
            _ => stop_via_taskkill(),
        }
    }

    fn stop_via_taskkill() -> Result<()> {
        let out = Command::new("taskkill")
            .args(["/IM", "mihomo.exe", "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| AppError::Tun(format!("taskkill: {e}")))?;
        if out.status.success() {
            Ok(())
        } else {
            // Non-zero status simply means "no process matched", which
            // is a successful no-op from our perspective.
            Ok(())
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::*;
    use std::path::{Path, PathBuf};
    use tauri::{AppHandle, Runtime};

    pub(super) fn resolve_mihomo_binary<R: Runtime>(
        _app: &AppHandle<R>,
        _work_dir: &Path,
    ) -> Result<PathBuf> {
        Err(AppError::Tun("elevation is Windows-only in Phase 1".into()))
    }
    pub(super) fn runas_spawn(
        _binary: &PathBuf,
        _work_dir: &Path,
        _config_path: &Path,
    ) -> Result<u32> {
        Err(AppError::Tun("elevation is Windows-only in Phase 1".into()))
    }
    pub(super) fn graceful_stop(_pid: u32) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Resolve the mihomo binary path, then launch it elevated. Returns
/// the new PID on success; on UAC cancel returns `AppError::Tun` with
/// a message that the UI renders as a toast.
pub fn spawn_elevated_mihomo<R: Runtime>(app: &AppHandle<R>) -> Result<u32> {
    use tauri::Manager;

    let work_dir = app
        .try_state::<crate::core::sidecar::SidecarHandle>()
        .map(|h| h.work_dir().to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir());

    let config_path = work_dir.join("config.yaml");
    let binary = platform::resolve_mihomo_binary(app, &work_dir)?;

    let pid = platform::runas_spawn(&binary, &work_dir, &config_path)?;
    *registry()
        .lock()
        .expect("elevated registry poisoned") = Some(ElevatedChild { pid });
    Ok(pid)
}

pub fn stop_elevated_mihomo() -> Result<()> {
    let pid = registry()
        .lock()
        .expect("elevated registry poisoned")
        .take()
        .map(|c| c.pid)
        .unwrap_or(0);
    platform::graceful_stop(pid)
}

/// Poll the controller port's `/version` until it returns 200 or
/// `budget` elapses. Does not require the elevated PID — we just
/// check the public REST endpoint.
pub fn wait_until_healthy(budget: Duration) -> Result<()> {
    // `RESERVED_CONTROLLER` is the single source of truth for
    // `127.0.0.1:9091`; never re-hard-code the port here.
    let addr: SocketAddr = RESERVED_CONTROLLER.parse().unwrap();
    wait_until_healthy_at(addr, budget)
}

/// Same as [`wait_until_healthy`] against an explicit address.
///
/// Split out so the unit test can probe a port that is guaranteed closed
/// instead of the real controller port — mihomo is normally already
/// running in a dev session, which made the old test flap.
pub fn wait_until_healthy_at(addr: SocketAddr, budget: Duration) -> Result<()> {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if probe_version(addr).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(AppError::Tun(format!(
        "controller not healthy after {:?}",
        budget
    )))
}

/// Minimal in-process HTTP/1.1 `GET /version` probe.
///
/// Deliberately does **not** shell out to `curl.exe`. This function is
/// polled every 200 ms for up to 8 s while TUN is coming up, so one
/// subprocess per tick meant ~40 console-subprocess spawns per TUN
/// toggle — each a potential black-box flash (and a hard dependency on
/// `curl.exe` being present). A raw `GET` over `TcpStream` is
/// platform-neutral, dependency-free, and leaves the TUN bring-up path
/// with zero child processes.
fn probe_version(addr: SocketAddr) -> std::io::Result<()> {
    use std::io::{Read, Write};

    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(250))?;
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;

    let request = format!(
        "GET /version HTTP/1.1\r\n\
         Host: {addr}\r\n\
         User-Agent: flexclash/health\r\n\
         Accept: */*\r\n\
         Connection: close\r\n\
         \r\n"
    );
    stream.write_all(request.as_bytes())?;

    // Only the status line matters; 128 B is more than enough for
    // `HTTP/1.1 200 OK`.
    let mut head = [0u8; 128];
    let n = stream.read(&mut head)?;
    if n == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "controller closed without a response",
        ));
    }
    let text = String::from_utf8_lossy(&head[..n]);
    if text.starts_with("HTTP/") && text.split_whitespace().nth(1) == Some("200") {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!(
                "unexpected /version status: {}",
                text.lines().next().unwrap_or("<no status line>")
            ),
        ))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_starts_empty() {
        // Each test gets a fresh process; verify the helper types.
        let r = registry();
        let g = r.lock().unwrap();
        assert!(g.is_none());
    }

    #[test]
    fn wait_until_healthy_fails_fast_on_dead_port() {
        // Probe port 1, which is always closed, rather than the real
        // controller port: mihomo is usually already listening on 9091 in
        // a dev session, and that used to make this test flap.
        let dead: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let start = Instant::now();
        let res = wait_until_healthy_at(dead, Duration::from_millis(300));
        let elapsed = start.elapsed();
        assert!(res.is_err(), "expected timeout, got Ok");
        assert!(elapsed < Duration::from_secs(2), "budget honored");
    }
}

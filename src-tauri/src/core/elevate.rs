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
//   * blocks the calling thread on the UAC consent dialog. Callers must
//     therefore run the spawn off the async runtime (see
//     `commands::tun::off_runtime`), so the modal blocks a dedicated
//     thread and nothing else.
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
use std::path::Path;
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

/// OS-level handle to a spawned kernel process.
///
/// **Windows**: the `hProcess` that `ShellExecuteExW` fills in when
/// `SEE_MASK_NOCLOSEPROCESS` is requested. This is the whole point of the
/// type: a process handle works across integrity levels, so a FlexClash
/// running *asInvoker* can still retire the UAC-elevated TUN kernel with
/// `TerminateProcess`. `taskkill` cannot — the UIPI integrity check rejects
/// it with "access denied", which is what used to leave the elevated kernel
/// holding 9091/7897 forever.
///
/// **Linux / macOS**: `()`. Those platforms spawn through
/// `std::process::Command`, so the `Child` we already keep is both the
/// handle and the reap; there is nothing extra to carry.
#[cfg(target_os = "windows")]
pub type ProcHandle = windows::Win32::Foundation::HANDLE;
#[cfg(not(target_os = "windows"))]
pub type ProcHandle = ();

/// A [`ProcHandle`] that may cross a thread boundary.
///
/// `HANDLE` is a bare `*mut c_void`, so it is neither `Send` nor `Sync` and
/// cannot live inside the process-global registry mutex as-is. The handle is
/// opened by `ShellExecuteExW` in this process and closed by
/// `graceful_stop` in this process; it only ever *moves* between threads
/// while the `ELEVATED_PID` mutex is held, so the raw pointer never escapes
/// an unsynchronised context. Asserting `Send` here states exactly that.
#[derive(Debug)]
struct OwnedHandle(ProcHandle);

// SAFETY: created and closed within this process; every access is
// serialised by the `ELEVATED_PID` registry mutex.
unsafe impl Send for OwnedHandle {}

#[derive(Debug)]
struct ElevatedChild {
    pid: u32,
    /// The OS child handle when the platform spawned the kernel through
    /// `std::process::Command` (Linux). Kept so `graceful_stop` can wait
    /// on it and reap it, preventing a `<defunct>` zombie. Windows launches
    /// via `ShellExecuteExW` and has no such handle (`None`).
    child: Option<std::process::Child>,
    /// Windows: the `hProcess` from the spawn, used to terminate the
    /// elevated kernel without needing elevation ourselves.
    handle: Option<OwnedHandle>,
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

/// Emit one lifecycle line from the elevate layer.
///
/// Kept as a named seam so the kernel-log channel can be introduced later
/// without touching the eight call sites that already depend on it.
fn lifecycle(line: &str) {
    eprintln!("[elevate] {line}");
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
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessId, TerminateProcess, WaitForSingleObject,
    };
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// Budget for any single `taskkill` invocation. `taskkill` is normally
    /// instant; it only runs long when the target is wedged, and in that
    /// case we would rather report a failure than hang the transition.
    const KILL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    /// How long to wait for a terminated kernel to actually exit before
    /// giving up on the reap.
    const REAP_TIMEOUT_MS: u32 = 5_000;

    /// Image-name glob for the sweep fallback.
    ///
    /// The bundled sidecar is `mihomo-x86_64-pc-windows-msvc.exe`, so
    /// `/IM mihomo.exe` never matched anything. Must stay in sync with
    /// `core::sidecar::hard_cleanup`, which already uses the glob.
    const KILL_IMAGE_GLOB: &str = "mihomo*.exe";

    /// `taskkill` exit code for "no running task matches that name" — a
    /// successful no-op from our point of view, not a failure.
    const TASKKILL_NO_MATCH: i32 = 128;

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
            candidates
                .first()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        )))
    }

    /// Spawn mihomo with `runas` via ShellExecuteExW. Returns the new PID,
    /// `None` for the `std::process::Child` (a shell launch has none) and the
    /// `hProcess` handle.
    ///
    /// `SEE_MASK_NOCLOSEPROCESS` is load-bearing: it is what fills
    /// `hProcess`, and holding that handle is the only way a non-elevated
    /// FlexClash can later terminate the elevated kernel. Without it the
    /// cleanup path could only reach for `taskkill`, which the UIPI
    /// integrity check rejects — the elevated kernel then survived "关闭 TUN"
    /// and kept the controller port bound. `wait` is still false: the
    /// UAC consent dialog is the only blocking step here.
    pub(super) fn runas_spawn(
        binary: &Path,
        work_dir: &std::path::Path,
        config_path: &std::path::Path,
    ) -> Result<(u32, Option<std::process::Child>, Option<ProcHandle>)> {
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
            fMask: SEE_MASK_NOASYNC | SEE_MASK_NOCLOSEPROCESS,
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
            nShow: SW_HIDE.0,
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
            return Err(AppError::Tun(format!("ShellExecuteExW(runas) failed: {e}")));
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
        let process = exec_info.hProcess;
        if process.is_invalid() {
            return Err(AppError::Tun(
                "ShellExecuteExW(runas) returned no process handle — cannot manage the \
                 elevated kernel"
                    .into(),
            ));
        }
        // The handle we now hold answers the PID exactly — no `tasklist`
        // scan, no PID-reuse race. Fall back to the scan only if the handle
        // somehow cannot report it.
        // SAFETY: `process` is a live handle we own; GetProcessId takes no
        // out-parameter and cannot affect the process.
        let pid = match unsafe { GetProcessId(process) } {
            0 => resolve_pid_via_tasklist(binary).unwrap_or(0),
            from_handle => from_handle,
        };
        Ok((pid, None, Some(process)))
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Run `binary args` elevated via `runas` and block until it exits.
    /// Returns the child's exit code.
    ///
    /// Unlike [`runas_spawn`] this is for short-lived helpers (`schtasks`,
    /// etc.) where the caller needs the result rather than a PID. We ask for
    /// `SEE_MASK_NOCLOSEPROCESS` so `hProcess` is filled in and can be waited
    /// on; without that flag the struct returns a null handle and the only
    /// option would be polling `tasklist` for a PID, which races.
    ///
    /// `ShellExecuteExW` itself blocks until the UAC consent dialog is
    /// resolved, so by the time we reach the wait the user has already
    /// answered. The bounded wait below only guards against a hung helper.
    pub(super) fn runas_exec_wait(binary: &std::path::Path, args: &str) -> Result<i32> {
        let verb = wide("runas");
        let file = wide(&binary.to_string_lossy());
        let parameters = wide(args);

        let mut exec_info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOASYNC | SEE_MASK_NOCLOSEPROCESS,
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(parameters.as_ptr()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };

        // SAFETY: POD struct of pointers; the wide buffers outlive the call.
        unsafe { ShellExecuteExW(&mut exec_info) }.map_err(|e| {
            // ERROR_CANCELLED (1223) == the user dismissed the UAC prompt.
            // Surfacing it verbatim lets the UI say "you cancelled" instead
            // of a generic failure.
            AppError::Elevation(format!("ShellExecuteExW(runas) failed: {e}"))
        })?;

        let h: isize = exec_info.hInstApp.0 as isize;
        if h <= 32 {
            return Err(AppError::Elevation(format!(
                "ShellExecuteExW returned hInstApp={h} (<=32 = error)"
            )));
        }

        let process = exec_info.hProcess;
        if process.is_invalid() {
            return Err(AppError::Elevation(
                "elevated helper returned no process handle".into(),
            ));
        }

        // 120s is far beyond any `schtasks` round-trip; a helper that takes
        // longer than this is wedged and the caller needs to hear about it
        // rather than have the UI hang forever.
        const TIMEOUT_MS: u32 = 120_000;
        // SAFETY: `process` is a valid handle we own for the duration.
        let waited = unsafe { WaitForSingleObject(process, TIMEOUT_MS) };
        if waited != WAIT_OBJECT_0 {
            // SAFETY: releasing the handle we asked for above.
            unsafe {
                let _ = CloseHandle(process);
            }
            return Err(AppError::Elevation(format!(
                "elevated helper did not exit within {TIMEOUT_MS} ms"
            )));
        }

        let mut code: u32 = 0;
        // SAFETY: valid handle, valid out-pointer.
        let status = unsafe { GetExitCodeProcess(process, &mut code) };
        // SAFETY: same handle; must be closed because we created it.
        unsafe {
            let _ = CloseHandle(process);
        }

        status.map_err(|e| AppError::Elevation(format!("GetExitCodeProcess: {e}")))?;
        Ok(code as i32)
    }

    fn resolve_pid_via_tasklist(binary: &Path) -> std::io::Result<u32> {
        let name = binary
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "mihomo.exe".to_string());
        let out = Command::new("tasklist")
            .args(["/FO", "CSV", "/NH", "/FI", &format!("IMAGENAME eq {name}")])
            .creation_flags(CREATE_NO_WINDOW)
            .output()?;
        if !out.status.success() {
            return Err(std::io::Error::other("tasklist exit non-zero"));
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

    /// Stop the elevated kernel.
    ///
    /// Primary path is the spawn handle: `TerminateProcess` on a handle we
    /// own works regardless of the integrity level of the target, so an
    /// asInvoker FlexClash can retire the UAC-elevated mihomo. The
    /// `taskkill` routes are only reached when we have no handle, and they
    /// are bounded — an unbounded `taskkill` used to be able to wedge the
    /// whole TUN transition.
    ///
    /// A failure here is **reported**, never swallowed: the old code turned
    /// every non-zero `taskkill` status into `Ok(())`, so "关闭 TUN" reported
    /// success while the elevated kernel went on holding 9091/7897.
    pub(super) fn graceful_stop(
        pid: u32,
        _child: Option<std::process::Child>,
        handle: Option<ProcHandle>,
    ) -> Result<()> {
        if let Some(h) = handle {
            lifecycle(&format!(
                "Sending termination to elevated kernel (pid={pid}, handle=owned)"
            ));
            if terminate_handle(h, 0) {
                lifecycle(&format!("Elevated kernel terminated (pid={pid})"));
                return Ok(());
            }
            lifecycle(&format!(
                "TerminateProcess failed for pid={pid}; falling back to taskkill"
            ));
        } else {
            lifecycle(&format!(
                "No spawn handle for pid={pid}; falling back to taskkill"
            ));
        }

        if pid != 0 {
            match taskkill(&["/PID", &pid.to_string(), "/T", "/F"]) {
                Ok(true) => {
                    lifecycle(&format!(
                        "Elevated kernel terminated via taskkill (pid={pid})"
                    ));
                    return Ok(());
                }
                Ok(false) => {}
                Err(e) => eprintln!("[elevate] taskkill /PID {pid} could not run: {e}"),
            }
        }

        // Name sweep. The glob is load-bearing — see KILL_IMAGE_GLOB.
        match taskkill(&["/IM", KILL_IMAGE_GLOB, "/T", "/F"]) {
            Ok(true) => {
                lifecycle("Elevated kernel terminated via image-name sweep");
                Ok(())
            }
            Ok(false) => Err(AppError::Tun(format!(
                "could not stop the elevated mihomo: taskkill found no killable target \
                 (image glob {KILL_IMAGE_GLOB}, timeout={KILL_TIMEOUT:?})"
            ))),
            Err(e) => Err(AppError::Tun(format!(
                "could not stop the elevated mihomo: {e}"
            ))),
        }
    }

    /// `taskkill` with a bounded budget. `Ok(true)` means the target is gone,
    /// `Ok(false)` means taskkill ran but matched nothing killable.
    fn taskkill(args: &[&str]) -> std::io::Result<bool> {
        let out = crate::core::proc::run_with_timeout("taskkill", args, KILL_TIMEOUT)?;
        if out.timed_out {
            eprintln!("[elevate] taskkill {args:?} timed out after {KILL_TIMEOUT:?}");
            return Ok(false);
        }
        if out.success || out.code == Some(TASKKILL_NO_MATCH) {
            Ok(true)
        } else {
            eprintln!(
                "[elevate] taskkill {args:?} exited {:?}: {}",
                out.code,
                out.stderr.trim()
            );
            Ok(false)
        }
    }

    /// Terminate through the owned handle, then close it.
    ///
    /// Returns whether the process was actually terminated. The wait is
    /// bounded so a kernel wedged in an uninterruptible wait cannot hold the
    /// transition open forever.
    fn terminate_handle(handle: ProcHandle, code: u32) -> bool {
        if handle.is_invalid() {
            return false;
        }
        // SAFETY: `handle` is a live PROCESS_ALL_ACCESS handle we own from
        // the spawn; TerminateProcess cannot fail for ownership reasons.
        let killed = unsafe { TerminateProcess(handle, code) }.is_ok();
        if killed {
            // SAFETY: bounded wait on the same handle, then we close it
            // exactly once below.
            let _ = unsafe { WaitForSingleObject(handle, REAP_TIMEOUT_MS) };
        }
        // SAFETY: we created this handle in `runas_spawn`, so we own the
        // close — and it must happen on every path.
        unsafe {
            let _ = CloseHandle(handle);
        }
        killed
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::*;
    use std::path::{Path, PathBuf};
    use tauri::{AppHandle, Runtime};

    /// Linux has no Wintun/UAC elevation step: mihomo only needs the
    /// `cap_net_admin` (+ `cap_net_bind_service`) capabilities, so we spawn
    /// the sidecar directly instead of failing with a Windows-only error.
    #[cfg(target_os = "linux")]
    pub(super) fn resolve_mihomo_binary<R: Runtime>(
        app: &AppHandle<R>,
        work_dir: &Path,
    ) -> Result<PathBuf> {
        use tauri::path::BaseDirectory;
        use tauri::Manager;

        const SIDECAR_BIN: &str = "mihomo-x86_64-unknown-linux-gnu";
        const SIDECAR_BIN_FALLBACK: &str = "mihomo";
        const SOURCE_BINARIES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/binaries");

        let mut candidates: Vec<PathBuf> = Vec::new();

        // (1) Tauri 2 native resolution (dev points at the source
        //     `binaries/` dir; production points at the bundle resource).
        if let Ok(p) = app.path().resolve(SIDECAR_BIN, BaseDirectory::Resource) {
            if p.exists() {
                candidates.push(p);
            }
        }

        // (2) Compile-time source-of-truth for `tauri:dev`.
        candidates.push(PathBuf::from(SOURCE_BINARIES_DIR).join(SIDECAR_BIN));

        // (3) & (4) Production installs: sidecar copied next to the
        //     running exe, optionally under `resources/`.
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join(SIDECAR_BIN));
                candidates.push(dir.join("resources").join(SIDECAR_BIN));
            }
        }

        // (5) AppData work dir (legacy / user-overridden) + bare fallback.
        candidates.push(work_dir.join(SIDECAR_BIN));
        candidates.push(work_dir.join(SIDECAR_BIN_FALLBACK));

        // (6) Bare `mihomo` next to the running exe.
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join(SIDECAR_BIN_FALLBACK));
            }
        }

        for cand in &candidates {
            if cand.exists() {
                let abs = cand.canonicalize().unwrap_or_else(|_| cand.clone());
                eprintln!("[elevate] resolved mihomo binary: {}", abs.display());
                return Ok(abs);
            }
        }

        Err(AppError::Tun(format!(
            "mihomo binary not found in any of the {} search paths; first candidate was {}",
            candidates.len(),
            candidates
                .first()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        )))
    }

    #[cfg(target_os = "macos")]
    pub(super) fn resolve_mihomo_binary<R: Runtime>(
        _app: &AppHandle<R>,
        _work_dir: &Path,
    ) -> Result<PathBuf> {
        Err(AppError::Tun("elevation is Windows-only in Phase 1".into()))
    }

    #[cfg(target_os = "linux")]
    pub(super) fn runas_spawn(
        binary: &Path,
        work_dir: &Path,
        config_path: &Path,
    ) -> Result<(u32, Option<std::process::Child>, Option<ProcHandle>)> {
        // A previous mihomo (the regular sidecar or a stale TUN child) may
        // still be holding the controller (9091) / mixed (7897) ports. If we
        // spawn a second kernel while those are bound, the new process fails
        // to bind and exits immediately, showing up as `<defunct>` and
        // leaving the app with no working kernel. Reap every mihomo instance
        // first, then give the OS a moment to release the sockets.
        pkill_all();
        std::thread::sleep(std::time::Duration::from_millis(300));

        let child = std::process::Command::new(binary)
            .args([
                "-d",
                work_dir.to_string_lossy().as_ref(),
                "-f",
                config_path.to_string_lossy().as_ref(),
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| AppError::Tun(format!("failed to launch mihomo: {e}")))?;
        // Linux carries no extra OS handle: the `Child` we keep is both the
        // handle and the reap.
        Ok((child.id(), Some(child), None))
    }

    #[cfg(target_os = "macos")]
    pub(super) fn runas_spawn(
        _binary: &Path,
        _work_dir: &Path,
        _config_path: &Path,
    ) -> Result<(u32, Option<std::process::Child>, Option<ProcHandle>)> {
        Err(AppError::Tun("elevation is Windows-only in Phase 1".into()))
    }

    /// Last-resort sweep by process name. Bounded: a wedged `pkill` must not
    /// be able to hold the TUN transition open.
    fn pkill_all() {
        if let Ok(out) = crate::core::proc::run_with_timeout(
            "pkill",
            &["-9", "-f", "mihomo"],
            std::time::Duration::from_secs(5),
        ) {
            if out.timed_out {
                eprintln!("[elevate] pkill -9 -f mihomo timed out");
            }
        }
    }

    /// Grace period between SIGTERM and the SIGKILL escalation.
    const TERM_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

    #[cfg(target_os = "linux")]
    pub(super) fn graceful_stop(
        pid: u32,
        mut child: Option<std::process::Child>,
        _handle: Option<ProcHandle>,
    ) -> Result<()> {
        if pid == 0 {
            pkill_all();
        } else {
            lifecycle(&format!("Sending SIGTERM to elevated kernel (pid={pid})"));
            // `kill -TERM` returning 0 only proves the signal was
            // *delivered*; it says nothing about whether mihomo will ever act
            // on it. The old code escalated to SIGKILL purely on that exit
            // status and then called an unbounded `child.wait()`, so a kernel
            // that hung while tearing down its TUN device / nft chain wedged
            // the transition forever. Escalate on a clock instead.
            let delivered = matches!(
                crate::core::proc::run_with_timeout(
                    "kill",
                    &["-TERM", &pid.to_string()],
                    std::time::Duration::from_secs(5),
                ),
                Ok(o) if o.success
            );
            if !delivered || !reap_within(child.as_mut(), TERM_GRACE) {
                lifecycle(&format!(
                    "SIGTERM grace expired; sending SIGKILL (pid={pid})"
                ));
                let _ = crate::core::proc::run_with_timeout(
                    "kill",
                    &["-KILL", &pid.to_string()],
                    std::time::Duration::from_secs(5),
                );
                if !reap_within(child.as_mut(), TERM_GRACE) {
                    pkill_all();
                }
            }
        }

        // Never leave a zombie behind. If the kernel somehow survived both
        // signals, take it down through the `Child` handle and reap it —
        // but still on a bounded clock.
        if let Some(c) = child.as_mut() {
            if !reap_within(Some(&mut *c), TERM_GRACE) {
                let _ = c.kill();
                reap_within(Some(&mut *c), TERM_GRACE);
            }
        }
        lifecycle("Elevated kernel reaped");
        Ok(())
    }

    /// Poll `child` for at most `budget`. Returns true once it is gone.
    ///
    /// A missing handle counts as "nothing to reap". A `try_wait` error is
    /// reported as *not* reaped, so the caller escalates rather than
    /// silently assuming success.
    fn reap_within(child: Option<&mut std::process::Child>, budget: std::time::Duration) -> bool {
        let Some(child) = child else { return true };
        let deadline = Instant::now() + budget;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return true,
                Ok(None) => {}
                Err(e) => {
                    eprintln!("[elevate] try_wait failed: {e}");
                    return false;
                }
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn graceful_stop(
        _pid: u32,
        _child: Option<std::process::Child>,
        _handle: Option<ProcHandle>,
    ) -> Result<()> {
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(super) fn runas_exec_wait(_binary: &Path, _args: &str) -> Result<i32> {
        Err(AppError::Elevation(
            "privilege elevation is Windows-only".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Resolve the mihomo binary path, then launch it elevated. Returns
/// the new PID on success; on UAC cancel returns `AppError::Tun` with
/// a message that the UI renders as a toast.
pub fn spawn_elevated_mihomo<R: Runtime>(app: &AppHandle<R>) -> Result<u32> {
    // Resolve the canonical work dir from the Tauri path API — deliberately
    // NOT from `SidecarHandle`, whose cached value is only populated once the
    // kernel has started (and degrades to `%TEMP%` before that), and
    // deliberately NOT with a `%TEMP%` fallback of our own.
    //
    // This is the difference between working and broken TUN: `-d` and the
    // `config.yaml` path are both derived from this answer, so a wrong answer
    // launches the elevated mihomo against `%TEMP%` — no profiles, no rules,
    // no DNS block — which the user sees as "TUN is on but nothing is
    // proxied". Failing loudly is strictly better than launching the wrong
    // binary against the wrong directory.
    let work_dir = crate::core::sidecar::work_dir_for(app)?;
    let config_path = work_dir.join("config.yaml");
    let binary = platform::resolve_mihomo_binary(app, &work_dir)?;

    let (pid, child, handle) = platform::runas_spawn(&binary, &work_dir, &config_path)?;
    *registry().lock().expect("elevated registry poisoned") = Some(ElevatedChild {
        pid,
        child,
        handle: handle.map(OwnedHandle),
    });
    Ok(pid)
}

/// Stop the elevated TUN kernel, reporting honestly whether it worked.
///
/// An empty registry still performs the image-name sweep: a stale elevated
/// kernel from a previous session can outlive our bookkeeping and would
/// otherwise keep the controller port bound.
pub fn stop_elevated_mihomo() -> Result<()> {
    let elevated = registry()
        .lock()
        .expect("elevated registry poisoned")
        .take();
    match elevated {
        Some(child) => platform::graceful_stop(child.pid, child.child, child.handle.map(|h| h.0)),
        None => platform::graceful_stop(0, None, None),
    }
}

/// Friendly hint appended to TUN startup failures on Linux.
///
/// Unlike Windows, there is no UAC/elevation gate: mihomo only needs the
/// `cap_net_admin` and `cap_net_bind_service` capabilities on the kernel
/// binary. When a launch or health-check fails, the user can fix it by
/// granting those capabilities once instead of being shown a generic
/// Windows-only error.
#[cfg(target_os = "linux")]
pub fn tun_startup_hint<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let path = crate::core::sidecar::work_dir_for(app)
        .ok()
        .and_then(|work_dir| platform::resolve_mihomo_binary(app, &work_dir).ok())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<内核路径>".to_string());
    Some(format!(
        "Linux 环境下开启 TUN 请执行：sudo setcap cap_net_admin,cap_net_bind_service=+ep {path}"
    ))
}

#[cfg(not(target_os = "linux"))]
pub fn tun_startup_hint<R: Runtime>(_app: &AppHandle<R>) -> Option<String> {
    None
}

/// Run a short-lived helper elevated and block until it finishes.
///
/// Used by the Task Scheduler autostart path, which needs `schtasks` to run
/// with administrator rights (creating a `HighestAvailable` task otherwise
/// fails with access-denied). Shows a UAC prompt; the caller is responsible
/// for making that an explicit, user-initiated action.
///
/// On non-Windows this is a typed error rather than a silent no-op: callers
/// would otherwise report success for something that never ran.
pub fn runas_exec_wait(binary: &Path, args: &str) -> Result<i32> {
    platform::runas_exec_wait(binary, args)
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
    let mut last_err: Option<std::io::Error> = None;
    while Instant::now() < deadline {
        match probe_version(addr) {
            Ok(()) => return Ok(()),
            // A refused connect on the first ticks is the *expected* bring-up
            // state, not an error. Keep the loop silent and remember the
            // cause; it is surfaced exactly once if the budget is exhausted.
            Err(e) => last_err = Some(e),
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let msg = format!("controller not healthy after {budget:?}: {last_err:?}");
    eprintln!("[probe] {addr} {msg}");
    Err(AppError::Tun(msg))
}

/// Minimal in-process HTTP/1.1 `GET /version` probe.
///
/// Deliberately does **not** shell out to `curl.exe`. This function is
/// polled every 200 ms for up to 15 s while TUN is coming up, so one
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
        Err(std::io::Error::other(format!(
            "unexpected /version status: {}",
            text.lines().next().unwrap_or("<no status line>")
        )))
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

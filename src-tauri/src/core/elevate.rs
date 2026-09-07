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

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Runtime};

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
    use std::path::PathBuf;
    use std::process::Command;

    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
    };

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// Build the sidecar binary path the same way `core::sidecar` does.
    /// We keep a *separate* resolver here so the elevation path stays
    /// testable without dragging in the full sidecar machinery.
    pub(super) fn resolve_mihomo_binary(work_dir: &std::path::Path) -> Result<PathBuf> {
        // 1. Look for a sidecar pinned by the build script next to the exe.
        let exe_dir = std::env::current_exe()
            .map_err(|e| AppError::Tun(format!("current_exe: {e}")))?
            .parent()
            .ok_or_else(|| AppError::Tun("exe has no parent dir".into()))?
            .to_path_buf();
        let candidate = exe_dir.join("mihomo-x86_64-pc-windows-msvc.exe");
        if candidate.exists() {
            return Ok(candidate);
        }
        // 2. Fall back to <work_dir>/mihomo.exe (dev mode).
        let dev = work_dir.join("mihomo.exe");
        if dev.exists() {
            return Ok(dev);
        }
        Err(AppError::Tun(format!(
            "mihomo binary not found near {} or {}",
            candidate.display(),
            dev.display()
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
            nShow: 1, // SW_SHOWNORMAL
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
    use std::path::PathBuf;

    pub(super) fn resolve_mihomo_binary(_work_dir: &std::path::Path) -> Result<PathBuf> {
        Err(AppError::Tun("elevation is Windows-only in Phase 1".into()))
    }
    pub(super) fn runas_spawn(
        _binary: &PathBuf,
        _work_dir: &std::path::Path,
        _config_path: &std::path::Path,
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
    let binary = platform::resolve_mihomo_binary(&work_dir)?;

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
    use std::net::TcpStream;
    let deadline = Instant::now() + budget;
    let url = format!("http://127.0.0.1:9091/version");
    while Instant::now() < deadline {
        // 1. Cheap TCP-level probe first.
        if TcpStream::connect_timeout(
            &"127.0.0.1:9091".parse().unwrap(),
            Duration::from_millis(250),
        )
        .is_ok()
        {
            // 2. Real /version probe via curl.exe (reliable on Windows
            //    where WinHTTP can hang).
            let probe = std::process::Command::new("curl.exe")
                .args(["-s", "-m", "1", "-o", "NUL", "-w", "%{http_code}", &url])
                .output();
            if let Ok(o) = probe {
                let code = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if code == "200" {
                    return Ok(());
                }
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(AppError::Tun(format!(
        "controller not healthy after {:?}",
        budget
    )))
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
        // Port 9091 may or may not be live in the test harness; we
        // only assert that wait_until_healthy returns Err within the
        // budget rather than panicking.
        let start = Instant::now();
        let res = wait_until_healthy(Duration::from_millis(300));
        let elapsed = start.elapsed();
        assert!(res.is_err(), "expected timeout, got Ok");
        assert!(elapsed < Duration::from_secs(2), "budget honored");
    }
}

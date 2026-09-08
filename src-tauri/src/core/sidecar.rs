//! Mihomo sidecar lifecycle management.
//!
//! Responsibilities:
//! - Ensure the working directory & a minimal default `config.yaml` exist
//!   (with port auto-refresh so a stale 9090 default never lingers).
//! - Spawn the Mihomo sidecar child process and track its handle
//!   (`Arc<Mutex<Option<CommandChild>>>`).
//! - Stream stdout/stderr to the frontend as `kernel://log` events.
//! - Provide graceful stop + hard cleanup fallback (no zombie `mihomo.exe`).
//!
//! All public functions are synchronous w.r.t. the mutex: the mutex is held
//! only for trivial state mutations; never across an `.await` point.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_shell::process::{CommandChild, CommandEvent, TerminatedPayload};
use tauri_plugin_shell::ShellExt;

use crate::error::{AppError, Result};

const SIDECAR_NAME: &str = "mihomo";
const DEFAULT_CONFIG_FILE: &str = "config.yaml";
const LOG_TAIL_CAP: usize = 200;

/// Expected external-controller port. The frontend hard-codes the same value
/// in `src/services/clash.ts`. If either drifts, `ensure_default_config` will
/// rewrite the on-disk yaml and emit `KERNEL_CONFIG_REFRESHED`.
pub const EXPECTED_CONTROLLER_PORT: u16 = 9091;

// ---------------------------------------------------------------------------
// State types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum KernelState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Crashed,
}

impl Default for KernelState {
    fn default() -> Self { KernelState::Stopped }
}

/// Result of `ensure_default_config`. Frontend can react via
/// `kernel://config-refreshed` event.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConfigRefresh {
    /// File did not exist; just written.
    Created,
    /// File existed but `external-controller` port didn't match.
    /// Overwritten; old port reported as `from_port` (None if unparseable).
    PortChanged { from_port: Option<u16>, to_port: u16 },
    /// File existed but the `flexclash-config-version` comment was
    /// older (or missing).  Overwritten; bumped to `to_version`.
    /// The UI should prompt the user to restart the kernel so the
    /// new rules / schema take effect.
    SchemaBumped {
        from_version: u32,
        to_version: u32,
        from_port: Option<u16>,
        to_port: u16,
    },
    /// File exists and port already matches; no write performed.
    Unchanged,
}

#[derive(Default)]
pub struct SidecarInner {
    /// The spawned child. `take()`-en out to kill; replaced on restart.
    pub child: Option<CommandChild>,
    pub state: KernelState,
    pub work_dir: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
    /// Bounded ring buffer of recent log lines, kept for crash diagnostics.
    pub recent_log_tail: Vec<String>,
}

/// Thread-safe handle exposed via `app.manage()`. Cheap to clone (Arc bump).
#[derive(Clone, Default)]
pub struct SidecarHandle(pub Arc<Mutex<SidecarInner>>);

impl SidecarHandle {
    pub fn new() -> Self { Self::default() }

    fn lock(&self) -> std::sync::MutexGuard<'_, SidecarInner> {
        self.0.lock().expect("sidecar mutex poisoned")
    }

    pub fn state(&self) -> KernelState { self.lock().state }

    pub fn set_state(&self, new_state: KernelState) {
        self.lock().state = new_state;
    }

    /// Resolved working directory used by the most recent `start()`.
    /// `None` before the first start; falls back to a tmp-style path
    /// when called pre-start so callers (TUN manager, elevate) never
    /// have to special-case the cold-boot path.
    pub fn work_dir(&self) -> PathBuf {
        self.lock()
            .work_dir
            .clone()
            .unwrap_or_else(std::env::temp_dir)
    }

    /// Resolved active config path. Same fallback rules as `work_dir()`.
    pub fn config_path(&self) -> PathBuf {
        self.lock()
            .config_path
            .clone()
            .unwrap_or_else(|| self.work_dir().join(DEFAULT_CONFIG_FILE))
    }

    /// Take the child out and kill it. Idempotent: safe to call when stopped.
    pub fn try_kill(&self) -> Result<()> {
        if let Some(child) = self.lock().child.take() {
            // CommandChild::kill consumes self.
            child.kill().map_err(|e| AppError::Shell(e.to_string()))?;
        }
        Ok(())
    }

    fn set_child(&self, child: Option<CommandChild>) {
        self.lock().child = child;
    }

    fn push_log(&self, line: String) {
        let mut g = self.lock();
        if g.recent_log_tail.len() >= LOG_TAIL_CAP {
            g.recent_log_tail.remove(0);
        }
        g.recent_log_tail.push(line);
    }
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// Start the Mihomo sidecar. Refuses if already running.
pub async fn start<R: Runtime>(app: &AppHandle<R>, handle: SidecarHandle) -> Result<()> {
    // Guard against double-start.
    {
        let cur = handle.state();
        if matches!(cur, KernelState::Starting | KernelState::Running) {
            return Err(AppError::AlreadyRunning);
        }
    }
    handle.set_state(KernelState::Starting);

    // 1) Materialise work dir + default config (with port-refresh logic).
    let work_dir = ensure_work_dir(app)?;
    let (config_path, refresh) = ensure_default_config(&work_dir)?;
    {
        let mut g = handle.lock();
        g.work_dir = Some(work_dir.clone());
        g.config_path = Some(config_path.clone());
    }

    // Announce config outcome via logs + event.
    match refresh {
        ConfigRefresh::Created => {
            let msg = format!("[config] created default config at {}", config_path.display());
            handle.push_log(msg.clone());
            let _ = app.emit(crate::events::KERNEL_LOG, msg);
        }
        ConfigRefresh::PortChanged { from_port, to_port } => {
            let msg = format!(
                "[config] external-controller port auto-refreshed: {} -> {}",
                from_port.map(|p| p.to_string()).unwrap_or_else(|| "?".into()),
                to_port
            );
            handle.push_log(msg.clone());
            let _ = app.emit(crate::events::KERNEL_LOG, msg);
            let _ = app.emit(
                crate::events::KERNEL_CONFIG_REFRESHED,
                ConfigRefresh::PortChanged { from_port, to_port },
            );
        }
        ConfigRefresh::SchemaBumped { from_version, to_version, from_port, to_port } => {
            let msg = format!(
                "[config] schema bumped v{} -> v{} (added TUN loopback bypass rules, etc.) — please restart the kernel",
                from_version, to_version
            );
            handle.push_log(msg.clone());
            let _ = app.emit(crate::events::KERNEL_LOG, msg);
            let _ = app.emit(
                crate::events::KERNEL_CONFIG_REFRESHED,
                ConfigRefresh::SchemaBumped { from_version, to_version, from_port, to_port },
            );
        }
        ConfigRefresh::Unchanged => {}
    }

    // 2) Spawn sidecar.
    let shell = app.shell();
    let cmd = shell.sidecar(SIDECAR_NAME)?.args([
        "-d",
        work_dir.to_string_lossy().as_ref(),
        "-f",
        config_path.to_string_lossy().as_ref(),
    ]);
    let (mut rx, child) = cmd.spawn()?;
    handle.set_child(Some(child));
    handle.set_state(KernelState::Running);
    let _ = app.emit(crate::events::KERNEL_STATE, KernelState::Running);

    // Surface a clear banner on the kernel log so the user can see what
    // URL to hit from a browser if the dashboard ever fails to connect.
    // Also serves as a sanity check that CORS allow-origins made it into
    // the rendered config (grep the user's mihomo log later if not).
    let boot_msg = format!(
        "[sidecar] mihomo up — RESTful API on http://127.0.0.1:{EXPECTED_CONTROLLER_PORT} (CORS: *)",
    );
    handle.push_log(boot_msg.clone());
    let _ = app.emit(crate::events::KERNEL_LOG, boot_msg);

    // 3) Drain events: forward logs, watch termination.
    let app_for_task = app.clone();
    let handle_for_task = handle.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(bytes) => {
                    let line = String::from_utf8_lossy(&bytes).into_owned();
                    handle_for_task.push_log(line.clone());
                    let _ = app_for_task.emit(crate::events::KERNEL_LOG, line);
                }
                CommandEvent::Stderr(bytes) => {
                    let line = String::from_utf8_lossy(&bytes).into_owned();
                    handle_for_task.push_log(line.clone());
                    let _ = app_for_task.emit(crate::events::KERNEL_LOG, line);
                }
                CommandEvent::Error(s) => {
                    let line = format!("[sidecar-error] {s}");
                    handle_for_task.push_log(line.clone());
                    let _ = app_for_task.emit(crate::events::KERNEL_LOG, line);
                }
                CommandEvent::Terminated(TerminatedPayload { code, signal }) => {
                    handle_for_task.set_child(None);
                    // Visible in dev-terminal + kernel log so a crash's real
                    // exit code / signal is never hidden by the watcher.
                    eprintln!(
                        "[sidecar] mihomo exited code={code:?} signal={signal:?}"
                    );
                    handle_for_task.push_log(format!(
                        "[sidecar] mihomo exited (code={code:?}, signal={signal:?})"
                    ));
                    let new_state = if code == Some(0) {
                        KernelState::Stopped
                    } else {
                        KernelState::Crashed
                    };
                    handle_for_task.set_state(new_state);
                    let _ = app_for_task.emit(crate::events::KERNEL_STATE, new_state);
                    let _ = app_for_task.emit(
                        crate::events::KERNEL_TERMINATED,
                        serde_json::json!({ "code": code, "signal": signal }),
                    );
                    break;
                }
                _ => {}
            }
        }
    });

    Ok(())
}

/// Graceful stop. No-op if already stopped/crashed.
pub fn stop(handle: SidecarHandle) -> Result<()> {
    let cur = handle.state();
    if matches!(cur, KernelState::Stopped | KernelState::Crashed) {
        return Ok(());
    }
    handle.set_state(KernelState::Stopping);
    handle.try_kill()?;
    // The background drain task will flip state to Stopped on Terminated.
    Ok(())
}

/// Restart: stop, brief pause (release port / file locks), start.
pub async fn restart<R: Runtime>(app: &AppHandle<R>, handle: SidecarHandle) -> Result<()> {
    stop(handle.clone())?;
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    start(app, handle).await
}

/// Last-resort cleanup. MUST NOT depend on `AppHandle` (it may be dropping).
/// Belt-and-suspenders against zombie processes on all desktop OSes.
pub fn hard_cleanup() {
    eprintln!("[shutdown] running hard_cleanup for Mihomo sidecar");
    #[cfg(target_os = "windows")]
    {
        // /IM wildcard matches mihomo.exe AND mihomo-x86_64-pc-windows-msvc.exe
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/IM", "mihomo*.exe"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let _ = std::process::Command::new("pkill")
            .args(["-9", "-f", "mihomo"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

/// Public so other modules (e.g. `commands::profile`) can locate the same
/// mihomo work dir that `start()` uses. Pure function: does not touch FS.
pub fn work_dir_for<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf> {
    let base = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Path(format!("app_local_data_dir: {e}")))?;
    Ok(base.join("mihomo"))
}

fn ensure_work_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf> {
    let dir = work_dir_for(app)?;
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
    }
    Ok(dir)
}

/// Materialise the bundled minimal config.
///
/// - Missing                  → create (`Created`).
/// - Stale port               → overwrite (`PortChanged`).
/// - Stale schema (bump the
///   `flexclash-config-version`
///   comment in the bundle
///   to force a refresh)     → overwrite (`SchemaBumped`).
/// - Fresh                    → no-op (`Unchanged`).
///
/// Phase 1 ships only one default yaml — `Real profiles` (M4) will land in
/// `profiles/<id>/config.yaml` and will NOT be touched by this function.
fn ensure_default_config(work_dir: &Path) -> Result<(PathBuf, ConfigRefresh)> {
    let path = work_dir.join(DEFAULT_CONFIG_FILE);
    let bundled = include_str!("../../resources/default_mihomo.yaml");
    let expected = format!("127.0.0.1:{EXPECTED_CONTROLLER_PORT}");

    if !path.exists() {
        std::fs::write(&path, bundled)?;
        return Ok((path, ConfigRefresh::Created));
    }

    let existing = std::fs::read_to_string(&path)?;

    // PROTECTION: a config WITHOUT our version marker is a user-activated
    // subscription (activate_profile fs-copies the profile yaml verbatim)
    // or a hand-edited file. NEVER clobber it back to the bundled default
    // — that is what made imported nodes vanish after a restart.
    if extract_config_version(&existing).is_none() {
        return Ok((path, ConfigRefresh::Unchanged));
    }

    // Bump detection — only for configs we own (they carry the marker).
    // A stale schema (e.g. new TUN bypass rules) is pushed by rewriting
    // the bundled default, never a user file.
    if let (Some(user_v), Some(bundled_v)) = (
        extract_config_version(&existing),
        extract_config_version(bundled),
    ) {
        if user_v < bundled_v {
            let from_port = extract_controller_port(&existing);
            std::fs::write(&path, bundled)?;
            return Ok((
                path,
                ConfigRefresh::SchemaBumped {
                    from_version: user_v,
                    to_version: bundled_v,
                    from_port,
                    to_port: EXPECTED_CONTROLLER_PORT,
                },
            ));
        }
    }

    if existing.contains(&expected) {
        return Ok((path, ConfigRefresh::Unchanged));
    }

    let from_port = extract_controller_port(&existing);
    std::fs::write(&path, bundled)?;
    Ok((path, ConfigRefresh::PortChanged { from_port, to_port: EXPECTED_CONTROLLER_PORT }))
}

/// Parses the `# flexclash-config-version: N` comment.  Returns `None`
/// if the marker is absent or malformed.
fn extract_config_version(yaml: &str) -> Option<u32> {
    yaml.lines()
        .map(str::trim_start)
        .find(|l| l.starts_with("# flexclash-config-version:"))
        .and_then(|l| l.rsplit(':').next())
        .and_then(|s| s.trim().parse().ok())
}

/// Naive single-line parser: looks for `external-controller: 127.0.0.1:NNNN`.
fn extract_controller_port(yaml: &str) -> Option<u16> {
    yaml.lines()
        .find(|l| l.trim_start().starts_with("external-controller:"))
        .and_then(|l| l.rsplit(':').next())
        .and_then(|s| s.trim().trim_matches('"').parse().ok())
}

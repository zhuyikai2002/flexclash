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
// Brings `Event::emit` into scope so `ConfigRefreshPayload` can be pushed with
// the typed `emit(app)` API instead of a raw `app.emit("…", …)` string.
use tauri_specta::Event;

use crate::config::profile::{patch_and_sanitize_yaml, RESERVED_MIXED_PORT};
use crate::error::{AppError, Result};

const SIDECAR_NAME: &str = "mihomo";
const DEFAULT_CONFIG_FILE: &str = "config.yaml";
const LOG_TAIL_CAP: usize = 200;

/// Expected external-controller port. The frontend hard-codes the same value
/// in `src/services/clash.ts`. If either drifts, `ensure_default_config` will
/// rewrite the on-disk yaml and emit `ConfigRefreshPayload`.
pub const EXPECTED_CONTROLLER_PORT: u16 = 9091;

// ---------------------------------------------------------------------------
// State types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum KernelState {
    #[default]
    Stopped,
    Starting,
    Running,
    Stopping,
    /// The supervisor is auto-restarting after a crash. A transient state the
    /// renderer renders as "transitioning" — the kernel is down but a
    /// supervised relaunch is already scheduled (see `core::supervisor`).
    Recovering,
    Crashed,
}

/// Why the sidecar process exited, as observed by the drain task.
///
/// This is the *internal* signal the supervisor (`core::supervisor`) consumes
/// to decide whether to auto-restart. Deliberately not a wire type: the
/// renderer sees the derived `KernelState` plus the typed `SupervisorEvent`.
#[derive(Debug, Clone)]
pub enum ExitEvent {
    /// Clean exit (code 0) or an intentional kill (`stop()` / app shutdown).
    Clean,
    /// Abnormal exit: the kernel crashed (non-zero exit or killed by signal).
    Crash {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

/// Result of `ensure_default_config`, delivered to the renderer as the typed
/// `ConfigRefreshPayload` event (see `core::kernel_events` for why the payloads
/// are typed rather than raw).
///
/// This used to be emitted on the string event `kernel://config-refreshed`,
/// with the renderer carrying a hand-written mirror of the enum in
/// `stores/kernel.ts`. Deriving `Event` retires both: the discriminant and
/// every field now have exactly one definition.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConfigRefreshPayload {
    /// File did not exist; just written.
    Created,
    /// File existed but `external-controller` port didn't match.
    /// Overwritten; old port reported as `from_port` (None if unparseable).
    PortChanged {
        from_port: Option<u16>,
        to_port: u16,
    },
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

pub struct SidecarInner {
    /// The spawned child. `take()`-en out to kill; replaced on restart.
    pub child: Option<CommandChild>,
    pub state: KernelState,
    pub work_dir: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
    /// Bounded ring buffer of recent log lines, kept for crash diagnostics.
    pub recent_log_tail: Vec<String>,
    /// Monotonic launch counter. The drain task snapshots it at spawn; on
    /// `Terminated` it only acts if it is still the latest launch, so a stale
    /// task from a superseded `start()` cannot clobber a newer state.
    generation: u64,
    /// Set when the child is killed *intentionally* (`stop()` / shutdown), so
    /// the drain task can tell a clean stop from a crash. Cleared by the next
    /// `start()`, which always launches a fresh child that is not being stopped.
    stop_requested: bool,
    /// Supervisor notification channel. The drain task sends an `ExitEvent`
    /// here the instant the child terminates; `core::supervisor` claims the
    /// receive half (once) and owns every restart decision.
    crash_tx: tokio::sync::mpsc::UnboundedSender<ExitEvent>,
    crash_rx: Option<tokio::sync::mpsc::UnboundedReceiver<ExitEvent>>,
}

impl Default for SidecarInner {
    fn default() -> Self {
        let (crash_tx, crash_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            child: None,
            state: KernelState::Stopped,
            work_dir: None,
            config_path: None,
            recent_log_tail: Vec::new(),
            generation: 0,
            stop_requested: false,
            crash_tx,
            crash_rx: Some(crash_rx),
        }
    }
}

/// Thread-safe handle exposed via `app.manage()`. Cheap to clone (Arc bump).
#[derive(Clone, Default)]
pub struct SidecarHandle(pub Arc<Mutex<SidecarInner>>);

impl SidecarHandle {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SidecarInner> {
        self.0.lock().expect("sidecar mutex poisoned")
    }

    pub fn state(&self) -> KernelState {
        self.lock().state
    }

    pub fn set_state(&self, new_state: KernelState) {
        self.lock().state = new_state;
    }

    /// Resolved working directory used by the most recent `start()`.
    ///
    /// Before the first start this resolves the canonical
    /// `<app_local_data_dir>/mihomo` through the Tauri path API instead of
    /// guessing. Every caller downstream derives a real filesystem path from
    /// this answer — the TUN config injection, the elevated `-d`, the active
    /// `config.yaml` — so returning `%TEMP%` here is not a harmless default,
    /// it is a silent misconfiguration (mihomo comes up with no profiles, no
    /// rules and no DNS, which reads to the user as "TUN on, nothing
    /// proxied"). `%TEMP%` survives only as a last resort for when the app
    /// handle itself is already gone, e.g. during teardown.
    pub fn work_dir(&self) -> PathBuf {
        if let Some(dir) = self.lock().work_dir.clone() {
            return dir;
        }
        if let Some(app) = crate::core::elevate::current_app_handle() {
            if let Ok(dir) = work_dir_for(&app) {
                return dir;
            }
        }
        eprintln!(
            "[sidecar] WARNING: work_dir() requested before the kernel started and no \
             app handle is available; falling back to {}",
            std::env::temp_dir().display()
        );
        std::env::temp_dir()
    }

    /// Resolved active config path. Same fallback rules as `work_dir()`.
    pub fn config_path(&self) -> PathBuf {
        self.lock()
            .config_path
            .clone()
            .unwrap_or_else(|| self.work_dir().join(DEFAULT_CONFIG_FILE))
    }

    /// Take the child out and kill it. Idempotent: safe to call when stopped.
    ///
    /// Marks the kill as *intentional* (`stop_requested`) so the drain task
    /// reports a `Clean` exit instead of a crash — otherwise the supervisor
    /// would auto-restart a kernel the user just stopped. This is the only
    /// place the flag is armed, and both intentional-stop call sites (`stop()`
    /// and app shutdown) go through here.
    pub fn try_kill(&self) -> Result<()> {
        // Two brief lock acquisitions, never held across the blocking `kill()`:
        // matching the module's discipline that the mutex covers only trivial
        // state mutations.
        self.lock().stop_requested = true;
        let child = self.lock().child.take();
        if let Some(child) = child {
            // CommandChild::kill consumes self.
            child.kill().map_err(|e| AppError::Shell(e.to_string()))?;
        }
        Ok(())
    }

    fn set_child(&self, child: Option<CommandChild>) {
        self.lock().child = child;
    }

    /// Mark the beginning of a fresh launch: clear the intentional-stop flag
    /// and bump the generation, returning the new generation for the drain
    /// task to snapshot against.
    fn begin_launch(&self) -> u64 {
        let mut g = self.lock();
        g.stop_requested = false;
        g.generation += 1;
        g.generation
    }

    /// Current launch generation. The drain task compares its snapshot against
    /// this to detect that it has been superseded by a newer `start()`.
    fn generation(&self) -> u64 {
        self.lock().generation
    }

    /// Read-and-clear the intentional-stop flag, for the drain task's
    /// `Terminated` classification.
    fn take_stop_requested(&self) -> bool {
        let mut g = self.lock();
        let was = g.stop_requested;
        g.stop_requested = false;
        was
    }

    /// Claim the crash-notification receiver. Called once by `core::supervisor`
    /// at setup; returns `None` if already claimed.
    pub fn take_crash_rx(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<ExitEvent>> {
        self.lock().crash_rx.take()
    }

    /// Notify the supervisor that the child exited. Fire-and-forget: the
    /// supervisor may be gone (e.g. during teardown), and a dropped receiver
    /// is not an error.
    fn notify_exit(&self, event: ExitEvent) {
        let _ = self.lock().crash_tx.send(event);
    }

    fn push_log(&self, line: String) {
        let mut g = self.lock();
        if g.recent_log_tail.len() >= LOG_TAIL_CAP {
            g.recent_log_tail.remove(0);
        }
        g.recent_log_tail.push(line.clone());
        // Persist every kernel/init log line to <work_dir>/runtime.log so a
        // crash's real cause survives process teardown (the ring buffer is
        // memory-only). Writing is best-effort and unbuffered per line.
        if let Some(dir) = &g.work_dir {
            use std::io::Write;
            if let Ok(f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("runtime.log"))
            {
                let mut f = f;
                let _ = writeln!(f, "{line}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// Start the Mihomo sidecar. Refuses if already running.
pub async fn start<R: Runtime>(app: &AppHandle<R>, handle: SidecarHandle) -> Result<()> {
    // Single choke point: while TUN owns the controller/mixed ports, starting
    // the regular sidecar is always wrong. It would fight the elevated TUN
    // kernel for 9091/7897, and the resulting forced exit would be handed to
    // the supervisor as a crash. Every caller (kernel commands, profile
    // reload fallbacks, supervisor, ...) funnels through here.
    if crate::core::tun::owns_ports(app) {
        let msg = "[sidecar] start refused — TUN owns the kernel (9091/7897)";
        eprintln!("{msg}");
        let _ = app.emit(crate::events::KERNEL_LOG, msg);
        // Normalise a stale `Running`/`Recovering` so the UI does not claim
        // the regular kernel is alive while the elevated TUN kernel is the
        // one serving 9091/7897.
        handle.set_state(KernelState::Stopped);
        return Ok(());
    }

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
        ConfigRefreshPayload::Created => {
            let msg = format!(
                "[config] created default config at {}",
                config_path.display()
            );
            handle.push_log(msg.clone());
            let _ = app.emit(crate::events::KERNEL_LOG, msg);
        }
        ConfigRefreshPayload::PortChanged { from_port, to_port } => {
            let msg = format!(
                "[config] external-controller port auto-refreshed: {} -> {}",
                from_port
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "?".into()),
                to_port
            );
            handle.push_log(msg.clone());
            let _ = app.emit(crate::events::KERNEL_LOG, msg);
            let _ = ConfigRefreshPayload::PortChanged { from_port, to_port }.emit(app);
        }
        ConfigRefreshPayload::SchemaBumped {
            from_version,
            to_version,
            from_port,
            to_port,
        } => {
            let msg = format!(
                "[config] schema bumped v{} -> v{} (added TUN loopback bypass rules, etc.) — please restart the kernel",
                from_version, to_version
            );
            handle.push_log(msg.clone());
            let _ = app.emit(crate::events::KERNEL_LOG, msg);
            let _ = ConfigRefreshPayload::SchemaBumped {
                from_version,
                to_version,
                from_port,
                to_port,
            }
            .emit(app);
        }
        ConfigRefreshPayload::Unchanged => {}
    }

    // 2) Spawn sidecar.
    //
    // HARD REFUSAL, immediately before the OS spawn. The top-of-function
    // guard already covers this, but checking again here means no future
    // refactor (or a TUN enable racing on another thread during config
    // materialisation) can ever reach `cmd.spawn()` while the elevated TUN
    // kernel owns 9091/7897. This is the last line of defence.
    if crate::core::tun::owns_ports(app) {
        eprintln!(
            "[sidecar-core] HARD REFUSAL: blocking mihomo spawn because TUN owns kernel!"
        );
        let _ = app.emit(
            crate::events::KERNEL_LOG,
            "[sidecar-core] HARD REFUSAL: spawn blocked — TUN owns kernel (9091/7897)",
        );
        handle.set_state(KernelState::Stopped);
        return Ok(());
    }
    let shell = app.shell();
    let cmd = shell.sidecar(SIDECAR_NAME)?.args([
        "-d",
        work_dir.to_string_lossy().as_ref(),
        "-f",
        config_path.to_string_lossy().as_ref(),
    ]);
    let (mut rx, child) = cmd.spawn()?;

    // Begin a fresh launch: clear any lingering intentional-stop flag and bump
    // the generation so a stale drain task (from a superseded start) can never
    // clobber this one's state.
    let my_generation = handle.begin_launch();

    // Bind the child into the kill-on-close job BEFORE we publish it as
    // running. If the UI saw `Running` first and the main process died in
    // between, mihomo would be left orphaned — which is the exact failure
    // this is here to prevent. Best-effort by design: a machine where the
    // job cannot be armed still gets a working kernel, just without the
    // nuclear-option guarantee.
    match crate::core::job_object::assign_child(child.pid()) {
        Ok(true) => {}
        Ok(false) => eprintln!(
            "[sidecar] mihomo pid={} not job-guarded (elevated or unavailable); \
             relies on graceful shutdown only",
            child.pid()
        ),
        Err(e) => eprintln!("[sidecar] WARNING: job-object guard failed: {e}"),
    }

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
                    // A stale drain task — superseded by a newer `start()` —
                    // must not touch state or notify the supervisor. Its child
                    // is gone and a newer one owns the handle now.
                    if handle_for_task.generation() != my_generation {
                        break;
                    }
                    handle_for_task.set_child(None);
                    let intentional = handle_for_task.take_stop_requested();
                    // Visible in dev-terminal + kernel log so a crash's real
                    // exit code / signal is never hidden by the watcher.
                    eprintln!("[sidecar] mihomo exited code={code:?} signal={signal:?}");
                    handle_for_task.push_log(format!(
                        "[sidecar] mihomo exited (code={code:?}, signal={signal:?})"
                    ));
                    let clean = intentional || code == Some(0);
                    let new_state = if clean {
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
                    // Hand the exit to the supervisor, which owns the restart
                    // decision (backoff + circuit breaker). This task ends here.
                    handle_for_task.notify_exit(if clean {
                        ExitEvent::Clean
                    } else {
                        ExitEvent::Crash { code, signal }
                    });
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
    match cur {
        KernelState::Stopped | KernelState::Crashed => return Ok(()),
        KernelState::Recovering => {
            // The supervisor is mid-backoff (no live child to kill). Park in
            // `Stopped`; the supervisor's post-sleep check sees it and stands
            // down instead of relaunching.
            handle.set_state(KernelState::Stopped);
            return Ok(());
        }
        _ => {}
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
        use std::os::windows::process::CommandExt;
        // /IM wildcard matches mihomo.exe AND mihomo-x86_64-pc-windows-msvc.exe
        let _ = std::process::Command::new("taskkill")
            .creation_flags(0x0800_0000)
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
fn ensure_default_config(work_dir: &Path) -> Result<(PathBuf, ConfigRefreshPayload)> {
    let path = work_dir.join(DEFAULT_CONFIG_FILE);
    let bundled = include_str!("../../resources/default_mihomo.yaml");
    let expected = format!("127.0.0.1:{EXPECTED_CONTROLLER_PORT}");

    if !path.exists() {
        std::fs::write(&path, bundled)?;
        return Ok((path, ConfigRefreshPayload::Created));
    }

    let existing = std::fs::read_to_string(&path)?;

    // PROTECTION: a config WITHOUT our version marker is a user-activated
    // subscription (activate_profile fs-copies the profile yaml verbatim)
    // or a hand-edited file. NEVER clobber it back to the bundled default
    // — that is what made imported nodes vanish after a restart.
    if extract_config_version(&existing).is_none() {
        // Still repair one class of breakage, though: configs written by
        // older builds (or activated straight from a subscription) can carry
        // a foreign inbound port — typically `mixed-port: 7890`. That is the
        // clash-family default, so as soon as another client is running the
        // port is taken and mihomo dies on startup, which reads to the user
        // as "the kernel keeps crashing".
        //
        // `patch_and_sanitize_yaml` only rewrites FlexClash-owned top-level
        // keys and leaves `proxies` / `proxy-groups` / `rules` alone, so no
        // node can be lost here. It does re-serialise the document (and thus
        // drops comments), which is why we only reach for it when the port
        // is actually wrong.
        if has_foreign_inbound_port(&existing) {
            eprintln!(
                "[sidecar] active config uses a non-reserved inbound port; \
                 re-sanitising to mixed-port {RESERVED_MIXED_PORT}"
            );
            if let Ok(sanitized) = patch_and_sanitize_yaml(&existing) {
                let _ = std::fs::write(&path, &sanitized);
            }
        }
        return Ok((path, ConfigRefreshPayload::Unchanged));
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
                ConfigRefreshPayload::SchemaBumped {
                    from_version: user_v,
                    to_version: bundled_v,
                    from_port,
                    to_port: EXPECTED_CONTROLLER_PORT,
                },
            ));
        }
    }

    if existing.contains(&expected) {
        return Ok((path, ConfigRefreshPayload::Unchanged));
    }

    let from_port = extract_controller_port(&existing);
    std::fs::write(&path, bundled)?;
    Ok((
        path,
        ConfigRefreshPayload::PortChanged {
            from_port,
            to_port: EXPECTED_CONTROLLER_PORT,
        },
    ))
}

/// True when `yaml` declares an inbound port that FlexClash does not own:
/// a legacy `port:` / `socks-port:` pair, a `mixed-port` set to something
/// other than `RESERVED_MIXED_PORT` (subscriptions commonly ship 7890), or
/// no inbound port at all.
///
/// Unparseable YAML reports `false` on purpose — `ensure_default_config` must
/// never be the thing that destroys a config it cannot understand.
fn has_foreign_inbound_port(yaml: &str) -> bool {
    let Ok(root) = serde_yaml::from_str::<serde_yaml::Value>(yaml) else {
        return false;
    };
    let Some(m) = root.as_mapping() else {
        return false;
    };
    let key = |k: &str| serde_yaml::Value::String(k.to_string());
    if m.contains_key(key("port")) || m.contains_key(key("socks-port")) {
        return true;
    }
    match m.get(key("mixed-port")).and_then(|v| v.as_u64()) {
        Some(p) => p != u64::from(RESERVED_MIXED_PORT),
        None => true,
    }
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

#[cfg(test)]
mod config_port_tests {
    use super::*;

    #[test]
    fn detects_subscription_default_7890() {
        assert!(has_foreign_inbound_port("mixed-port: 7890\n"));
    }

    #[test]
    fn accepts_the_reserved_port() {
        assert!(!has_foreign_inbound_port("mixed-port: 7897\n"));
    }

    #[test]
    fn detects_legacy_port_pair() {
        assert!(has_foreign_inbound_port(
            "port: 7890\nsocks-port: 7891\nmixed-port: 7897\n"
        ));
    }

    #[test]
    fn flags_missing_inbound_port() {
        assert!(has_foreign_inbound_port("mode: rule\n"));
    }

    #[test]
    fn unparseable_yaml_is_left_alone() {
        // must never be the thing that destroys a config it cannot read
        assert!(!has_foreign_inbound_port("this: [is: not: yaml"));
    }
}

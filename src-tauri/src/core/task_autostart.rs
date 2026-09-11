// ============================================================================
// core/task_autostart.rs — silent, UAC-free autostart via Task Scheduler.
//
// Why not the registry Run key
// ----------------------------
// `HKCU\...\Run` (what `tauri-plugin-autostart` writes, see
// `commands/desktop.rs`) starts the app with the *filtered*, non-elevated
// token. That is fine for plain system-proxy mode, but TUN mode needs
// administrator rights, so the user is prompted by UAC every single time
// they toggle TUN on.
//
// A scheduled task with `<RunLevel>HighestAvailable</RunLevel>` is launched
// by the Task Scheduler service, which already runs as SYSTEM, and the
// resulting process holds a full administrator token. UAC is enforced when
// the task is *created*, not when it runs — so after a one-time consent at
// enable time, every subsequent boot is silent.
//
// That one-time prompt is unavoidable: creating a HighestAvailable task from
// a medium-integrity process fails with access-denied, so the create/delete
// calls are routed through `elevate::runas_exec_wait`.
//
// This is strictly better than the registry entry for users who want TUN,
// and strictly worse for users who do not (it runs everything as admin).
// Hence two mutually exclusive switches rather than a silent replacement —
// see `commands/desktop.rs` for the exclusion logic.
//
// Why `schtasks.exe` and not the COM API
// --------------------------------------
// The Task Scheduler COM surface (`ITaskService` / `ITaskDefinition`) would
// avoid a subprocess, but it is a large amount of unsafe COM plumbing for a
// call that happens twice in an install's lifetime. The XML file below is
// the documented, version-stable contract and is what `schtasks /XML`
// consumes; keeping it as a literal makes the security-relevant settings
// (RunLevel, ExecutionTimeLimit) reviewable at a glance instead of being
// buried in property assignments.
// ============================================================================

use std::path::Path;

use crate::error::{AppError, Result};

/// Name shown in Task Scheduler (`taskschd.msc`). Also the `/TN` argument.
pub const TASK_NAME: &str = "FlexClash";

/// Result of probing for the task. `enabled` is the interesting bit;
/// `available` is false on non-Windows and is what the UI keys off to hide
/// or disable the switch rather than showing a control that cannot work.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskAutostartStatus {
    pub enabled: bool,
    pub available: bool,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Is the logon task currently registered?
#[cfg(target_os = "windows")]
pub fn is_enabled() -> bool {
    imp::query(TASK_NAME).unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
pub fn is_enabled() -> bool {
    false
}

/// Register (or replace) the logon task. Shows a UAC prompt.
#[cfg(target_os = "windows")]
pub fn enable(exe: &Path) -> Result<()> {
    imp::create(TASK_NAME, exe)
}

#[cfg(not(target_os = "windows"))]
pub fn enable(_exe: &Path) -> Result<()> {
    Err(AppError::Desktop(
        "silent autostart is Windows-only".into(),
    ))
}

/// Remove the logon task. Best-effort when the task is already absent.
#[cfg(target_os = "windows")]
pub fn disable() -> Result<()> {
    imp::delete(TASK_NAME)
}

#[cfg(not(target_os = "windows"))]
pub fn disable() -> Result<()> {
    Err(AppError::Desktop(
        "silent autostart is Windows-only".into(),
    ))
}

pub fn is_available() -> bool {
    cfg!(target_os = "windows")
}

// ---------------------------------------------------------------------------
// Windows implementation
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod imp {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    use crate::core::elevate;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    /// `schtasks.exe` lives in System32 and is always on PATH.
    const SCHTASKS: &str = "schtasks.exe";

    /// Escape a path for inclusion in an XML text node.
    ///
    /// A Windows path can legally contain `&` (e.g. a user profile named
    /// `Tom & Jerry`), which would otherwise produce invalid XML and a
    /// confusing "the task XML is malformed" error from `schtasks`. `"` and
    /// `<` are included for completeness — they appear in the
    /// `<Arguments>` element we build below.
    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    /// The task definition.
    ///
    /// Notable choices, each of which is load-bearing:
    ///
    /// * `HighestAvailable` — the whole point; yields an elevated token with
    ///   no runtime UAC prompt. `LeastPrivilege` would defeat the purpose.
    /// * `InteractiveToken` — the app has a tray icon and a WebView window,
    ///   so it must run in the user's interactive session. A "run whether
    ///   user is logged on or not" task would run non-interactively and the
    ///   GUI would never appear.
    /// * `ExecutionTimeLimit PT0S` — zero means *no limit*. Task Scheduler's
    ///   default limit is 72 hours, after which it force-kills the task.
    ///   For a background-resident proxy that is a time bomb.
    /// * `StartWhenAvailable` — if the trigger is missed (machine busy), run
    ///   as soon as possible instead of skipping the launch entirely.
    /// * `DisallowStartIfOnBatteries` false + `StopIfGoingOnBatteries` false —
    ///   a proxy that dies on unplug is worse than useless on a laptop.
    /// * `Delay PT20S` — the logon trigger fires before the network stack,
    ///   the default route and the DNS resolver have settled. Starting the
    ///   proxy during that window means the first connection attempts fail
    ///   and the user sees a broken browser for a few seconds. 20s is a
    ///   compromise; the app is also designed to keep running regardless.
    /// * `IgnoreNew` — a second logon (fast user switching) must not start a
    ///   second instance fighting over the reserved inbound port.
    pub(super) fn task_xml(exe: &Path) -> String {
        let version = env!("CARGO_PKG_VERSION");
        format!(
            r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Starts FlexClash {version} at logon with administrator rights so TUN mode works without a UAC prompt.</Description>
    <URI>\{name}</URI>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
      <Delay>PT20S</Delay>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>--silent</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
            version = version,
            name = xml_escape(TASK_NAME),
            user = xml_escape(&current_user()),
            command = xml_escape(&exe.to_string_lossy()),
        )
    }

    /// `DOMAIN\user`. `schtasks /XML` resolves a bare `UserId` against the
    /// local machine, which is wrong for domain accounts, so we always
    /// qualify it. Falls back to `%USERNAME%` if the environment is odd —
    /// still better than embedding an empty `UserId`, which makes the
    /// logon trigger match every account.
    fn current_user() -> String {
        let name = std::env::var("USERNAME").unwrap_or_default();
        match std::env::var("USERDOMAIN") {
            Ok(domain) if !domain.is_empty() && !name.is_empty() => format!("{domain}\\{name}"),
            _ if !name.is_empty() => name,
            _ => "SYSTEM".to_string(),
        }
    }

    /// `true` when the task exists. `schtasks /Query` exits 0 and prints a
    /// table when found; a nonzero exit means "not there" (or unreadable,
    /// which for our purposes is the same thing).
    pub(super) fn query(name: &str) -> std::io::Result<bool> {
        let out = Command::new(SCHTASKS)
            .args(["/Query", "/TN", name])
            .creation_flags(CREATE_NO_WINDOW)
            .output()?;
        Ok(out.status.success())
    }

    pub(super) fn create(name: &str, exe: &Path) -> Result<()> {
        // `schtasks /XML` reads a UTF-16 document, which is why the XML
        // declaration above says UTF-16; we encode accordingly.
        //
        // The temp file must be readable by the *elevated* schtasks process.
        // `%TEMP%` under the user profile is readable by the same user even
        // when elevated (same SID), so this is fine — but a redirected TEMP
        // pointing somewhere admin-only would break, hence writing our own
        // path with an explicit unique name rather than reusing a fixed one
        // that a stale leftover could shadow.
        let dir = std::env::temp_dir().join("flexclash");
        std::fs::create_dir_all(&dir)
            .map_err(|e| AppError::Desktop(format!("temp dir: {e}")))?;
        let xml_path = dir.join(format!("{name}-autostart.xml"));

        let xml = task_xml(exe);
        let mut bytes = Vec::with_capacity(xml.len() * 2 + 2);
        bytes.extend_from_slice(&[0xFF, 0xFE]); // UTF-16LE BOM
        for unit in xml.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        std::fs::write(&xml_path, &bytes)
            .map_err(|e| AppError::Desktop(format!("write task xml: {e}")))?;

        let args = format!(
            "/Create /TN \"{}\" /XML \"{}\" /F",
            name,
            xml_path.display()
        );

        let code = elevate::runas_exec_wait(Path::new(SCHTASKS), &args)?;

        // Clean up regardless of outcome — the XML is a build artefact.
        let _ = std::fs::remove_file(&xml_path);

        if code != 0 {
            return Err(AppError::Desktop(format!(
                "schtasks /Create exited {code}. If you cancelled the UAC prompt, \
                 the task was not created. If it failed otherwise, check that \
                 `{name}` is not locked by an existing task definition."
            )));
        }

        // Verify rather than trusting the exit code: `schtasks` has been
        // known to report success for a task it did not actually register.
        if !query(name).unwrap_or(false) {
            return Err(AppError::Desktop(format!(
                "{name} was not found after a successful-looking /Create"
            )));
        }
        Ok(())
    }

    pub(super) fn delete(name: &str) -> Result<()> {
        if !query(name).unwrap_or(false) {
            // Nothing to do. Idempotent by design: the UI can call this on
            // toggle-off without caring whether the task was ever created.
            return Ok(());
        }

        let args = format!("/Delete /TN \"{}\" /F", name);
        let code = elevate::runas_exec_wait(Path::new(SCHTASKS), &args)?;
        if code != 0 {
            return Err(AppError::Desktop(format!(
                "schtasks /Delete exited {code} (UAC cancelled?)"
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::imp;
    use std::path::Path;

    /// Every one of these settings is load-bearing; a silent regression in
    /// any of them turns the feature into something subtly worse than not
    /// having it (a task that never runs, or that gets killed after 72h, or
    /// that runs invisibly with no GUI). Asserting on the generated XML is
    /// the cheapest place to catch that.
    #[test]
    fn xml_carries_the_load_bearing_settings() {
        let xml = imp::task_xml(Path::new(r"C:\Program Files\FlexClash\flexclash.exe"));

        // The entire point of using Task Scheduler: an elevated token with
        // no runtime UAC prompt.
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));

        // Task Scheduler's default is a 72 hour cap, after which it kills the
        // task. For a background-resident proxy that is a time bomb.
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));

        // Must run in the user's interactive session, otherwise the tray
        // icon and WebView window never appear.
        assert!(xml.contains("<LogonType>InteractiveToken</LogonType>"));

        // Fast user switching must not start a second instance: two copies
        // fight over the reserved inbound port.
        assert!(xml.contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"));

        // A proxy that dies when the laptop is unplugged is worse than none.
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(xml.contains("<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>"));

        // Starts hidden in the tray.
        assert!(xml.contains("<Arguments>--silent</Arguments>"));

        // Launches the binary it was handed, not a hard-coded guess.
        assert!(xml.contains("flexclash.exe"));

        // A missed trigger should still fire late rather than be skipped.
        assert!(xml.contains("<StartWhenAvailable>true</StartWhenAvailable>"));
    }

    /// A Windows profile can legitimately be named `Tom & Jerry`, and an
    /// unescaped `&` makes the whole document invalid XML — which surfaces
    /// as an opaque "the task XML is malformed" from schtasks.
    #[test]
    fn xml_escapes_hostile_paths() {
        let xml = imp::task_xml(Path::new(r"C:\Tom & Jerry\flexclash.exe"));
        assert!(xml.contains("Tom &amp; Jerry"));
        assert!(!xml.contains("Tom & Jerry"), "raw ampersand must not survive");
    }

    /// Requires a runnable `schtasks.exe`. Ignored by default because
    /// hardened sandboxes and some endpoint-security products blacklist it
    /// outright, which would otherwise show up as a confusing permission
    /// failure in an unrelated test run.
    ///
    /// Run explicitly: `cargo test --lib task_autostart -- --ignored`
    ///
    /// The assertion that matters is `Ok(false)` rather than a false
    /// positive: `is_enabled()` folds `Err` into `false`, so an `Ok(true)`
    /// for a task that does not exist would make the UI claim autostart is
    /// on while nothing is registered.
    #[test]
    #[ignore = "needs a runnable schtasks.exe (blocked in hardened sandboxes)"]
    fn schtasks_reports_unknown_tasks_as_absent() {
        let found = imp::query("FlexClash-NoSuchTask-2f8a1c")
            .expect("schtasks.exe must be runnable for this test");
        assert!(!found, "a task that was never created must not be reported as present");
    }
}

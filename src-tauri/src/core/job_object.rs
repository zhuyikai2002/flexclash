// ============================================================================
// core/job_object.rs — Kernel-enforced child-process lifetime.
//
// Problem this solves
// -------------------
// FlexClash leaves behind a zombie `mihomo.exe` when the main process dies
// *without* running its shutdown path — the user hits "End task" in Task
// Manager (a hard `TerminateProcess`), the process panics, or the machine
// loses power. A runaway mihomo keeps holding the reserved inbound port
// (7897) and its TUN adapter, so the next launch fails to bind and the
// user's networking stays broken until they hunt the process down by hand.
//
// User-space cleanup cannot fix this: `shutdown.rs`, `hard_cleanup()` and
// every `Drop` impl are all skipped when the process is terminated. The
// only thing that still runs is the kernel.
//
// The mechanism
// -------------
// A Windows *job object* with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` armed.
// While the job's last handle is open the child runs normally; the moment
// that handle closes — including when the OS tears the process down on a
// hard kill — the kernel walks the job and terminates every member. This
// is atomic, needs no cooperation from the dying process, and costs no
// polling thread.
//
// Why only the sidecar path is guarded
// ------------------------------------
// TUN mode launches mihomo via `ShellExecuteExW` + `runas`, so that child
// runs at HIGH integrity while we run at MEDIUM. Windows forbids a
// medium-integrity process from assigning a high-integrity process to its
// job (the reverse is allowed — that is how the classic
// `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS` privilege-drop trick works), so
// `AssignProcessToJobObject` fails with ERROR_ACCESS_DENIED. There is no
// API that can adopt an already-running elevated process from below.
//
// This is not a regression: the TUN child is *already* covered for the
// ordinary exits by `elevate::stop_elevated_mihomo()`, which the TUN
// disable path and the shutdown hook both call. The elevated case needs a
// small elevated launcher that owns its own job — a separate change, not
// something this module can fake. See `assign_child()` for the explicit
// elevated-process carve-out that keeps TUN from spamming the log.
//
// Handle ownership
// ----------------
// The job handle lives in a process-global `OnceLock`. It is deliberately
// never closed by us: the kernel closes it during process teardown, which
// is exactly the event that triggers the kill. Dropping it early would
// kill a perfectly healthy mihomo, so there is no `Drop` impl here.
// ============================================================================

/// Arm a process-wide job object and adopt the child in it.
///
/// Returns `Ok(true)` when the child is now job-guarded, `Ok(false)` when
/// the child is legitimately out of reach (an elevated TUN-mode process),
/// and `Err` only for a genuine setup failure worth logging.
#[cfg(target_os = "windows")]
pub fn assign_child(pid: u32) -> std::io::Result<bool> {
    imp::assign_child(pid)
}

#[cfg(not(target_os = "windows"))]
pub fn assign_child(_pid: u32) -> std::io::Result<bool> {
    // POSIX has no equivalent kernel construct for this. `prctl(PR_SET_PDEATHSIG)`
    // is close on Linux but only fires on the *direct parent thread* dying,
    // not on a hard kill of the whole process group, and BSD/macOS have
    // nothing comparable at all. The existing `hard_cleanup()` pkill path
    // remains the Unix story.
    Ok(false)
}

/// Test-only visibility into whether the job was created successfully.
#[cfg(target_os = "windows")]
pub fn is_armed() -> bool {
    imp::is_armed()
}

#[cfg(not(target_os = "windows"))]
pub fn is_armed() -> bool {
    false
}

// ---------------------------------------------------------------------------
// Windows implementation
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod imp {
    use std::io;
    use std::sync::OnceLock;

    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
        JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    /// The one job every sidecar child is adopted into. `OnceLock` because
    /// the handle must be created lazily (first spawn) and then live for
    /// the rest of the process — see the module docs on why we never close it.
    static JOB: OnceLock<Option<JobHandle>> = OnceLock::new();

    /// Raw handle holder. `Send`/`Sync` are sound for a job object handle:
    /// it is just a kernel object reference, and every use below is either
    /// the `SetInformationJobObject` call made once at creation or a
    /// `AssignProcessToJobObject` that the kernel serialises internally.
    struct JobHandle(HANDLE);

    // SAFETY: a job object HANDLE is an opaque, process-wide kernel object
    // reference with no thread affinity. Sharing it across threads is what
    // it is designed for; all the APIs used here are thread-safe.
    unsafe impl Send for JobHandle {}
    unsafe impl Sync for JobHandle {}

    /// Create the job on first use, then reuse it forever.
    ///
    /// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` is the single flag that makes
    /// the whole design work, so a failure to set it is fatal for this
    /// module (we return `Err` and the caller logs + carries on unguarded
    /// rather than pretending the child is protected).
    fn job() -> io::Result<Option<HANDLE>> {
        let slot = JOB.get_or_init(|| match create_job() {
            Ok(h) => {
                eprintln!(
                    "[job] kill-on-close job object armed; \
                     sidecar children will not outlive this process"
                );
                Some(JobHandle(h))
            }
            Err(e) => {
                eprintln!("[job] WARNING: could not create job object: {e}");
                None
            }
        });
        Ok(slot.as_ref().map(|j| j.0))
    }

    fn create_job() -> io::Result<HANDLE> {
        // SAFETY: null attrs (not inheritable) and a null name (unnamed,
        // private to this process) are both documented as valid.
        let job = unsafe { CreateJobObjectW(None, None) }
            .map_err(|e| io::Error::other(format!("CreateJobObjectW: {e}")))?;

        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        // SAFETY: `info` is a correctly-typed, fully-initialised struct and
        // we pass its exact size, which is what the API contract requires
        // for `JobObjectExtendedLimitInformation`.
        let res = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if let Err(e) = res {
            // Do not leak the handle if we are about to bail.
            unsafe { let _ = CloseHandle(job); }
            return Err(io::Error::other(format!("SetInformationJobObject: {e}")));
        }
        Ok(job)
    }

    /// Right to assign a process to a job, plus the rights the kernel needs
    /// to later terminate it. `PROCESS_SET_QUOTA | PROCESS_TERMINATE` is the
    /// documented minimum for `AssignProcessToJobObject`;
    /// `PROCESS_QUERY_LIMITED_INFORMATION` lets us tell "elevated process"
    /// apart from "unexpected failure".
    const ACCESS: windows::Win32::System::Threading::PROCESS_ACCESS_RIGHTS =
        windows::Win32::System::Threading::PROCESS_ACCESS_RIGHTS(
            PROCESS_SET_QUOTA.0 | PROCESS_TERMINATE.0 | PROCESS_QUERY_LIMITED_INFORMATION.0,
        );

    pub(super) fn assign_child(pid: u32) -> io::Result<bool> {
        if pid == 0 {
            // `elevate::runas_spawn` returns 0 when it cannot resolve the
            // PID it just launched. Nothing to adopt; not an error.
            return Ok(false);
        }

        let Some(job) = job()? else {
            return Ok(false);
        };

        // SAFETY: OpenProcess with a valid access mask and no inheritance.
        let process = unsafe { OpenProcess(ACCESS, false, pid) };
        let process = match process {
            Ok(h) if !h.is_invalid() => h,
            Ok(_) | Err(_) => {
                // Opening failed outright — most often because the process
                // is elevated and we are not, so even
                // PROCESS_QUERY_LIMITED_INFORMATION is denied. Treat as
                // "out of reach" rather than an error: the TUN path relies
                // on `elevate::stop_elevated_mihomo` instead, and logging a
                // warning on every TUN enable would be pure noise.
                return Ok(false);
            }
        };

        // SAFETY: both handles are valid and owned for the duration of the
        // call. Assigning an already-assigned process is a no-op success,
        // which makes this idempotent across restarts.
        let res = unsafe { AssignProcessToJobObject(job, process) };
        // SAFETY: we opened `process` ourselves and never hand ownership
        // anywhere else, so closing it here is required to avoid a leak.
        unsafe { let _ = CloseHandle(process); }

        match res {
            Ok(()) => Ok(true),
            Err(e) => {
                // ERROR_ACCESS_DENIED (5) is the ordinary elevated-child
                // case. Anything else is worth surfacing.
                let code = e.code().0 as u32;
                if code != 5 {
                    eprintln!(
                        "[job] WARNING: AssignProcessToJobObject(pid={pid}) failed: {e}"
                    );
                }
                Ok(false)
            }
        }
    }

    pub(super) fn is_armed() -> bool {
        // Only meaningful once something has tried to spawn; does not
        // create the job as a side effect.
        JOB.get().map(|s| s.is_some()).unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    /// Arming must succeed on a normal desktop session. If this fails the
    /// whole guarantee is gone, so it is worth asserting rather than
    /// silently logging.
    #[test]
    fn job_arms_on_windows() {
        assert!(assign_child(0).is_ok(), "assign_child must not error for pid 0");
        // pid 0 is the "nothing to adopt" case; it must not arm anything by
        // itself, but calling with a real (self) pid below does.
        let me = std::process::id();
        let adopted = assign_child(me).expect("assign_child(self) must not error");
        // Assigning our own process to the job is allowed and is the
        // cheapest end-to-end proof that the job exists and is usable.
        assert!(adopted, "should be able to adopt our own (non-elevated) process");
        assert!(is_armed(), "job must report armed after a successful assign");
    }

    /// A PID that cannot exist must be reported as "not adopted" rather
    /// than blowing up — this is the path taken when the elevated child is
    /// out of reach mid-restart.
    #[test]
    fn dead_pid_is_not_an_error() {
        let res = assign_child(u32::MAX);
        assert!(res.is_ok(), "dead pid must return Ok(false), not Err");
        assert!(!res.unwrap(), "dead pid cannot be adopted");
    }
}

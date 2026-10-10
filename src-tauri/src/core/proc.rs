// ============================================================================
// core/proc.rs — bounded external-command execution.
//
// Why this module exists
// ----------------------
// `std::process::Command::output()` waits for the child **unconditionally**.
// Every helper the TUN lifecycle shells out to — `taskkill`, `route`,
// `netsh`, `kill`, `pkill` — could therefore wedge the caller for as long as
// the child felt like, and it did so while the caller was holding a lock.
// The concrete incident: `netsh interface delete interface` blocking on an
// adapter still owned by a live mihomo, which froze the TUN state machine
// and then starved the async runtime until every IPC call — and with it the
// whole UI — stopped answering.
//
// Everything that shells out now goes through [`run_with_timeout`], so a
// wedged helper degrades into a *reported failure* instead of a hang.
//
// Zero-panic by construction: no `unwrap`/`expect`/indexing, matching the
// rest of `core/` — under `panic = "abort"` a panic here would take the whole
// process down.
// ============================================================================

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Hide the child's console window on Windows. Without this every helper
/// flashed a console during a TUN toggle.
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How often we poll the child while it is within its budget.
const POLL: Duration = Duration::from_millis(20);

/// How long we wait for the pipe readers to finish after the child is gone.
///
/// A grandchild can hold the inherited pipe open long after the child we
/// spawned has been reaped — `cmd /C ping` is the canonical example — so
/// waiting for EOF unconditionally would put us straight back into the hang
/// this module exists to prevent. After this grace we snapshot whatever has
/// arrived and let the reader finish on its own.
const READER_SETTLE: Duration = Duration::from_millis(200);

/// What a bounded run produced.
#[derive(Debug, Default)]
pub struct Output {
    /// Exited with status 0 **and** finished inside its budget. A timed-out
    /// run is never `success`, whatever exit code the kill produced.
    pub success: bool,
    /// Raw exit status, when the child was allowed to report one. `taskkill`
    /// needs this: it returns 128 for "no running task matches that name",
    /// which is a successful no-op rather than a failure.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    /// The child outlived its budget and had to be killed.
    pub timed_out: bool,
}

/// Run `program` with `args`, kill it if it outlives `timeout`, and return
/// what it printed.
///
/// Both pipes are drained on their own threads: a helper that fills the OS
/// pipe buffer would otherwise block *before* we ever reach the exit check,
/// turning a chatty child into exactly the hang this module exists to
/// prevent.
///
/// **The bound is real but not absolute.** `kill` only reaches the process we
/// spawned; a grandchild that inherited the pipes can keep them open past the
/// deadline. We therefore never join the readers unconditionally — we wait a
/// short grace period, snapshot what has arrived and return. Worst case for
/// any call is `timeout + 2 × READER_SETTLE`.
pub fn run_with_timeout(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> std::io::Result<Output> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn()?;
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let out_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let err_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let (out_done_tx, out_done_rx) = mpsc::channel::<()>();
    let (err_done_tx, err_done_rx) = mpsc::channel::<()>();

    let out_sink = Arc::clone(&out_buf);
    let out_reader = std::thread::spawn(move || {
        drain_into(stdout_pipe, out_sink);
        let _ = out_done_tx.send(());
    });
    let err_sink = Arc::clone(&err_buf);
    let err_reader = std::thread::spawn(move || {
        drain_into(stderr_pipe, err_sink);
        let _ = err_done_tx.send(());
    });

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    let _ = child.kill();
                    // `kill` only signals. `wait` is what closes the handle.
                    break child.wait()?;
                }
                std::thread::sleep(POLL);
            }
        }
    };

    // Bounded settle, never an unbounded join — see `READER_SETTLE`.
    let _ = out_done_rx.recv_timeout(READER_SETTLE);
    let _ = err_done_rx.recv_timeout(READER_SETTLE);
    drop(out_reader);
    drop(err_reader);

    let code = status.code();
    let stdout = snapshot(&out_buf);
    let stderr = snapshot(&err_buf);

    Ok(Output {
        success: status.success() && !timed_out,
        code,
        stdout,
        stderr,
        timed_out,
    })
}

/// Copy a pipe into `sink` until EOF. Never panics and never blocks the
/// caller: the sink is a mutex-guarded `Vec`, and a read error just ends
/// the copy.
fn drain_into(pipe: Option<impl Read>, sink: Arc<Mutex<Vec<u8>>>) {
    let Some(mut p) = pipe else { return };
    let mut chunk = [0u8; 8192];
    loop {
        match p.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if let Ok(mut g) = sink.lock() {
                    g.extend_from_slice(&chunk[..n]);
                }
            }
        }
    }
}

fn snapshot(buf: &Mutex<Vec<u8>>) -> String {
    match buf.lock() {
        Ok(g) => String::from_utf8_lossy(&g).into_owned(),
        // A poisoned sink means some other reader panicked; whatever it had
        // written so far is still better than nothing.
        Err(_) => String::new(),
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    fn reports_a_fast_command_as_success() {
        let out = run_with_timeout("cmd", &["/C", "exit", "0"], Duration::from_secs(10))
            .expect("spawn cmd");
        assert!(out.success, "stderr={}", out.stderr);
        assert!(!out.timed_out);
        assert_eq!(out.code, Some(0));
    }

    #[test]
    fn captures_stdout() {
        let out = run_with_timeout("cmd", &["/C", "echo", "flexclash"], Duration::from_secs(10))
            .expect("spawn cmd");
        assert!(out.stdout.contains("flexclash"), "stdout={}", out.stdout);
    }

    #[test]
    fn kills_a_command_that_outlives_its_budget() {
        // `cmd /C ping` is the interesting case: the *grandchild* inherits
        // the pipes, so a naive `kill` + `join` would still block until the
        // grandchild exits (~4s) and the budget would be a lie.
        let started = Instant::now();
        let out = run_with_timeout(
            "cmd",
            &["/C", "ping -n 5 127.0.0.1 > nul"],
            Duration::from_millis(250),
        )
        .expect("spawn cmd");
        assert!(out.timed_out, "expected a timeout, got {out:?}");
        assert!(!out.success, "a timed-out run is never a success");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "budget not honoured: {:?}",
            started.elapsed()
        );
    }

    /// The regression this module exists for: a helper that fills its stdout
    /// pipe must not deadlock us, because both pipes are drained concurrently.
    #[test]
    fn a_chatty_child_does_not_deadlock_the_reader() {
        let out = run_with_timeout(
            "cmd",
            &[
                "/C",
                "for /L %i in (1,1,20000) do @echo flexclash-noisy-output",
            ],
            Duration::from_secs(20),
        )
        .expect("spawn cmd");
        assert!(
            out.stdout.len() > 64 * 1024,
            "expected >64KiB of output, got {} bytes",
            out.stdout.len()
        );
    }
}

#[cfg(all(test, not(target_os = "windows")))]
mod tests {
    use super::*;

    #[test]
    fn reports_a_fast_command_as_success() {
        let out =
            run_with_timeout("sh", &["-c", "exit 0"], Duration::from_secs(10)).expect("spawn");
        assert!(out.success);
        assert!(!out.timed_out);
    }

    #[test]
    fn kills_a_command_that_outlives_its_budget() {
        let started = Instant::now();
        let out =
            run_with_timeout("sh", &["-c", "sleep 5"], Duration::from_millis(250)).expect("spawn");
        assert!(out.timed_out);
        assert!(!out.success);
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}

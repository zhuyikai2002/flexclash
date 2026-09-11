#!/usr/bin/env python3
"""Real-machine acceptance test for the kill-on-close job object (Windows).

Run this after touching `core/job_object.rs` or `core/sidecar.rs`. It proves
the actual guarantee rather than the intent, on the real machine, using the
same hard-kill the user's Task-Manager "End task" performs.

What it does
------------
1. Builds a tiny standalone probe crate (`.job_probe/`) that arms a
   KILL_ON_JOB_CLOSE job the same way `core/job_object.rs` does, spawns a
   long-lived child, and adopts the child into that job.
2. Hard-kills the probe with `taskkill /F`, i.e. `TerminateProcess` -- the
   path that skips every Rust `Drop` impl and the whole shutdown module.
3. Asserts the child died too.

The probe does no cleanup of its own, so if the child dies, only the kernel
can have done it.

    python scripts/verify-job-object.py

Requires a working `cargo` on PATH. The probe lives in `.job_probe/` and is
created on first run.
"""
from __future__ import annotations

import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PROBE_DIR = ROOT / ".job_probe"

CARGO_TOML = """\
[package]
name = "jobprobe"
version = "0.0.0"
edition = "2021"

[[bin]]
name = "jobprobe"
path = "src/main.rs"

[dependencies]
windows = { version = "0.58", features = [
  "Win32_Foundation",
  "Win32_Security",
  "Win32_System_JobObjects",
  "Win32_System_Threading",
] }

[workspace]
"""

# Mirrors core/job_object.rs: create the job, arm KILL_ON_JOB_CLOSE, adopt a
# child, then never clean up. Kept intentionally free of any FlexClash import
# so the test measures Windows behaviour, not our own code path.
MAIN_RS = r"""
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
    JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

fn main() {
    unsafe {
        let job: HANDLE = CreateJobObjectW(None, None).expect("CreateJobObjectW");
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
        .expect("SetInformationJobObject");

        let child = Command::new("cmd.exe")
            .args(["/c", "ping 127.0.0.1 -n 120 > nul"])
            .creation_flags(0x0800_0000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn child");
        let child_pid = child.id();

        let access = windows::Win32::System::Threading::PROCESS_ACCESS_RIGHTS(
            PROCESS_SET_QUOTA.0 | PROCESS_TERMINATE.0,
        );
        let handle = OpenProcess(access, false, child_pid).expect("OpenProcess");
        AssignProcessToJobObject(job, handle).expect("AssignProcessToJobObject");

        println!("child_pid={child_pid} ready");
        std::mem::forget(child);
        std::mem::forget(handle);
        std::mem::forget(job);
    }
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
"""


def running(pid: int) -> bool:
    out = subprocess.run(
        ["tasklist", "/FI", f"PID eq {pid}", "/FO", "CSV", "/NH"],
        capture_output=True, text=True, errors="replace",
    ).stdout
    return str(pid) in out


def ensure_probe() -> Path:
    (PROBE_DIR / "src").mkdir(parents=True, exist_ok=True)
    (PROBE_DIR / "Cargo.toml").write_text(CARGO_TOML, encoding="utf-8")
    (PROBE_DIR / "src" / "main.rs").write_text(MAIN_RS, encoding="utf-8")
    print("building probe ...")
    r = subprocess.run(["cargo", "build", "--release"], cwd=PROBE_DIR,
                       capture_output=True, text=True, errors="replace")
    if r.returncode != 0:
        print(r.stdout)
        print(r.stderr)
        sys.exit("probe build failed")
    return PROBE_DIR / "target" / "release" / "jobprobe.exe"


def main() -> int:
    if sys.platform != "win32":
        sys.exit("this test is Windows-only")

    exe = ensure_probe()
    proc = subprocess.Popen([str(exe)], stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True, errors="replace")
    line = proc.stdout.readline().strip()
    print("harness:", line)
    if "child_pid=" not in line:
        print("unexpected output:", line, proc.stdout.read())
        return 1

    child_pid = int(line.split("child_pid=")[1].split()[0])
    parent_pid = proc.pid
    print(f"parent pid = {parent_pid}   child pid = {child_pid}\n")

    if not running(child_pid):
        subprocess.run(["taskkill", "/F", "/PID", str(parent_pid)], capture_output=True)
        print("RESULT: INCONCLUSIVE - child never started")
        return 1
    print(f"[1] child running before kill      : True")

    print(f"[2] hard-killing parent (taskkill /F /PID {parent_pid}) ...")
    subprocess.run(["taskkill", "/F", "/PID", str(parent_pid)],
                   capture_output=True, text=True, errors="replace")

    elapsed = None
    for i in range(40):
        if not running(child_pid):
            elapsed = i * 0.05
            break
        time.sleep(0.05)

    after = running(child_pid)
    print(f"[3] child running after parent died : {after}\n")

    if after:
        print(f"    -> ZOMBIE LEAKED. cleaning up pid {child_pid}")
        subprocess.run(["taskkill", "/F", "/PID", str(child_pid)], capture_output=True)
        print("RESULT: FAIL - the job object did not reap the child")
        return 1

    print(f"    -> reaped by the kernel in ~{elapsed:.2f}s (we did nothing)")
    print("RESULT: PASS - no zombie after a hard kill of the parent")
    return 0


if __name__ == "__main__":
    sys.exit(main())

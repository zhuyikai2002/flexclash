#!/usr/bin/env python3
"""Build the `latest.json` manifest the Tauri updater polls.

Run this AFTER `npm run tauri build` has completed with
`TAURI_SIGNING_PRIVATE_KEY` set — the build is what produces the `.sig`
file this script reads.

    set TAURI_SIGNING_PRIVATE_KEY=<path-or-contents>
    npm run tauri build
    python scripts/gen-latest-json.py

The manifest is written to
`src-tauri/target/release/bundle/latest.json`. Upload it together with the
installer it points at to the GitHub release.

Note on the payload: Tauri 2.11 signs the NSIS installer itself and emits
`<installer>.exe.sig`. It does NOT produce a separate `.nsis.zip` updater
artifact on this version, so the updater downloads the full installer.

Nothing here needs the private key — it only reads the public `.sig`.
"""
from __future__ import annotations

import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BUNDLE = ROOT / "src-tauri" / "target" / "release" / "bundle"
NSIS_DIR = BUNDLE / "nsis"

REPO = "https://github.com/zhuyikai2002/flexclash"
# Keep in sync with `tauri.conf.json > plugins.updater.endpoints`.
DOWNLOAD_BASE = f"{REPO}/releases/download"

# NOTE: the changelog the in-app updater dialog shows is DERIVED, not
# hand-copied. It used to be a constant here that had to be rewritten by
# hand on every release -- and the script itself warned that a stale value
# ships the previous version's notes to every user and fails silently, with
# no check to catch it. That warning was prophetic: v0.4.0 shipped with the
# v0.2.5 text because the constant was never touched. The summary is now
# extracted from release_notes.md (the same file `release.yml` publishes as
# the release body), so there is exactly ONE place to maintain. See
# `read_updater_notes` for the contract it enforces.
UPDATER_NOTES_MAX = 400


def read_updater_notes() -> str:
    """Derive the short updater summary from `release_notes.md`.

    Contract: every top-level numbered section heading (`## 1. Feat (...): ...`)
    becomes one clause, joined with `；`. Numbered headings only -- the
    un-numbered `## 兼容性说明` / `## 基础设施` boilerplate stays out.

    Fails the run instead of guessing when the file has no such headings or
    the derived summary is too long. A missing/oddly-shaped notes file must
    break the release, not silently publish last version's text.
    """
    md = (ROOT / "release_notes.md").read_text(encoding="utf-8")
    clauses = []
    for line in md.splitlines():
        m = re.match(r"^##\s+\d+\s*[.、)]\s*(.+?)\s*$", line)
        if m:
            clause = m.group(1).strip().rstrip("。")
            if clause:
                clauses.append(clause)
    if not clauses:
        sys.exit(
            "release_notes.md has no '## N. Title' section headings; "
            "cannot derive the updater notes. Add numbered sections or "
            "update the summary by hand and restore a constant."
        )
    notes = "；".join(clauses) + "。"
    if len(notes) > UPDATER_NOTES_MAX:
        sys.exit(
            f"derived updater notes are {len(notes)} chars (max "
            f"{UPDATER_NOTES_MAX}); shorten the section headings in "
            "release_notes.md -- long-form belongs in the body, not the dialog"
        )
    return notes


def read_version() -> str:
    conf = (ROOT / "src-tauri" / "tauri.conf.json").read_text(encoding="utf-8")
    m = re.search(r'"version"\s*:\s*"([^"]+)"', conf)
    if not m:
        sys.exit("could not read version from tauri.conf.json")
    return m.group(1)


def main() -> None:
    version = read_version()

    # Prefer the installer that matches the configured version; fall back to
    # the newest one so a stale build directory still produces something.
    exact = NSIS_DIR / f"FlexClash_{version}_x64-setup.exe"
    candidates = [exact] if exact.is_file() else sorted(NSIS_DIR.glob("*-setup.exe"))
    if not candidates:
        sys.exit(
            f"no NSIS installer in {NSIS_DIR}\n"
            "run `npm run tauri build` with TAURI_SIGNING_PRIVATE_KEY first"
        )
    installer = candidates[-1]

    sig_path = installer.with_suffix(installer.suffix + ".sig")
    if not sig_path.is_file():
        sys.exit(
            f"missing {sig_path}\n"
            "the build did not sign — is TAURI_SIGNING_PRIVATE_KEY set "
            "and does it match the pubkey in tauri.conf.json?"
        )
    signature = sig_path.read_text(encoding="utf-8").strip()

    manifest = {
        "version": version,
        "notes": read_updater_notes(),
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": {
            "windows-x86_64": {
                "signature": signature,
                "url": f"{DOWNLOAD_BASE}/v{version}/{installer.name}",
            }
        },
    }

    out = BUNDLE / "latest.json"
    out.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")

    print(f"wrote {out}")
    print(f"  version : {version}")
    print(f"  payload : {installer.name}")
    print(f"  url     : {manifest['platforms']['windows-x86_64']['url']}")
    print("\nupload to the release: latest.json + the installer above")


if __name__ == "__main__":
    main()

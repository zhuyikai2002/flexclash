#!/usr/bin/env python3
"""Build the `latest.json` manifest the Tauri updater polls.

Run this AFTER `npm run tauri build` has completed with
`TAURI_SIGNING_PRIVATE_KEY` set — the build is what produces the `.sig`
file this script reads.

    set TAURI_SIGNING_PRIVATE_KEY=<path-or-contents>
    npm run tauri build
    python scripts/gen-latest-json.py

The manifest is written to
`src-tauri/target/release/bundle/latest.json`; upload it together with the
`.nsis.zip` installer to the GitHub release.

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

NOTES = (
    "Fix: silent TUN privilege escalation with no console flash; "
    "Fix: kernel health probe no longer spawns curl subprocesses; "
    "Fix: force mixed-port 7897 so imported configs stop colliding with "
    "other Clash-family clients; "
    "Fix: WebView2 sandbox fallback for machines where the renderer "
    "could not start; "
    "Chore: ship updater artifacts so in-app updates work."
)


def read_version() -> str:
    conf = (ROOT / "src-tauri" / "tauri.conf.json").read_text(encoding="utf-8")
    m = re.search(r'"version"\s*:\s*"([^"]+)"', conf)
    if not m:
        sys.exit("could not read version from tauri.conf.json")
    return m.group(1)


def main() -> None:
    version = read_version()

    zips = sorted(NSIS_DIR.glob("*.nsis.zip"))
    if not zips:
        sys.exit(
            f"no .nsis.zip in {NSIS_DIR}\n"
            "run `npm run tauri build` with TAURI_SIGNING_PRIVATE_KEY first"
        )
    installer = zips[-1]

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
        "notes": NOTES,
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
    print("\nupload to the release: latest.json + the .nsis.zip above")


if __name__ == "__main__":
    main()

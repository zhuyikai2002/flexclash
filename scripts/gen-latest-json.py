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

# NOTE: this is the changelog the in-app updater dialog shows. It is NOT
# derived from `release_notes.md` or from `version`, so it MUST be rewritten
# by hand on every release. A stale value ships the previous version's notes
# to every user and fails silently -- there is no check that catches it.
# Keep it to a few short clauses; long-form notes belong in release_notes.md.
NOTES = (
    "Feat (TUN): 「严格路由」与「DNS 劫持」进阶开关正式可用，支持状态持久化与"
    "Mihomo 运行时配置热重载（PUT /configs?force=true）；"
    "Fix (DNS): 修复 DNS 劫持参数仅拦截 UDP 的漏洞，补齐 tcp://any:53，"
    "全面防范 DNS 泄漏；"
    "Fix (UI): 抹除前端组件中的历史硬编码，关于页面与更新模块全面接入 "
    "Tauri 运行时版本动态校验。"
)


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
    print("\nupload to the release: latest.json + the installer above")


if __name__ == "__main__":
    main()

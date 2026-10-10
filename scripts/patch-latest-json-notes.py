#!/usr/bin/env python3
"""Inject the changelog into an already-uploaded `latest.json` release asset.

WHY THIS EXISTS
---------------
`tauri-apps/tauri-action` generates the updater manifest itself and exposes no
input for its `notes` field, so the manifest it uploads always carries an empty
changelog. The Release *body* can be supplied (that is `releaseBody` in
`release.yml`), but the in-app update dialog reads `latest.json > notes` — so
without this script every user who updates sees a blank changelog. v0.6.4
shipped exactly that way.

`gen-latest-json.py` also injects the notes, but it requires locally built and
signed bundles for every platform, which a single CI matrix row does not have.
This script instead patches the artifact the workflow already uploaded, so the
signatures and URLs tauri-action computed are preserved byte-for-byte.

Usage (from the repository root, after `latest.json` has been downloaded):

    python3 scripts/patch-latest-json-notes.py manifest.json release_notes.md

It rewrites `manifest.json` in place. Exits non-zero — loudly — rather than
leaving an empty changelog for users to discover.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        sys.exit(f"usage: {Path(argv[0]).name} <manifest.json> <release_notes.md>")

    manifest_path = Path(argv[1])
    notes_path = Path(argv[2])

    if not manifest_path.is_file():
        sys.exit(f"{manifest_path} not found")
    if not notes_path.is_file():
        sys.exit(f"{notes_path} not found")

    notes = notes_path.read_text(encoding="utf-8").strip()
    if not notes:
        # Publishing here would show every user an empty dialog with no signal
        # that anything went wrong. Fail the release instead.
        sys.exit(f"{notes_path} is empty; refusing to publish a blank changelog")

    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

    if manifest.get("notes"):
        # Idempotent by design: a re-run, or a second matrix row racing this
        # one, must not rewrite an asset that already carries the changelog.
        print(f"notes already present ({len(manifest['notes'])} chars); leaving as-is")
        return 0

    manifest["notes"] = notes
    manifest_path.write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    print(f"patched {len(notes)} chars of notes into {manifest_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))

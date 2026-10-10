#!/usr/bin/env python3
"""Assert that a published `latest.json` actually carries a changelog.

Run against the manifest **downloaded from the release**, not against a local
build, so it verifies the bytes a user updating right now would actually
receive — including whatever the upload round-trip did to them.

Usage:

    python3 scripts/verify-latest-json-notes.py verify.json

Exits non-zero with a `::error::` annotation (which GitHub renders as a red
annotation on the job) so an empty changelog fails the release visibly instead
of reaching users.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

# Shorter than this and it is not a changelog — it is a truncated write, an
# encoding accident, or a placeholder. The real notes run thousands of chars.
MIN_REASONABLE_NOTES = 200


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        sys.exit(f"usage: {Path(argv[0]).name} <latest.json>")

    path = Path(argv[1])
    if not path.is_file():
        sys.exit(f"{path} not found")

    manifest = json.loads(path.read_text(encoding="utf-8"))

    version = manifest.get("version", "<missing>")
    notes = manifest.get("notes") or ""

    if len(notes) < MIN_REASONABLE_NOTES:
        print(
            f"::error::published latest.json for v{version} carries "
            f"{len(notes)} chars of notes (expected >= {MIN_REASONABLE_NOTES})"
        )
        return 1

    platforms = manifest.get("platforms") or {}
    missing_sig = [
        name
        for name, entry in platforms.items()
        if not (entry.get("signature") or "").strip()
    ]
    if missing_sig:
        print(
            f"::error::latest.json for v{version} has unsigned platforms: "
            f"{', '.join(missing_sig)}"
        )
        return 1

    print(
        f"verified: v{version} manifest carries {len(notes)} chars of notes "
        f"across {len(platforms)} signed platform(s)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))

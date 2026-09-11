#!/usr/bin/env python3
"""Replay what the Tauri updater does at runtime, before you publish.

`npm run tauri build` happily finishes even when the updater chain is broken
-- it only emits a `Warn` when the signing key and `plugins.updater.pubkey`
disagree, and that warning is easy to miss in a long build log. This script
checks the real thing instead:

  1. read `latest.json` -> signature + download url
  2. base64-decode the signature into minisign text
  3. pull the Ed25519 public key out of `tauri.conf.json`
  4. Ed25519-verify the signature over the actual installer bytes

If this passes, an installed client will accept the update.

    python scripts/gen-latest-json.py     # produce latest.json first
    python scripts/verify-updater-sig.py

Requires `cryptography` (for Ed25519); everything else is stdlib.

Note on minisign algorithm markers -- this is the part that is easy to get
wrong, and the reason this script exists:

    "Ed"  Ed25519 straight over the file bytes
    "ED"  BLAKE2b-512 prehash, then plain Ed25519 over that 64-byte digest.
          NOT RFC 8032 Ed25519ph: there is no dom2 prefix and SHA-512 is not
          involved. Tauri's signer emits "ED". Verifying "ED" as if it were
          "Ed" fails, which looks exactly like a corrupt download.
"""
from __future__ import annotations

import base64
import hashlib
import json
import pathlib
import sys

try:
    from cryptography.exceptions import InvalidSignature
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
except ImportError:
    sys.exit("missing dependency: pip install cryptography")

ROOT = pathlib.Path(__file__).resolve().parent.parent
BUNDLE = ROOT / "src-tauri" / "target" / "release" / "bundle"
CONF = ROOT / "src-tauri" / "tauri.conf.json"


def ms_lines(text: str) -> list[str]:
    """Non-empty lines of a minisign blob (which may be base64-wrapped)."""
    if text.lstrip().startswith("dW50cnVzdGVk"):
        text = base64.b64decode(text).decode()
    return [l for l in text.splitlines() if l.strip()]


def keyid(raw: bytes) -> str:
    """minisign stores an 8-byte key id at payload[2:10], shown big-endian."""
    return raw[2:10][::-1].hex().upper()


def main() -> int:
    manifest_path = BUNDLE / "latest.json"
    if not manifest_path.is_file():
        sys.exit(f"missing {manifest_path}\nrun scripts/gen-latest-json.py first")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    plat = manifest["platforms"]["windows-x86_64"]

    print("manifest version :", manifest["version"])
    print("manifest url     :", plat["url"])

    installer = BUNDLE / "nsis" / plat["url"].rsplit("/", 1)[-1]
    if not installer.is_file():
        sys.exit(f"missing {installer}\nthe url in latest.json points at a file we do not have")
    print("local installer  :", installer.name, f"({installer.stat().st_size:,} bytes)")

    lines = ms_lines(plat["signature"])
    trusted = next((l for l in lines if l.startswith("trusted comment")), "")
    print("trusted comment  :", trusted)

    payload = base64.b64decode(lines[1])
    alg = payload[0:2].decode()
    signature = payload[10:74]
    print("sig algorithm    :", alg)
    print("sig key id       :", keyid(payload))

    conf = json.loads(CONF.read_text(encoding="utf-8"))
    pub_lines = ms_lines(conf["plugins"]["updater"]["pubkey"])
    pub = base64.b64decode(pub_lines[1])
    print("config pubkey    :", pub_lines[0])
    print("config key id    :", keyid(pub))

    ok = True
    if payload[2:10] != pub[2:10]:
        print("!! key id mismatch -- the client would reject this update")
        ok = False

    data = installer.read_bytes()
    message = hashlib.blake2b(data, digest_size=64).digest() if alg == "ED" else data
    try:
        Ed25519PublicKey.from_public_bytes(pub[10:42]).verify(signature, message)
        print("\nEd25519 verify   : PASS")
    except InvalidSignature:
        print("\nEd25519 verify   : FAIL -- signature does not match the file bytes")
        ok = False

    if f"file:{installer.name}" not in trusted:
        print("!! trusted comment does not name this installer")
        ok = False

    print()
    print("RESULT:", "OK - safe to publish" if ok else "BROKEN - do not publish")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())

# FlexClash — Arch Linux packaging

This directory ships a **prebuilt binary** PKGBUILD for Arch / EndeavourOS
that consumes the Linux `.deb` published by
[`.github/workflows/release.yml`](../../.github/workflows/release.yml) on every
`v*` tag.

## What's here

| File | Purpose |
|---|---|
| `PKGBUILD` | `flexclash-bin` — extracts the Tauri `.deb` data payload, verifies `/usr/bin/flexclash` + `/usr/bin/mihomo` landed, and provides desktop entry / icon fallbacks |
| `flexclash.install` | `post_install` / `post_upgrade` run `setcap cap_net_admin,cap_net_bind_service=+ep /usr/bin/mihomo` (TUN support); `post_remove` clears system-level residual state (per-user data is preserved) |
| `flexclash.desktop` | Desktop entry fallback (the `.deb`'s own entry is preferred) |
| `flexclash.png` | Icon fallback (128×128, copied from `src-tauri/icons`) |

## Build & install

```sh
cd packaging/arch
makepkg -si          # build and install (prompts for sudo)
makepkg -f           # force-rebuild after a failed run
```

`makepkg` will download
`FlexClash_<pkgver>_amd64.deb` from the GitHub Release matching `pkgver`
(`PKGBUILD` currently pins `0.6.2`; bump `pkgver` and the version in
`tauri.conf.json`/`package.json` before tagging a release).

### Requirements

- The release tag `v<pkgver>` must exist and contain the `.deb` asset
  (the CI workflow publishes it).
- `base-devel` (makepkg) and `libarchive` (`bsdtar`).
- The kernel binary gets its capabilities via `libcap` (`setcap`), part of
  Arch base.

## After install

- Launch via `flexclash` or the desktop entry.
- The `mihomo` kernel is already `setcap`'d; TUN mode needs no root.
- Per-user data lives under `~/.local/share/com.flexclash.app/` and is **not**
  removed on uninstall.

## Verifying the caps

```sh
getcap /usr/bin/mihomo
# cap_net_admin,cap_net_bind_service=ep
```

## Switching to a source build

A from-source `flexclash` PKGBUILD is intentionally not shipped: a Tauri build
needs the same WebKitGTK toolchain plus Node.js/Rust, and the sidecar must be
fetched into `src-tauri/binaries/` first (see
[`.github/actions/fetch-mihomo`](../../.github/actions/fetch-mihomo/action.yml)
for the pinned version/digest). If you want one, base it on the CI steps in
`.github/workflows/release.yml` and use `npm run tauri build -- --bundles deb`,
then install the resulting `target/release/bundle/deb/…/FlexClash_*.deb` payload
the same way this PKGBUILD does.
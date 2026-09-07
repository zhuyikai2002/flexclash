# Contributing to FlexClash

Thanks for your interest in contributing! This document explains the project
layout, the test strategy, and the contribution workflow.

---

## Project Layout

```
flexclash/
├── src/                          # Vue 3 frontend
│   ├── components/               # 13 .vue SFCs
│   ├── stores/                   # 9 Pinia stores
│   ├── services/                 # 8 Tauri wrappers
│   ├── composables/              # useTrafficStream / useConnectionMonitor
│   ├── types/                    # clash.d.ts (Mihomo API types)
│   ├── utils/                    # format.ts
│   └── App.vue
│
├── src-tauri/                    # Rust backend
│   ├── src/
│   │   ├── commands/             # 8 IPC modules (24 commands)
│   │   ├── core/                 # 7 submodules (sidecar, tun, route_guard, ...)
│   │   ├── store/                # SQLite history (migrations, queries, sampler)
│   │   ├── proxy/                # System proxy (windows.rs + unix.rs stub)
│   │   ├── config/               # Mihomo YAML profile management
│   │   ├── events.rs             # Tauri event constants
│   │   ├── tray.rs
│   │   ├── error.rs
│   │   └── lib.rs                # Tauri Builder
│   ├── tests/                    # 3 Rust integration tests
│   ├── resources/default_mihomo.yaml
│   ├── capabilities/default.json
│   ├── icons/
│   ├── build.rs
│   ├── Cargo.toml
│   ├── Cargo.lock                # COMMITTED (binary crate reproducibility)
│   └── tauri.conf.json
│
├── package.json                  # Frontend deps
├── vite.config.ts
├── tailwind.config.js
├── postcss.config.js
├── tsconfig.json
├── index.html
├── .gitignore
├── LICENSE                       # MIT
└── README.md
```

### What is NOT in the repo (and why)

| Path | Why |
|------|-----|
| `node_modules/` | npm/yarn handles — listed in `.gitignore` |
| `dist/` | Vite build output — `.gitignore` |
| `src-tauri/target/` | Cargo build output — `.gitignore` |
| `src-tauri/binaries/mihomo-x86_64-pc-windows-msvc.exe` | 50MB sidecar — `.gitignore` (download separately) |
| `release-artifacts/` | Build installers, kept locally for manual upload |
| `*.log`, `verify-m*.ps1`, etc. | Dev-time scratch files — `.gitignore` |

---

## Development Setup

```powershell
# 1. Clone
git clone https://github.com/zhuyikai2002/flexclash.git
cd flexclash

# 2. Install Node deps
npm install

# 3. Provide mihomo sidecar (FlexClash does not redistribute the binary)
#    Download mihomo v1.19.30 from
#    https://github.com/MetaCubeX/mihomo/releases/tag/v1.19.30
#    Place as: src-tauri/binaries/mihomo-x86_64-pc-windows-msvc.exe

# 4. Run dev mode (Vite HMR + Tauri hot reload)
npm run tauri:dev
```

### Toolchain

- **Node**: ≥ 20 (npm 10+)
- **Rust**: ≥ 1.77 (`rustup default stable`)
- **Tauri CLI**: 2.x (installed via `npm install` as devDependency)
- **Platform**: Windows 10 1809+ / Windows 11 (Tauri 2 desktop builds)
- For TUN mode: Wintun driver is bundled with the Tauri 2 install

---

## Test Strategy

FlexClash uses a layered testing approach. **All tests run on PR.**

### Layer 1: Rust unit tests (`cargo test`)

Located in `#[cfg(test)] mod tests` blocks throughout `src-tauri/src/`.

```powershell
cd src-tauri
cargo test --lib           # 21 unit tests
cargo test --test profile   # 3 integration tests
```

Coverage highlights:
- `core::sidecar` — process lifecycle
- `core::tun` — YAML inject/strip
- `core::elevate` — UAC helpers
- `core::route_guard` — sweep contract
- `store::queries` — SQLite aggregation
- `store::sampler` — rate × window

### Layer 2: TypeScript type check (`vue-tsc`)

```powershell
npm run build               # runs vue-tsc --noEmit && vite build
```

Catches every type error before bundle generation. Must report 0 errors.

### Layer 3: Vite build smoke

`vite build` after `vue-tsc` validates that the full dependency graph
(including Pinia, vue-virtual, vueuse, etc.) resolves and tree-shakes
correctly. Reports bundle size; budget is **+10 KB gzip per milestone**.

### Layer 4: Tauri release build (manual / CI)

```powershell
npm run tauri:build
```

Produces:
- `src-tauri/target/release/flexclash.exe` (raw binary)
- `src-tauri/target/release/bundle/nsis/FlexClash_*_x64-setup.exe`
- `src-tauri/target/release/bundle/msi/FlexClash_*_x64_en-US.msi`

> ⚠️ The release build takes ~8-10 min from cold. CI is the right place
> for this; locally run only when validating installer changes.

### What this project does NOT do

- **No `verify-m*.ps1` scripts**: those were private developer scratch
  from initial bootstrap. Real tests live in `cargo test` + the TypeScript
  build pipeline above.
- **No PowerShell-specific quirks** (ANSI escape substitution, `Get-CimInstance`,
  hardcoded paths) are encoded anywhere. Everything is cross-platform
  Cargo / npm / Node.
- **No end-to-end UI automation yet**: future work may add WebDriver
  tests (Tauri WebDriver) — PRs welcome.

---

## Commit Convention

[Conventional Commits](https://www.conventionalcommits.org/):

```
feat:     new feature
fix:      bug fix
docs:     documentation only
refactor: no functional change
test:     test additions / changes
chore:    build, CI, deps
```

Examples from this repo:

```
feat: initial release of FlexClash v0.1.0 (Phase 1 & 2 complete)
docs: add comprehensive bilingual README with architecture diagrams
chore: remove developer scratch files for open-source release
```

---

## Pull Request Checklist

- [ ] `cargo test` passes locally (21 lib + integration)
- [ ] `npm run build` reports 0 type errors
- [ ] `cargo check` reports 0 warnings (CI enforces `-D warnings`)
- [ ] Frontend bundle gzip delta ≤ +10 KB (vs. previous milestone)
- [ ] No new dependencies added without justification in the PR body
- [ ] If you added a new Rust dependency: `src-tauri/Cargo.lock` is updated

---

## Coding Standards

### Rust
- `cargo clippy` clean (CI enforces `clippy::all -D warnings`)
- `Mutex<T>` guards **never** cross `.await` points
- All public types have a doc comment
- Errors via `thiserror::Error`, propagated through `AppError`

### TypeScript
- Strict mode (`tsconfig.json` has `"strict": true`)
- Prefer `type` over `interface` for unions
- No `any` unless absolutely necessary (and document why)
- Use Pinia composables over options API

### Vue
- `<script setup lang="ts">` always
- Composition API only
- Composables prefixed with `use`
- Component names PascalCase, filenames PascalCase.vue

---

## License

By contributing, you agree your contributions will be licensed under
the project's [MIT License](./LICENSE).

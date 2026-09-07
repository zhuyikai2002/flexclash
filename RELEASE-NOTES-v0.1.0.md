# FlexClash v0.1.0 — First Public Release

**发布日期**: 2026-09-07 · **代号**: Phase 1 & 2 Complete

FlexClash 是一款基于 Tauri 2 (Rust) + Vue 3 的轻量级 Mihomo (Clash Meta) 代理客户端。本版本是首个公开版本，包含完整的 Phase 1 (核心代理管理) 与 Phase 2 (高级流量/TUN) 功能。

---

## ✨ New Features

### 核心代理管理 (Phase 1)
- **Mihomo sidecar 进程生命周期** — START-INFOJSON 协议管理子进程，崩溃自动重启 + 抖动退避
- **Windows 系统代理接管** — 注册表 + WinINET 32/64 位原子化切换，**零断网**
- **实时流量监控** — WebSocket 直连 `/traffic`，前端独立管理背压
- **订阅管理** — 订阅 URL + 配置文件本地存储，per-profile YAML 注入
- **节点选择 / 代理组** — selector / fallback / url-test / load-balance 全支持
- **桌面融合** — Windows 11 Mica / Win10 Acrylic 毛玻璃、开机自启、静默启动、系统托盘、单实例锁

### 高级流量 / TUN (Phase 2)
- **启动自愈** — 启动时 `sweep_residual_routes()` 清理上次异常退出残留
- **连接虚拟列表** — `@tanstack/vue-virtual` 实现 1万+ 活跃连接 60fps 流畅滚动
- **TUN 透明代理** — Wintun 内核驱动 + Plan C UAC 提权状态机，配置文件 `tun:` 块原子化注入/回滚
- **SQLite 流量历史** — `rusqlite 0.31 (bundled)`，1h/24h/7d 预聚合查询 + 30 天滚动保留
- **原生 Canvas 图表** — DPR-aware 波形图渲染，**无第三方图表库依赖**（节省 ~80KB gzip）
- **规则诊断视图** — 实时拉取 mihomo `/rules`，支持类型筛选 + 全文搜索

### 工程质量
- 21 个 Rust 单元测试 (含 10 个 M10 store 专项)
- 10 个端到端验证脚本 (`verify-m1.ps1` ~ `verify-m10.ps1`)，累计 80+ 断言
- `cargo check`: 0 errors / 0 warnings
- `vue-tsc --noEmit`: 0 type errors
- 前端 bundle: **82 KB JS gzip / 5.6 KB CSS gzip**（vs Electron 200MB+）

---

## 🐛 Bug Fixes

首个公开版本，无修复项记录。

---

## 📦 Downloads

| 文件 | 平台 | 大小 | SHA-256 |
|------|------|------|---------|
| `FlexClash_0.1.0_x64-setup.exe` | Windows x64 (NSIS) | 15.59 MB | `2C123FFCE0DCB2F139B1A0B67B0EC6236E86A6ECE249F970A19A967284E655F0` |
| `FlexClash_0.1.0_x64_en-US.msi` | Windows x64 (WiX MSI) | 21.70 MB | `57F8C140ECB70438B7E6798D36DF81A10C4E2C79F41E513D26EE737F2E541CF7` |
| `flexclash.exe` | Windows x64 (raw portable) | 8.07 MB | `60A7640788FEED22777583A95AC8D41926BBCF2AE33DDC6E59D07709301AA89B` |

> **校验方法**:
> ```powershell
> Get-FileHash .\FlexClash_0.1.0_x64-setup.exe -Algorithm SHA256
> ```

### 推荐安装方式

普通用户请下载 **NSIS 安装包**（`.exe`），体积更小且提供"开机自启"选项。
企业批量部署（域控 GPO / Intune）请下载 **MSI 安装包**。

### 系统要求

- Windows 10 1809+ / Windows 11 (推荐 11 启用 Mica)
- mihomo v1.19.30 sidecar 会随安装包一同部署
- TUN 模式首次启动会触发 UAC 弹窗（仅一次）

---

## ⚠️ Known Limitations

- **代码签名**: 当前未配置 EV/OV 代码签名证书。Win11 上首次运行会触发 SmartScreen 警告（"未知发布者"），点击"仍要运行"即可。
- **平台**: 当前仅 Windows 完整支持。macOS / Linux 的 `#[cfg(unix)]` stub 已就位，待社区贡献驱动层。

---

## 🛠️ Checksums

```text
60A7640788FEED22777583A95AC8D41926BBCF2AE33DDC6E59D07709301AA89B  flexclash.exe
2C123FFCE0DCB2F139B1A0B67B0EC6236E86A6ECE249F970A19A967284E655F0  FlexClash_0.1.0_x64-setup.exe
57F8C140ECB70438B7E6798D36DF81A10C4E2C79F41E513D26EE737F2E541CF7  FlexClash_0.1.0_x64_en-US.msi
6AC25FCB26AFE8E1BEA24B6E6E80805BF884A33232D12E2D78DFA0B6C529AC14  mihomo.exe (sidecar, bundled in installers)
```

---

## 🙏 Credits

- [Mihomo](https://github.com/MetaCubeX/mihomo) by MetaCubeX — Clash Meta 核心引擎
- [Tauri](https://tauri.app/) by tauri-apps — Rust 桌面框架
- [Wintun](https://www.wintun.net/) by WireGuard — TUN 驱动
- [Vue.js](https://vuejs.org/) + [Vite](https://vitejs.dev/) + [Pinia](https://pinia.vuejs.org/) — 前端工具链

---

**License**: MIT · 详见 [LICENSE](./LICENSE)

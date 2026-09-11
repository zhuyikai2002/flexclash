🚀 What's New in v0.2.2

## 1. 核心体验重构：纯静默 TUN 模式提权

- **彻底消除黑框闪烁**：全量重构 Windows 子进程派生逻辑，严格收敛为 `CREATE_NO_WINDOW` 与显式 `SW_HIDE` 调用，杜绝 UAC 提权前后的控制台窗口闪烁。
- **干掉高频子进程轮询**：废弃原先每 200ms 调用一次 `curl.exe` 的暴力探针，重构为 Rust 原生进程内裸 TCP HTTP 探针，TUN 开启路径做到零外部依赖、零进程抖动（Process Churn）。

## 2. 基础设施：自动更新管线（Auto-Updater）正式闭环

- 深度集成 `tauri-plugin-updater`，采用 Minisign (Ed25519 + BLAKE2b-512) 工业级数字签名。
- 自动化构建脚本支持一键生成并校验 `latest.json` 签名摘要。

## ⚠️ 重要升级说明（必读）

由于 v0.2.2 正式重构了签名机制并轮换了最新的 Minisign 密钥对，**v0.2.1 及更早版本的存量用户无法通过客户端内置更新自动升级至此版本**。

请下载下方的 `FlexClash_0.2.2_x64-setup.exe` 进行**手动覆盖安装一次**。安装完成后，后续所有版本均支持客户端内一键静默自动更新！

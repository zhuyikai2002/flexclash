# FlexClash GitHub 发布手册

本手册涵盖从"本地 Git 仓库"到"GitHub Release 公开下载"的完整流程。

---

## 0. 仓库状态确认

当前状态（截至 2026-09-07）：

```
[main] 共有 2 个本地提交：
  c08c9a5  feat: initial release of FlexClash v0.1.0 (Phase 1 & 2 complete)
  e0887ff  docs: add comprehensive bilingual README with architecture diagrams

待推送文件: 0（所有文件已 commit）
待发布 artifacts:
  release-artifacts/FlexClash_0.1.0_x64-setup.exe    15.59 MB
  release-artifacts/FlexClash_0.1.0_x64_en-US.msi    21.70 MB
  release-artifacts/flexclash.exe                     8.07 MB
```

---

## 1. GitHub 认证（一次性）

打开 PowerShell，登录 GitHub：

```powershell
& "C:\Program Files\GitHub CLI\gh.exe" auth login
```

按提示选择：
1. **What account do you want to log into?** → `GitHub.com`
2. **What is your preferred protocol for Git operations?** → `HTTPS`（或 SSH，看你习惯）
3. **Authenticate Git with your GitHub credentials?** → `Yes`
4. **How would you like to authenticate GitHub CLI?** → `Login with a web browser`
   - gh 会打印一个一次性代码，例如 `XXXX-XXXX`
   - 按 Enter 打开浏览器，粘贴代码，授权
5. 成功后再次验证：

```powershell
& "C:\Program Files\GitHub CLI\gh.exe" auth status
# 期望输出: ✓ Logged in to github.com as <your-username>
```

> 💡 **不想用浏览器？** 也可以用 Personal Access Token (PAT):
> ```powershell
> $env:GH_TOKEN = "ghp_xxxxxxxxxxxxxxxxxxxx"
> & "C:\Program Files\GitHub CLI\gh.exe" auth login --with-token
> ```
> 在 https://github.com/settings/tokens 生成 PAT（需要 `repo` + `workflow` 权限）。

---

## 2. 创建远程仓库 + 推送

### 方法 A：一键创建并推送（推荐）

```powershell
Set-Location C:\Users\rik\projects\flexclash

# 创建公开仓库、添加 origin、推送 main
& "C:\Program Files\GitHub CLI\gh.exe" repo create flexclash `
    --public `
    --source=. `
    --remote=origin `
    --push `
    --description "FlexClash — lightweight Mihomo (Clash Meta) proxy client built on Tauri 2 + Vue 3"
```

成功后访问 https://github.com/yourname/flexclash 即可看到代码。

### 方法 B：手动创建（不通过 gh）

1. 在 https://github.com/new 创建空仓库（不要勾选 Initialize with README/.gitignore/license）
2. 本地推送：

```powershell
Set-Location C:\Users\rik\projects\flexclash
git remote add origin https://github.com/yourname/flexclash.git
git push -u origin main
```

---

## 3. 创建 v0.1.0 GitHub Release

将 3 个安装包上传并附上 Release Notes：

```powershell
Set-Location C:\Users\rik\projects\flexclash

# 一次性发布：上传 artifacts + 自动生成 release notes
& "C:\Program Files\GitHub CLI\gh.exe" release create v0.1.0 `
    .\release-artifacts\FlexClash_0.1.0_x64-setup.exe `
    .\release-artifacts\FlexClash_0.1.0_x64_en-US.msi `
    .\release-artifacts\flexclash.exe `
    --title "FlexClash v0.1.0 — First Public Release (Phase 1 & 2 Complete)" `
    --notes-file .\RELEASE-NOTES-v0.1.0.md `
    --target main
```

参数说明：
- `v0.1.0` — tag 名（首次推送时会自动创建 tag）
- 3 个文件路径 — 依次为 NSIS / MSI / raw exe
- `--title` — Release 页面顶部标题
- `--notes-file` — 从本地文件读取 Release Notes（也可直接用 `--notes "..."` 内联）
- `--target main` — tag 指向 main 分支 HEAD

成功后访问 https://github.com/yourname/flexclash/releases/tag/v0.1.0 即可看到下载页。

---

## 4. （可选）发布预发布版

如果想先用 pre-release 频道测试：

```powershell
& "C:\Program Files\GitHub CLI\gh.exe" release create v0.1.0-rc.1 `
    .\release-artifacts\FlexClash_0.1.0_x64-setup.exe `
    --prerelease `
    --title "FlexClash v0.1.0-rc.1" `
    --notes "Release candidate — please test before final release."
```

---

## 5. （可选）添加 Topics 让仓库可发现

```powershell
& "C:\Program Files\GitHub CLI\gh.exe" repo edit yourname/flexclash `
    --add-topic tauri `
    --add-topic rust `
    --add-topic vue3 `
    --add-topic clash `
    --add-topic mihomo `
    --add-topic proxy `
    --add-topic windows `
    --add-topic v2ray `
    --add-topic shadowsocks
```

---

## 6. （可选）配置 GitHub Pages 文档站点

如果想用 GitHub Pages 托管文档：

1. Settings → Pages → Source: `Deploy from a branch` → `main` / `/docs`
2. 创建 `docs/` 目录并放入详细文档
3. 推送后 https://yourname.github.io/flexclash/ 自动上线

---

## 7. 故障排查

| 症状 | 解决 |
|------|------|
| `gh: command not found` | 用全路径 `"C:\Program Files\GitHub CLI\gh.exe"`，或将其加入 PATH |
| `error: failed to push` | 检查 `git remote -v`；可能 origin 指向了其他仓库 |
| `SSL certificate problem` | `git config --global http.sslVerify false`（仅调试用） |
| 403 权限不足 | PAT 缺少 `repo` scope；重新生成时勾选 `repo` + `workflow` |
| `tag v0.1.0 already exists` | 改用 `v0.1.1` 或先 `git tag -d v0.1.0` + `git push origin :refs/tags/v0.1.0` |
| Release 页面空白 | 等待 1-2 分钟（GitHub 异步处理 assets） |
| SmartScreen 警告 | 未签名 → 用户点击"更多信息"→"仍要运行" |

---

## 8. 发布后清单

- [x] Git 仓库推送完成
- [x] Release v0.1.0 公开下载
- [ ] 在社交媒体（Twitter / V2EX / 微博）发布公告
- [ ] 提交到 awesome-tauri 列表
- [ ] 配置 GitHub Actions CI（后续）

---

**最后更新**: 2026-09-07 · 与 commit `e0887ff` 同步

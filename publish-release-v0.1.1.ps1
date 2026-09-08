# ================================================================
# publish-release-v0.1.1.ps1
# ------------------------------------------------------------
# 1) Set GH_TOKEN env (fine-grained PAT with `contents: write`)
# 2) Run this script. It will:
#    - Create annotated tag v0.1.1
#    - Push tag to origin
#    - Create GitHub Release with:
#        * Title: FlexClash v0.1.1 - 几何猫咪 + 桌面 UI 重塑 + i18n
#        * Body:  bilingual notes from RELEASE-NOTES-v0.1.1.md
#        * Assets:
#            - FlexClash_0.1.1_x64-setup.exe
#            - FlexClash_0.1.1_x64_en-US.msi
#            - flexclash.exe
#            - SHA256SUMS.txt
#        * Prerelease: false
#        * Draft:    false
# ================================================================

$ErrorActionPreference = 'Stop'
$env:Path = "C:\Program Files\GitHub CLI;$env:Path"

# sanity: must have a token
if (-not $env:GH_TOKEN) {
  Write-Host "GH_TOKEN is not set. Export a fine-grained PAT with `contents: write` first:" -ForegroundColor Red
  Write-Host '  $env:GH_TOKEN = "ghp_..."   # Windows PowerShell 7' -ForegroundColor Red
  exit 1
}

$root = 'C:\Users\rik\projects\flexclash'
$notes = Join-Path $root 'RELEASE-NOTES-v0.1.1.md'
$bundleDir = Join-Path $root 'src-tauri\target\release\bundle'
$exe = Join-Path $root 'src-tauri\target\release\flexclash.exe'
$artifacts = @(
  Join-Path $bundleDir 'nsis\FlexClash_0.1.1_x64-setup.exe',
  Join-Path $bundleDir 'msi\FlexClash_0.1.1_x64_en-US.msi',
  $exe,
  Join-Path $bundleDir 'SHA256SUMS.txt'
)

# Sanity check
foreach ($a in $artifacts) {
  if (-not (Test-Path $a)) { throw "missing artifact: $a" }
  $size = [math]::Round((Get-Item $a).Length / 1MB, 2)
  Write-Host "  artifact: $a ($size MB)"
}
if (-not (Test-Path $notes)) { throw "missing notes: $notes" }

Set-Location $root

# Tag (annotated) and push
$tag = 'v0.1.1'
Write-Host "=== creating annotated tag $tag ===" -ForegroundColor Cyan
& git.exe tag -d $tag 2>$null | Out-Null   # idempotent
& git.exe tag -a $tag -F $notes
& git.exe push origin $tag --force

# gh release create (uses $env:GH_TOKEN)
$title = 'FlexClash v0.1.1 - 几何猫咪 + 桌面 UI 重塑 + i18n'
Write-Host "=== creating GitHub Release $tag ===" -ForegroundColor Cyan
& gh.exe release create $tag @artifacts `
  --title $title `
  --notes-file $notes `
  --target main `
  --verify-tag

Write-Host "=== done ===" -ForegroundColor Green
& gh.exe release view $tag --json url,name,tagName,assets | Out-String

# Security: clear token
Remove-Item Env:GH_TOKEN -ErrorAction SilentlyContinue
Write-Host "(GH_TOKEN env cleared)" -ForegroundColor DarkGray

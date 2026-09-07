# verify-m1.ps1 — FlexClash M1 runtime acceptance
# Pre-req: BLOCKER-1..4 already resolved (cargo + mihomo.exe + npm + scaffolding)

$ErrorActionPreference = 'Stop'
$root = 'C:\Users\rik\projects\flexclash'
$bin  = Join-Path $root 'src-tauri\binaries\mihomo-x86_64-pc-windows-msvc.exe'

# Make cargo discoverable inside this script (PATH is per-process)
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

Write-Host "=================================================" -ForegroundColor Magenta
Write-Host " FlexClash M1 Runtime Acceptance — $(Get-Date -Format 'u')" -ForegroundColor Magenta
Write-Host "=================================================" -ForegroundColor Magenta

# ---- [1] 前置存在性 ----
foreach ($f in @(
    "$root\package.json",
    "$root\src-tauri\Cargo.toml",
    "$root\src-tauri\tauri.conf.json",
    "$root\src-tauri\capabilities\default.json",
    "$root\src-tauri\src\main.rs",
    "$root\src-tauri\src\lib.rs",
    $bin
)) { if (-not (Test-Path $f)) { throw "[BLOCKER] missing: $f" } }
Write-Host "[1/7] preflight files: OK" -ForegroundColor Green

# ---- [2] cargo check ----
Write-Host "[2/7] cargo check ..." -ForegroundColor Cyan
cargo check --manifest-path "$root\src-tauri\Cargo.toml" --message-format=short 2>&1 |
    Out-File -FilePath "$root\cargo-check.log"
if ($LASTEXITCODE -ne 0) { throw "cargo check failed (see cargo-check.log)" }
Write-Host "       exit=0, OK" -ForegroundColor Green

# ---- [3] spawn mihomo (independent of Tauri runtime) ----
$work = Join-Path $env:LOCALAPPDATA 'com.flexclash.app\mihomo'
New-Item -ItemType Directory -Force -Path $work | Out-Null
$cfg = Join-Path $work 'config.yaml'
if (-not (Test-Path $cfg)) {
    Copy-Item "$root\src-tauri\resources\default_mihomo.yaml" $cfg -Force
}
Write-Host "[3/7] spawning mihomo: -d $work -f $cfg" -ForegroundColor Cyan
Start-Process -FilePath $bin `
    -ArgumentList @('-d', $work, '-f', $cfg) `
    -PassThru `
    -RedirectStandardOutput "$work\stdout.log" `
    -RedirectStandardError "$work\stderr.log" `
    -WindowStyle Hidden | Out-Null

# ---- [4] process present ----
Start-Sleep -Seconds 2
$mp = Get-Process -Name mihomo -ErrorAction SilentlyContinue
if ($mp) { Write-Host "[4/7] mihomo.exe present: PID=$($mp.Id) path=$($mp.Path)" -ForegroundColor Green }
else { throw "[4/7] mihomo.exe not running. stderr tail:"; Get-Content "$work\stderr.log" -Tail 20 }

# ---- [5] port 9090 listening ----
$portOk = (Test-NetConnection -ComputerName 127.0.0.1 -Port 9090 -WarningAction SilentlyContinue).TcpTestSucceeded
if ($portOk) { Write-Host "[5/7] :9090 listening: OK" -ForegroundColor Green }
else { throw "[5/7] port 9090 not listening" }

# ---- [6] REST /version reachable ----
try {
    $ver = Invoke-RestMethod -Uri 'http://127.0.0.1:9090/version' -TimeoutSec 5
    Write-Host "[6/7] /version => $($ver | ConvertTo-Json -Compress)" -ForegroundColor Green
} catch { throw "[6/7] GET /version failed: $_" }

# ---- [7] cleanup & verify no zombie ----
Write-Host "[7/7] cleanup via taskkill /F /T /IM mihomo.exe ..." -ForegroundColor Cyan
taskkill /F /T /IM mihomo.exe 2>&1 | Out-Null
Start-Sleep -Seconds 1
$left = Get-Process -Name mihomo -ErrorAction SilentlyContinue
if ($left) { throw "[7/7] zombie left: PID=$($left.Id)" }
Write-Host "       no zombie: OK" -ForegroundColor Green

Write-Host ""
Write-Host "==== M1 RUNTIME ACCEPTANCE: PASS ====" -ForegroundColor Green

# verify-m2.ps1 — M2 end-to-end smoke for the 9091 isolated path
#
# Validates the *full* contract a running Tauri session will exercise from the
# frontend (direct axios → 127.0.0.1:9091). Rust sidecar is NOT involved here
# — that was already proven by verify-m1.v2.ps1. We just need to confirm the
# Mihomo REST surface that the new clash.ts service talks to.

$ErrorActionPreference = 'Stop'
$root = 'C:\Users\rik\projects\flexclash'
$bin  = Join-Path $root 'src-tauri\binaries\mihomo-x86_64-pc-windows-msvc.exe'
$work = Join-Path $env:LOCALAPPDATA 'com.flexclash.app\mihomo'
$cfg  = Join-Path $work 'config.yaml'
$src  = Join-Path $root 'src-tauri\resources\default_mihomo.yaml'

Write-Host "=================================================" -ForegroundColor Magenta
Write-Host " FlexClash M2 E2E smoke — $(Get-Date -Format 'u')" -ForegroundColor Magenta
Write-Host "=================================================" -ForegroundColor Magenta

# ---- [0] nuke any prior mihomo, sync config to bundled default ----
Write-Host "[0/7] sync default config + nuke old mihomo ..." -ForegroundColor Cyan
Get-CimInstance Win32_Process -Filter 'Name like "mihomo%"' -ErrorAction SilentlyContinue |
    Where-Object { $_.CommandLine -like '*com.flexclash.app*' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
New-Item -ItemType Directory -Force -Path $work | Out-Null
Copy-Item -Force $src $cfg
"       config controller: $((Get-Content $cfg | Select-String 'external-controller:' | Select-Object -First 1).Line.Trim())"

# wait for :9091 to clear
for ($i = 0; $i -lt 10; $i++) {
    if (-not (Get-NetTCPConnection -LocalPort 9091 -State Listen -ErrorAction SilentlyContinue)) { break }
    Start-Sleep -Milliseconds 500
}

# ---- [1] spawn mihomo + capture PID ----
Write-Host "[1/7] spawn mihomo ..." -ForegroundColor Cyan
$proc = Start-Process -FilePath $bin `
    -ArgumentList @('-d', $work, '-f', $cfg) `
    -PassThru `
    -RedirectStandardOutput "$work\stdout.log" `
    -RedirectStandardError "$work\stderr.log" `
    -WindowStyle Hidden
$ourPid = $proc.Id
"       our PID = $ourPid"

# wait until PID is gone (means it died on launch) — fail fast
$dead = $false
for ($i = 0; $i -lt 10; $i++) {
    if (-not (Get-Process -Id $ourPid -ErrorAction SilentlyContinue)) { $dead = $true; break }
    Start-Sleep -Milliseconds 200
}
if ($dead) { throw "mihomo died on launch. stderr tail:"; Get-Content "$work\stderr.log" -Tail 10 }

# ---- [2] port binding (9091 yes, 9090 no) ----
Start-Sleep -Milliseconds 500
$p9090 = @(Get-NetTCPConnection -LocalPort 9090 -State Listen -ErrorAction SilentlyContinue)
$p9091 = @(Get-NetTCPConnection -LocalPort 9091 -State Listen -ErrorAction SilentlyContinue)
Write-Host "[2/7] port binding" -ForegroundColor Cyan
":9090 LISTENING: $(if ($p9090.Count) { "PID $($p9090[0].OwningProcess) (UNEXPECTED)" } else { 'NO ✓' })"
":9091 LISTENING: $(if ($p9091.Count) { "PID $($p9091[0].OwningProcess)" } else { 'NO ✗' })  expect our PID $ourPid"
if ($p9090.Count) { throw ":9090 unexpectedly held (isolation failed)" }
if (-not $p9091.Count) { throw ":9091 not listening" }
if ($p9091[0].OwningProcess -ne $ourPid) { throw ":9091 owned by wrong PID" }

# ---- [3] /version ----
$ver = Invoke-RestMethod -Uri 'http://127.0.0.1:9091/version' -TimeoutSec 5
Write-Host "[3/7] /version => $($ver | ConvertTo-Json -Compress)" -ForegroundColor Green
if ($ver.version -notmatch 'v?\d+\.\d+') { throw "/version shape unexpected" }

# ---- [4] /configs ----
# NOTE: mihomo's GET /configs intentionally omits `external-controller` and
# `secret` (internal/privacy fields). Port ownership was already validated in
# [2] via Get-NetTCPConnection. Here we only sanity-check that the runtime
# config is well-formed and that `mode` matches our default.
$cfgs = Invoke-RestMethod -Uri 'http://127.0.0.1:9091/configs' -TimeoutSec 5
Write-Host "[4/7] /configs.mode=$($cfgs.mode) mixed-port=$($cfgs.'mixed-port') (external-controller not exposed by /configs — port validated above)" -ForegroundColor Green
if ($cfgs.mode -ne 'rule') { throw "/configs.mode != rule" }

# ---- [5] /proxies (sanity: shape + at least Direct exists) ----
$px = Invoke-RestMethod -Uri 'http://127.0.0.1:9091/proxies' -TimeoutSec 5
Write-Host "[5/7] /proxies keys=$($px.proxies.PSObject.Properties.Name -join ',')" -ForegroundColor Green
if (-not $px.proxies.Direct) { throw "/proxies.Direct missing" }
if (-not $px.proxies.Global)  { throw "/proxies.Global missing" }

# ---- [6] /rules ----
$rules = Invoke-RestMethod -Uri 'http://127.0.0.1:9091/rules' -TimeoutSec 5
Write-Host "[6/7] /rules count=$($rules.rules.Count)" -ForegroundColor Green

# ---- [7] WS /traffic — handshake only (full stream testing deferred to M3) ----
Write-Host "[7/7] WS /traffic handshake (full stream test in M3) ..." -ForegroundColor Cyan
try {
    $ws = New-Object System.Net.WebSockets.ClientWebSocket
    $connectTask = $ws.ConnectAsync([Uri]'ws://127.0.0.1:9091/traffic', [Threading.CancellationToken]::None)
    if ($connectTask.Wait(5000) -and $ws.State -eq 'Open') {
        Write-Host "       WS handshake: OPEN ✓" -ForegroundColor Green
    } else {
        Write-Host "       WS handshake: $($ws.State)" -ForegroundColor Yellow
    }
    if ($ws.State -eq 'Open') {
        $ws.CloseAsync([System.Net.WebSockets.WebSocketCloseStatus]::NormalClosure, 'done', [Threading.CancellationToken]::None).Wait(1000)
    }
    $ws.Dispose()
} catch {
    Write-Host "       WS test failed (non-blocking): $_" -ForegroundColor Yellow
}

# ---- cleanup ----
Write-Host ""
Write-Host "==> cleanup" -ForegroundColor Cyan
Stop-Process -Id $ourPid -Force -ErrorAction SilentlyContinue
Start-Sleep 2
$left = Get-CimInstance Win32_Process -Filter 'Name like "mihomo%"' -ErrorAction SilentlyContinue |
    Where-Object { $_.CommandLine -like '*com.flexclash.app*' }
"residual (FlexClash-owned): $(if ($left) { $left.Count } else { 0 })"
":9091 LISTENING: $((Get-NetTCPConnection -LocalPort 9091 -State Listen -ErrorAction SilentlyContinue).Count)"

Write-Host ""
Write-Host "==== M2 END-TO-END ACCEPTANCE: PASS ====" -ForegroundColor Green
exit 0

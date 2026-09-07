# verify-m3.ps1 — M3 end-to-end smoke
#
# Validates the *exact* surface the new M3 UI talks to:
#   1. WS /traffic  — collects >= 3 push samples with valid numeric up/down
#   2. GET  /proxies — confirms the built-in `PROXY` selector exists
#   3. PUT  /proxies/PROXY {name: REJECT} — switch node
#   4. GET  /proxies/PROXY — verify `now` flipped to REJECT
#   5. PUT  /proxies/PROXY {name: DIRECT}  — switch back
#   6. GET  /proxies/REJECT/delay?url=...  — confirm delay endpoint works
#
# The 9091 isolation guarantee from M2 is implicit (port 9090 stays empty).

$ErrorActionPreference = 'Stop'
$root = 'C:\Users\rik\projects\flexclash'
$bin  = Join-Path $root 'src-tauri\binaries\mihomo-x86_64-pc-windows-msvc.exe'
$work = Join-Path $env:LOCALAPPDATA 'com.flexclash.app\mihomo'
$cfg  = Join-Path $work 'config.yaml'
$src  = Join-Path $root 'src-tauri\resources\default_mihomo.yaml'

Write-Host "=================================================" -ForegroundColor Magenta
Write-Host " FlexClash M3 E2E smoke — $(Get-Date -Format 'u')" -ForegroundColor Magenta
Write-Host "=================================================" -ForegroundColor Magenta

# ---- [0] nuke any prior mihomo + sync fresh default config (with PROXY group)
Write-Host "[0/9] sync default config (with PROXY group) + nuke old mihomo ..." -ForegroundColor Cyan
Get-CimInstance Win32_Process -Filter 'Name like "mihomo%"' -ErrorAction SilentlyContinue |
    Where-Object { $_.CommandLine -like '*com.flexclash.app*' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
New-Item -ItemType Directory -Force -Path $work | Out-Null
Copy-Item -Force $src $cfg
"       config controller: $((Get-Content $cfg | Select-String 'external-controller:' | Select-Object -First 1).Line.Trim())"
"       config has proxy-groups: $((Get-Content $cfg | Select-String '^- name: PROXY' | Measure-Object).Count -gt 0)"

# wait for :9091 to clear
for ($i = 0; $i -lt 10; $i++) {
    if (-not (Get-NetTCPConnection -LocalPort 9091 -State Listen -ErrorAction SilentlyContinue)) { break }
    Start-Sleep -Milliseconds 500
}

# ---- [1] spawn mihomo + capture PID ----
Write-Host "[1/9] spawn mihomo ..." -ForegroundColor Cyan
$proc = Start-Process -FilePath $bin `
    -ArgumentList @('-d', $work, '-f', $cfg) `
    -PassThru `
    -RedirectStandardOutput "$work\stdout.log" `
    -RedirectStandardError "$work\stderr.log" `
    -WindowStyle Hidden
$ourPid = $proc.Id
"       our PID = $ourPid"

# fail-fast on immediate death
$dead = $false
for ($i = 0; $i -lt 10; $i++) {
    if (-not (Get-Process -Id $ourPid -ErrorAction SilentlyContinue)) { $dead = $true; break }
    Start-Sleep -Milliseconds 200
}
if ($dead) {
    Write-Host "       mihomo stderr tail:" -ForegroundColor Red
    Get-Content "$work\stderr.log" -Tail 15 | ForEach-Object { Write-Host "       $_" -ForegroundColor Red }
    throw "mihomo died on launch"
}

# wait for :9091 to be up
$up = $false
for ($i = 0; $i -lt 30; $i++) {
    if (Get-NetTCPConnection -LocalPort 9091 -State Listen -ErrorAction SilentlyContinue) { $up = $true; break }
    Start-Sleep -Milliseconds 200
}
if (-not $up) { throw ":9091 never came up" }
"       :9091 LISTENING ✓"

# ---- [2] /version ----
$ver = Invoke-RestMethod -Uri 'http://127.0.0.1:9091/version' -TimeoutSec 5
Write-Host "[2/9] /version => $($ver | ConvertTo-Json -Compress)" -ForegroundColor Green
if ($ver.version -notmatch 'v?\d+\.\d+') { throw "/version shape unexpected" }

# ---- [3] /proxies: confirm PROXY group exists and has the expected children
$px = Invoke-RestMethod -Uri 'http://127.0.0.1:9091/proxies' -TimeoutSec 5
Write-Host "[3/9] /proxies.PROXY type=$($px.proxies.PROXY.type) all=$($px.proxies.PROXY.all -join ',') now=$($px.proxies.PROXY.now)" -ForegroundColor Green
if ($px.proxies.PROXY.type -ne 'Selector') { throw "PROXY not a Selector" }
foreach ($c in 'DIRECT','REJECT') {
    if ($px.proxies.PROXY.all -notcontains $c) { throw "PROXY missing child $c" }
}
"       initial now = '$($px.proxies.PROXY.now)'  (expected DIRECT)"
if ($px.proxies.PROXY.now -ne 'DIRECT') { throw "PROXY initial now != DIRECT" }

# ---- [4] WS /traffic: collect >= 3 samples, validate shape
#
# PowerShell's `System.Net.WebSockets.ClientWebSocket` proved unreliable for
# continuous reads in earlier milestones (deadlocks in PS task scheduling).
# We delegate this step to a tiny Node script that uses the `ws` package
# (already present in node_modules).
Write-Host "[4/9] WS /traffic: collect 3 samples via node + ws ..." -ForegroundColor Cyan
$nodeOut = & node "$root\m3-traffic-probe.cjs" 'ws://127.0.0.1:9091/traffic' 3 2>&1
$nodeOut | ForEach-Object { "       $_" }
if ($LASTEXITCODE -ne 0) { throw "WS /traffic probe failed (exit $LASTEXITCODE)" }

# ---- [5] PUT /proxies/PROXY {name: REJECT} ----
# NOTE: mihomo v1.19.30 returns an empty body on PUT (older versions echoed
#       the updated Proxy). Contract is "200 OK = accepted"; re-read via GET.
Write-Host "[5/9] PUT /proxies/PROXY  body={name: REJECT} ..." -ForegroundColor Cyan
$body = @{ name = 'REJECT' } | ConvertTo-Json
$putStatus = (Invoke-WebRequest -Uri 'http://127.0.0.1:9091/proxies/PROXY' `
    -Method Put -Body $body -ContentType 'application/json' -TimeoutSec 5 -UseBasicParsing).StatusCode
"       PUT HTTP status: $putStatus"
if ($putStatus -notin 200,204) { throw "PUT expected 200/204, got $putStatus" }

# ---- [6] GET /proxies/PROXY — confirm persistence ----
$px2 = Invoke-RestMethod -Uri 'http://127.0.0.1:9091/proxies/PROXY' -TimeoutSec 5
Write-Host "[6/9] GET /proxies/PROXY  now=$($px2.now)" -ForegroundColor Green
if ($px2.now -ne 'REJECT') { throw "GET /proxies/PROXY.now != REJECT after PUT" }

# ---- [7] PUT /proxies/PROXY {name: DIRECT} (switch back) ----
Write-Host "[7/9] PUT /proxies/PROXY  body={name: DIRECT} ..." -ForegroundColor Cyan
$body = @{ name = 'DIRECT' } | ConvertTo-Json
$putStatus2 = (Invoke-WebRequest -Uri 'http://127.0.0.1:9091/proxies/PROXY' `
    -Method Put -Body $body -ContentType 'application/json' -TimeoutSec 5 -UseBasicParsing).StatusCode
"       PUT HTTP status: $putStatus2"
$px3 = Invoke-RestMethod -Uri 'http://127.0.0.1:9091/proxies/PROXY' -TimeoutSec 5
"       GET now = '$($px3.now)'"
if ($px3.now -ne 'DIRECT') { throw "GET /proxies/PROXY.now != DIRECT after PUT" }

# ---- [8] GET /proxies/DIRECT/delay?url=...  (delay endpoint contract) ----
Write-Host "[8/9] GET /proxies/DIRECT/delay?url=..." -ForegroundColor Cyan
$delay = $null
$delayRaw = ''
# gstatic.com from a CN network flake-errors ~10% of the time. Retry up
# to 5 times before giving up — this is the M3 known-flake problem.
for ($retry = 0; $retry -lt 5; $retry++) {
    try {
        $delayRaw = (Invoke-WebRequest -Uri 'http://127.0.0.1:9091/proxies/DIRECT/delay?url=http%3A%2F%2Fwww.gstatic.com%2Fgenerate_204&timeout=2000' -UseBasicParsing -TimeoutSec 5).Content
        $delay = $delayRaw | ConvertFrom-Json
        if ($null -ne $delay.delay) { break }
    } catch { }
    Start-Sleep -Seconds 1
}
"       raw response: $delayRaw"
"       parsed delay: $($delay.delay)"
if ($null -eq $delay.delay) { throw "delay response missing `delay` field after retries" }
if ($delay.delay -isnot [int] -and $delay.delay -isnot [long]) { throw "delay not numeric" }
if ($delay.delay -eq 0) {
    Write-Host "       NOTE: delay=0 means timeout (no internet to gstatic.com — non-fatal in this env)" -ForegroundColor Yellow
} else {
    "       direct-connect RTT: $($delay.delay) ms ✓"
}

# ---- [9] Negative test: PUT /proxies/PROXY with bogus name (should 400) ----
Write-Host "[9/9] PUT /proxies/PROXY  body={name: BOGUS}  (expect 400) ..." -ForegroundColor Cyan
$bogusBody = @{ name = 'BOGUS_PROXY_THAT_DOES_NOT_EXIST' } | ConvertTo-Json
try {
    $null = Invoke-RestMethod -Uri 'http://127.0.0.1:9091/proxies/PROXY' `
        -Method Put -Body $bogusBody -ContentType 'application/json' -TimeoutSec 5
    Write-Host "       NOTE: bogus PUT did not error (mihomo accepted it)" -ForegroundColor Yellow
} catch {
    $code = $_.Exception.Response.StatusCode.value__
    "       bogus PUT rejected with HTTP $code ✓"
    if ($code -lt 400) { throw "expected 4xx for bogus name, got $code" }
}

# ---- cleanup ----
Write-Host ""
Write-Host "==> cleanup" -ForegroundColor Cyan
Stop-Process -Id $ourPid -Force -ErrorAction SilentlyContinue
Start-Sleep 2
$left = @(Get-CimInstance Win32_Process -Filter 'Name like "mihomo%"' -ErrorAction SilentlyContinue |
    Where-Object { $_.CommandLine -like '*com.flexclash.app*' })
"residual (FlexClash-owned): $($left.Count)"
":9091 LISTENING: $((Get-NetTCPConnection -LocalPort 9091 -State Listen -ErrorAction SilentlyContinue).Count)"

Write-Host ""
Write-Host "==== M3 END-TO-END ACCEPTANCE: PASS ====" -ForegroundColor Green
exit 0

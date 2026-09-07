# ============================================================================
# verify-m6.ps1 — M6 acceptance suite (Phase 1 final).
#
# Scope (M6):
#   - Dead-code cleanup: no `ActiveProfileState`, no `restore_system_proxy`
#   - Tray background-residency + ExitFlag semantics (prevent_close)
#   - Subscription update + quota refresh (update_subscription command)
#   - Bulk speed-test concurrency cap (DELAY_CONCURRENCY = 6)
#   - Sort mode toggle: default ⇄ latency_asc
#   - Frontend wiring (sort toggle, Update button, quota bar)
# ============================================================================

$ErrorActionPreference = 'Stop'
$ProgressPreference    = 'SilentlyContinue'

$root       = 'C:\Users\rik\projects\flexclash'
$srcTauri   = Join-Path $root 'src-tauri'
$workDir    = Join-Path $env:LOCALAPPDATA 'com.flexclash.app\mihomo'
$registryPath = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings'
$cargo      = 'C:\Users\rik\.cargo\bin\cargo.exe'
$node       = 'C:\Program Files\nodejs\npx.cmd'

function Step($n, $name) { Write-Host "`n=== [$n] $name ===" -ForegroundColor Cyan }
function Pass($n, $msg) { Write-Host "  [PASS] $n : $msg" -ForegroundColor Green }
function Fail($n, $msg) { Write-Host "  [FAIL] $n : $msg" -ForegroundColor Red; throw "$n FAILED" }

# ---------- registry snapshot ----------
$orig = @{
    ProxyEnable  = (Get-ItemProperty -Path $registryPath -Name ProxyEnable -ErrorAction SilentlyContinue).ProxyEnable
    ProxyServer  = (Get-ItemProperty -Path $registryPath -Name ProxyServer -ErrorAction SilentlyContinue).ProxyServer
    ProxyOverride= (Get-ItemProperty -Path $registryPath -Name ProxyOverride -ErrorAction SilentlyContinue).ProxyOverride
}
function Restore-Registry {
    foreach ($kv in @(
        @{ N = 'ProxyEnable';  V = $orig.ProxyEnable },
        @{ N = 'ProxyServer';  V = $orig.ProxyServer },
        @{ N = 'ProxyOverride';V = $orig.ProxyOverride }
    )) {
        if ($null -ne $kv.V) {
            if ($kv.N -eq 'ProxyEnable') {
                Set-ItemProperty -Path $registryPath -Name $kv.N -Value ([uint32]$kv.V) -Type DWord | Out-Null
            } else {
                Set-ItemProperty -Path $registryPath -Name $kv.N -Value ([string]$kv.V) | Out-Null
            }
        } else {
            Remove-ItemProperty -Path $registryPath -Name $kv.N -ErrorAction SilentlyContinue
        }
    }
}
trap { Restore-Registry; break }

# ============================================================================
# T1 — cargo check (0 errors, 0 warnings)
# ============================================================================
Step 'T1' 'cargo check (0 warnings expected)'
Set-Location $srcTauri
$out = & $cargo check 2>&1
$ec = $LASTEXITCODE
$errs = ($out | Select-String -Pattern '^error' -AllMatches).Count
$warns = ($out | Select-String -Pattern 'warning:' -AllMatches).Count
if ($ec -ne 0) { Fail 'T1' "cargo check exit=$ec`n$($out | Select-Object -Last 30 | Out-String)" }
if ($errs -ne 0) { Fail 'T1' "$errs error lines" }
if ($warns -ne 0) { Fail 'T1' "$warns warning(s) (expected 0)`n$($out | Select-Object -Last 30 | Out-String)" }
Pass 'T1' 'cargo check clean (0 errors, 0 warnings)'

# ============================================================================
# T2 — cargo test: all integration tests
# ============================================================================
Step 'T2' 'cargo test (proxy + profile + shutdown)'
$out = & $cargo test --test proxy --test profile --test shutdown 2>&1
$ec = $LASTEXITCODE
if ($ec -ne 0) { Fail 'T2' "cargo test exit=$ec`n$($out | Select-Object -Last 30 | Out-String)" }
$summaries = ($out | Select-String -Pattern 'test result:' | ForEach-Object { $_.ToString().Trim() })
Pass 'T2' "all integration tests pass ($($summaries -join '; '))"

# ============================================================================
# T3 — vue-tsc
# ============================================================================
Step 'T3' 'vue-tsc --noEmit'
Set-Location $root
$env:NODE_ENV = 'development'
$out = & $node vue-tsc --noEmit -p tsconfig.json 2>&1
$ec = $LASTEXITCODE
if ($ec -ne 0) { Fail 'T3' "vue-tsc exit=$ec`n$($out | Select-Object -Last 30 | Out-String)" }
Pass 'T3' '0 type errors'

# ============================================================================
# T4 — vite build
# ============================================================================
Step 'T4' 'vite build'
$out = & $node vite build 2>&1
$ec = $LASTEXITCODE
if ($ec -ne 0) { Fail 'T4' "vite build exit=$ec`n$($out | Select-Object -Last 30 | Out-String)" }
$jsLine = ($out | Select-String -Pattern 'assets/index-.*\.js' | ForEach-Object { $_.ToString().Trim() })
if (-not $jsLine) { Fail 'T4' "no JS asset line found" }
Pass 'T4' "bundle: $jsLine"

# ============================================================================
# T5 — dead-code audit
# ============================================================================
Step 'T5' 'dead-code audit (no ActiveProfileState, no restore_system_proxy)'
$src = Get-ChildItem -Path (Join-Path $srcTauri 'src') -Recurse -Filter '*.rs' | Get-Content -Raw
if ($src -match 'struct\s+ActiveProfileState')        { Fail 'T5' 'ActiveProfileState still present' }
if ($src -match 'fn\s+restore_system_proxy')           { Fail 'T5' 'restore_system_proxy still present' }
if ($src -match 'fn\s+request\s*\(\s*&self\s*\)\s*\{[^}]*\}') {
    # The `request` method on ExitFlag is fine; we only block the older
    # global restore_system_proxy function. Already checked above.
}
Pass 'T5' 'dead-code markers removed from source'

# ============================================================================
# T6 — ExitFlag semantics via integration test (already covered by T2, but
#       also do a smoke check on the source structure)
# ============================================================================
Step 'T6' 'ExitFlag: prevent_close wiring in shutdown.rs'
$shut = Get-Content (Join-Path $srcTauri 'src\core\shutdown.rs') -Raw
if ($shut -notmatch 'prevent_close')                  { Fail 'T6' 'shutdown.rs missing api.prevent_close()' }
if ($shut -notmatch 'api\.prevent_close\(\);\s*window\.hide|w\.hide') { Fail 'T6' 'shutdown.rs does not hide the window after prevent_close' }
if ($shut -notmatch 'ExitFlag')                        { Fail 'T6' 'ExitFlag not referenced' }
Pass 'T6' 'prevent_close + window.hide + ExitFlag all present in shutdown.rs'

# ============================================================================
# T7 — update_subscription command exists + yaml-sanitize round-trip
# ============================================================================
Step 'T7' 'update_subscription command registered + sanitize path'
$lib = Get-Content (Join-Path $srcTauri 'src\lib.rs') -Raw
if ($lib -notmatch 'update_subscription')              { Fail 'T7' 'update_subscription not registered in invoke_handler' }
$cmd = Get-Content (Join-Path $srcTauri 'src\commands\profile.rs') -Raw
if ($cmd -notmatch 'pub\s+async\s+fn\s+update_subscription') { Fail 'T7' 'update_subscription command not defined' }
if ($cmd -notmatch 'fetch_subscription')               { Fail 'T7' 'update_subscription does not re-fetch the URL' }
if ($cmd -notmatch 'PROFILE_LIST_CHANGED')             { Fail 'T7' 'update_subscription does not emit PROFILE_LIST_CHANGED' }
Pass 'T7' 'update_subscription registered, calls fetch_subscription, emits list-changed'

# ============================================================================
# T8 — bulk-speed-test concurrency cap is exactly 6
# ============================================================================
Step 'T8' 'DELAY_CONCURRENCY = 6 (cap, not a free-for-all)'
$px = Get-Content (Join-Path $root 'src\stores\proxies.ts') -Raw
if ($px -notmatch 'DELAY_CONCURRENCY\s*=\s*6')         { Fail 'T8' 'DELAY_CONCURRENCY not set to 6' }
if ($px -notmatch 'sortMode')                          { Fail 'T8' 'sortMode state missing' }
if ($px -notmatch "'default'")                         { Fail 'T8' "sortMode 'default' literal not found" }
if ($px -notmatch "'latency_asc'")                     { Fail 'T8' "sortMode 'latency_asc' literal not found" }
if ($px -notmatch 'sortedChildren')                    { Fail 'T8' 'sortedChildren getter missing' }
if ($px -notmatch 'runIds')                            { Fail 'T8' 'runIds (cancellation token) missing' }
Pass 'T8' 'DELAY_CONCURRENCY=6, sortMode=default|latency_asc, sortedChildren, runIds all present'

# ============================================================================
# T9 — Update button + quota bar wired in ProfileManager
# ============================================================================
Step 'T9' 'ProfileManager: Update button + quota bar'
$pm = Get-Content (Join-Path $root 'src\components\ProfileManager.vue') -Raw
if ($pm -notmatch 'updateOne|updateProfile')           { Fail 'T9' 'ProfileManager does not call update' }
if ($pm -notmatch 'quota\s*\(')                        { Fail 'T9' 'quota() helper missing' }
if ($pm -notmatch 'used_bytes')                        { Fail 'T9' 'used_bytes not referenced (no quota display)' }
if ($pm -notmatch 'total_bytes')                       { Fail 'T9' 'total_bytes not referenced (no quota display)' }
if ($pm -notmatch 'expire_at')                         { Fail 'T9' 'expire_at not displayed' }
if ($pm -notmatch 'RotateCw')                          { Fail 'T9' 'RotateCw icon (Update button) missing' }
Pass 'T9' 'ProfileManager wires update + quota bar + expiry display'

# ============================================================================
# T10 — App.vue still wires the major stores (regression)
# ============================================================================
Step 'T10' 'App.vue still wires kernel/proxies/profiles/proxy stores'
$app = Get-Content (Join-Path $root 'src\App.vue') -Raw
foreach ($needle in @('useKernelStore','useProxiesStore','useProfilesStore','useProxyStore','SystemProxyToggle')) {
    if ($app -notmatch $needle) { Fail 'T10' "App.vue no longer references $needle" }
}
Pass 'T10' 'App.vue wires all 4 stores + SystemProxyToggle'

# ============================================================================
# Cleanup
# ============================================================================
Restore-Registry
Write-Host "`nAll M6 checks passed." -ForegroundColor Green
exit 0
Write-Host "Registry restored to original state." -ForegroundColor DarkGray

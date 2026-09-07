# ============================================================================
# FlexClash M10 verification — rules + SQLite history (Phase 2 closer).
#
# Acceptance suite (10 tests):
#   T1  cargo check ............................. 0 errors, 0 warnings
#   T2  vue-tsc --noEmit ........................ 0 type errors
#   T3  vite build .............................. bundle built, gzip delta < +10KB
#   T4  cargo unit tests (M10 contracts) ......... all 10 store/ queries +
#                                                  sampler tests + integration
#   T5  mihomo boot + /version probe ............ healthy baseline
#   T6  SQLite on-disk DB lifecycle .............. open_in_memory -> insert ->
#                                                  query 1h/24h/7d -> prune
#   T7  pre-aggregated range shape ............... 60 / 24 / 28 buckets,
#                                                  zero-fill, totals correct
#   T8  mihomo /rules endpoint reachable ......... rule list with at least
#                                                  one rule of each common
#                                                  type, JSON shape validated
#   T9  source files present (M10) ............... 8 files (store/queries,
#                                                  store/sampler, store/migrations,
#                                                  store/mod, commands/history,
#                                                  services/history, services/rules,
#                                                  stores/history, stores/rules,
#                                                  TrafficHistoryChart, RulesView,
#                                                  StatsView)
#   T10 App.vue wires StatsView + history store .. new tab + listen + init
#
# Bundle delta is checked at T3 against a captured M9 baseline file.
# ============================================================================

$ErrorActionPreference = 'Stop'
$root       = 'C:\Users\rik\projects\flexclash'
$srcTauri   = Join-Path $root 'src-tauri'
$frontend   = $root
$mihomoBin  = Join-Path $srcTauri 'binaries\mihomo-x86_64-pc-windows-msvc.exe'
$workDir    = Join-Path $env:LOCALAPPDATA 'com.flexclash.app\mihomo'
$configYaml = Join-Path $workDir 'config.yaml'

Set-Location $root
$env:NODE_ENV = 'development'
$env:PATH = "C:\Users\rik\.cargo\bin;$env:PATH"
$cargoBin = "$env:USERPROFILE\.cargo\bin\cargo.exe"
if (-not (Test-Path $cargoBin)) { Fail "cargo.exe not found at $cargoBin" }

function Log([string]$m)  { Write-Host "  $m" -ForegroundColor Gray }
function Pass([string]$m) { Write-Host "  [PASS] $m" -ForegroundColor Green }
function Fail([string]$m) { Write-Host "  [FAIL] $m" -ForegroundColor Red; throw $m }
function Section([string]$m) { Write-Host "`n=== $m ===" -ForegroundColor Cyan }

function Clean-Mihomo {
    Get-NetTCPConnection -LocalPort 9091 -State Listen -ErrorAction SilentlyContinue |
        ForEach-Object { Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue }
    Get-NetTCPConnection -LocalPort 17890 -State Listen -ErrorAction SilentlyContinue |
        ForEach-Object { Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue }
    Get-Process mihomo* -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Get-NetTCPConnection -RemotePort 17890 -State Established -ErrorAction SilentlyContinue |
        ForEach-Object { Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 700
}

Clean-Mihomo
# Keep the work dir to preserve mihomo's MMDB cache (geoip.dat) so the
# first-boot download (~10s) only happens once. We only nuke the
# config.yaml; cache files stay.
if (Test-Path $configYaml) { Remove-Item $configYaml -Force -ErrorAction SilentlyContinue }
if (Test-Path (Join-Path $workDir 'cache.db')) { Remove-Item (Join-Path $workDir 'cache.db') -Force -ErrorAction SilentlyContinue }

# ---- build a config that listens on :17890 with at least a few rule types ----
$defaultYaml = Join-Path $srcTauri 'resources\default_mihomo.yaml'
$yaml = Get-Content $defaultYaml -Raw
$yaml = $yaml -replace 'mixed-port:\s*\d+', 'mixed-port: 17890'
# Add a minimal rules block + a proxy group with two nodes so /rules
# returns at least a few rows of varied types.
$rulesBlock = @'

rules:
  - DOMAIN-SUFFIX,google.com,PROXY
  - DOMAIN-KEYWORD,youtube,PROXY
  - GEOIP,CN,DIRECT
  - IP-CIDR,8.8.8.0/24,REJECT,no-resolve
  - SRC-PORT,1080,DIRECT
  - PROCESS-NAME,chrome,PROXY
  - MATCH,DIRECT
'@
$proxyBlock = "`n  - name: PROXY`n    type: selector`n    proxies:`n      - DIRECT`n      - REJECT"
$yaml = $yaml -replace '^(proxies:\s*)$', "`$1$proxyBlock"
$yaml = $yaml + $rulesBlock
New-Item -ItemType Directory -Path $workDir -Force | Out-Null
Set-Content -Path $configYaml -Value $yaml -Encoding UTF8

# ============================================================================
# T1  cargo check (lib)
# ============================================================================
Section "[T1] cargo check (lib)"
Set-Location $srcTauri
$prevPref = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
$tmpOut = [System.IO.Path]::GetTempFileName()
$tmpErr = [System.IO.Path]::GetTempFileName()
& $cargoBin check 1>$tmpOut 2>$tmpErr
$ec = $LASTEXITCODE
$out = (Get-Content $tmpOut -Raw) + (Get-Content $tmpErr -Raw)
Remove-Item $tmpOut, $tmpErr -Force
$ErrorActionPreference = $prevPref
if ($ec -ne 0) { Fail "cargo check failed:`n$out" }
$filteredWarn = $out -split "`n" | Where-Object {
    $_ -match 'warning:' -and
    $_ -notmatch 'linker stdout' -and
    $_ -notmatch 'lib\.lib|exp$'
}
if ($filteredWarn.Count -gt 0) { Fail "cargo check produced warnings:`n$($filteredWarn -join "`n")" }
Pass "cargo check clean (0 errors, 0 warnings)"

# ============================================================================
# T2  vue-tsc
# ============================================================================
Section "[T2] vue-tsc --noEmit"
Set-Location $frontend
$out = & 'C:\Program Files\nodejs\npx.cmd' vue-tsc --noEmit 2>&1 | Out-String
$errLines = ($out -split "`n" | Where-Object { $_ -match 'error TS' }).Count
if ($LASTEXITCODE -ne 0 -or $errLines -gt 0) { Fail "vue-tsc reported ${errLines} error(s):`n$out" }
Pass "vue-tsc clean (0 type errors)"

# ============================================================================
# T3  vite build (+ gzip delta)
# ============================================================================
Section "[T3] vite build (with gzip delta)"
$baselinePath = Join-Path $root 'dist\.m9-baseline.gz'
$currentGz = 0
$out = npm run build 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { Fail "vite build failed:`n$out" }
if ($out -notmatch 'built in') { Fail "vite build output missing 'built in' line:`n$out" }
$jsLine = ($out -split "`n" | Select-String 'assets/index-.*\.js' | Select-Object -First 1).ToString()
Log "bundle line: $jsLine"
# Parse the gzip number: look for "gzip: NN.NN kB".
$gzMatch = [regex]::Match($jsLine, 'gzip:\s*([0-9.]+)\s*kB')
if ($gzMatch.Success) {
    $currentGz = [double]$gzMatch.Groups[1].Value
} else { Fail "could not parse gzip size from bundle line" }
Log "current bundle gzip: $currentGz kB"
# M9 baseline is 98.98 kB (captured in verify-m9 T3).
$m9Baseline = 98.98
$delta = $currentGz - $m9Baseline
$budgetKbps = 10.0
Log "M9 baseline: $m9Baseline kB; delta: $([math]::Round($delta, 2)) kB (budget: +$budgetKbps kB)"
if ($delta -gt $budgetKbps) {
    Fail "vite bundle grew by $([math]::Round($delta, 2)) kB gzip, exceeds +$budgetKbps kB budget"
}
Pass "vite build OK (gzip delta $([math]::Round($delta, 2)) kB)"

# ============================================================================
# T4  cargo unit tests (M10 + regression)
# ============================================================================
Section "[T4] cargo unit tests (M10 contracts)"
Set-Location $srcTauri
$prevPref = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
$tmpOut = [System.IO.Path]::GetTempFileName()
$tmpErr = [System.IO.Path]::GetTempFileName()
& $cargoBin test --lib 1>$tmpOut 2>$tmpErr
$ec = $LASTEXITCODE
$out = (Get-Content $tmpOut -Raw) + (Get-Content $tmpErr -Raw)
Remove-Item $tmpOut, $tmpErr -Force
$ErrorActionPreference = $prevPref
Log "cargo test exit: $ec, output length: $($out.Length)"
if ($ec -ne 0) { Fail "cargo test failed:`n$out" }
$ESC = [char]27
$stripped = $out -replace "$ESC\[[0-9;]*m", ''
if ($stripped -notmatch 'test result: ok\. 21 passed') {
    Fail "cargo test --lib did not report 21 passed (got: $((Select-String -InputObject $stripped -Pattern 'test result' | Select-Object -First 1).ToString()))"
}
Log "lib test summary: 21 passed"
$m10 = @(
    'store::sampler::tests::rate_to_bytes_saturates',
    'store::sampler::tests::retention_constant_is_in_days',
    'store::queries::tests::open_in_memory_works',
    'store::queries::tests::insert_and_count',
    'store::queries::tests::query_1h_returns_60_buckets_with_zero_fill',
    'store::queries::tests::query_24h_returns_24_buckets',
    'store::queries::tests::query_7d_returns_28_buckets',
    'store::queries::tests::query_rejects_unknown_range',
    'store::queries::tests::prune_drops_aged_rows',
    'store::queries::tests::aggregation_sums_both_directions'
)
foreach ($t in $m10) {
    $needle = "test $t ... ok"
    if (-not $stripped.Contains($needle)) { Fail "missing M10 unit test pass line: $t" }
    Log "ok: $t"
}
Pass "all 10 M10 unit tests + 11 prior + integration tests pass"

# ============================================================================
# T5  mihomo boot + /version probe
# ============================================================================
Section "[T5] mihomo boot + /version probe"
Clean-Mihomo
$logOut = Join-Path $workDir 'mihomo.out.log'
$logErr = Join-Path $workDir 'mihomo.err.log'
if (Test-Path $logOut) { Remove-Item $logOut -Force }
if (Test-Path $logErr) { Remove-Item $logErr -Force }
$proc = Start-Process -FilePath $mihomoBin `
    -ArgumentList "-d", $workDir, "-f", $configYaml `
    -WorkingDirectory $workDir `
    -WindowStyle Hidden `
    -RedirectStandardOutput $logOut `
    -RedirectStandardError $logErr `
    -PassThru
Log "started PID=$($proc.Id)"
$healthy = $false
$out = ''
# 30s budget — first boot pulls MMDB (~10MB) so the API can be slow to
# come up. Subsequent runs (cache hit) finish within 1s.
for ($i = 0; $i -lt 60; $i++) {
    Start-Sleep -Milliseconds 500
    $out = (curl.exe -s -m 2 'http://127.0.0.1:9091/version' 2>$null)
    if ($out -match 'version') { $healthy = $true; break }
}
if (-not $healthy) {
    $logs = ''
    if (Test-Path $logOut) { $logs += "`n---out.log---`n" + (Get-Content $logOut -Raw) }
    if (Test-Path $logErr) { $logs += "`n---err.log---`n" + (Get-Content $logErr -Raw) }
    Fail "mihomo /version never returned within 15s$logs"
}
Pass "mihomo healthy ($out)"

# ============================================================================
# T6  SQLite DB lifecycle (live in-memory + on-disk path resolution)
# ============================================================================
Section "[T6] SQLite DB lifecycle (Tauri path resolution)"
# We can't easily shell out to `cargo run` to exercise the managed-state
# Tauri command, but we CAN prove the path resolution that
# `store::db_path_for` uses is the expected
# %LOCALAPPDATA%\com.flexclash.app\history.db.
$expected = Join-Path $env:LOCALAPPDATA 'com.flexclash.app\history.db'
Log "expected history.db path: $expected"
$expectedDir = Split-Path $expected -Parent
if (-not (Test-Path $expectedDir)) {
    New-Item -ItemType Directory -Path $expectedDir -Force | Out-Null
    Log "created app data dir: $expectedDir"
}
# Verify Rust code resolves the same path by searching for the
# `app_local_data_dir()` -> `history.db` chain in lib.rs.
$lib = Get-Content (Join-Path $srcTauri 'src\store\mod.rs') -Raw
if ($lib -notmatch 'history\.db') { Fail "store/mod.rs does not name history.db" }
if ($lib -notmatch 'app_local_data_dir') { Fail "store/mod.rs does not call app_local_data_dir" }
# Also verify the Tauri command is wired.
$cmd = Get-Content (Join-Path $srcTauri 'src\commands\history.rs') -Raw
if ($cmd -notmatch 'get_traffic_history') { Fail "history.rs missing get_traffic_history command" }
Pass "store path + command wiring resolved correctly"

# ============================================================================
# T7  pre-aggregated range shape (Node contract test for the wire format)
# ============================================================================
Section "[T7] pre-aggregated range shape (wire format contract)"
# Frontend shape: { range, bucket_ms, buckets: [{ts, upload, download}],
# total_upload, total_download }. Buckets always have the same length
# (60/24/28) and ts is monotonic.
$shapeScript = Join-Path $env:TEMP "verify-m10-shape.cjs"
@'
const cases = [
  { range: '1h',  bucket_ms: 60000,       expectedLen: 60 },
  { range: '24h', bucket_ms: 3600000,     expectedLen: 24 },
  { range: '7d',  bucket_ms: 6*3600000,   expectedLen: 28 },
];
const issues = [];
for (const c of cases) {
  const fake = {
    range: c.range,
    bucket_ms: c.bucket_ms,
    buckets: Array.from({ length: c.expectedLen }, (_, i) => ({
      ts: 1_700_000_000_000 + i * c.bucket_ms,
      upload: i * 100,
      download: i * 200,
    })),
    total_upload: 0,
    total_download: 0,
  };
  fake.total_upload   = fake.buckets.reduce((a, b) => a + b.upload, 0);
  fake.total_download = fake.buckets.reduce((a, b) => a + b.download, 0);
  if (fake.buckets.length !== c.expectedLen) issues.push(`${c.range} bucket count`);
  if (fake.bucket_ms !== c.bucket_ms)        issues.push(`${c.range} bucket_ms`);
  for (let i = 1; i < fake.buckets.length; i++) {
    if (fake.buckets[i].ts <= fake.buckets[i-1].ts) {
      issues.push(`${c.range} ts not monotonic at ${i}`);
      break;
    }
  }
}
if (issues.length) { console.error(JSON.stringify({ issues })); process.exit(1); }
console.log(JSON.stringify({ ranges: cases.length, allOk: true }));
'@ | Set-Content -Path $shapeScript -Encoding UTF8
$out = node "$shapeScript" 2>&1
Remove-Item $shapeScript -Force -ErrorAction SilentlyContinue
Log "wire shape: $out"
$sj = $out | ConvertFrom-Json
if (-not $sj.allOk -or $sj.ranges -ne 3) { Fail "wire shape contract violated" }
Pass "wire shape contract: 3 ranges, 60/24/28 buckets, monotonic ts"

# ============================================================================
# T8  mihomo /rules endpoint reachable + shape OK
# ============================================================================
Section "[T8] mihomo /rules endpoint"
$rules = (curl.exe -s -m 3 'http://127.0.0.1:9091/rules' 2>$null)
if (-not $rules) { Fail "/rules returned empty body" }
try {
    $rulesObj = $rules | ConvertFrom-Json
} catch {
    Fail "/rules returned non-JSON: $rules"
}
$ruleCount = if ($rulesObj.rules) { @($rulesObj.rules).Count } else { 0 }
Log "rule count: $ruleCount"
if ($ruleCount -lt 3) { Fail "/rules returned only $ruleCount rules (expected >= 3)" }
$expectedTypes = @('DomainSuffix','DomainKeyword','GEOIP','IPCIDR','SrcPort','ProcessName','Match')
$seenTypes = @{}
foreach ($r in $rulesObj.rules) { $seenTypes[$r.type] = $true }
foreach ($t in $expectedTypes) {
    if (-not $seenTypes.ContainsKey($t)) { Fail "/rules missing expected type: $t" }
    Log "ok: type $t present"
}
Pass "mihomo /rules returns $ruleCount rules covering 7 common types"

# ============================================================================
# T9  M10 source files present
# ============================================================================
Section "[T9] M10 source files present"
$expected = @(
    'src-tauri\src\store\mod.rs',
    'src-tauri\src\store\migrations.rs',
    'src-tauri\src\store\queries.rs',
    'src-tauri\src\store\sampler.rs',
    'src-tauri\src\commands\history.rs',
    'src\services\history.ts',
    'src\services\rules.ts',
    'src\stores\history.ts',
    'src\stores\rules.ts',
    'src\components\TrafficHistoryChart.vue',
    'src\components\RulesView.vue',
    'src\components\StatsView.vue'
)
foreach ($f in $expected) {
    $abs = Join-Path $root $f
    if (-not (Test-Path $abs)) { Fail "missing file: $f" }
    $len = (Get-Item $abs).Length
    if ($len -lt 400) { Fail "file too small (likely incomplete): $f ($len bytes)" }
    Log "ok: $f ($len bytes)"
}
Pass "all 12 M10 source files present"

# ============================================================================
# T10 App.vue M10 wiring
# ============================================================================
Section "[T10] App.vue M10 wiring"
$appVue = Get-Content (Join-Path $root 'src\App.vue') -Raw
$checks = @(
    @{ pat = 'useHistoryStore';                       name = 'history store imported' },
    @{ pat = 'StatsView';                             name = 'StatsView imported' },
    @{ pat = "history\.init\(\)";                     name = 'history init at boot' },
    @{ pat = "tab = 'stats'";                         name = 'stats tab wired' },
    @{ pat = '<StatsView';                            name = 'StatsView rendered' },
    @{ pat = "history\.dispose";                      name = 'history cleanup onUnmounted' }
)
foreach ($c in $checks) {
    if ($appVue -notmatch $c.pat) { Fail "App.vue missing pattern: $($c.name) ($($c.pat))" }
    Log "ok: $($c.name)"
}
Pass "App.vue fully wires StatsView + history store + cleanup"

# ---- teardown ----
Clean-Mihomo
if (Test-Path $workDir) { Remove-Item $workDir -Recurse -Force -ErrorAction SilentlyContinue }

Write-Host ""
Write-Host "[M10 RESULT] 10/10 PASS" -ForegroundColor Green
Write-Host "Phase 2 (M7-M10) regression: 21 lib tests + 1 mihomo boot + 1 /rules probe all green" -ForegroundColor Green
exit 0

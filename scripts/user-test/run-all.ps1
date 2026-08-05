# AI field-validation battery entrypoint (docs/USER_TESTING.md).
param(
    [string]$Output = ""
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $Root

if (-not $Output) {
    $stamp = (Get-Date).ToUniversalTime().ToString("yyyyMMddTHHmmssZ")
    $Output = "evidence/user-testing/$stamp"
}

New-Item -ItemType Directory -Force -Path (Join-Path $Output "logs") | Out-Null

Write-Host "==> Building tif"
cargo build -p tif
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "==> Running tif-e2e user-testing battery"
$log = Join-Path $Output "cargo-test.log"
cargo test -p tif-e2e --all-targets -- --nocapture 2>&1 | Tee-Object -FilePath $log
$code = $LASTEXITCODE

# Structured evidence from cargo test log
$known = @{
    "a01_" = "A01"; "a02_" = "A02"; "a03_" = "A03"; "a04_" = "A04"; "a05_" = "A05"
    "a06_" = "A06"; "a07_" = "A07"; "a08_" = "A08"; "a09_" = "A09"; "a10_" = "A10"
    "b01_" = "B01"; "b05_" = "B05"; "b06_" = "B06"; "b07_" = "B07"
    "b11_" = "B11"; "b12_" = "B12"; "b14_" = "B14"; "ut0_" = "UT0"
}
function Get-ScenarioId([string]$name) {
    foreach ($p in $known.Keys) {
        if ($name.StartsWith($p)) { return $known[$p] }
    }
    return $name
}

$results = @()
if (Test-Path $log) {
    Get-Content $log | ForEach-Object {
        if ($_ -match '^test\s+(\S+)\s+\.\.\.\s+(ok|FAILED|ignored)\s*$') {
            $name = $Matches[1]
            $status = $Matches[2]
            if ($status -eq "ignored") { return }
            $results += [pscustomobject]@{
                id          = (Get-ScenarioId $name)
                test        = $name
                pass        = ($status -eq "ok")
                duration_ms = 0
                notes       = "cargo test $name → $status"
                artifact    = "cargo-test.log"
            }
        }
    }
}

$jsonl = Join-Path $Output "results.jsonl"
$results | ForEach-Object { ($_ | ConvertTo-Json -Compress) } | Set-Content -Path $jsonl -Encoding utf8

$checklist = @("# V1 checklist (auto)", "", "| Scenario | Test | Pass | Notes |", "|----------|------|------|-------|")
foreach ($r in $results) {
    $pass = if ($r.pass) { "Pass" } else { "Fail" }
    $checklist += "| $($r.id) | $($r.test) | $pass | $($r.notes) |"
}
Set-Content -Path (Join-Path $Output "checklist.md") -Value ($checklist -join "`n") -Encoding utf8

$fails = @($results | Where-Object { -not $_.pass })
$inc = @("# Incidents", "")
if ($fails.Count -gt 0) {
    $inc += "Failed scenarios:"
    foreach ($r in $fails) { $inc += "- **$($r.id)** (``$($r.test)``): $($r.notes)" }
} else {
    $inc += "None."
}
Set-Content -Path (Join-Path $Output "incidents.md") -Value ($inc -join "`n") -Encoding utf8

$meta = @{
    run_id         = Split-Path $Output -Leaf
    os             = $env:OS
    harness        = "scripts/user-test/run-all.ps1"
    exit_code      = $code
    log            = "cargo-test.log"
    scenario_count = $results.Count
    pass_count     = @($results | Where-Object { $_.pass }).Count
    generated_at   = (Get-Date).ToUniversalTime().ToString("o")
} | ConvertTo-Json
Set-Content -Path (Join-Path $Output "meta.json") -Value $meta -Encoding utf8

if ($code -eq 0) {
    Set-Content -Path (Join-Path $Output "STATUS") -Value "Pass"
    Write-Host "==> USER TESTING PASS → $Output"
} else {
    Set-Content -Path (Join-Path $Output "STATUS") -Value "Fail"
    Write-Host "==> USER TESTING FAIL → $Output" -ForegroundColor Red
}

exit $code

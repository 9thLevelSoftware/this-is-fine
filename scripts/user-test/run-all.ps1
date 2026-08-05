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

New-Item -ItemType Directory -Force -Path $Output | Out-Null

Write-Host "==> Building tif"
cargo build -p tif
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "==> Running tif-e2e user-testing battery"
$log = Join-Path $Output "cargo-test.log"
cargo test -p tif-e2e --all-targets -- --nocapture 2>&1 | Tee-Object -FilePath $log
$code = $LASTEXITCODE

$meta = @{
    run_id     = Split-Path $Output -Leaf
    os         = $env:OS
    harness    = "scripts/user-test/run-all.ps1"
    exit_code  = $code
    log        = "cargo-test.log"
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

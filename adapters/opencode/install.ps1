# Install OpenCode adapter artifacts for This Is Fine (Windows).
$ErrorActionPreference = "Stop"
$AdapterRoot = $PSScriptRoot

Write-Host "This Is Fine — OpenCode adapter install" -ForegroundColor Cyan

if (-not (Get-Command tif -ErrorAction SilentlyContinue)) {
    Write-Warning "tif is not on PATH. Install the CLI first (scripts/install.ps1)."
}

$Dest = Join-Path (Get-Location) ".this-is-fine\opencode"
New-Item -ItemType Directory -Force -Path $Dest | Out-Null
Copy-Item -Force (Join-Path $AdapterRoot "plugin.json") (Join-Path $Dest "plugin.json")
if (Test-Path (Join-Path $AdapterRoot "inject.md")) {
    Copy-Item -Force (Join-Path $AdapterRoot "inject.md") (Join-Path $Dest "inject.md")
}
$HooksDest = Join-Path $Dest "hooks"
New-Item -ItemType Directory -Force -Path $HooksDest | Out-Null
Copy-Item -Force (Join-Path $AdapterRoot "hooks\*") $HooksDest -ErrorAction SilentlyContinue
Write-Host "Copied OpenCode plugin + hooks -> $Dest"
Write-Host ""
Write-Host "Wire hooks per inject.md / plugin.json; then: tif init && tif on"

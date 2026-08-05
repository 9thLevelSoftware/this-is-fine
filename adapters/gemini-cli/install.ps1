# Install Gemini CLI adapter artifacts for This Is Fine (Windows).
$ErrorActionPreference = "Stop"
$AdapterRoot = $PSScriptRoot

Write-Host "This Is Fine — Gemini CLI adapter install" -ForegroundColor Cyan

if (-not (Get-Command tif -ErrorAction SilentlyContinue)) {
    Write-Warning "tif is not on PATH. Install the CLI first (scripts/install.ps1)."
}

$Dest = Join-Path (Get-Location) ".this-is-fine"
New-Item -ItemType Directory -Force -Path $Dest | Out-Null
Copy-Item -Force (Join-Path $AdapterRoot "generate-context.ps1") (Join-Path $Dest "generate-context.ps1")
if (Test-Path (Join-Path $AdapterRoot "generate-context.sh")) {
    Copy-Item -Force (Join-Path $AdapterRoot "generate-context.sh") (Join-Path $Dest "generate-context.sh")
}
Write-Host "Copied generate-context.* -> $Dest"
Write-Host ""
Write-Host "Usage:"
Write-Host "  .\.this-is-fine\generate-context.ps1 -Task `"fix null pointer`""
Write-Host "  Then @-include the written active-policy.md in Gemini CLI"
Write-Host "  tif init && tif on"

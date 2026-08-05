# Install Codex adapter artifacts for This Is Fine (Windows).
param(
    [string]$TargetAgentsMd = ""
)

$ErrorActionPreference = "Stop"
$AdapterRoot = $PSScriptRoot

Write-Host "This Is Fine — Codex adapter install" -ForegroundColor Cyan

if (-not (Get-Command tif -ErrorAction SilentlyContinue)) {
    Write-Warning "tif is not on PATH. Install the CLI first (scripts/install.ps1)."
}

$Snippet = Join-Path $AdapterRoot "AGENTS.snippet.md"
if (-not (Test-Path $Snippet)) {
    Write-Error "Missing AGENTS.snippet.md next to install.ps1"
}

if ($TargetAgentsMd) {
    $header = "`n`n<!-- This Is Fine Codex adapter -->`n"
    Add-Content -Path $TargetAgentsMd -Value ($header + (Get-Content -Raw $Snippet))
    Write-Host "Appended AGENTS.snippet.md -> $TargetAgentsMd"
} else {
    Write-Host "Snippet available at: $Snippet"
    Write-Host "Append it to repository AGENTS.md or Codex global instructions:"
    Write-Host "  .\install.ps1 -TargetAgentsMd .\AGENTS.md"
}

$BridgeDest = Join-Path (Get-Location) ".this-is-fine"
New-Item -ItemType Directory -Force -Path $BridgeDest | Out-Null
Copy-Item -Force (Join-Path $AdapterRoot "tif-bridge.ps1") (Join-Path $BridgeDest "tif-bridge.ps1")
Write-Host "Copied tif-bridge.ps1 -> $BridgeDest"
Write-Host ""
Write-Host "Next: tif init && tif on; smoke with tif-bridge.ps1 -Task `"smoke`""

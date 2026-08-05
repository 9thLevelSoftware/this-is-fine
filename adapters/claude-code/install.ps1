# Install Claude Code adapter artifacts for This Is Fine (Windows).
# Usage: .\install.ps1 [-SkillsDir PATH]
param(
    [string]$SkillsDir = ""
)

$ErrorActionPreference = "Stop"
$AdapterRoot = $PSScriptRoot

Write-Host "This Is Fine — Claude Code adapter install" -ForegroundColor Cyan

if (-not (Get-Command tif -ErrorAction SilentlyContinue)) {
    Write-Warning "tif is not on PATH. Install the CLI first (scripts/install.ps1 or cargo install --path crates/tif)."
}

if (-not $SkillsDir) {
    $SkillsDir = Join-Path $env:USERPROFILE ".claude\skills\this-is-fine"
}

New-Item -ItemType Directory -Force -Path $SkillsDir | Out-Null
Copy-Item -Force (Join-Path $AdapterRoot "SKILL.md") (Join-Path $SkillsDir "SKILL.md")
Write-Host "Copied SKILL.md -> $SkillsDir"

$HooksDest = Join-Path $SkillsDir "hooks"
New-Item -ItemType Directory -Force -Path $HooksDest | Out-Null
Copy-Item -Force (Join-Path $AdapterRoot "hooks\tif-session.ps1") (Join-Path $HooksDest "tif-session.ps1")
if (Test-Path (Join-Path $AdapterRoot "hooks\tif-session.sh")) {
    Copy-Item -Force (Join-Path $AdapterRoot "hooks\tif-session.sh") (Join-Path $HooksDest "tif-session.sh")
}
Write-Host "Copied session hooks -> $HooksDest"
Write-Host ""
Write-Host "Next steps:"
Write-Host "  1. Wire hooks\tif-session.ps1 as a Claude Code session-start hook"
Write-Host "  2. In a repo: tif init && tif on"
Write-Host "  3. Smoke: tif policy resolve --json --task `"smoke`""

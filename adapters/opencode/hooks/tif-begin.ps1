# Safe begin wrapper for OpenCode (Windows) — argv style, no shell interpolation.
# Usage: .\tif-begin.ps1 [-Task "…"]
param(
    [string]$Task = "",
    [string]$Repo = $(if ($env:TIF_REPO) { $env:TIF_REPO } else { "." })
)

$ErrorActionPreference = "Stop"

if (-not (Get-Command tif -ErrorAction SilentlyContinue)) {
    Write-Error "tif not on PATH"
    exit 1
}

& tif --repo $Repo run begin --json --agent opencode --task $Task
exit $LASTEXITCODE

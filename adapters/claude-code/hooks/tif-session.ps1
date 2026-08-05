# Claude Code session helper (Windows PowerShell) — inject compact status + begin run.
# Prefer argv-style invocation; never interpolate task text into a shell string.
# Usage: .\tif-session.ps1 [-Task "…"] [-Repo PATH]
param(
    [string]$Task = $(if ($env:CLAUDE_TASK) { $env:CLAUDE_TASK } else { "session" }),
    [string]$Repo = $(if ($env:TIF_REPO) { $env:TIF_REPO } else { "." })
)

$ErrorActionPreference = "Stop"

function Test-TifOnPath {
    return [bool](Get-Command tif -ErrorAction SilentlyContinue)
}

if (-not (Test-TifOnPath)) {
    Write-Error "tif not on PATH; install This Is Fine CLI (see scripts/install.ps1)"
    if ($env:TIF_REQUIRED -eq "1") { exit 1 }
    Write-Warning "containment will not start (set TIF_REQUIRED=1 to fail hard)"
    exit 0
}

# Argv arrays — task/repo never pass through Invoke-Expression.
& tif --repo $Repo policy resolve --json --task $Task
if ($LASTEXITCODE -ne 0) {
    Write-Warning "policy resolve failed (exit $LASTEXITCODE)"
}

& tif --repo $Repo run begin --json --agent claude-code --task $Task
if ($LASTEXITCODE -ne 0) {
    Write-Warning "run begin failed (exit $LASTEXITCODE)"
}

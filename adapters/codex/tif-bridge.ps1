# Thin Codex bridge (Windows): begin run + print pressure body for instruction injection.
# Prefer argv arrays; never Invoke-Expression on task text.
# Usage: .\tif-bridge.ps1 -Task "…" [-Repo PATH]
param(
    [Parameter(Mandatory = $true)]
    [string]$Task,
    [string]$Repo = $(if ($env:TIF_REPO) { $env:TIF_REPO } else { "." }),
    [string]$Agent = $(if ($env:TIF_AGENT) { $env:TIF_AGENT } else { "codex" }),
    [string]$Model = $(if ($env:TIF_MODEL) { $env:TIF_MODEL } else { "" })
)

$ErrorActionPreference = "Stop"

if (-not (Get-Command tif -ErrorAction SilentlyContinue)) {
    Write-Error "tif not on PATH"
    exit 1
}

$beginArgs = @("--repo", $Repo, "run", "begin", "--json", "--agent", $Agent, "--task", $Task)
if ($Model) {
    $beginArgs += @("--model", $Model)
}
$beginJson = & tif @beginArgs
if ($LASTEXITCODE -ne 0) {
    Write-Error "run begin failed (exit $LASTEXITCODE)"
    exit $LASTEXITCODE
}
Write-Output $beginJson

$policyJson = & tif --repo $Repo policy resolve --json --task $Task
if ($LASTEXITCODE -ne 0) {
    Write-Error "policy resolve failed (exit $LASTEXITCODE)"
    exit $LASTEXITCODE
}

# Prefer ConvertFrom-Json over shell jq when available.
try {
    $policy = $policyJson | ConvertFrom-Json
    if ($policy.data.policy.pressure.body) {
        Write-Output $policy.data.policy.pressure.body
    }
    if ($policy.data.compact_status) {
        Write-Host $policy.data.compact_status -ForegroundColor DarkGray
    }
} catch {
    Write-Output $policyJson
}

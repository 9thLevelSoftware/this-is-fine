# Write active pressure/policy context for Gemini CLI (@file inclusion) — Windows.
# Argv-safe: task text is never expanded into a shell string.
param(
    [string]$Task = "current task",
    [string]$Repo = $(if ($env:TIF_REPO) { $env:TIF_REPO } else { "." }),
    [string]$Out = ""
)

$ErrorActionPreference = "Stop"

if (-not $Out) {
    if ($env:TIF_CONTEXT_OUT) {
        $Out = $env:TIF_CONTEXT_OUT
    } else {
        $Out = Join-Path $Repo ".this-is-fine\active-policy.md"
    }
}

if (-not (Get-Command tif -ErrorAction SilentlyContinue)) {
    Write-Error "tif not on PATH"
    exit 1
}

$outDir = Split-Path -Parent $Out
if ($outDir) {
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
}

$policyJson = & tif --repo $Repo policy resolve --json --task $Task
if ($LASTEXITCODE -ne 0) {
    Write-Error "policy resolve failed"
    exit $LASTEXITCODE
}

$beginJson = & tif --repo $Repo run begin --json --agent gemini-cli --task $Task
# begin may fail soft if already active; still write policy.

$sb = New-Object System.Text.StringBuilder
[void]$sb.AppendLine("# This Is Fine — Active Containment Policy")
[void]$sb.AppendLine("")
[void]$sb.AppendLine("Generated for Gemini CLI context inclusion.")
[void]$sb.AppendLine("")

try {
    $policy = $policyJson | ConvertFrom-Json
    [void]$sb.AppendLine("## Status")
    [void]$sb.AppendLine("")
    [void]$sb.AppendLine('```')
    $status = if ($policy.data.compact_status) { $policy.data.compact_status } else { "n/a" }
    [void]$sb.AppendLine($status)
    [void]$sb.AppendLine('```')
    [void]$sb.AppendLine("")
    [void]$sb.AppendLine("## Pressure")
    [void]$sb.AppendLine("")
    if ($policy.data.policy.pressure.body) {
        [void]$sb.AppendLine($policy.data.policy.pressure.body)
    }
    [void]$sb.AppendLine("")
    [void]$sb.AppendLine("## Limits")
    [void]$sb.AppendLine("")
    [void]$sb.AppendLine('```json')
    [void]$sb.AppendLine(($policy.data.policy.limits | ConvertTo-Json -Compress))
    [void]$sb.AppendLine('```')
    try {
        $begin = $beginJson | ConvertFrom-Json
        if ($begin.data.run_id) {
            [void]$sb.AppendLine("")
            [void]$sb.AppendLine("run_id: ``$($begin.data.run_id)``")
        }
    } catch { }
} catch {
    [void]$sb.AppendLine('```json')
    [void]$sb.AppendLine($policyJson)
    [void]$sb.AppendLine('```')
}

[void]$sb.AppendLine("")
[void]$sb.AppendLine("Contain the fire. Do not remodel the building.")

Set-Content -Path $Out -Value $sb.ToString() -Encoding UTF8
Write-Host "Wrote $Out" -ForegroundColor DarkGray
Write-Output $Out

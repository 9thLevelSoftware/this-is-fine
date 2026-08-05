# Safe complete wrapper — pass run_id as parameter, never unquoted shell expand.
# Usage: .\tif-complete.ps1 -RunId <id>
param(
    [Parameter(Mandatory = $true)]
    [string]$RunId,
    [string]$Repo = $(if ($env:TIF_REPO) { $env:TIF_REPO } else { "." })
)

$ErrorActionPreference = "Stop"

if (-not (Get-Command tif -ErrorAction SilentlyContinue)) {
    Write-Error "tif not on PATH"
    exit 1
}

& tif --repo $Repo run complete --json $RunId --from-git
exit $LASTEXITCODE

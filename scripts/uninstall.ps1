# Uninstall a user-local This Is Fine binary install (does not remove repo state).
#
# Usage:
#   .\scripts\uninstall.ps1 [-Prefix DIR] [-PurgeSecrets]
param(
    [string]$Prefix = $(if ($env:TIF_INSTALL_PREFIX) { $env:TIF_INSTALL_PREFIX } else { Join-Path $env:LOCALAPPDATA "this-is-fine" }),
    [switch]$PurgeSecrets
)

$ErrorActionPreference = "Stop"
$BinDir = Join-Path $Prefix "bin"
$removed = $false
foreach ($name in @("tif.exe", "tif")) {
    $p = Join-Path $BinDir $name
    if (Test-Path $p) {
        Remove-Item -Force $p
        Write-Host "Removed $p"
        $removed = $true
    }
}
if (-not $removed) {
    Write-Host "No tif binary under $BinDir"
}

if ($PurgeSecrets) {
    $candidates = @(
        (Join-Path $env:LOCALAPPDATA "this-is-fine"),
        (Join-Path $env:APPDATA "this-is-fine")
    )
    foreach ($d in $candidates) {
        if (Test-Path $d) {
            # Only remove secrets subdir if present; do not wipe Prefix bin parent if shared
            $secrets = Join-Path $d "secrets"
            if (Test-Path $secrets) {
                Remove-Item -Recurse -Force $secrets
                Write-Host "Purged $secrets"
            }
        }
    }
}

Write-Host "Uninstall complete (repo .this-is-fine/ state is left intact)."

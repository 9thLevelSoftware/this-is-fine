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
    # Must match credentials::secrets_dir() — ProjectDirs application name "tif":
    # typically %APPDATA%\tif\secrets (config_dir on Windows).
    $candidates = New-Object System.Collections.Generic.List[string]
    if ($env:APPDATA) {
        $candidates.Add((Join-Path $env:APPDATA "tif\secrets"))
    }
    if ($env:LOCALAPPDATA) {
        $candidates.Add((Join-Path $env:LOCALAPPDATA "tif\secrets"))
    }
    # Custom prefix installs may co-locate secrets under Prefix.
    $candidates.Add((Join-Path $Prefix "secrets"))
    $seen = @{}
    foreach ($d in $candidates) {
        if ($seen.ContainsKey($d)) { continue }
        $seen[$d] = $true
        if (Test-Path $d) {
            Remove-Item -Recurse -Force $d
            Write-Host "Purged secrets $d"
        }
    }
}

Write-Host "Uninstall complete (repo .this-is-fine/ state is left intact)."

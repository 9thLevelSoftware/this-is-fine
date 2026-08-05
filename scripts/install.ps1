# Install This Is Fine (`tif`) on Windows for the current user.
#
# Usage:
#   irm https://raw.githubusercontent.com/9thLevelSoftware/this-is-fine/main/scripts/install.ps1 | iex
#   .\scripts\install.ps1 [-Version v0.1.0] [-FromSource]
param(
    [string]$Version = $(if ($env:TIF_VERSION) { $env:TIF_VERSION } else { "" }),
    [switch]$FromSource,
    [string]$Prefix = $(if ($env:TIF_INSTALL_PREFIX) { $env:TIF_INSTALL_PREFIX } else { Join-Path $env:LOCALAPPDATA "this-is-fine" }),
    [string]$Repo = $(if ($env:TIF_REPO_SLUG) { $env:TIF_REPO_SLUG } else { "9thLevelSoftware/this-is-fine" })
)

$ErrorActionPreference = "Stop"
$BinDir = Join-Path $Prefix "bin"
New-Item -ItemType Directory -Force -Path $BinDir | Out-Null

function Install-FromSource {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw "cargo not found. Install Rust from https://rustup.rs or use a release binary."
    }
    Write-Host "Building from source (cargo install)…"
    if ((Test-Path "Cargo.toml") -and (Test-Path "crates\tif")) {
        cargo install --path crates/tif --root $Prefix --locked
        if ($LASTEXITCODE -ne 0) { cargo install --path crates/tif --root $Prefix }
    } else {
        cargo install --git "https://github.com/$Repo.git" tif --root $Prefix --locked
        if ($LASTEXITCODE -ne 0) { cargo install --git "https://github.com/$Repo.git" tif --root $Prefix }
    }
    Write-Host "Installed tif to $(Join-Path $BinDir 'tif.exe')"
}

function Install-FromRelease {
    if (-not $Version) {
        $rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
        $Version = $rel.tag_name
    }
    if (-not $Version) {
        Write-Warning "Could not resolve latest release; falling back to -FromSource"
        Install-FromSource
        return
    }
    $asset = "tif-x86_64-pc-windows-msvc.zip"
    $url = "https://github.com/$Repo/releases/download/$Version/$asset"
    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("tif-install-" + [guid]::NewGuid().ToString())
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null
    $zip = Join-Path $tmp $asset
    Write-Host "Downloading $url…"
    try {
        Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
    } catch {
        Write-Warning "Release asset missing; falling back to -FromSource"
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
        Install-FromSource
        return
    }
    Expand-Archive -Path $zip -DestinationPath $tmp -Force
    $exe = Get-ChildItem -Path $tmp -Recurse -Filter "tif.exe" | Select-Object -First 1
    if (-not $exe) { throw "tif.exe not found in release archive" }
    Copy-Item -Force $exe.FullName (Join-Path $BinDir "tif.exe")
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    Write-Host "Installed tif $Version to $(Join-Path $BinDir 'tif.exe')"
}

if ($FromSource) {
    Install-FromSource
} else {
    Install-FromRelease
}

# Prepend bin dir for current session
if ($env:PATH -notlike "*$BinDir*") {
    $env:PATH = "$BinDir;$env:PATH"
    Write-Host "Added $BinDir to PATH for this session."
    Write-Host "Persist with: [Environment]::SetEnvironmentVariable('Path', `"$BinDir;`" + [Environment]::GetEnvironmentVariable('Path','User'), 'User')"
}

& (Join-Path $BinDir "tif.exe") --version
Write-Host "Done. Next: tif init && tif on"

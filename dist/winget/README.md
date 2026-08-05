# WinGet packaging notes — This Is Fine

WinGet manifests are published under a package id such as `9thLevelSoftware.ThisIsFine`.

## Layout (expected)

```text
manifests/9/9thLevelSoftware/ThisIsFine/<version>/
  9thLevelSoftware.ThisIsFine.yaml              # version
  9thLevelSoftware.ThisIsFine.installer.yaml    # installers + sha256
  9thLevelSoftware.ThisIsFine.locale.en-US.yaml # locale
```

## Installer

- Prefer the GitHub Release zip: `tif-x86_64-pc-windows-msvc.zip`
- Nested installer type: `portable` (single `tif.exe`) or `zip` extract
- `InstallerSha256` must match `SHA256SUMS` from the release workflow
- Silent switches: N/A for portable; document PATH addition for users

## Draft installer YAML fields

```yaml
PackageIdentifier: 9thLevelSoftware.ThisIsFine
PackageVersion: 0.1.0
Installers:
  - Architecture: x64
    InstallerType: zip
    InstallerUrl: https://github.com/9thLevelSoftware/this-is-fine/releases/download/v0.1.0/tif-x86_64-pc-windows-msvc.zip
    InstallerSha256: REPLACE_WITH_RELEASE_SHA256
    NestedInstallerType: portable
    NestedInstallerFiles:
      - RelativeFilePath: tif-x86_64-pc-windows-msvc/tif.exe
        PortableCommandAlias: tif
ManifestType: installer
ManifestVersion: 1.6.0
```

## Process

1. Cut a `v*` tag → `.github/workflows/release.yml` builds assets + checksums
2. Copy SHA256 into the installer manifest
3. Open a PR against [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs)
4. Validate with `winget validate --manifest …` and `winget install --manifest …`

Until the first public package is accepted, Windows users should use:

```powershell
irm https://raw.githubusercontent.com/9thLevelSoftware/this-is-fine/main/scripts/install.ps1 | iex
# or
cargo install --path crates/tif
```

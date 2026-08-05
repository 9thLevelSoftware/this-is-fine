# Release / install dry-run (Q4 / P1-1)

Procedure to Pass **Q4** (install without Rust from a real Release) and optionally **P1-1** (signed releases).

## Preconditions

- CI green on the commit to tag (`ubuntu` / `windows` / `macos` + `user-testing`)
- [V1_READINESS.md](V1_READINESS.md) residual Must list reviewed
- No open issues labeled `P0` / `blocker`
- A **git checkout** of this-is-fine (for local installer invocation)

> **Important:** Do **not** pipe `install.sh` into bash for SUMS-verified release installs unless you accept that a piped script must download `lib/sha256-verify.sh` from the network (supported fallback). Prefer invoking `./scripts/install.sh` from a checkout so sibling libs resolve offline-friendly after download of only the release assets.

## Steps

### 1. Tag RC (optional) or v1.0.0

```bash
git checkout main && git pull
# ensure clean tree
git tag -a v0.1.1-rc.1 -m "RC for install dry-run"
git push origin v0.1.1-rc.1
```

GitHub Actions `release.yml` builds multi-target assets + `SHA256SUMS` (+ cosign if secret set).

### 2. Unix install dry-run (checkout-local installer)

```bash
git clone https://github.com/9thLevelSoftware/this-is-fine.git
cd this-is-fine
export TIF_INSTALL_PREFIX=/tmp/tif-dry-run
rm -rf "$TIF_INSTALL_PREFIX"
# From checkout so scripts/lib/sha256-verify.sh is adjacent:
./scripts/install.sh --version v0.1.1-rc.1 --prefix "$TIF_INSTALL_PREFIX"
"$TIF_INSTALL_PREFIX/bin/tif" --version
# Upgrade once
./scripts/install.sh --version v0.1.1-rc.1 --prefix "$TIF_INSTALL_PREFIX"
# Uninstall
./scripts/uninstall.sh --prefix "$TIF_INSTALL_PREFIX" --purge-secrets
test ! -e "$TIF_INSTALL_PREFIX/bin/tif"
```

### 3. Windows install dry-run

```powershell
cd path\to\this-is-fine   # checkout
$env:TIF_INSTALL_PREFIX = "$env:TEMP\tif-dry-run"
Remove-Item -Recurse -Force $env:TIF_INSTALL_PREFIX -ErrorAction SilentlyContinue
.\scripts\install.ps1 -Version v0.1.1-rc.1 -Prefix $env:TIF_INSTALL_PREFIX
& "$env:TIF_INSTALL_PREFIX\bin\tif.exe" --version
.\scripts\uninstall.ps1 -Prefix $env:TIF_INSTALL_PREFIX -PurgeSecrets
```

### 4. Optional cosign (P1-1)

If release published `*.sig` and `COSIGN_PUBLIC_KEY` is available:

```bash
export COSIGN_PUBLIC_KEY=/path/to/cosign.pub
export TIF_REQUIRE_COSIGN=1
./scripts/install.sh --version v0.1.1-rc.1 --prefix /tmp/tif-cosign
```

### 5. Record evidence

Paste versions, OS, and links into V1_READINESS **Q4** / **P1-1** Evidence columns; flip Status to **Pass**.

## Automated partial coverage

- **D01/D02** — SUMS verify logic (`scripts/lib/sha256-verify.sh`)
- **P18** e2e — real `install.{sh,ps1} --from-source` → upgrade → `uninstall` + secrets purge

## Evidence log

| Date | OS | Tag | Result | Operator |
|------|-----|-----|--------|----------|
| 2026-08-05 | Windows | v0.1.1-rc.1 | Pass — SUMS verify, install, upgrade, uninstall | AI field agent |
| | Unix | v0.1.1-rc.1 | Pending field dry-run | |

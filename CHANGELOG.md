# Changelog

All notable changes to This Is Fine are documented here.

Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).  
Versioning: see [docs/VERSIONING.md](docs/VERSIONING.md).

## [Unreleased]

### Changed

- Workspace package version **0.1.1** (clap `--version` matches next release tags)

### Added

- **F5 vendor day checklist** — [docs/F5_VENDOR_DAY.md](docs/F5_VENDOR_DAY.md)

## [0.1.1-rc.1] - 2026-08-05

Release candidate for **Q4 install dry-run** (GitHub Release assets + SHA256SUMS).

> **Note:** Assets for tag `v0.1.1-rc.1` were built when the crate still reported **`tif 0.1.0`**. Identify that RC by **install tag + SHA256SUMS**, not by `tif --version` alone. Subsequent tags (after workspace version 0.1.1) report matching versions.

### Added (since 0.1.0 foundation)

- Full production surface (phases 0–10): Firebreak, Five-Alarm, adapters, TUI, distribution
- AI field-validation battery (`crates/tif-e2e` Tiers A–D, C01 N=50 soak)
- Install SUMS verification (`scripts/lib/sha256-verify.sh`); uninstall scripts
- V1 readiness evidence closeout + internal X2 security review pack
- Residual rails: P18 install/upgrade/uninstall e2e; adapter install smoke

### Residual for v1.0.0

- Must **F5** real agent host E2E (see [F5_VENDOR_DAY.md](docs/F5_VENDOR_DAY.md))
- Must **Q3** tag `v1.0.0` + dedicated CHANGELOG section
- Must **Q4** Unix `install.sh` field dry-run (Windows already recorded for this RC)
- Optional P1-1 cosign when secrets configured

## [0.1.0] — 2026-08-04

### Added

- Initial MVP foundation: `tif-core` + `tif` CLI
- Config, policy, pressure, scoring, verification, orchestrator, audit
- Fire Levels 1–5 (Five-Alarm escalation-only)
- Read-only CI templates (GitHub Actions, GitLab)
- Design specification under `docs/superpowers/specs/`

Phases 1–10 detail (reviewers, Firebreak, Five-Alarm, intelligence, adapters, TUI, distribution, hardening) landed between 0.1.0 and 0.1.1-rc.1; see git history and prior PR descriptions for full notes.

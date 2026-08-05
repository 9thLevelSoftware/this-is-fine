# Changelog

All notable changes to This Is Fine are documented here.

Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).  
Versioning: see [docs/VERSIONING.md](docs/VERSIONING.md).

## [Unreleased]

## [1.0.0] - 2026-08-05

First **stable 1.0** release of This Is Fine (`tif`): local-first agent restraint with correctness floor, Firebreak, Five-Alarm recovery, multi-agent adapters, and checksummed multi-OS installs.

### Highlights

- **Production surface (phases 0–10)** — config/policy, Firebreak closed loop, Five-Alarm, inspector/audit, adaptation, adapters, TUI, distribution, GA hardening
- **Install without Rust** — `scripts/install.sh` / `install.ps1` with SHA-256 SUMS verify; uninstall + secrets purge; multi-target GitHub Release assets
- **AI field validation** — `crates/tif-e2e` Tiers A–D (including C01 N=50 soak); evidence in [docs/V1_READINESS.md](docs/V1_READINESS.md)
- **Q4 field Pass** — Win + macOS install dry-run against `v0.1.1-rc.1` ([docs/RELEASE_DRY_RUN.md](docs/RELEASE_DRY_RUN.md))
- **F5 vendor Pass (accepted)** — real agent products: Claude Code 2.1.219 (macOS) + OpenCode 1.18.13 (Windows); split-agent Win+Unix accepted for Must F5
- **Protocol v1** — adapter JSON `protocol_version: 1`; config schema `version = 1` ([docs/VERSIONING.md](docs/VERSIONING.md))

### Added (since 0.1.0)

- Full production CLI + `tif-core` library surface
- First-class adapters: Claude Code, Codex, Gemini CLI, OpenCode
- AI agent playbooks for residual field tasks ([docs/AGENT_Q4_F5_PLAYBOOK.md](docs/AGENT_Q4_F5_PLAYBOOK.md))
- Internal X2 security review pack + threat model

### Known limitations (not blockers for 1.0)

- Optional **cosign** signed releases (P1-1) when secrets configured
- **Homebrew / WinGet** not published as official channels (binary-only via GitHub Releases + install scripts)
- Same-agent F5 on both OS is optional; current F5 uses different products per OS

### Upgrade

```bash
# Unix
./scripts/install.sh --version v1.0.0
# Windows
.\scripts\install.ps1 -Version v1.0.0
```

`tif --version` reports **`tif 1.0.0`** for this tag.

## [0.1.1-rc.1] - 2026-08-05

Release candidate for **Q4 install dry-run** (GitHub Release assets + SHA256SUMS).

> **Note:** Assets for tag `v0.1.1-rc.1` were built when the crate still reported **`tif 0.1.0`**. Identify that RC by **install tag + SHA256SUMS**, not by `tif --version` alone.

### Added (since 0.1.0 foundation)

- Full production surface (phases 0–10): Firebreak, Five-Alarm, adapters, TUI, distribution
- AI field-validation battery (`crates/tif-e2e` Tiers A–D, C01 N=50 soak)
- Install SUMS verification (`scripts/lib/sha256-verify.sh`); uninstall scripts
- V1 readiness evidence closeout + internal X2 security review pack
- Residual rails: P18 install/upgrade/uninstall e2e; adapter install smoke

## [0.1.0] — 2026-08-04

### Added

- Initial MVP foundation: `tif-core` + `tif` CLI
- Config, policy, pressure, scoring, verification, orchestrator, audit
- Fire Levels 1–5 (Five-Alarm escalation-only)
- Read-only CI templates (GitHub Actions, GitLab)
- Design specification under `docs/superpowers/specs/`

Phases 1–10 detail (reviewers, Firebreak, Five-Alarm, intelligence, adapters, TUI, distribution, hardening) landed between 0.1.0 and 0.1.1-rc.1; see git history and prior PR descriptions for full notes.

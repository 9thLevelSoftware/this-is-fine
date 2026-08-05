# Changelog

All notable changes to This Is Fine are documented here.

Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).  
Versioning: see [docs/VERSIONING.md](docs/VERSIONING.md).

## [Unreleased]

### Added

- **Phase 2 automatic isolated Firebreak closed loop**
  - `RunOrchestrator::run_firebreak_auto`: authorized backend → re-verify → absolute/git ranking → apply or approval queue
  - Auto-apply when `approval.auto_apply_firebreak` (default `true`) and path is non-sensitive
  - Approval queue: `AwaitingApproval` + optional `approval_ttl_hours`; CLI `tif approve` / `tif reject`
  - CLI: `tif firebreak --auto` (same closed loop as `tif run complete --auto-firebreak`)
  - Isolation apply prefers `session.candidate_path` (reviewer output); skips `.tif-candidate` on overlay
  - Adaptation outcomes persisted in local SQLite (`tif adaptation`)
  - Fail-safe: failed backend, larger candidate, failed re-verify, and approval-required paths never apply
- **Phase 1 reviewer execution plane**
  - `ReviewerBackend` trait with mock, OpenAI-compatible HTTP, Anthropic, and process backends
  - Credential resolution (`env:VAR` / `file:PATH`)
  - Egress-enforced context packaging + secret redaction before send
  - JSON file-tree output contract with path-traversal rejection
  - `FirebreakEngine::generate_with_backend` (isolation only; never applies)
  - CLI: `tif reviewer list|probe|test`, `tif firebreak --invoke-backend`
- Production-oriented documentation:
  - Adapter JSON protocol v1 (`docs/protocol/v1.md`)
  - Config schema v1 reference (`docs/config/schema-v1.md`)
  - Threat model (`docs/security/threat-model.md`)
  - Versioning policy (`docs/VERSIONING.md`)
- Multi-OS CI workflow for fmt, clippy, and tests (`.github/workflows/ci.yml`)
- Cargo feature flags on `tif-core` for future reviewer providers (`provider-mock` default; optional HTTP/process backends scaffolded)
- Isolation-backed Firebreak apply/rollback (git worktree + snapshot), prune-on-delete apply, sticky dual-failure restore
- Diff metrics: git, unified diff, absolute tree weight for fair non-git ranking
- Interactive TUI foundation (`tif tui`)
- Installable adapter artifacts for Claude Code, Codex, Gemini CLI, OpenCode

### Fixed

- Verification pipe deadlock (concurrent drain) and UTF-8-safe output tails
- OutOfControl runs remain open for manual Firebreak; closed OOC can reopen
- Pressure `allowed_families` no longer injects disallowed fallbacks
- Firebreak `candidate_ready` requires `within_containment`
- No fabricated candidate metrics; absolute ranking for non-git trees
- GC protects worktree isolation baselines while sessions live

### Security

- `allow_source_egress` defaults to false
- Verification command injection marker rejection
- Fail-closed production Firebreak without isolation + re-verify

## [0.1.0] — 2026-08-04

### Added

- Initial MVP foundation: `tif-core` + `tif` CLI
- Config, policy, pressure, scoring, verification, orchestrator, audit
- Fire Levels 1–5 (Five-Alarm escalation-only)
- Read-only CI templates (GitHub Actions, GitLab)
- Design specification under `docs/superpowers/specs/`

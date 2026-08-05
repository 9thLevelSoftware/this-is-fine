# Changelog

All notable changes to This Is Fine are documented here.

Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).  
Versioning: see [docs/VERSIONING.md](docs/VERSIONING.md).

## [Unreleased]

### Added

- **Phase 7 agent adapters (E2E)**
  - Windows PowerShell installers and hooks for Claude Code, Codex, Gemini CLI, OpenCode (alongside bash)
  - Adapter JSON protocol conformance tests (`tif_core::adapter_conformance`) with mocked CLI envelopes
  - Troubleshooting guide in `adapters/README.md`; argv-safe quoting across hooks
- **Phase 8 production TUI**
  - Live run event stream, Firebreak approve/reject/rollback, reviewer probe, audit filter, settings from shared config
  - Layout: header · nav · center · metrics · bottom events; narrow terminals collapse the metrics panel
  - Confirmations for destructive ops (rollback, audit purge); screen/state unit tests
- **Phase 9 CI write + distribution**
  - Guarded CI write job templates (GitHub Actions + GitLab) requiring `ci.allow_write` and `TIF_ALLOW_WRITE`
  - GitHub Actions release workflow: multi-target binaries, checksums, attach on `v*` tags
  - `scripts/install.sh` / `scripts/install.ps1`; Homebrew formula stub; WinGet packaging notes
  - User guide + recovery runbook (`docs/user-guide.md`); README polish
- **Phase 10 hardening / GA**
  - Chaos tests: dual-failure `restore_pending` recovery; disk-full soft error classification (`TifError::DiskFull`)
  - Policy resolve fast-path performance smoke test
  - GA checklist complete in `docs/ROADMAP.md` with residual risks listed honestly
  - Threat model updated for adapters, CI write path, distribution checksums
- **Phase 4 intelligence depth**
  - Multi-language inspector: JS/TS (`package.json` scripts + npm/pnpm/yarn/bun), Python (pytest/pyproject/poetry), Go (`go.mod`), Rust clippy/fmt confidence gates
  - `confident: false` when evidence is weak; never invents test commands without scripts/config
  - Dependency deltas from `Cargo.toml` / `package.json` / `go.mod` (runtime deps only)
  - Generated-code path heuristics; `scoring_version` stamp on Damage Assessments
  - Test-change policy helpers flag weakened assertion patterns as notes
  - Config `[simplicity.exceptions]` optional list recorded as audit notes
- **Phase 5 audit / privacy / retention**
  - `tif audit --gc` runs age+size audit GC and isolation GC (`gc_expired_isolation` + rollback retention)
  - Rollback retention: `max_days` **or** `successful_commits` (git commit count when available)
  - Expanded secret redaction (JWT, DB connection strings, cookies, AWS-style keys)
  - Binary artifact skip in audit store
  - SQLite `busy_timeout` + WAL mode
  - Exclusive apply lockfile under `.this-is-fine/apply.lock`
- **Phase 6 adaptation & pressure evaluation**
  - ≥2 versioned pressure variants per family in `pressure.rs`
  - Persist pressure variant stats; promotion/demotion stubs with safety gates
  - Offline eval harness (promote/demote mock variants in tests)
  - Self-apply allowlist only: fire level bias ≤4, soft thresholds, pressure template — never floor / sensitive / verify
  - CLI: `tif adaptation status|recommend|reset` (optional `--apply` on recommend)
- **Phase 3 Five-Alarm staged recovery** (design §8.2)
  - Escalation gate: current containment failure required; historical risk alone refused
  - Stage 1: intensified Firebreak (higher attempt budget, stricter reviewer wording)
  - Stage 2: preserve Stage 1 candidate; select a different authorized model
  - Stage 3: clean-room context (task, criteria, policy, verification plan, failure summary — **no previous implementation code**)
  - Stage 4: verify all candidates; apply smallest verified; retain rejects for rollback
  - Audit timeline of stages on `RunRecord.five_alarm`
  - CLI: `tif five-alarm --plan` / `tif five-alarm --run <id> [--apply] [--historical-risk]`
  - Orchestrator: `RunOrchestrator::run_five_alarm`
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

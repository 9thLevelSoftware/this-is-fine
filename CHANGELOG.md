# Changelog

All notable changes to This Is Fine are documented here.

Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).  
Versioning: see [docs/VERSIONING.md](docs/VERSIONING.md).

## [Unreleased]

### Added

- **F5 vendor day checklist** — [docs/F5_VENDOR_DAY.md](docs/F5_VENDOR_DAY.md)

## [0.1.1-rc.1] - 2026-08-05

Release candidate for **Q4 install dry-run** (GitHub Release assets + SHA256SUMS). Not a SemVer stability promise beyond current `0.1.x` candidate quality.

### Added (since 0.1.0 foundation)

- Full production surface (phases 0–10): Firebreak, Five-Alarm, adapters, TUI, distribution
- AI field-validation battery (`crates/tif-e2e` Tiers A–D, C01 N=50 soak)
- Install SUMS verification (`scripts/lib/sha256-verify.sh`); uninstall scripts
- V1 readiness evidence closeout + internal X2 security review pack
- Residual rails: P18 install/upgrade/uninstall e2e; adapter install smoke

### Residual for v1.0.0

- Must **F5** real agent host E2E (see F5_VENDOR_DAY.md)
- Must **Q3** tag `v1.0.0` + dedicated CHANGELOG section
- Must **Q4** complete RELEASE_DRY_RUN against this or a later tag
- Optional P1-1 cosign when secrets configured

## [Unreleased archive notes]

Historical detail for phases 0–10 remains summarized in git history and prior PR descriptions; this file tracks release-facing notes going forward.
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

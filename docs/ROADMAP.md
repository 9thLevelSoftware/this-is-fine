# Production roadmap

This document tracks the **production-grade** delivery plan (full design surface, not MVP-only).

Canonical design: [`docs/superpowers/specs/2026-08-04-this-is-fine-design.md`](superpowers/specs/2026-08-04-this-is-fine-design.md).

## Phase status

| Phase | Title | Status |
|-------|-------|--------|
| **0** | Stabilize foundation, protocol/config docs, multi-OS CI, provider feature flags | **Done** (in tree) |
| **1** | Reviewer execution plane (authorized backends, egress, credentials) | **Done** (in tree) |
| **2** | Automatic isolated Firebreak closed loop + approval queue | **Done** (in tree) |
| **3** | Five-Alarm staged recovery | **Done** (in tree) |
| **4** | Inspector / metrics / Damage Assessment depth | **Done** (in tree) |
| **5** | Audit, privacy, retention, concurrency, path hardening | **Done** (in tree) |
| **6** | Adaptation & pressure evaluation | **Done** (in tree) |
| **7** | First-class agent adapters (E2E + Windows installers) | **Done** (in tree) |
| **8** | Production TUI (write actions, live run) | **Done** (in tree) |
| **9** | CI write paths, signed distribution, packaging | **Done** (in tree) |
| **10** | Hardening, performance, GA | **Done** (in tree) |

## Non-negotiable safety rules

1. Correctness floor is a **gate**, not a weight.  
2. Reviewers only from the user-authorized local pool.  
3. Source egress only with explicit reviewer permission.  
4. No apply without isolation + re-verify (+ approval if required).  
5. Adaptation cannot weaken floor, sensitive-path rules, or required verification.

## Docs map

| Doc | Purpose |
|-----|---------|
| [protocol/v1.md](protocol/v1.md) | Adapter JSON protocol |
| [config/schema-v1.md](config/schema-v1.md) | Configuration reference |
| [security/threat-model.md](security/threat-model.md) | Threats and controls |
| [VERSIONING.md](VERSIONING.md) | SemVer / schema / protocol versions |
| [user-guide.md](user-guide.md) | Install, recovery runbook |
| [V1_READINESS.md](V1_READINESS.md) | v1.0 Must/Should checklist + soak program |
| [USER_TESTING.md](USER_TESTING.md) | AI-executable field validation / user testing plan |
| [security/internal-review-x2.md](security/internal-review-x2.md) | Internal X2 security review pack |
| [adapters/VERSION_MATRIX.md](adapters/VERSION_MATRIX.md) | Adapter × OS × protocol matrix |
| [RELEASE_DRY_RUN.md](RELEASE_DRY_RUN.md) | Q4/P1-1 release install dry-run procedure |
| [../CHANGELOG.md](../CHANGELOG.md) | Release notes |

## Exit criteria for GA

See production acceptance checklist below (design §21 surface).

**v1.0 / “fully production ready” gate:** [`V1_READINESS.md`](V1_READINESS.md) — Must (P0) items with owners and pass/fail criteria. Phases 0–10 make the product *capable*; that checklist makes *ready* measurable.

## GA checklist (Phase 10 — complete)

| Criterion | Status | Notes |
|-----------|--------|-------|
| Multi-OS install without Rust | **Done** | `scripts/install.sh`, `scripts/install.ps1`; release workflow on `v*` tags |
| Auto Firebreak with authorized reviewers | **Done** | Phase 2 closed loop; fail-closed without pool |
| Five-Alarm staged recovery | **Done** | Phase 3; current-failure gate |
| Adapters E2E + Windows installers | **Done** | Claude Code, Codex, Gemini CLI, OpenCode; PS + bash; conformance tests |
| Operational TUI | **Done** | Live events, approve/reject, probe, audit filter, confirmations, narrow layout |
| CI write path (guarded) | **Done** | `ci.allow_write` + `TIF_ALLOW_WRITE`; patch artifact only |
| Signed / checksummed releases | **Partial** | SHA256SUMS on every tag release; cosign/GPG signing **not** wired yet |
| Security sign-off | **Done (doc)** | Threat model updated; residual risks listed honestly |
| Chaos: dual-failure restore | **Done** | Sticky `restore_pending` + rollback recovery tests |
| Hung verifier | **Done** | Timeout + process-group kill (existing) |
| Disk-full soft path | **Done** | `TifError::DiskFull` classification on isolation I/O |
| Policy resolve performance smoke | **Done** | Unit budget for fast-path compiles |

### Residual risks (honest)

1. **Binary signing** — release assets ship with SHA-256 checksums; detached cosign/sigstore signatures are not yet automated. Supply-chain consumers should pin checksums until signing lands.  
2. **Homebrew / WinGet** — formula and manifest stubs exist under `dist/`; packages are not published to public indexes yet.  
3. **Cross aarch64 Linux** — release matrix uses `cross`; environment drift can break that matrix cell independently of x86_64.  
4. **Hosted provider API drift** — OpenAI-compatible / Anthropic backends need ongoing contract tests as vendors change schemas.  
5. **Dual-failure restore** — rare; when both apply and automatic restore fail, operator intervention is required (documented in user-guide recovery runbook). Sticky flags make the state visible.  
6. **Adapter host quirks** — agent products change hook/skill paths; install docs may lag vendor UI renames.  
7. **Multi-language scoring** — inspector heuristics can mis-score exotic layouts; floor + explicit verification remain the safety net.  
8. **CI write job** — intentionally does not auto-open PRs without human review of the patch artifact; misconfiguration of `allow_write` is mitigated by a second CI variable guard.

## Phase 7–10 delivery summary

- **7** — PS installers + hooks; adapter conformance module; troubleshooting in `adapters/README.md`; argv-safe scripts.  
- **8** — Production TUI layout, write actions, confirmations, narrow collapse, screen/state tests.  
- **9** — Guarded CI write templates; release workflow; install scripts; Homebrew/WinGet stubs; user guide.  
- **10** — Chaos/perf tests; GA checklist; threat-model update.

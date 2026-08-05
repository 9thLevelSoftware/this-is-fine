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
| **4** | Inspector / metrics / Damage Assessment depth | Planned |
| **5** | Audit, privacy, retention, concurrency, path hardening | Planned |
| **6** | Adaptation & pressure evaluation | Planned |
| **7** | First-class agent adapters (E2E + Windows installers) | Planned |
| **8** | Production TUI (write actions, live run) | Planned |
| **9** | CI write paths, signed distribution, packaging | Planned |
| **10** | Hardening, performance, GA | Planned |

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
| [../CHANGELOG.md](../CHANGELOG.md) | Release notes |

## Exit criteria for GA

See production acceptance checklist in the implementation plan (expanded design §21): multi-OS install without Rust, auto Firebreak with authorized reviewers, Five-Alarm, adapters E2E, operational TUI, CI write path, signed releases, security sign-off.

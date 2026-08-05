# Internal security review (X2) — This Is Fine

**Date:** 2026-08-05  
**Scope:** Isolation / apply, credentials, process backend, egress, audit tiers, adapters  
**Method:** Code path trace against `docs/security/threat-model.md` + automated regression battery (`crates/tif-e2e`, unit tests)  
**Verdict:** **Pass for Conditional v1.0** with residual risks listed below (no open Critical findings)

Related: [threat-model.md](threat-model.md) · [V1_READINESS.md](../V1_READINESS.md) · [USER_TESTING.md](../USER_TESTING.md)

---

## 1. Checklist (V1_READINESS §7)

| Trace | Result | Notes / evidence |
|-------|--------|------------------|
| Apply path CLI → orchestrator → isolator → source | **OK** | `tif` `cmd_run_complete` / Firebreak → `apply_verified_candidate` only after re-verify + ranking; dual-failure sticky `restore_pending` |
| Credential resolve → HTTP/process backend | **OK** | `credentials::resolve_credential`; `file:` allowlisted under secrets dir (A09 / unit tests); env refs never invent models |
| Egress decision for hosted reviewers | **OK** | `allow_source_egress` default false; `build_reviewer_context` errors before send (A06, unit tests) |
| Adapter hooks: argv vs shell, task injection | **OK** | Adapter installers prefer argv; `adapter_conformance` scans unsafe patterns |
| Audit tiers sample | **OK** | Metadata omits bodies (A10); redacted redacts secrets before persist |
| `file:` allowlist Win + Unix | **OK** | Path policy unit tests + A09; lexical + canonicalize under secrets dir |
| Process env scrub | **OK** | `env_clear` + allowlist (A07 e2e dumps child env) |
| GC does not delete live isolation baselines | **OK** | Isolation GC unit tests; B15 audit `--gc` smoke |
| Residual risks in release notes | **OK** | Threat model § residual; CHANGELOG residual honesty |

---

## 2. Path traces (summary)

### 2.1 Apply

1. Reviewer writes only under isolation session path.  
2. Candidate re-verified with same verification plan.  
3. Ranking / hard limits must prefer smaller verified candidate.  
4. `apply_to_source` under exclusive lock; baseline preserved for rollback.  
5. Failure paths: larger candidate, backend fail, empty pool → `applied=false`, original preserved (A02–A04, B05–B06).

### 2.2 Credentials & process

1. `env:VAR` / bare env name / `file:PATH` only.  
2. `file:` rejects `..`, outside secrets dir (A09).  
3. Process backend: `env_clear` then PATH/HOME/locale allowlist (A07).  

### 2.3 Adapters

1. JSON protocol major version gate.  
2. Hooks document refuse on `ok: false`.  
3. Conformance tests ship fixture envelopes.

---

## 3. Findings

| ID | Severity | Finding | Disposition |
|----|----------|---------|-------------|
| R1 | Residual | Cosign not required on every release unless secret set | Accept for Conditional; track P1-1 dry-run |
| R2 | Residual | Dual-failure still needs operator rollback | Accept; B13 recovery drill + user-guide §4 |
| R3 | Residual | Validation strip detection is floor/operator signal, not AST proof | Accept; B04 covers floor + refuse paths; no silent larger apply over auth |
| R4 | Info | Hosted API contract drift | Mitigated by P1-3 fixture contracts; re-run on provider upgrades |

**Open Critical / P0 from this review:** none.

---

## 4. Sign-off

| Role | Name | Date | Notes |
|------|------|------|-------|
| SEC (internal) | AI field review (automated + path trace) | 2026-08-05 | Replace with human SEC owner before marketing GA |
| TL | — | — | Confirm architecture still matches traces after next large PR |

Human SEC owner should countersign before **unconditional** v1.0 marketing language.

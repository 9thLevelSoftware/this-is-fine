# This Is Fine — AI-Executable User Testing Plan

**Status:** Active — UT-0 scaffold landed (`crates/tif-e2e`, fixtures, scripts, CI job). Tiers A/B partially automated; expand via UT-1+.  
**Audience:** AI agents and QA owners performing field validation toward v1.0  
**Related:** [V1_READINESS.md](V1_READINESS.md) · [user-guide.md](user-guide.md) · [design §20.4](superpowers/specs/2026-08-04-this-is-fine-design.md) · [protocol/v1.md](protocol/v1.md)

---

## 0. Why this plan exists

v1.0 **Must** items **V1–V3** (and much of **F\*** / **S\***) require *field* evidence: real journeys, recovery drills, soak volume. Human-only soak is slow and sparse. **AI agents can drive the `tif` CLI, fixture repos, mock reviewers, and JSON protocol far faster** — and produce machine-checkable evidence packs.

This document is the **comprehensive user testing program**: scenario catalog, ownership split (AI vs human), harness design, evidence format, acceptance gates, and runbook for agents.

**Naming:** Prefer **“AI field validation battery”** over casual “user testing” in release evidence so claims stay honest. Vendor GUI sessions (if any) are labeled separately.

---

## 1. Goals and non-goals

### Goals

1. Produce pass/fail evidence for V1_READINESS Must items that are *exercisable without a human sitting at a keyboard*.
2. Cover design **§20.4** end-to-end narrative scenarios end-to-end via CLI.
3. Substitute classic “10 business days / 50 tasks” soak (**V1**) with a **50-task scripted battery** of equal or greater scenario diversity, with incident logging.
4. Be **repeatable** on clean checkout + CI (Linux + Windows primary; macOS nice).
5. Leave only **true residual human items** (org decisions, optional live LLM keys, optional vendor GUI).

### Non-goals

- Proving hosted LLM *quality* (safety does not depend on model cleverness; mock + process backends are primary).
- Replacing unit/integration tests inside `tif-core` (those stay; this suite is *user-journey* + *field* evidence).
- Marketing “fully GA” without flipping V1_READINESS statuses with linked evidence.

---

## 2. Success definition

**User testing is “done” for v1.0 purposes when:**

| # | Criterion |
|---|-----------|
| 1 | **Tiers A + B** all Pass in CI on the release-candidate commit (Win + Linux at minimum). |
| 2 | **Tier C** runs ≥ **50** tasks, ≥ **98%** Pass, **zero P0** incidents; evidence pack recorded. |
| 3 | Evidence pack fills Must: **F1–F4, F6, S1–S8, X3–X4, V3**, and **V1** via C01 (AI soak substitute). |
| 4 | **F5** either **Pass (protocol)** (adapter lifecycle) or **Pass (vendor)** (one real agent product once). |
| 5 | Residual human items (**V4**, **P1-2**, optional live keys) are explicit product choices, not “untested software.” |

**Runtime budget (stretch):** full Tier A–C battery &lt; 30 min on CI Linux (target &lt; 15 min).

---

## 3. AI vs human ownership

### 3.1 AI-primary (execute fully)

| V1 / design | What | How AI runs it |
|-------------|------|----------------|
| **S1–S3, S5, S7–S8** | Safety invariants | Fixtures + CLI/e2e; assert no apply on fail paths |
| **F1–F4** | Firebreak, approval, Five-Alarm, incomplete verify | Temp repos + mock reviewers + `tif --json` |
| **F6** | CLI supportable | Drive every user-guide command; JSON vs protocol |
| **X3–X4** | Secrets path, audit tiers | Config errors + audit DB inspection |
| **V3** | Recovery drill | Script dual-failure / rollback using *only* user-guide commands |
| **V1** | Soak volume | Tier C 50-task battery (see §6) |
| **Q1** | Multi-OS | CI matrix on this suite |
| **§20.4** | 10 narrative E2E scenarios | Mapped to B01–B10 / A10 (§9) |
| **P1-3, P1-6, P1-10** | Contracts, perf smoke, CI write guard | Fixture / timed runs / template checks |
| **P1-9** | TUI destructive ops | Prefer CLI equivalents + existing TUI unit/state tests |

### 3.2 AI-simulated soak (V1 substitute)

Classic soak: ≥10 business days **or** ≥50 real coding tasks on real repos.

**Accepted AI substitute for V1 Pass (document explicitly in evidence):**

- **N ≥ 50** scripted tasks across **≥ 2** fixture projects (e.g. `rust-mini` + `js-mini`).
- Each task: `run begin` → plant implementation (minimal / bloat / strip-validation / sensitive) → `run complete --from-git` → optional Firebreak → assert safety invariants.
- Matrix: Fire Levels 1–4, approval on/off, mock reduce/fail, incomplete verify, Five-Alarm path.
- Wall clock may be minutes; count **tasks + diversity**, not calendar days.
- Incident log with severity P0/P1/P2 (same definitions as V1_READINESS §6).

Evidence line example:

```text
AI field validation battery: N=50 tasks, 0 P0, 1 P1 fixed, commit <sha>, 2026-08-05T…
```

### 3.3 Human-only or human-gated (minimal residual)

| Item | Why not fully AI |
|------|------------------|
| **F5 vendor** | Real Claude Code / Codex / Gemini / OpenCode product install + GUI |
| **V4** | Support rota naming — org decision |
| **P1-2** | Package publish or “binary-only v1.0” decision |
| **P1-1** first signed tag | Needs GitHub secrets once; AI prepares workflow |
| **Q4** install from *published* Release | Needs a real tag; AI can RC-tag if authorized or simulate SUMS locally (D01–D02) |
| Live hosted LLM Firebreak | API keys; AI still proves egress deny without keys |

**F5 proxy rule:** Mark **F5 Pass (protocol)** when adapter scripts + JSON lifecycle pass on Win + Linux. Mark **Pass (vendor)** only after one real product run (human or cloud agent with the product installed).

---

## 4. Test architecture

### 4.1 Layout (target)

```text
docs/USER_TESTING.md          # this plan
tests/user/
  fixtures/
    rust-mini/                # cargo lib + smoke test + explicit verify
    rust-bloat/               # oversized correct tree for OOC/Firebreak
    js-mini/                  # package.json test script
    security-sensitive/       # src/auth/** + approval policy
  scenarios/                  # optional TOML scenario defs
  expected/                   # JSON outcome schemas (optional)
crates/tif-e2e/               # preferred long-term: Rust integration crate
  Cargo.toml
  src/lib.rs                  # helpers: copy fixture, run tif, assert JSON
  tests/
    tier_a_safety.rs
    tier_b_journeys.rs
    tier_c_soak.rs
    tier_d_install.rs
scripts/user-test/
  run-all.sh
  run-all.ps1
  run-scenario.sh             # optional single-ID runner
evidence/user-testing/        # generated; gitignored except sample
  <run-id>/
```

**Recommendation:** Implement **`crates/tif-e2e`** (depends on workspace; spawns `CARGO_BIN_EXE_tif` or `target/debug/tif`) + thin shell entrypoints for agents. Keep fixtures under `tests/user/fixtures/`.

### 4.2 Driver rules (mandatory)

1. **Always temp-copy fixtures** — never mutate files under `tests/user/fixtures/`.
2. Prefer **`tif --json`** + structured asserts over scraping human text.
3. **Mock reviewer only** unless `TIF_E2E_LIVE=1` (optional live suite, not required for v1 safety).
4. Capture per scenario: stdout, stderr, exit code, optional audit export, content hashes before/after apply/rollback.
5. On failure: leave temp dir path in the log for debugging.
6. **Idempotent:** fresh temp dirs every run; no reliance on leftover `.this-is-fine` state.
7. **Isolation:** one scenario = one temp repo; parallel up to N=4 carefully around git locks.

### 4.3 CLI surface under test

Primary commands (all with `--json` where applicable):

| Command | Journeys |
|---------|----------|
| `tif init` / `on` / `off` / `status` | B14, B11 |
| `tif policy resolve --task …` | B11, B12 |
| `tif run begin` / `run complete --from-git` / `--auto-firebreak` | B01–B08, C01 |
| `tif assess --from-git` | B03, C03 |
| `tif firebreak --auto` / `--apply` / `--candidate` | B05–B07, A0x |
| `tif approve` / `reject` / `rollback` | B06, B07, B13 |
| `tif five-alarm --plan` / `--run` / `--historical-risk` | B08, B09 |
| `tif verify` / `inspect` / `audit` / `audit --gc` | B11, B15, A10 |
| `tif reviewer list|probe|test` | A02, B10, B11 |
| `tif adaptation status|recommend|reset` | B11 |
| `tif fire-level` | C01 matrix |

---

## 5. Scenario catalog

Each scenario: **ID**, **maps to**, **setup**, **steps**, **assert**.

### Tier A — Safety (ship-blocking)

| ID | Name | V1 | Setup / steps | Assert |
|----|------|----|---------------|--------|
| **A01** | Correctness floor beats smaller incorrect | S1 | Score/rank fixtures or assess path where smaller candidate fails verification | Incorrect never preferred over larger correct |
| **A02** | Empty reviewer pool fail-closed | S3 | Config with no `[[reviewers]]`; force OOC complete + auto Firebreak | No apply; clear error; source intact |
| **A03** | Backend fail preserves source | S2, F1 | Mock forced fail (`TIF_MOCK_FAIL` or equivalent); Firebreak | Source tree hash unchanged; `applied=false` |
| **A04** | Larger candidate not applied | S2 | Mock/candidate larger than original | `applied=false`; source intact |
| **A05** | Incomplete verify fails floor | F4 | Empty verification plan; discovery off | Floor fail / incomplete_plan; no apply |
| **A06** | Egress deny blocks source package | S5 | Hosted-style reviewer with `allow_source_egress=false` requesting source | Error before network; no secret egress |
| **A07** | Process env scrub | S7 | Process backend; secret in parent env; child dumps env to file | Child env lacks secret |
| **A08** | Symlink / path escape rejected | S8 | Candidate with `..`, absolute path, or symlink escape | Stage/apply errors; source clean |
| **A09** | Credential file outside secrets dir | X3 | `file:` credential pointing outside allowlisted secrets dir | Config/resolve error |
| **A10** | Metadata audit tier stores no bodies | X4 | Run with audit tier=metadata; inspect CAS/DB | No prompt/diff bodies |

### Tier B — Core user journeys (ship-blocking)

| ID | Name | Maps to | Steps | Assert |
|----|------|---------|-------|--------|
| **B01** | Happy contained task | §20.4 small bug | begin → plant minimal fix → complete --from-git | Contained; no Firebreak required |
| **B02** | Unnecessary dependency Firebreak | §20.4 feature+dep | Plant bloat + fake dep; auto Firebreak mock reduce | Smaller candidate or ready; dep limited / OOC handled safely |
| **B03** | Refactor expands scope | §20.4 refactor | Unrelated files changed | Score flags unrelated; OOC or hard limit |
| **B04** | Security must not strip validation | §20.4 security | Candidate removes validation | Floor fail / not applied |
| **B05** | Firebreak fail preserves | §20.4 fail FB | Mock fail after OOC | Original workspace kept |
| **B06** | Firebreak success + rollback | F1, §20.4 | Apply smaller candidate → `tif rollback` | Files match pre-apply hashes |
| **B07** | Sensitive path approval | F2, §20.4 | Edit under `src/auth/**` | `AwaitingApproval`; approve applies; reject keeps original |
| **B08** | Five-Alarm clean-room | F3, §20.4 | Current failure + multi mock pool | Staged plan; clean-room omits prior patch body |
| **B09** | Historical risk alone insufficient | F3 | `five-alarm --historical-risk` without current failure | Forbidden / no escalate |
| **B10** | Hosted denied without auth | §20.4 hosted deny | No egress / no credential | Fail-closed message |
| **B11** | Full CLI user-guide tour | F6 | Every supportable command from user-guide quick path + recovery section | Exit 0 or documented non-zero; JSON envelope `protocol_version` major=1 |
| **B12** | Adapter protocol lifecycle | F5-proxy | policy resolve → run begin → complete --json (adapter-shaped) | States coherent; refuse unknown major |
| **B13** | Recovery drill (docs only) | V3 | Follow user-guide recovery against restore_pending / dual-failure fixture | Recovery succeeds; time-to-recover logged |
| **B14** | Init / on / off / status | F6 | Fresh temp repo | Config files present; enable flags flip |
| **B15** | Audit show / gc | X4 | After runs: `audit --json`; `audit --gc` | No crash; GC does not delete live isolation baselines |

### Tier C — Soak battery (V1 AI substitute)

| ID | Name | Count | Description |
|----|------|-------|-------------|
| **C01** | Task battery | **50** | Matrix: fixtures × fire level × plant type × approval | Aggregate `results.jsonl` + incident log |
| **C02** | Concurrent apply lock | 5 | Two processes race apply | One wins or clear lock error; no corrupt tree |
| **C03** | Perf smoke | 3 | policy resolve ×100; assess large synthetic diff | Record p95; feed P1-6 |

**C01 matrix dimensions (example generator):**

| Dimension | Values |
|-----------|--------|
| Fixture | rust-mini, js-mini (and optionally rust-bloat) |
| Fire level | 1, 2, 3, 4 |
| Plant | minimal, bloat, none (no-op), strip-validation (subset) |
| Approval | off, on (sensitive fixture only) |
| Reviewer outcome | mock-ok, mock-fail (subset) |

Generate combinations until N≥50; prefer diversity over pure cartesian product.

### Tier D — Install / release (AI prepares; secrets may gate)

| ID | Name | Assert |
|----|------|--------|
| **D01** | Install SUMS match | Local fake release dir; scripts accept matching SHA-256 |
| **D02** | Install refuse missing/mismatch SUMS | Exit non-zero without `--skip-verify` |
| **D03** | Release workflow present | `release.yml` has SUMS + optional cosign hooks |
| **D04** | CI write path guard | Template / config: write requires explicit allow flags |

### Tier E — Explicit residual (document only)

| ID | Residual |
|----|----------|
| **E01** | Real vendor agent GUI day (optional) |
| **E02** | Calendar multi-day team soak (optional if C01 Pass) |
| **E03** | Package registry publish decision |

---

## 6. Fixture repository specs

### `rust-mini`

- `Cargo.toml` library crate + `src/lib.rs` with trivial `add(a,b)`.
- `tests/smoke.rs` or inline tests.
- `.this-is-fine.toml`: explicit verification e.g. `cargo test` **or** a trivial always-pass command for CI speed (`echo ok` / `true` where full cargo is too heavy — prefer real `cargo test` when matrix allows).
- Limits: e.g. `new_runtime_dependencies = 0`.
- Local config: mock reviewer in pool.

### `rust-bloat`

- Same baseline as mini, plus unused modules, extra files, optional fake dependency line for OOC scoring.
- Used for B02, B05, B06.

### `js-mini`

- `package.json` with `"test": "node -e \"process.exit(0)\""`.
- Inspector discovery must not invent npm tests without scripts.

### `security-sensitive`

- `src/auth/login.rs` (or equivalent) containing validation.
- Shared config: `sensitive_paths = ["src/auth/**"]`, `require_firebreak_approval = true` (or product equivalent).

### Plant helpers (harness API)

| Helper | Effect |
|--------|--------|
| `plant_minimal_fix(repo)` | Small correct change for B01 |
| `plant_bloat(repo)` | Large unnecessary surface for OOC |
| `plant_strip_validation(repo)` | Removes checks for B04 / floor |
| `plant_sensitive_edit(repo)` | Touches `src/auth/**` for B07 |
| `plant_unrelated_refactor(repo)` | Extra unrelated files for B03 |

---

## 7. Evidence pack format

```text
evidence/user-testing/<run-id>/
  meta.json           # commit, date, OS, tif version, agent id
  results.jsonl       # one line per scenario
  incidents.md        # P0/P1/P2 found during battery
  checklist.md        # V1_READINESS ID → Pass/Fail + proof path
  logs/
    A01.log
    B06.log
    ...
  hashes/
    B06_before.sha256
    B06_after_apply.sha256
    B06_after_rollback.sha256
```

### `meta.json` (example)

```json
{
  "run_id": "20260805T120000Z",
  "git_sha": "abc123…",
  "tif_version": "0.1.0",
  "os": "windows",
  "agent": "grok-build",
  "tiers": ["A", "B", "C"],
  "v1_substitute": true
}
```

### `results.jsonl` (example line)

```json
{"id":"B06","pass":true,"duration_ms":1234,"v1":["F1","S2"],"artifact":"logs/B06.log"}
```

### `checklist.md` mapping

For each Must ID touched, one row:

| V1 ID | Result | Evidence |
|-------|--------|----------|
| F1 | Pass | B06, logs/B06.log |
| V1 | Pass | C01 N=50, 0 P0 |
| … | … | … |

**AI final report to human:** summary table + path to evidence pack + recommended V1_READINESS status edits.

---

## 8. How an AI agent executes a full run

### 8.1 Preconditions

```text
- Clean working tree preferred (or dedicated worktree)
- Rust toolchain available
- git available
- No need for hosted API keys for Tier A–C
```

### 8.2 Procedure

```text
1. Build:     cargo build -p tif
              cargo test -p tif-e2e --all-features   # once crate exists
2. Run:       ./scripts/user-test/run-all.sh --output evidence/user-testing/$(date -u +%Y%m%dT%H%M%SZ)
              # Windows:
              .\scripts\user-test\run-all.ps1 -Output evidence/user-testing/<run-id>
3. Gate:      Fail the run if any Tier A or B scenario failed
4. Parse:     results.jsonl → write checklist.md
5. Fix loop:  On failure → implement fix → re-run failed IDs only → full re-run before claim Pass
6. Commit:    Evidence as CI artifact and/or PR (avoid huge binary logs; keep jsonl + short logs)
7. Update:    docs/V1_READINESS.md Status + Evidence columns
8. Report:    Human-facing Pass/Fail summary
```

### 8.3 Single-scenario debug

```bash
# Filter by Rust test function name (see crates/tif-e2e/tests/*.rs):
cargo test -p tif-e2e b06_firebreak_success_and_rollback -- --nocapture
cargo test -p tif-e2e a02_empty_reviewer_pool -- --nocapture
```

### 8.4 Incident log rules (during C01)

| Severity | Definition | Action |
|----------|------------|--------|
| **P0** | Data loss, silent apply of unverified code, secret leakage, unrecoverable workspace | Fail battery; block v1.0 |
| **P1** | Wrong apply blocked late, adapter broken, install broken on primary OS | Fix or document workaround; ≤2 open without workaround for soak exit |
| **P2** | UX, docs, non-default path | Track; does not block V1 substitute alone |

---

## 9. Design §20.4 → scenario map

| Design §20.4 scenario | Scenario ID |
|----------------------|-------------|
| Small bug already solvable with existing utility | **B01** |
| Feature where model adds dependency unnecessarily | **B02** |
| Refactor expands beyond requested boundary | **B03** |
| Security change where minimalism must not remove validation | **B04** |
| Failing Firebreak preserves original | **B05** |
| Successful Firebreak + automatic application and rollback | **B06** |
| Sensitive-path Firebreak requiring approval | **B07** |
| Five-Alarm clean-room recovery | **B08** |
| Metadata-only audit mode | **A10** |
| Hosted reviewer denied by repository policy | **B10** |

---

## 10. Implementation phases (harness build)

| Phase | Scope | Exit |
|-------|--------|------|
| **UT-0** Scaffold | `crates/tif-e2e` or `tests/user` + fixtures + `scripts/user-test/run-all` + CI job | Empty/smoke scenario green |
| **UT-1** Tier A | A01–A10 automated; link/dedupe existing unit tests | All A Pass in CI |
| **UT-2** Tier B | B01–B15 with plant helpers | All B Pass in CI |
| **UT-3** Tier C | Generator N=50; aggregate + incidents | ≥98% Pass, 0 P0 |
| **UT-4** Tier D | SUMS fixture install tests | D01–D02 Pass |
| **UT-5** Docs close | This file current; V1_READINESS evidence filled | PR |
| **UT-6** Recurring | Nightly/CI or on-demand AI “run full battery” | Regression |

**Current status:** **UT-0 + UT-1 (Tier A) complete; Tier B partial.**  
- Crate: `crates/tif-e2e`  
- Fixtures: `tests/user/fixtures/{rust-mini,rust-bloat,js-mini,security-sensitive}`  
- Scripts: `scripts/user-test/run-all.sh` / `run-all.ps1`  
- CI: `user-testing` job on ubuntu + windows  
- Automated IDs:  
  - **Tier A (all):** A01–A10  
  - **Tier B:** B01, B05–B07, B11, B12, B14  
  - **Pending:** B02–B04, B08–B10, B13, B15, Tier C soak, Tier D install

---

## 11. CI integration

Add job `user-testing` (or extend `.github/workflows/ci.yml`):

```yaml
# Sketch — implement in UT-0
user-testing:
  strategy:
    matrix:
      os: [ubuntu-latest, windows-latest]
  runs-on: ${{ matrix.os }}
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - run: cargo test -p tif-e2e --all-features
    - uses: actions/upload-artifact@v4
      if: always()
      with:
        name: user-testing-${{ matrix.os }}
        path: evidence/user-testing/
```

Fail PR if Tier A/B fail. Tier C may be `main`-only or nightly if runtime is large.

---

## 12. Acceptance criteria for the testing program itself

| Criterion | Required |
|-----------|----------|
| Tier A all Pass in CI | Yes |
| Tier B all Pass in CI | Yes |
| Tier C ≥50 tasks, ≥98% Pass, 0 P0 | Yes for V1 substitute |
| Evidence pack reproducible on clean checkout | Yes |
| Human steps reduced to §3.3 list only | Yes |
| Full battery runtime budget | &lt; 30 min Linux CI (stretch) |
| Honest labeling (AI battery vs vendor GUI) | Yes |

---

## 13. Risks and mitigations

| Risk | Mitigation |
|------|------------|
| Mock Firebreak ≠ real LLM quality | Optional live suite; v1 safety independent of LLM quality |
| Windows vs Linux path/git quirks | CI matrix for `tif-e2e` |
| Flaky verify commands | Prefer deterministic fixture verify; full clippy optional |
| Evidence noise / huge logs | Strict scenario IDs + jsonl; truncate verbose logs |
| Over-claiming “user testing” | Call it **AI field validation battery**; F5 vendor separate |
| Fixtures too toy-like for V1 wording “real repos” | Prefer multi-language fixtures + document substitute; optional second phase on public OSS clones under AI control |

---

## 14. Mapping to V1_READINESS Must IDs (evidence targets)

| Must ID | Primary scenarios | Notes |
|---------|-------------------|-------|
| S1 | A01 | + existing unit tests |
| S2 | A03, A04, B05, B06 | |
| S3 | A02 | |
| S4 | B13 | Dual-failure / restore_pending |
| S5 | A06, B10 | |
| S6 | Review + A10 paths | No secrets in audit metadata |
| S7 | A07 | |
| S8 | A08 | |
| F1 | B05, B06 | |
| F2 | B07 | |
| F3 | B08, B09 | |
| F4 | A05 | |
| F5 | B12 (+ E01 vendor) | Protocol vs vendor |
| F6 | B11, B14 | |
| X3 | A09 | |
| X4 | A10, B15 | |
| V1 | C01 | AI soak substitute |
| V2 | incidents.md | No open P0 |
| V3 | B13 | Timed recovery |
| V4 | — | Human residual |
| Q4/Q5 | D01–D02 (+ real release) | Local then field |

---

## 15. Immediate next steps

1. **UT-0:** Scaffold `crates/tif-e2e` + `tests/user/fixtures/*` + `scripts/user-test/run-all.{sh,ps1}` + CI job.
2. **UT-1/2:** Implement Tier A then B with named scenario IDs; reuse phase-2/3 unit coverage where it already proves an ID (link in checklist, still run e2e where user-visible).
3. **UT-3:** 50-task generator; first full evidence pack on Windows + Linux.
4. Update **V1_READINESS.md** Status/Evidence from Unknown → Pass where proven.
5. Optional: schedule recurring AI battery (nightly CI or agent loop).

---

## 16. One-line charter

> **If the CLI, filesystem, and mock reviewers can exercise it, AI runs it, asserts it, and files the evidence — so humans only decide policy and optional vendor/live edges.**

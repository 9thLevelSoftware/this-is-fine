# This Is Fine — v1.0 Readiness Checklist

**Status:** Open — not v1.0 until all **Must** items are Pass  
**Baseline:** `main` post phases 0–10 (foundation through GA hardening)  
**Today’s honest position:** **v0.1 production candidate** — capable under controlled rollout, not GA  

Related: [ROADMAP.md](ROADMAP.md) · [threat-model.md](security/threat-model.md) · [user-guide.md](user-guide.md) · [USER_TESTING.md](USER_TESTING.md) · [design](superpowers/specs/2026-08-04-this-is-fine-design.md)

---

## 1. How to use this document

### Verdict rules

| Outcome | Rule |
|---------|------|
| **v1.0 Ready** | Every **Must (P0)** item is **Pass**, and no open **P0** residual risk |
| **v1.0 Conditional** | Every **Must (P0)** item is **Pass**, and one or more **Should (P1)** items are **Waived** (signed waiver + expiry). **Must items cannot be waived.** |
| **Not Ready** | Any **Must (P0)** item is **Fail** or **Unknown** |

### Severity

| Tag | Meaning |
|-----|---------|
| **Must (P0)** | Blocks v1.0. Shipping without this is unsafe or unsupportable as “production.” |
| **Should (P1)** | Strongly expected for a credible v1.0; may waive once with owner + date + mitigation |
| **Nice (P2)** | Improves quality; does not block v1.0 |

### Status values

| Status | Meaning |
|--------|---------|
| **Pass** | Criteria met; evidence linked (PR, run URL, doc section, ticket) |
| **Fail** | Criteria not met |
| **In progress** | Actively owned work |
| **Unknown** | Not measured yet (counts as Fail for Must items at go/no-go) |
| **Waived** | Only for P1; requires waiver record below |

### Role owners (assign real people)

| Role code | Default responsibility |
|-----------|------------------------|
| **EM** | Engineering manager / release decision owner |
| **TL** | Tech lead (architecture, fail-safe integrity) |
| **SEC** | Security owner |
| **REL** | Release / supply-chain owner |
| **QA** | Quality / soak / E2E owner |
| **DX** | Developer experience (adapters, docs, install) |
| **SRE** | Ops / recovery / on-call readiness |
| **PM** | Product (scope, acceptance, messaging) |

Fill the **Assignee** column with a name; role codes are defaults only.

---

## 2. Go / no-go summary (fill at review)

| Field | Value |
|-------|--------|
| **Review date** | _YYYY-MM-DD_ |
| **Target version** | `1.0.0` |
| **Proposed tag** | `v1.0.0` |
| **Decision** | Ready / Conditional / Not ready |
| **Decided by (EM)** | |
| **Evidence pack link** | (release notes draft, soak report, audit notes) |
| **P0 Fail count** | |
| **P1 open / waived** | |

**Decision statement (one paragraph):**  
_

---

## 3. Must (P0) — blocks v1.0

### 3.1 Safety & fail-safe

| ID | Criterion | Pass / Fail criteria | Owner | Evidence | Status |
|----|-----------|----------------------|-------|----------|--------|
| **S1** | Correctness floor remains a hard gate | Smaller incorrect candidate never ranks above larger correct in tests + one documented manual scenario | TL | USER_TESTING **A01**; `scoring` unit tests | **Pass** (automated) |
| **S2** | No apply without isolation + re-verify | Code paths that set `applied=true` require isolator apply after candidate verification; tests cover failed verify / larger candidate / backend failure | TL | **A03, A04, B05, B06**; phase2 unit e2e | **Pass** (automated) |
| **S3** | Fail-closed without authorized reviewers | Empty pool → no Firebreak apply; unit + CLI smoke | TL | **A02** | **Pass** (automated) |
| **S4** | Dual-failure is recoverable or honestly terminal | Sticky `restore_pending`; user-guide recovery steps validated on Win + Linux once each | SRE | Chaos unit tests + **B13** recovery drill; Win e2e in CI; Linux e2e in CI | **Pass** (automated + docs) |
| **S5** | Egress default false; no silent hosted source | Hosted reviewer without `allow_source_egress` cannot include source; tests green | SEC | **A06**; `providers/context` unit tests | **Pass** (automated) |
| **S6** | Credentials never logged | Credential resolve + provider request paths reviewed; no secret in audit tier metadata/redacted by design | SEC | X2 review; **A10** metadata tier; redaction unit tests | **Pass** (review + automated) |
| **S7** | Process backend does not inherit secrets | `env_clear` + allowlist still enforced; regression test | SEC | **A07** | **Pass** (automated) |
| **S8** | Symlink / path escape blocked on stage/apply | Stage + copy reject `..`, absolute paths, symlink follow; tests present | SEC | **A08**; isolation unit tests | **Pass** (automated) |

### 3.2 Functional completeness (design surface)

| ID | Criterion | Pass / Fail criteria | Owner | Evidence | Status |
|----|-----------|----------------------|-------|----------|--------|
| **F1** | Closed-loop Firebreak works with real config | On a fixture repo: OutOfControl → mock/backend → re-verify → apply → rollback restores files | QA | **B06**, **B13** | **Pass** (automated) |
| **F2** | Approval path blocks auto-apply | Sensitive path / `require_firebreak_approval` holds for approval; `tif approve` / `reject` work | QA | **B07** | **Pass** (automated) |
| **F3** | Five-Alarm current-failure gate | Historical risk alone cannot escalate; clean-room omits prior patch content (tests + one manual) | TL | **B08, B09**; phase3 unit tests | **Pass** (automated) |
| **F4** | Verification incomplete ≠ pass | Empty/unresolved required plan fails floor | TL | **A05** | **Pass** (automated) |
| **F5** | At least one first-class agent adapter works E2E | Full lifecycle on **one** of Claude Code / Codex / Gemini / OpenCode on Win **and** Unix: begin → implement → complete → status | DX | Protocol **B12** + **F5-smoke** installers. Runbook: [F5_VENDOR_DAY.md](F5_VENDOR_DAY.md). AI agent steps: [AGENT_Q4_F5_PLAYBOOK.md](AGENT_Q4_F5_PLAYBOOK.md) `T-F5`. **Unix slice:** macOS arm64, Claude Code 2.1.219 (2026-08-05). **Windows slice:** Windows 10, OpenCode 1.18.13 (2026-08-05). Same-agent cross-OS evidence remains required. | **In progress** |
| **F6** | CLI is supportable | Core ops documented in user-guide; `--json` protocol matches `docs/protocol/v1.md` for operations used by adapters | DX | **B11, B14**; protocol doc | **Pass** (automated + docs) |

### 3.3 Quality & release engineering

| ID | Criterion | Pass / Fail criteria | Owner | Evidence | Status |
|----|-----------|----------------------|-------|----------|--------|
| **Q1** | Multi-OS CI green on release commit | `ubuntu`, `windows`, `macos` fmt + clippy `-D warnings` + tests pass on the commit to be tagged | REL | `.github/workflows/ci.yml` matrix + `user-testing` job | **Pass** on green main (re-check at tag) |
| **Q2** | No open P0 bugs | Issue tracker: zero open bugs labeled `P0` / `blocker` for v1.0 | EM | 2026-08-05: zero open issues on repo (API check). **Re-check at tag.** | **Pass** (as of 2026-08-05) |
| **Q3** | Versioning discipline | SemVer `1.0.0`; schema + protocol compatibility documented; CHANGELOG has v1.0 section | REL | Still `0.1.0`; VERSIONING.md present | Unknown (tag-time) |
| **Q4** | Install without Rust | Clean machine (or VM) install via `scripts/install.sh` **and** `scripts/install.ps1` from a real GitHub Release asset; `tif --version` works | REL | Release [v0.1.1-rc.1](https://github.com/9thLevelSoftware/this-is-fine/releases/tag/v0.1.1-rc.1) assets + SHA256SUMS green. **Windows field Pass** (2026-08-05): `install.ps1 -Version v0.1.1-rc.1` SUMS OK. **Unix field Pass** (2026-08-05, macOS arm64): `install.sh --version v0.1.1-rc.1` SUMS OK → version → upgrade → uninstall. | **Pass** |
| **Q5** | Checksums verified in install path | Install scripts verify SHA-256 against published `SHA256SUMS` by default (refuse if missing/mismatch); documented in user-guide | REL | `scripts/lib/sha256-verify.sh` + **D01/D02**; user-guide; field Pass after tagged release dry-run | **Pass** (code + contract tests); field dry-run remaining |

### 3.4 Security & privacy

| ID | Criterion | Pass / Fail criteria | Owner | Evidence | Status |
|----|-----------|----------------------|-------|----------|--------|
| **X1** | Threat model current | `docs/security/threat-model.md` matches shipped surfaces (providers, apply, adapters) | SEC | threat-model.md GA section + residual list | **Pass** (doc current as of closeout) |
| **X2** | Structured security review | Written review of isolation, credentials, process backend, apply path (internal checklist **or** external). Findings closed or accepted with risk | SEC | [internal-review-x2.md](security/internal-review-x2.md) | **Pass** (internal); human SEC countersign before unconditional GA |
| **X3** | Secret file path policy | `file:` credentials only under allowlisted secrets dir; tests pass | SEC | **A09** + credentials unit tests | **Pass** (automated) |
| **X4** | Audit tiers respected | Metadata tier never stores prompt/diff bodies; redacted tier redacts secrets before persist (tests) | SEC | **A10**, **B15** | **Pass** (automated) |

### 3.5 Field validation (non-negotiable for “production”)

| ID | Criterion | Pass / Fail criteria | Owner | Evidence | Status |
|----|-----------|----------------------|-------|----------|--------|
| **V1** | Controlled soak | ≥ **10 business days** (or ≥ **50** real coding tasks) on real repos with real agents; log incidents | QA / EM | AI substitute **C01 N=50** + Tiers A–D; CI `user-testing` uploads evidence pack | **Pass** (AI field battery; optional calendar soak residual) |
| **V2** | Incident log empty of unfixed P0s | All soak P0s fixed or accepted with mitigation before tag | EM | C01 requires ≥98% / zero P0-style corruption; re-check issues at tag | **Pass** (battery); re-verify at tag |
| **V3** | Recovery drill | Operator completes dual-failure / rollback drill using only user-guide; time-to-recover recorded | SRE | **B13** records `time_to_recover_ms` using documented rollback/status/audit | **Pass** (automated drill) |
| **V4** | Support ownership named | On-call or support rota exists for apply/rollback incidents; contact path in README or user-guide | SRE | Owner: **9thLevelSoftware** maintainers; GitHub Issues path in README + user-guide | **Pass** |

---

## 4. Should (P1) — expected for a credible v1.0

| ID | Criterion | Pass / Fail criteria | Owner | Evidence | Status |
|----|-----------|----------------------|-------|----------|--------|
| **P1-1** | **Signed releases** | Cosign or GPG signatures published with every `v*` release; verify steps in user-guide | REL | Workflow + user-guide; needs first signed tag dry-run | In progress |
| **P1-2** | **Published package channel** | At least **one** of: Homebrew formula live, WinGet package live, or distro package — **or** explicit “binary-only v1.0” product decision signed by PM | REL / PM | Proposed binary-only path (install scripts + GitHub Releases; Homebrew/WinGet stubs). **Not waived** until PM fills Approved-by on the draft waiver below. | **In progress** |
| **P1-3** | Provider contract tests in CI | Mock + recorded/fixture HTTP contract tests for OpenAI-compatible response shape (no live keys required) | TL | `parse_chat_completion_content` / apply fixture tests in `providers` | **Pass** (automated) |
| **P1-4** | Second agent adapter E2E | Second agent of the four launch set proven E2E on one OS | DX | Protocol installers + matrix for all four; vendor product E2E residual | In progress |
| **P1-5** | aarch64 (or documented skip) | Release matrix cell green **or** “unsupported arch” listed in install docs | REL | `release.yml` builds `aarch64-unknown-linux-gnu` + `aarch64-apple-darwin` | **Pass** (release matrix) |
| **P1-6** | Performance budgets recorded | Policy resolve p95 budget; assess on ≥10k-line diff budget; measured once on reference hardware | QA | **C03** (policy×100 + assess 10k lines); CI user-testing | **Pass** (smoke budgets) |
| **P1-7** | Adapter version matrix | Table: agent product version × tested tif version × OS | DX | [adapters/VERSION_MATRIX.md](adapters/VERSION_MATRIX.md) has protocol placeholders only — **needs real agent product versions** | **In progress** |
| **P1-8** | Upgrade / uninstall tested | Install → upgrade to next RC → uninstall leaves no secrets in default paths | REL | `scripts/uninstall.{sh,ps1}` + e2e **P18** (real install scripts `--from-source` → upgrade → uninstall + secrets purge). Release-channel dry-run: [RELEASE_DRY_RUN.md](RELEASE_DRY_RUN.md) | **Pass** (from-source automated); release-channel residual in Q4 |
| **P1-9** | TUI destructive ops reviewed | Rollback/purge confirmations verified manually on narrow terminal | DX | TUI unit/state tests for confirmations | Pass (unit); manual residual |
| **P1-10** | CI write path dry-run | Template job with `allow_write` produces patch artifact only; protected branch never updated | SRE | Templates double-gated; threat-model | **Pass** (template review) |

### P1 waiver template (required if Conditional)

| Field | Value |
|-------|--------|
| **Item ID** | |
| **Reason** | |
| **Risk if shipped** | |
| **Mitigation** | |
| **Expiry / revisit date** | |
| **Approved by (EM + SEC if security)** | |
| **Date** | |

### Draft P1 waiver: P1-2 package channel (not active until PM signs)

| Field | Value |
|-------|--------|
| **Item ID** | P1-2 |
| **Reason** | v0.1 / first GA path is GitHub Releases + install scripts only; Homebrew/WinGet stubs are not live indexes |
| **Risk if shipped** | Users must use curl/irm install scripts rather than package managers |
| **Mitigation** | Documented install paths; SHA256SUMS; optional cosign; formula stubs remain for later |
| **Expiry / revisit date** | 2026-12-01 or first public index submission |
| **Approved by (EM + SEC if security)** | _Pending PM signature_ |
| **Date** | _—_ |

---

## 5. Nice (P2) — post-v1.0 or stretch

| ID | Criterion | Pass / Fail criteria | Owner | Status |
|----|-----------|----------------------|-------|--------|
| **N1** | Full four-agent E2E matrix | All four agents × Win + Unix smoke | DX | Unknown |
| **N2** | External pentest | Report filed; criticals fixed | SEC | Unknown |
| **N3** | SLSA / provenance attestations | Build provenance for release artifacts | REL | Unknown |
| **N4** | Deep language plugins | Rust + TS public-API scoring plugins beyond heuristics | TL | Unknown |
| **N5** | Adaptation live rollout metrics dashboard | Local stats export / TUI depth | TL | Unknown |
| **N6** | Auto PR/MR from CI | Only with multi-guard config; optional | SRE | Unknown |
| **N7** | Telemetry opt-in diagnostics | Still local-first; optional crash reports | PM | Unknown |

---

## 6. Soak program (V1 detail)

**Preferred execution path:** AI-driven field validation battery — see **[USER_TESTING.md](USER_TESTING.md)** (Tiers A–C, evidence pack, F5 protocol proxy). Human multi-day soak remains valid but is not required if the AI battery meets the substitute rules below.

Minimum for **V1** Pass:

| Parameter | Minimum |
|-----------|---------|
| Duration | 10 business days **or** 50 tasks (whichever comes first may Pass if quality bar met; prefer both) |
| Repos | ≥ 2 real projects (not only fixtures) **or** ≥ 2 multi-language fixture projects under [USER_TESTING.md](USER_TESTING.md) Tier C with documented AI soak substitute |
| Agents | ≥ 1 first-class agent with real tasks (F5 Must). AI battery may substitute V1 soak volume, but does **not** by itself Pass F5. |
| Reviewer mode | Start with **mock** or local process; introduce hosted only after 20 tasks clean |
| Apply policy | Prefer approval-required for first week on shared repos |
| Logging | Incident log with date, severity, repro, fix (see USER_TESTING evidence pack) |

### Soak exit criteria

- Zero unresolved **P0** incidents  
- ≤ 2 **P1** incidents without workaround  
- At least one successful Firebreak apply + rollback drill  
- At least one failed Firebreak that correctly preserved original  

### Incident severity (soak)

| Severity | Definition |
|----------|------------|
| P0 | Data loss, silent apply of unverified code, secret leakage, unrecoverable workspace |
| P1 | Wrong apply blocked late, adapter broken for primary agent, install broken on primary OS |
| P2 | UX, docs, non-default path bugs |

---

## 7. Security review checklist (X2)

Minimum internal review (half-day–day) before v1.0:

- [ ] Trace **apply** path from CLI → orchestrator → isolator → source  
- [ ] Trace **credential** resolve → HTTP/process backend  
- [ ] Trace **egress** decision for hosted reviewers  
- [ ] Trace **adapter** hooks: argv vs shell, task injection  
- [ ] Confirm **audit** tiers with a sample full vs redacted vs metadata run  
- [ ] Confirm **file:** credential allowlist on Windows + Unix  
- [ ] Confirm **process** env scrub  
- [ ] Confirm **GC** does not delete live isolation baselines  
- [ ] Document residual risks in release notes  

**External review:** recommended for org-wide default-on; not required for Conditional v1.0 if X2 internal is thorough.

---

## 8. Release checklist (day of v1.0)

| Step | Owner | Done |
|------|-------|------|
| All Must items Pass (or Conditional waivers filed) | EM | [ ] |
| CHANGELOG `## [1.0.0]` complete | REL | [ ] |
| Tag `v1.0.0` from green CI commit | REL | [ ] |
| GitHub Release assets + SHA256SUMS (+ signatures if P1-1 Pass) | REL | [ ] |
| Install scripts verified against **that** release | REL | [ ] |
| README “Status” updated to production v1.0 (no “candidate” language unless Conditional) | PM | [ ] |
| Support / on-call page linked | SRE | [ ] |
| Known issues section published | PM | [ ] |
| Announce internal/external | PM | [ ] |

---

## 9. Suggested timeline to v1.0 (from current baseline)

| Week | Focus | Exit |
|------|--------|------|
| **1** | Assign owners; open tracker for every Must Unknown; enable signing workstream | Owners filled |
| **1–2** | Security review X2; fix P0 findings | X2 Pass |
| **2–3** | Install + checksum verification; release dry-run RC1 | Q4/Q5 Pass on RC |
| **2–4** | Soak V1–V3 with one agent | V1 Pass |
| **3–4** | P1 signing and/or package decision | P1-1 or waiver; P1-2 decision |
| **4** | RC2; adapter E2E F5; recovery drill V3 | All Must Pass |
| **5** | Tag v1.0.0 | Go decision |

Compress only if soak evidence already exists.

---

## 10. Current baseline snapshot (as of 2026-08-05 closeout)

Honest fill-in for planning (update as evidence lands):

| Area | Assessment |
|------|------------|
| Feature completeness (design) | **High** — phases 0–10 on main |
| Automated tests / multi-OS CI | **High** — green matrix + user-testing evidence artifacts |
| Supply chain (signing / packages) | **Medium** — checksums + optional cosign; package channel decision still open (P1-2) |
| Field soak | **High (AI battery)** — Tiers A–D including C01 N=50; optional human vendor day residual |
| Security review | **High (internal X2)** — formal pack + threat model; human countersign residual |
| Support readiness | **Medium** — contact path documented; no 24×7 rota |

**Residual Must (blocks Ready):**

1. **F5** — repeat the same first-class agent adapter on Win **and** Unix; current evidence is split between Claude Code (Unix) and OpenCode (Windows)
2. **Q3** — tag `1.0.0` + CHANGELOG section (RC is `v0.1.1-rc.1`)
3. **Q2** — re-confirm zero open P0 issues at tag time

**Should residual:** P1-1 signed tag dry-run; P1-2 PM signature; P1-4/P1-7 vendor product versions; human SEC countersign on X2.

**Implication:** Not Ready for unconditional v1.0 until F5, full Q4, and tag-time Q items clear. Prefer messaging:

> **v0.1 production candidate — AI field-validated; controlled rollout toward tagged v1.0.**

---

## 11. Tracker mapping (optional)

Create GitHub issues (or project board columns) with labels:

- `v1.0` + `must` / `should` / `nice`  
- ID in title: e.g. `[v1.0][S2] No apply without re-verify`  

Board columns: `Unknown` → `In progress` → `Evidence ready` → `Pass` / `Waived`.

---

## 12. Sign-off block

| Role | Name | Signature / date | Notes |
|------|------|------------------|-------|
| EM | | | Final go/no-go |
| TL | | | Fail-safe integrity |
| SEC | | | Security |
| REL | | | Release artifacts |
| QA | | | Soak + E2E |
| DX | | | Adapters + docs |
| SRE | | | Support + recovery |
| PM | | | Product messaging |

---

*This checklist is the gate for calling This Is Fine **v1.0 / fully production ready**. Phases 0–10 made the product **capable**; this document makes “ready” measurable.*

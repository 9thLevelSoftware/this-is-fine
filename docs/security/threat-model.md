# Threat model — This Is Fine

**Maps to design §19.** Living document; update when backends or adapters gain surface area.

**GA status (Phase 10):** controls below reflect the production tree; residual risks are called out explicitly.

## Assets

| Asset | Sensitivity |
|-------|-------------|
| User source code | High |
| Credentials / API keys (local config, env) | Critical |
| Audit store (prompts, diffs, logs) | High (depends on tier) |
| Isolation baselines / rollback trees | High |
| Repository integrity (apply path) | Critical |
| Hosted model provider channels | High (egress) |
| Release binaries / install scripts | High (supply chain) |

## Trust boundaries

```text
[Coding agent] --CLI/JSON--> [tif core] --optional HTTPS--> [Hosted reviewer]
      |                         |
      |                         +--> [Local FS: .this-is-fine/, worktrees]
      v
[User workspace]
```

- **Repository config** (`.this-is-fine.toml`) is **trusted** for verification command strings (defense-in-depth validation still applies).  
- **Adapter-supplied commands** are **not** trusted without the same validation policy.  
- **Reviewer output** is **untrusted** until re-verified in isolation.  
- **Hosted providers** receive only redacted, egress-approved packages.  
- **CI write jobs** are **untrusted** until both config (`ci.allow_write`) and pipeline variable (`TIF_ALLOW_WRITE`) opt in; outputs are patch artifacts, not protected-branch force-pushes.

## Threats and controls

| Threat | Controls |
|--------|----------|
| Command injection via verification | Reject control chars / `$()` / backticks; trusted-config only; prefer argv arrays |
| Secret leakage in logs/prompts | Redaction before CAS; audit tiers; egress default **false** |
| Symlink / path traversal on snapshot/apply | Selective copy; skip symlink files/dirs; reject `.` / `..` / absolute escapes |
| Malicious repo config | Schema validation; incomplete verification fails floor |
| Untrusted Firebreak patches | Isolation-only write; re-verify; ranking; approval for sensitive paths |
| Unauthorized hosted model | Pool from local config only; never invent models |
| Source egress without consent | `allow_source_egress` default false; enforce before send |
| Race original vs worktree | Exclusive apply lockfile; session sticky state |
| Corrupt rollback artifacts | Baseline integrity checks; dual-failure sticky restore + operator rollback |
| SQLite crash / lock | Bundled SQLite; WAL + busy timeout |
| Hung child processes | Timeouts; process group / tree kill |
| Supply chain (binary/deps) | Multi-OS release builds + **SHA256SUMS** on tags; cosign/sigstore **not yet** automated |
| Adapter shell injection | Prefer argv; quoted task/run ids; PS + bash installers; conformance unsafe-pattern scan |
| Disk exhaustion mid-apply | Soft `TifError::DiskFull`; dual-failure sticky flags; GC guidance in user guide |
| Accidental CI repo mutation | Write job double-gated (`allow_write` + `TIF_ALLOW_WRITE`); artifact-only by default |
| Destructive TUI ops | Confirmations for rollback and audit purge |

## Fail-safe invariant

> No unverified or failed simplification may replace a known-good implementation.

Production Firebreak is **fail-closed** without isolation + re-verify (+ approval when required). With authorized backends, the Phase 2 closed loop still never applies unverified, larger, or out-of-containment candidates. Simulated metrics must never authorize apply.

## Residual risks (tracked post-GA)

1. Detached binary **signing** (cosign/GPG) not automated — consumers should pin SHA-256 from release `SHA256SUMS`.  
2. Hosted provider APIs change — contract tests required on an ongoing cadence.  
3. Dual-failure restore (apply fail + restore fail) still requires operator intervention; errors and sticky flags are honest.  
4. Homebrew/WinGet packages are stubs until first public index submission.  
5. Multi-language intelligence may mis-score exotic repositories; verification floor remains authoritative.  
6. Adapter hosts rename hooks/skills — install docs can lag.

## Security checklist (GA sign-off)

- [x] Correctness floor cannot be weakened by adaptation  
- [x] Reviewer pool is local-config-only  
- [x] Egress default deny  
- [x] Isolation + re-verify before apply  
- [x] Approval path for sensitive / required cases  
- [x] Secret redaction in audit paths  
- [x] Adapter protocol fail-closed on `ok: false` / bad major  
- [x] CI write double-gated  
- [x] Recovery runbook for dual-failure / disk full  
- [ ] Automated release **signing** (checksums only today)  
- [ ] Public package-index publication (Homebrew/WinGet)

## Review cadence

- Revisit this document when backends, adapters, or distribution change.  
- Treat unchecked items above as explicit follow-ups, not silent assumptions.

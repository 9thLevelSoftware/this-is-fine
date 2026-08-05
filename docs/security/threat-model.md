# Threat model — This Is Fine

**Maps to design §19.** Living document; update when backends or adapters gain surface area.

## Assets

| Asset | Sensitivity |
|-------|-------------|
| User source code | High |
| Credentials / API keys (local config, env) | Critical |
| Audit store (prompts, diffs, logs) | High (depends on tier) |
| Isolation baselines / rollback trees | High |
| Repository integrity (apply path) | Critical |
| Hosted model provider channels | High (egress) |

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

## Threats and controls

| Threat | Controls (current / planned) |
|--------|------------------------------|
| Command injection via verification | Reject control chars / `$()` / backticks; document trusted-config only; prefer argv arrays (Phase 1+) |
| Secret leakage in logs/prompts | Redaction before CAS; audit tiers; egress default **false** |
| Symlink / path traversal on snapshot/apply | Selective copy guards; planned full deny of absolute escapes + symlink policy (Phase 5) |
| Malicious repo config | Schema validation; incomplete verification fails floor |
| Untrusted Firebreak patches | Isolation-only write; re-verify; ranking; approval for sensitive paths |
| Unauthorized hosted model | Pool from local config only; never invent models |
| Source egress without consent | `allow_source_egress` default false; enforce before send (Phase 1) |
| Race original vs worktree | Single-apply locks (Phase 5); session sticky state |
| Corrupt rollback artifacts | Baseline integrity checks; dual-failure sticky restore |
| SQLite crash / lock | Bundled SQLite; planned WAL + busy timeout (Phase 5) |
| Hung child processes | Timeouts; process group / tree kill |
| Supply chain (binary/deps) | Planned SBOM + signing (Phase 9) |
| Adapter shell injection | Prefer argv; OpenCode wrappers; `TIF_REQUIRED` hard-fail option |

## Fail-safe invariant

> No unverified or failed simplification may replace a known-good implementation.

Production Firebreak is **fail-closed** without isolation + re-verify (+ approval when required). With authorized backends, the Phase 2 closed loop still never applies unverified, larger, or out-of-containment candidates. Simulated metrics must never authorize apply.

## Residual risks (tracked)

1. Multi-language intelligence may mis-score until Phase 4 plugins mature.  
2. Hosted provider APIs change — contract tests required.  
3. Dual-failure restore (apply fail + restore fail) requires operator intervention; errors must be honest.  

## Review cadence

- Revisit this document at the end of each production phase.  
- Security checklist sign-off required before GA (Phase 10).

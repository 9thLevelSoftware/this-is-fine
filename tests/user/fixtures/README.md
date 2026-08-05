# User-testing fixtures

Temp-copied by `tif-e2e` / `scripts/user-test` — **never mutate in place during a run**.

| Fixture | Purpose |
|---------|---------|
| `rust-mini` | Happy path, contained tasks, CLI tour |
| `rust-bloat` | Out-of-control / Firebreak apply + rollback |
| `js-mini` | Second project language for soak diversity |
| `security-sensitive` | `src/auth/**` approval path |

Verification uses `echo tif-ok` for speed and OS portability. Offline mock reviewers are declared in shared `.this-is-fine.toml` (committed; no secrets).

See [`docs/USER_TESTING.md`](../../../docs/USER_TESTING.md).

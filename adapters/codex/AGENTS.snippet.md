# This Is Fine (Codex / AGENTS.md fragment)

Append the following to repository `AGENTS.md` or Codex instructions when containment is enabled.

---

## This Is Fine containment

Before implementing a task, if `tif` is available:

```text
tif run begin --json --agent codex --task "<task summary>"
tif policy resolve --json --task "<task summary>"
```

Obey the pressure body and hard limits from the policy response.

Rules:

1. Smallest correct change wins.
2. No new runtime dependencies unless required for correctness.
3. Prefer reuse; avoid speculative abstractions and extra files.
4. Do not weaken tests to make a candidate pass.
5. On completion: `tif run complete --json <run_id> --from-git`
6. If out of control: `tif firebreak --json --run-id <run_id>`
7. Rollback: `tif rollback --json <run_id>`

Contain the fire. Do not remodel the building.

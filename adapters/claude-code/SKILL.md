---
name: this-is-fine
description: >
  Inject This Is Fine containment pressure and policy for the current task.
  Prefer the smallest correct change. Call the tif JSON CLI for lifecycle events.
---

# This Is Fine — Containment Skill

**Contain the fire. Do not remodel the building.**

When This Is Fine is active in the repository (`.this-is-fine.toml` with `enabled = true`):

## At task start

1. Resolve policy and begin a run:

```bash
tif policy resolve --json --task "$TASK"
tif run begin --json --agent claude-code --task "$TASK"
```

2. Treat `data.policy.pressure.body` and hard limits in `data.policy.limits` as **session law**.
3. Surface `data.compact_status` to the user when useful.

## While implementing

- Prefer reuse over new abstractions, files, and dependencies.
- Do not add runtime dependencies unless required for correctness.
- Keep diffs minimal and scoped to the task.
- Do not weaken tests merely to pass.

## At task completion

```bash
tif run complete --json "$RUN_ID" --from-git
# or explicit metrics:
# tif run complete --json "$RUN_ID" --files-changed N --lines-added M --deps-added 0
```

If the assessment is **Out of Control**, offer Firebreak:

```bash
tif firebreak --json --run-id "$RUN_ID"
# With an isolated smaller candidate tree:
# tif firebreak --json --run-id "$RUN_ID" --candidate /path/to/candidate --apply
```

Rollback after a successful apply:

```bash
tif rollback --json "$RUN_ID"
```

## Safety

- Verification and Firebreak remain outside the implementation model.
- Failed or unverified Firebreak **never** modifies the original workspace.
- Do not authorize new hosted reviewer models from this skill.

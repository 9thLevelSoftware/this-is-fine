# Claude Code adapter (stub)

## Integration approach

1. Call `tif policy resolve --json --task "…"` at task start.
2. Inject `data.policy.pressure.body` into a Claude Code skill, CLAUDE.md snippet, or session instruction.
3. Show `data.compact_status` in the UI.
4. On completion, call `tif run complete --json <run_id> …` with diff metrics.
5. If assessment is out of control, call `tif firebreak --json --run-id <run_id>`.

## Example skill fragment

```markdown
# This Is Fine — Containment

When This Is Fine is active, follow the pressure scenario and containment
limits supplied by `tif policy resolve`. Prefer the smallest correct change.
Do not add dependencies, files, or abstractions unless required for correctness.

Contain the fire. Do not remodel the building.
```

## Lifecycle mapping

| Claude Code event | `tif` command |
|---|---|
| Task start | `tif run begin --json --agent claude-code --task …` |
| Task complete | `tif run complete --json <run_id> …` |
| Manual simplify | `tif firebreak --json --run-id …` |
| Rollback | `tif rollback --json <run_id>` |

Full native hook wiring is deferred; this directory documents the contract for implementers.

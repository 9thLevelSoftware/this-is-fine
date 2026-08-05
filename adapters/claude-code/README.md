# Claude Code adapter

Installable artifacts (not stubs):

| File | Purpose |
|---|---|
| [`SKILL.md`](./SKILL.md) | Native skill: pressure + lifecycle CLI calls |
| [`hooks/tif-session.sh`](./hooks/tif-session.sh) | Session-start helper |
| [`install.md`](./install.md) | Activation steps |

## Lifecycle mapping

| Claude Code event | `tif` command |
|---|---|
| Task start | `tif run begin --json --agent claude-code --task …` |
| Task complete | `tif run complete --json <run_id> --from-git` |
| Manual simplify | `tif firebreak --json --run-id …` |
| Rollback | `tif rollback --json <run_id>` |

See [install.md](./install.md).

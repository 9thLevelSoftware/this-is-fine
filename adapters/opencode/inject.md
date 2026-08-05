# OpenCode injection notes

On session start:

1. `tif run begin --json --agent opencode --task "…"`
2. Inject `data.policy.pressure.body` into system/developer prompts.
3. Show `data.compact_status` in the status bar when available.

On finish:

1. `tif run complete --json <run_id> --from-git`
2. If out of control, surface `tif firebreak --json --run-id <run_id>`.

See [`plugin.json`](./plugin.json) for a declarative command map.

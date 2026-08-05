# Install — Codex

1. Install `tif` on `PATH`.
2. Append [`AGENTS.snippet.md`](./AGENTS.snippet.md) to repository `AGENTS.md` or Codex global instructions.
3. Optionally use [`tif-bridge.sh`](./tif-bridge.sh) at session start to print pressure text for injection.
4. `tif init && tif on` in the target repository.

## Lifecycle

| Event | Command |
|---|---|
| Start | `tif run begin --json --agent codex --task …` |
| Complete | `tif run complete --json <run_id> --from-git` |
| Firebreak | `tif firebreak --json --run-id …` |
| Rollback | `tif rollback --json <run_id>` |

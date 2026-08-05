# Install — Claude Code

1. Ensure `tif` is on `PATH` (`cargo install --path crates/tif` or a release binary).
2. Copy [`SKILL.md`](./SKILL.md) into your Claude Code skills directory (project or user skills).
3. Optionally wire [`hooks/tif-session.sh`](./hooks/tif-session.sh) as a session-start hook.
4. In a repository: `tif init && tif on`.

## Activation check

```bash
tif status
tif policy resolve --json --task "smoke test"
```

JSON responses include `protocol_version: 1`. Fail closed on unsupported versions.

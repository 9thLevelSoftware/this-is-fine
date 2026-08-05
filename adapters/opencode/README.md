# OpenCode adapter (stub)

## Integration approach

1. Plugin or middleware calls `tif` over CLI JSON.
2. On session start: `tif run begin --json --agent opencode`.
3. Inject `policy.pressure.body` into OpenCode system/developer prompts.
4. On finish: `tif run complete --json` with file/line/dependency metrics.
5. Optional: surface `tif status` in the OpenCode status bar.

## Plugin surface (illustrative)

```json
{
  "name": "this-is-fine",
  "commands": {
    "contain": "tif on",
    "release": "tif off",
    "assess": "tif assess --json",
    "firebreak": "tif firebreak --json"
  }
}
```

## Contract version

Adapters should check `protocol_version` in JSON responses and fail closed on unsupported versions.

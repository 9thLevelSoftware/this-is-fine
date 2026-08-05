# Gemini CLI adapter (stub)

## Integration approach

1. `tif policy resolve --json` before generation.
2. Pass pressure + policy summary through Gemini CLI context flags or a generated context file under `.this-is-fine/`.
3. `tif run begin --json --agent gemini-cli`.
4. After edits: `tif assess --json` and/or `tif run complete --json`.

## Context file sketch

Write `.this-is-fine/active-policy.md` from the resolved policy body so Gemini CLI can include it with `@.this-is-fine/active-policy.md`.

## Safety

Verification and Firebreak remain outside the model. Gemini CLI is only used for implementation or as an *authorized* reviewer when listed in `.this-is-fine.local.toml`.

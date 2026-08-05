# Codex adapter (stub)

## Integration approach

1. Resolve policy: `tif policy resolve --json --task "…"`.
2. Append the pressure body to Codex instructions / AGENTS guidance for the session.
3. Begin a run: `tif run begin --json --agent codex --model <model> --task "…"`.
4. On completion, submit metrics via `tif run complete --json`.

## Example instruction inject

```text
[This Is Fine]
{{pressure.body}}

Hard limits are enforced after your change by independent verification and scoring.
A failed or larger Firebreak candidate will never replace a known-good implementation.
```

## Notes

- Prefer repository-local configuration over global defaults.
- Never auto-select unauthorized hosted reviewers.

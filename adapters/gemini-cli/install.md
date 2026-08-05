# Install — Gemini CLI

1. Install `tif` on `PATH`.
2. Run [`generate-context.sh`](./generate-context.sh) `"task summary"` before generation.
3. Include the generated file with Gemini CLI, e.g. `@.this-is-fine/active-policy.md`.
4. After edits: `tif run complete --json <run_id> --from-git` or `tif assess --from-git`.

## Safety

Verification and Firebreak stay in the Rust core. Gemini is only an implementation model unless explicitly listed as an authorized reviewer in `.this-is-fine.local.toml`.

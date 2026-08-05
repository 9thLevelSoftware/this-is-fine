# Install — OpenCode

1. Install `tif` on `PATH`.
2. Register [`plugin.json`](./plugin.json) with your OpenCode plugin loader, or map the same commands in middleware.
3. Prefer **argv arrays** from `plugin.json` (no shell). If the host only supports shell hooks, use [`hooks/tif-begin.sh`](./hooks/tif-begin.sh) and [`hooks/tif-complete.sh`](./hooks/tif-complete.sh) which quote safely.
4. Follow [`inject.md`](./inject.md) for pressure injection.
5. `tif init && tif on` in the target repository.

Check `protocol_version` in every JSON response (expected: `1`).

## Security

Do not expand `${TASK}` / `${RUN_ID}` into a shell string. Task text with quotes or metacharacters can break out of naive interpolation. Use argv arrays or the wrapper scripts.

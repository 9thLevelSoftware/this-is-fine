#!/usr/bin/env bash
# Safe complete wrapper — pass run_id as $1, never interpolate into unquoted shell.
set -euo pipefail
RUN_ID="${1:?run_id required}"
REPO="${TIF_REPO:-.}"
exec tif --repo "$REPO" run complete --json "$RUN_ID" --from-git

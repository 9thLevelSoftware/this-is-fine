#!/usr/bin/env bash
# Safe argv-style begin for hosts that only support shell hooks.
# Usage: tif-begin.sh "task text"
set -euo pipefail
TASK="${1:-}"
REPO="${TIF_REPO:-.}"
exec tif --repo "$REPO" run begin --json --agent opencode --task "$TASK"

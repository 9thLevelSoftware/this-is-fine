#!/usr/bin/env bash
# Claude Code session helper — inject compact status + begin containment run.
# Prefer argv arrays; task text is never expanded into an unquoted shell string.
# Install: copy SKILL.md into your Claude skills directory; source this from a session hook.
set -euo pipefail

TASK="${1:-${CLAUDE_TASK:-session}}"
REPO="${TIF_REPO:-.}"

if ! command -v tif >/dev/null 2>&1; then
  echo "ERROR: tif not on PATH; install This Is Fine CLI (cargo install --path crates/tif or scripts/install.sh)" >&2
  if [[ "${TIF_REQUIRED:-0}" == "1" ]]; then
    exit 1
  fi
  echo "WARNING: containment will not start (set TIF_REQUIRED=1 to fail hard)" >&2
  exit 0
fi

# Argv form: each argument is a separate word (safe for spaces/quotes in TASK).
tif --repo "${REPO}" policy resolve --json --task "${TASK}" || true
tif --repo "${REPO}" run begin --json --agent claude-code --task "${TASK}" || true

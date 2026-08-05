#!/usr/bin/env bash
# Write active pressure/policy context for Gemini CLI (@file inclusion).
set -euo pipefail

TASK="${1:-current task}"
REPO="${TIF_REPO:-.}"
OUT="${TIF_CONTEXT_OUT:-$REPO/.this-is-fine/active-policy.md}"

mkdir -p "$(dirname "$OUT")"

if ! command -v tif >/dev/null 2>&1; then
  echo "tif not on PATH" >&2
  exit 1
fi

POLICY_JSON="$(tif --repo "$REPO" policy resolve --json --task "$TASK")"
BEGIN_JSON="$(tif --repo "$REPO" run begin --json --agent gemini-cli --task "$TASK" || true)"

{
  echo "# This Is Fine — Active Containment Policy"
  echo
  echo "Generated for Gemini CLI context inclusion."
  echo
  if command -v jq >/dev/null 2>&1; then
    echo "## Status"
    echo
    echo '```'
    echo "$POLICY_JSON" | jq -r '.data.compact_status // "n/a"'
    echo '```'
    echo
    echo "## Pressure"
    echo
    echo "$POLICY_JSON" | jq -r '.data.policy.pressure.body // empty'
    echo
    echo "## Limits"
    echo
    echo '```json'
    echo "$POLICY_JSON" | jq '.data.policy.limits'
    echo '```'
    RUN_ID="$(echo "$BEGIN_JSON" | jq -r '.data.run_id // empty')"
    if [[ -n "$RUN_ID" ]]; then
      echo
      echo "run_id: \`$RUN_ID\`"
    fi
  else
    echo '```json'
    echo "$POLICY_JSON"
    echo '```'
  fi
  echo
  echo "Contain the fire. Do not remodel the building."
} >"$OUT"

echo "Wrote $OUT" >&2
echo "$OUT"

#!/usr/bin/env bash
# Thin Codex bridge: begin run + print pressure body for instruction injection.
# Prefer argv arrays; never interpolate task text into eval/shell -c strings.
set -euo pipefail

TASK="${1:?task text required}"
REPO="${TIF_REPO:-.}"
AGENT="${TIF_AGENT:-codex}"
MODEL="${TIF_MODEL:-}"

ARGS=(--repo "${REPO}" run begin --json --agent "${AGENT}" --task "${TASK}")
if [[ -n "${MODEL}" ]]; then
  ARGS+=(--model "${MODEL}")
fi

BEGIN_JSON="$(tif "${ARGS[@]}")"
echo "${BEGIN_JSON}"

POLICY_ARGS=(--repo "${REPO}" policy resolve --json --task "${TASK}")
POLICY_JSON="$(tif "${POLICY_ARGS[@]}")"
# Extract pressure body with jq if present; else print full policy JSON.
if command -v jq >/dev/null 2>&1; then
  echo "${POLICY_JSON}" | jq -r '.data.policy.pressure.body // empty'
  echo "${POLICY_JSON}" | jq -r '.data.compact_status // empty' >&2
else
  echo "${POLICY_JSON}"
fi

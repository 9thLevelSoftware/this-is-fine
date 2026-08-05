#!/usr/bin/env bash
# Install Gemini CLI adapter artifacts for This Is Fine (Unix).
set -euo pipefail
ADAPTER_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="$(pwd)/.this-is-fine"

echo "This Is Fine — Gemini CLI adapter install"
if ! command -v tif >/dev/null 2>&1; then
  echo "WARNING: tif is not on PATH." >&2
fi

mkdir -p "${DEST}"
cp -f "${ADAPTER_ROOT}/generate-context.sh" "${DEST}/generate-context.sh"
chmod +x "${DEST}/generate-context.sh" 2>/dev/null || true
echo "Copied generate-context.sh -> ${DEST}"
echo "Usage: ${DEST}/generate-context.sh \"task text\""

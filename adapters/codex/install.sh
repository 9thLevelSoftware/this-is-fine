#!/usr/bin/env bash
# Install Codex adapter artifacts for This Is Fine (Unix).
set -euo pipefail
ADAPTER_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET_AGENTS="${1:-}"

echo "This Is Fine — Codex adapter install"
if ! command -v tif >/dev/null 2>&1; then
  echo "WARNING: tif is not on PATH." >&2
fi

if [[ -n "${TARGET_AGENTS}" ]]; then
  {
    echo ""
    echo "<!-- This Is Fine Codex adapter -->"
    cat "${ADAPTER_ROOT}/AGENTS.snippet.md"
  } >>"${TARGET_AGENTS}"
  echo "Appended AGENTS.snippet.md -> ${TARGET_AGENTS}"
else
  echo "Snippet: ${ADAPTER_ROOT}/AGENTS.snippet.md"
  echo "Usage: $0 path/to/AGENTS.md"
fi

DEST="$(pwd)/.this-is-fine"
mkdir -p "${DEST}"
cp -f "${ADAPTER_ROOT}/tif-bridge.sh" "${DEST}/tif-bridge.sh"
chmod +x "${DEST}/tif-bridge.sh" 2>/dev/null || true
echo "Copied tif-bridge.sh -> ${DEST}"

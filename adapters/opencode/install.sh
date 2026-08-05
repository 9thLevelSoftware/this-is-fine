#!/usr/bin/env bash
# Install OpenCode adapter artifacts for This Is Fine (Unix).
set -euo pipefail
ADAPTER_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="$(pwd)/.this-is-fine/opencode"

echo "This Is Fine — OpenCode adapter install"
if ! command -v tif >/dev/null 2>&1; then
  echo "WARNING: tif is not on PATH." >&2
fi

mkdir -p "${DEST}/hooks"
cp -f "${ADAPTER_ROOT}/plugin.json" "${DEST}/plugin.json"
cp -f "${ADAPTER_ROOT}/inject.md" "${DEST}/inject.md" 2>/dev/null || true
cp -f "${ADAPTER_ROOT}/hooks/"* "${DEST}/hooks/" 2>/dev/null || true
chmod +x "${DEST}/hooks/"*.sh 2>/dev/null || true
echo "Copied OpenCode plugin + hooks -> ${DEST}"
echo "Wire hooks per inject.md / plugin.json; then: tif init && tif on"

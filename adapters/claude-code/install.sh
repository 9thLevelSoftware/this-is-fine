#!/usr/bin/env bash
# Install Claude Code adapter artifacts for This Is Fine (Unix).
set -euo pipefail
ADAPTER_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SKILLS_DIR="${1:-${HOME}/.claude/skills/this-is-fine}"

echo "This Is Fine — Claude Code adapter install"
if ! command -v tif >/dev/null 2>&1; then
  echo "WARNING: tif is not on PATH. Install via scripts/install.sh or cargo install --path crates/tif." >&2
fi

mkdir -p "${SKILLS_DIR}/hooks"
cp -f "${ADAPTER_ROOT}/SKILL.md" "${SKILLS_DIR}/SKILL.md"
cp -f "${ADAPTER_ROOT}/hooks/tif-session.sh" "${SKILLS_DIR}/hooks/tif-session.sh"
if [[ -f "${ADAPTER_ROOT}/hooks/tif-session.ps1" ]]; then
  cp -f "${ADAPTER_ROOT}/hooks/tif-session.ps1" "${SKILLS_DIR}/hooks/tif-session.ps1"
fi
chmod +x "${SKILLS_DIR}/hooks/tif-session.sh" 2>/dev/null || true
echo "Copied skill + hooks -> ${SKILLS_DIR}"
echo "Next: wire hooks/tif-session.sh; in a repo run: tif init && tif on"

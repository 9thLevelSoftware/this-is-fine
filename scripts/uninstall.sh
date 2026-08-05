#!/usr/bin/env bash
# Uninstall a user-local This Is Fine binary install (does not remove repo state).
#
# Usage:
#   ./scripts/uninstall.sh [--prefix DIR] [--purge-secrets]
set -euo pipefail

PREFIX="${TIF_INSTALL_PREFIX:-${HOME}/.local}"
PURGE_SECRETS=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix) PREFIX="$2"; shift 2 ;;
    --purge-secrets) PURGE_SECRETS=1; shift ;;
    -h|--help)
      echo "Usage: $0 [--prefix DIR] [--purge-secrets]"
      exit 0
      ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

BIN_DIR="${PREFIX}/bin"
removed=0
for name in tif tif.exe; do
  p="${BIN_DIR}/${name}"
  if [[ -f "$p" ]] || [[ -L "$p" ]]; then
    rm -f "$p"
    echo "Removed ${p}"
    removed=1
  fi
done

if [[ "$removed" -eq 0 ]]; then
  echo "No tif binary under ${BIN_DIR}"
fi

if [[ "${PURGE_SECRETS}" -eq 1 ]]; then
  # Platform config dir used by tif credentials (directories crate: ProjectDirs)
  candidates=(
    "${HOME}/.config/this-is-fine"
    "${HOME}/.local/share/this-is-fine"
    "${HOME}/Library/Application Support/this-is-fine"
  )
  for d in "${candidates[@]}"; do
    if [[ -d "$d" ]]; then
      rm -rf "$d"
      echo "Purged secrets/config dir ${d}"
    fi
  done
fi

echo "Uninstall complete (repo .this-is-fine/ state is left intact)."

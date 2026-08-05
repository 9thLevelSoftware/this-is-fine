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
      echo "  --purge-secrets  Remove only credential secrets dirs (not full config trees)"
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
  # Must match credentials::secrets_dir() — ProjectDirs application name "tif":
  #   ~/.config/tif/secrets  or  $XDG_CONFIG_HOME/tif/secrets
  #   macOS: ~/Library/Application Support/tif/secrets
  # Also purge PREFIX/secrets if a custom install layout put credentials there.
  candidates=(
    "${XDG_CONFIG_HOME:-${HOME}/.config}/tif/secrets"
    "${HOME}/.config/tif/secrets"
    "${HOME}/Library/Application Support/tif/secrets"
    "${PREFIX}/secrets"
  )
  # De-dupe
  seen=""
  for d in "${candidates[@]}"; do
    case " ${seen} " in
      *" ${d} "*) continue ;;
    esac
    seen="${seen} ${d}"
    if [[ -d "$d" ]]; then
      rm -rf "$d"
      echo "Purged secrets ${d}"
    fi
  done
fi

echo "Uninstall complete (repo .this-is-fine/ state is left intact)."

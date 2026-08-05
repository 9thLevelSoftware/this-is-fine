#!/usr/bin/env bash
# Shared SHA-256 verification helpers for install.sh and local e2e (no network).
#
# Usage (CLI):
#   scripts/lib/sha256-verify.sh <asset_path> <asset_name> <sums_path>
# Exit 0 on match; non-zero on missing SUMS entry or mismatch.
set -euo pipefail

sha256_file() {
  local f="$1"
  local out
  if command -v sha256sum >/dev/null 2>&1; then
    out="$(sha256sum "$f" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    out="$(shasum -a 256 "$f" | awk '{print $1}')"
  else
    echo "ERROR: need sha256sum or shasum" >&2
    return 1
  fi
  # Keep only hex digits (MSYS/Git Bash path escapes can pollute $1).
  printf '%s' "$out" | tr -cd '0-9a-fA-F' | tr 'A-F' 'a-f'
  printf '\n'
}

# Verify asset against a local SHA256SUMS file (already on disk).
# Args: asset_path asset_name sums_path
verify_against_sums() {
  local asset_path="$1"
  local asset_name="$2"
  local sums="$3"
  local expected actual

  if [[ ! -f "${sums}" ]]; then
    echo "ERROR: SHA256SUMS not found at ${sums}" >&2
    return 1
  fi
  if [[ ! -f "${asset_path}" ]]; then
    echo "ERROR: asset not found at ${asset_path}" >&2
    return 1
  fi

  # Lines: <hex>  <filename>  or <hex> *filename
  expected="$(grep -E "[[:space:]]${asset_name}\$" "${sums}" | head -n1 | awk '{print $1}' || true)"
  if [[ -z "${expected}" ]]; then
    expected="$(awk -v n="${asset_name}" '$2 == n || $2 == "*"n || $NF == n { print $1; exit }' "${sums}")"
  fi
  if [[ -z "${expected}" ]]; then
    echo "ERROR: ${asset_name} not listed in SHA256SUMS" >&2
    return 1
  fi
  expected="$(printf '%s' "${expected}" | tr -cd '0-9a-fA-F' | tr 'A-F' 'a-f')"
  actual="$(sha256_file "${asset_path}")"
  if [[ "${actual}" != "${expected}" ]]; then
    echo "ERROR: checksum mismatch for ${asset_name}" >&2
    echo "  expected: ${expected}" >&2
    echo "  actual:   ${actual}" >&2
    return 1
  fi
  echo "SHA-256 OK (${actual})"
  return 0
}

# When executed as a script (not sourced), run CLI mode.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  if [[ $# -lt 3 ]]; then
    echo "Usage: $0 <asset_path> <asset_name> <sums_path>" >&2
    exit 2
  fi
  verify_against_sums "$1" "$2" "$3"
fi

#!/usr/bin/env bash
# Install This Is Fine (`tif`) for the current user without requiring a full
# Rust toolchain when a GitHub release binary is available.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/9thLevelSoftware/this-is-fine/main/scripts/install.sh | bash
#   ./scripts/install.sh [--version v0.1.0] [--from-source] [--skip-verify]
set -euo pipefail

REPO="${TIF_REPO_SLUG:-9thLevelSoftware/this-is-fine}"
VERSION="${TIF_VERSION:-}"
FROM_SOURCE=0
SKIP_VERIFY=0
PREFIX="${TIF_INSTALL_PREFIX:-${HOME}/.local}"
BIN_DIR="${PREFIX}/bin"

usage() {
  cat <<'EOF'
install.sh — install the tif CLI

Options:
  --version VER     Release tag (default: latest)
  --from-source     Build with cargo install (requires Rust)
  --skip-verify     Skip SHA-256 verification (not recommended)
  --prefix DIR      Install prefix (default: ~/.local)
  -h, --help        Show help

Verification (default for release installs):
  Downloads SHA256SUMS from the same release and checks the asset digest.
  Optional: set TIF_REQUIRE_COSIGN=1 to also require cosign verify-blob when
  a .sig asset is published (needs cosign on PATH).
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --from-source) FROM_SOURCE=1; shift ;;
    --skip-verify) SKIP_VERIFY=1; shift ;;
    --prefix) PREFIX="$2"; BIN_DIR="${PREFIX}/bin"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown arg: $1" >&2; usage; exit 1 ;;
  esac
done

mkdir -p "${BIN_DIR}"

detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "${os}" in
    Linux) os_part="unknown-linux-gnu" ;;
    Darwin) os_part="apple-darwin" ;;
    *) echo "unsupported OS: ${os}" >&2; return 1 ;;
  esac
  case "${arch}" in
    x86_64|amd64) arch_part="x86_64" ;;
    aarch64|arm64) arch_part="aarch64" ;;
    *) echo "unsupported arch: ${arch}" >&2; return 1 ;;
  esac
  echo "${arch_part}-${os_part}"
}

install_from_source() {
  if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo not found; install Rust from https://rustup.rs or use a release binary" >&2
    exit 1
  fi
  echo "Building from source (cargo install)…"
  if [[ -f Cargo.toml ]] && [[ -d crates/tif ]]; then
    cargo install --path crates/tif --root "${PREFIX}" --locked 2>/dev/null \
      || cargo install --path crates/tif --root "${PREFIX}"
  else
    cargo install --git "https://github.com/${REPO}.git" tif --root "${PREFIX}" --locked 2>/dev/null \
      || cargo install --git "https://github.com/${REPO}.git" tif --root "${PREFIX}"
  fi
  echo "Installed tif to ${BIN_DIR}/tif"
}

sha256_file() {
  local f="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$f" | awk '{print $1}'
  else
    shasum -a 256 "$f" | awk '{print $1}'
  fi
}

verify_asset() {
  local asset_path="$1"
  local asset_name="$2"
  local version="$3"
  local tmp="$4"
  local sums_url sums expected actual

  if [[ "${SKIP_VERIFY}" -eq 1 ]]; then
    echo "WARNING: skipping SHA-256 verification (--skip-verify)" >&2
    return 0
  fi

  sums_url="https://github.com/${REPO}/releases/download/${version}/SHA256SUMS"
  sums="${tmp}/SHA256SUMS"
  echo "Verifying ${asset_name} against ${sums_url}…"
  if ! curl -fsSL "${sums_url}" -o "${sums}"; then
    echo "ERROR: could not download SHA256SUMS for ${version}; refusing to install without verification." >&2
    echo "Re-run with --skip-verify only if you accept the risk, or use --from-source." >&2
    return 1
  fi

  # Lines look like: <hex>  <filename>  or <hex> *filename
  expected="$(grep -E "[[:space:]]${asset_name}\$" "${sums}" | head -n1 | awk '{print $1}')"
  if [[ -z "${expected}" ]]; then
    # Try matching basename only if path-style
    expected="$(awk -v n="${asset_name}" '$2 == n || $2 == "*"n || $NF == n { print $1; exit }' "${sums}")"
  fi
  if [[ -z "${expected}" ]]; then
    echo "ERROR: ${asset_name} not listed in SHA256SUMS" >&2
    return 1
  fi
  actual="$(sha256_file "${asset_path}")"
  if [[ "${actual}" != "${expected}" ]]; then
    echo "ERROR: checksum mismatch for ${asset_name}" >&2
    echo "  expected: ${expected}" >&2
    echo "  actual:   ${actual}" >&2
    return 1
  fi
  echo "SHA-256 OK (${actual})"

  if [[ "${TIF_REQUIRE_COSIGN:-0}" == "1" ]]; then
    if ! command -v cosign >/dev/null 2>&1; then
      echo "ERROR: TIF_REQUIRE_COSIGN=1 but cosign not on PATH" >&2
      return 1
    fi
    local sig_url sig
    sig_url="https://github.com/${REPO}/releases/download/${version}/${asset_name}.sig"
    sig="${tmp}/${asset_name}.sig"
    if ! curl -fsSL "${sig_url}" -o "${sig}"; then
      echo "ERROR: signature ${asset_name}.sig not found for this release" >&2
      return 1
    fi
    # Public key optional: COSIGN_PUBLIC_KEY path or keyless verify if configured at release.
    if [[ -n "${COSIGN_PUBLIC_KEY:-}" ]]; then
      cosign verify-blob --key "${COSIGN_PUBLIC_KEY}" --signature "${sig}" "${asset_path}"
    else
      echo "WARNING: signature file present but COSIGN_PUBLIC_KEY unset; downloaded .sig only" >&2
      echo "Set COSIGN_PUBLIC_KEY to a PEM public key to enforce cosign verify-blob." >&2
    fi
  fi
}

install_from_release() {
  local target asset url tmp
  target="$(detect_target)"
  if [[ -z "${VERSION}" ]]; then
    VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
      | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)"
  fi
  if [[ -z "${VERSION}" ]]; then
    echo "Could not resolve latest release; falling back to --from-source" >&2
    install_from_source
    return
  fi
  asset="tif-${target}.tar.gz"
  url="https://github.com/${REPO}/releases/download/${VERSION}/${asset}"
  tmp="$(mktemp -d)"
  echo "Downloading ${url}…"
  if ! curl -fsSL "${url}" -o "${tmp}/${asset}"; then
    echo "Release asset missing; falling back to --from-source" >&2
    rm -rf "${tmp}"
    install_from_source
    return
  fi
  if ! verify_asset "${tmp}/${asset}" "${asset}" "${VERSION}" "${tmp}"; then
    rm -rf "${tmp}"
    exit 1
  fi
  tar -xzf "${tmp}/${asset}" -C "${tmp}"
  # Archive layout: tif-<target>/tif
  if [[ -f "${tmp}/tif-${target}/tif" ]]; then
    install -m 755 "${tmp}/tif-${target}/tif" "${BIN_DIR}/tif"
  elif [[ -f "${tmp}/tif" ]]; then
    install -m 755 "${tmp}/tif" "${BIN_DIR}/tif"
  else
    find "${tmp}" -type f -name tif -exec install -m 755 {} "${BIN_DIR}/tif" \;
  fi
  rm -rf "${tmp}"
  echo "Installed tif ${VERSION} to ${BIN_DIR}/tif"
}

if [[ "${FROM_SOURCE}" -eq 1 ]]; then
  install_from_source
else
  install_from_release
fi

if ! command -v tif >/dev/null 2>&1; then
  echo "Add ${BIN_DIR} to PATH, e.g.:"
  echo "  export PATH=\"${BIN_DIR}:\$PATH\""
fi
tif --version 2>/dev/null || "${BIN_DIR}/tif" --version
echo "Done. Next: tif init && tif on"

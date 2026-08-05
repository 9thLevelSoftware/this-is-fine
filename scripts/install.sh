#!/usr/bin/env bash
# Install This Is Fine (`tif`) for the current user without requiring a full
# Rust toolchain when a GitHub release binary is available.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/9thLevelSoftware/this-is-fine/main/scripts/install.sh | bash
#   ./scripts/install.sh [--version v0.1.0] [--from-source]
set -euo pipefail

REPO="${TIF_REPO_SLUG:-9thLevelSoftware/this-is-fine}"
VERSION="${TIF_VERSION:-}"
FROM_SOURCE=0
PREFIX="${TIF_INSTALL_PREFIX:-${HOME}/.local}"
BIN_DIR="${PREFIX}/bin"

usage() {
  cat <<'EOF'
install.sh — install the tif CLI

Options:
  --version VER   Release tag (default: latest)
  --from-source   Build with cargo install (requires Rust)
  --prefix DIR    Install prefix (default: ~/.local)
  -h, --help      Show help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --from-source) FROM_SOURCE=1; shift ;;
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

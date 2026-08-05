#!/usr/bin/env bash
# AI field-validation battery entrypoint (docs/USER_TESTING.md).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

OUT=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output|-o)
      OUT="${2:-}"
      shift 2
      ;;
    --help|-h)
      echo "Usage: $0 [--output evidence/user-testing/RUN_ID]"
      exit 0
      ;;
    *)
      echo "Unknown arg: $1" >&2
      exit 2
      ;;
  esac
done

if [[ -z "$OUT" ]]; then
  OUT="evidence/user-testing/$(date -u +%Y%m%dT%H%M%SZ)"
fi

mkdir -p "$OUT"

echo "==> Building tif"
cargo build -p tif

echo "==> Running tif-e2e user-testing battery"
set +e
cargo test -p tif-e2e --all-targets -- --nocapture 2>&1 | tee "$OUT/cargo-test.log"
CODE=${PIPESTATUS[0]}
set -e

# Structured evidence pack (real exit code in meta.json; never swallow STATUS).
EVIDENCE_SCRIPT="$SCRIPT_DIR/write_evidence.py"
if command -v python3 >/dev/null 2>&1; then
  python3 "$EVIDENCE_SCRIPT" "$OUT" "$CODE"
elif command -v python >/dev/null 2>&1; then
  python "$EVIDENCE_SCRIPT" "$OUT" "$CODE"
else
  echo "warning: python not found; writing minimal meta only" >&2
  cat >"$OUT/meta.json" <<EOF
{
  "run_id": "$(basename "$OUT")",
  "harness": "scripts/user-test/run-all.sh",
  "exit_code": $CODE,
  "log": "cargo-test.log",
  "note": "write_evidence.py skipped (no python)"
}
EOF
fi

if [[ $CODE -eq 0 ]]; then
  echo "Pass" >"$OUT/STATUS"
  echo "==> USER TESTING PASS → $OUT"
else
  echo "Fail" >"$OUT/STATUS"
  echo "==> USER TESTING FAIL → $OUT" >&2
fi
exit "$CODE"

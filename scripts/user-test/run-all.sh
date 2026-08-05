#!/usr/bin/env bash
# AI field-validation battery entrypoint (docs/USER_TESTING.md).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

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

mkdir -p "$OUT/logs"

echo "==> Building tif"
cargo build -p tif

echo "==> Running tif-e2e user-testing battery"
set +e
cargo test -p tif-e2e --all-targets -- --nocapture 2>&1 | tee "$OUT/cargo-test.log"
CODE=${PIPESTATUS[0]}
set -e

# Structured evidence pack: per-scenario results from cargo test log.
python3 - "$OUT" <<'PY' || python - "$OUT" <<'PY'
import json, os, re, sys
from datetime import datetime, timezone

out = sys.argv[1]
log_path = os.path.join(out, "cargo-test.log")
log = open(log_path, encoding="utf-8", errors="replace").read() if os.path.isfile(log_path) else ""

known = {
    "a01_": "A01", "a02_": "A02", "a03_": "A03", "a04_": "A04", "a05_": "A05",
    "a06_": "A06", "a07_": "A07", "a08_": "A08", "a09_": "A09", "a10_": "A10",
    "b01_": "B01", "b05_": "B05", "b06_": "B06", "b07_": "B07",
    "b11_": "B11", "b12_": "B12", "b14_": "B14", "ut0_": "UT0",
}

def scenario_id(name: str) -> str:
    for p, sid in known.items():
        if name.startswith(p):
            return sid
    return name

pat = re.compile(r"^test\s+(\S+)\s+\.\.\.\s+(ok|FAILED|ignored)\s*$", re.M)
results = []
for m in pat.finditer(log):
    name, status = m.group(1), m.group(2)
    if status == "ignored":
        continue
    sid = scenario_id(name)
    results.append({
        "id": sid,
        "test": name,
        "pass": status == "ok",
        "duration_ms": 0,
        "notes": f"cargo test {name} → {status}",
        "artifact": "cargo-test.log",
    })

with open(os.path.join(out, "results.jsonl"), "w", encoding="utf-8") as f:
    for r in results:
        f.write(json.dumps(r) + "\n")

lines = ["# V1 checklist (auto)\n", "", "| Scenario | Test | Pass | Notes |", "|----------|------|------|-------|"]
for r in results:
    lines.append(f"| {r['id']} | {r['test']} | {'Pass' if r['pass'] else 'Fail'} | {r['notes']} |")
open(os.path.join(out, "checklist.md"), "w", encoding="utf-8").write("\n".join(lines) + "\n")

fails = [r for r in results if not r["pass"]]
inc = ["# Incidents\n", ""]
if fails:
    inc.append("Failed scenarios:\n")
    for r in fails:
        inc.append(f"- **{r['id']}** (`{r['test']}`): {r['notes']}\n")
else:
    inc.append("None.\n")
open(os.path.join(out, "incidents.md"), "w", encoding="utf-8").write("".join(inc))

meta = {
    "run_id": os.path.basename(out.rstrip("/\\")),
    "os": os.name,
    "harness": "scripts/user-test/run-all.sh",
    "exit_code": int(os.environ.get("TIF_E2E_EXIT", "0")),
    "log": "cargo-test.log",
    "scenario_count": len(results),
    "pass_count": sum(1 for r in results if r["pass"]),
    "generated_at": datetime.now(timezone.utc).isoformat(),
}
open(os.path.join(out, "meta.json"), "w", encoding="utf-8").write(json.dumps(meta, indent=2) + "\n")
print(f"Wrote evidence pack: {len(results)} scenarios ({meta['pass_count']} pass)")
PY

export TIF_E2E_EXIT="$CODE"
# Re-write meta with real exit if python ran before export; patch exit_code.
if [[ -f "$OUT/meta.json" ]] && command -v python3 >/dev/null 2>&1; then
  python3 -c "import json,os; p=os.path.join(r'''$OUT''','meta.json'); m=json.load(open(p)); m['exit_code']=$CODE; json.dump(m, open(p,'w'), indent=2); open(p,'a').write('\n')" 2>/dev/null || true
elif [[ -f "$OUT/meta.json" ]] && command -v python >/dev/null 2>&1; then
  python -c "import json,os; p=os.path.join(r'''$OUT''','meta.json'); m=json.load(open(p)); m['exit_code']=$CODE; json.dump(m, open(p,'w'), indent=2); open(p,'a').write('\n')" 2>/dev/null || true
fi

if [[ $CODE -eq 0 ]]; then
  echo "Pass" >"$OUT/STATUS"
  echo "==> USER TESTING PASS → $OUT"
else
  echo "Fail" >"$OUT/STATUS"
  echo "==> USER TESTING FAIL → $OUT" >&2
fi
exit "$CODE"

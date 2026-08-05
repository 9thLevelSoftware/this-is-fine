#!/usr/bin/env python3
"""Parse cargo test log into a user-testing evidence pack (docs/USER_TESTING.md)."""
from __future__ import annotations

import json
import os
import re
import sys
from datetime import datetime, timezone

KNOWN = {
    "a01_": "A01",
    "a02_": "A02",
    "a03_": "A03",
    "a04_": "A04",
    "a05_": "A05",
    "a06_": "A06",
    "a07_": "A07",
    "a08_": "A08",
    "a09_": "A09",
    "a10_": "A10",
    "b01_": "B01",
    "b05_": "B05",
    "b06_": "B06",
    "b07_": "B07",
    "b11_": "B11",
    "b12_": "B12",
    "b14_": "B14",
    "ut0_": "UT0",
}


def scenario_id(name: str) -> str:
    for p, sid in KNOWN.items():
        if name.startswith(p):
            return sid
    return name


def main() -> int:
    if len(sys.argv) < 2:
        print("Usage: write_evidence.py OUT_DIR [EXIT_CODE]", file=sys.stderr)
        return 2
    out = sys.argv[1]
    exit_code = int(sys.argv[2]) if len(sys.argv) > 2 else 0
    os.makedirs(out, exist_ok=True)

    log_path = os.path.join(out, "cargo-test.log")
    log = ""
    if os.path.isfile(log_path):
        with open(log_path, encoding="utf-8", errors="replace") as f:
            log = f.read()

    pat = re.compile(r"^test\s+(\S+)\s+\.\.\.\s+(ok|FAILED|ignored)\s*$", re.M)
    results = []
    for m in pat.finditer(log):
        name, status = m.group(1), m.group(2)
        if status == "ignored":
            continue
        results.append(
            {
                "id": scenario_id(name),
                "test": name,
                "pass": status == "ok",
                "duration_ms": 0,
                "notes": f"cargo test {name} → {status}",
                "artifact": "cargo-test.log",
            }
        )

    with open(os.path.join(out, "results.jsonl"), "w", encoding="utf-8") as f:
        for r in results:
            f.write(json.dumps(r) + "\n")

    lines = [
        "# V1 checklist (auto)",
        "",
        "| Scenario | Test | Pass | Notes |",
        "|----------|------|------|-------|",
    ]
    for r in results:
        lines.append(
            f"| {r['id']} | {r['test']} | {'Pass' if r['pass'] else 'Fail'} | {r['notes']} |"
        )
    with open(os.path.join(out, "checklist.md"), "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")

    fails = [r for r in results if not r["pass"]]
    inc = ["# Incidents", ""]
    if fails:
        inc.append("Failed scenarios:")
        for r in fails:
            inc.append(f"- **{r['id']}** (`{r['test']}`): {r['notes']}")
    else:
        inc.append("None.")
    with open(os.path.join(out, "incidents.md"), "w", encoding="utf-8") as f:
        f.write("\n".join(inc) + "\n")

    meta = {
        "run_id": os.path.basename(out.rstrip("/\\")),
        "os": os.name,
        "harness": "scripts/user-test/write_evidence.py",
        "exit_code": exit_code,
        "log": "cargo-test.log",
        "scenario_count": len(results),
        "pass_count": sum(1 for r in results if r["pass"]),
        "generated_at": datetime.now(timezone.utc).isoformat(),
    }
    with open(os.path.join(out, "meta.json"), "w", encoding="utf-8") as f:
        json.dump(meta, f, indent=2)
        f.write("\n")

    print(
        f"Wrote evidence pack: {len(results)} scenarios ({meta['pass_count']} pass), exit_code={exit_code}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

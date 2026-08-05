# AI agent playbook — Q4 (Unix install) + F5 (vendor day)

**Audience:** An AI coding agent (Claude Code, Codex, Cursor, Copilot Workspace, etc.) with shell access, network, and git write access to a checkout of [9thLevelSoftware/this-is-fine](https://github.com/9thLevelSoftware/this-is-fine).

**Goal:** Close residual **Must** items **Q4** (Unix half) and **F5** with real evidence, then open a PR that updates readiness docs honestly.

**Canonical human procedures:** [RELEASE_DRY_RUN.md](RELEASE_DRY_RUN.md), [F5_VENDOR_DAY.md](F5_VENDOR_DAY.md), checklist [V1_READINESS.md](V1_READINESS.md).

---

## Honesty rules (do not skip)

1. **Never mark Q4 or F5 Pass without running the commands on that OS** and recording date/operator/output snippets.
2. **Never use `--skip-verify` / `-SkipVerify`** for release dry-run evidence.
3. **Never claim F5 Pass for protocol-only smoke** (`B12`, adapter install without a real agent product, or hand-edit-only without product version).
4. If a step fails, record **Fail** or leave **In progress** with the error — do not rewrite criteria to match the failure.
5. Prefer a **dedicated branch + PR** for evidence updates; do not force-push main.
6. Do not publish tags (`v1.0.0`) unless the human operator explicitly asks.

---

## Current baseline (as of playbook authoring)

| Item | Status | Notes |
|------|--------|-------|
| **Q4 Windows** | **Pass** | `v0.1.1-rc.1` — see evidence log in [RELEASE_DRY_RUN.md](RELEASE_DRY_RUN.md) |
| **Q4 Unix** | **Pending** | Field dry-run of `scripts/install.sh` still required |
| **F5** | **In progress** | Runbook exists; Win **and** Unix real-agent rows required |
| **Release tag** | `v0.1.1-rc.1` | Assets + `SHA256SUMS` published. Binary may report `tif 0.1.0` — identify by **tag + SUMS**, not version alone. |

Default release tag for this playbook: **`v0.1.1-rc.1`**. Override with env `TIF_RELEASE_TAG` if a newer RC/GA tag is specified.

---

## Task map

| Task ID | Name | OS required | Automates? | Updates |
|---------|------|-------------|------------|---------|
| **T-Q4-UNIX** | Unix install dry-run | Linux or macOS | Yes (shell + network) | `RELEASE_DRY_RUN.md`, `V1_READINESS.md` Q4 |
| **T-Q4-WIN** | Windows install dry-run | Windows | Yes if not already Pass | Same (only if re-running) |
| **T-F5** | Vendor day E2E | Win **and** Unix each once | Partial — needs real agent product | `adapters/VERSION_MATRIX.md`, `V1_READINESS.md` F5 |

**Recommended order:** `T-Q4-UNIX` → `T-F5` on that same Unix host (reuse installed `tif`) → on a Windows host, `T-F5` (Q4 Windows already Pass).

---

# T-Q4-UNIX — Unix install dry-run

## Preconditions

- OS: Linux (x86_64 or aarch64) or macOS (x86_64 or arm64)
- Tools: `bash`, `curl`, `git`, `sha256sum` or `shasum`
- Network access to `github.com`
- Writable temp dir (e.g. `/tmp`)
- **Do not** require Rust for the release path

## Steps (run exactly)

```bash
set -euo pipefail

export TIF_RELEASE_TAG="${TIF_RELEASE_TAG:-v0.1.1-rc.1}"
export TIF_INSTALL_PREFIX="${TIF_INSTALL_PREFIX:-/tmp/tif-q4-dry-run}"
export WORKDIR="${WORKDIR:-/tmp/tif-q4-work}"

rm -rf "$WORKDIR" "$TIF_INSTALL_PREFIX"
mkdir -p "$WORKDIR"
cd "$WORKDIR"

git clone --depth 1 https://github.com/9thLevelSoftware/this-is-fine.git
cd this-is-fine
# Prefer main tip so uninstall/install scripts match current docs:
git fetch --depth 1 origin main
git checkout origin/main

# --- install (SUMS verified by default) ---
./scripts/install.sh --version "$TIF_RELEASE_TAG" --prefix "$TIF_INSTALL_PREFIX"
BIN="$TIF_INSTALL_PREFIX/bin/tif"
test -x "$BIN"

# Identity (record both; version string may lag the tag)
echo "=== tif --version ==="
"$BIN" --version
echo "=== install tag ==="
echo "$TIF_RELEASE_TAG"

# --- upgrade once (same tag; must succeed / be idempotent) ---
./scripts/install.sh --version "$TIF_RELEASE_TAG" --prefix "$TIF_INSTALL_PREFIX"
"$BIN" --version

# --- uninstall + secrets purge ---
./scripts/uninstall.sh --prefix "$TIF_INSTALL_PREFIX" --purge-secrets
if test -e "$BIN"; then
  echo "FAIL: binary still present after uninstall: $BIN" >&2
  exit 1
fi
echo "=== Q4 Unix: PASS ==="
```

### Optional: piped-install smoke (secondary; not a substitute)

Only if checkout-local install already passed. Document separately; do not replace checkout install.

```bash
# Accepts network fetch of sha256-verify.sh when not adjacent
curl -fsSL "https://raw.githubusercontent.com/9thLevelSoftware/this-is-fine/main/scripts/install.sh" \
  | bash -s -- --version "$TIF_RELEASE_TAG" --prefix /tmp/tif-q4-piped
/tmp/tif-q4-piped/bin/tif --version
```

## Pass criteria (all required)

- [ ] `install.sh` exits 0 without `--skip-verify`
- [ ] Binary at `$PREFIX/bin/tif` runs `--version`
- [ ] Second install (upgrade) exits 0
- [ ] `uninstall.sh --purge-secrets` removes the binary
- [ ] OS string recorded (e.g. `uname -a`)

## Fail / abort

- SUMS mismatch, missing asset for arch, network error, or binary still present after uninstall → leave Q4 **In progress**, open issue or PR comment with logs. Do **not** mark Pass.

## Evidence to capture (paste into PR body + docs)

```
Date: YYYY-MM-DD
Operator: <agent name or "AI field agent">
OS: <uname -srm>
Tag: v0.1.1-rc.1
tif --version: <exact output>
Prefix: /tmp/tif-q4-dry-run
Result: Pass | Fail
Notes: <any>
```

## Doc updates after Pass

1. **`docs/RELEASE_DRY_RUN.md`** — evidence log table: fill Unix row (date, OS, tag, Pass, operator).
2. **`docs/V1_READINESS.md`** — **Q4** row:
   - Evidence: append Unix field Pass with date (keep Windows Pass text).
   - Status: **`Pass`** only if **both** Windows and Unix field dry-runs are recorded.
   - Residual Must: remove Q4 item if Pass.
3. Branch name suggestion: `evidence/q4-unix-<YYYYMMDD>`
4. Commit message suggestion:

   ```
   docs: record Unix Q4 install dry-run Pass for v0.1.1-rc.1
   ```

---

# T-Q4-WIN — Windows install dry-run (only if re-needed)

Skip if evidence log already has **Windows Pass** for the tag under test.

```powershell
$ErrorActionPreference = "Stop"
$tag = if ($env:TIF_RELEASE_TAG) { $env:TIF_RELEASE_TAG } else { "v0.1.1-rc.1" }
$prefix = if ($env:TIF_INSTALL_PREFIX) { $env:TIF_INSTALL_PREFIX } else { Join-Path $env:TEMP "tif-q4-dry-run" }
$work = Join-Path $env:TEMP "tif-q4-work"

Remove-Item -Recurse -Force $work, $prefix -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $work | Out-Null
Set-Location $work
git clone --depth 1 https://github.com/9thLevelSoftware/this-is-fine.git
Set-Location this-is-fine
git fetch --depth 1 origin main
git checkout origin/main

.\scripts\install.ps1 -Version $tag -Prefix $prefix
& "$prefix\bin\tif.exe" --version
.\scripts\install.ps1 -Version $tag -Prefix $prefix
.\scripts\uninstall.ps1 -Prefix $prefix -PurgeSecrets
if (Test-Path "$prefix\bin\tif.exe") { throw "binary still present after uninstall" }
Write-Host "=== Q4 Windows: PASS ==="
```

---

# T-F5 — Real agent host E2E (vendor day)

## What counts as Pass

On **each** of **Windows** and **Unix**:

1. `tif` installed from the release tag (prefer after Q4 path).
2. One first-class adapter installed from `adapters/<name>/`.
3. The **real agent product** is present and identified by version (not just scripts).
4. Lifecycle: **begin → implement → complete → status** with JSON envelopes showing `protocol_version: 1` and success `ok: true` (not rejected/unverified).
5. Matrix row filled with product version, tif version **and** tag, OS, Pass, date/operator.

| Agent | Dir | Suggested product version command |
|-------|-----|-----------------------------------|
| Claude Code | `adapters/claude-code/` | `claude --version` (or product About) |
| Codex | `adapters/codex/` | product `--version` / About |
| Gemini CLI | `adapters/gemini-cli/` | product `--version` / About |
| OpenCode | `adapters/opencode/` | product `--version` / About |

**Pick one agent for F5 Must.** A second agent on one OS advances **P1-4** / **P1-7** (Should).

## Preconditions

- Q4 path available on this OS (or reuse existing good install)
- Real agent product binary/app installed **or** ability to install it
- Git for scratch repo
- Checkout of this-is-fine (for adapter install scripts)

## If no real agent product is available

1. Still run the **tif lifecycle** (below) with hand-edit — useful smoke.
2. Record result as **Protocol field re-run — not F5**.
3. **Do not** flip F5 to Pass.
4. Stop after documenting the gap (product not installed).

## Steps — Unix (bash)

```bash
set -euo pipefail

export TIF_RELEASE_TAG="${TIF_RELEASE_TAG:-v0.1.1-rc.1}"
export AGENT_ID="${AGENT_ID:-claude-code}"   # codex | gemini | opencode
export MODEL_ID="${MODEL_ID:-default}"
export TIF_PREFIX="${TIF_PREFIX:-$HOME/.local}"
export PATH="${TIF_PREFIX}/bin:$PATH"
export REPO_ROOT="${REPO_ROOT:-$PWD}"       # checkout of this-is-fine
export SCRATCH="${SCRATCH:-/tmp/tif-f5-scratch}"

# 0) Ensure tif on PATH from release (if missing, run T-Q4-UNIX with prefix ~/.local)
command -v tif
tif --version
echo "Install tag: $TIF_RELEASE_TAG"

# 1) Detect real agent product (adjust for AGENT_ID)
case "$AGENT_ID" in
  claude-code)
    if command -v claude >/dev/null 2>&1; then
      PRODUCT_VER="$(claude --version 2>&1 | head -1)"
    else
      echo "NO_PRODUCT: Claude Code CLI not found — cannot Pass F5" >&2
      PRODUCT_VER="MISSING"
    fi
    ADAPTER_DIR="$REPO_ROOT/adapters/claude-code"
    ;;
  codex)
    PRODUCT_VER="$(command -v codex >/dev/null && codex --version 2>&1 | head -1 || echo MISSING)"
    ADAPTER_DIR="$REPO_ROOT/adapters/codex"
    ;;
  gemini|gemini-cli)
    AGENT_ID="gemini"
    PRODUCT_VER="$(command -v gemini >/dev/null && gemini --version 2>&1 | head -1 || echo MISSING)"
    ADAPTER_DIR="$REPO_ROOT/adapters/gemini-cli"
    ;;
  opencode)
    PRODUCT_VER="$(command -v opencode >/dev/null && opencode --version 2>&1 | head -1 || echo MISSING)"
    ADAPTER_DIR="$REPO_ROOT/adapters/opencode"
    ;;
  *)
    echo "Unknown AGENT_ID=$AGENT_ID" >&2
    exit 2
    ;;
esac
echo "Product: $PRODUCT_VER"
echo "OS: $(uname -srm)"

# 2) Install adapter
if [[ -x "$ADAPTER_DIR/install.sh" ]]; then
  "$ADAPTER_DIR/install.sh"
else
  echo "Missing $ADAPTER_DIR/install.sh" >&2
  exit 1
fi

# 3) Scratch repo with verification plan (required for clean complete)
rm -rf "$SCRATCH"
mkdir -p "$SCRATCH" && cd "$SCRATCH"
git init
git config user.email "f5@example.com"
git config user.name "f5"
echo 'pub fn add(a: i32, b: i32) -> i32 { a + b }' > lib.rs
cat > .this-is-fine.toml <<'EOF'
version = 1
enabled = true
default_fire_level = 3
[verification]
commands = ["echo tif-ok"]
discover = false
[audit]
tier = "metadata"
EOF
git add -A && git commit -m "baseline"

tif init --force
tif on

# 4) Begin
BEGIN_JSON="$(tif run begin --task "F5 tiny fix" --json --agent "$AGENT_ID" --model "$MODEL_ID")"
echo "$BEGIN_JSON"
# Prefer jq if present; else sed
if command -v jq >/dev/null 2>&1; then
  RUN_ID="$(printf '%s' "$BEGIN_JSON" | jq -r '.data.run_id // .run_id // empty')"
  BEGIN_OK="$(printf '%s' "$BEGIN_JSON" | jq -r '.ok // empty')"
  BEGIN_PV="$(printf '%s' "$BEGIN_JSON" | jq -r '.protocol_version // empty')"
else
  RUN_ID="$(printf '%s' "$BEGIN_JSON" | sed -n 's/.*"run_id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
  BEGIN_OK=""; BEGIN_PV=""
fi
test -n "$RUN_ID"

# 5) Implement — prefer real agent; hand-edit only as fallback
if [[ "$PRODUCT_VER" != "MISSING" ]]; then
  # Instruct the product (non-interactive example; adapt to host):
  # claude -p "In this repo, append a comment // f5 touch to lib.rs and stop."
  # If non-interactive agent invocation is unavailable, open the product in this
  # directory and perform one minimal edit, then continue.
  echo ">>> Use the real agent product now in $SCRATCH, then re-run complete steps."
  echo ">>> If you can drive the product non-interactively, do so before complete."
fi
# Minimal tree change (allowed as fallback; F5 Pass still needs product version present)
echo '// f5 touch' >> lib.rs

# 6) Complete + status
COMPLETE_JSON="$(tif run complete "$RUN_ID" --from-git --json --verification-passed true)"
echo "$COMPLETE_JSON"
STATUS_JSON="$(tif status --json)"
echo "$STATUS_JSON"

# 7) Assertions
assert_ok_pv() {
  local label="$1" json="$2"
  if command -v jq >/dev/null 2>&1; then
    local ok pv
    ok="$(printf '%s' "$json" | jq -r '.ok // empty')"
    pv="$(printf '%s' "$json" | jq -r '.protocol_version // empty')"
    echo "$label ok=$ok protocol_version=$pv"
    [[ "$ok" == "true" ]] || { echo "FAIL: $label ok != true" >&2; return 1; }
    [[ "$pv" == "1" ]] || { echo "FAIL: $label protocol_version != 1" >&2; return 1; }
  else
    echo "$json" | grep -q '"ok"[[:space:]]*:[[:space:]]*true' || { echo "FAIL: $label missing ok:true" >&2; return 1; }
    echo "$json" | grep -q '"protocol_version"[[:space:]]*:[[:space:]]*1' || { echo "FAIL: $label missing protocol_version:1" >&2; return 1; }
  fi
}
assert_ok_pv begin "$BEGIN_JSON"
assert_ok_pv complete "$COMPLETE_JSON"
assert_ok_pv status "$STATUS_JSON"

if [[ "$PRODUCT_VER" == "MISSING" ]]; then
  echo "=== F5: NOT PASS (no real agent product). Protocol lifecycle OK. ==="
  exit 0
fi
echo "=== F5 OS slice candidate PASS (record matrix row) ==="
echo "PRODUCT_VER=$PRODUCT_VER"
echo "TIF_VER=$(tif --version)"
echo "TAG=$TIF_RELEASE_TAG"
echo "OS=$(uname -srm)"
echo "AGENT_ID=$AGENT_ID"
```

## Steps — Windows (PowerShell)

```powershell
$ErrorActionPreference = "Stop"
$tag = if ($env:TIF_RELEASE_TAG) { $env:TIF_RELEASE_TAG } else { "v0.1.1-rc.1" }
$agentId = if ($env:AGENT_ID) { $env:AGENT_ID } else { "claude-code" }
$modelId = if ($env:MODEL_ID) { $env:MODEL_ID } else { "default" }
$repoRoot = if ($env:REPO_ROOT) { $env:REPO_ROOT } else { (Get-Location).Path }
$scratch = if ($env:SCRATCH) { $env:SCRATCH } else { Join-Path $env:TEMP "tif-f5-scratch" }

# Ensure tif on PATH (after install.ps1 to user prefix or Q4 prefix added to PATH)
tif --version

$productVer = "MISSING"
switch ($agentId) {
  "claude-code" {
    if (Get-Command claude -ErrorAction SilentlyContinue) {
      $productVer = (claude --version 2>&1 | Select-Object -First 1 | Out-String).Trim()
    }
    $adapter = Join-Path $repoRoot "adapters\claude-code\install.ps1"
  }
  "codex" {
    if (Get-Command codex -ErrorAction SilentlyContinue) {
      $productVer = (codex --version 2>&1 | Select-Object -First 1 | Out-String).Trim()
    }
    $adapter = Join-Path $repoRoot "adapters\codex\install.ps1"
  }
  default { throw "Set AGENT_ID to a supported adapter; install.ps1 path may need mapping" }
}
Write-Host "Product: $productVer"
& $adapter

Remove-Item -Recurse -Force $scratch -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $scratch | Out-Null
Set-Location $scratch
git init
git config user.email "f5@example.com"
git config user.name "f5"
Set-Content -Path lib.rs -Value 'pub fn add(a: i32, b: i32) -> i32 { a + b }'
@"
version = 1
enabled = true
default_fire_level = 3
[verification]
commands = ["echo tif-ok"]
discover = false
[audit]
tier = "metadata"
"@ | Set-Content -Path .this-is-fine.toml -Encoding utf8
git add -A; git commit -m "baseline"
tif init --force
tif on

$begin = tif run begin --task "F5 tiny fix" --json --agent $agentId --model $modelId | ConvertFrom-Json
if (-not $begin.ok) { throw "begin not ok: $($begin | ConvertTo-Json -Compress)" }
$runId = $begin.data.run_id
Add-Content -Path lib.rs -Value "// f5 touch"
$complete = tif run complete $runId --from-git --json --verification-passed true | ConvertFrom-Json
$status = tif status --json | ConvertFrom-Json
if (-not $complete.ok) { throw "complete not ok" }
if (-not $status.ok) { throw "status not ok" }
if ($begin.protocol_version -ne 1 -or $complete.protocol_version -ne 1) { throw "protocol_version != 1" }

if ($productVer -eq "MISSING") {
  Write-Host "=== F5: NOT PASS (no real agent product). Protocol lifecycle OK. ==="
} else {
  Write-Host "=== F5 OS slice candidate PASS ==="
  Write-Host "PRODUCT_VER=$productVer TAG=$tag"
}
```

## Why verification flags matter

`tif run complete --from-git` without a verification plan and without `--verification-passed true` treats an empty plan as **incomplete** and fails the correctness floor → rejected/unverified run. The sample `.this-is-fine.toml` (or real agent-asserted verification) is required for clean F5 evidence. See [F5_VENDOR_DAY.md](F5_VENDOR_DAY.md).

## Doc updates after F5 Pass (both OSes)

1. **`docs/adapters/VERSION_MATRIX.md`** — add **vendor product** rows (not only protocol rows):

   | Agent | Product version tested | tif version / commit | OS | Mode | Result |
   |-------|------------------------|----------------------|----|------|--------|
   | Claude Code | `<product ver>` | `tif x.y.z` + tag `v0.1.1-rc.1` | Ubuntu 24.04 | **Vendor E2E** | **Pass** YYYY-MM-DD |
   | Claude Code | `<product ver>` | same | Windows 11 | **Vendor E2E** | **Pass** YYYY-MM-DD |

2. **`docs/V1_READINESS.md`** — **F5**:
   - Evidence: link matrix rows + operator/date.
   - Status: **`Pass`** only when **Win and Unix** vendor rows exist.
   - Residual Must: remove F5 item.

3. Optional Should: fill **P1-4** / **P1-7** if a second agent or more versions were run.

4. Commit message suggestion:

   ```
   docs: record F5 vendor day Pass (Win+Unix, <agent>)
   ```

## F5 Pass criteria checklist

- [ ] Real agent product version string recorded (not `MISSING`)
- [ ] Adapter install ran on that OS
- [ ] `begin` / `complete` / `status` JSON: `ok: true`, `protocol_version: 1`
- [ ] Scratch repo used verification plan or `--verification-passed true`
- [ ] Repeated for **second OS**
- [ ] Matrix + V1_READINESS updated in a PR

---

## End-to-end agent procedure (single prompt style)

Copy this block as the agent task:

```text
You are closing v1 readiness residual Must items for 9thLevelSoftware/this-is-fine.

Read docs/AGENT_Q4_F5_PLAYBOOK.md and follow it exactly, including honesty rules.

1) If this host is Linux or macOS: run T-Q4-UNIX against tag v0.1.1-rc.1.
   On Pass, update docs/RELEASE_DRY_RUN.md and docs/V1_READINESS.md Q4 as specified.
2) Run T-F5 on this OS for AGENT_ID=claude-code (or first available product).
   If product MISSING, do not mark F5 Pass; still capture protocol lifecycle output.
3) Open a PR with evidence-only doc changes. Include command transcripts in the PR body.
4) Do not tag v1.0.0. Do not use --skip-verify. Do not mark Pass without evidence.

If this host is Windows: skip Q4 if already Pass in RELEASE_DRY_RUN.md; run T-F5 only.
If you cannot access a second OS, complete what you can and leave F5 In progress with a clear residual.
```

---

## PR template for evidence

```markdown
## Summary
- [ ] Q4 Unix field dry-run
- [ ] F5 Unix vendor slice
- [ ] F5 Windows vendor slice

## Evidence
### Q4
- Tag:
- OS:
- `tif --version`:
- Result:

### F5
- Agent product + version:
- OS(es):
- Matrix rows:
- Result:

## Honesty
- [ ] No --skip-verify
- [ ] F5 Pass only with real product versions on required OSes
```

---

## Out of scope (do not do in this playbook)

- Tagging `v1.0.0` / Q3 versioning discipline
- PM signature on P1-2 package channel
- Cosign P1-1 (optional extra; only if keys and release `.sig` exist)
- Claiming multi-agent N1 matrix complete after a single agent F5

---

## Quick reference links

| Doc | Role |
|-----|------|
| [RELEASE_DRY_RUN.md](RELEASE_DRY_RUN.md) | Human Q4 procedure + evidence log |
| [F5_VENDOR_DAY.md](F5_VENDOR_DAY.md) | Human F5 checklist |
| [V1_READINESS.md](V1_READINESS.md) | Must/Should status board |
| [adapters/VERSION_MATRIX.md](adapters/VERSION_MATRIX.md) | F5 evidence table |
| [user-guide.md](user-guide.md) | Install/uninstall user docs |
| Release `v0.1.1-rc.1` | https://github.com/9thLevelSoftware/this-is-fine/releases/tag/v0.1.1-rc.1 |

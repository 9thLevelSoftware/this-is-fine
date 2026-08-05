# F5 vendor day — real agent host E2E

**Purpose:** Satisfy Must **F5** (and optionally **P1-4**, **P1-7**) by exercising one first-class adapter **inside a real agent product** on **Windows and Unix**.

**AI agents:** executable task map, honesty rules, and end-to-end prompt: [AGENT_Q4_F5_PLAYBOOK.md](AGENT_Q4_F5_PLAYBOOK.md) (`T-F5`).

Protocol/install smoke (**B12**, **F5-smoke**) is **not** enough for F5 Pass.

## Choose one agent

| Agent | Installer | Success criteria |
|-------|-----------|------------------|
| Claude Code | `adapters/claude-code/` | Session hook runs `tif`; begin → work → complete → status |
| Codex | `adapters/codex/` | Bridge + AGENTS snippet; same lifecycle |
| Gemini CLI | `adapters/gemini-cli/` | Context generator + lifecycle |
| OpenCode | `adapters/opencode/` | Plugin hooks + lifecycle |

## Per OS (repeat on Win and Unix)

1. Install `tif` from the RC release (see [RELEASE_DRY_RUN.md](RELEASE_DRY_RUN.md)).  
2. Record identity (do **not** trust the tag alone if crate version lagged a prior tag):  
   ```bash
   tif --version
   # also note the release tag you installed (e.g. v0.1.1-rc.1)
   ```  
3. Install the adapter from the matching directory (`install.sh` / `install.ps1`).  
4. Install / open the **real** agent product (note exact product version).  
5. Create a **scratch git repo with verification configured**, then run the lifecycle:

   ```bash
   mkdir /tmp/tif-f5-scratch && cd /tmp/tif-f5-scratch
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

   AGENT_ID="claude-code"   # or codex / gemini / opencode
   MODEL_ID="default"
   BEGIN_JSON=$(tif run begin --task "tiny fix" --json --agent "$AGENT_ID" --model "$MODEL_ID")
   echo "$BEGIN_JSON"
   RUN_ID=$(printf '%s' "$BEGIN_JSON" | sed -n 's/.*"run_id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
   # If sed fails, copy run_id from BEGIN_JSON manually into RUN_ID.

   # Implement a minimal change in the agent (or hand-edit):
   echo '// f5 touch' >> lib.rs

   # After the agent (or you) finished the edit:
   tif run complete "$RUN_ID" --from-git --json --verification-passed true
   tif status --json
   ```

   **PowerShell equivalent (run_id extraction):**

   ```powershell
   $begin = tif run begin --task "tiny fix" --json --agent "claude-code" --model "default" | ConvertFrom-Json
   $runId = $begin.data.run_id
   # edit files...
   tif run complete $runId --from-git --json --verification-passed true
   tif status --json
   ```

6. Confirm success envelopes use `protocol_version: 1` and `ok: true` (not rejected/unverified).  
7. Optional: force OOC and confirm Firebreak approve/reject if sensitive paths apply.

### Why verification is required

`tif run complete --from-git` without a verification plan and without `--verification-passed true` treats an empty plan as **incomplete** and fails the correctness floor. The sample `.this-is-fine.toml` above (or an agent-asserted `--verification-passed true` after real checks) is required for clean F5 evidence.

## Record results

Update [adapters/VERSION_MATRIX.md](adapters/VERSION_MATRIX.md) with:

| Field | Example |
|-------|---------|
| Agent product version | Claude Code 1.x.y |
| tif version / tag | `tif --version` output **and** install tag `v0.1.1-rc.1` |
| OS | Windows 11 / Ubuntu 24.04 |
| Result | Pass |
| Date / operator | YYYY-MM-DD, name |
| Log link | gist / issue / CI artifact |

Then flip **F5** in [V1_READINESS.md](V1_READINESS.md) to **Pass** with a link to that matrix row.

## Time budget

~30–60 minutes per OS if the agent product is already installed.

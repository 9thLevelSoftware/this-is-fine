# F5 vendor day — real agent host E2E

**Purpose:** Satisfy Must **F5** (and optionally **P1-4**, **P1-7**) by exercising one first-class adapter **inside a real agent product** on **Windows and Unix**.

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
2. `tif --version` records the version under test.  
3. Install the adapter from the matching directory (`install.sh` / `install.ps1`).  
4. Install / open the **real** agent product (note exact product version).  
5. In a scratch git repo:  
   ```bash
   tif init && tif on
   tif run begin --task "tiny fix" --json --agent <id> --model <id>
   # implement a minimal change in the agent
   tif run complete <run_id> --from-git --json
   tif status --json
   ```  
6. Confirm envelopes use `protocol_version: 1` and `ok: true` for success paths.  
7. Optional: force OOC and confirm Firebreak approval/reject if sensitive paths apply.

## Record results

Update [adapters/VERSION_MATRIX.md](adapters/VERSION_MATRIX.md) with:

| Field | Example |
|-------|---------|
| Agent product version | Claude Code 1.x.y |
| tif version / tag | `v0.1.1-rc.1` |
| OS | Windows 11 / Ubuntu 24.04 |
| Result | Pass |
| Date / operator | YYYY-MM-DD, name |
| Log link | gist / issue / CI artifact |

Then flip **F5** in [V1_READINESS.md](V1_READINESS.md) to **Pass** with a link to that matrix row.

## Time budget

~30–60 minutes per OS if the agent product is already installed.

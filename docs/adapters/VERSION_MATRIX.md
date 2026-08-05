# Adapter version matrix (P1-7)

Records which agent product versions were exercised against which `tif` protocol version.

Protocol: **v1** (`protocol_version: 1`) — see [protocol/v1.md](../protocol/v1.md).  
Automated coverage: `tif_core::adapter_conformance` + USER_TESTING **B12** (lifecycle) and **F5 Pass (protocol)**.

| Agent | Product version tested | tif version / commit | OS | Mode | Result |
|-------|------------------------|----------------------|----|------|--------|
| Claude Code | *hooks install docs* | workspace `0.1.0` | Win + Unix scripts present | Protocol (JSON lifecycle) | Pass (protocol) via B12 + conformance |
| Codex | *AGENTS snippet + bridge* | workspace `0.1.0` | Win + Unix | Protocol | Pass (protocol) |
| Gemini CLI | *context generators* | workspace `0.1.0` | Win + Unix | Protocol | Pass (protocol) |
| OpenCode | *plugin hooks* | workspace `0.1.0` | Win + Unix | Protocol | Pass (protocol) |

**Vendor product E2E (GUI / real agent binary):** not yet filled — required for **F5 Pass (vendor)** and **P1-4** second-agent vendor claim. Fill a row with exact product version when a human or cloud agent runs a real install day.

## How to update

1. Install adapter from `adapters/<name>/`.  
2. Run begin → implement → complete → status with `--json`.  
3. Record product version, OS, tif commit SHA, date, and link to log or PR.

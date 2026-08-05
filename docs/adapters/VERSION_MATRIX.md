# Adapter version matrix (P1-7)

Records which agent product versions were exercised against which `tif` protocol version.

Protocol: **v1** (`protocol_version: 1`) — see [protocol/v1.md](../protocol/v1.md).  
Automated coverage: `tif_core::adapter_conformance` + USER_TESTING **B12** (lifecycle) and **F5 Pass (protocol)**.

| Agent | Product version tested | tif version / commit | OS | Mode | Result |
|-------|------------------------|----------------------|----|------|--------|
| Claude Code | `2.1.219 (Claude Code)` | `tif 0.1.0` (tag `v0.1.1-rc.1`) | macOS (Darwin 25.5.0 arm64) | **Vendor E2E** | **Pass (Unix slice)** 2026-08-05 |
| OpenCode | `1.18.13` | `tif 0.1.0` (tag `v0.1.1-rc.1`) | Windows 10 (10.0.26200.0) | **Vendor E2E** | **Pass (Windows slice)** 2026-08-05 |
| Claude Code | *hooks install docs* | workspace `0.1.0` | Win + Unix scripts present | Protocol (JSON lifecycle) | Protocol-ready (B12 + conformance) — **not F5** |
| Codex | *AGENTS snippet + bridge* | workspace `0.1.0` | Win + Unix | Protocol | Protocol-ready — **not F5** |
| Gemini CLI | *context generators* | workspace `0.1.0` | Win + Unix | Protocol | Protocol-ready — **not F5** |
| OpenCode | *plugin hooks* | workspace `0.1.0` | Win + Unix | Protocol | Protocol-ready — **not F5** |

**Vendor product E2E (GUI / real agent binary):** Unix and Windows slices recorded above for OpenCode/Claude Code on 2026-08-05; both required OS rows are now present for Must F5.

## How to update

1. Install adapter from `adapters/<name>/`.  
2. Run begin → implement → complete → status with `--json`.  
3. Record product version, OS, tif commit SHA, date, and link to log or PR.

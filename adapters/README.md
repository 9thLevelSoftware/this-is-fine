# Agent Adapters

This Is Fine integrates with coding agents through a **versioned JSON CLI protocol**. Each first-class agent uses:

1. **Native hooks, skills, rules, or instructions** to inject pressure and containment policy.
2. A **thin adapter** that calls the Rust core (`tif … --json`).
3. An optional wrapper only where native lifecycle controls are insufficient.

## Protocol

Protocol version: **1** (see `tif-core::protocol::PROTOCOL_VERSION` and [`docs/protocol/v1.md`](../docs/protocol/v1.md)).

Canonical operations:

```text
tif policy resolve --json
tif run begin --json --task "…"
tif run complete --json <run_id> --from-git
tif assess --json --from-git
tif firebreak --json [--candidate PATH --apply]
tif verify --json
tif rollback --json <run_id>
tif audit show --json
```

Responses use the envelope:

```json
{
  "protocol_version": 1,
  "ok": true,
  "data": { }
}
```

On failure: `"ok": false` and `"error": "…"`. Adapters **must not** treat failed envelopes as success.

## First-class adapters (installable artifacts)

| Agent | Directory | Install (Unix) | Install (Windows) | Artifacts |
|---|---|---|---|---|
| Claude Code | [`claude-code/`](claude-code/) | `install.sh` | `install.ps1` | `SKILL.md`, session hooks |
| Codex | [`codex/`](codex/) | `install.sh` | `install.ps1` | `AGENTS.snippet.md`, `tif-bridge.*` |
| Gemini CLI | [`gemini-cli/`](gemini-cli/) | `install.sh` | `install.ps1` | `generate-context.*` |
| OpenCode | [`opencode/`](opencode/) | `install.sh` | `install.ps1` | `plugin.json`, begin/complete hooks |

Each adapter directory has **`install.md`** plus platform installers. Adapters are intentionally thin: policy compilation, scoring, verification, Firebreak, isolation, and audit remain in the Rust core.

### Quick install

```bash
# Unix
./adapters/claude-code/install.sh
./adapters/codex/install.sh ./AGENTS.md
./adapters/gemini-cli/install.sh
./adapters/opencode/install.sh
```

```powershell
# Windows PowerShell
.\adapters\claude-code\install.ps1
.\adapters\codex\install.ps1 -TargetAgentsMd .\AGENTS.md
.\adapters\gemini-cli\install.ps1
.\adapters\opencode\install.ps1
```

## Adapter responsibilities

- Announce task start and completion
- Request a compiled policy (`tif policy resolve`)
- Inject or reference the policy through supported native mechanisms
- Supply task text, repository path, and agent/model identity
- Submit completion metrics (`--from-git` preferred)
- Display compact status
- Invoke Firebreak and rollback

## Shell safety (argv preferred)

Adapters **prefer argv arrays** over shell string interpolation:

| Bad | Good |
|-----|------|
| `eval "tif run begin --task $TASK"` | `tif run begin --json --task "$TASK"` |
| PowerShell `Invoke-Expression` | `& tif --repo $Repo … --task $Task` |
| Unquoted `$RUN_ID` in complete | `"${RUN_ID}"` / `-RunId $RunId` |

Hooks must quote task text and run ids. Never pipe untrusted agent output into `sh -c`. Verification commands belong in `.this-is-fine.toml`, not adapter-built shell strings.

## Conformance

Workspace tests exercise the JSON protocol envelope and sample adapter payloads (`crates/tif-core` adapter_conformance module). After installing an adapter:

```bash
tif status
tif policy resolve --json --task "smoke test"
# expect protocol_version: 1 and ok: true
```

## Local-only rule

Adapters must not authorize new hosted reviewer models. Source egress requires explicit local configuration of a user-authorized reviewer (`allow_source_egress`).

## Troubleshooting

| Symptom | Likely cause | Fix |
|---------|--------------|-----|
| `tif: command not found` / not on PATH | CLI not installed | `scripts/install.sh` / `scripts/install.ps1`, or `cargo install --path crates/tif` |
| `ok: false` on `policy resolve` | Missing/invalid config | `tif init`; check `version = 1` in `.this-is-fine.toml` |
| `ok: false` on `run begin` | Containment suspended | `tif on` or pass `--force` |
| Session hook exits 0 but no run | Soft-fail when `tif` missing | Set `TIF_REQUIRED=1` to fail hard; install `tif` |
| JSON parse errors in agent | Human diagnostics on stdout | Always pass `--json`; read stdout only for the envelope |
| `protocol_version` unsupported | Adapter older/newer than core | Bump adapter or pin tif; refuse major ≠ 1 |
| Firebreak never applies | No authorized reviewers / approval required | Configure `[[reviewers]]` in `.this-is-fine.local.toml`; `tif approve <run_id>` |
| PowerShell execution policy | Scripts blocked | `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned` or `powershell -File …` |
| Spaces in task break hooks | Unquoted expansion | Use quoted `"${TASK}"` / PowerShell parameters (shipped hooks already do this) |
| Complete fails with run not found | Wrong repo / run id | Pass same `--repo` / `TIF_REPO` used at begin; check `tif audit show` |
| Windows line endings on `.sh` | CRLF from git | Enable `core.autocrlf` carefully or run under Git Bash/WSL |
| Gemini context file empty | `jq` missing and parse failed | Install `jq` optional; scripts fall back to raw JSON |
| OpenCode hooks not firing | Plugin not wired | Follow `opencode/inject.md` and `plugin.json` paths |

### Debug checklist

1. `tif --version` and `tif status`
2. `tif policy resolve --json --task "debug"` — validate envelope
3. Confirm `.this-is-fine/` is writable
4. Re-run the adapter install script for your OS
5. See [`docs/user-guide.md`](../docs/user-guide.md) recovery runbook for dual-failure / rollback

# Agent Adapters

This Is Fine integrates with coding agents through a **versioned JSON CLI protocol**. Each first-class agent uses:

1. **Native hooks, skills, rules, or instructions** to inject pressure and containment policy.
2. A **thin adapter** that calls the Rust core (`tif … --json`).
3. An optional wrapper only where native lifecycle controls are insufficient.

## Protocol

Protocol version: **1** (see `tif-core::protocol::PROTOCOL_VERSION`).

Canonical operations:

```text
tif policy resolve --json
tif run begin --json --task "…"
tif run complete --json <run_id> --lines-added N …
tif assess --json --lines-added N …
tif firebreak --json
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

On failure: `"ok": false` and `"error": "…"`.

Human-readable CLI output and JSON are generated from the same domain results.

## Adapter responsibilities

An adapter must be able to:

- Announce task start and completion
- Request a compiled policy (`tif policy resolve`)
- Inject or reference the policy through supported native mechanisms
- Supply task text (or a safe digest), repository path, and agent/model identity
- Submit completion metrics / diff summary
- Display compact status (`🔥 Containment active · Fire Level N`)
- Invoke Firebreak and rollback operations

## First-class adapters

| Agent | Directory | Notes |
|---|---|---|
| Claude Code | [`claude-code/`](claude-code/) | Skills / hooks injection |
| Codex | [`codex/`](codex/) | Instructions + CLI bridge |
| Gemini CLI | [`gemini-cli/`](gemini-cli/) | Context file / flags |
| OpenCode | [`opencode/`](opencode/) | Plugin-style bridge |

Adapters are intentionally thin. Policy compilation, scoring, verification, Firebreak, and audit remain in the Rust core.

## Local-only rule

Adapters must not authorize new hosted reviewer models. Source egress requires explicit local configuration of a user-authorized reviewer.

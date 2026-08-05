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

On failure: `"ok": false` and `"error": "…"`.

## First-class adapters (installable artifacts)

| Agent | Directory | Artifacts |
|---|---|---|
| Claude Code | [`claude-code/`](claude-code/) | `SKILL.md`, session hook, `install.md` |
| Codex | [`codex/`](codex/) | `AGENTS.snippet.md`, `tif-bridge.sh`, `install.md` |
| Gemini CLI | [`gemini-cli/`](gemini-cli/) | `generate-context.sh`, `install.md` |
| OpenCode | [`opencode/`](opencode/) | `plugin.json`, `inject.md`, `install.md` |

Each adapter directory has an **`install.md`** with activation steps. Adapters are intentionally thin: policy compilation, scoring, verification, Firebreak, isolation, and audit remain in the Rust core.

## Adapter responsibilities

- Announce task start and completion
- Request a compiled policy (`tif policy resolve`)
- Inject or reference the policy through supported native mechanisms
- Supply task text, repository path, and agent/model identity
- Submit completion metrics (`--from-git` preferred)
- Display compact status
- Invoke Firebreak and rollback

## Local-only rule

Adapters must not authorize new hosted reviewer models. Source egress requires explicit local configuration of a user-authorized reviewer.

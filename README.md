# This Is Fine

**Contain the fire. Do not remodel the building.**

This Is Fine is a **local-first** adaptive restraint and simplification system for coding agents. It reduces unnecessary code, files, dependencies, abstractions, and token usage while enforcing a strict **correctness floor**.

> This Is Fine is a repository-aware simplicity governor for coding agents. It applies controlled pressure, measurable containment policies, and verified simplification to produce the smallest correct implementation.

## Product vocabulary

| Technical concept | Product term |
|---|---|
| Stress-prompt context | **Pressure Scenario** |
| Anti-bloat rules | **Containment Policy** |
| Aggressiveness | **Fire Level** |
| Repository discovery | **Source Inspection** |
| Diff and policy report | **Damage Assessment** |
| Automatic simplification | **Firebreak** |
| Avoidable additions | **Fuel Added** |
| Successful result | **Contained** |
| Excessive result | **Out of Control** |
| Emergency recovery | **Five-Alarm** |

## Status

Foundation is solid and fail-safe. **Production-grade delivery** (real reviewer backends, automatic Firebreak loop, Five-Alarm recovery, full adapters/TUI/distribution) is tracked in [`docs/ROADMAP.md`](docs/ROADMAP.md).

## Features (current)

- **Rust core** with `tif` CLI (JSON protocol for agents)
- **Configuration**: `.this-is-fine.toml` + `.this-is-fine.local.toml`
- **Fire Levels 1–5** (Five-Alarm is post-failure escalation only)
- **Policy compiler**, pressure scenarios, task classification
- **Simplicity scoring** with weights, hard limits, correctness-floor gate
- **Verification** planner/runner (explicit config first, safe discovery second)
- **Firebreak** with user-authorized reviewers only (fail-safe; auto LLM backend in progress)
- **Audit** store: SQLite + content-addressed artifacts (local only)
- **Isolation** with real apply/rollback: Git worktree and non-Git snapshot
- **Git-aware metrics** (`tif assess --from-git`, unified diff parse)
- **Interactive TUI** (`tif tui`) — Dashboard, Current Run, Damage Assessment, and more
- **CI templates** for GitHub Actions and GitLab (read-only by default)
- **Installable adapters** for Claude Code, Codex, Gemini CLI, OpenCode
- **Provider feature flags** on `tif-core` (`provider-mock` default; HTTP/process backends planned)

## Install

### From source

```bash
cargo install --path crates/tif
```

Or build the workspace:

```bash
cargo build --release -p tif
# binary: target/release/tif
```

Requirements: Rust 1.75+ (for building). End users of release binaries need no Rust toolchain.

### Platforms

Windows, macOS, and Linux are supported. Paths and shell invocation are cross-platform aware.

## Quick start

```bash
# In your repository
tif init
tif on
tif status

# Compile pressure + policy for a task
tif policy resolve --task "fix null pointer in parser"

# Begin a containment run (agent adapters call this)
tif run begin --task "fix null pointer in parser" --agent claude-code

# After implementation: score metrics (explicit or from git)
tif assess --files-changed 1 --lines-added 12 --deps-added 0
tif assess --from-git

# Plan/run verification
tif verify --dry-run
tif verify

# Fire Level
tif fire-level
tif fire-level 4

# Interactive TUI (keyboard: ↑↓ / 1-8 / r refresh / q quit)
tif tui

# Suspend containment for exploratory work
tif off
```

### JSON (agent adapters)

```bash
tif policy resolve --json --task "add feature X"
tif run begin --json --task "add feature X" --agent codex
tif assess --json --lines-added 40 --deps-added 1
tif audit show --json
```

## Configuration

| File | Purpose |
|---|---|
| `.this-is-fine.toml` | Shared, committed repository policy |
| `.this-is-fine.local.toml` | Machine-local reviewers, credentials, endpoints (gitignored) |
| `.this-is-fine/` | Local SQLite audit DB and artifacts |

Illustrative shared config:

```toml
version = 1
enabled = true
default_fire_level = 3

[verification]
commands = [
  "cargo fmt --check",
  "cargo clippy --all-targets --all-features -- -D warnings",
  "cargo test --all-features"
]

[simplicity.weights]
runtime_dependency = 100
new_file = 25
public_interface = 20
abstraction = 15
added_line = 1
unrelated_change = 50

[simplicity.limits]
new_runtime_dependencies = 0

[approval]
sensitive_paths = ["src/auth/**", "migrations/**"]

[audit]
tier = "redacted"   # metadata | redacted | full
max_age_days = 90
max_size_mb = 1024

[rollback]
max_days = 7
successful_commits = 3
```

### Fire Levels

| Level | Name | Behavior |
|---|---|---|
| 1 | Ember | Light brevity and reuse guidance |
| 2 | Smolder | Stronger YAGNI pressure |
| 3 | Containment | Default guarded mode |
| 4 | Critical | Aggressive reduction |
| 5 | Five-Alarm | Escalation only after current-task containment failure |

## Correctness floor

Minimalism is bounded by a non-negotiable gate. A smaller incorrect candidate can **never** defeat a larger correct candidate. Adaptation cannot lower the floor. No unverified Firebreak may replace a known-good implementation.

## CLI reference (core)

```text
tif init
tif on | off
tif status
tif inspect
tif policy resolve [--task …] [--fire-level N]
tif run begin|complete|show|status
tif assess [--files-added N] [--lines-added N] [--deps-added N]
tif verify [--dry-run]
tif firebreak [--run-id …] [--candidate PATH] [--apply]
tif assess --from-git | --from-diff PATH
tif fire-level [1-4]
tif rollback <run_id>
tif audit show| --gc | --purge
tif five-alarm --plan
tif adaptation
tif tui
```

Add `--json` for the adapter protocol. See [`docs/protocol/v1.md`](docs/protocol/v1.md).

## Architecture (crates)

```text
crates/tif-core   # domain library (+ providers feature flags)
crates/tif        # CLI + TUI binary
adapters/         # installable agent adapters
ci/               # consumer-repo GitHub Actions + GitLab templates
.github/workflows # multi-OS CI for this repository
docs/             # design, protocol, config, security, roadmap
```

## Privacy

- No cloud telemetry
- No unauthorized hosted models
- Audit data stays on disk under `.this-is-fine/`
- Source egress only when a user-authorized reviewer allows it

## Visual identity

Product motifs are original (controlled flame in a terminal, extinguisher-as-brace, etc.). Do **not** copy the well-known “This Is Fine” comic artwork.

## Development

```bash
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

MIT OR Apache-2.0

## Documentation

| Doc | Description |
|-----|-------------|
| [Design specification](docs/superpowers/specs/2026-08-04-this-is-fine-design.md) | Product and system design |
| [Production roadmap](docs/ROADMAP.md) | Phases 0–10 toward GA |
| [Protocol v1](docs/protocol/v1.md) | Adapter JSON contract |
| [Config schema v1](docs/config/schema-v1.md) | Configuration reference |
| [Threat model](docs/security/threat-model.md) | Security boundaries |
| [Versioning](docs/VERSIONING.md) | SemVer / schema / protocol |
| [Changelog](CHANGELOG.md) | Release notes |

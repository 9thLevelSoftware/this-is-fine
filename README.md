# This Is Fine

<p align="center">
  <img src="assets/this-is-fine-banner.png" alt="Dog in a burning room, calmly saying THIS IS FINE." width="720" />
</p>

<p align="center">
  <strong>Let's turn up the heat!</strong><br/>
  <em>v1.0.0 — production release. The house is still on fire. The coffee is excellent.</em>
</p>

---

**This Is Fine** (`tif`) is a **local-first** restraint system for coding agents. When your AI co-pilot decides the bugfix needs a new microservice, three abstraction layers, and a dependency on `left-pad-redux`, This Is Fine is the calm dog who says: *maybe just fix the null check*.

It applies controlled pressure, measurable containment policies, and **verified** simplification so you get the **smallest correct** implementation — not the smallest *interesting* one.

> Minimalism is bounded by a non-negotiable **correctness floor**.  
> A smaller wrong answer can never beat a larger right one.  
> We will not ship vibes.

---

## Why this exists

Coding agents are great at *adding*. They are… less great at *stopping*.

This Is Fine sits next to your agent (Claude Code, Codex, Gemini CLI, OpenCode, or anything that speaks JSON) and:

1. **Turns up the heat** (Fire Levels 1–4) so the agent feels social pressure to stay small  
2. **Scores the damage** (files, lines, deps, abstractions — Fuel Added)  
3. **Verifies** the result still works  
4. Runs a **Firebreak** if things go Out of Control (isolated simplify → re-verify → approve)  
5. Escalates to **Five-Alarm** only after current containment has already failed (we do not open with the fire hose)

Everything stays on your machine. No cloud telemetry. No surprise model calls. Your secrets stay in your burning living room, where they belong.

---

## Install (no Rust required)

Pick your preferred level of firey goodness.

### One-liner (recommended)

```bash
# Unix / macOS / WSL — installs latest release, verifies SHA-256 against SHA256SUMS
curl -fsSL https://raw.githubusercontent.com/9thLevelSoftware/this-is-fine/main/scripts/install.sh | bash
```

```powershell
# Windows PowerShell
irm https://raw.githubusercontent.com/9thLevelSoftware/this-is-fine/main/scripts/install.ps1 | iex
```

Pin a version:

```bash
./scripts/install.sh --version v1.0.0
```

```powershell
.\scripts\install.ps1 -Version v1.0.0
```

Then confirm the dog is house-trained:

```bash
tif --version   # → tif 1.0.0
```

> Prefer checkout-local install so SUMS helpers resolve offline-friendly: clone the repo, then `./scripts/install.sh --version v1.0.0`.

### From source (for people who *want* more fire)

```bash
cargo install --path crates/tif
# or
cargo build --release -p tif   # → target/release/tif
```

Requires **Rust 1.75+**. End users of release binaries need no Rust — we already suffered for you.

### Uninstall (when the coffee runs out)

```bash
./scripts/uninstall.sh --prefix ~/.local
# optional: also purge credential secrets dirs
./scripts/uninstall.sh --prefix ~/.local --purge-secrets
```

```powershell
.\scripts\uninstall.ps1 -PurgeSecrets
```

Release assets + checksums: [GitHub Releases](https://github.com/9thLevelSoftware/this-is-fine/releases).

---

## 60-second quick start

```bash
cd your-perfectly-normal-repo   # smoke optional

tif init          # lay down config + local state
tif on            # start containment (the dog sits down)
tif status        # how bad is it, really?

# Tell the agent (or yourself) what the policy wants
tif policy resolve --task "fix null pointer in parser"

# Agent adapters call this around real work
tif run begin --task "fix null pointer in parser" --agent claude-code --json

# …agent implements the tiniest correct fix…

tif run complete <run_id> --from-git --json --verification-passed true
tif assess --from-git
tif status --json
```

Suspend containment when you're deliberately exploring (yes, that's allowed):

```bash
tif off    # temporary leave of absence for the dog
tif on     # back to work
```

Interactive command center (keyboard: `↑↓` / `1-8` / `r` refresh / `q` quit):

```bash
tif tui
```

Full recovery runbook (dual-failure, hung verify, disk full): **[docs/user-guide.md](docs/user-guide.md)**.

---

## Agent adapters

Wire This Is Fine into a real agent product. Protocol smoke is not enough for glory — use the real host when you can.

| Agent | Install |
|-------|---------|
| Claude Code | `adapters/claude-code/install.sh` / `install.ps1` |
| Codex | `adapters/codex/install.sh` / `install.ps1` |
| Gemini CLI | `adapters/gemini-cli/install.sh` / `install.ps1` |
| OpenCode | `adapters/opencode/install.sh` / `install.ps1` |

Always pass **`--json`**. Refuse envelopes with unknown major `protocol_version`. Spec: [`docs/protocol/v1.md`](docs/protocol/v1.md).

---

## Product vocabulary

Because "bloat" lacked *panache*.

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

### Fire Levels

| Level | Name | Vibes |
|---|---|---|
| 1 | Ember | "Maybe we don't need a monorepo." |
| 2 | Smolder | Stronger YAGNI side-eye |
| 3 | Containment | **Default.** Guarded mode. Sip coffee. |
| 4 | Critical | Aggressive reduction. The dog is still smiling. |
| 5 | Five-Alarm | **Escalation only** after current-task containment failure. Not a lifestyle. |

```bash
tif fire-level        # show
tif fire-level 4      # turn up the heat (1–4 operational)
```

---

## Configuration

| File | Purpose |
|---|---|
| `.this-is-fine.toml` | Shared, committed repository policy |
| `.this-is-fine.local.toml` | Machine-local reviewers, credentials, endpoints (**gitignored**) |
| `.this-is-fine/` | Local SQLite audit DB and artifacts (your black box recorder) |

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

Schema reference: [`docs/config/schema-v1.md`](docs/config/schema-v1.md).

---

## Correctness floor (the part that is not a joke)

- A smaller **incorrect** candidate can **never** defeat a larger **correct** candidate  
- Adaptation cannot weaken the floor, sensitive-path rules, or required verification  
- No unverified Firebreak may replace a known-good implementation  
- Source egress only with explicit reviewer permission  

If the house is on fire *and* the tests fail, we do not redecorate. We put the fire out.

---

## CLI cheat sheet

```text
tif init | on | off | status
tif inspect
tif policy resolve [--task …] [--fire-level N] [--json]
tif run begin|complete|show|status
tif assess [--from-git] [--files-changed N] [--lines-added N] [--deps-added N]
tif verify [--dry-run]
tif firebreak [--run-id …] [--candidate PATH] [--apply]
tif fire-level [1-4]
tif rollback <run_id>
tif audit show | --gc | --purge
tif five-alarm --plan | --run <run_id> [--apply]
tif adaptation status | recommend | reset
tif tui
```

Add `--json` for the adapter protocol.

---

## What you get in v1.0

- **Rust core** + `tif` CLI (JSON protocol for agents)  
- **Firebreak** closed loop with user-authorized reviewers (isolation + re-verify + approval)  
- **Five-Alarm** staged recovery after current containment failure  
- **Production TUI** — live events, approve/reject, probe, audit filter  
- **Adapters** for Claude Code, Codex, Gemini CLI, OpenCode (Unix + Windows)  
- **Install scripts** with SHA-256 SUMS verification; multi-OS release assets  
- **Audit** store: SQLite + content-addressed artifacts (local only)  
- **CI templates** (GitHub Actions / GitLab) — read-only by default  

Field evidence and readiness: [`docs/V1_READINESS.md`](docs/V1_READINESS.md).

---

## Architecture (for the curious)

```text
crates/tif-core   # domain library (+ provider feature flags)
crates/tif        # CLI + TUI binary
crates/tif-e2e    # field-validation battery
adapters/         # installable agent adapters
ci/               # consumer-repo CI templates
docs/             # design, protocol, security, readiness
```

---

## Privacy

- No cloud telemetry  
- No unauthorized hosted models  
- Audit data stays under `.this-is-fine/`  
- Reviewers only from the user-authorized local pool  

---

## Support / when it is *not* fine

**Owner:** [9thLevelSoftware](https://github.com/9thLevelSoftware) maintainers.

1. Recovery runbook → [`docs/user-guide.md`](docs/user-guide.md)  
2. Open a GitHub issue with label `incident` (or `P0` for data loss / silent bad apply)  
3. Attach redacted `tif audit --json`, OS, and `tif --version` when possible  

Best-effort community support. Not a 24×7 fire department.

---

## Development

```bash
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

---

## Documentation

| Doc | Description |
|-----|-------------|
| [User guide](docs/user-guide.md) | Install, adapters, recovery |
| [V1 readiness](docs/V1_READINESS.md) | Must/Should checklist + evidence |
| [Protocol v1](docs/protocol/v1.md) | Adapter JSON contract |
| [Config schema v1](docs/config/schema-v1.md) | Configuration reference |
| [Threat model](docs/security/threat-model.md) | Security boundaries |
| [Versioning](docs/VERSIONING.md) | SemVer / schema / protocol |
| [Adapters](adapters/README.md) | Agent install + troubleshooting |
| [Design specification](docs/superpowers/specs/2026-08-04-this-is-fine-design.md) | Full product design |
| [Changelog](CHANGELOG.md) | Release notes |
| [Roadmap](docs/ROADMAP.md) | Phases 0–10 |

---

## License

MIT OR Apache-2.0

---

<p align="center">
  <em>This is fine.</em><br/>
  <sub>Please verify your Firebreak candidates. The dog is not a unit test.</sub>
</p>

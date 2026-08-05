# This Is Fine — User guide

**Contain the fire. Do not remodel the building.**

This guide covers day-to-day use, recovery, and distribution. Design depth lives in [`superpowers/specs/2026-08-04-this-is-fine-design.md`](superpowers/specs/2026-08-04-this-is-fine-design.md). Protocol details: [`protocol/v1.md`](protocol/v1.md).

## Install (no Rust required for release binaries)

### Unix

```bash
curl -fsSL https://raw.githubusercontent.com/9thLevelSoftware/this-is-fine/main/scripts/install.sh | bash
# or from a checkout:
./scripts/install.sh --from-source
```

### Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/9thLevelSoftware/this-is-fine/main/scripts/install.ps1 | iex
# or:
.\scripts\install.ps1 -FromSource
```

### From source

```bash
cargo install --path crates/tif
```

Package managers: Homebrew formula stub in [`dist/homebrew/this-is-fine.rb`](../dist/homebrew/this-is-fine.rb); WinGet notes in [`dist/winget/README.md`](../dist/winget/README.md).

### Verify release installs (checksums)

Release install scripts **require** a matching `SHA256SUMS` entry for the downloaded asset (unless you pass `--skip-verify` / `-SkipVerify`).

Manual check:

```bash
# Unix
curl -fsSLO "https://github.com/9thLevelSoftware/this-is-fine/releases/download/vX.Y.Z/SHA256SUMS"
curl -fsSLO "https://github.com/9thLevelSoftware/this-is-fine/releases/download/vX.Y.Z/tif-x86_64-unknown-linux-gnu.tar.gz"
sha256sum -c SHA256SUMS --ignore-missing
```

Optional cosign (when the release publishes `*.sig` and you have the public key):

```bash
export COSIGN_PUBLIC_KEY=/path/to/cosign.pub
export TIF_REQUIRE_COSIGN=1
./scripts/install.sh --version vX.Y.Z
```

See also [V1_READINESS.md](V1_READINESS.md) items **Q4**, **Q5**, **P1-1**.

## Quick start

```bash
cd your-repo
tif init
tif on
tif status
tif policy resolve --task "fix null pointer in parser"
tif tui   # interactive; requires a TTY
```

## Agent adapters

See [`adapters/README.md`](../adapters/README.md). Each first-class agent has Unix + Windows installers:

| Agent | Unix | Windows |
|-------|------|---------|
| Claude Code | `adapters/claude-code/install.sh` | `install.ps1` |
| Codex | `adapters/codex/install.sh` | `install.ps1` |
| Gemini CLI | `adapters/gemini-cli/install.sh` | `install.ps1` |
| OpenCode | `adapters/opencode/install.sh` | `install.ps1` |

Always pass `--json` from adapters. Refuse envelopes with unknown major `protocol_version`.

## Firebreak and approval

```bash
tif run complete --run-id ID --from-git --auto-firebreak
tif firebreak --auto --run-id ID
tif approve <run_id>    # AwaitingApproval
tif reject <run_id>
tif rollback <run_id>
```

In the TUI (`tif tui`): open **Firebreak**, select a run, `a` approve / `x` reject / `R` rollback (with confirmation).

## Support contact

- **Primary:** GitHub Issues — https://github.com/9thLevelSoftware/this-is-fine/issues (label `incident` / `P0` for blockers)  
- **Runbook:** this document (recovery sections below)  
- **Security reports:** prefer private disclosure if available; otherwise open a security-labeled issue without secrets  

v0.1 is best-effort community support, not a committed SLA.

## Recovery runbook

### 1. Containment suspended

```bash
tif status
tif on
```

### 2. Run stuck / not found

```bash
tif audit show
tif run show <run_id>
```

Confirm the same `--repo` / `TIF_REPO` used at `run begin`.

### 3. Firebreak did not apply

- No authorized reviewers → configure `[[reviewers]]` in `.this-is-fine.local.toml`
- Sensitive path or `require_firebreak_approval` → `tif approve <run_id>`
- Candidate larger or failed re-verify → original retained (fail-closed); inspect audit events

### 4. Dual-failure: apply failed and restore failed (`restore_pending`)

Symptoms: error mentions `restore also failed` / `restore_pending=true`; workspace may be partially modified.

```bash
# Prefer automatic retry via rollback
tif rollback <run_id> --json

# Inspect session flags
tif run show <run_id> --json
```

If rollback fails (missing baseline):

1. Do **not** run further Firebreak applies on this repo until restored.
2. Recover files from git (`git checkout -- .` / `git restore`) if the project is git-backed and dirty state is known.
3. Isolation baselines live under `.this-is-fine/snapshots/` — if present, support can re-run restore from the baseline directory.
4. After recovery: `tif audit show` and start a **new** run; do not reuse a dual-failure session without restore.

### 5. Hung verification

Verification commands are killed after the runner timeout (default 600s). Hung children are process-group killed on Unix. Re-run:

```bash
tif verify --json
```

If a command routinely hangs, fix the project script or split it; do not disable the correctness floor.

### 6. Disk full / I/O errors

Isolation apply/snapshot surfaces honest I/O errors (including no-space). Free disk under the repo and `.this-is-fine/`, then:

```bash
tif audit --gc
tif rollback <run_id>   # if apply left sticky state
```

### 7. Purge local audit (destructive)

```bash
tif audit --purge
# or TUI Audit screen → P (confirm y)
```

### 8. Five-Alarm escalation

Only after **current** containment failure (not historical risk alone):

```bash
tif five-alarm --plan
tif five-alarm --run <run_id> [--apply]
```

## CI

Read-only templates: [`ci/github-actions/this-is-fine.yml`](../ci/github-actions/this-is-fine.yml), [`ci/gitlab/this-is-fine.yml`](../ci/gitlab/this-is-fine.yml).

Write path requires **both** config (`ci.allow_write`) and CI variable (`TIF_ALLOW_WRITE=true`). Patch artifacts only; no force-push to protected branches.

## Configuration pointers

| File | Purpose |
|------|---------|
| `.this-is-fine.toml` | Shared policy (commit) |
| `.this-is-fine.local.toml` | Reviewers / credentials (gitignore) |
| `.this-is-fine/` | SQLite audit, snapshots, locks |

Full schema: [`config/schema-v1.md`](config/schema-v1.md).

## Privacy

- No cloud telemetry
- No unauthorized hosted models
- Source egress only when a local reviewer sets `allow_source_egress = true`

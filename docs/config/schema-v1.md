# Configuration schema v1

**Schema version field:** `version = 1`  
**Shared file (commit):** `.this-is-fine.toml`  
**Local file (gitignored):** `.this-is-fine.local.toml`  
**State directory:** `.this-is-fine/` (audit DB, artifacts, isolation)

## Merge order

Later layers override earlier ones **field-wise** (including nested option limits):

1. Built-in defaults  
2. Shared `.this-is-fine.toml`  
3. Local `.this-is-fine.local.toml`  
4. Task / Fire Level (policy compile time)  
5. Adaptive recommendations (when self-apply allowlisted)  
6. Explicit CLI overrides  

Mismatching `version` → hard error (`UnsupportedSchema`).

## Top-level fields

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `version` | u32 | required `1` | Schema version |
| `enabled` | bool | `true` | Containment on/off |
| `default_fire_level` | u8 | `3` | **1–4 only**; Five-Alarm forbidden |
| `exclusions` | string[] | `[]` | Glob paths excluded from metrics/isolation |

## `[verification]`

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `commands` | string[] | `[]` | Explicit shell commands (trusted config) |
| `discover` | bool | `true` | Safe discovery when commands empty |

**Rules:** empty + discover-off ⇒ incomplete plan (not a vacuous pass). Control characters and `$()` / `` ` `` rejected in command strings.

## `[simplicity.weights]`

Penalties (higher = more expensive fuel). Defaults:

| Field | Default |
|-------|---------|
| `runtime_dependency` | 100 |
| `new_file` | 25 |
| `public_interface` | 20 |
| `abstraction` | 15 |
| `added_line` | 1 |
| `unrelated_change` | 50 |
| `configuration_surface` | 10 |
| `generated_code` | 5 |
| `duplication` | 12 |

## `[simplicity.limits]`

Hard caps (`null` / omitted = no cap). Partial local tables **merge field-wise** and do not wipe shared caps.

| Field | Type | Example |
|-------|------|---------|
| `new_runtime_dependencies` | u32? | `0` |
| `new_files` | u32? | |
| `public_interfaces` | u32? | |
| `abstractions` | u32? | |
| `added_lines` | u32? | |
| `score` | f64? | max weighted score |

## `[simplicity.exceptions]`

Optional string list of justified exceptions (paths/globs or free-text rationales). Recorded on Damage Assessments as **audit notes only** — never bypasses the correctness floor.

```toml
[simplicity]
exceptions = ["vendor/** third-party pin", "generated protobuf stubs"]
```

## `[approval]`

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `sensitive_paths` | string[] | `[]` | Globs requiring Firebreak approval |
| `sensitive_task_classes` | string[] | `[]` | Task categories requiring approval |
| `require_firebreak_approval` | bool | `false` | Always require approval for apply |
| `auto_apply_firebreak` | bool | **`true`** | Auto-apply non-sensitive candidates after re-verify + ranking |
| `approval_ttl_hours` | u32? | | Optional hours until pending approval expires |

## `[audit]`

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `tier` | string | `"redacted"` | `metadata` \| `redacted` \| `full` |
| `max_age_days` | u32 | `90` | Retention age |
| `max_size_mb` | u32 | `1024` | Retention size budget |

## `[rollback]`

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `max_days` | u32 | `7` | Baseline retention |
| `successful_commits` | u32 | `3` | Or N good commits after apply (git when available); whichever comes first with `max_days` |

## `[ci]`

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `on_violation` | string | `"fail"` | `fail` \| `warn` |
| `allow_write` | bool | `false` | Optional CI write path |
| `allow_pr` | bool | `false` | Optional PR/MR generation |

## `[pressure]`

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `include_baseline` | bool | `true` | Baseline containment prompt |
| `allowed_families` | string[] | `[]` | Empty = all curated families |

Families include: `production_incident`, `release_freeze`, `limited_maintenance`, `context_fire`, `breach_containment`.

## `[[reviewers]]` (local config)

Only user-authorized models. Never auto-added by adaptation.

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `id` | string | required | Stable id |
| `provider` | string | required | `mock`, `openai_compatible` (aliases: `openai`, `ollama`), `anthropic`, `process` |
| `model` | string | required | Model id |
| `endpoint` | string? | | HTTPS base URL (OpenAI-compatible) or API host |
| `credential_ref` | string? | | `ENV_NAME`, `env:NAME`, or `file:PATH` |
| `allow_source_egress` | bool | **`false`** | Hosted source leave machine |
| `eligible_task_types` | string[] | `[]` | Empty = all |
| `max_firebreak_attempts` | u32 | `2` | |
| `priority` | i32 | `0` | Higher preferred |
| `timeout_secs` | u64 | `120` | HTTP/process timeout |
| `max_input_tokens` | u64? | | Advisory for context packaging |
| `max_output_tokens` | u64? | | Provider max_tokens |
| `max_context_bytes` | u64 | `256000` | Truncate packaged prompts |
| `process_argv` | string[]? | | For `process`: argv with `{isolation}` / `{request_json}` |

## Validation error catalog (actionable)

| Condition | User-facing guidance |
|-----------|----------------------|
| `version != 1` | Upgrade config schema or install a tif that supports this version |
| `default_fire_level == 5` | Five-Alarm cannot be default; use 1–4 |
| Invalid audit tier | Use `metadata`, `redacted`, or `full` |
| Invalid CI mode | Use `fail` or `warn` |
| Empty reviewer id | Set unique `[[reviewers]].id` |
| Malformed TOML | Fix syntax; `tif` will not silently default on `verify` / `run complete` |

## Example shared file

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
discover = false

[simplicity.limits]
new_runtime_dependencies = 0

[approval]
sensitive_paths = ["src/auth/**", "migrations/**"]

[audit]
tier = "redacted"
max_age_days = 90

[ci]
on_violation = "fail"
allow_write = false
allow_pr = false
```

## Example local file (never commit)

```toml
version = 1

[[reviewers]]
id = "local-sim"
provider = "mock"
model = "fixture"
allow_source_egress = false
priority = 10

[[reviewers]]
id = "hosted-spare"
provider = "openai_compatible"
model = "gpt-example"
endpoint = "https://api.example.com/v1"
credential_ref = "TIF_REVIEWER_API_KEY"
allow_source_egress = false
priority = 1
```

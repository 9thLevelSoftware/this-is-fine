# Versioning policy

## Crate / product version (SemVer)

Workspace package version (currently `1.0.0`) follows SemVer for the `tif` binary and `tif-core` crate:

| Bump | When |
|------|------|
| **MAJOR** | Breaking CLI flags, removed commands, or breaking `tif-core` public API used by external crates |
| **MINOR** | Backward-compatible features (new commands, additive JSON fields) |
| **PATCH** | Bug fixes, docs, internal refactors |

From 1.0.0 onward: breaking CLI/`tif-core` public API changes require a MAJOR bump. Avoid silent protocol breaks (use `protocol_version`).

## Config schema version

Field: `version` in `.this-is-fine.toml` / local file.

- Independent of crate SemVer.  
- Current: **1**.  
- Unknown versions are **hard rejected** with upgrade guidance.  
- Migrations land as explicit functions when v2 is introduced.

## Adapter protocol version

Field: `protocol_version` in every JSON response.

- Current: **1**.  
- Additive fields allowed without bump.  
- Breaking envelope/semantics → **2** with dual-support window if needed.

## Scoring version

When Damage Assessment or weight semantics change in a way that breaks adaptation comparability, stamp `scoring_version` on assessments (currently `"1"` via `SCORING_VERSION`). Bump when weight semantics change.

## Release artifacts

- Git tags: `vMAJOR.MINOR.PATCH`  
- Checksums and optional signatures accompany binaries (Phase 9).  
- Install channels (WinGet/Scoop/Homebrew) consume the same artifacts.

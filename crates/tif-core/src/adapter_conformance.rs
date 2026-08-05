//! Adapter JSON protocol conformance helpers and tests.
//!
//! Exercises the versioned envelope with **mocked CLI outputs** (no real agent,
//! no network). Adapters must parse these shapes safely and refuse bad majors.

use serde_json::Value;

use crate::protocol::PROTOCOL_VERSION;

/// Minimum checks every adapter must apply to a CLI JSON envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvelopeCheck {
    pub protocol_version: u32,
    pub ok: bool,
    pub has_data: bool,
    pub error: Option<String>,
}

/// Parse and validate a raw CLI stdout string as a protocol envelope.
pub fn check_envelope(raw: &str) -> Result<EnvelopeCheck, String> {
    let v: Value = serde_json::from_str(raw).map_err(|e| format!("invalid json: {e}"))?;
    let protocol_version = v
        .get("protocol_version")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| "missing protocol_version".to_string())? as u32;
    let ok = v
        .get("ok")
        .and_then(|x| x.as_bool())
        .ok_or_else(|| "missing ok".to_string())?;
    let has_data = v.get("data").map(|d| !d.is_null()).unwrap_or(false);
    let error = v
        .get("error")
        .and_then(|e| e.as_str())
        .map(|s| s.to_string());
    if protocol_version != PROTOCOL_VERSION {
        return Err(format!(
            "unsupported protocol_version {protocol_version} (expected {PROTOCOL_VERSION})"
        ));
    }
    if ok && error.is_some() {
        // Soft warning shape still allowed; adapters should prefer data.
    }
    if !ok && error.is_none() {
        return Err("ok:false without error message".into());
    }
    Ok(EnvelopeCheck {
        protocol_version,
        ok,
        has_data,
        error,
    })
}

/// Whether adapters should proceed after this envelope (ok + supported version).
pub fn adapter_may_proceed(raw: &str) -> bool {
    matches!(check_envelope(raw), Ok(c) if c.ok)
}

/// Mock CLI stdout samples used by conformance tests and docs.
pub mod mocks {
    /// Successful `tif policy resolve --json` (envelope-level; adapters read compact_status + pressure).
    pub const POLICY_RESOLVE_OK: &str = r#"{
  "protocol_version": 1,
  "ok": true,
  "data": {
    "policy": {
      "policy_id": "mock-policy",
      "policy_version": "1",
      "compiled_at": "2026-01-01T00:00:00Z",
      "enabled": true,
      "fire_level": "containment",
      "task_category": "bug_fix",
      "pressure": {
        "family": "baseline",
        "template_id": "baseline-v1",
        "template_version": "1",
        "fire_level": "containment",
        "body": "CONTAINMENT MODE\nContain the fire.",
        "compact_status": "🔥 Containment active · Fire Level 3"
      },
      "verification": { "commands": [], "discover": true },
      "weights": {
        "runtime_dependency": 100.0,
        "new_file": 25.0,
        "public_interface": 20.0,
        "abstraction": 15.0,
        "added_line": 1.0,
        "unrelated_change": 50.0
      },
      "limits": {
        "new_runtime_dependencies": null,
        "new_files": null,
        "public_interfaces": null,
        "abstractions": null,
        "added_lines": null,
        "score": null
      },
      "sensitive_paths": [],
      "require_firebreak_approval": false,
      "exclusions": [],
      "notes": ["task_category=bug_fix"]
    },
    "compact_status": "🔥 Containment active · Fire Level 3"
  }
}"#;

    pub const RUN_BEGIN_OK: &str = r#"{
  "protocol_version": 1,
  "ok": true,
  "data": {
    "run_id": "run-mock-001",
    "state": "implementing",
    "policy": null,
    "compact_status": "🔥 Containment active · Fire Level 3"
  }
}"#;

    pub const RUN_BEGIN_FAIL: &str = r#"{
  "protocol_version": 1,
  "ok": false,
  "error": "containment is suspended; use tif on or --force"
}"#;

    pub const VERIFY_OK: &str = r#"{
  "protocol_version": 1,
  "ok": true,
  "data": {
    "report": {
      "checks": [],
      "all_required_passed": true,
      "has_unresolved": false,
      "has_unresolved_required": false,
      "incomplete_plan": false
    }
  }
}"#;

    pub const ROLLBACK_OK: &str = r#"{
  "protocol_version": 1,
  "ok": true,
  "data": {
    "run_id": "run-mock-001",
    "restored": true,
    "reason": "restored",
    "message": "Restored original from baseline"
  }
}"#;

    pub const UNSUPPORTED_MAJOR: &str = r#"{
  "protocol_version": 99,
  "ok": true,
  "data": {}
}"#;

    pub const MALFORMED: &str = r#"{ not json"#;
}

/// Static patterns adapters must avoid (shell injection / unsafe interpolation).
pub fn shell_script_looks_unsafe(script: &str) -> Vec<&'static str> {
    let mut hits = Vec::new();
    let lower = script.to_ascii_lowercase();
    // Word-boundary-ish: reject shell `eval` and PowerShell Invoke-Expression.
    for line in lower.lines() {
        let t = line.trim_start();
        if t.starts_with('#') {
            continue; // comments may mention eval in docs
        }
        if t.starts_with("eval ")
            || t.starts_with("eval\"")
            || t.starts_with("eval'")
            || t == "eval"
        {
            hits.push("eval");
            break;
        }
        if t.contains("invoke-expression") {
            hits.push("Invoke-Expression");
            break;
        }
    }
    if script.contains("sh -c \"$TASK\"") || script.contains("bash -c \"$TASK\"") {
        hits.push("shell -c on TASK");
    }
    // Bash: unquoted --task $VAR is unsafe. PowerShell `& tif … --task $Task` is
    // argv-safe (parameter binding), so only flag shell scripts.
    let looks_like_powershell = lower.contains("param(")
        || lower.contains("$erroractionpreference")
        || lower.contains("invoke-")
        || script.contains(".ps1");
    if !looks_like_powershell
        && (script.contains("--task $TASK")
            || script.contains("--task $1")
            || script.contains("--task $Task"))
    {
        hits.push("unquoted --task $VAR");
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::policy::{PolicyCompileRequest, PolicyCompiler};
    use crate::protocol::{
        emit_json, AssessResult, JsonResponse, PolicyResolveResult, RollbackResult, RunBeginResult,
        VerifyResult,
    };
    use crate::scoring::DiffMetrics;
    use crate::FireLevel;

    #[test]
    fn envelope_accepts_policy_resolve_mock() {
        let c = check_envelope(mocks::POLICY_RESOLVE_OK).expect("envelope");
        assert_eq!(c.protocol_version, 1);
        assert!(c.ok);
        assert!(c.has_data);
        assert!(adapter_may_proceed(mocks::POLICY_RESOLVE_OK));
    }

    #[test]
    fn envelope_rejects_unsupported_major() {
        let err = check_envelope(mocks::UNSUPPORTED_MAJOR).unwrap_err();
        assert!(err.contains("unsupported protocol_version"));
        assert!(!adapter_may_proceed(mocks::UNSUPPORTED_MAJOR));
    }

    #[test]
    fn envelope_fail_closed_on_error() {
        let c = check_envelope(mocks::RUN_BEGIN_FAIL).unwrap();
        assert!(!c.ok);
        assert!(c.error.is_some());
        assert!(!adapter_may_proceed(mocks::RUN_BEGIN_FAIL));
    }

    #[test]
    fn envelope_rejects_malformed() {
        assert!(check_envelope(mocks::MALFORMED).is_err());
    }

    #[test]
    fn typed_deserialize_run_begin_mock() {
        let env: JsonResponse<RunBeginResult> =
            serde_json::from_str(mocks::RUN_BEGIN_OK).expect("typed");
        assert!(env.ok);
        assert_eq!(env.protocol_version, PROTOCOL_VERSION);
        let data = env.data.expect("data");
        assert_eq!(data.run_id, "run-mock-001");
    }

    #[test]
    fn typed_deserialize_policy_resolve_mock() {
        let env: JsonResponse<PolicyResolveResult> =
            serde_json::from_str(mocks::POLICY_RESOLVE_OK).expect("policy typed");
        assert!(env.ok);
        let data = env.data.expect("data");
        assert!(data.compact_status.contains("Fire Level") || !data.compact_status.is_empty());
        assert!(data.policy.pressure.body.contains("CONTAINMENT"));
    }

    #[test]
    fn typed_deserialize_rollback_mock() {
        let env: JsonResponse<RollbackResult> =
            serde_json::from_str(mocks::ROLLBACK_OK).expect("typed");
        assert!(env.data.unwrap().restored);
    }

    #[test]
    fn real_policy_compile_emits_conforming_envelope() {
        let cfg = Config::default();
        let policy = PolicyCompiler::new()
            .compile(
                &cfg,
                &PolicyCompileRequest {
                    task_text: Some("fix null pointer".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let data = PolicyResolveResult {
            compact_status: crate::audit::compact_status(policy.fire_level.as_u8(), policy.enabled),
            policy,
        };
        let raw = emit_json(&JsonResponse::ok(data)).unwrap();
        let c = check_envelope(&raw).expect("live compile envelope");
        assert!(c.ok);
        assert_eq!(c.protocol_version, PROTOCOL_VERSION);
        // Round-trip typed
        let _: JsonResponse<PolicyResolveResult> = serde_json::from_str(&raw).unwrap();
    }

    #[test]
    fn verify_and_assess_mock_shapes() {
        let env: JsonResponse<VerifyResult> =
            serde_json::from_str(mocks::VERIFY_OK).expect("verify mock");
        assert!(env.data.unwrap().report.all_required_passed);
        let assess = JsonResponse::ok(AssessResult {
            assessment: crate::assess::DamageAssessor::build(
                "mock",
                "original",
                FireLevel::Containment,
                DiffMetrics::default(),
                crate::scoring::SimplicityScorer::new(Default::default(), Default::default())
                    .score(
                        &DiffMetrics::default(),
                        &crate::scoring::CorrectnessFloor::all_pass(),
                    ),
                crate::scoring::CorrectnessFloor::all_pass(),
                None,
            ),
        });
        let raw = emit_json(&assess).unwrap();
        assert!(adapter_may_proceed(&raw));
    }

    #[test]
    fn shipped_adapter_hooks_prefer_safe_quoting() {
        // Paths relative to crate (workspace root may vary); scan embedded samples.
        let samples = [
            include_str!("../../../adapters/claude-code/hooks/tif-session.sh"),
            include_str!("../../../adapters/codex/tif-bridge.sh"),
            include_str!("../../../adapters/gemini-cli/generate-context.sh"),
            include_str!("../../../adapters/opencode/hooks/tif-begin.sh"),
            include_str!("../../../adapters/opencode/hooks/tif-complete.sh"),
            include_str!("../../../adapters/claude-code/hooks/tif-session.ps1"),
            include_str!("../../../adapters/codex/tif-bridge.ps1"),
            include_str!("../../../adapters/opencode/hooks/tif-begin.ps1"),
            include_str!("../../../adapters/opencode/hooks/tif-complete.ps1"),
        ];
        for (i, s) in samples.iter().enumerate() {
            let hits = shell_script_looks_unsafe(s);
            assert!(
                hits.is_empty(),
                "adapter sample {i} has unsafe patterns: {hits:?}"
            );
        }
    }

    #[test]
    fn err_envelope_helper() {
        let raw = emit_json(&JsonResponse::<()>::err("boom")).unwrap();
        let c = check_envelope(&raw).unwrap();
        assert!(!c.ok);
        assert_eq!(c.error.as_deref(), Some("boom"));
    }
}

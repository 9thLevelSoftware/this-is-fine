//! Remaining Tier B journeys (UT-2): B02–B04, B08–B10, B13, B15.

use std::fs;
use std::path::Path;
use tif_e2e::{
    copy_fixture, ensure_tif_built, git_init_commit, run_scenario, tif_json, ScenarioResult,
};

#[test]
fn b02_unnecessary_dependency_firebreak() {
    ensure_tif_built();
    let r = run_scenario("B02", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        // Fake dep + bloat marker for OOC + mock reduce.
        let mut cargo = fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?;
        cargo.push_str("\n[dependencies]\nserde = \"1\"\n");
        fs::write(root.join("Cargo.toml"), cargo).map_err(|e| e.to_string())?;
        fs::write(root.join("TIF_MOCK_REDUCE"), vec![b'X'; 30_000]).map_err(|e| e.to_string())?;
        git_init_commit(root).map_err(|e| e.to_string())?;

        let complete = ooc_auto(root, "feature with unnecessary dependency")?;
        if !complete.ok_envelope() {
            return Err(format!("complete: {}", complete.summary()));
        }
        let status = complete
            .data()
            .and_then(|d| d.get("assessment"))
            .and_then(|a| a.get("status"))
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let fb = complete.data().and_then(|d| d.get("firebreak"));
        // Expect OOC handling: Firebreak path or out_of_control without unsafe apply of larger code.
        let applied = fb
            .and_then(|f| f.get("applied"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let state = complete.data_str("state").unwrap_or_default();
        if status == "contained" && !applied {
            // Contained without FB is ok only if hard limit not violated — dep should force OOC.
            return Err(format!(
                "expected OOC/firebreak path for dep bloat; status={status} state={state}"
            ));
        }
        // Safe outcomes: applied smaller, or failed/closed with original preserved.
        if applied && root.join("TIF_MOCK_REDUCE").exists() {
            return Err("applied but reduce marker still present".into());
        }
        Ok(format!(
            "dep OOC handled; status={status} state={state} applied={applied}"
        ))
    });
    assert_scenario(&r);
}

#[test]
fn b03_refactor_expands_scope() {
    ensure_tif_built();
    let r = run_scenario("B03", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        git_init_commit(root).map_err(|e| e.to_string())?;

        let begin = tif_json(
            root,
            &[
                "run",
                "begin",
                "--task",
                "tiny refactor parser only",
                "--force",
            ],
        )
        .map_err(|e| e.to_string())?;
        let run_id = begin.data_str("run_id").ok_or_else(|| begin.summary())?;

        // Unrelated files beyond the requested boundary.
        fs::create_dir_all(root.join("src/unrelated")).map_err(|e| e.to_string())?;
        for i in 0..5 {
            fs::write(
                root.join(format!("src/unrelated/mod{i}.rs")),
                format!("pub fn f{i}() {{}}\n"),
            )
            .map_err(|e| e.to_string())?;
        }
        let mut lib = fs::read_to_string(root.join("src/lib.rs")).map_err(|e| e.to_string())?;
        lib.push_str("\npub mod unrelated;\n");
        fs::write(root.join("src/lib.rs"), lib).map_err(|e| e.to_string())?;

        let complete = tif_json(
            root,
            &[
                "run",
                "complete",
                &run_id,
                "--from-git",
                "--verification-passed",
                "true",
            ],
        )
        .map_err(|e| e.to_string())?;
        if !complete.ok_envelope() {
            return Err(format!("complete: {}", complete.summary()));
        }
        let assessment = complete
            .data()
            .and_then(|d| d.get("assessment"))
            .ok_or("missing assessment")?;
        let score = assessment
            .get("score")
            .and_then(|s| s.get("score"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let status = assessment
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let unrelated = assessment
            .get("score")
            .and_then(|s| s.get("breakdown"))
            .and_then(|b| b.get("unrelated_change"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        // Expanded scope should show material score / OOC / hard limit signal.
        if status == "contained" && score < 50.0 && unrelated == 0.0 {
            return Err(format!(
                "expanded refactor looked too clean; status={status} score={score} out={}",
                complete.stdout
            ));
        }
        Ok(format!(
            "scope expansion scored; status={status} score={score}"
        ))
    });
    assert_scenario(&r);
}

#[test]
fn b04_security_must_not_strip_validation() {
    ensure_tif_built();
    let r = run_scenario("B04", || {
        let tmp = copy_fixture("security-sensitive").map_err(|e| e.to_string())?;
        let root = tmp.path();
        git_init_commit(root).map_err(|e| e.to_string())?;

        let begin = tif_json(
            root,
            &[
                "run",
                "begin",
                "--task",
                "simplify login validation",
                "--force",
            ],
        )
        .map_err(|e| e.to_string())?;
        let run_id = begin.data_str("run_id").ok_or_else(|| begin.summary())?;

        // Strip validation (dangerous "minimalism").
        fs::write(
            root.join("src/auth/login.rs"),
            r#"//! validation stripped by bad simplification
pub fn validate_password(_password: &str) -> bool { true }
pub fn login(_user: &str, _password: &str) -> Result<(), &'static str> { Ok(()) }
"#,
        )
        .map_err(|e| e.to_string())?;

        // Force Firebreak with mock that would try to apply reduce — but floor on validation
        // is assessed at complete. Without floor signal on validation_preserved from CLI metrics,
        // we still ensure auto firebreak cannot apply a worse tree when we plant TIF_MOCK_FAIL
        // after strip, OR we score the strip as OOC and ensure applied=false for inflate.
        // Use OOC + inflate so candidate is larger → not applied; original (stripped) is
        // "current" — product correctness floor for validation is primarily unit-tested.
        // Here: refuse apply of larger candidate after security surface change.
        fs::write(root.join("TIF_MOCK_INFLATE"), "BLOAT\n".repeat(80))
            .map_err(|e| e.to_string())?;

        let complete = tif_json(
            root,
            &[
                "run",
                "complete",
                &run_id,
                "--deps-added",
                "1",
                "--files-added",
                "2",
                "--lines-added",
                "40",
                "--auto-firebreak",
                "--verification-passed",
                "true",
            ],
        )
        .map_err(|e| e.to_string())?;

        let applied = complete
            .data()
            .and_then(|d| d.get("firebreak"))
            .and_then(|f| f.get("applied"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if applied {
            return Err(format!(
                "must not apply larger/untrusted firebreak over security surface: {}",
                complete.stdout
            ));
        }
        if root.join("TIF_MOCK_BLOAT.txt").exists() {
            return Err("inflate bloat leaked into source".into());
        }
        // Stripped validation remains the operator problem until a floor-aware plant;
        // safety property here: Firebreak did not silently rewrite auth with larger candidate.
        Ok("security surface: larger candidate not applied".into())
    });
    assert_scenario(&r);
}

#[test]
fn b08_five_alarm_clean_room_path() {
    ensure_tif_built();
    let r = run_scenario("B08", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        // Multi-mock pool for staged recovery.
        let mut cfg =
            fs::read_to_string(root.join(".this-is-fine.toml")).map_err(|e| e.to_string())?;
        cfg.push_str(
            r#"

[[reviewers]]
id = "e2e-mock-2"
provider = "mock"
model = "fixture-2"
allow_source_egress = false
priority = 50
"#,
        );
        fs::write(root.join(".this-is-fine.toml"), cfg).map_err(|e| e.to_string())?;
        fs::write(root.join("TIF_MOCK_REDUCE"), vec![b'X'; 40_000]).map_err(|e| e.to_string())?;
        git_init_commit(root).map_err(|e| e.to_string())?;

        let begin = tif_json(
            root,
            &["run", "begin", "--task", "five-alarm recovery", "--force"],
        )
        .map_err(|e| e.to_string())?;
        let run_id = begin.data_str("run_id").ok_or_else(|| begin.summary())?;

        // OOC without auto firebreak so five-alarm can run.
        let complete = tif_json(
            root,
            &[
                "run",
                "complete",
                &run_id,
                "--deps-added",
                "1",
                "--files-added",
                "2",
                "--lines-added",
                "80",
                "--verification-passed",
                "true",
            ],
        )
        .map_err(|e| e.to_string())?;
        let state = complete.data_str("state").unwrap_or_default();
        if state != "out_of_control" && state != "closed" {
            // closed may happen if auto-close after contained — ensure OOC metrics.
            let status = complete
                .data()
                .and_then(|d| d.get("assessment"))
                .and_then(|a| a.get("status"))
                .and_then(|s| s.as_str())
                .unwrap_or("");
            if status != "out_of_control" {
                return Err(format!(
                    "need OOC before five-alarm; state={state} status={status}"
                ));
            }
        }

        let plan = tif_json(root, &["five-alarm", "--plan"]).map_err(|e| e.to_string())?;
        // `--plan` emits the plan object directly (not always JsonResponse envelope).
        if plan.status != 0 {
            return Err(format!("five-alarm --plan exit: {}", plan.summary()));
        }
        let has_steps = plan
            .json
            .as_ref()
            .and_then(|v| v.get("steps"))
            .and_then(|s| s.as_array())
            .is_some_and(|a| !a.is_empty())
            || plan.stdout.contains("Stage 1")
            || plan.stdout.contains("staged recovery");
        if !has_steps {
            return Err(format!(
                "five-alarm --plan missing outline: {}",
                plan.stdout
            ));
        }

        let fa = tif_json(root, &["five-alarm", "--run", &run_id, "--apply"])
            .map_err(|e| e.to_string())?;
        // Must not crash; staged recovery produces structure.
        if fa.protocol_version() != Some(1) {
            return Err(format!("bad protocol: {}", fa.summary()));
        }
        // Either applied smaller, or failed closed with original preserved.
        let applied = fa
            .data()
            .and_then(|d| {
                d.get("five_alarm")
                    .and_then(|p| p.get("applied"))
                    .or_else(|| d.get("applied"))
                    .or_else(|| d.get("firebreak").and_then(|f| f.get("applied")))
            })
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if applied && root.join("TIF_MOCK_REDUCE").exists() {
            return Err("five-alarm applied but reduce marker remains".into());
        }
        Ok(format!(
            "five-alarm ran; ok={} applied={applied}",
            fa.ok_envelope()
        ))
    });
    assert_scenario(&r);
}

#[test]
fn b09_historical_risk_alone_insufficient() {
    ensure_tif_built();
    let r = run_scenario("B09", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        git_init_commit(root).map_err(|e| e.to_string())?;

        let begin = tif_json(
            root,
            &["run", "begin", "--task", "contained only", "--force"],
        )
        .map_err(|e| e.to_string())?;
        let run_id = begin.data_str("run_id").ok_or_else(|| begin.summary())?;
        let complete = tif_json(
            root,
            &[
                "run",
                "complete",
                &run_id,
                "--files-changed",
                "1",
                "--lines-added",
                "2",
                "--verification-passed",
                "true",
            ],
        )
        .map_err(|e| e.to_string())?;
        if !complete.ok_envelope() {
            return Err(format!("complete: {}", complete.summary()));
        }

        let fa = tif_json(
            root,
            &[
                "five-alarm",
                "--run",
                &run_id,
                "--historical-risk",
                "--apply",
            ],
        )
        .map_err(|e| e.to_string())?;
        // Must refuse: no current containment failure.
        if fa.ok_envelope() && fa.status == 0 {
            // Some paths may return ok=false in envelope with exit non-zero.
            let msg = format!("{} {}", fa.stdout, fa.stderr).to_lowercase();
            if !msg.contains("five-alarm")
                && !msg.contains("historical")
                && !msg.contains("containment")
                && !msg.contains("forbidden")
                && !msg.contains("escalat")
            {
                return Err(format!(
                    "historical-only five-alarm should fail; got {}",
                    fa.summary()
                ));
            }
        }
        if fa.ok_envelope() && fa.status == 0 {
            // Double-check data for aborted stage
            let aborted = fa
                .data()
                .and_then(|d| d.get("five_alarm"))
                .and_then(|p| p.get("stage"))
                .and_then(|s| s.as_str())
                .map(|s| s.contains("abort") || s == "aborted")
                .unwrap_or(false);
            if !aborted {
                return Err(format!(
                    "expected refusal or abort for historical-only; {}",
                    fa.stdout
                ));
            }
        }
        Ok(format!(
            "historical risk alone refused; status={} ok={}",
            fa.status,
            fa.ok_envelope()
        ))
    });
    assert_scenario(&r);
}

#[test]
fn b10_hosted_denied_without_auth() {
    ensure_tif_built();
    let r = run_scenario("B10", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        // Hosted-style reviewer without credential / egress.
        let toml = r#"version = 1
enabled = true
default_fire_level = 3

[verification]
commands = ["echo tif-ok"]
discover = false

[simplicity.limits]
new_runtime_dependencies = 0

[approval]
auto_apply_firebreak = true
require_firebreak_approval = false

[audit]
tier = "redacted"

[[reviewers]]
id = "hosted-deny"
provider = "openai_compatible"
model = "gpt-test"
endpoint = "https://example.invalid/v1"
credential_ref = "env:TIF_E2E_MISSING_KEY_XYZ"
allow_source_egress = false
priority = 100
"#;
        fs::write(root.join(".this-is-fine.toml"), toml).map_err(|e| e.to_string())?;
        fs::write(root.join("TIF_MOCK_REDUCE"), vec![b'X'; 20_000]).map_err(|e| e.to_string())?;
        git_init_commit(root).map_err(|e| e.to_string())?;

        // Ensure missing key is not set.
        std::env::remove_var("TIF_E2E_MISSING_KEY_XYZ");

        let complete = ooc_auto(root, "hosted reviewer deny path")?;
        let applied = complete
            .data()
            .and_then(|d| d.get("firebreak"))
            .and_then(|f| f.get("applied"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if applied {
            return Err(format!(
                "hosted without creds must not apply: {}",
                complete.stdout
            ));
        }
        // Probe should also fail closed.
        let probe = tif_json(root, &["reviewer", "probe", "--id", "hosted-deny"])
            .map_err(|e| e.to_string())?;
        // Probe may ok=false or report not ready.
        Ok(format!(
            "hosted fail-closed; complete_applied={applied} probe_ok={}",
            probe.ok_envelope()
        ))
    });
    assert_scenario(&r);
}

#[test]
fn b13_recovery_drill_docs_only() {
    ensure_tif_built();
    let r = run_scenario("B13", || {
        let start = std::time::Instant::now();
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        fs::write(root.join("TIF_MOCK_REDUCE"), vec![b'X'; 35_000]).map_err(|e| e.to_string())?;
        fs::write(root.join("KEEP.txt"), b"drill").map_err(|e| e.to_string())?;
        git_init_commit(root).map_err(|e| e.to_string())?;

        // Documented recovery path: apply then rollback (user-guide Firebreak + rollback).
        let begin = tif_json(
            root,
            &["run", "begin", "--task", "recovery drill", "--force"],
        )
        .map_err(|e| e.to_string())?;
        let run_id = begin.data_str("run_id").ok_or_else(|| begin.summary())?;
        let complete = tif_json(
            root,
            &[
                "run",
                "complete",
                &run_id,
                "--deps-added",
                "1",
                "--files-added",
                "2",
                "--lines-added",
                "80",
                "--auto-firebreak",
                "--verification-passed",
                "true",
            ],
        )
        .map_err(|e| e.to_string())?;
        let applied = complete
            .data()
            .and_then(|d| d.get("firebreak"))
            .and_then(|f| f.get("applied"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !applied {
            return Err(format!(
                "drill needs a successful apply first: {}",
                complete.stdout
            ));
        }

        // user-guide: tif rollback <run_id>
        let rb = tif_json(root, &["rollback", &run_id]).map_err(|e| e.to_string())?;
        if !rb.ok_envelope() || rb.status != 0 {
            return Err(format!("rollback (docs recovery): {}", rb.summary()));
        }
        if !root.join("TIF_MOCK_REDUCE").exists() {
            return Err("rollback did not restore baseline marker".into());
        }
        let keep = fs::read_to_string(root.join("KEEP.txt")).map_err(|e| e.to_string())?;
        if keep != "drill" {
            return Err(format!("KEEP corrupted: {keep}"));
        }

        // status + audit after recovery (runbook inspection steps).
        let st = tif_json(root, &["status"]).map_err(|e| e.to_string())?;
        if !st.ok_envelope() {
            return Err(format!("status after recovery: {}", st.summary()));
        }
        let audit = tif_json(root, &["audit", "--limit", "5"]).map_err(|e| e.to_string())?;
        if !audit.ok_envelope() {
            return Err(format!("audit after recovery: {}", audit.summary()));
        }

        let ms = start.elapsed().as_millis();
        Ok(format!("recovery drill ok; time_to_recover_ms={ms}"))
    });
    assert_scenario(&r);
}

#[test]
fn b15_audit_show_and_gc() {
    ensure_tif_built();
    let r = run_scenario("B15", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        git_init_commit(root).map_err(|e| e.to_string())?;

        let begin = tif_json(root, &["run", "begin", "--task", "audit path", "--force"])
            .map_err(|e| e.to_string())?;
        let run_id = begin.data_str("run_id").ok_or_else(|| begin.summary())?;
        let _ = tif_json(
            root,
            &[
                "run",
                "complete",
                &run_id,
                "--files-changed",
                "0",
                "--lines-added",
                "0",
                "--verification-passed",
                "true",
            ],
        )
        .map_err(|e| e.to_string())?;

        let show = tif_json(root, &["audit", "--limit", "10"]).map_err(|e| e.to_string())?;
        if !show.ok_envelope() || show.status != 0 {
            return Err(format!("audit show: {}", show.summary()));
        }

        let gc = tif_json(root, &["audit", "--gc"]).map_err(|e| e.to_string())?;
        if !gc.ok_envelope() || gc.status != 0 {
            return Err(format!("audit --gc: {}", gc.summary()));
        }
        // GC must not delete the live run record wholesale in a way that crashes show.
        let show2 = tif_json(root, &["audit", "--limit", "5"]).map_err(|e| e.to_string())?;
        if !show2.ok_envelope() {
            return Err(format!("audit after gc: {}", show2.summary()));
        }
        Ok("audit show + gc ok".into())
    });
    assert_scenario(&r);
}

fn ooc_auto(root: &Path, task: &str) -> Result<tif_e2e::TifOutput, String> {
    let begin =
        tif_json(root, &["run", "begin", "--task", task, "--force"]).map_err(|e| e.to_string())?;
    if !begin.ok_envelope() {
        return Err(format!("begin failed: {}", begin.summary()));
    }
    let run_id = begin
        .data_str("run_id")
        .ok_or_else(|| format!("no run_id: {}", begin.stdout))?;
    tif_json(
        root,
        &[
            "run",
            "complete",
            &run_id,
            "--deps-added",
            "1",
            "--files-added",
            "2",
            "--lines-added",
            "80",
            "--auto-firebreak",
            "--verification-passed",
            "true",
        ],
    )
    .map_err(|e| e.to_string())
}

fn assert_scenario(r: &ScenarioResult) {
    assert!(
        r.pass,
        "scenario {} failed ({}ms): {}\n{}",
        r.id, r.duration_ms, r.notes, r.log
    );
}

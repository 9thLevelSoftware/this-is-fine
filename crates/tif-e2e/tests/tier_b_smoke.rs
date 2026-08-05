//! Tier B smoke scenarios — core CLI user journeys (UT-0).

use std::fs;
use tif_e2e::{
    copy_fixture, ensure_tif_built, git_init_commit, run_scenario, tif_json, tree_hash,
    ScenarioResult,
};

#[test]
fn b14_init_on_off_status() {
    ensure_tif_built();
    let r = run_scenario("B14", || {
        let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
        let root = tmp.path();
        git_init_commit(root).map_err(|e| e.to_string())?;

        let init = tif_json(root, &["init", "--force"]).map_err(|e| e.to_string())?;
        // init may print human text; also check files exist.
        if !root.join(".this-is-fine.toml").is_file() {
            return Err(format!(
                "init did not write shared config: {}",
                init.summary()
            ));
        }

        let on = tif_json(root, &["on"]).map_err(|e| e.to_string())?;
        if !on.ok_envelope() {
            return Err(format!("on failed: {}", on.summary()));
        }

        let status = tif_json(root, &["status"]).map_err(|e| e.to_string())?;
        if !status.ok_envelope() {
            return Err(format!("status failed: {}", status.summary()));
        }
        if status.protocol_version() != Some(1) {
            return Err(format!(
                "bad protocol_version: {:?}",
                status.protocol_version()
            ));
        }
        let enabled = status
            .data()
            .and_then(|d| d.get("enabled"))
            .and_then(|v| v.as_bool());
        if enabled != Some(true) {
            return Err(format!("expected enabled=true: {}", status.stdout));
        }

        let off = tif_json(root, &["off"]).map_err(|e| e.to_string())?;
        if !off.ok_envelope() {
            return Err(format!("off failed: {}", off.summary()));
        }
        let status2 = tif_json(root, &["status"]).map_err(|e| e.to_string())?;
        let enabled2 = status2
            .data()
            .and_then(|d| d.get("enabled"))
            .and_then(|v| v.as_bool());
        if enabled2 != Some(false) {
            return Err(format!(
                "expected enabled=false after off: {}",
                status2.stdout
            ));
        }

        Ok("init/on/status/off cycle ok".into())
    });
    assert_scenario(&r);
}

#[test]
fn b01_happy_contained_task() {
    ensure_tif_built();
    let r = run_scenario("B01", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        git_init_commit(root).map_err(|e| e.to_string())?;

        let begin = tif_json(
            root,
            &[
                "run",
                "begin",
                "--task",
                "fix null pointer in parser",
                "--force",
            ],
        )
        .map_err(|e| e.to_string())?;
        if !begin.ok_envelope() {
            return Err(format!("begin failed: {}", begin.summary()));
        }
        if begin.protocol_version() != Some(1) {
            return Err("protocol_version != 1".into());
        }
        let run_id = begin.data_str("run_id").ok_or("missing run_id")?;

        // Minimal plant: small comment-only style change within limits.
        let lib = root.join("src/lib.rs");
        let mut body = fs::read_to_string(&lib).map_err(|e| e.to_string())?;
        body.push_str("\n// tiny contained fix\n");
        fs::write(&lib, body).map_err(|e| e.to_string())?;

        let complete = tif_json(
            root,
            &[
                "run",
                "complete",
                &run_id,
                "--files-changed",
                "1",
                "--lines-added",
                "1",
                "--verification-passed",
                "true",
            ],
        )
        .map_err(|e| e.to_string())?;
        if !complete.ok_envelope() {
            return Err(format!("complete failed: {}", complete.summary()));
        }

        let state = complete.data_str("state").unwrap_or_default();
        // Contained or closed after success.
        if !matches!(state.as_str(), "contained" | "closed") {
            // Some paths may leave scoring states; check assessment if present.
            let status = complete
                .data()
                .and_then(|d| d.get("assessment"))
                .and_then(|a| a.get("status"))
                .and_then(|s| s.as_str())
                .unwrap_or("");
            if status != "contained" && state != "contained" {
                return Err(format!(
                    "expected contained; state={state} assessment_status={status}\n{}",
                    complete.stdout
                ));
            }
        }

        let fb_applied = complete
            .data()
            .and_then(|d| d.get("firebreak"))
            .and_then(|f| f.get("applied"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if fb_applied {
            return Err("happy path should not need Firebreak apply".into());
        }

        Ok(format!("contained ok; state={state}"))
    });
    assert_scenario(&r);
}

#[test]
fn b11_cli_tour_json_protocol() {
    ensure_tif_built();
    let r = run_scenario("B11", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        git_init_commit(root).map_err(|e| e.to_string())?;

        let commands: &[&[&str]] = &[
            &["status"],
            &["policy", "resolve", "--task", "cli tour"],
            &["inspect"],
            &["reviewer", "list"],
            &["fire-level"],
            &["adaptation", "status"],
            &["audit"],
            &["verify", "--dry-run"],
        ];

        let mut notes = Vec::new();
        for args in commands {
            let out = tif_json(root, args).map_err(|e| e.to_string())?;
            if out.protocol_version() != Some(1) {
                return Err(format!(
                    "{args:?}: missing protocol_version 1: {}",
                    out.summary()
                ));
            }
            // dry-run / empty audit may still be ok envelopes.
            if !out.ok_envelope() && out.status == 0 {
                return Err(format!("{args:?}: status 0 but ok=false: {}", out.stdout));
            }
            notes.push(format!("{}:ok={}", args.join(" "), out.ok_envelope()));
        }

        Ok(notes.join("; "))
    });
    assert_scenario(&r);
}

#[test]
fn b12_adapter_protocol_lifecycle() {
    ensure_tif_built();
    let r = run_scenario("B12", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        git_init_commit(root).map_err(|e| e.to_string())?;

        let policy = tif_json(root, &["policy", "resolve", "--task", "adapter lifecycle"])
            .map_err(|e| e.to_string())?;
        if !policy.ok_envelope() || policy.protocol_version() != Some(1) {
            return Err(format!("policy resolve: {}", policy.summary()));
        }

        let begin = tif_json(
            root,
            &[
                "run",
                "begin",
                "--task",
                "adapter lifecycle",
                "--agent",
                "e2e",
                "--model",
                "mock",
                "--force",
            ],
        )
        .map_err(|e| e.to_string())?;
        if !begin.ok_envelope() {
            return Err(format!("begin: {}", begin.summary()));
        }
        let run_id = begin.data_str("run_id").ok_or("no run_id")?;
        let state1 = begin.data_str("state").unwrap_or_default();

        let complete = tif_json(
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
        if !complete.ok_envelope() {
            return Err(format!("complete: {}", complete.summary()));
        }
        let state2 = complete.data_str("state").unwrap_or_default();

        let show = tif_json(root, &["run", "show", &run_id]).map_err(|e| e.to_string())?;
        // show should be ok if run exists.
        if !show.ok_envelope() && show.status != 0 {
            return Err(format!("run show: {}", show.summary()));
        }

        Ok(format!(
            "lifecycle protocol_version=1 begin_state={state1} complete_state={state2}"
        ))
    });
    assert_scenario(&r);
}

#[test]
fn b06_firebreak_success_and_rollback() {
    ensure_tif_built();
    let r = run_scenario("B06", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        // Plant mock reduce marker + keep file before baseline commit? Firebreak isolation
        // snapshots at begin; plant reduce marker before begin so baseline includes it.
        fs::write(root.join("TIF_MOCK_REDUCE"), vec![b'X'; 40_000]).map_err(|e| e.to_string())?;
        fs::write(root.join("KEEP.txt"), b"original-keep").map_err(|e| e.to_string())?;
        git_init_commit(root).map_err(|e| e.to_string())?;

        let before = tree_hash(root).map_err(|e| e.to_string())?;

        let begin = tif_json(
            root,
            &[
                "run",
                "begin",
                "--task",
                "shrink oversized feature",
                "--force",
            ],
        )
        .map_err(|e| e.to_string())?;
        if !begin.ok_envelope() {
            return Err(format!("begin: {}", begin.summary()));
        }
        let run_id = begin.data_str("run_id").ok_or("no run_id")?;

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

        if !complete.ok_envelope() {
            return Err(format!("complete/firebreak: {}", complete.summary()));
        }

        let applied = complete
            .data()
            .and_then(|d| d.get("firebreak"))
            .and_then(|f| f.get("applied"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let state = complete.data_str("state").unwrap_or_default();

        if !applied && state != "applied" {
            return Err(format!(
                "expected Firebreak apply; state={state} out={}",
                complete.stdout
            ));
        }

        if root.join("TIF_MOCK_REDUCE").exists() {
            return Err("TIF_MOCK_REDUCE should be removed by mock reduce".into());
        }
        let keep = fs::read_to_string(root.join("KEEP.txt")).map_err(|e| e.to_string())?;
        if keep != "original-keep" {
            return Err(format!("KEEP.txt corrupted: {keep}"));
        }

        let after_apply = tree_hash(root).map_err(|e| e.to_string())?;
        if after_apply == before {
            return Err("tree hash unchanged after apply (reduce marker still present?)".into());
        }

        let rb = tif_json(root, &["rollback", &run_id]).map_err(|e| e.to_string())?;
        if !rb.ok_envelope() {
            return Err(format!("rollback failed: {}", rb.summary()));
        }

        if !root.join("TIF_MOCK_REDUCE").exists() {
            return Err("rollback should restore TIF_MOCK_REDUCE".into());
        }
        let keep2 = fs::read_to_string(root.join("KEEP.txt")).map_err(|e| e.to_string())?;
        if keep2 != "original-keep" {
            return Err(format!("KEEP.txt after rollback: {keep2}"));
        }

        let after_rb = tree_hash(root).map_err(|e| e.to_string())?;
        if after_rb != before {
            return Err(format!(
                "tree after rollback != baseline\nbefore={before}\nafter_rb={after_rb}"
            ));
        }

        Ok(format!("apply+rollback ok; state_after_complete={state}"))
    });
    assert_scenario(&r);
}

#[test]
fn b07_sensitive_path_requires_approval() {
    ensure_tif_built();
    let r = run_scenario("B07", || {
        let tmp = copy_fixture("security-sensitive").map_err(|e| e.to_string())?;
        let root = tmp.path();
        // Mock reduce so candidate is smaller and ready for approval.
        fs::write(root.join("TIF_MOCK_REDUCE"), vec![b'X'; 40_000]).map_err(|e| e.to_string())?;
        // Touch sensitive path so approval policy engages (also require_firebreak_approval=true).
        let login = root.join("src/auth/login.rs");
        let mut body = fs::read_to_string(&login).map_err(|e| e.to_string())?;
        body.push_str("\n// sensitive touch\n");
        fs::write(&login, body).map_err(|e| e.to_string())?;
        git_init_commit(root).map_err(|e| e.to_string())?;

        let begin = tif_json(
            root,
            &["run", "begin", "--task", "auth hardening", "--force"],
        )
        .map_err(|e| e.to_string())?;
        if !begin.ok_envelope() {
            return Err(format!("begin: {}", begin.summary()));
        }
        let run_id = begin.data_str("run_id").ok_or("no run_id")?;

        let complete = tif_json(
            root,
            &[
                "run",
                "complete",
                &run_id,
                "--deps-added",
                "1",
                "--files-added",
                "1",
                "--files-changed",
                "1",
                "--lines-added",
                "80",
                "--auto-firebreak",
                "--verification-passed",
                "true",
            ],
        )
        .map_err(|e| e.to_string())?;

        let state = complete.data_str("state").unwrap_or_default();
        let fb = complete.data().and_then(|d| d.get("firebreak"));
        let applied = fb
            .and_then(|f| f.get("applied"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let requires = fb
            .and_then(|f| f.get("requires_approval"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if applied {
            return Err(format!(
                "must not auto-apply when approval required; state={state} out={}",
                complete.stdout
            ));
        }
        if state != "awaiting_approval" && !requires {
            return Err(format!(
                "expected awaiting_approval or requires_approval; state={state} out={}",
                complete.stdout
            ));
        }

        // Reject path leaves reduce marker.
        let rej = tif_json(root, &["reject", &run_id]).map_err(|e| e.to_string())?;
        if !rej.ok_envelope() {
            return Err(format!("reject failed: {}", rej.summary()));
        }
        if !root.join("TIF_MOCK_REDUCE").exists() {
            return Err("reject should leave original bloat marker".into());
        }

        // Fresh run → approve applies.
        let begin2 = tif_json(
            root,
            &["run", "begin", "--task", "auth approve path", "--force"],
        )
        .map_err(|e| e.to_string())?;
        let run_id2 = begin2.data_str("run_id").ok_or("no run_id2")?;
        let complete2 = tif_json(
            root,
            &[
                "run",
                "complete",
                &run_id2,
                "--deps-added",
                "1",
                "--files-added",
                "1",
                "--lines-added",
                "80",
                "--auto-firebreak",
                "--verification-passed",
                "true",
            ],
        )
        .map_err(|e| e.to_string())?;
        let state2 = complete2.data_str("state").unwrap_or_default();
        if state2 != "awaiting_approval"
            && !complete2
                .data()
                .and_then(|d| d.get("firebreak"))
                .and_then(|f| f.get("requires_approval"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        {
            return Err(format!("second run not awaiting approval: {state2}"));
        }

        let ap = tif_json(root, &["approve", &run_id2]).map_err(|e| e.to_string())?;
        if !ap.ok_envelope() {
            return Err(format!("approve failed: {}", ap.summary()));
        }
        if root.join("TIF_MOCK_REDUCE").exists() {
            return Err("approve should apply mock reduce (delete marker)".into());
        }

        Ok(format!(
            "approval gate ok; reject kept original; approve applied; first_state={state}"
        ))
    });
    assert_scenario(&r);
}

#[test]
fn b05_firebreak_fail_preserves_source() {
    ensure_tif_built();
    let r = run_scenario("B05", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        fs::write(root.join("TIF_MOCK_REDUCE"), vec![b'X'; 10_000]).map_err(|e| e.to_string())?;
        fs::write(root.join("TIF_MOCK_FAIL"), b"1").map_err(|e| e.to_string())?;
        fs::write(
            root.join("src/lib.rs"),
            b"pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
        )
        .map_err(|e| e.to_string())?;
        git_init_commit(root).map_err(|e| e.to_string())?;
        let before = tree_hash(root).map_err(|e| e.to_string())?;

        let begin = tif_json(
            root,
            &["run", "begin", "--task", "fail firebreak", "--force"],
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
        if applied {
            return Err(format!(
                "must not apply on backend fail: {}",
                complete.stdout
            ));
        }

        let after = tree_hash(root).map_err(|e| e.to_string())?;
        if after != before {
            return Err(format!(
                "source changed on failed firebreak\n{before}\n{after}"
            ));
        }
        if !root.join("TIF_MOCK_REDUCE").exists() {
            return Err("reduce marker should remain".into());
        }

        Ok("failed firebreak preserved source".into())
    });
    assert_scenario(&r);
}

fn assert_scenario(r: &ScenarioResult) {
    assert!(
        r.pass,
        "scenario {} failed ({}ms): {}\n{}",
        r.id, r.duration_ms, r.notes, r.log
    );
}

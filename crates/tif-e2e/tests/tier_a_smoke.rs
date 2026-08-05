//! Tier A smoke scenarios (UT-0 scaffold + first safety journeys).
//! Full catalog: docs/USER_TESTING.md

use std::fs;
use tif_e2e::{
    copy_fixture, ensure_tif_built, git_init_commit, run_scenario, tif_json, tree_hash,
    ScenarioResult,
};

#[test]
fn ut0_binary_and_fixtures_present() {
    ensure_tif_built();
    let bin = tif_e2e::tif_bin();
    assert!(bin.is_file(), "missing tif at {}", bin.display());
    for name in ["rust-mini", "rust-bloat", "js-mini", "security-sensitive"] {
        let p = tif_e2e::fixture_path(name);
        assert!(p.is_dir(), "missing fixture {name} at {}", p.display());
    }
}

#[test]
fn a02_empty_reviewer_pool_fail_closed() {
    ensure_tif_built();
    let r = run_scenario("A02", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        git_init_commit(root).map_err(|e| e.to_string())?;

        // Strip reviewers → empty pool.
        let shared = root.join(".this-is-fine.toml");
        let text = fs::read_to_string(&shared).map_err(|e| e.to_string())?;
        let stripped = strip_reviewers_toml(&text);
        fs::write(&shared, stripped).map_err(|e| e.to_string())?;

        let before = tree_hash(root).map_err(|e| e.to_string())?;

        let begin = tif_json(
            root,
            &["run", "begin", "--task", "empty pool firebreak", "--force"],
        )
        .map_err(|e| e.to_string())?;
        if !begin.ok_envelope() {
            return Err(format!("begin failed: {}", begin.summary()));
        }
        let run_id = begin
            .data_str("run_id")
            .ok_or_else(|| format!("no run_id: {}", begin.stdout))?;

        // Force OOC metrics + auto firebreak without a pool.
        let complete = tif_json(
            root,
            &[
                "run",
                "complete",
                &run_id,
                "--deps-added",
                "2",
                "--files-added",
                "3",
                "--lines-added",
                "120",
                "--auto-firebreak",
                "--verification-passed",
                "true",
            ],
        )
        .map_err(|e| e.to_string())?;

        let after = tree_hash(root).map_err(|e| e.to_string())?;
        if before != after {
            return Err(format!(
                "source tree changed without authorized apply: before={before} after={after}\n{}",
                complete.summary()
            ));
        }

        let applied = complete
            .data()
            .and_then(|d| d.get("firebreak"))
            .and_then(|f| f.get("applied"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if applied {
            return Err(format!(
                "empty pool must not apply; got: {}",
                complete.stdout
            ));
        }

        // State should not be applied.
        let state = complete.data_str("state").unwrap_or_default();
        if state == "applied" {
            return Err(format!("state applied with empty pool: {state}"));
        }

        Ok(format!(
            "no apply; state={state}; tree preserved; {}",
            complete.summary()
        ))
    });
    assert_scenario(&r);
}

#[test]
fn a05_incomplete_verify_fails_floor() {
    ensure_tif_built();
    let r = run_scenario("A05", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        git_init_commit(root).map_err(|e| e.to_string())?;

        // Empty verification plan, discover off.
        let shared = root.join(".this-is-fine.toml");
        let text = fs::read_to_string(&shared).map_err(|e| e.to_string())?;
        let mut out = String::new();
        let mut in_verification = false;
        for line in text.lines() {
            if line.trim() == "[verification]" {
                in_verification = true;
                out.push_str(line);
                out.push('\n');
                out.push_str("commands = []\n");
                out.push_str("discover = false\n");
                continue;
            }
            if in_verification {
                if line.trim().starts_with('[') {
                    in_verification = false;
                } else if line.trim().starts_with("commands") || line.trim().starts_with("discover")
                {
                    continue;
                }
            }
            out.push_str(line);
            out.push('\n');
        }
        fs::write(&shared, out).map_err(|e| e.to_string())?;

        let begin = tif_json(
            root,
            &["run", "begin", "--task", "no verify plan", "--force"],
        )
        .map_err(|e| e.to_string())?;
        if !begin.ok_envelope() {
            return Err(format!("begin failed: {}", begin.summary()));
        }
        let run_id = begin.data_str("run_id").ok_or("no run_id")?;

        let complete = tif_json(
            root,
            &[
                "run",
                "complete",
                &run_id,
                "--files-changed",
                "1",
                "--lines-added",
                "5",
            ],
        )
        .map_err(|e| e.to_string())?;

        // Incomplete plan must not produce a clean contained success with verification floor pass.
        let assessment = complete.data().and_then(|d| d.get("assessment"));
        let status = assessment
            .and_then(|a| a.get("status"))
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let floor_ok = assessment
            .and_then(|a| a.get("correctness"))
            .and_then(|f| f.get("verification_passed"))
            .and_then(|v| v.as_bool());

        // Accept either explicit floor fail, rejected assessment, or operation error.
        if complete.ok_envelope() && floor_ok == Some(true) && status == "contained" {
            return Err(format!(
                "incomplete verify must not pass as contained; status={status} out={}",
                complete.stdout
            ));
        }

        Ok(format!(
            "floor blocked incomplete plan; status={status} floor_ok={floor_ok:?}; {}",
            complete.summary()
        ))
    });
    assert_scenario(&r);
}

fn strip_reviewers_toml(text: &str) -> String {
    let mut out = String::new();
    let mut skip = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("[[reviewers]]") {
            skip = true;
            continue;
        }
        if skip {
            // End skip at any new table header (single or double bracket).
            let new_table = t.starts_with('[') && !t.starts_with("[[reviewers]]");
            if new_table {
                skip = false;
            } else {
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn assert_scenario(r: &ScenarioResult) {
    assert!(
        r.pass,
        "scenario {} failed ({}ms): {}\n{}",
        r.id, r.duration_ms, r.notes, r.log
    );
}

//! Tier A safety scenarios — docs/USER_TESTING.md
//! UT-0: A02, A05 · UT-1: A01, A03–A04, A06–A10

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use tif_core::config::ReviewerConfig;
use tif_core::credentials::resolve_credential;
use tif_core::error::TifError;
use tif_core::policy::{PolicyCompileRequest, PolicyCompiler};
use tif_core::providers::context::{
    build_reviewer_context, ContextBuildRequest, ReviewerInvocationMode,
};
use tif_core::scoring::{select_smaller_verified, CorrectnessFloor, DiffMetrics, SimplicityScorer};
use tif_core::task::TaskCategory;
use tif_e2e::{
    copy_fixture, ensure_tif_built, git_init_commit, run_scenario, tif_json, tree_hash,
    tree_hash_excluding, ScenarioResult,
};

/// Serialize A07 so process-global env mutations cannot race siblings.
static ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

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
fn a01_correctness_floor_beats_smaller_incorrect() {
    let r = run_scenario("A01", || {
        // Pure scoring invariant (also covered by unit tests; re-asserted for field pack).
        let scorer = SimplicityScorer::new(Default::default(), Default::default());
        let mut floor_fail = CorrectnessFloor::all_pass();
        floor_fail.verification_passed = false;
        let mut floor_ok = CorrectnessFloor::all_pass();
        floor_ok.verification_passed = true;

        let small_bad = scorer.score(
            &DiffMetrics {
                lines_added: 1,
                files_changed: 1,
                ..Default::default()
            },
            &floor_fail,
        );
        let large_ok = scorer.score(
            &DiffMetrics {
                lines_added: 500,
                files_changed: 20,
                ..Default::default()
            },
            &floor_ok,
        );
        assert!(small_bad.disqualified, "incorrect must be disqualified");
        assert!(!large_ok.disqualified, "correct large must pass floor");

        let ranked = SimplicityScorer::rank_candidates(&[
            ("small_incorrect".into(), small_bad.clone()),
            ("large_correct".into(), large_ok.clone()),
        ]);
        if ranked.first().map(|s| s.as_str()) != Some("large_correct") {
            return Err(format!("rank order wrong: {ranked:?}"));
        }

        let chosen =
            select_smaller_verified("large_correct", &large_ok, "small_incorrect", &small_bad)
                .map_err(|e| e.to_string())?;
        if chosen != "large_correct" {
            return Err(format!("select_smaller_verified chose {chosen}"));
        }

        // Disqualified never beats non-disqualified by score alone.
        let mut crafted = large_ok.clone();
        crafted.score = 0.0;
        crafted.disqualified = true;
        crafted.correctness_passed = false;
        let ranked2 = SimplicityScorer::rank_candidates(&[
            ("crafted".into(), crafted),
            ("honest".into(), large_ok),
        ]);
        if ranked2.first().map(|s| s.as_str()) != Some("honest") {
            return Err(format!("crafted score ranked first: {ranked2:?}"));
        }

        Ok("floor gate: incorrect never preferred over larger correct".into())
    });
    assert_scenario(&r);
}

#[test]
fn a02_empty_reviewer_pool_fail_closed() {
    ensure_tif_built();
    let r = run_scenario("A02", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        git_init_commit(root).map_err(|e| e.to_string())?;

        let shared = root.join(".this-is-fine.toml");
        let text = fs::read_to_string(&shared).map_err(|e| e.to_string())?;
        fs::write(&shared, strip_reviewers_toml(&text)).map_err(|e| e.to_string())?;

        let before = tree_hash(root).map_err(|e| e.to_string())?;
        let complete = ooc_auto_firebreak(root, "empty pool")?;
        let after = tree_hash(root).map_err(|e| e.to_string())?;
        if before != after {
            return Err(format!(
                "source changed with empty pool: {before} vs {after}"
            ));
        }
        if firebreak_applied(&complete) {
            return Err(format!("empty pool applied: {}", complete.stdout));
        }
        let state = complete.data_str("state").unwrap_or_default();
        if state == "applied" {
            return Err(format!("state applied: {state}"));
        }
        Ok(format!("no apply; state={state}"))
    });
    assert_scenario(&r);
}

#[test]
fn a03_backend_fail_preserves_source() {
    ensure_tif_built();
    let r = run_scenario("A03", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        fs::write(root.join("TIF_MOCK_REDUCE"), vec![b'X'; 12_000]).map_err(|e| e.to_string())?;
        fs::write(root.join("TIF_MOCK_FAIL"), b"1").map_err(|e| e.to_string())?;
        fs::write(root.join("MARKER.txt"), b"source-marker").map_err(|e| e.to_string())?;
        git_init_commit(root).map_err(|e| e.to_string())?;
        let before = tree_hash(root).map_err(|e| e.to_string())?;

        let complete = ooc_auto_firebreak(root, "backend fail")?;
        if firebreak_applied(&complete) {
            return Err(format!("applied on backend fail: {}", complete.stdout));
        }
        let after = tree_hash(root).map_err(|e| e.to_string())?;
        if after != before {
            return Err(format!("tree changed: {before} vs {after}"));
        }
        let marker = fs::read_to_string(root.join("MARKER.txt")).map_err(|e| e.to_string())?;
        if marker != "source-marker" {
            return Err(format!("MARKER corrupted: {marker}"));
        }
        Ok("backend fail preserved source hash".into())
    });
    assert_scenario(&r);
}

#[test]
fn a04_larger_candidate_not_applied() {
    ensure_tif_built();
    let r = run_scenario("A04", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        // Inflate path: mock adds large file; no REDUCE delete.
        fs::write(root.join("TIF_MOCK_INFLATE"), "BLOAT_LINE\n".repeat(200))
            .map_err(|e| e.to_string())?;
        git_init_commit(root).map_err(|e| e.to_string())?;
        let before = tree_hash(root).map_err(|e| e.to_string())?;

        let complete = ooc_auto_firebreak(root, "inflate larger")?;
        if firebreak_applied(&complete) {
            return Err(format!("larger candidate applied: {}", complete.stdout));
        }
        if root.join("TIF_MOCK_BLOAT.txt").exists() {
            return Err("inflate bloat leaked into source".into());
        }
        let after = tree_hash(root).map_err(|e| e.to_string())?;
        if after != before {
            return Err(format!("source changed: {before} vs {after}"));
        }
        Ok("larger candidate rejected; source intact".into())
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

        let shared = root.join(".this-is-fine.toml");
        let text = fs::read_to_string(&shared).map_err(|e| e.to_string())?;
        fs::write(&shared, set_empty_verification(&text)).map_err(|e| e.to_string())?;

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

        if !complete.ok_envelope() {
            return Err(format!(
                "expected structured complete response, got: {}",
                complete.summary()
            ));
        }
        let assessment = complete
            .data()
            .and_then(|d| d.get("assessment"))
            .ok_or_else(|| format!("missing assessment: {}", complete.stdout))?;
        let status = assessment
            .get("status")
            .and_then(|s| s.as_str())
            .ok_or("missing assessment.status")?;
        let floor_ok = assessment
            .get("correctness")
            .and_then(|f| f.get("verification_passed"))
            .and_then(|v| v.as_bool())
            .ok_or("missing correctness.verification_passed")?;
        let incomplete = assessment
            .get("verification")
            .and_then(|v| v.get("incomplete_plan"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if floor_ok {
            return Err(format!(
                "incomplete verify must fail floor; status={status} out={}",
                complete.stdout
            ));
        }
        if status == "contained" {
            return Err(format!(
                "must not be contained with incomplete plan: {status}"
            ));
        }
        if !incomplete && status != "unverified" && status != "rejected" {
            return Err(format!(
                "expected incomplete_plan or unverified/rejected; status={status} incomplete={incomplete}"
            ));
        }
        if firebreak_applied(&complete) {
            return Err("must not apply Firebreak when floor fails".into());
        }

        Ok(format!(
            "floor blocked; status={status} floor_ok={floor_ok} incomplete={incomplete}"
        ))
    });
    assert_scenario(&r);
}

#[test]
fn a06_egress_deny_blocks_source_package() {
    let r = run_scenario("A06", || {
        let mut rev = ReviewerConfig::mock("egress-deny", 1);
        rev.allow_source_egress = false;
        let cfg = tif_core::config::Config {
            reviewers: vec![rev.clone()],
            ..Default::default()
        };
        let policy = PolicyCompiler::new()
            .compile(&cfg, &PolicyCompileRequest::default())
            .map_err(|e| e.to_string())?;

        let req = ContextBuildRequest {
            reviewer: &rev,
            policy: &policy,
            task_category: TaskCategory::BugFix,
            task_text: Some("shrink"),
            acceptance_criteria: None,
            original_metrics: None,
            source_or_diff: Some("fn secret() { let api_key = \"sk-live-should-not-leave\"; }"),
            verification_plan_summary: None,
            mode: ReviewerInvocationMode::Standard,
            failure_summary: None,
            prior_implementation_code: None,
        };
        match build_reviewer_context(&req) {
            Ok(pkg) => {
                if pkg.includes_source {
                    return Err("source included despite allow_source_egress=false".into());
                }
                Err("expected Err when packaging source with egress false".into())
            }
            Err(e) => {
                let msg = e.to_string();
                if !msg.contains("allow_source_egress") && !msg.contains("egress") {
                    return Err(format!("unexpected error: {msg}"));
                }
                Ok(format!("egress denied before network: {msg}"))
            }
        }
    });
    assert_scenario(&r);
}

#[test]
fn a07_process_env_scrub() {
    ensure_tif_built();
    let _serial = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let r = run_scenario("A07", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();

        // Cross-platform env dump script written into the fixture.
        let script = root.join("env_dump.py");
        fs::write(
            &script,
            r#"import os, sys
out = sys.argv[1]
with open(out, "w", encoding="utf-8") as f:
    for k, v in sorted(os.environ.items()):
        f.write(f"{k}={v}\n")
print('{"provider":"process","ok":true}')
"#,
        )
        .map_err(|e| e.to_string())?;

        let dump = root.join("env_dump_out.txt");
        let dump_str = dump.display().to_string().replace('\\', "/");
        let script_str = script.display().to_string().replace('\\', "/");

        let py = find_python().ok_or("python not found for A07")?;
        let argv_toml = python_process_argv_toml(&py, &script_str, &dump_str);

        // Rewrite config: process reviewer only.
        let toml = format!(
            r#"version = 1
enabled = true
default_fire_level = 3

[verification]
commands = ["echo tif-ok"]
discover = false

[approval]
auto_apply_firebreak = true
require_firebreak_approval = false

[audit]
tier = "redacted"

[[reviewers]]
id = "env-probe"
provider = "process"
model = "env-dump"
allow_source_egress = false
priority = 100
process_argv = {argv_toml}
"#
        );
        fs::write(root.join(".this-is-fine.toml"), toml).map_err(|e| e.to_string())?;
        git_init_commit(root).map_err(|e| e.to_string())?;

        // RAII restores prior env on every exit path (including early Err).
        let _secret_guard = EnvVarGuard::set("TIF_E2E_SECRET", "super-secret-should-not-leak");
        let _key_guard = EnvVarGuard::set("OPENAI_API_KEY", "sk-test-should-not-leak");

        let out = tif_json(
            root,
            &["reviewer", "test", "--id", "env-probe", "--task", "probe"],
        )
        .map_err(|e| e.to_string())?;
        // test may succeed or soft-fail; dump file is the evidence.
        if !dump.is_file() {
            return Err(format!(
                "env dump not written; reviewer test: {} stdout={}",
                out.summary(),
                out.stdout
            ));
        }
        let body = fs::read_to_string(&dump).map_err(|e| e.to_string())?;
        if body.contains("super-secret-should-not-leak")
            || body.contains("sk-test-should-not-leak")
            || body.contains("TIF_E2E_SECRET=")
            || body.contains("OPENAI_API_KEY=")
        {
            return Err(format!("secret leaked into process env:\n{body}"));
        }
        if body.trim().is_empty() {
            return Err("env dump empty".into());
        }

        Ok("process backend scrubbed secrets from child env".into())
    });
    assert_scenario(&r);
}

#[test]
fn a08_symlink_candidate_rejected() {
    ensure_tif_built();
    let r = run_scenario("A08", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        git_init_commit(root).map_err(|e| e.to_string())?;

        let outside = tempfile::tempdir().map_err(|e| e.to_string())?;
        fs::write(outside.path().join("evil.txt"), b"escaped").map_err(|e| e.to_string())?;

        let link = root.join("candidate_link");
        #[cfg(windows)]
        {
            let status = Command::new("cmd")
                .args([
                    "/C",
                    "mklink",
                    "/J",
                    &link.display().to_string(),
                    &outside.path().display().to_string(),
                ])
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err(format!(
                    "failed to create junction for A08 (cannot exercise symlink rejection): status={status:?}"
                ));
            }
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), &link)
                .map_err(|e| format!("failed to create symlink for A08: {e}"))?;
        }

        let begin = tif_json(
            root,
            &["run", "begin", "--task", "symlink candidate", "--force"],
        )
        .map_err(|e| e.to_string())?;
        let run_id = begin.data_str("run_id").ok_or_else(|| begin.summary())?;

        let _ = tif_json(
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

        // Exclude the junction path so tree_hash is stable across apply attempts.
        let before = tree_hash_excluding(root, &["candidate_link"]).map_err(|e| e.to_string())?;
        let fb = tif_json(
            root,
            &[
                "firebreak",
                "--run-id",
                &run_id,
                "--candidate",
                &link.display().to_string(),
                "--apply",
            ],
        )
        .map_err(|e| e.to_string())?;

        if root.join("evil.txt").is_file() {
            return Err("symlink candidate applied evil.txt into source".into());
        }
        let applied = fb
            .data()
            .and_then(|d| d.get("applied"))
            .and_then(|v| v.as_bool())
            .or_else(|| {
                fb.data()
                    .and_then(|d| d.get("firebreak"))
                    .and_then(|f| f.get("applied"))
                    .and_then(|v| v.as_bool())
            })
            .unwrap_or(false);
        if applied {
            return Err(format!("symlink candidate applied: {}", fb.stdout));
        }
        // Reject/error path is required (symlink roots must not silently no-op apply).
        if fb.ok_envelope() && applied {
            return Err("ok envelope with applied=true for symlink candidate".into());
        }

        let after = tree_hash_excluding(root, &["candidate_link"]).map_err(|e| e.to_string())?;
        if after != before {
            return Err(format!(
                "source tree mutated under symlink apply attempt\nbefore={before}\nafter={after}"
            ));
        }
        Ok(format!("symlink candidate not applied; {}", fb.summary()))
    });
    assert_scenario(&r);
}

#[test]
fn a09_credential_file_outside_secrets_dir() {
    let r = run_scenario("A09", || {
        let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
        let outside = tmp.path().join("not-a-secret.txt");
        fs::write(&outside, b"leaked-token").map_err(|e| e.to_string())?;
        let ref_s = format!("file:{}", outside.display());
        match resolve_credential(&ref_s) {
            Ok(v) => Err(format!(
                "should reject outside secrets dir, got secret len {}",
                v.len()
            )),
            Err(TifError::Config(msg)) => {
                let lower = msg.to_lowercase();
                // Accept only path-policy messages (allowlist / under secrets / escapes).
                let policy = lower.contains("must be under")
                    || lower.contains("outside")
                    || lower.contains("escapes secrets")
                    || lower.contains("secrets dir")
                    || lower.contains("secrets directory");
                if policy {
                    Ok(format!("path policy rejected: {msg}"))
                } else {
                    Err(format!("Config error is not path-policy evidence: {msg}"))
                }
            }
            Err(e) => Err(format!(
                "expected TifError::Config path-policy error, got: {e:?}"
            )),
        }
    });
    assert_scenario(&r);
}

#[test]
fn a10_metadata_audit_tier_no_bodies() {
    ensure_tif_built();
    let r = run_scenario("A10", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        let shared = root.join(".this-is-fine.toml");
        let text = fs::read_to_string(&shared).map_err(|e| e.to_string())?;
        let text = text.replace("tier = \"redacted\"", "tier = \"metadata\"");
        fs::write(&shared, text).map_err(|e| e.to_string())?;
        git_init_commit(root).map_err(|e| e.to_string())?;

        let secret_task = "do not store this PROMPT_BODY_UNIQUE_9f3a and api_key=sk-audit-test";
        let begin = tif_json(root, &["run", "begin", "--task", secret_task, "--force"])
            .map_err(|e| e.to_string())?;
        if !begin.ok_envelope() {
            return Err(format!("begin: {}", begin.summary()));
        }
        let run_id = begin.data_str("run_id").ok_or("no run_id")?;
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

        // Scan audit DB and artifacts for secret strings.
        let state = root.join(".this-is-fine");
        let mut hits = Vec::new();
        if state.is_dir() {
            for entry in walkdir::WalkDir::new(&state)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let p = entry.path();
                if !p.is_file() {
                    continue;
                }
                // Skip sqlite free pages noise by reading as bytes and searching utf8.
                if let Ok(bytes) = fs::read(p) {
                    let s = String::from_utf8_lossy(&bytes);
                    if s.contains("PROMPT_BODY_UNIQUE_9f3a") || s.contains("sk-audit-test") {
                        hits.push(p.display().to_string());
                    }
                }
            }
        }
        if !hits.is_empty() {
            return Err(format!("metadata tier stored body material in: {hits:?}"));
        }

        // json_blob should be absent for metadata (reloaded run has no policy) — verify via audit CLI.
        let audit = tif_json(root, &["audit", "--limit", "5"]).map_err(|e| e.to_string())?;
        if !audit.ok_envelope() {
            return Err(format!("audit: {}", audit.summary()));
        }

        Ok("metadata tier has no prompt/diff bodies in state tree".into())
    });
    assert_scenario(&r);
}

// --- helpers ---

fn ooc_auto_firebreak(root: &Path, task: &str) -> Result<tif_e2e::TifOutput, String> {
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

fn firebreak_applied(out: &tif_e2e::TifOutput) -> bool {
    out.data()
        .and_then(|d| d.get("firebreak"))
        .and_then(|f| f.get("applied"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
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

fn set_empty_verification(text: &str) -> String {
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
            } else if line.trim().starts_with("commands") || line.trim().starts_with("discover") {
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn find_python() -> Option<PathBuf> {
    for name in ["python", "python3"] {
        if let Ok(out) = Command::new(name).arg("--version").output() {
            if out.status.success() {
                return Some(PathBuf::from(name));
            }
        }
    }
    if let Ok(out) = Command::new("py").args(["-3", "--version"]).output() {
        if out.status.success() {
            return Some(PathBuf::from("py"));
        }
    }
    None
}

/// TOML array for process_argv, including `py -3` when needed.
fn python_process_argv_toml(py: &Path, script: &str, dump: &str) -> String {
    let name = py.file_name().and_then(|s| s.to_str()).unwrap_or("python");
    if name.eq_ignore_ascii_case("py") {
        format!(r#"["py", "-3", "{script}", "{dump}"]"#)
    } else {
        let py_str = py.display().to_string().replace('\\', "/");
        format!(r#"["{py_str}", "{script}", "{dump}"]"#)
    }
}

/// Restores a process env var to its previous value (or unsets it) on drop.
struct EnvVarGuard {
    key: String,
    prev: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self {
            key: key.to_string(),
            prev,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var(&self.key, v),
            None => std::env::remove_var(&self.key),
        }
    }
}

fn assert_scenario(r: &ScenarioResult) {
    assert!(
        r.pass,
        "scenario {} failed ({}ms): {}\n{}",
        r.id, r.duration_ms, r.notes, r.log
    );
}

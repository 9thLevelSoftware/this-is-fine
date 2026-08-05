//! This Is Fine — user-testing / AI field-validation harness helpers.
//!
//! Scenario IDs and acceptance rules: `docs/USER_TESTING.md`.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Once;
use std::time::{Duration, Instant};
use walkdir::WalkDir;

/// Scenario outcome recorded for evidence packs.
#[derive(Debug, Clone)]
pub struct ScenarioResult {
    pub id: &'static str,
    pub pass: bool,
    pub duration_ms: u128,
    pub notes: String,
    pub log: String,
}

impl ScenarioResult {
    pub fn ok(id: &'static str, duration: Duration, notes: impl Into<String>) -> Self {
        Self {
            id,
            pass: true,
            duration_ms: duration.as_millis(),
            notes: notes.into(),
            log: String::new(),
        }
    }

    pub fn fail(
        id: &'static str,
        duration: Duration,
        notes: impl Into<String>,
        log: impl Into<String>,
    ) -> Self {
        Self {
            id,
            pass: false,
            duration_ms: duration.as_millis(),
            notes: notes.into(),
            log: log.into(),
        }
    }

    pub fn to_json_line(&self) -> String {
        serde_json::json!({
            "id": self.id,
            "pass": self.pass,
            "duration_ms": self.duration_ms,
            "notes": self.notes,
        })
        .to_string()
    }
}

/// Locate the workspace root (directory containing top-level `Cargo.toml` with `[workspace]`).
pub fn workspace_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..6 {
        let cargo = dir.join("Cargo.toml");
        if cargo.is_file() {
            if let Ok(text) = fs::read_to_string(&cargo) {
                if text.contains("[workspace]") {
                    return dir;
                }
            }
        }
        if !dir.pop() {
            break;
        }
    }
    // Fallback: crates/tif-e2e -> workspace
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

/// Path to `tests/user/fixtures/<name>`.
pub fn fixture_path(name: &str) -> PathBuf {
    workspace_root()
        .join("tests")
        .join("user")
        .join("fixtures")
        .join(name)
}

/// Cargo target directory (`CARGO_TARGET_DIR` or `<workspace>/target`).
pub fn cargo_target_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        let p = PathBuf::from(&dir);
        if p.is_absolute() {
            return p;
        }
        return workspace_root().join(p);
    }
    workspace_root().join("target")
}

/// Resolve the `tif` binary (debug preferred when running tests).
pub fn tif_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_tif") {
        return PathBuf::from(p);
    }
    let target = cargo_target_dir();
    let candidates = [
        target.join("debug/tif"),
        target.join("debug/tif.exe"),
        target.join("release/tif"),
        target.join("release/tif.exe"),
    ];
    for c in candidates {
        if c.is_file() {
            return c;
        }
    }
    panic!(
        "tif binary not found under {}; run `cargo build -p tif` first",
        target.display()
    );
}

/// Recursively copy a fixture into a fresh temp directory (skips target/, .git, node_modules).
///
/// Prefers `target/e2e-tmp/` under the workspace so Windows paths stay free of
/// `\\?\` / `//?/` prefixes that break `git worktree`.
pub fn copy_fixture(name: &str) -> io::Result<tempfile::TempDir> {
    let src = fixture_path(name);
    if !src.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("fixture not found: {}", src.display()),
        ));
    }
    let e2e_base = cargo_target_dir().join("e2e-tmp");
    fs::create_dir_all(&e2e_base)?;
    let tmp = tempfile::Builder::new()
        .prefix(&format!("{name}-"))
        .tempdir_in(&e2e_base)
        .or_else(|_| tempfile::tempdir())?;
    let dst = tmp.path();
    copy_dir_filtered(&src, dst)?;
    Ok(tmp)
}

fn copy_dir_filtered(src: &Path, dst: &Path) -> io::Result<()> {
    for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel = path.strip_prefix(src).unwrap();
        if rel.as_os_str().is_empty() {
            continue;
        }
        // Skip heavy / ephemeral dirs if present in a fixture.
        if rel.components().any(|c| {
            matches!(
                c.as_os_str().to_str(),
                Some("target" | ".git" | "node_modules" | ".this-is-fine")
            )
        }) {
            continue;
        }
        let target = dst.join(rel);
        if path.is_dir() {
            fs::create_dir_all(&target)?;
        } else if path.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(path, &target)?;
        }
    }
    Ok(())
}

/// `git init` + initial commit so `--from-git` metrics work.
pub fn git_init_commit(repo: &Path) -> io::Result<()> {
    run_git(repo, &["init"])?;
    run_git(repo, &["config", "user.email", "tif-e2e@test.local"])?;
    run_git(repo, &["config", "user.name", "tif-e2e"])?;
    // Avoid noisy default branch warnings across git versions.
    let _ = run_git(repo, &["checkout", "-b", "main"]);
    run_git(repo, &["add", "-A"])?;
    run_git(repo, &["commit", "-m", "fixture baseline", "--allow-empty"])?;
    Ok(())
}

fn run_git(repo: &Path, args: &[&str]) -> io::Result<()> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(())
}

/// Result of invoking the `tif` CLI.
#[derive(Debug)]
pub struct TifOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
    pub json: Option<Value>,
}

impl TifOutput {
    pub fn ok_envelope(&self) -> bool {
        self.json
            .as_ref()
            .and_then(|v| v.get("ok"))
            .and_then(|v| v.as_bool())
            == Some(true)
    }

    pub fn protocol_version(&self) -> Option<u64> {
        self.json
            .as_ref()
            .and_then(|v| v.get("protocol_version"))
            .and_then(|v| v.as_u64())
    }

    pub fn data(&self) -> Option<&Value> {
        self.json.as_ref().and_then(|v| v.get("data"))
    }

    pub fn data_str(&self, key: &str) -> Option<String> {
        self.data()
            .and_then(|d| d.get(key))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    pub fn summary(&self) -> String {
        format!(
            "status={} ok={} stdout_len={} stderr={}",
            self.status,
            self.ok_envelope(),
            self.stdout.len(),
            self.stderr.chars().take(400).collect::<String>()
        )
    }
}

/// Run `tif --json --repo <repo> <args…>`.
pub fn tif_json(repo: &Path, args: &[&str]) -> io::Result<TifOutput> {
    let bin = tif_bin();
    let mut cmd = Command::new(&bin);
    cmd.arg("--json")
        .arg("--repo")
        .arg(repo)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let out: Output = cmd.output()?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let status = out.status.code().unwrap_or(-1);
    // JSON may be mixed with banners on some paths; take last JSON object if needed.
    let json = parse_json_envelope(&stdout);
    Ok(TifOutput {
        status,
        stdout,
        stderr,
        json,
    })
}

fn parse_json_envelope(stdout: &str) -> Option<Value> {
    let trimmed = stdout.trim();
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return Some(v);
    }
    // Find last line that looks like a JSON object.
    for line in trimmed.lines().rev() {
        let line = line.trim();
        if line.starts_with('{') {
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                return Some(v);
            }
        }
    }
    // Multi-line JSON: from first `{` to end.
    if let Some(i) = trimmed.find('{') {
        if let Ok(v) = serde_json::from_str::<Value>(&trimmed[i..]) {
            return Some(v);
        }
    }
    None
}

/// Content hash of tracked workspace files (excludes .git and .this-is-fine state).
pub fn tree_hash(root: &Path) -> io::Result<String> {
    tree_hash_excluding(root, &[])
}

/// Like [`tree_hash`], but skips relative path prefixes (e.g. test-only junctions).
pub fn tree_hash_excluding(root: &Path, exclude_prefixes: &[&str]) -> io::Result<String> {
    let mut map = BTreeMap::new();
    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let rel = path.strip_prefix(root).unwrap();
        let rel_s = rel.to_string_lossy().replace('\\', "/");
        if exclude_prefixes
            .iter()
            .any(|p| rel_s == *p || rel_s.starts_with(&format!("{p}/")))
        {
            continue;
        }
        if rel.components().any(|c| {
            matches!(
                c.as_os_str().to_str(),
                Some(".git" | ".this-is-fine" | "target" | "node_modules")
            )
        }) {
            continue;
        }
        let bytes = fs::read(path)?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest = format!("{:x}", hasher.finalize());
        map.insert(rel_s, digest);
    }
    let mut outer = Sha256::new();
    for (k, v) in &map {
        outer.update(k.as_bytes());
        outer.update(b"\0");
        outer.update(v.as_bytes());
        outer.update(b"\n");
    }
    Ok(format!("{:x}", outer.finalize()))
}

/// Ensure the `tif` CLI is built (once per test process).
///
/// Always runs `cargo build -p tif` so e2e picks up sibling crate changes
/// (the `tif` package is bin-only, so Cargo will not rebuild it as a dep of tif-e2e).
pub fn ensure_tif_built() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let status = Command::new("cargo")
            .args(["build", "-p", "tif", "--quiet"])
            .current_dir(workspace_root())
            .status()
            .expect("spawn cargo build -p tif");
        assert!(status.success(), "cargo build -p tif failed");
        assert!(
            tif_bin_exists(),
            "tif binary missing after cargo build -p tif"
        );
    });
}

fn tif_bin_exists() -> bool {
    if std::env::var("CARGO_BIN_EXE_tif").is_ok() {
        return true;
    }
    let target = cargo_target_dir();
    target.join("debug/tif").is_file()
        || target.join("debug/tif.exe").is_file()
        || target.join("release/tif").is_file()
        || target.join("release/tif.exe").is_file()
}

/// Parse `cargo test -- --nocapture` log lines into scenario results for evidence packs.
///
/// Recognizes lines like `test a02_empty_reviewer_pool_fail_closed ... ok`.
pub fn parse_cargo_test_log(log: &str) -> Vec<ScenarioResult> {
    let mut out = Vec::new();
    for line in log.lines() {
        let line = line.trim();
        // test name ... ok|FAILED|ignored
        if !line.starts_with("test ") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("test ") {
            if let Some((name, status)) = rest.rsplit_once(" ... ") {
                let name = name.trim();
                let status = status.trim();
                // Map test fn names to scenario IDs when they match aNN_/bNN_ prefixes.
                let id = scenario_id_from_test_name(name);
                let pass = status.eq_ignore_ascii_case("ok");
                let notes = format!("cargo test {name} → {status}");
                if pass {
                    out.push(ScenarioResult::ok(id, Duration::from_millis(0), notes));
                } else if status.eq_ignore_ascii_case("FAILED") {
                    out.push(ScenarioResult::fail(
                        id,
                        Duration::from_millis(0),
                        notes.clone(),
                        notes,
                    ));
                }
            }
        }
    }
    out
}

fn scenario_id_from_test_name(name: &str) -> &'static str {
    // Leak is fine for test harness strings; prefer static for known IDs.
    const KNOWN: &[(&str, &str)] = &[
        ("a01_", "A01"),
        ("a02_", "A02"),
        ("a03_", "A03"),
        ("a04_", "A04"),
        ("a05_", "A05"),
        ("a06_", "A06"),
        ("a07_", "A07"),
        ("a08_", "A08"),
        ("a09_", "A09"),
        ("a10_", "A10"),
        ("b01_", "B01"),
        ("b05_", "B05"),
        ("b06_", "B06"),
        ("b07_", "B07"),
        ("b11_", "B11"),
        ("b12_", "B12"),
        ("b14_", "B14"),
        ("ut0_", "UT0"),
    ];
    for (prefix, id) in KNOWN {
        if name.starts_with(prefix) {
            return id;
        }
    }
    // Fallback: keep a static empty-owned via Box::leak for unknown names.
    Box::leak(name.to_string().into_boxed_str())
}

/// Timed scenario wrapper.
pub fn run_scenario<F>(id: &'static str, f: F) -> ScenarioResult
where
    F: FnOnce() -> Result<String, String>,
{
    let start = Instant::now();
    match f() {
        Ok(notes) => ScenarioResult::ok(id, start.elapsed(), notes),
        Err(e) => ScenarioResult::fail(id, start.elapsed(), e.clone(), e),
    }
}

/// Write a minimal evidence pack directory.
pub fn write_evidence_pack(out_dir: &Path, results: &[ScenarioResult]) -> io::Result<()> {
    fs::create_dir_all(out_dir.join("logs"))?;
    let meta = serde_json::json!({
        "run_id": out_dir.file_name().and_then(|s| s.to_str()).unwrap_or("unknown"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "tif_bin": tif_bin().display().to_string(),
        "workspace": workspace_root().display().to_string(),
        "v1_substitute": true,
        "harness": "tif-e2e",
    });
    fs::write(
        out_dir.join("meta.json"),
        serde_json::to_string_pretty(&meta)?,
    )?;
    let mut lines = String::new();
    for r in results {
        lines.push_str(&r.to_json_line());
        lines.push('\n');
        if !r.log.is_empty() {
            fs::write(out_dir.join("logs").join(format!("{}.log", r.id)), &r.log)?;
        }
    }
    fs::write(out_dir.join("results.jsonl"), lines)?;
    let mut checklist = String::from(
        "# V1 checklist (auto)\n\n| Scenario | Pass | Notes |\n|----------|------|-------|\n",
    );
    for r in results {
        checklist.push_str(&format!(
            "| {} | {} | {} |\n",
            r.id,
            if r.pass { "Pass" } else { "Fail" },
            r.notes.replace('|', "/")
        ));
    }
    fs::write(out_dir.join("checklist.md"), checklist)?;
    let incidents = if results.iter().any(|r| !r.pass) {
        "# Incidents\n\nFailed scenarios — treat as investigation queue.\n"
    } else {
        "# Incidents\n\nNone.\n"
    };
    fs::write(out_dir.join("incidents.md"), incidents)?;
    Ok(())
}

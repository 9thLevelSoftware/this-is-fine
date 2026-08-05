//! Verification planner and runner.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::VerificationConfig;
use crate::error::{Result, TifError};
use crate::inspector::{DiscoveredCommand, RepoInspection};

/// A planned verification check.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationCheck {
    pub id: String,
    pub category: VerificationCategory,
    pub command: String,
    pub source: CheckSource,
    pub required: bool,
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationCategory {
    Build,
    UnitTest,
    IntegrationTest,
    Lint,
    TypeCheck,
    Format,
    Security,
    Custom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckSource {
    ExplicitConfig,
    Discovered,
    Unresolved,
}

/// Outcome of a single check.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CheckResult {
    pub check: VerificationCheck,
    pub status: CheckStatus,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub stdout_tail: String,
    pub stderr_tail: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    Failed,
    Unresolved,
    Skipped,
    TimedOut,
}

/// Full verification report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerificationReport {
    pub checks: Vec<CheckResult>,
    /// True only when there is at least one required check and every required check passed.
    pub all_required_passed: bool,
    /// Any check ended Unresolved (including optional low-confidence discoveries).
    pub has_unresolved: bool,
    /// A **required** check is Unresolved (uncertain required work — not a pass).
    pub has_unresolved_required: bool,
    /// True when the plan is empty / only unresolved placeholders (not a verified pass).
    pub incomplete_plan: bool,
}

impl VerificationReport {
    /// No usable verification plan — **not** a verification pass.
    pub fn empty_incomplete() -> Self {
        Self {
            checks: Vec::new(),
            all_required_passed: false,
            has_unresolved: true,
            has_unresolved_required: false,
            incomplete_plan: true,
        }
    }

    /// Safe for correctness-floor `verification_passed`.
    ///
    /// Optional low-confidence discoveries may remain `Unresolved` without failing the floor;
    /// only required checks and plan completeness matter. Uncertain checks are never treated
    /// as passed (`all_required_passed` already fails required Unresolved/Failed/TimedOut).
    pub fn satisfies_correctness_verification(&self) -> bool {
        self.all_required_passed && !self.incomplete_plan
    }
}

/// Builds a verification plan from config and inspection.
#[derive(Debug, Default)]
pub struct VerificationPlanner;

impl VerificationPlanner {
    pub fn new() -> Self {
        Self
    }

    pub fn plan(
        &self,
        config: &VerificationConfig,
        inspection: Option<&RepoInspection>,
    ) -> Vec<VerificationCheck> {
        let mut checks = Vec::new();

        if !config.commands.is_empty() {
            for (i, cmd) in config.commands.iter().enumerate() {
                checks.push(VerificationCheck {
                    id: format!("explicit-{i}"),
                    category: categorize_command(cmd),
                    command: cmd.clone(),
                    source: CheckSource::ExplicitConfig,
                    required: true,
                    evidence: Some("configured in .this-is-fine.toml".into()),
                });
            }
            return checks;
        }

        if config.discover {
            if let Some(insp) = inspection {
                for (i, dc) in insp.verification_commands.iter().enumerate() {
                    checks.push(discovered_to_check(i, dc));
                }
            }
        }

        if checks.is_empty() {
            checks.push(VerificationCheck {
                id: "unresolved-0".into(),
                category: VerificationCategory::Custom,
                command: String::new(),
                source: CheckSource::Unresolved,
                required: false,
                evidence: Some(
                    "no verification commands configured or discovered with confidence".into(),
                ),
            });
        }

        checks
    }
}

fn discovered_to_check(i: usize, dc: &DiscoveredCommand) -> VerificationCheck {
    VerificationCheck {
        id: format!("discovered-{i}"),
        category: dc.category,
        command: dc.command.clone(),
        source: if dc.confident {
            CheckSource::Discovered
        } else {
            CheckSource::Unresolved
        },
        required: dc.confident,
        evidence: Some(dc.evidence.clone()),
    }
}

fn categorize_command(cmd: &str) -> VerificationCategory {
    let c = cmd.to_ascii_lowercase();
    if c.contains("fmt") || c.contains("format") {
        VerificationCategory::Format
    } else if c.contains("clippy") || c.contains("lint") || c.contains("eslint") {
        VerificationCategory::Lint
    } else if c.contains("test") {
        if c.contains("integration") {
            VerificationCategory::IntegrationTest
        } else {
            VerificationCategory::UnitTest
        }
    } else if c.contains("typecheck") || c.contains("tsc") || c.contains("mypy") {
        VerificationCategory::TypeCheck
    } else if c.contains("build") || c.contains("compile") {
        VerificationCategory::Build
    } else if c.contains("audit") || c.contains("security") {
        VerificationCategory::Security
    } else {
        VerificationCategory::Custom
    }
}

/// Executes verification checks in a working directory.
///
/// Commands come from **trusted repository configuration** only. They still run through a
/// shell for cross-platform ergonomics, with control-character rejection and a hard timeout.
#[derive(Debug)]
pub struct VerificationRunner {
    pub timeout: Duration,
    pub dry_run: bool,
}

impl Default for VerificationRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(600),
            dry_run: false,
        }
    }
}

impl VerificationRunner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn run(&self, root: &Path, checks: &[VerificationCheck]) -> Result<VerificationReport> {
        let mut results = Vec::new();
        for check in checks {
            results.push(self.run_one(root, check)?);
        }
        Ok(summarize_results(results))
    }

    fn run_one(&self, root: &Path, check: &VerificationCheck) -> Result<CheckResult> {
        if check.source == CheckSource::Unresolved || check.command.is_empty() {
            return Ok(CheckResult {
                check: check.clone(),
                status: CheckStatus::Unresolved,
                exit_code: None,
                duration_ms: 0,
                stdout_tail: String::new(),
                stderr_tail: "unresolved check — not treated as passed".into(),
            });
        }

        if let Err(msg) = validate_command_string(&check.command) {
            return Ok(CheckResult {
                check: check.clone(),
                status: CheckStatus::Failed,
                exit_code: None,
                duration_ms: 0,
                stdout_tail: String::new(),
                stderr_tail: msg,
            });
        }

        if self.dry_run {
            return Ok(CheckResult {
                check: check.clone(),
                status: CheckStatus::Skipped,
                exit_code: None,
                duration_ms: 0,
                stdout_tail: format!("dry-run: {}", check.command),
                stderr_tail: String::new(),
            });
        }

        let start = Instant::now();
        let outcome =
            run_shell_command_with_timeout(root, &check.command, self.timeout).map_err(|e| {
                TifError::Verification(format!("failed to spawn '{}': {e}", check.command))
            })?;
        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(CheckResult {
            check: check.clone(),
            status: outcome.status,
            exit_code: outcome.exit_code,
            duration_ms,
            stdout_tail: tail_str(&outcome.stdout, 2000),
            stderr_tail: tail_str(&outcome.stderr, 2000),
        })
    }
}

fn summarize_results(results: Vec<CheckResult>) -> VerificationReport {
    let has_unresolved = results
        .iter()
        .any(|r| matches!(r.status, CheckStatus::Unresolved));
    let has_unresolved_required = results
        .iter()
        .any(|r| r.check.required && matches!(r.status, CheckStatus::Unresolved));
    let has_required = results.iter().any(|r| r.check.required);
    let any_required_failed = results.iter().any(|r| {
        r.check.required && !matches!(r.status, CheckStatus::Passed | CheckStatus::Skipped)
    });
    // Incomplete when there is no required executable check — optional unresolved
    // suggestions alone never constitute a verification plan.
    let incomplete_plan = results.is_empty()
        || !has_required
        || results.iter().all(|r| {
            matches!(r.check.source, CheckSource::Unresolved) || r.check.command.is_empty()
        });

    // Vacuous pass forbidden: need at least one required check and all of them passed.
    let all_required_passed = has_required && !any_required_failed && !incomplete_plan;

    VerificationReport {
        checks: results,
        all_required_passed,
        has_unresolved,
        has_unresolved_required,
        incomplete_plan,
    }
}

struct CommandOutcome {
    status: CheckStatus,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Reject control characters and obvious injection markers in config command strings.
///
/// Note: repository config is trusted; this is defense-in-depth. Adapter-supplied command
/// strings must not be executed without the same policy.
pub fn validate_command_string(command: &str) -> std::result::Result<(), String> {
    if command.is_empty() {
        return Err("empty command".into());
    }
    if command.chars().any(|c| c.is_control() && c != '\t') {
        return Err("command contains control characters (rejected)".into());
    }
    for marker in ['\n', '\r', '`'] {
        if command.contains(marker) {
            return Err(format!(
                "command contains disallowed character (U+{:04X})",
                marker as u32
            ));
        }
    }
    if command.contains("$(") || command.contains("${") {
        return Err(
            "command contains disallowed shell expansion `$(` / `${` (use simple commands)".into(),
        );
    }
    Ok(())
}

fn run_shell_command_with_timeout(
    root: &Path,
    command: &str,
    timeout: Duration,
) -> std::io::Result<CommandOutcome> {
    let mut child = spawn_verification_shell(root, command)?;

    let start = Instant::now();
    loop {
        match child.try_wait()? {
            Some(_) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_end(&mut stdout);
                }
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_end(&mut stderr);
                }
                let status = child.wait()?;
                return Ok(CommandOutcome {
                    status: if status.success() {
                        CheckStatus::Passed
                    } else {
                        CheckStatus::Failed
                    },
                    exit_code: status.code(),
                    stdout: String::from_utf8_lossy(&stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr).into_owned(),
                });
            }
            None => {
                if start.elapsed() >= timeout {
                    kill_child_tree(&child);
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(CommandOutcome {
                        status: CheckStatus::TimedOut,
                        exit_code: None,
                        stdout: String::new(),
                        stderr: format!("verification timed out after {}s", timeout.as_secs()),
                    });
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
    }
}

fn spawn_verification_shell(root: &Path, command: &str) -> std::io::Result<std::process::Child> {
    if cfg!(target_os = "windows") {
        Command::new("cmd")
            .args(["/C", command])
            .current_dir(root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    } else {
        // New process group so timeout can kill shell + grandchildren (sleep, cargo, …).
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            Command::new("sh")
                .args(["-c", command])
                .current_dir(root)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .process_group(0)
                .spawn()
        }
        #[cfg(not(unix))]
        {
            Command::new("sh")
                .args(["-c", command])
                .current_dir(root)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
        }
    }
}

fn kill_child_tree(child: &std::process::Child) {
    let id = child.id();
    if cfg!(target_os = "windows") {
        let _ = Command::new("taskkill")
            .args(["/PID", &id.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    } else {
        // Negative PID = process group (requires spawn with process_group(0)).
        let pgid = format!("-{id}");
        let _ = Command::new("kill")
            .args(["-TERM", &pgid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("kill")
            .args(["-KILL", &pgid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        // Fallback: direct child if not in a group for any reason.
        let _ = Command::new("kill")
            .args(["-KILL", &id.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn tail_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        s[s.len() - max..].to_string()
    }
}

/// Convenience: plan and optionally run.
pub fn plan_and_run(
    root: &Path,
    config: &VerificationConfig,
    inspection: Option<&RepoInspection>,
    dry_run: bool,
) -> Result<VerificationReport> {
    let planner = VerificationPlanner::new();
    let checks = planner.plan(config, inspection);
    let mut runner = VerificationRunner::new();
    runner.dry_run = dry_run;
    runner.run(root, &checks)
}

/// Resolve default root path helpers.
pub fn normalize_root(path: Option<PathBuf>) -> PathBuf {
    path.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VerificationConfig;

    #[test]
    fn prefers_explicit_commands() {
        let cfg = VerificationConfig {
            commands: vec!["cargo test".into(), "cargo fmt --check".into()],
            discover: true,
        };
        let checks = VerificationPlanner::new().plan(&cfg, None);
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].source, CheckSource::ExplicitConfig);
        assert_eq!(checks[0].category, VerificationCategory::UnitTest);
        assert_eq!(checks[1].category, VerificationCategory::Format);
    }

    #[test]
    fn unresolved_when_empty() {
        let cfg = VerificationConfig {
            commands: vec![],
            discover: false,
        };
        let checks = VerificationPlanner::new().plan(&cfg, None);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].source, CheckSource::Unresolved);
        assert!(!checks[0].required);
    }

    #[test]
    fn empty_plan_is_not_a_pass() {
        let report = summarize_results(vec![]);
        assert!(!report.all_required_passed);
        assert!(report.incomplete_plan);
        assert!(!report.satisfies_correctness_verification());
    }

    #[test]
    fn only_unresolved_is_not_a_pass() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = VerificationConfig {
            commands: vec![],
            discover: false,
        };
        let report = plan_and_run(dir.path(), &cfg, None, false).unwrap();
        assert!(report.incomplete_plan || report.has_unresolved);
        assert!(!report.satisfies_correctness_verification());
        assert!(!report.all_required_passed);
    }

    #[test]
    fn optional_unresolved_does_not_fail_required_pass() {
        // Required cargo-test passed; low-confidence fmt discovery remains Unresolved.
        let results = vec![
            CheckResult {
                check: VerificationCheck {
                    id: "discovered-0".into(),
                    category: VerificationCategory::UnitTest,
                    command: "cargo test".into(),
                    source: CheckSource::Discovered,
                    required: true,
                    evidence: Some("Cargo.toml present".into()),
                },
                status: CheckStatus::Passed,
                exit_code: Some(0),
                duration_ms: 10,
                stdout_tail: String::new(),
                stderr_tail: String::new(),
            },
            CheckResult {
                check: VerificationCheck {
                    id: "discovered-1".into(),
                    category: VerificationCategory::Format,
                    command: "cargo fmt --check".into(),
                    source: CheckSource::Unresolved,
                    required: false,
                    evidence: Some("fmt suggested, not required".into()),
                },
                status: CheckStatus::Unresolved,
                exit_code: None,
                duration_ms: 0,
                stdout_tail: String::new(),
                stderr_tail: "unresolved check — not treated as passed".into(),
            },
        ];
        let report = summarize_results(results);
        assert!(report.has_unresolved);
        assert!(!report.has_unresolved_required);
        assert!(report.all_required_passed);
        assert!(!report.incomplete_plan);
        assert!(
            report.satisfies_correctness_verification(),
            "optional unresolved must not poison the correctness floor"
        );
    }

    #[test]
    fn dry_run_skips_execution() {
        let dir = tempfile::tempdir().unwrap();
        let checks = vec![VerificationCheck {
            id: "t".into(),
            category: VerificationCategory::Custom,
            command: "echo hello".into(),
            source: CheckSource::ExplicitConfig,
            required: true,
            evidence: None,
        }];
        let runner = VerificationRunner {
            dry_run: true,
            ..Default::default()
        };
        let report = runner.run(dir.path(), &checks).unwrap();
        assert!(report.all_required_passed);
        assert!(!report.incomplete_plan);
        assert_eq!(report.checks[0].status, CheckStatus::Skipped);
    }

    #[test]
    fn rejects_control_characters_in_commands() {
        assert!(validate_command_string("cargo test\nrm -rf /").is_err());
        assert!(validate_command_string("cargo test $(evil)").is_err());
        assert!(validate_command_string("cargo test").is_ok());
    }

    #[test]
    fn timeout_kills_hanging_command() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = if cfg!(target_os = "windows") {
            "ping -n 30 127.0.0.1 >nul"
        } else {
            "sleep 30"
        };
        let checks = vec![VerificationCheck {
            id: "hang".into(),
            category: VerificationCategory::Custom,
            command: cmd.into(),
            source: CheckSource::ExplicitConfig,
            required: true,
            evidence: None,
        }];
        let runner = VerificationRunner {
            timeout: Duration::from_millis(400),
            dry_run: false,
        };
        let report = runner.run(dir.path(), &checks).unwrap();
        assert_eq!(report.checks[0].status, CheckStatus::TimedOut);
        assert!(!report.all_required_passed);
    }
}

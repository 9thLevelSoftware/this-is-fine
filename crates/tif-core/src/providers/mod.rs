//! Reviewer provider backends (Phase 1).
//!
//! Feature flags (see `Cargo.toml`):
//! - `provider-mock` (default): fixture backend for tests and offline CI
//! - `provider-openai-compatible`: HTTP OpenAI-compatible chat completions
//! - `provider-anthropic`: Anthropic Messages API (thin HTTP client)
//! - `provider-process`: local process / CLI wrapper
//!
//! Backends write **only** under the isolation workspace. Apply to the user
//! workspace still requires isolation re-verify + authorize (Phase 2).

pub mod context;

use crate::config::ReviewerConfig;
use crate::error::{Result, TifError};
use crate::policy::ContainmentPolicy;
use crate::task::TaskCategory;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

pub use context::{
    apply_file_tree_to_dir, assert_clean_room_package, build_reviewer_context,
    parse_reviewer_file_tree, validate_relative_path, ContextBuildRequest, ReviewerContextPackage,
    ReviewerFileTreeOutput, ReviewerInvocationMode,
};

/// Work package sent to an authorized reviewer backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewerTask {
    pub run_id: String,
    pub reviewer: ReviewerConfig,
    pub task_category: TaskCategory,
    pub task_text: Option<String>,
    pub acceptance_criteria: Option<String>,
    pub policy: ContainmentPolicy,
    /// Isolation workspace the reviewer may write into (never the user source root).
    pub isolation_root: PathBuf,
    /// Optional path to a redacted unified diff or summary artifact on disk.
    pub context_artifact: Option<PathBuf>,
    /// Pre-built context package (system + user prompts).
    pub context: ReviewerContextPackage,
    pub max_output_bytes: u64,
    /// Resolved API key / secret (never log this).
    #[serde(skip_serializing)]
    pub credential: Option<String>,
}

/// Structured result from a reviewer backend (untrusted until re-verified).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewerPatch {
    pub reviewer_id: String,
    pub provider: String,
    pub model: String,
    /// Root of the candidate tree written by the backend (under isolation).
    pub candidate_root: PathBuf,
    #[serde(default)]
    pub unified_diff: Option<String>,
    pub notes: Vec<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// Lightweight connectivity probe result (no repo mutation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub reviewer_id: String,
    pub provider: String,
    pub ok: bool,
    pub message: String,
    pub latency_ms: u64,
}

/// Pluggable reviewer execution plane.
pub trait ReviewerBackend: Send + Sync {
    fn provider_kind(&self) -> &str;

    /// Generate a smaller candidate inside the isolation workspace.
    ///
    /// Implementations **must not** write outside `task.isolation_root`.
    fn complete(&self, task: &ReviewerTask) -> Result<ReviewerPatch>;

    /// Optional connectivity check (default: ok for offline backends).
    fn probe(&self, reviewer: &ReviewerConfig, credential: Option<&str>) -> Result<ProbeResult> {
        let _ = credential;
        Ok(ProbeResult {
            reviewer_id: reviewer.id.clone(),
            provider: reviewer.provider.clone(),
            ok: true,
            message: format!("provider `{}` probe not required", self.provider_kind()),
            latency_ms: 0,
        })
    }
}

/// Registry of backends available in this build.
#[derive(Default)]
pub struct BackendRegistry {
    backends: Vec<Box<dyn ReviewerBackend>>,
}

impl BackendRegistry {
    pub fn new() -> Self {
        let mut reg = Self {
            backends: Vec::new(),
        };
        #[cfg(feature = "provider-mock")]
        reg.register(Box::new(mock::MockBackend));
        #[cfg(feature = "provider-openai-compatible")]
        reg.register(Box::new(openai_compatible::OpenAiCompatibleBackend));
        #[cfg(feature = "provider-anthropic")]
        reg.register(Box::new(anthropic::AnthropicBackend));
        #[cfg(feature = "provider-process")]
        reg.register(Box::new(process::ProcessBackend));
        reg
    }

    pub fn register(&mut self, backend: Box<dyn ReviewerBackend>) {
        self.backends.push(backend);
    }

    pub fn get(&self, provider_kind: &str) -> Option<&dyn ReviewerBackend> {
        self.backends
            .iter()
            .find(|b| b.provider_kind() == provider_kind)
            .map(|b| b.as_ref())
    }

    pub fn list_kinds(&self) -> Vec<String> {
        self.backends
            .iter()
            .map(|b| b.provider_kind().to_string())
            .collect()
    }
}

/// Resolve a backend for a configured provider string; never invents unauthorized models.
pub fn backend_for_provider<'a>(
    registry: &'a BackendRegistry,
    provider: &str,
) -> Result<&'a dyn ReviewerBackend> {
    // Normalize common aliases.
    let kind = match provider {
        "openai" | "openai-compatible" => "openai_compatible",
        "ollama" => "openai_compatible",
        other => other,
    };
    registry.get(kind).ok_or_else(|| {
        TifError::UnauthorizedReviewer(format!(
            "no compiled backend for provider `{provider}` (available: {:?})",
            registry.list_kinds()
        ))
    })
}

/// Seed isolation with a copy of the source tree for the reviewer to edit.
pub fn stage_source_into_isolation(
    source_root: &std::path::Path,
    isolation_root: &std::path::Path,
) -> Result<()> {
    copy_dir_filtered(source_root, isolation_root, source_root)?;
    Ok(())
}

fn copy_dir_filtered(
    src: &std::path::Path,
    dst: &std::path::Path,
    root: &std::path::Path,
) -> Result<()> {
    use std::fs;
    use std::path::Component;
    fs::create_dir_all(dst)?;
    // follow_links(false) is the WalkDir default — never follow symlink dirs.
    for entry in walkdir::WalkDir::new(src)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        // Skip symlinks entirely (file or dir).
        if entry.path_is_symlink() || entry.file_type().is_symlink() {
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .map_err(|e| TifError::Other(format!("strip prefix: {e}")))?;
        // Reject `.` / `..` components and skip heavy / local state.
        if rel.components().any(|c| {
            matches!(c, Component::ParentDir | Component::CurDir) || {
                let s = c.as_os_str();
                s == ".git"
                    || s == "target"
                    || s == "node_modules"
                    || s == ".this-is-fine"
                    || s == ".tif-candidate"
            }
        }) {
            continue;
        }
        let target = dst.join(rel);
        // Lexical containment under dst.
        if !target.starts_with(dst) {
            return Err(TifError::Other(format!(
                "copy destination escapes root: {}",
                target.display()
            )));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(entry.path(), &target)?;
    }
    Ok(())
}

fn candidate_dir(isolation_root: &std::path::Path) -> PathBuf {
    isolation_root.join(".tif-candidate")
}

fn ensure_candidate_seeded(isolation_root: &std::path::Path) -> Result<PathBuf> {
    let candidate = candidate_dir(isolation_root);
    if candidate.exists() {
        let _ = std::fs::remove_dir_all(&candidate);
    }
    // Seed from isolation root (which should already be a source snapshot).
    copy_dir_filtered(isolation_root, &candidate, isolation_root)?;
    Ok(candidate)
}

#[cfg(feature = "provider-mock")]
pub mod mock {
    use super::*;
    use std::fs;

    /// Offline backend used in tests and CI. Does not call the network.
    #[derive(Debug)]
    pub struct MockBackend;

    impl ReviewerBackend for MockBackend {
        fn provider_kind(&self) -> &str {
            "mock"
        }

        fn complete(&self, task: &ReviewerTask) -> Result<ReviewerPatch> {
            // Test hook: force backend failure (source untouched).
            if task.isolation_root.join("TIF_MOCK_FAIL").is_file() {
                return Err(TifError::Other(
                    "mock backend forced failure (TIF_MOCK_FAIL present)".into(),
                ));
            }

            let candidate = ensure_candidate_seeded(&task.isolation_root)?;
            // Optional reduction: delete marker file if present (can be a large bloat file).
            let marker = candidate.join("TIF_MOCK_REDUCE");
            if marker.is_file() {
                let _ = fs::remove_file(&marker);
            }
            // Optional inflation for "larger candidate" tests.
            let inflate = task.isolation_root.join("TIF_MOCK_INFLATE");
            if inflate.is_file() {
                let body = fs::read_to_string(&inflate).unwrap_or_else(|_| "INFLATE\n".into());
                let _ = fs::write(candidate.join("TIF_MOCK_BLOAT.txt"), body.repeat(32));
            }
            // Also honor a JSON file tree if present at isolation root.
            let scripted = task.isolation_root.join("TIF_MOCK_OUTPUT.json");
            if scripted.is_file() {
                let text = fs::read_to_string(&scripted)?;
                let tree = parse_reviewer_file_tree(&text)?;
                apply_file_tree_to_dir(&candidate, &tree, task.max_output_bytes)?;
            }
            Ok(ReviewerPatch {
                reviewer_id: task.reviewer.id.clone(),
                provider: "mock".into(),
                model: task.reviewer.model.clone(),
                candidate_root: candidate,
                unified_diff: None,
                notes: vec!["mock backend: no network; candidate seeded from isolation".into()],
                input_tokens: Some(0),
                output_tokens: Some(0),
            })
        }

        fn probe(
            &self,
            reviewer: &ReviewerConfig,
            _credential: Option<&str>,
        ) -> Result<ProbeResult> {
            Ok(ProbeResult {
                reviewer_id: reviewer.id.clone(),
                provider: "mock".into(),
                ok: true,
                message: "mock backend ready (offline)".into(),
                latency_ms: 0,
            })
        }
    }
}

#[cfg(feature = "provider-openai-compatible")]
pub mod openai_compatible {
    use super::*;
    use std::time::Instant;

    /// Parse a chat-completions JSON body (fixture-testable; no network).
    pub fn parse_chat_completion_content(
        status: u16,
        value: &serde_json::Value,
    ) -> Result<(String, Option<u64>, Option<u64>)> {
        if !(200..300).contains(&status) {
            return Err(TifError::Other(format!(
                "openai_compatible HTTP {status}: {value}"
            )));
        }
        let content = value
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TifError::Other(
                    "openai_compatible response missing choices[0].message.content".into(),
                )
            })?
            .to_string();
        let in_tok = value
            .pointer("/usage/prompt_tokens")
            .and_then(|v| v.as_u64());
        let out_tok = value
            .pointer("/usage/completion_tokens")
            .and_then(|v| v.as_u64());
        Ok((content, in_tok, out_tok))
    }

    /// Apply a parsed OpenAI chat response to an isolation tree (contract path).
    pub fn apply_openai_chat_response(
        task: &ReviewerTask,
        status: u16,
        value: &serde_json::Value,
    ) -> Result<ReviewerPatch> {
        let (content, in_tok, out_tok) = parse_chat_completion_content(status, value)?;
        let tree = parse_reviewer_file_tree(&content)?;
        let candidate = ensure_candidate_seeded(&task.isolation_root)?;
        apply_file_tree_to_dir(&candidate, &tree, task.max_output_bytes)?;
        Ok(ReviewerPatch {
            reviewer_id: task.reviewer.id.clone(),
            provider: "openai_compatible".into(),
            model: task.reviewer.model.clone(),
            candidate_root: candidate,
            unified_diff: None,
            notes: tree.notes,
            input_tokens: in_tok,
            output_tokens: out_tok,
        })
    }

    /// OpenAI-compatible HTTP chat completions backend.
    #[derive(Debug)]
    pub struct OpenAiCompatibleBackend;

    impl ReviewerBackend for OpenAiCompatibleBackend {
        fn provider_kind(&self) -> &str {
            "openai_compatible"
        }

        fn complete(&self, task: &ReviewerTask) -> Result<ReviewerPatch> {
            let endpoint = task
                .reviewer
                .endpoint
                .as_deref()
                .unwrap_or("https://api.openai.com/v1");
            let url = format!("{}/chat/completions", endpoint.trim_end_matches('/'));
            let api_key = task.credential.as_deref().ok_or_else(|| {
                TifError::Config(format!(
                    "reviewer `{}` requires credential_ref for openai_compatible",
                    task.reviewer.id
                ))
            })?;

            let max_tokens = task.reviewer.max_output_tokens.unwrap_or(4096);
            let body = serde_json::json!({
                "model": task.reviewer.model,
                "temperature": 0.2,
                "max_tokens": max_tokens,
                "messages": [
                    {"role": "system", "content": task.context.system_prompt},
                    {"role": "user", "content": task.context.user_prompt},
                ]
            });

            let agent = ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(task.reviewer.timeout_secs.max(1)))
                .build();

            let resp = agent
                .post(&url)
                .set("Authorization", &format!("Bearer {api_key}"))
                .set("Content-Type", "application/json")
                .send_json(&body)
                .map_err(|e| TifError::Other(format!("openai_compatible request failed: {e}")))?;

            let status = resp.status();
            let value: serde_json::Value = resp
                .into_json()
                .map_err(|e| TifError::Other(format!("openai_compatible bad JSON: {e}")))?;
            apply_openai_chat_response(task, status, &value)
        }

        fn probe(
            &self,
            reviewer: &ReviewerConfig,
            credential: Option<&str>,
        ) -> Result<ProbeResult> {
            let start = Instant::now();
            let endpoint = reviewer
                .endpoint
                .as_deref()
                .unwrap_or("https://api.openai.com/v1");
            let url = format!("{}/models", endpoint.trim_end_matches('/'));
            let agent = ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(reviewer.timeout_secs.clamp(1, 30)))
                .build();
            let mut req = agent.get(&url);
            if let Some(key) = credential {
                req = req.set("Authorization", &format!("Bearer {key}"));
            }
            match req.call() {
                Ok(resp) => {
                    let ok = (200..300).contains(&resp.status());
                    Ok(ProbeResult {
                        reviewer_id: reviewer.id.clone(),
                        provider: "openai_compatible".into(),
                        ok,
                        message: format!("GET /models -> HTTP {}", resp.status()),
                        latency_ms: start.elapsed().as_millis() as u64,
                    })
                }
                Err(e) => Ok(ProbeResult {
                    reviewer_id: reviewer.id.clone(),
                    provider: "openai_compatible".into(),
                    ok: false,
                    message: format!("probe failed: {e}"),
                    latency_ms: start.elapsed().as_millis() as u64,
                }),
            }
        }
    }
}

#[cfg(feature = "provider-anthropic")]
pub mod anthropic {
    use super::*;
    use std::time::Instant;

    /// Anthropic Messages API backend.
    #[derive(Debug)]
    pub struct AnthropicBackend;

    impl ReviewerBackend for AnthropicBackend {
        fn provider_kind(&self) -> &str {
            "anthropic"
        }

        fn complete(&self, task: &ReviewerTask) -> Result<ReviewerPatch> {
            let endpoint = task
                .reviewer
                .endpoint
                .as_deref()
                .unwrap_or("https://api.anthropic.com");
            let url = format!("{}/v1/messages", endpoint.trim_end_matches('/'));
            let api_key = task.credential.as_deref().ok_or_else(|| {
                TifError::Config(format!(
                    "reviewer `{}` requires credential_ref for anthropic",
                    task.reviewer.id
                ))
            })?;

            let max_tokens = task.reviewer.max_output_tokens.unwrap_or(4096);
            let body = serde_json::json!({
                "model": task.reviewer.model,
                "max_tokens": max_tokens,
                "system": task.context.system_prompt,
                "messages": [
                    {"role": "user", "content": task.context.user_prompt}
                ]
            });

            let agent = ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(task.reviewer.timeout_secs.max(1)))
                .build();

            let resp = agent
                .post(&url)
                .set("x-api-key", api_key)
                .set("anthropic-version", "2023-06-01")
                .set("Content-Type", "application/json")
                .send_json(&body)
                .map_err(|e| TifError::Other(format!("anthropic request failed: {e}")))?;

            let status = resp.status();
            let value: serde_json::Value = resp
                .into_json()
                .map_err(|e| TifError::Other(format!("anthropic bad JSON: {e}")))?;
            if !(200..300).contains(&status) {
                return Err(TifError::Other(format!("anthropic HTTP {status}: {value}")));
            }

            let content = value
                .pointer("/content/0/text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    TifError::Other("anthropic response missing content[0].text".into())
                })?;

            let tree = parse_reviewer_file_tree(content)?;
            let candidate = ensure_candidate_seeded(&task.isolation_root)?;
            apply_file_tree_to_dir(&candidate, &tree, task.max_output_bytes)?;

            Ok(ReviewerPatch {
                reviewer_id: task.reviewer.id.clone(),
                provider: "anthropic".into(),
                model: task.reviewer.model.clone(),
                candidate_root: candidate,
                unified_diff: None,
                notes: tree.notes,
                input_tokens: value
                    .pointer("/usage/input_tokens")
                    .and_then(|v| v.as_u64()),
                output_tokens: value
                    .pointer("/usage/output_tokens")
                    .and_then(|v| v.as_u64()),
            })
        }

        fn probe(
            &self,
            reviewer: &ReviewerConfig,
            credential: Option<&str>,
        ) -> Result<ProbeResult> {
            let start = Instant::now();
            // Anthropic has no lightweight public list; do a minimal validated auth check shape.
            if credential.is_none() {
                return Ok(ProbeResult {
                    reviewer_id: reviewer.id.clone(),
                    provider: "anthropic".into(),
                    ok: false,
                    message: "missing credential".into(),
                    latency_ms: 0,
                });
            }
            Ok(ProbeResult {
                reviewer_id: reviewer.id.clone(),
                provider: "anthropic".into(),
                ok: true,
                message: "credential present; use complete() for full path (no free list endpoint)"
                    .into(),
                latency_ms: start.elapsed().as_millis() as u64,
            })
        }
    }
}

#[cfg(feature = "provider-process")]
pub mod process {
    use super::*;
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    /// Local process backend: runs argv with isolation path and request JSON.
    #[derive(Debug)]
    pub struct ProcessBackend;

    impl ReviewerBackend for ProcessBackend {
        fn provider_kind(&self) -> &str {
            "process"
        }

        fn complete(&self, task: &ReviewerTask) -> Result<ReviewerPatch> {
            let argv = task.reviewer.process_argv.as_ref().ok_or_else(|| {
                TifError::Config(format!(
                    "reviewer `{}` provider=process requires process_argv",
                    task.reviewer.id
                ))
            })?;
            if argv.is_empty() {
                return Err(TifError::Config("process_argv is empty".into()));
            }

            let request_path = task.isolation_root.join(".tif-reviewer-request.json");
            let request = serde_json::json!({
                "run_id": task.run_id,
                "reviewer_id": task.reviewer.id,
                "model": task.reviewer.model,
                "system": task.context.system_prompt,
                "user": task.context.user_prompt,
                "includes_source": task.context.includes_source,
            });
            std::fs::write(&request_path, serde_json::to_vec_pretty(&request)?)?;

            let isolation_s = task.isolation_root.to_string_lossy();
            let request_s = request_path.to_string_lossy();
            let mut args: Vec<String> = argv
                .iter()
                .map(|a| {
                    a.replace("{isolation}", &isolation_s)
                        .replace("{request_json}", &request_s)
                })
                .collect();
            let program = args.remove(0);

            let timeout = Duration::from_secs(task.reviewer.timeout_secs.max(1));
            let (stdout, stderr, status_code, timed_out) =
                run_process_with_timeout(&program, &args, &task.isolation_root, timeout)?;

            if timed_out {
                return Err(TifError::Other(format!(
                    "process reviewer timed out after {}s: {stderr}",
                    timeout.as_secs()
                )));
            }
            if status_code != Some(0) {
                return Err(TifError::Other(format!(
                    "process reviewer exited {status_code:?}: {stderr}"
                )));
            }

            let candidate = ensure_candidate_seeded(&task.isolation_root)?;
            // Prefer stdout JSON; else look for TIF_PROCESS_OUTPUT.json in isolation.
            let tree = if stdout.trim().starts_with('{') {
                parse_reviewer_file_tree(&stdout)?
            } else {
                let path = task.isolation_root.join("TIF_PROCESS_OUTPUT.json");
                if path.is_file() {
                    let text = std::fs::read_to_string(path)?;
                    parse_reviewer_file_tree(&text)?
                } else if !stdout.trim().is_empty() {
                    parse_reviewer_file_tree(&stdout)?
                } else {
                    // Process edited the isolation tree in place — candidate already seeded.
                    return Ok(ReviewerPatch {
                        reviewer_id: task.reviewer.id.clone(),
                        provider: "process".into(),
                        model: task.reviewer.model.clone(),
                        candidate_root: candidate,
                        unified_diff: None,
                        notes: vec!["process backend: no JSON output; using seeded tree".into()],
                        input_tokens: None,
                        output_tokens: None,
                    });
                }
            };
            apply_file_tree_to_dir(&candidate, &tree, task.max_output_bytes)?;

            Ok(ReviewerPatch {
                reviewer_id: task.reviewer.id.clone(),
                provider: "process".into(),
                model: task.reviewer.model.clone(),
                candidate_root: candidate,
                unified_diff: None,
                notes: tree.notes,
                input_tokens: None,
                output_tokens: None,
            })
        }

        fn probe(
            &self,
            reviewer: &ReviewerConfig,
            _credential: Option<&str>,
        ) -> Result<ProbeResult> {
            let start = Instant::now();
            let argv = match &reviewer.process_argv {
                Some(a) if !a.is_empty() => a,
                _ => {
                    return Ok(ProbeResult {
                        reviewer_id: reviewer.id.clone(),
                        provider: "process".into(),
                        ok: false,
                        message: "process_argv missing".into(),
                        latency_ms: 0,
                    });
                }
            };
            // Existence check only — do not execute arbitrary probe payloads.
            let program = &argv[0];
            let ok = std::path::Path::new(program).exists() || which_in_path(program);
            Ok(ProbeResult {
                reviewer_id: reviewer.id.clone(),
                provider: "process".into(),
                ok,
                message: if ok {
                    format!("process binary `{program}` found")
                } else {
                    format!("process binary `{program}` not found on PATH or as path")
                },
                latency_ms: start.elapsed().as_millis() as u64,
            })
        }
    }

    /// Spawn process reviewer with env scrub, pipe drain, and timeout (kill tree).
    fn run_process_with_timeout(
        program: &str,
        args: &[String],
        cwd: &std::path::Path,
        timeout: Duration,
    ) -> Result<(String, String, Option<i32>, bool)> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Scrub environment: never pass API keys / secrets. Re-add minimal allowlist.
        apply_minimal_env(&mut cmd);
        // New process group on Unix so timeout can kill grandchildren.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| TifError::Other(format!("failed to spawn process reviewer: {e}")))?;

        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        let stdout_handle = thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut out) = stdout_pipe {
                let _ = out.read_to_end(&mut buf);
            }
            buf
        });
        let stderr_handle = thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut err) = stderr_pipe {
                let _ = err.read_to_end(&mut buf);
            }
            buf
        });

        let start = Instant::now();
        let timed_out = loop {
            match child.try_wait() {
                Ok(Some(_)) => break false,
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        kill_child_tree(&child);
                        let _ = child.kill();
                        break true;
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                Err(e) => {
                    return Err(TifError::Other(format!(
                        "process reviewer wait failed: {e}"
                    )));
                }
            }
        };

        let status = child
            .wait()
            .map_err(|e| TifError::Other(format!("process reviewer wait failed: {e}")))?;
        let stdout =
            String::from_utf8_lossy(&stdout_handle.join().unwrap_or_default()).into_owned();
        let mut stderr =
            String::from_utf8_lossy(&stderr_handle.join().unwrap_or_default()).into_owned();
        if timed_out {
            let msg = format!("process reviewer timed out after {}s", timeout.as_secs());
            if stderr.is_empty() {
                stderr = msg;
            } else {
                stderr = format!("{msg}\n{stderr}");
            }
        }
        Ok((stdout, stderr, status.code(), timed_out))
    }

    /// Clear env and re-add a minimal allowlist (no API keys).
    fn apply_minimal_env(cmd: &mut Command) {
        cmd.env_clear();
        const ALLOW: &[&str] = &[
            "PATH",
            "HOME",
            "USERPROFILE",
            "HOMEDRIVE",
            "HOMEPATH",
            "SystemRoot",
            "SYSTEMROOT",
            "WINDIR",
            "COMSPEC",
            "PATHEXT",
            "LANG",
            "TMP",
            "TEMP",
            "TMPDIR",
            "USER",
            "USERNAME",
            "LOGNAME",
        ];
        for key in ALLOW {
            if let Ok(v) = std::env::var(key) {
                cmd.env(key, v);
            }
        }
        // Pass through locale vars (LC_*), never secrets.
        for (k, v) in std::env::vars() {
            if k.starts_with("LC_") {
                cmd.env(k, v);
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
            let _ = Command::new("kill")
                .args(["-KILL", &id.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }

    fn which_in_path(program: &str) -> bool {
        if let Ok(path) = std::env::var("PATH") {
            for dir in std::env::split_paths(&path) {
                let p = dir.join(program);
                if p.is_file() {
                    return true;
                }
                #[cfg(windows)]
                {
                    let p_exe = dir.join(format!("{program}.exe"));
                    if p_exe.is_file() {
                        return true;
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::policy::{PolicyCompileRequest, PolicyCompiler};
    use tempfile::tempdir;

    fn sample_task(iso: PathBuf) -> ReviewerTask {
        let cfg = Config::default();
        let policy = PolicyCompiler::new()
            .compile(&cfg, &PolicyCompileRequest::default())
            .unwrap();
        let reviewer = ReviewerConfig::mock("r1", 1);
        let ctx = build_reviewer_context(&ContextBuildRequest {
            reviewer: &reviewer,
            policy: &policy,
            task_category: TaskCategory::BugFix,
            task_text: Some("reduce"),
            acceptance_criteria: None,
            original_metrics: None,
            source_or_diff: None,
            verification_plan_summary: None,
            mode: ReviewerInvocationMode::Standard,
            failure_summary: None,
            prior_implementation_code: None,
        })
        .unwrap();
        ReviewerTask {
            run_id: "t1".into(),
            reviewer,
            task_category: TaskCategory::BugFix,
            task_text: Some("reduce".into()),
            acceptance_criteria: None,
            policy,
            isolation_root: iso,
            context_artifact: None,
            context: ctx,
            max_output_bytes: 1_000_000,
            credential: None,
        }
    }

    #[test]
    #[cfg(feature = "provider-mock")]
    fn mock_backend_is_registered() {
        let reg = BackendRegistry::new();
        assert!(reg.list_kinds().iter().any(|k| k == "mock"));
        assert!(backend_for_provider(&reg, "mock").is_ok());
        assert!(backend_for_provider(&reg, "nope").is_err());
    }

    #[test]
    #[cfg(feature = "provider-mock")]
    fn mock_complete_stays_inside_isolation() {
        let dir = tempdir().unwrap();
        let iso = dir.path().join("iso");
        std::fs::create_dir_all(&iso).unwrap();
        std::fs::write(iso.join("main.rs"), b"fn main() {}").unwrap();

        let task = sample_task(iso.clone());
        let backend = mock::MockBackend;
        let patch = backend.complete(&task).unwrap();
        assert!(patch.candidate_root.starts_with(&iso));
        assert!(patch.candidate_root.ends_with(".tif-candidate"));
        assert!(patch.candidate_root.join("main.rs").is_file());
    }

    #[test]
    #[cfg(feature = "provider-mock")]
    fn mock_applies_scripted_json_output() {
        let dir = tempdir().unwrap();
        let iso = dir.path().join("iso");
        std::fs::create_dir_all(&iso).unwrap();
        std::fs::write(iso.join("a.rs"), b"old").unwrap();
        std::fs::write(
            iso.join("TIF_MOCK_OUTPUT.json"),
            r#"{"files":[{"path":"b.rs","content":"new"}],"delete":["a.rs"],"notes":["n"]}"#,
        )
        .unwrap();
        let task = sample_task(iso);
        let patch = mock::MockBackend.complete(&task).unwrap();
        assert!(patch.candidate_root.join("b.rs").is_file());
        assert!(!patch.candidate_root.join("a.rs").exists());
    }

    #[test]
    #[cfg(feature = "provider-openai-compatible")]
    fn openai_alias_resolves() {
        let reg = BackendRegistry::new();
        assert!(backend_for_provider(&reg, "openai").is_ok());
        assert!(backend_for_provider(&reg, "openai_compatible").is_ok());
    }

    #[test]
    #[cfg(feature = "provider-openai-compatible")]
    fn openai_chat_fixture_contract() {
        let fixture = r#"{
          "id": "chatcmpl-test",
          "choices": [{
            "index": 0,
            "message": {
              "role": "assistant",
              "content": "{\"files\":[{\"path\":\"src/lib.rs\",\"content\":\"pub fn ok() {}\\n\"}],\"delete\":[],\"notes\":[\"minimal\"]}"
            },
            "finish_reason": "stop"
          }],
          "usage": { "prompt_tokens": 12, "completion_tokens": 34 }
        }"#;
        let value: serde_json::Value = serde_json::from_str(fixture).unwrap();
        let (content, in_t, out_t) =
            openai_compatible::parse_chat_completion_content(200, &value).unwrap();
        assert!(content.contains("src/lib.rs"));
        assert_eq!(in_t, Some(12));
        assert_eq!(out_t, Some(34));
        assert!(openai_compatible::parse_chat_completion_content(500, &value).is_err());
        assert!(openai_compatible::parse_chat_completion_content(
            200,
            &serde_json::json!({"choices": []})
        )
        .is_err());

        let dir = tempdir().unwrap();
        let iso = dir.path().join("iso");
        std::fs::create_dir_all(iso.join("src")).unwrap();
        std::fs::write(iso.join("src/lib.rs"), b"old").unwrap();
        let task = sample_task(iso);
        let patch = openai_compatible::apply_openai_chat_response(&task, 200, &value).unwrap();
        assert!(patch.candidate_root.join("src/lib.rs").is_file());
        let body = std::fs::read_to_string(patch.candidate_root.join("src/lib.rs")).unwrap();
        assert!(body.contains("pub fn ok"));
        assert_eq!(patch.notes, vec!["minimal".to_string()]);
    }

    #[test]
    #[cfg(feature = "provider-anthropic")]
    fn anthropic_fixture_extracts_text() {
        let value = serde_json::json!({
            "content": [{"type": "text", "text": "{\"files\":[],\"delete\":[],\"notes\":[\"n\"]}"}],
            "usage": {"input_tokens": 1, "output_tokens": 2}
        });
        let text = value
            .pointer("/content/0/text")
            .and_then(|v| v.as_str())
            .unwrap();
        let tree = parse_reviewer_file_tree(text).unwrap();
        assert_eq!(tree.notes, vec!["n".to_string()]);
    }
}

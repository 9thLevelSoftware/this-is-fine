//! Build redacted reviewer context packages with egress enforcement.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::audit::redact_secrets;
use crate::config::ReviewerConfig;
use crate::error::{Result, TifError};
use crate::policy::ContainmentPolicy;
use crate::scoring::DiffMetrics;
use crate::task::TaskCategory;

/// Packaged prompts for a reviewer backend (already redacted).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewerContextPackage {
    pub system_prompt: String,
    pub user_prompt: String,
    /// True only when source/diff content was included (requires allow_source_egress).
    pub includes_source: bool,
    pub truncated: bool,
}

/// How a reviewer invocation is framed (standard Firebreak vs Five-Alarm stages).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewerInvocationMode {
    #[default]
    Standard,
    /// Stage 1: intensified Firebreak — stricter wording, higher attempt budget.
    Intensified,
    /// Stage 3: clean-room — original state + task/policy/failure summary only.
    /// Must never include previous implementation code.
    CleanRoom,
}

/// Inputs for context packaging.
#[derive(Debug, Clone)]
pub struct ContextBuildRequest<'a> {
    pub reviewer: &'a ReviewerConfig,
    pub policy: &'a ContainmentPolicy,
    pub task_category: TaskCategory,
    pub task_text: Option<&'a str>,
    pub acceptance_criteria: Option<&'a str>,
    pub original_metrics: Option<&'a DiffMetrics>,
    /// Unified diff or file excerpts. Included only when `allow_source_egress`.
    /// Forbidden for [`ReviewerInvocationMode::CleanRoom`].
    pub source_or_diff: Option<&'a str>,
    pub verification_plan_summary: Option<&'a str>,
    /// Framing mode (standard / intensified / clean-room).
    pub mode: ReviewerInvocationMode,
    /// Structured failure summary for Five-Alarm (clean-room / intensified).
    pub failure_summary: Option<&'a str>,
    /// Previous implementation patch/code. Clean-room **rejects** non-empty values.
    pub prior_implementation_code: Option<&'a str>,
}

impl<'a> ContextBuildRequest<'a> {
    /// Convenience for standard Firebreak context (legacy call sites).
    pub fn standard(
        reviewer: &'a ReviewerConfig,
        policy: &'a ContainmentPolicy,
        task_category: TaskCategory,
    ) -> Self {
        Self {
            reviewer,
            policy,
            task_category,
            task_text: None,
            acceptance_criteria: None,
            original_metrics: None,
            source_or_diff: None,
            verification_plan_summary: None,
            mode: ReviewerInvocationMode::Standard,
            failure_summary: None,
            prior_implementation_code: None,
        }
    }
}

const SYSTEM_BASE: &str = r#"You are a Firebreak reviewer for This Is Fine.
Your job is to produce the SMALLEST correct implementation that satisfies the task
and verification plan while obeying the containment policy.

Rules:
1. Prefer deletion, reuse, and configuration over new files/dependencies/abstractions.
2. Never weaken security, validation, error handling, or required tests.
3. Do not invent speculative flexibility.
4. Output ONLY a JSON object (no markdown fences) with this shape:
{
  "files": [{"path": "relative/path", "content": "full file contents"}],
  "delete": ["relative/paths/to/remove"],
  "notes": ["brief justification"]
}
Paths must be relative and must not contain `..` or absolute roots.
"#;

const SYSTEM_INTENSIFIED: &str = r#"You are an INTENSIFIED Firebreak reviewer for This Is Fine (Five-Alarm Stage 1).
Your job is MAXIMUM restraint: the SMALLEST correct implementation that satisfies the
task and verification plan while obeying the containment policy.

Strict rules (non-negotiable):
1. Prefer deletion and reuse; forbid new dependencies unless the task cannot succeed without them.
2. Prefer the fewest files, fewest public interfaces, and fewest abstractions possible.
3. Never weaken security, validation, error handling, or required tests.
4. Do not invent speculative flexibility, configuration knobs, or framework layers.
5. Every added line must be justified by the acceptance criteria.
6. Output ONLY a JSON object (no markdown fences) with this shape:
{
  "files": [{"path": "relative/path", "content": "full file contents"}],
  "delete": ["relative/paths/to/remove"],
  "notes": ["brief justification"]
}
Paths must be relative and must not contain `..` or absolute roots.
"#;

const SYSTEM_CLEAN_ROOM: &str = r#"You are a CLEAN-ROOM implementation model for This Is Fine (Five-Alarm Stage 3).
You must solve the task from the original repository state, task description, acceptance
criteria, containment policy, verification plan, and a structured failure summary only.

You will NOT receive previous implementation code, patches, or candidate diffs.
Do not assume or invent details from a prior attempt beyond the failure summary.

Rules:
1. Produce the SMALLEST correct implementation that satisfies the task and verification plan.
2. Prefer deletion, reuse, and configuration over new files/dependencies/abstractions.
3. Never weaken security, validation, error handling, or required tests.
4. Output ONLY a JSON object (no markdown fences) with this shape:
{
  "files": [{"path": "relative/path", "content": "full file contents"}],
  "delete": ["relative/paths/to/remove"],
  "notes": ["brief justification"]
}
Paths must be relative and must not contain `..` or absolute roots.
"#;

/// Build a redacted context package. Enforces egress policy and clean-room invariants.
pub fn build_reviewer_context(req: &ContextBuildRequest<'_>) -> Result<ReviewerContextPackage> {
    // Clean-room hard gate: no previous implementation code in the package.
    if req.mode == ReviewerInvocationMode::CleanRoom {
        if req
            .prior_implementation_code
            .map(|s| !s.is_empty())
            .unwrap_or(false)
        {
            return Err(TifError::Other(
                "clean-room context forbids prior_implementation_code (no previous patch content)"
                    .into(),
            ));
        }
        if req.source_or_diff.map(|s| !s.is_empty()).unwrap_or(false) {
            return Err(TifError::Other(
                "clean-room context forbids source_or_diff (no previous implementation code)"
                    .into(),
            ));
        }
    }

    let want_source = req.mode != ReviewerInvocationMode::CleanRoom
        && req.source_or_diff.map(|s| !s.is_empty()).unwrap_or(false);
    if want_source && !req.reviewer.allow_source_egress {
        return Err(TifError::UnauthorizedReviewer(format!(
            "reviewer `{}` has allow_source_egress=false; cannot include source/diff in provider request",
            req.reviewer.id
        )));
    }

    let system_base = match req.mode {
        ReviewerInvocationMode::Standard => SYSTEM_BASE,
        ReviewerInvocationMode::Intensified => SYSTEM_INTENSIFIED,
        ReviewerInvocationMode::CleanRoom => SYSTEM_CLEAN_ROOM,
    };

    let mut user = String::new();
    if req.mode == ReviewerInvocationMode::Intensified {
        user.push_str(
            "## Mode\nINTENSIFIED Firebreak (Five-Alarm Stage 1) — maximum restraint.\n\n",
        );
    } else if req.mode == ReviewerInvocationMode::CleanRoom {
        user.push_str(
            "## Mode\nCLEAN-ROOM (Five-Alarm Stage 3) — no previous implementation code.\n\n",
        );
    }

    user.push_str("## Task\n");
    user.push_str(&format!("category: {}\n", req.task_category.as_str()));
    if let Some(t) = req.task_text {
        user.push_str("text:\n");
        user.push_str(t);
        user.push('\n');
    }
    if let Some(a) = req.acceptance_criteria {
        user.push_str("\n## Acceptance criteria\n");
        user.push_str(a);
        user.push('\n');
    }

    user.push_str("\n## Containment policy\n");
    user.push_str(&format!("policy_id: {}\n", req.policy.policy_id));
    user.push_str(&format!("fire_level: {}\n", req.policy.fire_level));
    user.push_str(&format!("limits: {:?}\n", req.policy.limits));
    user.push_str(&format!(
        "require_firebreak_approval: {}\n",
        req.policy.require_firebreak_approval
    ));
    user.push_str(&format!(
        "sensitive_paths: {:?}\n",
        req.policy.sensitive_paths
    ));

    if let Some(m) = req.original_metrics {
        user.push_str("\n## Original metrics (fuel)\n");
        user.push_str(&format!(
            "files_added={} files_changed={} lines_added={} deps_added={} abstractions={}\n",
            m.files_added,
            m.files_changed,
            m.lines_added,
            m.runtime_dependencies_added,
            m.abstractions_added
        ));
    }

    if let Some(v) = req.verification_plan_summary {
        user.push_str("\n## Verification plan\n");
        user.push_str(v);
        user.push('\n');
    }

    if let Some(fs) = req.failure_summary {
        if !fs.is_empty() {
            user.push_str("\n## Structured failure summary\n");
            user.push_str(fs);
            user.push('\n');
        }
    }

    let mut includes_source = false;
    if req.mode == ReviewerInvocationMode::CleanRoom {
        user.push_str(
            "\n## Source / previous implementation\n\
             Not provided (clean-room). Implement from task, criteria, policy, verification plan, \
             and failure summary only. Do not request or reconstruct prior patch content.\n",
        );
    } else if want_source {
        if let Some(src) = req.source_or_diff {
            user.push_str("\n## Source / diff (redacted)\n");
            user.push_str(src);
            user.push('\n');
            includes_source = true;
        }
    } else {
        user.push_str(
            "\n## Source\nNot included (egress disabled or not provided). Work only from metrics and policy; prefer minimal structural changes.\n",
        );
    }

    // Always redact secrets before send / log.
    let system_prompt = redact_secrets(system_base);
    let mut user_prompt = redact_secrets(&user);
    let mut truncated = false;
    let max = req.reviewer.max_context_bytes as usize;
    if user_prompt.len() > max {
        // Truncate on char boundary.
        let mut end = max;
        while end > 0 && !user_prompt.is_char_boundary(end) {
            end -= 1;
        }
        user_prompt.truncate(end);
        user_prompt.push_str("\n\n[truncated by max_context_bytes]\n");
        truncated = true;
    }

    // Final clean-room assertion: package must not contain prior patch markers.
    if req.mode == ReviewerInvocationMode::CleanRoom {
        assert_clean_room_package(&system_prompt, &user_prompt, includes_source)?;
    }

    Ok(ReviewerContextPackage {
        system_prompt,
        user_prompt,
        includes_source,
        truncated,
    })
}

/// Assert a clean-room package has no prior implementation / patch content.
pub fn assert_clean_room_package(
    system_prompt: &str,
    user_prompt: &str,
    includes_source: bool,
) -> Result<()> {
    if includes_source {
        return Err(TifError::Other(
            "clean-room package must not include source/diff content".into(),
        ));
    }
    let combined = format!("{system_prompt}\n{user_prompt}").to_lowercase();
    // Markers that indicate previous implementation *payload* leaked into the package.
    // Instructional text that says "do not include prior patch" is allowed; these
    // patterns target actual leaked sections/headers.
    const FORBIDDEN: &[&str] = &[
        "## source / diff",
        "previous implementation code:",
        "unified diff of previous",
        "candidate patch body:",
        "```diff",
        "diff --git ",
    ];
    for marker in FORBIDDEN {
        if combined.contains(marker) {
            return Err(TifError::Other(format!(
                "clean-room package contains forbidden prior-implementation marker: {marker}"
            )));
        }
    }
    Ok(())
}

/// Reject path traversal in reviewer-proposed paths.
pub fn validate_relative_path(path: &str) -> Result<()> {
    let p = Path::new(path);
    if p.is_absolute() {
        return Err(TifError::Other(format!(
            "reviewer path must be relative: {path}"
        )));
    }
    for c in p.components() {
        use std::path::Component;
        match c {
            Component::ParentDir => {
                return Err(TifError::Other(format!(
                    "reviewer path must not contain ..: {path}"
                )));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(TifError::Other(format!(
                    "reviewer path must not be rooted: {path}"
                )));
            }
            _ => {}
        }
    }
    if path.is_empty() {
        return Err(TifError::Other("reviewer path is empty".into()));
    }
    Ok(())
}

/// JSON file-tree output contract from reviewers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReviewerFileTreeOutput {
    #[serde(default)]
    pub files: Vec<ReviewerFileEntry>,
    #[serde(default)]
    pub delete: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewerFileEntry {
    pub path: String,
    pub content: String,
}

/// Parse model text into the file-tree contract (strips optional markdown fences).
pub fn parse_reviewer_file_tree(text: &str) -> Result<ReviewerFileTreeOutput> {
    let trimmed = text.trim();
    let json_slice = if let Some(rest) = trimmed.strip_prefix("```json") {
        rest.strip_suffix("```").unwrap_or(rest).trim()
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest.strip_suffix("```").unwrap_or(rest).trim()
    } else {
        trimmed
    };
    // Try full parse; if fails, attempt to extract outermost JSON object.
    match serde_json::from_str::<ReviewerFileTreeOutput>(json_slice) {
        Ok(v) => Ok(v),
        Err(e1) => {
            if let (Some(start), Some(end)) = (json_slice.find('{'), json_slice.rfind('}')) {
                if end > start {
                    return serde_json::from_str(&json_slice[start..=end]).map_err(|e2| {
                        TifError::Other(format!(
                            "failed to parse reviewer JSON output: {e1}; extract err: {e2}"
                        ))
                    });
                }
            }
            Err(TifError::Other(format!(
                "failed to parse reviewer JSON output: {e1}"
            )))
        }
    }
}

/// Apply a file-tree output under `candidate_root` (must already exist or be created).
/// Does not touch paths outside `candidate_root`.
pub fn apply_file_tree_to_dir(
    candidate_root: &Path,
    tree: &ReviewerFileTreeOutput,
    max_output_bytes: u64,
) -> Result<()> {
    std::fs::create_dir_all(candidate_root)?;
    let mut written: u64 = 0;
    for f in &tree.files {
        validate_relative_path(&f.path)?;
        written = written.saturating_add(f.content.len() as u64);
        if written > max_output_bytes {
            return Err(TifError::Other(format!(
                "reviewer output exceeds max_output_bytes ({max_output_bytes})"
            )));
        }
        let target = candidate_root.join(&f.path);
        // Ensure target stays under candidate_root.
        let canon_root = candidate_root;
        if !target.starts_with(canon_root) {
            return Err(TifError::Other(format!(
                "path escapes candidate root: {}",
                f.path
            )));
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, f.content.as_bytes())?;
    }
    for d in &tree.delete {
        validate_relative_path(d)?;
        let target = candidate_root.join(d);
        if !target.starts_with(candidate_root) {
            return Err(TifError::Other(format!("delete path escapes root: {d}")));
        }
        if target.is_file() {
            let _ = std::fs::remove_file(&target);
        } else if target.is_dir() {
            let _ = std::fs::remove_dir_all(&target);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ReviewerConfig};
    use crate::policy::{PolicyCompileRequest, PolicyCompiler};
    use tempfile::tempdir;

    fn policy() -> ContainmentPolicy {
        let cfg = Config::default();
        PolicyCompiler::new()
            .compile(&cfg, &PolicyCompileRequest::default())
            .unwrap()
    }

    #[test]
    fn egress_false_blocks_source() {
        let mut rev = ReviewerConfig::mock("r", 1);
        rev.allow_source_egress = false;
        let p = policy();
        let req = ContextBuildRequest {
            reviewer: &rev,
            policy: &p,
            task_category: TaskCategory::BugFix,
            task_text: Some("fix"),
            acceptance_criteria: None,
            original_metrics: None,
            source_or_diff: Some("fn secret() { let api_key = \"sk-abc\"; }"),
            verification_plan_summary: None,
            mode: ReviewerInvocationMode::Standard,
            failure_summary: None,
            prior_implementation_code: None,
        };
        assert!(build_reviewer_context(&req).is_err());
    }

    #[test]
    fn egress_true_includes_redacted_source() {
        let mut rev = ReviewerConfig::mock("r", 1);
        rev.allow_source_egress = true;
        let p = policy();
        let req = ContextBuildRequest {
            reviewer: &rev,
            policy: &p,
            task_category: TaskCategory::BugFix,
            task_text: Some("fix"),
            acceptance_criteria: None,
            original_metrics: None,
            source_or_diff: Some("password=hunter2"),
            verification_plan_summary: Some("cargo test"),
            mode: ReviewerInvocationMode::Standard,
            failure_summary: None,
            prior_implementation_code: None,
        };
        let pkg = build_reviewer_context(&req).unwrap();
        assert!(pkg.includes_source);
        assert!(!pkg.user_prompt.contains("hunter2"));
    }

    #[test]
    fn intensified_uses_stricter_wording() {
        let rev = ReviewerConfig::mock("r", 1);
        let p = policy();
        let req = ContextBuildRequest {
            reviewer: &rev,
            policy: &p,
            task_category: TaskCategory::BugFix,
            task_text: Some("shrink"),
            acceptance_criteria: Some("pass tests"),
            original_metrics: None,
            source_or_diff: None,
            verification_plan_summary: Some("cargo test"),
            mode: ReviewerInvocationMode::Intensified,
            failure_summary: Some("exceeded new_files limit"),
            prior_implementation_code: None,
        };
        let pkg = build_reviewer_context(&req).unwrap();
        assert!(pkg.system_prompt.contains("INTENSIFIED"));
        assert!(pkg.user_prompt.contains("INTENSIFIED"));
        assert!(pkg.user_prompt.contains("Structured failure summary"));
        assert!(!pkg.includes_source);
    }

    #[test]
    fn clean_room_forbids_prior_implementation_code() {
        let rev = ReviewerConfig::mock("r", 1);
        let p = policy();
        let req = ContextBuildRequest {
            reviewer: &rev,
            policy: &p,
            task_category: TaskCategory::FeatureAddition,
            task_text: Some("add flag"),
            acceptance_criteria: Some("works"),
            original_metrics: None,
            source_or_diff: None,
            verification_plan_summary: Some("cargo test"),
            mode: ReviewerInvocationMode::CleanRoom,
            failure_summary: Some("prior attempt exceeded containment"),
            prior_implementation_code: Some("fn bloated() { /* huge */ }"),
        };
        let err = build_reviewer_context(&req).unwrap_err();
        assert!(
            err.to_string().contains("prior_implementation_code")
                || err.to_string().contains("clean-room"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn clean_room_forbids_source_or_diff() {
        let mut rev = ReviewerConfig::mock("r", 1);
        rev.allow_source_egress = true;
        let p = policy();
        let req = ContextBuildRequest {
            reviewer: &rev,
            policy: &p,
            task_category: TaskCategory::FeatureAddition,
            task_text: Some("add flag"),
            acceptance_criteria: None,
            original_metrics: None,
            source_or_diff: Some("diff --git a/x b/x"),
            verification_plan_summary: None,
            mode: ReviewerInvocationMode::CleanRoom,
            failure_summary: Some("ooc"),
            prior_implementation_code: None,
        };
        assert!(build_reviewer_context(&req).is_err());
    }

    #[test]
    fn clean_room_package_has_no_prior_patch_content() {
        let rev = ReviewerConfig::mock("r2", 2);
        let p = policy();
        let req = ContextBuildRequest {
            reviewer: &rev,
            policy: &p,
            task_category: TaskCategory::FeatureAddition,
            task_text: Some("minimal flag"),
            acceptance_criteria: Some("cli --flag"),
            original_metrics: None,
            source_or_diff: None,
            verification_plan_summary: Some("cargo test"),
            mode: ReviewerInvocationMode::CleanRoom,
            failure_summary: Some("stage1 still out of containment; hard limit new_files"),
            prior_implementation_code: None,
        };
        let pkg = build_reviewer_context(&req).unwrap();
        assert!(!pkg.includes_source);
        assert!(pkg.system_prompt.contains("CLEAN-ROOM"));
        assert!(pkg.user_prompt.contains("failure summary") || pkg.user_prompt.contains("Failure"));
        assert!(!pkg.user_prompt.to_lowercase().contains("## source / diff"));
        assert_clean_room_package(&pkg.system_prompt, &pkg.user_prompt, pkg.includes_source)
            .unwrap();
    }

    #[test]
    fn rejects_parent_dir_paths() {
        assert!(validate_relative_path("../etc/passwd").is_err());
        assert!(validate_relative_path("src/lib.rs").is_ok());
    }

    #[test]
    fn apply_file_tree_writes_and_deletes() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("cand");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/old.rs"), b"old").unwrap();
        let tree = ReviewerFileTreeOutput {
            files: vec![ReviewerFileEntry {
                path: "src/new.rs".into(),
                content: "fn x() {}".into(),
            }],
            delete: vec!["src/old.rs".into()],
            notes: vec![],
        };
        apply_file_tree_to_dir(&root, &tree, 1_000_000).unwrap();
        assert!(root.join("src/new.rs").is_file());
        assert!(!root.join("src/old.rs").exists());
    }

    #[test]
    fn parse_fenced_json() {
        let text = "```json\n{\"files\":[{\"path\":\"a.rs\",\"content\":\"x\"}],\"delete\":[],\"notes\":[]}\n```";
        let t = parse_reviewer_file_tree(text).unwrap();
        assert_eq!(t.files.len(), 1);
    }
}

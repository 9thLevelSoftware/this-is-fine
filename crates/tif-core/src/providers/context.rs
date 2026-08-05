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
    pub source_or_diff: Option<&'a str>,
    pub verification_plan_summary: Option<&'a str>,
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

/// Build a redacted context package. Enforces egress policy.
pub fn build_reviewer_context(req: &ContextBuildRequest<'_>) -> Result<ReviewerContextPackage> {
    let want_source = req.source_or_diff.map(|s| !s.is_empty()).unwrap_or(false);
    if want_source && !req.reviewer.allow_source_egress {
        return Err(TifError::UnauthorizedReviewer(format!(
            "reviewer `{}` has allow_source_egress=false; cannot include source/diff in provider request",
            req.reviewer.id
        )));
    }

    let mut user = String::new();
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

    let mut includes_source = false;
    if want_source {
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
    let system_prompt = redact_secrets(SYSTEM_BASE);
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

    Ok(ReviewerContextPackage {
        system_prompt,
        user_prompt,
        includes_source,
        truncated,
    })
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
        };
        let pkg = build_reviewer_context(&req).unwrap();
        assert!(pkg.includes_source);
        assert!(!pkg.user_prompt.contains("hunter2"));
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

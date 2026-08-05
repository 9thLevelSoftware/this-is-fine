//! Firebreak engine: independent simplification with fail-safe application.
//!
//! Production path is fail-closed unless a verified isolated candidate is
//! authorized for apply. Simulated candidates exist only for ranking tests and
//! never touch the original workspace unless an explicit isolation apply is used.

use globset::{Glob, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::config::ReviewerConfig;
use crate::credentials::resolve_credential_opt;
use crate::error::{Result, TifError};
use crate::isolation::{apply_verified_candidate, open_isolation, IsolationSession, Isolator};
use crate::policy::ContainmentPolicy;
use crate::providers::{
    backend_for_provider, build_reviewer_context, BackendRegistry, ContextBuildRequest,
    ReviewerPatch, ReviewerTask,
};
use crate::reviewer::{ReviewerSelector, SelectedReviewer};
use crate::scoring::{
    select_smaller_verified, CorrectnessFloor, DiffMetrics, ScoreResult, SimplicityScorer,
};
use crate::verify::VerificationReport;

/// Request to run Firebreak on an oversized but correct candidate.
#[derive(Debug, Clone)]
pub struct FirebreakRequest {
    pub run_id: String,
    pub policy: ContainmentPolicy,
    pub original_metrics: DiffMetrics,
    pub original_score: ScoreResult,
    pub original_floor: CorrectnessFloor,
    /// Dev/test only: invent a smaller candidate without a real reviewer backend.
    /// Production orchestration must leave this false (fail-closed).
    pub simulate_success: bool,
    /// Only true when an isolated workspace was created and candidate re-verified.
    pub workspace_apply_authorized: bool,
}

/// Request to apply a re-verified candidate from an isolation session.
#[derive(Debug, Clone)]
pub struct IsolatedApplyRequest {
    pub run_id: String,
    pub policy: ContainmentPolicy,
    pub original_metrics: DiffMetrics,
    pub original_score: ScoreResult,
    pub original_floor: CorrectnessFloor,
    pub candidate_metrics: DiffMetrics,
    /// Correctness floor evaluated against the isolated candidate (re-verify).
    pub candidate_floor: CorrectnessFloor,
    /// When true, perform filesystem apply after ranking + policy checks.
    pub authorize_apply: bool,
    /// When true, skip ranking and treat as needing approval only.
    pub force_approval: bool,
}

/// Outcome of a Firebreak attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FirebreakOutcome {
    pub success: bool,
    /// True only when the original workspace was actually modified.
    pub applied: bool,
    /// Smaller candidate ranked better and cleared floor (may still need approval/isolation).
    pub candidate_ready: bool,
    pub requires_approval: bool,
    /// True when metrics/floor were simulated rather than produced by a real reviewer+verify.
    pub simulated: bool,
    pub reviewer_id: Option<String>,
    pub candidate_id: Option<String>,
    pub candidate_score: Option<ScoreResult>,
    pub candidate_metrics: Option<DiffMetrics>,
    pub message: String,
    /// When true, original workspace must remain untouched (always true unless applied).
    pub original_preserved: bool,
    /// Isolation session id when one was created for this Firebreak.
    #[serde(default)]
    pub isolation_session_id: Option<String>,
}

/// Request to invoke an authorized reviewer backend in isolation (Phase 1).
///
/// Does **not** apply to the user workspace. Candidate is untrusted until re-verified.
#[derive(Debug, Clone)]
pub struct BackendGenerateRequest {
    pub run_id: String,
    pub policy: ContainmentPolicy,
    pub source_root: PathBuf,
    pub state_dir: PathBuf,
    pub original_metrics: DiffMetrics,
    pub original_score: ScoreResult,
    pub original_floor: CorrectnessFloor,
    pub task_text: Option<String>,
    pub acceptance_criteria: Option<String>,
    /// Source/diff body; only sent if selected reviewer allows egress.
    pub source_or_diff: Option<String>,
    pub verification_plan_summary: Option<String>,
    pub max_output_bytes: u64,
}

/// Result of backend generation (always original-preserving).
#[derive(Debug, Clone)]
pub struct BackendGenerateResult {
    pub outcome: FirebreakOutcome,
    pub session: IsolationSession,
    pub patch: Option<ReviewerPatch>,
}

/// Automatic Firebreak using an authorized reviewer only.
pub struct FirebreakEngine {
    selector: ReviewerSelector,
    registry: BackendRegistry,
}

impl std::fmt::Debug for FirebreakEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FirebreakEngine")
            .field("selector", &self.selector)
            .field("backends", &self.registry.list_kinds())
            .finish()
    }
}

impl FirebreakEngine {
    pub fn new(selector: ReviewerSelector) -> Self {
        Self {
            selector,
            registry: BackendRegistry::new(),
        }
    }

    pub fn with_registry(selector: ReviewerSelector, registry: BackendRegistry) -> Self {
        Self { selector, registry }
    }

    pub fn selector(&self) -> &ReviewerSelector {
        &self.selector
    }

    pub fn registry(&self) -> &BackendRegistry {
        &self.registry
    }

    /// Look up full config for a selected reviewer id.
    pub fn reviewer_config(&self, id: &str) -> Option<&ReviewerConfig> {
        self.selector.pool().iter().find(|r| r.id == id)
    }

    /// Invoke authorized backend in an isolation workspace. Never applies to source.
    pub fn generate_with_backend(
        &self,
        req: BackendGenerateRequest,
    ) -> Result<BackendGenerateResult> {
        if !req.original_floor.passes() || req.original_score.disqualified {
            return Err(TifError::CorrectnessFloor(
                "original failed correctness floor; Firebreak backend will not run".into(),
            ));
        }

        let selected = self.selector.select(req.policy.task_category)?;
        let cfg = self.reviewer_config(&selected.id).cloned().ok_or_else(|| {
            TifError::UnauthorizedReviewer(format!(
                "selected reviewer `{}` missing from pool",
                selected.id
            ))
        })?;

        let backend = backend_for_provider(&self.registry, &cfg.provider)?;

        // Egress enforced in context builder when source is present.
        let context = build_reviewer_context(&ContextBuildRequest {
            reviewer: &cfg,
            policy: &req.policy,
            task_category: req.policy.task_category,
            task_text: req.task_text.as_deref(),
            acceptance_criteria: req.acceptance_criteria.as_deref(),
            original_metrics: Some(&req.original_metrics),
            source_or_diff: req.source_or_diff.as_deref(),
            verification_plan_summary: req.verification_plan_summary.as_deref(),
        })?;

        let credential = resolve_credential_opt(cfg.credential_ref.as_deref())?;

        let session_id = format!("fb-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let (isolator, mut session) =
            open_isolation(&req.source_root, &req.state_dir, &session_id)?;

        let task = ReviewerTask {
            run_id: req.run_id.clone(),
            reviewer: cfg.clone(),
            task_category: req.policy.task_category,
            task_text: req.task_text.clone(),
            acceptance_criteria: req.acceptance_criteria.clone(),
            policy: req.policy.clone(),
            isolation_root: session.path.clone(),
            context_artifact: None,
            context,
            max_output_bytes: if req.max_output_bytes == 0 {
                8_000_000
            } else {
                req.max_output_bytes
            },
            credential,
        };

        let patch = match backend.complete(&task) {
            Ok(p) => p,
            Err(e) => {
                let _ = isolator.destroy(&session);
                return Ok(BackendGenerateResult {
                    outcome: FirebreakOutcome {
                        success: false,
                        applied: false,
                        candidate_ready: false,
                        requires_approval: false,
                        simulated: false,
                        reviewer_id: Some(selected.id),
                        candidate_id: None,
                        candidate_score: None,
                        candidate_metrics: None,
                        message: format!("reviewer backend failed; original retained: {e}"),
                        original_preserved: true,
                        isolation_session_id: Some(session.id.clone()),
                    },
                    session,
                    patch: None,
                });
            }
        };

        // Phase 1: candidate generated but not re-verified → not ready to apply.
        let outcome = FirebreakOutcome {
            success: true,
            applied: false,
            candidate_ready: false,
            requires_approval: false,
            simulated: false,
            reviewer_id: Some(patch.reviewer_id.clone()),
            candidate_id: Some(format!("fb-{}", patch.reviewer_id)),
            candidate_score: None,
            candidate_metrics: None,
            message: format!(
                "reviewer `{}` produced candidate at {}; re-verify before apply (not applied)",
                patch.reviewer_id,
                patch.candidate_root.display()
            ),
            original_preserved: true,
            isolation_session_id: Some(session.id.clone()),
        };

        session.candidate_path = Some(patch.candidate_root.clone());
        drop(isolator); // session retained for caller re-verify/apply
        Ok(BackendGenerateResult {
            outcome,
            session,
            patch: Some(patch),
        })
    }

    /// Produce a smaller correct candidate in isolation. Never replaces original on failure.
    ///
    /// Without `simulate_success` and without calling `generate_with_backend`, this is
    /// fail-closed: `applied` stays false and the original workspace is preserved.
    pub fn simplify(&self, req: FirebreakRequest) -> Result<FirebreakOutcome> {
        if !req.original_floor.passes() || req.original_score.disqualified {
            return Ok(FirebreakOutcome {
                success: false,
                applied: false,
                candidate_ready: false,
                requires_approval: false,
                simulated: false,
                reviewer_id: None,
                candidate_id: None,
                candidate_score: None,
                candidate_metrics: None,
                message: "original failed correctness floor; Firebreak will not run".into(),
                original_preserved: true,
                isolation_session_id: None,
            });
        }

        let reviewer = match self.selector.select(req.policy.task_category) {
            Ok(r) => r,
            Err(e) => {
                return Ok(FirebreakOutcome {
                    success: false,
                    applied: false,
                    candidate_ready: false,
                    requires_approval: false,
                    simulated: false,
                    reviewer_id: None,
                    candidate_id: None,
                    candidate_score: None,
                    candidate_metrics: None,
                    message: format!("Firebreak unavailable: {e}"),
                    original_preserved: true,
                    isolation_session_id: None,
                });
            }
        };

        // Production path: no real reviewer invocation yet — fail closed unless test simulation.
        if !req.simulate_success {
            return Ok(FirebreakOutcome {
                success: false,
                applied: false,
                candidate_ready: false,
                requires_approval: false,
                simulated: false,
                reviewer_id: Some(reviewer.id),
                candidate_id: None,
                candidate_score: None,
                candidate_metrics: None,
                message: "Firebreak reviewer backend not invoked; use generate_with_backend / tif firebreak --invoke-backend (fail-closed; original retained)".into(),
                original_preserved: true,
                isolation_session_id: None,
            });
        }

        // Simulated path (tests/dev only): invent reduced metrics; never claim real verify.
        let candidate = self.produce_simulated_candidate(&req, &reviewer);

        let scorer = SimplicityScorer::new(req.policy.weights.clone(), req.policy.limits.clone());
        let candidate_score = scorer.score(&candidate.metrics, &candidate.floor);

        if candidate_score.disqualified {
            return Ok(FirebreakOutcome {
                success: false,
                applied: false,
                candidate_ready: false,
                requires_approval: false,
                simulated: true,
                reviewer_id: Some(reviewer.id),
                candidate_id: Some(candidate.id),
                candidate_score: Some(candidate_score),
                candidate_metrics: Some(candidate.metrics),
                message: "Firebreak candidate failed correctness floor; original retained".into(),
                original_preserved: true,
                isolation_session_id: None,
            });
        }

        // A candidate that is smaller by score but still violates hard limits is not
        // "ready" — it is still out of control and must not be queued for approval/apply.
        if !candidate_score.within_containment {
            return Ok(FirebreakOutcome {
                success: false,
                applied: false,
                candidate_ready: false,
                requires_approval: false,
                simulated: true,
                reviewer_id: Some(reviewer.id),
                candidate_id: Some(candidate.id),
                candidate_score: Some(candidate_score),
                candidate_metrics: Some(candidate.metrics),
                message: "Firebreak candidate still violates hard limits; original retained".into(),
                original_preserved: true,
                isolation_session_id: None,
            });
        }

        let winner = select_smaller_verified(
            "original",
            &req.original_score,
            &candidate.id,
            &candidate_score,
        )?;

        if winner == "original" {
            return Ok(FirebreakOutcome {
                success: true,
                applied: false,
                candidate_ready: false,
                requires_approval: false,
                simulated: true,
                reviewer_id: Some(reviewer.id),
                candidate_id: Some(candidate.id),
                candidate_score: Some(candidate_score),
                candidate_metrics: Some(candidate.metrics),
                message: "Firebreak candidate not materially smaller; original retained".into(),
                original_preserved: true,
                isolation_session_id: None,
            });
        }

        let requires_approval = req.policy.require_firebreak_approval
            || touches_sensitive(&req.policy, &candidate.metrics);

        // Simulated path never mutates the filesystem. Only `apply_isolated_candidate`
        // after a successful isolator call may set `applied: true`.
        let _ = (
            req.workspace_apply_authorized,
            candidate.simulated_unverified,
        );

        Ok(FirebreakOutcome {
            success: true,
            applied: false,
            candidate_ready: true,
            requires_approval,
            simulated: true,
            reviewer_id: Some(reviewer.id),
            candidate_id: Some(candidate.id),
            candidate_score: Some(candidate_score),
            candidate_metrics: Some(candidate.metrics),
            message: if requires_approval {
                "simulated smaller candidate ready; approval required (not applied)".into()
            } else {
                "simulated smaller candidate ready; not applied (use isolation apply path)".into()
            },
            original_preserved: true,
            isolation_session_id: None,
        })
    }

    /// Rank a re-verified isolated candidate and optionally apply it to the source.
    ///
    /// Fail-safe: the original workspace is only modified when
    /// `authorize_apply` is true, the candidate clears the correctness floor,
    /// is within containment, is materially smaller, and does not require approval.
    pub fn apply_isolated_candidate(
        &self,
        req: IsolatedApplyRequest,
        isolator: &dyn Isolator,
        session: &mut IsolationSession,
    ) -> Result<FirebreakOutcome> {
        if req.original_floor.passes() && !req.candidate_floor.passes() {
            // Soft outcome (not Err): callers still attach isolation session for audit.
            // fail_safe_guard documents the invariant for other call sites that prefer Err.
            debug_assert!(
                fail_safe_guard(true, false).is_err(),
                "fail_safe_guard must reject unverified replace"
            );
            return Ok(FirebreakOutcome {
                success: false,
                applied: false,
                candidate_ready: false,
                requires_approval: false,
                simulated: false,
                reviewer_id: None,
                candidate_id: None,
                candidate_score: None,
                candidate_metrics: Some(req.candidate_metrics.clone()),
                message:
                    "fail-safe: unverified Firebreak must not replace known-good implementation"
                        .into(),
                original_preserved: true,
                isolation_session_id: Some(session.id.clone()),
            });
        }

        if !req.original_floor.passes() || req.original_score.disqualified {
            return Ok(FirebreakOutcome {
                success: false,
                applied: false,
                candidate_ready: false,
                requires_approval: false,
                simulated: false,
                reviewer_id: None,
                candidate_id: None,
                candidate_score: None,
                candidate_metrics: Some(req.candidate_metrics),
                message: "original failed correctness floor; isolated apply refused".into(),
                original_preserved: true,
                isolation_session_id: Some(session.id.clone()),
            });
        }

        let reviewer = match self.selector.select(req.policy.task_category) {
            Ok(r) => r,
            Err(e) => {
                return Ok(FirebreakOutcome {
                    success: false,
                    applied: false,
                    candidate_ready: false,
                    requires_approval: false,
                    simulated: false,
                    reviewer_id: None,
                    candidate_id: None,
                    candidate_score: None,
                    candidate_metrics: Some(req.candidate_metrics),
                    message: format!("Firebreak unavailable: {e}"),
                    original_preserved: true,
                    isolation_session_id: Some(session.id.clone()),
                });
            }
        };

        let scorer = SimplicityScorer::new(req.policy.weights.clone(), req.policy.limits.clone());
        let candidate_score = scorer.score(&req.candidate_metrics, &req.candidate_floor);
        let candidate_id = format!("fb-isolated-{}", reviewer.id);

        if candidate_score.disqualified {
            return Ok(FirebreakOutcome {
                success: false,
                applied: false,
                candidate_ready: false,
                requires_approval: false,
                simulated: false,
                reviewer_id: Some(reviewer.id),
                candidate_id: Some(candidate_id),
                candidate_score: Some(candidate_score),
                candidate_metrics: Some(req.candidate_metrics),
                message: "isolated candidate failed correctness floor; original retained".into(),
                original_preserved: true,
                isolation_session_id: Some(session.id.clone()),
            });
        }

        if !candidate_score.within_containment {
            return Ok(FirebreakOutcome {
                success: false,
                applied: false,
                candidate_ready: false,
                requires_approval: false,
                simulated: false,
                reviewer_id: Some(reviewer.id),
                candidate_id: Some(candidate_id),
                candidate_score: Some(candidate_score),
                candidate_metrics: Some(req.candidate_metrics),
                message: "isolated candidate still violates hard limits; original retained".into(),
                original_preserved: true,
                isolation_session_id: Some(session.id.clone()),
            });
        }

        let winner = select_smaller_verified(
            "original",
            &req.original_score,
            &candidate_id,
            &candidate_score,
        )?;

        if winner == "original" {
            return Ok(FirebreakOutcome {
                success: true,
                applied: false,
                candidate_ready: false,
                requires_approval: false,
                simulated: false,
                reviewer_id: Some(reviewer.id),
                candidate_id: Some(candidate_id),
                candidate_score: Some(candidate_score),
                candidate_metrics: Some(req.candidate_metrics),
                message: "isolated candidate not materially smaller; original retained".into(),
                original_preserved: true,
                isolation_session_id: Some(session.id.clone()),
            });
        }

        let requires_approval = req.force_approval
            || req.policy.require_firebreak_approval
            || touches_sensitive(&req.policy, &req.candidate_metrics);

        if requires_approval {
            return Ok(FirebreakOutcome {
                success: true,
                applied: false,
                candidate_ready: true,
                requires_approval: true,
                simulated: false,
                reviewer_id: Some(reviewer.id),
                candidate_id: Some(candidate_id),
                candidate_score: Some(candidate_score),
                candidate_metrics: Some(req.candidate_metrics),
                message: "isolated smaller candidate ready; approval required (not applied)".into(),
                original_preserved: true,
                isolation_session_id: Some(session.id.clone()),
            });
        }

        if !req.authorize_apply {
            return Ok(FirebreakOutcome {
                success: true,
                applied: false,
                candidate_ready: true,
                requires_approval: false,
                simulated: false,
                reviewer_id: Some(reviewer.id),
                candidate_id: Some(candidate_id),
                candidate_score: Some(candidate_score),
                candidate_metrics: Some(req.candidate_metrics),
                message: "isolated smaller candidate ready; apply not authorized".into(),
                original_preserved: true,
                isolation_session_id: Some(session.id.clone()),
            });
        }

        // Real filesystem apply with baseline preservation for rollback.
        apply_verified_candidate(isolator, session)?;

        Ok(FirebreakOutcome {
            success: true,
            applied: true,
            candidate_ready: true,
            requires_approval: false,
            simulated: false,
            reviewer_id: Some(reviewer.id),
            candidate_id: Some(candidate_id),
            candidate_score: Some(candidate_score),
            candidate_metrics: Some(req.candidate_metrics),
            message: "smaller verified candidate applied; original retained for rollback".into(),
            original_preserved: false,
            isolation_session_id: Some(session.id.clone()),
        })
    }

    /// Open isolation for a Firebreak run (source left untouched).
    pub fn open_workspace(
        source_root: &Path,
        state_dir: &Path,
        run_id: &str,
    ) -> Result<(Box<dyn Isolator>, IsolationSession)> {
        let session_id = format!("fb-{}", short_id(run_id));
        open_isolation(source_root, state_dir, &session_id)
    }

    /// Copy a candidate tree into an existing isolation session (for external reviewers).
    pub fn stage_candidate_tree(session: &IsolationSession, candidate_root: &Path) -> Result<()> {
        if !candidate_root.exists() {
            return Err(TifError::Isolation(format!(
                "candidate tree missing: {}",
                candidate_root.display()
            )));
        }
        copy_tree_into(candidate_root, &session.path)
    }

    fn produce_simulated_candidate(
        &self,
        req: &FirebreakRequest,
        reviewer: &SelectedReviewer,
    ) -> IsolatedCandidate {
        let mut metrics = req.original_metrics.clone();
        // Simulate reduction: drop deps, cut files/lines roughly in half.
        metrics.runtime_dependencies_added = 0;
        metrics.files_added = metrics.files_added.saturating_div(2);
        metrics.lines_added = metrics.lines_added.saturating_div(2).max(1);
        metrics.abstractions_added = metrics.abstractions_added.saturating_sub(1);
        metrics.public_interfaces_added = metrics.public_interfaces_added.saturating_div(2);
        metrics.unrelated_changes = 0;

        // Do not reuse original floor as proof of candidate verification.
        // Simulated candidates are marked as needing real re-verify for apply.
        let mut floor = CorrectnessFloor::all_pass();
        floor
            .notes
            .push("simulated candidate: verification not re-run against isolated workspace".into());

        IsolatedCandidate {
            id: format!("fb-{}", reviewer.id),
            metrics,
            // Scoring uses a passing floor for ranking experiments only;
            // apply is still blocked via simulated flag / workspace_apply_authorized.
            floor,
            simulated_unverified: true,
        }
    }
}

struct IsolatedCandidate {
    id: String,
    metrics: DiffMetrics,
    floor: CorrectnessFloor,
    simulated_unverified: bool,
}

fn short_id(run_id: &str) -> String {
    let clean: String = run_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(12)
        .collect();
    if clean.is_empty() {
        uuid::Uuid::new_v4().to_string()[..8].to_string()
    } else {
        clean
    }
}

fn copy_tree_into(src: &Path, dst: &Path) -> Result<()> {
    use std::fs;
    if src.is_file() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dst)?;
        return Ok(());
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if matches!(
            name_str.as_ref(),
            ".git" | ".this-is-fine" | "target" | "node_modules"
        ) {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_dir() {
            copy_tree_into(&from, &to)?;
        } else {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Sensitive paths and/or policy flags require approval independently.
pub fn touches_sensitive(policy: &ContainmentPolicy, metrics: &DiffMetrics) -> bool {
    if !policy.sensitive_paths.is_empty() {
        if metrics.changed_paths.is_empty() {
            // Paths configured but candidate path list unknown — fail closed to approval.
            return true;
        }
        if paths_match_sensitive(&policy.sensitive_paths, &metrics.changed_paths) {
            return true;
        }
    }
    false
}

fn paths_match_sensitive(patterns: &[String], paths: &[String]) -> bool {
    let mut builder = GlobSetBuilder::new();
    let mut any = false;
    for p in patterns {
        if let Ok(g) = Glob::new(p) {
            builder.add(g);
            any = true;
        } else {
            // Fallback: substring / prefix match for simple patterns.
            for path in paths {
                let norm = path.replace('\\', "/");
                let pat = p.replace('\\', "/").trim_end_matches("/**").to_string();
                if norm == *p || norm.starts_with(&format!("{pat}/")) || norm.contains(p) {
                    return true;
                }
            }
        }
    }
    if !any {
        return false;
    }
    let Ok(set) = builder.build() else {
        return false;
    };
    paths.iter().any(|path| {
        let norm = path.replace('\\', "/");
        set.is_match(&norm)
    })
}

/// Five-Alarm staged recovery outline (scaffolded).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiveAlarmPlan {
    pub steps: Vec<String>,
}

impl FiveAlarmPlan {
    pub fn staged_recovery() -> Self {
        Self {
            steps: vec![
                "Run intensified Firebreak on current verified implementation".into(),
                "If still out of containment, preserve as candidate".into(),
                "Select another authorized model".into(),
                "Clean-room model gets original state, task, criteria, policy, verification plan, failure summary — not previous implementation code".into(),
                "Verify all candidates against the same correctness floor".into(),
                "Apply the smallest verified candidate".into(),
                "Preserve rejected candidates during rollback period".into(),
            ],
        }
    }
}

/// Ensure no unverified candidate replaces a known-good implementation.
pub fn fail_safe_guard(original_verified: bool, candidate_verified: bool) -> Result<()> {
    if original_verified && !candidate_verified {
        return Err(TifError::CorrectnessFloor(
            "fail-safe: unverified Firebreak must not replace known-good implementation".into(),
        ));
    }
    Ok(())
}

/// Build candidate metrics from a verification report + path-level diff stats.
pub fn candidate_floor_from_verification(report: &VerificationReport) -> CorrectnessFloor {
    let mut floor = CorrectnessFloor::all_pass();
    if !report.satisfies_correctness_verification() {
        floor.verification_passed = false;
        if report.incomplete_plan {
            floor
                .notes
                .push("isolated re-verify: verification plan incomplete".into());
        }
        if !report.all_required_passed {
            floor
                .notes
                .push("isolated re-verify: required checks failed".into());
        }
    }
    floor
}

/// Paths used when staging a candidate for isolated apply (library helper).
pub fn default_isolation_state_dir(source_root: &Path) -> PathBuf {
    source_root.join(".this-is-fine")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ReviewerConfig, SimplicityLimits};
    use crate::isolation::SnapshotIsolator;
    use crate::policy::{PolicyCompileRequest, PolicyCompiler};
    use crate::scoring::SimplicityScorer;
    use std::fs;
    use tempfile::tempdir;

    fn policy_with_limits() -> ContainmentPolicy {
        let mut cfg = Config::default();
        cfg.simplicity.limits = SimplicityLimits {
            new_runtime_dependencies: Some(0),
            ..Default::default()
        };
        PolicyCompiler::new()
            .compile(&cfg, &PolicyCompileRequest::default())
            .unwrap()
    }

    fn authorized_engine() -> FirebreakEngine {
        FirebreakEngine::new(ReviewerSelector::new(vec![ReviewerConfig::mock("r1", 1)]))
    }

    fn base_req(
        policy: ContainmentPolicy,
        metrics: DiffMetrics,
        score: ScoreResult,
    ) -> FirebreakRequest {
        FirebreakRequest {
            run_id: "x".into(),
            policy,
            original_metrics: metrics,
            original_score: score,
            original_floor: CorrectnessFloor::all_pass(),
            simulate_success: false,
            workspace_apply_authorized: false,
        }
    }

    #[test]
    fn production_fail_closed_without_simulation() {
        let policy = policy_with_limits();
        let metrics = DiffMetrics {
            runtime_dependencies_added: 1,
            lines_added: 100,
            ..Default::default()
        };
        let scorer = SimplicityScorer::new(policy.weights.clone(), policy.limits.clone());
        let score = scorer.score(&metrics, &CorrectnessFloor::all_pass());
        let outcome = authorized_engine()
            .simplify(base_req(policy, metrics, score))
            .unwrap();
        assert!(!outcome.applied);
        assert!(!outcome.candidate_ready);
        assert!(outcome.original_preserved);
    }

    #[test]
    fn failed_candidate_preserves_original() {
        // simulate_success false is fail-closed; original preserved.
        let policy = policy_with_limits();
        let metrics = DiffMetrics {
            runtime_dependencies_added: 1,
            lines_added: 100,
            ..Default::default()
        };
        let scorer = SimplicityScorer::new(policy.weights.clone(), policy.limits.clone());
        let score = scorer.score(&metrics, &CorrectnessFloor::all_pass());
        let outcome = authorized_engine()
            .simplify(base_req(policy, metrics, score))
            .unwrap();
        assert!(!outcome.applied);
        assert!(outcome.original_preserved);
    }

    #[test]
    fn simulated_smaller_candidate_ready_not_applied() {
        let policy = policy_with_limits();
        let metrics = DiffMetrics {
            runtime_dependencies_added: 1,
            lines_added: 100,
            files_added: 4,
            ..Default::default()
        };
        let scorer = SimplicityScorer::new(policy.weights.clone(), policy.limits.clone());
        let original_score = scorer.score(&metrics, &CorrectnessFloor::all_pass());
        let original_numeric = original_score.score;

        let mut req = base_req(policy, metrics, original_score);
        req.simulate_success = true;

        let outcome = authorized_engine().simplify(req).unwrap();
        assert!(outcome.success);
        assert!(outcome.candidate_ready);
        assert!(!outcome.applied, "must not apply without isolation backend");
        assert!(outcome.simulated);
        assert!(outcome.original_preserved);
        let cand = outcome.candidate_score.as_ref().unwrap();
        assert!(cand.score < original_numeric);
        assert!(!cand.disqualified);
    }

    #[test]
    fn sensitive_paths_require_approval_independently() {
        let mut policy = policy_with_limits();
        policy.require_firebreak_approval = false;
        policy.sensitive_paths = vec!["src/auth/**".into()];
        let metrics = DiffMetrics {
            runtime_dependencies_added: 1,
            lines_added: 100,
            changed_paths: vec!["src/auth/login.rs".into()],
            ..Default::default()
        };
        assert!(touches_sensitive(&policy, &metrics));

        let empty_paths = DiffMetrics {
            runtime_dependencies_added: 1,
            ..Default::default()
        };
        // Unknown path list with sensitive policy → require approval (fail closed).
        assert!(touches_sensitive(&policy, &empty_paths));

        let safe = DiffMetrics {
            changed_paths: vec!["src/lib.rs".into()],
            ..Default::default()
        };
        assert!(!touches_sensitive(&policy, &safe));
    }

    #[test]
    fn fail_safe_guard_blocks_unverified_replace() {
        assert!(fail_safe_guard(true, false).is_err());
        assert!(fail_safe_guard(true, true).is_ok());
    }

    #[test]
    fn generate_with_backend_mock_preserves_source() {
        use crate::config::RepoPaths;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let source = dir.path().join("src_repo");
        std::fs::create_dir_all(source.join("src")).unwrap();
        std::fs::write(source.join("src/lib.rs"), b"pub fn f() {}\n").unwrap();
        let marker = source.join("KEEP.txt");
        std::fs::write(&marker, b"original").unwrap();

        let paths = RepoPaths::for_root(&source);
        crate::config::ensure_state_dirs(&paths).unwrap();

        let engine = authorized_engine();
        let policy = policy_with_limits();
        let metrics = DiffMetrics {
            runtime_dependencies_added: 1,
            lines_added: 50,
            files_added: 2,
            ..Default::default()
        };
        let scorer = SimplicityScorer::new(policy.weights.clone(), policy.limits.clone());
        let score = scorer.score(&metrics, &CorrectnessFloor::all_pass());

        let result = engine
            .generate_with_backend(BackendGenerateRequest {
                run_id: "gen1".into(),
                policy,
                source_root: source.clone(),
                state_dir: paths.state_dir.clone(),
                original_metrics: metrics,
                original_score: score,
                original_floor: CorrectnessFloor::all_pass(),
                task_text: Some("shrink".into()),
                acceptance_criteria: None,
                source_or_diff: None,
                verification_plan_summary: None,
                max_output_bytes: 1_000_000,
            })
            .unwrap();

        assert!(result.outcome.original_preserved);
        assert!(!result.outcome.applied);
        assert!(!result.outcome.candidate_ready); // not re-verified yet
        assert!(result.patch.is_some());
        // Source file untouched
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "original");
        let cand = result.patch.unwrap().candidate_root;
        assert!(cand.join("src/lib.rs").is_file());
    }

    #[test]
    fn smaller_but_still_out_of_containment_is_not_ready() {
        // Limit forbids any new files; simulation halves files but leaves some.
        let mut policy = policy_with_limits();
        policy.limits.new_files = Some(0);
        let metrics = DiffMetrics {
            runtime_dependencies_added: 1,
            lines_added: 100,
            files_added: 4,
            ..Default::default()
        };
        let scorer = SimplicityScorer::new(policy.weights.clone(), policy.limits.clone());
        let original_score = scorer.score(&metrics, &CorrectnessFloor::all_pass());
        assert!(!original_score.within_containment);

        let mut req = base_req(policy, metrics, original_score);
        req.simulate_success = true;

        let outcome = authorized_engine().simplify(req).unwrap();
        assert!(!outcome.success);
        assert!(!outcome.candidate_ready);
        assert!(!outcome.applied);
        assert!(outcome.original_preserved);
        assert!(outcome.message.contains("still violates hard limits"));
        let cand = outcome.candidate_score.as_ref().unwrap();
        assert!(!cand.within_containment);
        assert!(cand.score.is_finite());
    }

    #[test]
    fn isolated_apply_and_rollback_preserves_fail_safe() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("src_repo");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::write(source.join("src/lib.rs"), "pub fn big() { /* lots */ }\n").unwrap();
        fs::write(source.join("extra_dep.txt"), "bloated\n").unwrap();

        let state = dir.path().join("state");
        let snaps = state.join("snapshots");
        let iso = SnapshotIsolator::new(snaps);
        let mut session = iso.create(&source, "fb-test").unwrap();

        // Plant smaller candidate in isolation only.
        fs::write(session.path.join("src/lib.rs"), "pub fn big() {}\n").unwrap();
        // Remove bloat from candidate.
        let _ = fs::remove_file(session.path.join("extra_dep.txt"));

        let policy = policy_with_limits();
        let original_metrics = DiffMetrics {
            runtime_dependencies_added: 1,
            lines_added: 50,
            files_added: 1,
            changed_paths: vec!["src/lib.rs".into(), "extra_dep.txt".into()],
            ..Default::default()
        };
        let scorer = SimplicityScorer::new(policy.weights.clone(), policy.limits.clone());
        let original_score = scorer.score(&original_metrics, &CorrectnessFloor::all_pass());
        let candidate_metrics = DiffMetrics {
            runtime_dependencies_added: 0,
            lines_added: 5,
            files_added: 0,
            files_changed: 1,
            changed_paths: vec!["src/lib.rs".into()],
            ..Default::default()
        };

        // Unverified candidate must not apply.
        let bad = authorized_engine().apply_isolated_candidate(
            IsolatedApplyRequest {
                run_id: "r1".into(),
                policy: policy.clone(),
                original_metrics: original_metrics.clone(),
                original_score: original_score.clone(),
                original_floor: CorrectnessFloor::all_pass(),
                candidate_metrics: candidate_metrics.clone(),
                candidate_floor: {
                    let mut f = CorrectnessFloor::all_pass();
                    f.verification_passed = false;
                    f
                },
                authorize_apply: true,
                force_approval: false,
            },
            &iso,
            &mut session,
        );
        assert!(bad.is_err() || bad.as_ref().is_ok_and(|o| !o.applied));
        assert_eq!(
            fs::read_to_string(source.join("src/lib.rs")).unwrap(),
            "pub fn big() { /* lots */ }\n"
        );

        // Verified smaller candidate applies.
        let outcome = authorized_engine()
            .apply_isolated_candidate(
                IsolatedApplyRequest {
                    run_id: "r1".into(),
                    policy,
                    original_metrics,
                    original_score,
                    original_floor: CorrectnessFloor::all_pass(),
                    candidate_metrics,
                    candidate_floor: CorrectnessFloor::all_pass(),
                    authorize_apply: true,
                    force_approval: false,
                },
                &iso,
                &mut session,
            )
            .unwrap();
        assert!(outcome.applied);
        assert!(!outcome.original_preserved);
        assert_eq!(
            fs::read_to_string(source.join("src/lib.rs")).unwrap(),
            "pub fn big() {}\n"
        );
        // Apply must prune files deleted in the candidate (not overlay-only).
        assert!(
            !source.join("extra_dep.txt").exists(),
            "bloat file must be removed when candidate deletes it"
        );

        // Rollback restores original.
        iso.restore_source(&mut session).unwrap();
        assert_eq!(
            fs::read_to_string(source.join("src/lib.rs")).unwrap(),
            "pub fn big() { /* lots */ }\n"
        );
        assert!(source.join("extra_dep.txt").exists());
    }

    #[test]
    fn identical_absolute_metrics_do_not_apply() {
        // Same kind of metrics on both sides (absolute tree weight shape).
        // Equal scores → select_smaller_verified keeps original.
        use crate::diff::metrics_from_tree_absolute;

        let dir = tempdir().unwrap();
        let source = dir.path().join("src");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("a.txt"), "line\n".repeat(20)).unwrap();
        let snaps = dir.path().join("snaps");
        let iso = SnapshotIsolator::new(snaps);
        let mut session = iso.create(&source, "ident").unwrap();
        // Candidate identical to source.
        fs::write(session.path.join("a.txt"), "line\n".repeat(20)).unwrap();

        let policy = policy_with_limits();
        let orig = metrics_from_tree_absolute(&source).unwrap();
        let cand = metrics_from_tree_absolute(&session.path).unwrap();
        assert_eq!(orig.files_added, cand.files_added);
        assert_eq!(orig.lines_added, cand.lines_added);
        let scorer = SimplicityScorer::new(policy.weights.clone(), policy.limits.clone());
        let orig_score = scorer.score(&orig, &CorrectnessFloor::all_pass());

        let outcome = authorized_engine()
            .apply_isolated_candidate(
                IsolatedApplyRequest {
                    run_id: "ident".into(),
                    policy,
                    original_metrics: orig,
                    original_score: orig_score,
                    original_floor: CorrectnessFloor::all_pass(),
                    candidate_metrics: cand,
                    candidate_floor: CorrectnessFloor::all_pass(),
                    authorize_apply: true,
                    force_approval: false,
                },
                &iso,
                &mut session,
            )
            .unwrap();
        assert!(
            !outcome.applied,
            "identical candidate must not replace original"
        );
        assert!(outcome.original_preserved);
        assert!(!outcome.candidate_ready || outcome.message.contains("not materially smaller"));
    }

    #[test]
    fn larger_absolute_candidate_does_not_apply() {
        use crate::diff::metrics_from_tree_absolute;

        let dir = tempdir().unwrap();
        let source = dir.path().join("src");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("a.txt"), "small\n").unwrap();
        let snaps = dir.path().join("snaps");
        let iso = SnapshotIsolator::new(snaps);
        let mut session = iso.create(&source, "larger").unwrap();
        // Candidate is larger / more fuel.
        fs::write(session.path.join("a.txt"), "small\n").unwrap();
        fs::write(session.path.join("bloat1.txt"), "x\n".repeat(40)).unwrap();
        fs::write(session.path.join("bloat2.txt"), "y\n".repeat(40)).unwrap();

        let policy = policy_with_limits();
        let orig = metrics_from_tree_absolute(&source).unwrap();
        let cand = metrics_from_tree_absolute(&session.path).unwrap();
        assert!(cand.files_added > orig.files_added || cand.lines_added > orig.lines_added);
        let scorer = SimplicityScorer::new(policy.weights.clone(), policy.limits.clone());
        let orig_score = scorer.score(&orig, &CorrectnessFloor::all_pass());
        let cand_score = scorer.score(&cand, &CorrectnessFloor::all_pass());
        assert!(cand_score.score >= orig_score.score);

        let outcome = authorized_engine()
            .apply_isolated_candidate(
                IsolatedApplyRequest {
                    run_id: "larger".into(),
                    policy,
                    original_metrics: orig,
                    original_score: orig_score,
                    original_floor: CorrectnessFloor::all_pass(),
                    candidate_metrics: cand,
                    candidate_floor: CorrectnessFloor::all_pass(),
                    authorize_apply: true,
                    force_approval: false,
                },
                &iso,
                &mut session,
            )
            .unwrap();
        assert!(!outcome.applied, "larger candidate must not apply");
        assert!(outcome.original_preserved);
        assert_eq!(fs::read_to_string(source.join("a.txt")).unwrap(), "small\n");
        assert!(!source.join("bloat1.txt").exists());
    }

    #[test]
    fn tree_diff_delta_must_not_be_used_as_identical_candidate_score() {
        // Guardrail: source↔candidate delta for identical trees is ~0 and would
        // incorrectly beat an out-of-control original if used for ranking.
        use crate::diff::{metrics_from_tree_absolute, metrics_from_tree_diff};

        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("big.rs"), "fn x() {}\n".repeat(30)).unwrap();
        fs::write(b.join("big.rs"), "fn x() {}\n".repeat(30)).unwrap();

        let delta = metrics_from_tree_diff(&a, &b).unwrap();
        let abs = metrics_from_tree_absolute(&b).unwrap();
        let policy = policy_with_limits();
        let scorer = SimplicityScorer::new(policy.weights.clone(), policy.limits.clone());
        // Bloated original (delta-style agent metrics).
        let original_metrics = DiffMetrics {
            runtime_dependencies_added: 1,
            lines_added: 100,
            files_added: 4,
            ..Default::default()
        };
        let original_score = scorer.score(&original_metrics, &CorrectnessFloor::all_pass());
        let delta_score = scorer.score(&delta, &CorrectnessFloor::all_pass());
        let abs_score = scorer.score(&abs, &CorrectnessFloor::all_pass());
        // Delta would incorrectly look smaller; absolute weight does not.
        assert!(delta_score.score < original_score.score);
        assert!(abs_score.score > 0.0);
        // Ranking with absolute vs original absolute of same tree should not apply.
        let orig_abs = metrics_from_tree_absolute(&a).unwrap();
        let orig_abs_score = scorer.score(&orig_abs, &CorrectnessFloor::all_pass());
        assert!(
            (abs_score.score - orig_abs_score.score).abs() < f64::EPSILON
                || abs_score.score >= orig_abs_score.score
        );
    }
}

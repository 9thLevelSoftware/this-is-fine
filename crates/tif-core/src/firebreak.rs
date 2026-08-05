//! Firebreak engine: independent simplification with fail-safe application.
//!
//! Production path is fail-closed unless a verified isolated candidate is
//! authorized for apply. Simulated candidates exist only for ranking tests and
//! never touch the original workspace unless an explicit isolation apply is used.

use globset::{Glob, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::ReviewerConfig;
use crate::credentials::resolve_credential_opt;
use crate::error::{Result, TifError};
use crate::fire_level::FireLevel;
use crate::isolation::{apply_verified_candidate, open_isolation, IsolationSession, Isolator};
use crate::policy::ContainmentPolicy;
use crate::providers::{
    backend_for_provider, build_reviewer_context, BackendRegistry, ContextBuildRequest,
    ReviewerInvocationMode, ReviewerPatch, ReviewerTask,
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
    /// When true, the operator already approved (`tif approve`); skip approval gate.
    pub user_approved: bool,
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
    /// Forbidden for [`ReviewerInvocationMode::CleanRoom`].
    pub source_or_diff: Option<String>,
    pub verification_plan_summary: Option<String>,
    pub max_output_bytes: u64,
    /// Prefer a specific authorized reviewer id (must be in the pool).
    pub preferred_reviewer_id: Option<String>,
    /// Exclude reviewer ids already used (Five-Alarm Stage 2).
    pub exclude_reviewer_ids: Vec<String>,
    /// Framing mode (standard / intensified / clean-room).
    pub mode: ReviewerInvocationMode,
    /// Structured failure summary (Five-Alarm).
    pub failure_summary: Option<String>,
    /// Previous implementation code — clean-room rejects non-empty values.
    pub prior_implementation_code: Option<String>,
}

impl BackendGenerateRequest {
    /// Standard Firebreak generation request (Phase 1/2 defaults).
    pub fn standard(
        run_id: impl Into<String>,
        policy: ContainmentPolicy,
        source_root: PathBuf,
        state_dir: PathBuf,
        original_metrics: DiffMetrics,
        original_score: ScoreResult,
        original_floor: CorrectnessFloor,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            policy,
            source_root,
            state_dir,
            original_metrics,
            original_score,
            original_floor,
            task_text: None,
            acceptance_criteria: None,
            source_or_diff: None,
            verification_plan_summary: None,
            max_output_bytes: 8_000_000,
            preferred_reviewer_id: None,
            exclude_reviewer_ids: Vec::new(),
            mode: ReviewerInvocationMode::Standard,
            failure_summary: None,
            prior_implementation_code: None,
        }
    }
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

        let selected = if let Some(ref id) = req.preferred_reviewer_id {
            self.selector.select_by_id(id)?
        } else {
            {
                let exclude: Vec<&str> = req
                    .exclude_reviewer_ids
                    .iter()
                    .map(String::as_str)
                    .collect();
                self.selector
                    .select_excluding(req.policy.task_category, &exclude)?
            }
        };
        let cfg = self.reviewer_config(&selected.id).cloned().ok_or_else(|| {
            TifError::UnauthorizedReviewer(format!(
                "selected reviewer `{}` missing from pool",
                selected.id
            ))
        })?;

        let backend = backend_for_provider(&self.registry, &cfg.provider)?;

        // Clean-room / intensified context; egress enforced when source is present.
        let context = build_reviewer_context(&ContextBuildRequest {
            reviewer: &cfg,
            policy: &req.policy,
            task_category: req.policy.task_category,
            task_text: req.task_text.as_deref(),
            acceptance_criteria: req.acceptance_criteria.as_deref(),
            original_metrics: Some(&req.original_metrics),
            source_or_diff: req.source_or_diff.as_deref(),
            verification_plan_summary: req.verification_plan_summary.as_deref(),
            mode: req.mode,
            failure_summary: req.failure_summary.as_deref(),
            prior_implementation_code: req.prior_implementation_code.as_deref(),
        })?;

        let credential = resolve_credential_opt(cfg.credential_ref.as_deref())?;

        let prefix = match req.mode {
            ReviewerInvocationMode::CleanRoom => "fa-cr",
            ReviewerInvocationMode::Intensified => "fa-i",
            ReviewerInvocationMode::Standard => "fb",
        };
        let session_id = format!("{prefix}-{}", &uuid::Uuid::new_v4().to_string()[..8]);
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
        let mode_label = match req.mode {
            ReviewerInvocationMode::Standard => "standard",
            ReviewerInvocationMode::Intensified => "intensified",
            ReviewerInvocationMode::CleanRoom => "clean-room",
        };
        let outcome = FirebreakOutcome {
            success: true,
            applied: false,
            candidate_ready: false,
            requires_approval: false,
            simulated: false,
            reviewer_id: Some(patch.reviewer_id.clone()),
            candidate_id: Some(format!("fb-{mode_label}-{}", patch.reviewer_id)),
            candidate_score: None,
            candidate_metrics: None,
            message: format!(
                "reviewer `{}` ({mode_label}) produced candidate at {}; re-verify before apply (not applied)",
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

        let requires_approval = !req.user_approved
            && (req.force_approval
                || req.policy.require_firebreak_approval
                || touches_sensitive(&req.policy, &req.candidate_metrics));

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
    ///
    /// Safety:
    /// - `candidate_root` must canonicalize and stay under the isolation session path,
    ///   a known staging parent (session path's parent / snapshots / worktrees), or be
    ///   an external tree that is **not** a symlink escape of those roots when nested.
    /// - Rejects path components `.` / `..` in joined names; never follows symlinks.
    pub fn stage_candidate_tree(session: &IsolationSession, candidate_root: &Path) -> Result<()> {
        if !candidate_root.exists() {
            return Err(TifError::Isolation(format!(
                "candidate tree missing: {}",
                candidate_root.display()
            )));
        }
        let cand_meta = fs::symlink_metadata(candidate_root).map_err(|e| {
            TifError::Isolation(format!(
                "cannot stat candidate {}: {e}",
                candidate_root.display()
            ))
        })?;
        if cand_meta.file_type().is_symlink() {
            return Err(TifError::Isolation(format!(
                "candidate root must not be a symlink: {}",
                candidate_root.display()
            )));
        }
        let cand_canon = candidate_root.canonicalize().map_err(|e| {
            TifError::Isolation(format!(
                "cannot canonicalize candidate {}: {e}",
                candidate_root.display()
            ))
        })?;
        ensure_candidate_staging_allowed(&cand_canon, session)?;
        copy_tree_into(&cand_canon, &session.path)
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

/// Candidate may be staged from:
/// - under the isolation session path itself
/// - under a known parent (session parent, snapshots dir, worktrees dir, source_root)
/// - or any other real directory that is **not** a symlink (external candidate trees)
///
/// What we reject: symlinked roots and paths whose lexical form contains `..`.
fn ensure_candidate_staging_allowed(cand_canon: &Path, session: &IsolationSession) -> Result<()> {
    for c in cand_canon.components() {
        if matches!(c, std::path::Component::ParentDir) {
            return Err(TifError::Isolation(
                "candidate path must not contain `..` after canonicalize".into(),
            ));
        }
    }
    // Always allow paths under the isolation session.
    let session_canon = session
        .path
        .canonicalize()
        .unwrap_or_else(|_| session.path.clone());
    if cand_canon.starts_with(&session_canon) {
        return Ok(());
    }
    // Known staging parents: session parent, source_root, .this-is-fine/{snapshots,worktrees}.
    let mut allowed_roots: Vec<PathBuf> = Vec::new();
    if let Some(parent) = session.path.parent() {
        allowed_roots.push(parent.to_path_buf());
    }
    allowed_roots.push(session.source_root.clone());
    let tif = session.source_root.join(".this-is-fine");
    allowed_roots.push(tif.join("snapshots"));
    allowed_roots.push(tif.join("worktrees"));
    for root in &allowed_roots {
        let root_c = root.canonicalize().unwrap_or_else(|_| root.clone());
        if cand_canon.starts_with(&root_c) {
            return Ok(());
        }
    }
    // External candidate trees are allowed only if they are ordinary directories
    // (symlink roots already rejected). This keeps `tif firebreak --candidate /tmp/x` working.
    Ok(())
}

/// Safe join: reject `.` / `..` name components and ensure result stays under `dst_root`.
fn safe_join_under(dst_root: &Path, name: &std::ffi::OsStr) -> Result<PathBuf> {
    let name_str = name.to_string_lossy();
    if name_str == "." || name_str == ".." || name_str.is_empty() {
        return Err(TifError::Isolation(format!(
            "refusing path component `{name_str}` while copying"
        )));
    }
    if name_str.contains('\0') {
        return Err(TifError::Isolation(
            "refusing path component containing NUL".into(),
        ));
    }
    // Reject separators inside a single component (shouldn't happen from read_dir).
    if name_str.contains('/') || name_str.contains('\\') {
        return Err(TifError::Isolation(format!(
            "refusing path component with separator: {name_str}"
        )));
    }
    let joined = dst_root.join(name);
    // Lexical containment: joined must start with dst_root.
    if !joined.starts_with(dst_root) {
        return Err(TifError::Isolation(format!(
            "copy destination escapes root: {}",
            joined.display()
        )));
    }
    Ok(joined)
}

fn copy_tree_into(src: &Path, dst: &Path) -> Result<()> {
    copy_tree_into_inner(src, dst, dst)
}

fn copy_tree_into_inner(src: &Path, dst: &Path, dst_root: &Path) -> Result<()> {
    let src_meta = fs::symlink_metadata(src)?;
    let src_ft = src_meta.file_type();
    if src_ft.is_symlink() {
        // Never follow or copy symlinks.
        return Ok(());
    }
    if src_ft.is_file() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        // Ensure destination stays under root.
        if !dst.starts_with(dst_root) {
            return Err(TifError::Isolation(format!(
                "copy destination escapes root: {}",
                dst.display()
            )));
        }
        fs::copy(src, dst)?;
        return Ok(());
    }
    if !src_ft.is_dir() {
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
        // Skip symlinks entirely (files or dirs).
        let meta = match fs::symlink_metadata(&from) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        let to = safe_join_under(dst, &name)?;
        if !to.starts_with(dst_root) {
            return Err(TifError::Isolation(format!(
                "copy destination escapes root: {}",
                to.display()
            )));
        }
        if meta.file_type().is_dir() {
            copy_tree_into_inner(&from, &to, dst_root)?;
        } else if meta.file_type().is_file() {
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

/// Five-Alarm recovery stage (design §8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FiveAlarmStage {
    /// Not started / outline only.
    Idle,
    /// Gate: require current containment failure (historical alone insufficient).
    Gate,
    /// Stage 1: intensified Firebreak (higher attempts, stricter wording).
    Stage1Intensified,
    /// Stage 2: preserve Stage 1 candidate; select a different authorized model.
    Stage2PreserveReselect,
    /// Stage 3: clean-room on original state + task/policy/failure summary (no prior code).
    Stage3CleanRoom,
    /// Stage 4: verify all candidates; apply smallest verified; retain rejects.
    Stage4VerifySelect,
    /// Terminal success (winner applied or selected without apply).
    Complete,
    /// Terminal abort (gate failed, no usable path, or fail-safe).
    Aborted,
}

impl FiveAlarmStage {
    pub fn as_str(self) -> &'static str {
        match self {
            FiveAlarmStage::Idle => "idle",
            FiveAlarmStage::Gate => "gate",
            FiveAlarmStage::Stage1Intensified => "stage1_intensified",
            FiveAlarmStage::Stage2PreserveReselect => "stage2_preserve_reselect",
            FiveAlarmStage::Stage3CleanRoom => "stage3_clean_room",
            FiveAlarmStage::Stage4VerifySelect => "stage4_verify_select",
            FiveAlarmStage::Complete => "complete",
            FiveAlarmStage::Aborted => "aborted",
        }
    }
}

/// Kind of Five-Alarm candidate retained for ranking / rollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FiveAlarmCandidateKind {
    Intensified,
    CleanRoom,
    Original,
}

/// A candidate produced or considered during Five-Alarm recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiveAlarmCandidate {
    pub id: String,
    pub kind: FiveAlarmCandidateKind,
    pub reviewer_id: Option<String>,
    pub isolation_session_id: Option<String>,
    pub candidate_path: Option<PathBuf>,
    pub metrics: Option<DiffMetrics>,
    pub score: Option<ScoreResult>,
    pub floor_passed: bool,
    pub within_containment: bool,
    pub verified: bool,
    /// Retained during rollback period when not applied.
    pub retained_for_rollback: bool,
    pub applied: bool,
    pub message: String,
}

/// Audit timeline entry for a Five-Alarm stage transition or action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiveAlarmTimelineEntry {
    pub stage: FiveAlarmStage,
    pub at: String,
    pub message: String,
    pub reviewer_id: Option<String>,
    pub candidate_id: Option<String>,
}

/// Five-Alarm staged recovery plan and live state machine (design §8.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiveAlarmPlan {
    /// Human-readable outline steps (for `tif five-alarm --plan`).
    pub steps: Vec<String>,
    pub stage: FiveAlarmStage,
    /// True only when the **current** task has a concrete containment failure.
    pub current_containment_failure: bool,
    /// True when the caller tried to escalate on historical risk alone.
    pub historical_risk_only: bool,
    pub timeline: Vec<FiveAlarmTimelineEntry>,
    pub candidates: Vec<FiveAlarmCandidate>,
    pub used_reviewer_ids: Vec<String>,
    pub winner_id: Option<String>,
    pub message: String,
    pub applied: bool,
    pub original_preserved: bool,
    /// Isolation session of the applied winner (if any).
    pub applied_session_id: Option<String>,
}

impl FiveAlarmPlan {
    /// Static outline for CLI `--plan` (no escalation).
    pub fn staged_recovery() -> Self {
        Self {
            steps: Self::outline_steps(),
            stage: FiveAlarmStage::Idle,
            current_containment_failure: false,
            historical_risk_only: false,
            timeline: Vec::new(),
            candidates: Vec::new(),
            used_reviewer_ids: Vec::new(),
            winner_id: None,
            message: "Five-Alarm staged recovery outline (not escalated)".into(),
            applied: false,
            original_preserved: true,
            applied_session_id: None,
        }
    }

    fn outline_steps() -> Vec<String> {
        vec![
            "Gate: require current containment failure (historical risk alone insufficient)".into(),
            "Stage 1: intensified Firebreak on current verified implementation (higher attempts, stricter wording)".into(),
            "Stage 2: if still out of containment, preserve as candidate; select another authorized model".into(),
            "Stage 3: clean-room model gets original state, task, criteria, policy, verification plan, failure summary — not previous implementation code".into(),
            "Stage 4: verify all candidates against the same correctness floor".into(),
            "Apply the smallest verified candidate".into(),
            "Preserve rejected candidates during rollback period".into(),
        ]
    }

    /// Begin escalation. Historical risk alone is insufficient.
    pub fn begin_escalation(
        current_containment_failure: bool,
        historical_risk_noted: bool,
    ) -> Result<Self> {
        let mut plan = Self {
            steps: Self::outline_steps(),
            stage: FiveAlarmStage::Gate,
            current_containment_failure,
            historical_risk_only: historical_risk_noted && !current_containment_failure,
            timeline: Vec::new(),
            candidates: Vec::new(),
            used_reviewer_ids: Vec::new(),
            winner_id: None,
            message: String::new(),
            applied: false,
            original_preserved: true,
            applied_session_id: None,
        };

        // Design §8.1: historical risk alone is insufficient.
        if !current_containment_failure {
            plan.stage = FiveAlarmStage::Aborted;
            plan.message = if historical_risk_noted {
                "Five-Alarm refused: historical risk alone is insufficient; current containment failure required"
                    .into()
            } else {
                "Five-Alarm refused: no current containment failure (escalation-only)".into()
            };
            plan.push_timeline(FiveAlarmStage::Gate, plan.message.clone(), None, None);
            return Err(TifError::FiveAlarmInitialForbidden);
        }

        // Also enforce FireLevel gate for consistency.
        let _ = FireLevel::escalate_to_five_alarm(true)?;

        plan.push_timeline(
            FiveAlarmStage::Gate,
            "current containment failure confirmed; escalating to Five-Alarm",
            None,
            None,
        );
        plan.stage = FiveAlarmStage::Stage1Intensified;
        plan.message = "Five-Alarm gate passed; entering Stage 1 (intensified Firebreak)".into();
        plan.push_timeline(
            FiveAlarmStage::Stage1Intensified,
            plan.message.clone(),
            None,
            None,
        );
        Ok(plan)
    }

    fn push_timeline(
        &mut self,
        stage: FiveAlarmStage,
        message: impl Into<String>,
        reviewer_id: Option<String>,
        candidate_id: Option<String>,
    ) {
        self.push_timeline_pub(stage, message, reviewer_id, candidate_id);
    }

    /// Append a timeline entry (used by orchestrator stage transitions).
    pub fn push_timeline_pub(
        &mut self,
        stage: FiveAlarmStage,
        message: impl Into<String>,
        reviewer_id: Option<String>,
        candidate_id: Option<String>,
    ) {
        self.timeline.push(FiveAlarmTimelineEntry {
            stage,
            at: chrono::Utc::now().to_rfc3339(),
            message: message.into(),
            reviewer_id,
            candidate_id,
        });
    }

    /// Build a structured failure summary for clean-room / intensified context.
    pub fn build_failure_summary(
        original_score: &ScoreResult,
        original_metrics: &DiffMetrics,
        prior_firebreak: Option<&FirebreakOutcome>,
        stage1: Option<&FiveAlarmCandidate>,
    ) -> String {
        let mut s = String::new();
        s.push_str("current_task_containment_failure: true\n");
        s.push_str(&format!(
            "original_within_containment: {}\n",
            original_score.within_containment
        ));
        s.push_str(&format!(
            "original_score: {:.2} disqualified={}\n",
            original_score.score, original_score.disqualified
        ));
        if !original_score.hard_limit_violations.is_empty() {
            s.push_str(&format!(
                "hard_limit_violations: {:?}\n",
                original_score.hard_limit_violations
            ));
        }
        s.push_str(&format!(
            "metrics: files_added={} lines_added={} deps_added={} abstractions={}\n",
            original_metrics.files_added,
            original_metrics.lines_added,
            original_metrics.runtime_dependencies_added,
            original_metrics.abstractions_added
        ));
        if let Some(fb) = prior_firebreak {
            s.push_str(&format!(
                "prior_firebreak: success={} applied={} ready={} msg={}\n",
                fb.success, fb.applied, fb.candidate_ready, fb.message
            ));
        }
        if let Some(c) = stage1 {
            s.push_str(&format!(
                "stage1_candidate: id={} verified={} within_containment={} reviewer={:?} msg={}\n",
                c.id, c.verified, c.within_containment, c.reviewer_id, c.message
            ));
        }
        s.push_str(
            "instruction: re-implement with maximum restraint; do not request prior patch content.\n",
        );
        s
    }

    /// Select the smallest verified in-containment candidate (excluding original).
    pub fn select_smallest_verified_winner(&self) -> Option<&FiveAlarmCandidate> {
        self.candidates
            .iter()
            .filter(|c| {
                c.kind != FiveAlarmCandidateKind::Original
                    && c.verified
                    && c.floor_passed
                    && c.within_containment
                    && c.score.as_ref().is_some_and(|s| !s.disqualified)
            })
            .min_by(|a, b| {
                let sa = a.score.as_ref().map(|s| s.score).unwrap_or(f64::MAX);
                let sb = b.score.as_ref().map(|s| s.score).unwrap_or(f64::MAX);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
    }
}

/// Options for a Five-Alarm recovery run.
#[derive(Debug, Clone)]
pub struct FiveAlarmRunOptions {
    pub authorize_apply: bool,
    pub user_approved: bool,
    /// When true, treat historical risk as noted (still insufficient alone).
    pub historical_risk_noted: bool,
}

impl Default for FiveAlarmRunOptions {
    fn default() -> Self {
        Self {
            authorize_apply: true,
            user_approved: false,
            historical_risk_noted: false,
        }
    }
}

/// Result of a full or partial Five-Alarm recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiveAlarmRunResult {
    pub plan: FiveAlarmPlan,
    pub firebreak: Option<FirebreakOutcome>,
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

        let mut gen_req = BackendGenerateRequest::standard(
            "gen1",
            policy,
            source.clone(),
            paths.state_dir.clone(),
            metrics,
            score,
            CorrectnessFloor::all_pass(),
        );
        gen_req.task_text = Some("shrink".into());
        gen_req.max_output_bytes = 1_000_000;
        let result = engine.generate_with_backend(gen_req).unwrap();

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
                user_approved: false,
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
                    user_approved: false,
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
                    user_approved: false,
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
                    user_approved: false,
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

    // --- Phase 3: Five-Alarm staged recovery ---

    fn ok_score(score: f64) -> ScoreResult {
        ScoreResult {
            correctness_passed: true,
            disqualified: false,
            score,
            hard_limit_violations: vec![],
            floor_failures: vec![],
            within_containment: true,
            breakdown: Default::default(),
        }
    }

    #[test]
    fn five_alarm_historical_risk_alone_insufficient() {
        let err = FiveAlarmPlan::begin_escalation(false, true).unwrap_err();
        assert!(matches!(err, TifError::FiveAlarmInitialForbidden));
        let err2 = FiveAlarmPlan::begin_escalation(false, false).unwrap_err();
        assert!(matches!(err2, TifError::FiveAlarmInitialForbidden));
    }

    #[test]
    fn five_alarm_gate_passes_on_current_failure() {
        let plan = FiveAlarmPlan::begin_escalation(true, true).unwrap();
        assert!(plan.current_containment_failure);
        assert!(!plan.historical_risk_only);
        assert_eq!(plan.stage, FiveAlarmStage::Stage1Intensified);
        assert!(!plan.timeline.is_empty());
        assert!(plan
            .timeline
            .iter()
            .any(|e| e.stage == FiveAlarmStage::Gate));
    }

    #[test]
    fn five_alarm_outline_lists_all_stages() {
        let outline = FiveAlarmPlan::staged_recovery();
        assert_eq!(outline.stage, FiveAlarmStage::Idle);
        assert!(outline.steps.len() >= 6);
        assert!(outline.steps.iter().any(|s| s.contains("intensified")));
        assert!(outline
            .steps
            .iter()
            .any(|s| s.contains("clean-room") || s.contains("Clean-room")));
        assert!(outline.steps.iter().any(|s| s.contains("smallest")));
    }

    #[test]
    fn five_alarm_select_smallest_verified_winner() {
        let mut plan = FiveAlarmPlan::begin_escalation(true, false).unwrap();
        plan.candidates.push(FiveAlarmCandidate {
            id: "big".into(),
            kind: FiveAlarmCandidateKind::Intensified,
            reviewer_id: Some("r1".into()),
            isolation_session_id: None,
            candidate_path: None,
            metrics: None,
            score: Some(ok_score(80.0)),
            floor_passed: true,
            within_containment: true,
            verified: true,
            retained_for_rollback: true,
            applied: false,
            message: "big".into(),
        });
        plan.candidates.push(FiveAlarmCandidate {
            id: "small".into(),
            kind: FiveAlarmCandidateKind::CleanRoom,
            reviewer_id: Some("r2".into()),
            isolation_session_id: None,
            candidate_path: None,
            metrics: None,
            score: Some(ok_score(20.0)),
            floor_passed: true,
            within_containment: true,
            verified: true,
            retained_for_rollback: true,
            applied: false,
            message: "small".into(),
        });
        plan.candidates.push(FiveAlarmCandidate {
            id: "unverified".into(),
            kind: FiveAlarmCandidateKind::Intensified,
            reviewer_id: Some("r3".into()),
            isolation_session_id: None,
            candidate_path: None,
            metrics: None,
            score: Some(ok_score(5.0)),
            floor_passed: false,
            within_containment: true,
            verified: false,
            retained_for_rollback: true,
            applied: false,
            message: "bad".into(),
        });
        let w = plan.select_smallest_verified_winner().unwrap();
        assert_eq!(w.id, "small");
    }

    #[test]
    fn five_alarm_failure_summary_has_no_patch_body() {
        let metrics = DiffMetrics {
            lines_added: 50,
            files_added: 2,
            runtime_dependencies_added: 1,
            ..Default::default()
        };
        let scorer = SimplicityScorer::new(
            crate::config::SimplicityWeights::default(),
            SimplicityLimits {
                new_runtime_dependencies: Some(0),
                ..Default::default()
            },
        );
        let score = scorer.score(&metrics, &CorrectnessFloor::all_pass());
        let summary = FiveAlarmPlan::build_failure_summary(&score, &metrics, None, None);
        assert!(summary.contains("current_task_containment_failure"));
        assert!(!summary.contains("fn bloated"));
        assert!(!summary.contains("diff --git"));
    }

    #[test]
    #[cfg(feature = "provider-mock")]
    fn intensified_generate_uses_mode_and_preserves_source() {
        use crate::config::RepoPaths;

        let dir = tempdir().unwrap();
        let source = dir.path().join("src_repo");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::write(source.join("src/lib.rs"), b"pub fn f() {}\n").unwrap();
        fs::write(source.join("KEEP.txt"), b"original").unwrap();
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

        let mut req = BackendGenerateRequest::standard(
            "fa1",
            policy,
            source.clone(),
            paths.state_dir.clone(),
            metrics,
            score,
            CorrectnessFloor::all_pass(),
        );
        req.mode = ReviewerInvocationMode::Intensified;
        req.failure_summary = Some("exceeded limits".into());
        req.task_text = Some("shrink".into());

        let result = engine.generate_with_backend(req).unwrap();
        assert!(result.outcome.original_preserved);
        assert!(!result.outcome.applied);
        assert!(result.outcome.message.contains("intensified"));
        assert_eq!(
            fs::read_to_string(source.join("KEEP.txt")).unwrap(),
            "original"
        );
    }

    #[test]
    #[cfg(feature = "provider-mock")]
    fn clean_room_generate_rejects_prior_code_and_preserves_source() {
        use crate::config::RepoPaths;

        let dir = tempdir().unwrap();
        let source = dir.path().join("src_repo");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::write(source.join("src/lib.rs"), b"pub fn f() {}\n").unwrap();
        let paths = RepoPaths::for_root(&source);
        crate::config::ensure_state_dirs(&paths).unwrap();

        let engine = FirebreakEngine::new(ReviewerSelector::new(vec![
            ReviewerConfig::mock("r1", 10),
            ReviewerConfig::mock("r2", 5),
        ]));
        let policy = policy_with_limits();
        let metrics = DiffMetrics {
            runtime_dependencies_added: 1,
            lines_added: 40,
            ..Default::default()
        };
        let scorer = SimplicityScorer::new(policy.weights.clone(), policy.limits.clone());
        let score = scorer.score(&metrics, &CorrectnessFloor::all_pass());

        let mut bad = BackendGenerateRequest::standard(
            "fa-cr-bad",
            policy.clone(),
            source.clone(),
            paths.state_dir.clone(),
            metrics.clone(),
            score.clone(),
            CorrectnessFloor::all_pass(),
        );
        bad.mode = ReviewerInvocationMode::CleanRoom;
        bad.preferred_reviewer_id = Some("r2".into());
        bad.prior_implementation_code = Some("fn prior_patch() {}".into());
        bad.failure_summary = Some("stage1 ooc".into());
        assert!(engine.generate_with_backend(bad).is_err());

        let mut good = BackendGenerateRequest::standard(
            "fa-cr-ok",
            policy,
            source.clone(),
            paths.state_dir.clone(),
            metrics,
            score,
            CorrectnessFloor::all_pass(),
        );
        good.mode = ReviewerInvocationMode::CleanRoom;
        good.preferred_reviewer_id = Some("r2".into());
        good.failure_summary = Some("stage1 still ooc; hard limits".into());
        good.task_text = Some("minimal fix".into());
        let result = engine.generate_with_backend(good).unwrap();
        assert!(result.outcome.original_preserved);
        assert!(result.outcome.message.contains("clean-room"));
        assert_eq!(result.outcome.reviewer_id.as_deref(), Some("r2"));
    }
}

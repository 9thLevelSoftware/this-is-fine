//! Firebreak engine: independent simplification with fail-safe application.

use globset::{Glob, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use crate::error::{Result, TifError};
use crate::policy::ContainmentPolicy;
use crate::reviewer::{ReviewerSelector, SelectedReviewer};
use crate::scoring::{
    select_smaller_verified, CorrectnessFloor, DiffMetrics, ScoreResult, SimplicityScorer,
};

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
    /// MVP never sets this from production; tests may set it to exercise apply path.
    pub workspace_apply_authorized: bool,
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
}

/// Automatic Firebreak using an authorized reviewer only.
#[derive(Debug)]
pub struct FirebreakEngine {
    selector: ReviewerSelector,
}

impl FirebreakEngine {
    pub fn new(selector: ReviewerSelector) -> Self {
        Self { selector }
    }

    /// Produce a smaller correct candidate in isolation. Never replaces original on failure.
    ///
    /// Without a real reviewer backend + isolation + re-verification, this is fail-closed:
    /// `applied` stays false and the original workspace is preserved.
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
                message: "Firebreak reviewer backend not invoked; original retained (fail-closed until isolation + re-verify)".into(),
                original_preserved: true,
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
            });
        }

        let requires_approval = req.policy.require_firebreak_approval
            || touches_sensitive(&req.policy, &candidate.metrics);

        // Never apply without isolation + authorized workspace apply (MVP: no real apply).
        let can_apply =
            req.workspace_apply_authorized && !requires_approval && !candidate.simulated_unverified;

        Ok(FirebreakOutcome {
            success: true,
            applied: can_apply,
            candidate_ready: true,
            requires_approval,
            simulated: true,
            reviewer_id: Some(reviewer.id),
            candidate_id: Some(candidate.id),
            candidate_score: Some(candidate_score),
            candidate_metrics: Some(candidate.metrics),
            message: if requires_approval {
                "simulated smaller candidate ready; approval required (not applied)".into()
            } else if can_apply {
                "smaller verified candidate applied; original retained for rollback".into()
            } else {
                "simulated smaller candidate ready; not applied (no isolation/re-verify backend)"
                    .into()
            },
            original_preserved: !can_apply,
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ReviewerConfig, SimplicityLimits};
    use crate::policy::{PolicyCompileRequest, PolicyCompiler};
    use crate::scoring::SimplicityScorer;

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
        FirebreakEngine::new(ReviewerSelector::new(vec![ReviewerConfig {
            id: "r1".into(),
            provider: "mock".into(),
            model: "m".into(),
            endpoint: None,
            credential_ref: None,
            allow_source_egress: false,
            eligible_task_types: vec![],
            max_firebreak_attempts: 2,
            priority: 1,
        }]))
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
}

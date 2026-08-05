//! Run orchestrator and state machine with stable run IDs.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use crate::assess::{AssessmentStatus, DamageAssessment, DamageAssessor};
use crate::config::{Config, RepoPaths};
use crate::diff::metrics_from_tree_absolute;
use crate::error::{Result, TifError};
use crate::fire_level::FireLevel;
use crate::firebreak::{
    candidate_floor_from_verification, BackendGenerateRequest, FirebreakEngine, FirebreakOutcome,
    FirebreakRequest,
};
use crate::inspector::RepositoryInspector;
use crate::isolation::{isolator_for_session, session_candidate_root, IsolationSession};
use crate::policy::{ContainmentPolicy, PolicyCompileRequest, PolicyCompiler};
use crate::providers::{backend_for_provider, BackendRegistry};
use crate::reviewer::ReviewerSelector;
use crate::scoring::{CorrectnessFloor, DiffMetrics, ScoreResult, SimplicityScorer};
use crate::verify::{plan_and_run, VerificationReport};

/// Stable local run identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunId(pub String);

impl RunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Run lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Preflight,
    PolicySelected,
    AgentInjected,
    Implementing,
    ImplementationComplete,
    DiffCaptured,
    Verifying,
    Scoring,
    Contained,
    OutOfControl,
    FirebreakRunning,
    CandidateComparing,
    AwaitingApproval,
    Applied,
    Restored,
    Rejected,
    Closed,
    Failed,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Preflight => "preflight",
            RunState::PolicySelected => "policy_selected",
            RunState::AgentInjected => "agent_injected",
            RunState::Implementing => "implementing",
            RunState::ImplementationComplete => "implementation_complete",
            RunState::DiffCaptured => "diff_captured",
            RunState::Verifying => "verifying",
            RunState::Scoring => "scoring",
            RunState::Contained => "contained",
            RunState::OutOfControl => "out_of_control",
            RunState::FirebreakRunning => "firebreak_running",
            RunState::CandidateComparing => "candidate_comparing",
            RunState::AwaitingApproval => "awaiting_approval",
            RunState::Applied => "applied",
            RunState::Restored => "restored",
            RunState::Rejected => "rejected",
            RunState::Closed => "closed",
            RunState::Failed => "failed",
        }
    }

    /// Whether transition `from -> to` is allowed.
    pub fn can_transition(from: RunState, to: RunState) -> bool {
        use RunState::*;
        matches!(
            (from, to),
            (Preflight, PolicySelected)
                | (Preflight, Failed)
                | (PolicySelected, AgentInjected)
                | (PolicySelected, Failed)
                | (AgentInjected, Implementing)
                | (Implementing, ImplementationComplete)
                | (Implementing, Failed)
                | (ImplementationComplete, DiffCaptured)
                | (DiffCaptured, Verifying)
                | (Verifying, Scoring)
                | (Verifying, Rejected)
                | (Verifying, Failed)
                | (Scoring, Contained)
                | (Scoring, OutOfControl)
                | (Scoring, Rejected)
                | (Contained, Closed)
                // Manual Firebreak is allowed from a contained run.
                | (Contained, FirebreakRunning)
                | (OutOfControl, FirebreakRunning)
                | (OutOfControl, Closed)
                | (FirebreakRunning, CandidateComparing)
                | (FirebreakRunning, Restored)
                | (FirebreakRunning, Failed)
                | (CandidateComparing, AwaitingApproval)
                | (CandidateComparing, Applied)
                | (CandidateComparing, Restored)
                | (AwaitingApproval, Applied)
                | (AwaitingApproval, Restored)
                | (Applied, Closed)
                // Rollback after successful Firebreak apply.
                | (Applied, Restored)
                | (Restored, Closed)
                | (Rejected, Closed)
                | (Failed, Closed)
                // Allow re-entry for complete/assess from begin
                | (PolicySelected, DiffCaptured)
                | (AgentInjected, DiffCaptured)
                | (Implementing, DiffCaptured)
        )
    }
}

/// In-memory (and serializable) run record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: RunId,
    pub state: RunState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub repo_root: String,
    pub task_text: Option<String>,
    pub agent_id: Option<String>,
    pub model_id: Option<String>,
    pub policy: Option<ContainmentPolicy>,
    pub metrics: Option<DiffMetrics>,
    pub verification: Option<VerificationReport>,
    pub score: Option<ScoreResult>,
    pub assessment: Option<DamageAssessment>,
    pub firebreak: Option<FirebreakOutcome>,
    pub rollback_candidate_id: Option<String>,
    /// Isolation session for Firebreak apply/rollback (filesystem-backed).
    #[serde(default)]
    pub isolation_session: Option<IsolationSession>,
    /// When set, pending Firebreak approval expires at this time (UTC).
    #[serde(default)]
    pub approval_expires_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub events: Vec<String>,
}

impl RunRecord {
    pub fn new(repo_root: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: RunId::new(),
            state: RunState::Preflight,
            created_at: now,
            updated_at: now,
            repo_root: repo_root.into(),
            task_text: None,
            agent_id: None,
            model_id: None,
            policy: None,
            metrics: None,
            verification: None,
            score: None,
            assessment: None,
            firebreak: None,
            rollback_candidate_id: None,
            isolation_session: None,
            approval_expires_at: None,
            error: None,
            events: vec!["run created".into()],
        }
    }

    pub fn transition(&mut self, to: RunState) -> Result<()> {
        if !RunState::can_transition(self.state, to) {
            return Err(TifError::InvalidTransition {
                from: self.state.as_str().into(),
                to: to.as_str().into(),
            });
        }
        self.events
            .push(format!("{} -> {}", self.state.as_str(), to.as_str()));
        self.state = to;
        self.updated_at = Utc::now();
        Ok(())
    }

    pub fn fail(&mut self, msg: impl Into<String>) {
        self.error = Some(msg.into());
        let _ = self.transition(RunState::Failed);
        if self.state != RunState::Failed {
            self.state = RunState::Failed;
            self.updated_at = Utc::now();
        }
    }
}

/// Orchestrates the containment run lifecycle.
#[derive(Debug, Default)]
pub struct RunOrchestrator;

impl RunOrchestrator {
    pub fn new() -> Self {
        Self
    }

    /// Begin a run: preflight + policy selection.
    pub fn begin(
        &self,
        config: &Config,
        repo_root: &str,
        req: BeginRunRequest,
    ) -> Result<RunRecord> {
        let mut run = RunRecord::new(repo_root);
        run.task_text = req.task_text.clone();
        run.agent_id = req.agent_id.clone();
        run.model_id = req.model_id.clone();

        if !config.enabled && !req.force {
            run.fail("containment is disabled (tif off); use --force to run anyway");
            return Ok(run);
        }

        let compile_req = PolicyCompileRequest {
            task_text: req.task_text,
            task_category: req.task_category,
            fire_level: req.fire_level,
            escalate_five_alarm: false,
            current_containment_failure: false,
            enabled: Some(true),
            adaptive_limit_scale: None,
            model_id: req.model_id,
        };

        let policy = match PolicyCompiler::new().compile(config, &compile_req) {
            Ok(p) => p,
            Err(e) => {
                run.fail(e.to_string());
                return Ok(run);
            }
        };

        run.transition(RunState::PolicySelected)?;
        run.policy = Some(policy);
        run.transition(RunState::AgentInjected)?;
        run.transition(RunState::Implementing)?;
        Ok(run)
    }

    /// Complete implementation: capture metrics, verify, score, maybe firebreak.
    pub fn complete(
        &self,
        config: &Config,
        run: &mut RunRecord,
        metrics: DiffMetrics,
        verification: VerificationReport,
        correctness: CorrectnessFloor,
        auto_firebreak: bool,
    ) -> Result<()> {
        if matches!(run.state, RunState::Failed | RunState::Closed) {
            return Err(TifError::InvalidTransition {
                from: run.state.as_str().into(),
                to: "complete".into(),
            });
        }

        // Move to diff captured from implementing (or earlier agent states).
        while !matches!(
            run.state,
            RunState::DiffCaptured
                | RunState::Verifying
                | RunState::Scoring
                | RunState::Contained
                | RunState::OutOfControl
        ) {
            let next = match run.state {
                RunState::Implementing => RunState::ImplementationComplete,
                RunState::ImplementationComplete => RunState::DiffCaptured,
                RunState::AgentInjected | RunState::PolicySelected => RunState::DiffCaptured,
                other => {
                    return Err(TifError::InvalidTransition {
                        from: other.as_str().into(),
                        to: "diff_captured".into(),
                    });
                }
            };
            run.transition(next)?;
        }

        run.metrics = Some(metrics.clone());
        run.transition(RunState::Verifying)?;
        run.verification = Some(verification.clone());

        let mut floor = correctness;
        if !verification.satisfies_correctness_verification() {
            floor.verification_passed = false;
            if verification.incomplete_plan {
                floor
                    .notes
                    .push("verification plan incomplete — not treated as passed".into());
            }
            if verification.has_unresolved_required {
                floor
                    .notes
                    .push("required verification check is unresolved".into());
            }
            if !verification.all_required_passed {
                floor
                    .notes
                    .push("required verification checks failed or missing".into());
            }
        }

        run.transition(RunState::Scoring)?;
        let policy = run
            .policy
            .as_ref()
            .ok_or_else(|| TifError::Other("run missing policy".into()))?;

        let scorer = SimplicityScorer::new(policy.weights.clone(), policy.limits.clone());
        let score = scorer.score(&metrics, &floor);
        run.score = Some(score.clone());

        let fire_level = policy.fire_level;
        let mut assessment = DamageAssessor::build(
            run.id.as_str(),
            "original",
            fire_level,
            metrics,
            score.clone(),
            floor.clone(),
            Some(verification.clone()),
        );
        assessment.pressure_template_id = Some(policy.pressure.template_id.clone());
        assessment.policy_version = Some(policy.policy_version.clone());

        if score.disqualified {
            // Incomplete / required-unresolved → Unverified; hard required failures → Rejected.
            // Optional low-confidence Unresolved discoveries do not force Unverified.
            assessment.status =
                if verification.incomplete_plan || verification.has_unresolved_required {
                    AssessmentStatus::Unverified
                } else {
                    AssessmentStatus::Rejected
                };
            run.assessment = Some(assessment);
            run.transition(RunState::Rejected)?;
            return Ok(());
        }

        if score.within_containment {
            assessment.status = AssessmentStatus::Contained;
            run.assessment = Some(assessment);
            run.transition(RunState::Contained)?;
            return Ok(());
        }

        assessment.status = AssessmentStatus::OutOfControl;
        run.assessment = Some(assessment);
        run.transition(RunState::OutOfControl)?;

        if auto_firebreak {
            self.run_firebreak(config, run, &floor)?;
        }

        Ok(())
    }

    /// Run Firebreak against an out-of-control but correct implementation.
    ///
    /// When authorized reviewers with compiled backends are configured, runs the
    /// automatic isolated closed loop: backend → re-verify → rank → optional apply.
    /// Without backends, remains fail-closed (original retained).
    pub fn run_firebreak(
        &self,
        config: &Config,
        run: &mut RunRecord,
        original_floor: &CorrectnessFloor,
    ) -> Result<()> {
        self.run_firebreak_auto(
            config,
            run,
            original_floor,
            FirebreakAutoOptions {
                authorize_apply: config.approval.auto_apply_firebreak,
                user_approved: false,
            },
        )
    }

    /// Automatic isolated Firebreak closed loop (Phase 2).
    pub fn run_firebreak_auto(
        &self,
        config: &Config,
        run: &mut RunRecord,
        original_floor: &CorrectnessFloor,
        options: FirebreakAutoOptions,
    ) -> Result<()> {
        if run.state != RunState::FirebreakRunning {
            run.transition(RunState::FirebreakRunning)?;
        }

        let policy = run
            .policy
            .clone()
            .ok_or_else(|| TifError::Other("run missing policy".into()))?;
        let metrics = run.metrics.clone().unwrap_or_default();
        let original_score = run
            .score
            .clone()
            .ok_or_else(|| TifError::Other("run missing score".into()))?;

        if !self.has_usable_reviewer_backend(config) {
            let selector = ReviewerSelector::new(config.reviewers.clone());
            let engine = FirebreakEngine::new(selector);
            let outcome = engine.simplify(FirebreakRequest {
                run_id: run.id.as_str().to_string(),
                policy,
                original_metrics: metrics,
                original_score,
                original_floor: original_floor.clone(),
                simulate_success: false,
                workspace_apply_authorized: false,
            })?;
            return self.finalize_firebreak(config, run, outcome);
        }

        let source_root = PathBuf::from(&run.repo_root);
        if !source_root.exists() {
            let outcome = FirebreakOutcome {
                success: false,
                applied: false,
                candidate_ready: false,
                requires_approval: false,
                simulated: false,
                reviewer_id: None,
                candidate_id: None,
                candidate_score: None,
                candidate_metrics: None,
                message: format!(
                    "Firebreak closed loop aborted: repo root missing ({})",
                    source_root.display()
                ),
                original_preserved: true,
                isolation_session_id: None,
            };
            return self.finalize_firebreak(config, run, outcome);
        }

        let paths = RepoPaths::for_root(&source_root);
        crate::config::ensure_state_dirs(&paths)?;

        let selector = ReviewerSelector::new(config.reviewers.clone());
        let engine = FirebreakEngine::new(selector);

        let plan_summary = run
            .verification
            .as_ref()
            .map(|v| {
                format!(
                    "required_passed={} incomplete={} checks={}",
                    v.all_required_passed,
                    v.incomplete_plan,
                    v.checks.len()
                )
            })
            .or_else(|| Some("use repository verification plan after generation".into()));

        let gen = engine.generate_with_backend(BackendGenerateRequest {
            run_id: run.id.as_str().to_string(),
            policy: policy.clone(),
            source_root: source_root.clone(),
            state_dir: paths.state_dir.clone(),
            original_metrics: metrics,
            original_score: original_score.clone(),
            original_floor: original_floor.clone(),
            task_text: run.task_text.clone(),
            acceptance_criteria: None,
            source_or_diff: None,
            verification_plan_summary: plan_summary,
            max_output_bytes: 8_000_000,
        })?;

        // Attach session early (before re-verify / apply) for crash recovery.
        run.isolation_session = Some(gen.session.clone());
        run.events
            .push(format!("firebreak backend: {}", gen.outcome.message));

        if !gen.outcome.success || gen.patch.is_none() {
            return self.finalize_firebreak(config, run, gen.outcome);
        }

        let patch = gen.patch.unwrap();
        let mut session = gen.session;
        session.candidate_path = Some(patch.candidate_root.clone());
        run.isolation_session = Some(session.clone());

        // Re-verify candidate tree with the same verification plan (never lower the floor).
        let verify_root = session_candidate_root(&session).to_path_buf();
        let inspection = RepositoryInspector::new()
            .inspect(&verify_root)
            .map_err(|e| {
                TifError::Other(format!(
                    "failed to inspect firebreak candidate {}: {e}",
                    verify_root.display()
                ))
            })?;
        let report =
            match plan_and_run(&verify_root, &config.verification, Some(&inspection), false) {
                Ok(r) => {
                    if r.incomplete_plan {
                        run.events.push(
                            "firebreak re-verify: plan incomplete (no required checks ran)".into(),
                        );
                    }
                    r
                }
                Err(e) => {
                    run.events
                        .push(format!("firebreak re-verify plan_and_run failed: {e}"));
                    let mut outcome = gen.outcome;
                    outcome.success = false;
                    outcome.candidate_ready = false;
                    outcome.message = format!("re-verify failed; original retained: {e}");
                    outcome.original_preserved = true;
                    outcome.applied = false;
                    return self.finalize_firebreak(config, run, outcome);
                }
            };

        let candidate_floor = candidate_floor_from_verification(&report);
        run.events.push(format!(
            "firebreak re-verify: required_passed={} incomplete={}",
            report.all_required_passed, report.incomplete_plan
        ));

        // Rank with absolute tree metrics on both sides (same measurement kind).
        let ranking_original_metrics = metrics_from_tree_absolute(&source_root)?;
        let candidate_metrics = metrics_from_tree_absolute(&verify_root)?;
        let scorer = SimplicityScorer::new(policy.weights.clone(), policy.limits.clone());
        let ranking_original_score = scorer.score(&ranking_original_metrics, original_floor);

        let isolator = isolator_for_session(&session, &paths.snapshots_dir());
        self.run_firebreak_isolated(
            config,
            run,
            IsolatedFirebreakParams {
                original_floor,
                isolator: isolator.as_ref(),
                session: &mut session,
                candidate_metrics,
                candidate_floor,
                authorize_apply: options.authorize_apply,
                ranking_original_metrics: Some(ranking_original_metrics),
                ranking_original_score: Some(ranking_original_score),
                user_approved: options.user_approved,
            },
        )
    }

    fn has_usable_reviewer_backend(&self, config: &Config) -> bool {
        if config.reviewers.is_empty() {
            return false;
        }
        let registry = BackendRegistry::new();
        config
            .reviewers
            .iter()
            .any(|r| backend_for_provider(&registry, &r.provider).is_ok())
    }

    /// Approve a pending Firebreak candidate and apply after re-check.
    pub fn approve_firebreak(&self, config: &Config, run: &mut RunRecord) -> Result<()> {
        if run.state != RunState::AwaitingApproval {
            return Err(TifError::InvalidTransition {
                from: run.state.as_str().into(),
                to: "approve".into(),
            });
        }
        if let Some(exp) = run.approval_expires_at {
            if Utc::now() > exp {
                run.events
                    .push("approval expired; rejecting pending firebreak".into());
                return self.reject_firebreak(run, Some("approval expired".into()));
            }
        }

        let Some(mut session) = run.isolation_session.clone() else {
            return Err(TifError::Isolation(
                "no isolation session on run; cannot approve firebreak".into(),
            ));
        };
        let floor = run
            .assessment
            .as_ref()
            .map(|a| a.correctness.clone())
            .unwrap_or_else(CorrectnessFloor::all_pass);
        if !floor.passes() {
            return Err(TifError::CorrectnessFloor(
                "original correctness floor no longer passes; refuse approve".into(),
            ));
        }

        let source_root = PathBuf::from(&run.repo_root);
        let paths = RepoPaths::for_root(&source_root);
        let verify_root = session_candidate_root(&session).to_path_buf();
        let inspection = RepositoryInspector::new().inspect(&verify_root)?;
        let report = plan_and_run(&verify_root, &config.verification, Some(&inspection), false)?;
        let candidate_floor = candidate_floor_from_verification(&report);
        if !candidate_floor.passes() {
            run.events
                .push("approve: candidate failed re-verify; not applying".into());
            return self.reject_firebreak(
                run,
                Some("candidate failed re-verify at approve time".into()),
            );
        }

        let policy = run
            .policy
            .clone()
            .ok_or_else(|| TifError::Other("run missing policy".into()))?;
        let ranking_original_metrics = metrics_from_tree_absolute(&source_root)?;
        let candidate_metrics = metrics_from_tree_absolute(&verify_root)?;
        let scorer = SimplicityScorer::new(policy.weights.clone(), policy.limits.clone());
        let ranking_original_score = scorer.score(&ranking_original_metrics, &floor);

        // Move AwaitingApproval → FirebreakRunning for isolated apply transitions.
        run.state = RunState::FirebreakRunning;
        run.updated_at = Utc::now();
        run.events
            .push("awaiting_approval -> firebreak_running (approve)".into());
        run.approval_expires_at = None;

        let isolator = isolator_for_session(&session, &paths.snapshots_dir());
        self.run_firebreak_isolated(
            config,
            run,
            IsolatedFirebreakParams {
                original_floor: &floor,
                isolator: isolator.as_ref(),
                session: &mut session,
                candidate_metrics,
                candidate_floor,
                authorize_apply: true,
                ranking_original_metrics: Some(ranking_original_metrics),
                ranking_original_score: Some(ranking_original_score),
                user_approved: true,
            },
        )
    }

    /// Reject a pending Firebreak candidate (original preserved).
    pub fn reject_firebreak(&self, run: &mut RunRecord, reason: Option<String>) -> Result<()> {
        if !matches!(
            run.state,
            RunState::AwaitingApproval | RunState::FirebreakRunning | RunState::CandidateComparing
        ) {
            return Err(TifError::InvalidTransition {
                from: run.state.as_str().into(),
                to: "reject".into(),
            });
        }
        let msg = reason.unwrap_or_else(|| "operator rejected firebreak candidate".into());
        run.events.push(format!("firebreak rejected: {msg}"));
        run.approval_expires_at = None;
        run.firebreak = Some(FirebreakOutcome {
            success: true,
            applied: false,
            candidate_ready: false,
            requires_approval: false,
            simulated: false,
            reviewer_id: run.firebreak.as_ref().and_then(|f| f.reviewer_id.clone()),
            candidate_id: run.firebreak.as_ref().and_then(|f| f.candidate_id.clone()),
            candidate_score: run
                .firebreak
                .as_ref()
                .and_then(|f| f.candidate_score.clone()),
            candidate_metrics: run
                .firebreak
                .as_ref()
                .and_then(|f| f.candidate_metrics.clone()),
            message: format!("Firebreak rejected; original retained ({msg})"),
            original_preserved: true,
            isolation_session_id: run.isolation_session.as_ref().map(|s| s.id.clone()),
        });
        run.rollback_candidate_id = Some("original".into());
        if run.state == RunState::AwaitingApproval
            || run.state == RunState::FirebreakRunning
            || run.state == RunState::CandidateComparing
        {
            // Prefer legal transition when possible.
            if run.state == RunState::AwaitingApproval {
                let _ = run.transition(RunState::Restored);
            } else if run.state == RunState::FirebreakRunning {
                let _ = run.transition(RunState::CandidateComparing);
                let _ = run.transition(RunState::Restored);
            } else {
                let _ = run.transition(RunState::Restored);
            }
        }
        if run.state != RunState::Restored {
            run.state = RunState::Restored;
            run.updated_at = Utc::now();
        }
        if let Some(ref mut a) = run.assessment {
            a.rollback_available = false;
            a.summary = format!("Firebreak rejected · {msg}");
            a.status = AssessmentStatus::OutOfControl;
        }
        Ok(())
    }

    /// Run Firebreak with a re-verified isolated candidate and optional apply.
    pub fn run_firebreak_isolated(
        &self,
        config: &Config,
        run: &mut RunRecord,
        params: IsolatedFirebreakParams<'_>,
    ) -> Result<()> {
        if run.state != RunState::FirebreakRunning {
            run.transition(RunState::FirebreakRunning)?;
        }

        let policy = run
            .policy
            .clone()
            .ok_or_else(|| TifError::Other("run missing policy".into()))?;
        let metrics = params
            .ranking_original_metrics
            .clone()
            .or_else(|| run.metrics.clone())
            .unwrap_or_default();
        let original_score = params
            .ranking_original_score
            .clone()
            .or_else(|| run.score.clone())
            .ok_or_else(|| TifError::Other("run missing score".into()))?;

        // Always attach session before apply so emergency rollback can find baseline
        // even if apply I/O fails (partial apply / dual-failure sticky flags).
        run.isolation_session = Some(params.session.clone());

        let selector = ReviewerSelector::new(config.reviewers.clone());
        let engine = FirebreakEngine::new(selector);
        let apply_result = engine.apply_isolated_candidate(
            crate::firebreak::IsolatedApplyRequest {
                run_id: run.id.as_str().to_string(),
                policy,
                original_metrics: metrics,
                original_score,
                original_floor: params.original_floor.clone(),
                candidate_metrics: params.candidate_metrics,
                candidate_floor: params.candidate_floor,
                authorize_apply: params.authorize_apply,
                force_approval: false,
                user_approved: params.user_approved,
            },
            params.isolator,
            params.session,
        );

        // Re-attach session after apply (baseline_path / applied / restore_pending).
        run.isolation_session = Some(params.session.clone());

        match apply_result {
            Ok(outcome) => self.finalize_firebreak(config, run, outcome),
            Err(e) => {
                run.events
                    .push(format!("isolated firebreak apply error: {e}"));
                // Session remains on the run (with applied/restore_pending sticky flags)
                // so `tif rollback` can retry. Prefer Failed over a false "restored".
                if run.state == RunState::FirebreakRunning {
                    run.fail(format!("isolated firebreak apply error: {e}"));
                }
                Err(e)
            }
        }
    }

    fn finalize_firebreak(
        &self,
        config: &Config,
        run: &mut RunRecord,
        outcome: FirebreakOutcome,
    ) -> Result<()> {
        run.firebreak = Some(outcome.clone());
        run.transition(RunState::CandidateComparing)?;

        if outcome.requires_approval && outcome.candidate_ready {
            run.transition(RunState::AwaitingApproval)?;
            run.approval_expires_at = config
                .approval
                .approval_ttl_hours
                .map(|h| Utc::now() + Duration::hours(i64::from(h).max(1)));
            if let Some(ref mut a) = run.assessment {
                a.status = AssessmentStatus::FirebreakPending;
                a.reviewer_id = outcome.reviewer_id.clone();
                a.summary = format!("Firebreak pending approval · {}", outcome.message);
            }
            return Ok(());
        }

        if outcome.applied {
            run.rollback_candidate_id = Some("original".into());
            run.transition(RunState::Applied)?;
            if let Some(ref mut a) = run.assessment {
                a.rollback_available = true;
                a.reviewer_id = outcome.reviewer_id.clone();
                a.candidate_id = outcome
                    .candidate_id
                    .clone()
                    .unwrap_or_else(|| "firebreak".into());
                a.summary = format!("Firebreak applied · {}", outcome.message);
                a.status = AssessmentStatus::Contained;
                if let Some(ref m) = outcome.candidate_metrics {
                    a.metrics = m.clone();
                }
                if let Some(ref s) = outcome.candidate_score {
                    a.score = s.clone();
                }
            }
            return Ok(());
        }

        // Fail-closed / not applied: preserve original.
        run.rollback_candidate_id = Some("original".into());
        run.transition(RunState::Restored)?;
        if let Some(ref mut a) = run.assessment {
            a.rollback_available = run.isolation_session.as_ref().is_some_and(|s| s.applied);
            a.summary = format!("Firebreak did not replace original · {}", outcome.message);
        }
        Ok(())
    }

    /// Restore the original workspace after a successful or partial Firebreak apply.
    pub fn rollback_isolation(
        &self,
        run: &mut RunRecord,
        isolator: &dyn crate::isolation::Isolator,
    ) -> Result<()> {
        let Some(mut session) = run.isolation_session.clone() else {
            return Err(TifError::Isolation(
                "no isolation session recorded for this run; cannot restore files".into(),
            ));
        };
        if !session.applied && !session.restore_pending {
            run.events
                .push("rollback: isolation session not applied (original already intact)".into());
            run.isolation_session = Some(session);
            return Ok(());
        }
        isolator.restore_source(&mut session)?;
        run.isolation_session = Some(session);
        run.rollback_candidate_id = Some("original".into());
        run.events
            .push("filesystem rollback restored original from baseline".into());
        if run.state == RunState::Applied || run.state == RunState::CandidateComparing {
            let _ = run.transition(RunState::Restored);
        } else {
            run.state = RunState::Restored;
            run.updated_at = Utc::now();
        }
        if let Some(ref mut a) = run.assessment {
            a.rollback_available = false;
            a.summary = "Rolled back to original implementation".into();
            a.candidate_id = "original".into();
        }
        Ok(())
    }

    pub fn close(&self, run: &mut RunRecord) -> Result<()> {
        if run.state == RunState::Closed {
            return Ok(());
        }
        // Allow close from terminal-ish states
        match run.state {
            RunState::Contained
            | RunState::Applied
            | RunState::Restored
            | RunState::Rejected
            | RunState::Failed
            | RunState::OutOfControl
            | RunState::AwaitingApproval => {
                run.events.push(format!("{} -> closed", run.state.as_str()));
                run.state = RunState::Closed;
                run.updated_at = Utc::now();
                Ok(())
            }
            other => Err(TifError::InvalidTransition {
                from: other.as_str().into(),
                to: "closed".into(),
            }),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeginRunRequest {
    pub task_text: Option<String>,
    pub task_category: Option<crate::task::TaskCategory>,
    pub fire_level: Option<FireLevel>,
    pub agent_id: Option<String>,
    pub model_id: Option<String>,
    pub force: bool,
}

/// Parameters for isolation-backed Firebreak apply.
pub struct IsolatedFirebreakParams<'a> {
    pub original_floor: &'a CorrectnessFloor,
    pub isolator: &'a dyn crate::isolation::Isolator,
    pub session: &'a mut IsolationSession,
    pub candidate_metrics: DiffMetrics,
    pub candidate_floor: CorrectnessFloor,
    pub authorize_apply: bool,
    /// When set (with [`Self::ranking_original_score`]), override run metrics for
    /// ranking so both sides use the same measurement kind (e.g. absolute tree weight).
    pub ranking_original_metrics: Option<DiffMetrics>,
    pub ranking_original_score: Option<ScoreResult>,
    /// Operator already approved (`tif approve`).
    pub user_approved: bool,
}

/// Options for the automatic Firebreak closed loop.
#[derive(Debug, Clone)]
pub struct FirebreakAutoOptions {
    /// Apply when candidate is ready and does not require approval.
    pub authorize_apply: bool,
    /// Operator already approved (skips approval gate).
    pub user_approved: bool,
}

impl Default for FirebreakAutoOptions {
    fn default() -> Self {
        Self {
            authorize_apply: true,
            user_approved: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::verify::{
        CheckResult, CheckSource, CheckStatus, VerificationCategory, VerificationCheck,
    };

    fn pass_report() -> VerificationReport {
        VerificationReport {
            checks: vec![CheckResult {
                check: VerificationCheck {
                    id: "t".into(),
                    category: VerificationCategory::UnitTest,
                    command: "true".into(),
                    source: CheckSource::ExplicitConfig,
                    required: true,
                    evidence: None,
                },
                status: CheckStatus::Passed,
                exit_code: Some(0),
                duration_ms: 1,
                stdout_tail: String::new(),
                stderr_tail: String::new(),
            }],
            all_required_passed: true,
            has_unresolved: false,
            has_unresolved_required: false,
            incomplete_plan: false,
        }
    }

    #[test]
    fn contained_happy_path() {
        let cfg = Config::default();
        let orch = RunOrchestrator::new();
        let mut run = orch
            .begin(
                &cfg,
                "/repo",
                BeginRunRequest {
                    task_text: Some("fix bug".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(run.state, RunState::Implementing);

        orch.complete(
            &cfg,
            &mut run,
            DiffMetrics {
                lines_added: 2,
                files_changed: 1,
                ..Default::default()
            },
            pass_report(),
            CorrectnessFloor::all_pass(),
            false,
        )
        .unwrap();
        assert_eq!(run.state, RunState::Contained);
    }

    #[test]
    fn out_of_control_triggers_firebreak() {
        let mut cfg = Config::default();
        cfg.simplicity.limits.new_runtime_dependencies = Some(0);
        // authorize a reviewer for firebreak
        cfg.reviewers
            .push(crate::config::ReviewerConfig::mock("r1", 1));

        let orch = RunOrchestrator::new();
        let mut run = orch
            .begin(
                &cfg,
                "/repo",
                BeginRunRequest {
                    task_text: Some("add feature".into()),
                    ..Default::default()
                },
            )
            .unwrap();

        orch.complete(
            &cfg,
            &mut run,
            DiffMetrics {
                runtime_dependencies_added: 1,
                lines_added: 50,
                files_added: 2,
                ..Default::default()
            },
            pass_report(),
            CorrectnessFloor::all_pass(),
            true,
        )
        .unwrap();

        // Production Firebreak is fail-closed without isolation/reviewer backend.
        assert_eq!(run.state, RunState::Restored);
        assert!(run.firebreak.is_some());
        assert!(!run.firebreak.as_ref().unwrap().applied);
        assert!(run.firebreak.as_ref().unwrap().original_preserved);
    }

    #[test]
    fn incomplete_verification_rejects() {
        let cfg = Config::default();
        let orch = RunOrchestrator::new();
        let mut run = orch
            .begin(
                &cfg,
                "/repo",
                BeginRunRequest {
                    task_text: Some("fix bug".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        orch.complete(
            &cfg,
            &mut run,
            DiffMetrics {
                lines_added: 2,
                ..Default::default()
            },
            VerificationReport::empty_incomplete(),
            CorrectnessFloor::all_pass(),
            false,
        )
        .unwrap();
        assert_eq!(run.state, RunState::Rejected);
        assert!(run.score.as_ref().unwrap().disqualified);
    }

    #[test]
    fn invalid_transition_errors() {
        let mut run = RunRecord::new("/r");
        assert!(run.transition(RunState::Closed).is_err());
    }

    #[test]
    fn begin_fails_when_disabled() {
        let cfg = Config {
            enabled: false,
            ..Config::default()
        };
        let run = RunOrchestrator::new()
            .begin(
                &cfg,
                "/repo",
                BeginRunRequest {
                    task_text: Some("x".into()),
                    force: false,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(run.state, RunState::Failed);
        assert!(run.error.is_some());
    }

    #[test]
    fn manual_firebreak_from_contained_is_legal() {
        assert!(RunState::can_transition(
            RunState::Contained,
            RunState::FirebreakRunning
        ));
    }

    /// Phase 2 helpers: temp repo with mock reviewer + always-pass verification.
    #[cfg(feature = "provider-mock")]
    mod phase2 {
        use super::*;
        use crate::config::{
            ensure_state_dirs, ApprovalConfig, ReviewerConfig, SimplicityConfig, SimplicityLimits,
            VerificationConfig,
        };
        use crate::isolation::isolator_for_session;
        use std::fs;
        use tempfile::tempdir;

        fn phase2_cfg(require_approval: bool) -> Config {
            let mut cfg = Config::default();
            cfg.verification = VerificationConfig {
                commands: vec!["echo tif-ok".into()],
                discover: false,
            };
            cfg.simplicity = SimplicityConfig {
                limits: SimplicityLimits {
                    new_runtime_dependencies: Some(0),
                    ..Default::default()
                },
                ..Default::default()
            };
            cfg.approval = ApprovalConfig {
                require_firebreak_approval: require_approval,
                auto_apply_firebreak: true,
                approval_ttl_hours: Some(24),
                ..Default::default()
            };
            cfg.reviewers.push(ReviewerConfig::mock("phase2-mock", 100));
            cfg
        }

        fn plant_repo(root: &std::path::Path, reduce_bytes: usize, fail: bool, inflate: bool) {
            fs::create_dir_all(root.join("src")).unwrap();
            fs::write(root.join("src/lib.rs"), b"pub fn f() -> i32 { 1 }\n").unwrap();
            if reduce_bytes > 0 {
                fs::write(root.join("TIF_MOCK_REDUCE"), vec![b'X'; reduce_bytes]).unwrap();
            }
            if fail {
                fs::write(root.join("TIF_MOCK_FAIL"), b"1").unwrap();
            }
            if inflate {
                // Inflate body is repeated 32x by the mock backend.
                fs::write(root.join("TIF_MOCK_INFLATE"), "BLOAT_LINE\n".repeat(200)).unwrap();
            }
            let paths = crate::config::RepoPaths::for_root(root);
            ensure_state_dirs(&paths).unwrap();
        }

        fn ooc_metrics() -> DiffMetrics {
            DiffMetrics {
                runtime_dependencies_added: 1,
                lines_added: 80,
                files_added: 2,
                ..Default::default()
            }
        }

        #[test]
        fn e2e_mock_oversized_generate_reverify_apply_rollback() {
            let dir = tempdir().unwrap();
            let root = dir.path().join("repo");
            plant_repo(&root, 50_000, false, false);
            let keep = root.join("KEEP.txt");
            fs::write(&keep, b"original-keep").unwrap();

            let cfg = phase2_cfg(false);
            let orch = RunOrchestrator::new();
            let mut run = orch
                .begin(
                    &cfg,
                    root.to_str().unwrap(),
                    BeginRunRequest {
                        task_text: Some("shrink feature".into()),
                        ..Default::default()
                    },
                )
                .unwrap();

            orch.complete(
                &cfg,
                &mut run,
                ooc_metrics(),
                pass_report(),
                CorrectnessFloor::all_pass(),
                true, // auto firebreak closed loop
            )
            .unwrap();

            assert!(
                run.firebreak.as_ref().is_some_and(|f| f.applied),
                "expected apply, got: {:?}",
                run.firebreak
            );
            assert_eq!(run.state, RunState::Applied);
            assert!(!root.join("TIF_MOCK_REDUCE").exists());
            assert_eq!(fs::read_to_string(&keep).unwrap(), "original-keep");

            // Rollback restores bloat marker from baseline.
            let paths = crate::config::RepoPaths::for_root(&root);
            let session = run.isolation_session.clone().expect("session");
            let isolator = isolator_for_session(&session, &paths.snapshots_dir());
            orch.rollback_isolation(&mut run, isolator.as_ref())
                .unwrap();
            assert_eq!(run.state, RunState::Restored);
            assert!(root.join("TIF_MOCK_REDUCE").exists());
            assert_eq!(fs::read_to_string(&keep).unwrap(), "original-keep");
        }

        #[test]
        fn failed_backend_preserves_source() {
            let dir = tempdir().unwrap();
            let root = dir.path().join("repo");
            plant_repo(&root, 10_000, true, false);
            fs::write(root.join("src/lib.rs"), b"pub fn f() -> i32 { 42 }\n").unwrap();

            let cfg = phase2_cfg(false);
            let orch = RunOrchestrator::new();
            let mut run = orch
                .begin(
                    &cfg,
                    root.to_str().unwrap(),
                    BeginRunRequest {
                        task_text: Some("fail path".into()),
                        ..Default::default()
                    },
                )
                .unwrap();

            orch.complete(
                &cfg,
                &mut run,
                ooc_metrics(),
                pass_report(),
                CorrectnessFloor::all_pass(),
                true,
            )
            .unwrap();

            assert!(!run.firebreak.as_ref().unwrap().applied);
            assert!(run.firebreak.as_ref().unwrap().original_preserved);
            assert_eq!(run.state, RunState::Restored);
            assert_eq!(
                fs::read_to_string(root.join("src/lib.rs")).unwrap(),
                "pub fn f() -> i32 { 42 }\n"
            );
            assert!(root.join("TIF_MOCK_REDUCE").exists());
        }

        #[test]
        fn larger_candidate_not_applied() {
            let dir = tempdir().unwrap();
            let root = dir.path().join("repo");
            // No REDUCE delete; INFLATE adds a large file → larger candidate.
            plant_repo(&root, 0, false, true);
            fs::write(root.join("src/lib.rs"), b"pub fn f() {}\n").unwrap();

            let cfg = phase2_cfg(false);
            let orch = RunOrchestrator::new();
            let mut run = orch
                .begin(
                    &cfg,
                    root.to_str().unwrap(),
                    BeginRunRequest {
                        task_text: Some("inflate path".into()),
                        ..Default::default()
                    },
                )
                .unwrap();

            orch.complete(
                &cfg,
                &mut run,
                ooc_metrics(),
                pass_report(),
                CorrectnessFloor::all_pass(),
                true,
            )
            .unwrap();

            let fb = run.firebreak.as_ref().unwrap();
            assert!(
                !fb.applied,
                "larger candidate must not apply: {}",
                fb.message
            );
            assert!(fb.original_preserved);
            assert!(!root.join("TIF_MOCK_BLOAT.txt").exists());
            assert_eq!(run.state, RunState::Restored);
        }

        #[test]
        fn approval_required_does_not_auto_apply() {
            let dir = tempdir().unwrap();
            let root = dir.path().join("repo");
            plant_repo(&root, 40_000, false, false);

            let cfg = phase2_cfg(true);
            let orch = RunOrchestrator::new();
            let mut run = orch
                .begin(
                    &cfg,
                    root.to_str().unwrap(),
                    BeginRunRequest {
                        task_text: Some("needs approval".into()),
                        ..Default::default()
                    },
                )
                .unwrap();

            orch.complete(
                &cfg,
                &mut run,
                ooc_metrics(),
                pass_report(),
                CorrectnessFloor::all_pass(),
                true,
            )
            .unwrap();

            let fb = run.firebreak.as_ref().unwrap();
            assert!(fb.candidate_ready);
            assert!(fb.requires_approval);
            assert!(!fb.applied);
            assert!(fb.original_preserved);
            assert_eq!(run.state, RunState::AwaitingApproval);
            assert!(run.approval_expires_at.is_some());
            assert!(root.join("TIF_MOCK_REDUCE").exists());

            // Reject path leaves source intact.
            orch.reject_firebreak(&mut run, Some("nope".into()))
                .unwrap();
            assert_eq!(run.state, RunState::Restored);
            assert!(root.join("TIF_MOCK_REDUCE").exists());
        }

        #[test]
        fn approve_applies_pending_candidate() {
            let dir = tempdir().unwrap();
            let root = dir.path().join("repo");
            plant_repo(&root, 40_000, false, false);

            let cfg = phase2_cfg(true);
            let orch = RunOrchestrator::new();
            let mut run = orch
                .begin(
                    &cfg,
                    root.to_str().unwrap(),
                    BeginRunRequest {
                        task_text: Some("approve me".into()),
                        ..Default::default()
                    },
                )
                .unwrap();

            orch.complete(
                &cfg,
                &mut run,
                ooc_metrics(),
                pass_report(),
                CorrectnessFloor::all_pass(),
                true,
            )
            .unwrap();
            assert_eq!(run.state, RunState::AwaitingApproval);

            orch.approve_firebreak(&cfg, &mut run).unwrap();
            assert!(
                run.firebreak.as_ref().is_some_and(|f| f.applied),
                "approve should apply: {:?}",
                run.firebreak
            );
            assert_eq!(run.state, RunState::Applied);
            assert!(!root.join("TIF_MOCK_REDUCE").exists());
        }
    }
}

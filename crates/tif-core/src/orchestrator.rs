//! Run orchestrator and state machine with stable run IDs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::assess::{AssessmentStatus, DamageAssessment, DamageAssessor};
use crate::config::Config;
use crate::error::{Result, TifError};
use crate::fire_level::FireLevel;
use crate::firebreak::{FirebreakEngine, FirebreakOutcome, FirebreakRequest};
use crate::policy::{ContainmentPolicy, PolicyCompileRequest, PolicyCompiler};
use crate::reviewer::ReviewerSelector;
use crate::scoring::{CorrectnessFloor, DiffMetrics, ScoreResult, SimplicityScorer};
use crate::verify::VerificationReport;

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
    pub fn run_firebreak(
        &self,
        config: &Config,
        run: &mut RunRecord,
        original_floor: &CorrectnessFloor,
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

        let selector = ReviewerSelector::new(config.reviewers.clone());
        let engine = FirebreakEngine::new(selector);
        // Production: fail-closed. Simulation is for unit tests only via FirebreakEngine directly.
        let outcome = engine.simplify(FirebreakRequest {
            run_id: run.id.as_str().to_string(),
            policy,
            original_metrics: metrics,
            original_score,
            original_floor: original_floor.clone(),
            simulate_success: false,
            workspace_apply_authorized: false,
        })?;

        run.firebreak = Some(outcome.clone());
        run.transition(RunState::CandidateComparing)?;

        if outcome.requires_approval && outcome.candidate_ready {
            run.transition(RunState::AwaitingApproval)?;
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
            }
            return Ok(());
        }

        // Fail-closed / not applied: preserve original.
        run.rollback_candidate_id = Some("original".into());
        run.transition(RunState::Restored)?;
        if let Some(ref mut a) = run.assessment {
            a.rollback_available = false;
            a.summary = format!("Firebreak did not replace original · {}", outcome.message);
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
        cfg.reviewers.push(crate::config::ReviewerConfig {
            id: "r1".into(),
            provider: "mock".into(),
            model: "mock-model".into(),
            endpoint: None,
            credential_ref: None,
            allow_source_egress: false,
            eligible_task_types: vec![],
            max_firebreak_attempts: 2,
            priority: 1,
        });

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
}

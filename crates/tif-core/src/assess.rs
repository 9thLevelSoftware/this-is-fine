//! Damage Assessment: diff and policy report.

use serde::{Deserialize, Serialize};

use crate::fire_level::FireLevel;
use crate::scoring::{CorrectnessFloor, DiffMetrics, ScoreResult};
use crate::verify::VerificationReport;

/// Full Damage Assessment report for a run/candidate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DamageAssessment {
    pub run_id: String,
    pub candidate_id: String,
    pub fire_level: FireLevel,
    pub metrics: DiffMetrics,
    pub score: ScoreResult,
    pub correctness: CorrectnessFloor,
    pub verification: Option<VerificationReport>,
    pub files_added: Vec<String>,
    pub files_changed: Vec<String>,
    pub files_deleted: Vec<String>,
    pub files_renamed: Vec<(String, String)>,
    pub dependencies_added: Vec<String>,
    pub dependencies_removed: Vec<String>,
    pub public_apis_changed: Vec<String>,
    pub abstractions_detected: Vec<String>,
    pub tests_changed: Vec<TestChange>,
    pub pressure_template_id: Option<String>,
    pub policy_version: Option<String>,
    pub reviewer_id: Option<String>,
    pub rollback_available: bool,
    pub status: AssessmentStatus,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentStatus {
    /// Successful result within containment.
    Contained,
    /// Correct but exceeds containment (Fuel Added).
    OutOfControl,
    /// Correctness floor failed.
    Rejected,
    /// Awaiting verification.
    Unverified,
    /// Firebreak in progress or pending.
    FirebreakPending,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestChange {
    pub path: String,
    pub reason: String,
}

/// Builder for Damage Assessment from components.
pub struct DamageAssessor;

impl DamageAssessor {
    pub fn build(
        run_id: impl Into<String>,
        candidate_id: impl Into<String>,
        fire_level: FireLevel,
        metrics: DiffMetrics,
        score: ScoreResult,
        correctness: CorrectnessFloor,
        verification: Option<VerificationReport>,
    ) -> DamageAssessment {
        let status = if !correctness.passes() || score.disqualified {
            AssessmentStatus::Rejected
        } else if score.within_containment {
            AssessmentStatus::Contained
        } else {
            AssessmentStatus::OutOfControl
        };

        let summary = match status {
            AssessmentStatus::Contained => format!(
                "Contained · score {:.1} · Fire Level {}",
                score.score,
                fire_level.as_u8()
            ),
            AssessmentStatus::OutOfControl => format!(
                "Out of Control · score {:.1} · violations: {} · Fire Level {}",
                score.score,
                score.hard_limit_violations.len(),
                fire_level.as_u8()
            ),
            AssessmentStatus::Rejected => format!(
                "Rejected · correctness floor failed · Fire Level {}",
                fire_level.as_u8()
            ),
            AssessmentStatus::Unverified => "Unverified".into(),
            AssessmentStatus::FirebreakPending => "Firebreak pending".into(),
        };

        DamageAssessment {
            run_id: run_id.into(),
            candidate_id: candidate_id.into(),
            fire_level,
            metrics,
            score,
            correctness,
            verification,
            files_added: Vec::new(),
            files_changed: Vec::new(),
            files_deleted: Vec::new(),
            files_renamed: Vec::new(),
            dependencies_added: Vec::new(),
            dependencies_removed: Vec::new(),
            public_apis_changed: Vec::new(),
            abstractions_detected: Vec::new(),
            tests_changed: Vec::new(),
            pressure_template_id: None,
            policy_version: None,
            reviewer_id: None,
            rollback_available: false,
            status,
            summary,
        }
    }

    /// Parse a unified-diff style summary into rough metrics (MVP heuristic).
    pub fn metrics_from_diff_stats(
        files_added: u32,
        files_changed: u32,
        files_deleted: u32,
        lines_added: u32,
        lines_removed: u32,
        runtime_deps_added: u32,
    ) -> DiffMetrics {
        DiffMetrics {
            runtime_dependencies_added: runtime_deps_added,
            files_added,
            files_changed,
            files_deleted,
            public_interfaces_added: 0,
            abstractions_added: 0,
            lines_added,
            lines_removed,
            configuration_surface_added: 0,
            generated_code_lines: 0,
            duplication_indicators: 0,
            unrelated_changes: 0,
            tests_changed: 0,
            changed_paths: Vec::new(),
        }
    }

    /// Enrich a damage assessment with path lists derived from metrics.
    pub fn attach_paths(
        mut assessment: DamageAssessment,
        files_added: Vec<String>,
        files_changed: Vec<String>,
        files_deleted: Vec<String>,
    ) -> DamageAssessment {
        assessment.files_added = files_added;
        assessment.files_changed = files_changed;
        assessment.files_deleted = files_deleted;
        if assessment.metrics.changed_paths.is_empty() {
            let mut paths = Vec::new();
            paths.extend(assessment.files_added.iter().cloned());
            paths.extend(assessment.files_changed.iter().cloned());
            paths.extend(assessment.files_deleted.iter().cloned());
            assessment.metrics.changed_paths = paths;
        }
        assessment
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{SimplicityLimits, SimplicityWeights};
    use crate::scoring::SimplicityScorer;

    #[test]
    fn contained_when_within_limits() {
        let scorer =
            SimplicityScorer::new(SimplicityWeights::default(), SimplicityLimits::default());
        let metrics = DiffMetrics {
            lines_added: 3,
            files_changed: 1,
            ..Default::default()
        };
        let floor = CorrectnessFloor::all_pass();
        let score = scorer.score(&metrics, &floor);
        let da = DamageAssessor::build(
            "run1",
            "c1",
            FireLevel::Containment,
            metrics,
            score,
            floor,
            None,
        );
        assert_eq!(da.status, AssessmentStatus::Contained);
    }

    #[test]
    fn out_of_control_on_hard_limit() {
        let scorer = SimplicityScorer::new(
            SimplicityWeights::default(),
            SimplicityLimits {
                new_runtime_dependencies: Some(0),
                ..Default::default()
            },
        );
        let metrics = DiffMetrics {
            runtime_dependencies_added: 2,
            lines_added: 10,
            ..Default::default()
        };
        let floor = CorrectnessFloor::all_pass();
        let score = scorer.score(&metrics, &floor);
        let da = DamageAssessor::build(
            "run1",
            "c1",
            FireLevel::Critical,
            metrics,
            score,
            floor,
            None,
        );
        assert_eq!(da.status, AssessmentStatus::OutOfControl);
    }
}

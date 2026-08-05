//! Simplicity scorer with weights, hard limits, and correctness-floor gate.

use serde::{Deserialize, Serialize};

use crate::config::{SimplicityLimits, SimplicityWeights};
use crate::error::{Result, TifError};

/// Bump when Damage Assessment weight semantics change in a way that breaks
/// adaptation comparability across runs.
pub const SCORING_VERSION: &str = "1";

/// Measurable diff metrics used for scoring.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DiffMetrics {
    pub runtime_dependencies_added: u32,
    pub files_added: u32,
    pub files_changed: u32,
    pub files_deleted: u32,
    pub public_interfaces_added: u32,
    pub abstractions_added: u32,
    pub lines_added: u32,
    pub lines_removed: u32,
    pub configuration_surface_added: u32,
    pub generated_code_lines: u32,
    pub duplication_indicators: u32,
    pub unrelated_changes: u32,
    pub tests_changed: u32,
    /// Paths touched by the candidate (used for sensitive-path approval).
    #[serde(default)]
    pub changed_paths: Vec<String>,
}

/// Correctness-floor evaluation (gate, not a weighted score).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CorrectnessFloor {
    pub acceptance_criteria_met: bool,
    pub no_behavior_regression: bool,
    pub security_boundaries_intact: bool,
    pub validation_preserved: bool,
    pub verification_passed: bool,
    pub existing_tests_pass: bool,
    pub required_new_tests_present: bool,
    pub notes: Vec<String>,
}

impl CorrectnessFloor {
    /// All-pass floor for verified candidates.
    pub fn all_pass() -> Self {
        Self {
            acceptance_criteria_met: true,
            no_behavior_regression: true,
            security_boundaries_intact: true,
            validation_preserved: true,
            verification_passed: true,
            existing_tests_pass: true,
            required_new_tests_present: true,
            notes: Vec::new(),
        }
    }

    pub fn passes(&self) -> bool {
        self.acceptance_criteria_met
            && self.no_behavior_regression
            && self.security_boundaries_intact
            && self.validation_preserved
            && self.verification_passed
            && self.existing_tests_pass
            && self.required_new_tests_present
    }

    pub fn failure_reasons(&self) -> Vec<String> {
        let mut reasons = Vec::new();
        if !self.acceptance_criteria_met {
            reasons.push("acceptance criteria not satisfied".into());
        }
        if !self.no_behavior_regression {
            reasons.push("existing required behavior regressed".into());
        }
        if !self.security_boundaries_intact {
            reasons.push("security boundaries weakened".into());
        }
        if !self.validation_preserved {
            reasons.push("required validation or error handling removed".into());
        }
        if !self.verification_passed {
            reasons.push("required repository verification failed".into());
        }
        if !self.existing_tests_pass {
            reasons.push("existing tests failed due to candidate".into());
        }
        if !self.required_new_tests_present {
            reasons.push("new tests needed to prove changed behavior are absent".into());
        }
        reasons.extend(self.notes.iter().cloned());
        reasons
    }
}

/// Result of simplicity evaluation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScoreResult {
    /// Whether the candidate cleared the correctness floor.
    pub correctness_passed: bool,
    /// Disqualified before scoring when floor fails.
    pub disqualified: bool,
    pub score: f64,
    /// Hard containment limit breaches (not correctness-floor reasons).
    pub hard_limit_violations: Vec<String>,
    /// Correctness-floor failure reasons when disqualified.
    #[serde(default)]
    pub floor_failures: Vec<String>,
    pub within_containment: bool,
    pub breakdown: ScoreBreakdown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ScoreBreakdown {
    pub runtime_dependency: f64,
    pub new_file: f64,
    pub public_interface: f64,
    pub abstraction: f64,
    pub added_line: f64,
    pub unrelated_change: f64,
    pub configuration_surface: f64,
    pub generated_code: f64,
    pub duplication: f64,
}

/// Repository-specific simplicity scorer.
#[derive(Debug, Clone)]
pub struct SimplicityScorer {
    pub weights: SimplicityWeights,
    pub limits: SimplicityLimits,
}

impl SimplicityScorer {
    pub fn new(weights: SimplicityWeights, limits: SimplicityLimits) -> Self {
        Self { weights, limits }
    }

    /// Score a candidate. Correctness floor is a gate: failed floor ⇒ disqualified.
    pub fn score(&self, metrics: &DiffMetrics, floor: &CorrectnessFloor) -> ScoreResult {
        if !floor.passes() {
            return ScoreResult {
                correctness_passed: false,
                disqualified: true,
                score: f64::INFINITY,
                hard_limit_violations: Vec::new(),
                floor_failures: floor.failure_reasons(),
                within_containment: false,
                breakdown: ScoreBreakdown::default(),
            };
        }

        let breakdown = ScoreBreakdown {
            runtime_dependency: self.weights.runtime_dependency
                * metrics.runtime_dependencies_added as f64,
            new_file: self.weights.new_file * metrics.files_added as f64,
            public_interface: self.weights.public_interface
                * metrics.public_interfaces_added as f64,
            abstraction: self.weights.abstraction * metrics.abstractions_added as f64,
            added_line: self.weights.added_line * metrics.lines_added as f64,
            unrelated_change: self.weights.unrelated_change * metrics.unrelated_changes as f64,
            configuration_surface: self.weights.configuration_surface
                * metrics.configuration_surface_added as f64,
            generated_code: self.weights.generated_code * metrics.generated_code_lines as f64,
            duplication: self.weights.duplication * metrics.duplication_indicators as f64,
        };

        let score = breakdown.runtime_dependency
            + breakdown.new_file
            + breakdown.public_interface
            + breakdown.abstraction
            + breakdown.added_line
            + breakdown.unrelated_change
            + breakdown.configuration_surface
            + breakdown.generated_code
            + breakdown.duplication;

        let violations = self.check_hard_limits(metrics, score);
        let within = violations.is_empty();

        ScoreResult {
            correctness_passed: true,
            disqualified: false,
            score,
            hard_limit_violations: violations,
            floor_failures: Vec::new(),
            within_containment: within,
            breakdown,
        }
    }

    fn check_hard_limits(&self, metrics: &DiffMetrics, score: f64) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(max) = self.limits.new_runtime_dependencies {
            if metrics.runtime_dependencies_added > max {
                v.push(format!(
                    "new_runtime_dependencies {} > limit {max}",
                    metrics.runtime_dependencies_added
                ));
            }
        }
        if let Some(max) = self.limits.new_files {
            if metrics.files_added > max {
                v.push(format!("new_files {} > limit {max}", metrics.files_added));
            }
        }
        if let Some(max) = self.limits.public_interfaces {
            if metrics.public_interfaces_added > max {
                v.push(format!(
                    "public_interfaces {} > limit {max}",
                    metrics.public_interfaces_added
                ));
            }
        }
        if let Some(max) = self.limits.abstractions {
            if metrics.abstractions_added > max {
                v.push(format!(
                    "abstractions {} > limit {max}",
                    metrics.abstractions_added
                ));
            }
        }
        if let Some(max) = self.limits.added_lines {
            if metrics.lines_added > max {
                v.push(format!("added_lines {} > limit {max}", metrics.lines_added));
            }
        }
        if let Some(max) = self.limits.score {
            if score > max {
                v.push(format!("score {score} > limit {max}"));
            }
        }
        v
    }

    /// Rank candidates: correctness first, then lower score wins.
    /// A smaller incorrect candidate can never defeat a larger correct candidate.
    /// When both are disqualified, order is stable (equal) — do not rank by crafted scores.
    pub fn rank_candidates(results: &[(String, ScoreResult)]) -> Vec<String> {
        let mut scored: Vec<_> = results.iter().enumerate().collect();
        scored.sort_by(
            |(ia, a), (ib, b)| match (a.1.disqualified, b.1.disqualified) {
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                (true, true) => ia.cmp(ib),
                (false, false) => {
                    a.1.score
                        .partial_cmp(&b.1.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| ia.cmp(ib))
                }
            },
        );
        scored.into_iter().map(|(_, (id, _))| id.clone()).collect()
    }

    /// Ensure adaptation cannot lower the correctness floor (API marker).
    pub fn assert_floor_immutable(_floor: &CorrectnessFloor) -> Result<()> {
        // Correctness floor is never tunable by adaptation; this function documents the invariant.
        Ok(())
    }
}

/// Compare two verified candidates; prefer smaller (lower score) if both pass floor.
pub fn select_smaller_verified(
    original_id: &str,
    original: &ScoreResult,
    candidate_id: &str,
    candidate: &ScoreResult,
) -> Result<String> {
    if original.disqualified {
        return Err(TifError::CorrectnessFloor(
            "original implementation failed correctness floor".into(),
        ));
    }
    if candidate.disqualified {
        // Fail-safe: keep original
        return Ok(original_id.to_string());
    }
    if candidate.score < original.score {
        Ok(candidate_id.to_string())
    } else {
        Ok(original_id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{SimplicityLimits, SimplicityWeights};

    fn scorer() -> SimplicityScorer {
        SimplicityScorer::new(
            SimplicityWeights::default(),
            SimplicityLimits {
                new_runtime_dependencies: Some(0),
                ..Default::default()
            },
        )
    }

    #[test]
    fn correctness_floor_disqualifies_before_score() {
        let mut floor = CorrectnessFloor::all_pass();
        floor.verification_passed = false;
        let metrics = DiffMetrics {
            lines_added: 1,
            ..Default::default()
        };
        let result = scorer().score(&metrics, &floor);
        assert!(result.disqualified);
        assert!(!result.correctness_passed);
        assert!(result.score.is_infinite());
    }

    #[test]
    fn hard_limit_on_deps() {
        let metrics = DiffMetrics {
            runtime_dependencies_added: 1,
            lines_added: 5,
            ..Default::default()
        };
        let result = scorer().score(&metrics, &CorrectnessFloor::all_pass());
        assert!(!result.within_containment);
        assert!(!result.hard_limit_violations.is_empty());
        assert!(result.score >= 100.0);
    }

    fn ok_score(score: f64, within: bool) -> ScoreResult {
        ScoreResult {
            correctness_passed: true,
            disqualified: false,
            score,
            hard_limit_violations: vec![],
            floor_failures: vec![],
            within_containment: within,
            breakdown: ScoreBreakdown::default(),
        }
    }

    #[test]
    fn smaller_incorrect_never_beats_larger_correct() {
        let good = ok_score(200.0, false);
        let bad = ScoreResult {
            correctness_passed: false,
            disqualified: true,
            score: f64::INFINITY,
            hard_limit_violations: vec![],
            floor_failures: vec!["verification failed".into()],
            within_containment: false,
            breakdown: ScoreBreakdown::default(),
        };
        let ranking = SimplicityScorer::rank_candidates(&[
            ("small-bad".into(), bad),
            ("large-good".into(), good),
        ]);
        assert_eq!(ranking[0], "large-good");
    }

    #[test]
    fn floor_failures_not_in_hard_limits() {
        let mut floor = CorrectnessFloor::all_pass();
        floor.verification_passed = false;
        let result = scorer().score(&DiffMetrics::default(), &floor);
        assert!(result.disqualified);
        assert!(result.hard_limit_violations.is_empty());
        assert!(!result.floor_failures.is_empty());
    }

    #[test]
    fn select_prefers_smaller_verified() {
        let winner =
            select_smaller_verified("orig", &ok_score(100.0, false), "fb", &ok_score(40.0, true))
                .unwrap();
        assert_eq!(winner, "fb");
    }

    #[test]
    fn failed_firebreak_keeps_original() {
        let candidate = ScoreResult {
            correctness_passed: false,
            disqualified: true,
            score: f64::INFINITY,
            hard_limit_violations: vec![],
            floor_failures: vec!["verification failed".into()],
            within_containment: false,
            breakdown: ScoreBreakdown::default(),
        };
        let winner =
            select_smaller_verified("orig", &ok_score(100.0, false), "fb", &candidate).unwrap();
        assert_eq!(winner, "orig");
    }
}

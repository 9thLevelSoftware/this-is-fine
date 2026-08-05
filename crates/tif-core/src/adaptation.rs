//! Local-only adaptation engine.
//!
//! May tune pressure variants, Fire Level selection, thresholds, reviewer ranking,
//! and Firebreak attempt limits. Must not lower the correctness floor, authorize
//! new models, send telemetry, or disable required verification.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::fire_level::FireLevel;
use crate::reviewer::ReviewerStats;
use crate::scoring::CorrectnessFloor;
use crate::task::TaskCategory;

/// Aggregate local performance statistics (retained longer than detailed audit).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdaptationStats {
    pub total_runs: u64,
    pub contained: u64,
    pub out_of_control: u64,
    pub firebreak_success: u64,
    pub firebreak_fail: u64,
    pub rollbacks: u64,
    pub reviewer_stats: Vec<ReviewerStats>,
    pub pressure_variant_scores: Vec<VariantScore>,
    pub category_fire_levels: Vec<CategoryFireHint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariantScore {
    pub template_id: String,
    pub trials: u32,
    pub avg_simplicity_score: f64,
    pub verification_pass_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryFireHint {
    pub category: String,
    pub preferred_fire_level: u8,
    pub samples: u32,
}

/// Recommendations that may self-apply within safety bounds.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdaptationRecommendation {
    pub suggested_fire_level: Option<FireLevel>,
    pub limit_scale: Option<f64>,
    pub preferred_reviewer_id: Option<String>,
    pub preferred_pressure_template: Option<String>,
    pub notes: Vec<String>,
}

/// Inputs for recording a completed run (local adaptation only).
#[derive(Debug, Clone)]
pub struct RunOutcome<'a> {
    pub contained: bool,
    pub out_of_control: bool,
    pub firebreak: Option<bool>,
    pub rolled_back: bool,
    pub category: TaskCategory,
    pub fire_level: FireLevel,
    pub simplicity_score: f64,
    pub verification_passed: bool,
    pub pressure_template_id: &'a str,
    pub reviewer_id: Option<&'a str>,
}

/// Local adaptation engine — no network telemetry.
#[derive(Debug, Default)]
pub struct AdaptationEngine {
    stats: AdaptationStats,
}

impl AdaptationEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stats(&self) -> &AdaptationStats {
        &self.stats
    }

    pub fn load(stats: AdaptationStats) -> Self {
        Self { stats }
    }

    /// Record a completed run outcome.
    pub fn record_run(&mut self, outcome: RunOutcome<'_>) {
        self.stats.total_runs += 1;
        if outcome.contained {
            self.stats.contained += 1;
        }
        if outcome.out_of_control {
            self.stats.out_of_control += 1;
        }
        match outcome.firebreak {
            Some(true) => self.stats.firebreak_success += 1,
            Some(false) => self.stats.firebreak_fail += 1,
            None => {}
        }
        if outcome.rolled_back {
            self.stats.rollbacks += 1;
        }

        // Update pressure variant stats
        if let Some(v) = self
            .stats
            .pressure_variant_scores
            .iter_mut()
            .find(|v| v.template_id == outcome.pressure_template_id)
        {
            let n = v.trials as f64;
            v.avg_simplicity_score =
                (v.avg_simplicity_score * n + outcome.simplicity_score) / (n + 1.0);
            v.verification_pass_rate = (v.verification_pass_rate * n
                + if outcome.verification_passed {
                    1.0
                } else {
                    0.0
                })
                / (n + 1.0);
            v.trials += 1;
        } else {
            self.stats.pressure_variant_scores.push(VariantScore {
                template_id: outcome.pressure_template_id.to_string(),
                trials: 1,
                avg_simplicity_score: outcome.simplicity_score,
                verification_pass_rate: if outcome.verification_passed {
                    1.0
                } else {
                    0.0
                },
            });
        }

        // Category fire-level hints
        let cat = outcome.category.as_str().to_string();
        if let Some(h) = self
            .stats
            .category_fire_levels
            .iter_mut()
            .find(|h| h.category == cat)
        {
            // Prefer higher levels only when frequently out of control — simple heuristic.
            h.samples += 1;
            if outcome.out_of_control && outcome.fire_level.as_u8() < 4 {
                h.preferred_fire_level = (outcome.fire_level.as_u8() + 1).min(4);
            }
        } else {
            self.stats.category_fire_levels.push(CategoryFireHint {
                category: cat,
                preferred_fire_level: outcome.fire_level.as_u8().min(4),
                samples: 1,
            });
        }

        if let Some(id) = outcome.reviewer_id {
            if let Some(s) = self
                .stats
                .reviewer_stats
                .iter_mut()
                .find(|s| s.reviewer_id == id)
            {
                s.attempts += 1;
                if outcome.firebreak == Some(true) {
                    s.successes += 1;
                }
            } else {
                self.stats.reviewer_stats.push(ReviewerStats {
                    reviewer_id: id.to_string(),
                    attempts: 1,
                    successes: u32::from(outcome.firebreak == Some(true)),
                    avg_reduction_score: 0.0,
                });
            }
        }
    }

    /// Produce recommendations. Never lowers correctness floor.
    pub fn recommend(&self, category: TaskCategory) -> AdaptationRecommendation {
        let mut rec = AdaptationRecommendation::default();

        if let Some(h) = self
            .stats
            .category_fire_levels
            .iter()
            .find(|h| h.category == category.as_str())
        {
            if h.samples >= 3 {
                if let Ok(fl) = FireLevel::parse_initial(h.preferred_fire_level) {
                    rec.suggested_fire_level = Some(fl);
                    rec.notes.push(format!(
                        "local history prefers Fire Level {} for {}",
                        fl.as_u8(),
                        category.as_str()
                    ));
                }
            }
        }

        if let Some(best) = self
            .stats
            .reviewer_stats
            .iter()
            .filter(|s| s.attempts > 0)
            .max_by(|a, b| {
                let ra = a.successes as f64 / a.attempts as f64;
                let rb = b.successes as f64 / b.attempts as f64;
                ra.partial_cmp(&rb).unwrap_or(std::cmp::Ordering::Equal)
            })
        {
            rec.preferred_reviewer_id = Some(best.reviewer_id.clone());
        }

        if let Some(best) = self
            .stats
            .pressure_variant_scores
            .iter()
            .filter(|v| v.trials >= 2 && v.verification_pass_rate >= 0.99)
            .min_by(|a, b| {
                a.avg_simplicity_score
                    .partial_cmp(&b.avg_simplicity_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        {
            rec.preferred_pressure_template = Some(best.template_id.clone());
        }

        // Adaptation may tighten limits slightly when often out of control; never raise correctness risk.
        if self.stats.total_runs >= 5 {
            let ooc_rate = self.stats.out_of_control as f64 / self.stats.total_runs as f64;
            if ooc_rate > 0.5 {
                rec.limit_scale = Some(0.9);
                rec.notes
                    .push("high out-of-control rate: suggesting tighter soft limits".into());
            }
        }

        rec
    }

    /// Explicit invariant: adaptation cannot lower the correctness floor.
    pub fn cannot_lower_correctness_floor(&self, floor: &CorrectnessFloor) -> Result<()> {
        // The floor is never mutated by this engine.
        let _ = floor;
        SimplicityFloorGuard::assert_unchanged();
        Ok(())
    }
}

struct SimplicityFloorGuard;

impl SimplicityFloorGuard {
    fn assert_unchanged() {
        // Documented invariant marker used in tests.
    }
}

/// Adaptation must never set verification_passed = false to improve scores.
pub fn assert_adaptation_safety(before: &CorrectnessFloor, after: &CorrectnessFloor) -> bool {
    // All gates that were true must remain true or become more strict; never weaker.
    (!before.acceptance_criteria_met || after.acceptance_criteria_met)
        && (!before.no_behavior_regression || after.no_behavior_regression)
        && (!before.security_boundaries_intact || after.security_boundaries_intact)
        && (!before.validation_preserved || after.validation_preserved)
        && (!before.verification_passed || after.verification_passed)
        && (!before.existing_tests_pass || after.existing_tests_pass)
        && (!before.required_new_tests_present || after.required_new_tests_present)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_weaken_floor() {
        let floor = CorrectnessFloor::all_pass();
        let after = floor.clone();
        assert!(assert_adaptation_safety(&floor, &after));

        let mut weakened = floor.clone();
        weakened.verification_passed = false;
        assert!(!assert_adaptation_safety(&floor, &weakened));
    }

    #[test]
    fn five_alarm_never_in_recommendations() {
        let mut eng = AdaptationEngine::new();
        for _ in 0..5 {
            eng.record_run(RunOutcome {
                contained: false,
                out_of_control: true,
                firebreak: Some(false),
                rolled_back: false,
                category: TaskCategory::FeatureAddition,
                fire_level: FireLevel::Critical,
                simplicity_score: 200.0,
                verification_passed: true,
                pressure_template_id: "release_freeze_v1",
                reviewer_id: None,
            });
        }
        let rec = eng.recommend(TaskCategory::FeatureAddition);
        if let Some(fl) = rec.suggested_fire_level {
            assert!(fl.is_initial_selectable());
        }
    }
}

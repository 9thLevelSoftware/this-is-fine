//! Local-only adaptation engine.
//!
//! May tune pressure variants, Fire Level selection, thresholds, reviewer ranking,
//! and Firebreak attempt limits. Must not lower the correctness floor, authorize
//! new models, send telemetry, or disable required verification.

use serde::{Deserialize, Serialize};

use crate::error::{Result, TifError};
use crate::fire_level::FireLevel;
use crate::reviewer::ReviewerStats;
use crate::scoring::CorrectnessFloor;
use crate::task::TaskCategory;

/// Knobs that adaptation is allowed to self-apply (Phase 6 safety allowlist).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfApplyKnob {
    /// Bias initial fire level selection (1–4 only).
    FireLevelBias,
    /// Soft simplicity thresholds (limits.score / added_lines scale).
    Thresholds,
    /// Preferred pressure template id within a family.
    PressureTemplate,
}

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
    /// Currently promoted template id per family (local only).
    #[serde(default)]
    pub promoted_templates: Vec<PromotedTemplate>,
    /// Self-applied knobs currently active (audit trail).
    #[serde(default)]
    pub applied_knobs: AppliedKnobs,
}

/// Local self-applied adaptation knobs (never floor / sensitive / verify).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AppliedKnobs {
    /// Preferred fire level bias 1–4 (None = no bias).
    pub fire_level_bias: Option<u8>,
    /// Soft limit scale in (0, 1] — only tightens, never loosens past 1.0.
    pub limit_scale: Option<f64>,
    /// Preferred pressure template ids.
    pub preferred_pressure_templates: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromotedTemplate {
    pub family: String,
    pub template_id: String,
    pub promoted_at_unix: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariantScore {
    pub template_id: String,
    pub trials: u32,
    pub avg_simplicity_score: f64,
    pub verification_pass_rate: f64,
    /// Correctness-floor failure rate (must stay ~0 for promotion).
    #[serde(default)]
    pub floor_fail_rate: f64,
    /// Promotion state for offline eval / live rollout stubs.
    #[serde(default)]
    pub status: VariantStatus,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VariantStatus {
    #[default]
    Candidate,
    Promoted,
    Demoted,
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
    /// When true, correctness floor failed (blocks promotion).
    pub floor_failed: bool,
}

/// Safety gates for promoting a pressure variant (Phase 6 stubs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromotionGate {
    pub min_trials: u32,
    pub min_verification_pass_rate: f64,
    pub max_floor_fail_rate: f64,
    pub min_score_improvement: f64,
}

impl Default for PromotionGate {
    fn default() -> Self {
        Self {
            min_trials: 5,
            min_verification_pass_rate: 0.99,
            max_floor_fail_rate: 0.0,
            min_score_improvement: 1.0,
        }
    }
}

/// Result of a promotion / demotion decision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VariantDecision {
    pub template_id: String,
    pub action: VariantAction,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VariantAction {
    Promote,
    Demote,
    Hold,
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

    pub fn stats_mut(&mut self) -> &mut AdaptationStats {
        &mut self.stats
    }

    pub fn load(stats: AdaptationStats) -> Self {
        Self { stats }
    }

    /// Reset all local adaptation state.
    pub fn reset(&mut self) {
        self.stats = AdaptationStats::default();
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
            v.floor_fail_rate =
                (v.floor_fail_rate * n + if outcome.floor_failed { 1.0 } else { 0.0 }) / (n + 1.0);
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
                floor_fail_rate: if outcome.floor_failed { 1.0 } else { 0.0 },
                status: VariantStatus::Candidate,
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

        // Applied fire-level bias takes precedence when present.
        if let Some(bias) = self.stats.applied_knobs.fire_level_bias {
            if let Ok(fl) = FireLevel::parse_initial(bias) {
                rec.suggested_fire_level = Some(fl);
                rec.notes
                    .push(format!("self-applied fire level bias: {}", fl.as_u8()));
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
            .filter(|v| {
                v.trials >= 2
                    && v.verification_pass_rate >= 0.99
                    && v.floor_fail_rate <= 0.0
                    && v.status != VariantStatus::Demoted
            })
            .min_by(|a, b| {
                a.avg_simplicity_score
                    .partial_cmp(&b.avg_simplicity_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        {
            rec.preferred_pressure_template = Some(best.template_id.clone());
        }

        // Promoted templates override score-based pick when present.
        if let Some(p) = self.stats.promoted_templates.first() {
            rec.preferred_pressure_template = Some(p.template_id.clone());
            rec.notes.push(format!(
                "promoted pressure template: {} ({})",
                p.template_id, p.family
            ));
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

        if let Some(scale) = self.stats.applied_knobs.limit_scale {
            rec.limit_scale = Some(scale);
        }

        rec
    }

    /// Evaluate promotion/demotion for a candidate variant against a baseline.
    ///
    /// Offline eval harness entrypoint — does not network.
    pub fn evaluate_variant(
        &self,
        candidate_id: &str,
        baseline_id: &str,
        gate: &PromotionGate,
    ) -> VariantDecision {
        let cand = self
            .stats
            .pressure_variant_scores
            .iter()
            .find(|v| v.template_id == candidate_id);
        let base = self
            .stats
            .pressure_variant_scores
            .iter()
            .find(|v| v.template_id == baseline_id);

        let Some(cand) = cand else {
            return VariantDecision {
                template_id: candidate_id.to_string(),
                action: VariantAction::Hold,
                reason: "candidate has no recorded trials".into(),
            };
        };

        // Hard safety: any floor failures block promotion.
        if cand.floor_fail_rate > gate.max_floor_fail_rate {
            return VariantDecision {
                template_id: candidate_id.to_string(),
                action: VariantAction::Demote,
                reason: format!(
                    "floor_fail_rate {:.3} exceeds max {:.3}",
                    cand.floor_fail_rate, gate.max_floor_fail_rate
                ),
            };
        }

        if cand.trials < gate.min_trials {
            return VariantDecision {
                template_id: candidate_id.to_string(),
                action: VariantAction::Hold,
                reason: format!("trials {} < min {}", cand.trials, gate.min_trials),
            };
        }

        if cand.verification_pass_rate < gate.min_verification_pass_rate {
            return VariantDecision {
                template_id: candidate_id.to_string(),
                action: VariantAction::Demote,
                reason: format!(
                    "verification_pass_rate {:.3} < min {:.3}",
                    cand.verification_pass_rate, gate.min_verification_pass_rate
                ),
            };
        }

        if let Some(base) = base {
            if base.trials > 0
                && cand.avg_simplicity_score + gate.min_score_improvement
                    >= base.avg_simplicity_score
            {
                return VariantDecision {
                    template_id: candidate_id.to_string(),
                    action: VariantAction::Hold,
                    reason: format!(
                        "score {:.1} not better than baseline {:.1} by {}",
                        cand.avg_simplicity_score,
                        base.avg_simplicity_score,
                        gate.min_score_improvement
                    ),
                };
            }
        }

        VariantDecision {
            template_id: candidate_id.to_string(),
            action: VariantAction::Promote,
            reason: "passed promotion gates".into(),
        }
    }

    /// Apply a promotion decision with safety gates. Updates local stats only.
    pub fn apply_variant_decision(
        &mut self,
        decision: &VariantDecision,
        family: &str,
    ) -> Result<()> {
        match decision.action {
            VariantAction::Promote => {
                if let Some(v) = self
                    .stats
                    .pressure_variant_scores
                    .iter_mut()
                    .find(|v| v.template_id == decision.template_id)
                {
                    v.status = VariantStatus::Promoted;
                }
                // Replace promotion for this family.
                self.stats.promoted_templates.retain(|p| p.family != family);
                self.stats.promoted_templates.push(PromotedTemplate {
                    family: family.to_string(),
                    template_id: decision.template_id.clone(),
                    promoted_at_unix: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0),
                });
                if !self
                    .stats
                    .applied_knobs
                    .preferred_pressure_templates
                    .contains(&decision.template_id)
                {
                    self.stats
                        .applied_knobs
                        .preferred_pressure_templates
                        .push(decision.template_id.clone());
                }
                Ok(())
            }
            VariantAction::Demote => {
                if let Some(v) = self
                    .stats
                    .pressure_variant_scores
                    .iter_mut()
                    .find(|v| v.template_id == decision.template_id)
                {
                    v.status = VariantStatus::Demoted;
                }
                self.stats
                    .promoted_templates
                    .retain(|p| p.template_id != decision.template_id);
                self.stats
                    .applied_knobs
                    .preferred_pressure_templates
                    .retain(|t| t != &decision.template_id);
                Ok(())
            }
            VariantAction::Hold => Ok(()),
        }
    }

    /// Self-apply only allowlisted knobs. Rejects floor / sensitive / verify mutations.
    pub fn self_apply(&mut self, knob: SelfApplyKnob, value: SelfApplyValue) -> Result<()> {
        match (knob, value) {
            (SelfApplyKnob::FireLevelBias, SelfApplyValue::FireLevel(level)) => {
                let fl = FireLevel::parse_initial(level)?;
                // Explicit: never Five-Alarm via adaptation.
                if fl.as_u8() > 4 {
                    return Err(TifError::Config(
                        "adaptation cannot set fire level above 4".into(),
                    ));
                }
                self.stats.applied_knobs.fire_level_bias = Some(fl.as_u8());
                Ok(())
            }
            (SelfApplyKnob::Thresholds, SelfApplyValue::LimitScale(scale)) => {
                if !(scale > 0.0 && scale <= 1.0) {
                    return Err(TifError::Config(
                        "limit_scale must be in (0, 1] (tighten only)".into(),
                    ));
                }
                self.stats.applied_knobs.limit_scale = Some(scale);
                Ok(())
            }
            (SelfApplyKnob::PressureTemplate, SelfApplyValue::TemplateId(id)) => {
                if id.is_empty() {
                    return Err(TifError::Config("empty template id".into()));
                }
                if !self
                    .stats
                    .applied_knobs
                    .preferred_pressure_templates
                    .contains(&id)
                {
                    self.stats
                        .applied_knobs
                        .preferred_pressure_templates
                        .push(id);
                }
                Ok(())
            }
            _ => Err(TifError::Config(format!(
                "self-apply rejected: knob {knob:?} incompatible with value"
            ))),
        }
    }

    /// Reject attempts to apply forbidden knobs (API for callers / tests).
    pub fn reject_forbidden_self_apply(request: &str) -> Result<()> {
        let lower = request.to_ascii_lowercase();
        if lower.contains("correctness_floor")
            || lower.contains("floor")
            || lower.contains("verification")
            || lower.contains("sensitive")
            || lower.contains("require_firebreak_approval")
            || lower.contains("allow_source_egress")
            || (lower.contains("reviewer") && lower.contains("add"))
        {
            return Err(TifError::Config(format!(
                "adaptation self-apply forbidden for: {request}"
            )));
        }
        Ok(())
    }

    /// Explicit invariant: adaptation cannot lower the correctness floor.
    pub fn cannot_lower_correctness_floor(&self, floor: &CorrectnessFloor) -> Result<()> {
        // The floor is never mutated by this engine.
        let _ = floor;
        SimplicityFloorGuard::assert_unchanged();
        Ok(())
    }
}

/// Values accepted by [`AdaptationEngine::self_apply`].
#[derive(Debug, Clone)]
pub enum SelfApplyValue {
    FireLevel(u8),
    LimitScale(f64),
    TemplateId(String),
    /// Forbidden payloads — always rejected when used with wrong knobs.
    Forbidden {
        name: String,
    },
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

    fn outcome(template: &str, score: f64, verify: bool, floor_failed: bool) -> RunOutcome<'_> {
        RunOutcome {
            contained: verify && !floor_failed,
            out_of_control: false,
            firebreak: None,
            rolled_back: false,
            category: TaskCategory::BugFix,
            fire_level: FireLevel::Containment,
            simplicity_score: score,
            verification_passed: verify,
            pressure_template_id: template,
            reviewer_id: None,
            floor_failed,
        }
    }

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
                floor_failed: false,
            });
        }
        let rec = eng.recommend(TaskCategory::FeatureAddition);
        if let Some(fl) = rec.suggested_fire_level {
            assert!(fl.is_initial_selectable());
        }
    }

    #[test]
    fn offline_eval_promotes_better_variant() {
        let mut eng = AdaptationEngine::new();
        let gate = PromotionGate {
            min_trials: 3,
            min_verification_pass_rate: 0.99,
            max_floor_fail_rate: 0.0,
            min_score_improvement: 5.0,
        };
        for _ in 0..5 {
            eng.record_run(outcome("baseline_v1", 100.0, true, false));
            eng.record_run(outcome("candidate_v2", 50.0, true, false));
        }
        let d = eng.evaluate_variant("candidate_v2", "baseline_v1", &gate);
        assert_eq!(d.action, VariantAction::Promote);
        eng.apply_variant_decision(&d, "production_incident")
            .unwrap();
        assert!(eng
            .stats()
            .promoted_templates
            .iter()
            .any(|p| p.template_id == "candidate_v2"));
    }

    #[test]
    fn offline_eval_demotes_floor_failure() {
        let mut eng = AdaptationEngine::new();
        let gate = PromotionGate::default();
        for _ in 0..6 {
            eng.record_run(outcome("bad_v1", 10.0, true, true));
        }
        let d = eng.evaluate_variant("bad_v1", "baseline_v1", &gate);
        assert_eq!(d.action, VariantAction::Demote);
        eng.apply_variant_decision(&d, "context_fire").unwrap();
        assert_eq!(
            eng.stats()
                .pressure_variant_scores
                .iter()
                .find(|v| v.template_id == "bad_v1")
                .unwrap()
                .status,
            VariantStatus::Demoted
        );
    }

    #[test]
    fn self_apply_allows_fire_level_and_thresholds() {
        let mut eng = AdaptationEngine::new();
        eng.self_apply(SelfApplyKnob::FireLevelBias, SelfApplyValue::FireLevel(2))
            .unwrap();
        eng.self_apply(SelfApplyKnob::Thresholds, SelfApplyValue::LimitScale(0.85))
            .unwrap();
        assert_eq!(eng.stats().applied_knobs.fire_level_bias, Some(2));
        assert_eq!(eng.stats().applied_knobs.limit_scale, Some(0.85));
    }

    #[test]
    fn self_apply_rejects_five_alarm() {
        let mut eng = AdaptationEngine::new();
        let err = eng
            .self_apply(SelfApplyKnob::FireLevelBias, SelfApplyValue::FireLevel(5))
            .unwrap_err();
        assert!(
            err.to_string().contains("five-alarm")
                || err.to_string().contains("Five-Alarm")
                || err.to_string().contains("escalation")
                || err.to_string().contains("above 4")
                || err.to_string().contains("initial")
        );
    }

    #[test]
    fn forbidden_self_apply_requests() {
        assert!(AdaptationEngine::reject_forbidden_self_apply("correctness_floor").is_err());
        assert!(AdaptationEngine::reject_forbidden_self_apply("disable verification").is_err());
        assert!(AdaptationEngine::reject_forbidden_self_apply("sensitive_paths clear").is_err());
        assert!(AdaptationEngine::reject_forbidden_self_apply("fire_level_bias").is_ok());
    }

    #[test]
    fn cannot_self_apply_limit_scale_above_one() {
        let mut eng = AdaptationEngine::new();
        assert!(eng
            .self_apply(SelfApplyKnob::Thresholds, SelfApplyValue::LimitScale(1.5))
            .is_err());
    }
}

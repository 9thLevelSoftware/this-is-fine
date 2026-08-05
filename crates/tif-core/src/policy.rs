//! Containment policy compiler.
//!
//! Merge order:
//! 1. Global defaults
//! 2. Shared repository configuration
//! 3. Local repository overrides
//! 4. Task-derived policy
//! 5. Current Fire Level
//! 6. Adaptive model-specific adjustments
//! 7. Explicit command-line overrides

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::{Config, SimplicityLimits, SimplicityWeights, VerificationConfig};
use crate::error::Result;
use crate::fire_level::FireLevel;
use crate::pressure::{PressureEngine, PressureScenario};
use crate::task::{TaskCategory, TaskClassifier};

/// Versioned, machine-readable containment policy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContainmentPolicy {
    pub policy_id: String,
    pub policy_version: String,
    pub compiled_at: DateTime<Utc>,
    pub enabled: bool,
    pub fire_level: FireLevel,
    pub task_category: TaskCategory,
    pub pressure: PressureScenario,
    pub verification: VerificationConfig,
    pub weights: SimplicityWeights,
    pub limits: SimplicityLimits,
    pub sensitive_paths: Vec<String>,
    pub require_firebreak_approval: bool,
    pub exclusions: Vec<String>,
    /// Effective hard-limit scaling note for adapters.
    pub notes: Vec<String>,
}

/// Inputs for policy compilation.
#[derive(Debug, Clone, Default)]
pub struct PolicyCompileRequest {
    pub task_text: Option<String>,
    pub task_category: Option<TaskCategory>,
    /// Initial fire level override (must not be Five-Alarm unless escalation).
    pub fire_level: Option<FireLevel>,
    /// Explicit escalation after current containment failure.
    pub escalate_five_alarm: bool,
    pub current_containment_failure: bool,
    pub enabled: Option<bool>,
    /// Adaptive adjustments applied after config (local-only).
    pub adaptive_limit_scale: Option<f64>,
    pub model_id: Option<String>,
}

/// Compiles a versioned policy from config + runtime inputs.
#[derive(Debug, Default)]
pub struct PolicyCompiler;

impl PolicyCompiler {
    pub fn new() -> Self {
        Self
    }

    pub fn compile(
        &self,
        config: &Config,
        req: &PolicyCompileRequest,
    ) -> Result<ContainmentPolicy> {
        let classifier = TaskClassifier::new();
        let task_category = req
            .task_category
            .or_else(|| req.task_text.as_deref().map(|t| classifier.classify(t)))
            .unwrap_or(TaskCategory::Unknown);

        let fire_level = if req.escalate_five_alarm {
            FireLevel::escalate_to_five_alarm(req.current_containment_failure)?
        } else if let Some(fl) = req.fire_level {
            // Single enforcement path: Five-Alarm only with current containment failure.
            if fl == FireLevel::FiveAlarm && !req.current_containment_failure {
                return Err(crate::error::TifError::FiveAlarmInitialForbidden);
            }
            fl
        } else {
            // Prefer task suggestion if default is containment and task differs.
            let configured = FireLevel::parse_initial(config.default_fire_level)?;
            let suggested = task_category.suggested_fire_level();
            if configured == FireLevel::Containment && suggested != FireLevel::Containment {
                suggested
            } else {
                configured
            }
        };

        let pressure_engine = PressureEngine::from_config(
            config.pressure.include_baseline,
            &config.pressure.allowed_families,
        );
        let pressure = pressure_engine.select(task_category, fire_level)?;

        let mut limits = config.simplicity.limits.clone();
        let mult = fire_level.pressure_multiplier();
        // Higher fire levels tighten soft score budgets if present.
        if let Some(score) = limits.score {
            limits.score = Some(score / mult);
        }

        // Adaptive scale may tighten limits further but never lowers correctness floor.
        if let Some(scale) = req.adaptive_limit_scale {
            if scale > 0.0 && scale < 1.0 {
                if let Some(n) = limits.new_files {
                    limits.new_files = Some(((n as f64) * scale).floor() as u32);
                }
                if let Some(n) = limits.added_lines {
                    limits.added_lines = Some(((n as f64) * scale).floor() as u32);
                }
            }
        }

        let mut notes = vec![
            format!("task_category={}", task_category.as_str()),
            format!("fire_level={}", fire_level.as_u8()),
            format!("pressure_family={}", pressure.family),
        ];
        if let Some(ref m) = req.model_id {
            notes.push(format!("model={m}"));
        }

        Ok(ContainmentPolicy {
            policy_id: Uuid::new_v4().to_string(),
            policy_version: "1".into(),
            compiled_at: Utc::now(),
            enabled: req.enabled.unwrap_or(config.enabled),
            fire_level,
            task_category,
            pressure,
            verification: config.verification.clone(),
            weights: config.simplicity.weights.clone(),
            limits,
            sensitive_paths: config.approval.sensitive_paths.clone(),
            require_firebreak_approval: config.approval.require_firebreak_approval,
            exclusions: config.exclusions.clone(),
            notes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn compile_default_policy() {
        let cfg = Config::default();
        let policy = PolicyCompiler::new()
            .compile(
                &cfg,
                &PolicyCompileRequest {
                    task_text: Some("fix bug in parser".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(policy.task_category, TaskCategory::BugFix);
        assert!(policy.fire_level.is_initial_selectable());
        assert!(policy.pressure.body.contains("CONTAINMENT MODE"));
    }

    #[test]
    fn five_alarm_requires_failure() {
        let cfg = Config::default();
        let err = PolicyCompiler::new()
            .compile(
                &cfg,
                &PolicyCompileRequest {
                    fire_level: Some(FireLevel::FiveAlarm),
                    current_containment_failure: false,
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert!(matches!(
            err,
            crate::error::TifError::FiveAlarmInitialForbidden
        ));
    }

    #[test]
    fn five_alarm_allowed_after_failure() {
        let cfg = Config::default();
        let policy = PolicyCompiler::new()
            .compile(
                &cfg,
                &PolicyCompileRequest {
                    escalate_five_alarm: true,
                    current_containment_failure: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(policy.fire_level, FireLevel::FiveAlarm);
    }

    /// Performance smoke: policy resolve fast path stays well under a soft budget.
    ///
    /// Compile is pure in-memory (config + templates); adapters call this on every
    /// task start, so regressions here are user-visible latency.
    #[test]
    fn policy_resolve_fast_path_smoke() {
        use std::time::Instant;
        let cfg = Config::default();
        let compiler = PolicyCompiler::new();
        // Warm once (UUID/clock paths).
        let _ = compiler
            .compile(
                &cfg,
                &PolicyCompileRequest {
                    task_text: Some("fix null pointer".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let n = 200usize;
        let start = Instant::now();
        for i in 0..n {
            let p = compiler
                .compile(
                    &cfg,
                    &PolicyCompileRequest {
                        task_text: Some(format!("fix bug number {i}")),
                        ..Default::default()
                    },
                )
                .unwrap();
            assert!(p.fire_level.is_initial_selectable());
        }
        let elapsed = start.elapsed();
        // Soft budget: 200 compiles in < 500ms on debug CI hosts (usually << 50ms).
        assert!(
            elapsed.as_millis() < 500,
            "policy resolve fast path too slow: {elapsed:?} for {n} compiles"
        );
    }
}

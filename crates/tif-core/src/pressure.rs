//! Pressure engine: curated scenario templates and baseline containment prompt.

use serde::{Deserialize, Serialize};

use crate::fire_level::FireLevel;
use crate::task::TaskCategory;

/// Versioned pressure scenario ready for adapter injection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PressureScenario {
    pub family: String,
    pub template_id: String,
    pub template_version: String,
    pub fire_level: FireLevel,
    pub body: String,
    pub compact_status: String,
}

/// Curated scenario template (versioned wording variants per family).
#[derive(Debug, Clone)]
pub struct ScenarioTemplate {
    pub id: &'static str,
    pub family: &'static str,
    pub version: &'static str,
    pub title: &'static str,
    pub body: &'static str,
    /// When true, this is the default/promoted baseline for the family.
    pub is_default: bool,
}

/// Baseline containment prompt shared across families.
pub const BASELINE_CONTAINMENT_PROMPT: &str = r#"THIS IS FINE — CONTAINMENT MODE

The codebase is under active operational pressure. Time, context, and review
capacity are limited. Resolve the requested problem with the smallest correct
and verifiable intervention.

Treat every unnecessary file, dependency, abstraction, and line of code as
additional fire.

Before making changes:
1. Confirm that the requested behavior does not already exist.
2. Reuse the closest existing implementation or project convention.
3. Prefer configuration, deletion, or a small modification over new machinery.
4. Prefer platform and standard-library capabilities over dependencies.
5. Change the fewest files and public interfaces possible.
6. Do not add speculative flexibility or future-facing abstractions.
7. Preserve security, validation, error handling, and required tests.
8. Do not reduce required scope merely to make the diff smaller.
9. Briefly justify any larger solution that is genuinely necessary.

Contain the fire. Do not remodel the building."#;

/// At least two versioned variants per family (Phase 6).
const TEMPLATES: &[ScenarioTemplate] = &[
    // production_incident
    ScenarioTemplate {
        id: "production_incident_v1",
        family: "production_incident",
        version: "1",
        title: "Production Incident",
        body: "A production incident is open. Stabilize with the smallest verified fix. Avoid drive-by refactors and dependency additions.",
        is_default: true,
    },
    ScenarioTemplate {
        id: "production_incident_v2",
        family: "production_incident",
        version: "2",
        title: "Production Incident (tight)",
        body: "Production is degraded. Ship the minimal verified patch only. No refactors, no new dependencies, no public API growth. Prefer a one-file fix when correct.",
        is_default: false,
    },
    // release_freeze
    ScenarioTemplate {
        id: "release_freeze_v1",
        family: "release_freeze",
        version: "1",
        title: "Release Freeze",
        body: "A release freeze is in effect. Ship only the minimum required change. Prefer existing patterns; block nonessential public API growth.",
        is_default: true,
    },
    ScenarioTemplate {
        id: "release_freeze_v2",
        family: "release_freeze",
        version: "2",
        title: "Release Freeze (strict)",
        body: "Release freeze: change only what the ticket requires. Reject nice-to-have cleanups. Prefer config flags over new code paths.",
        is_default: false,
    },
    // limited_maintenance_window
    ScenarioTemplate {
        id: "limited_maintenance_window_v1",
        family: "limited_maintenance_window",
        version: "1",
        title: "Limited Maintenance Window",
        body: "The maintenance window is short. Refactor only what the task requires. Do not expand module boundaries or introduce new layers.",
        is_default: true,
    },
    ScenarioTemplate {
        id: "limited_maintenance_window_v2",
        family: "limited_maintenance_window",
        version: "2",
        title: "Limited Maintenance Window (clock)",
        body: "Maintenance window is closing. Optimize for reversibility: small diffs, existing helpers, delete dead code only when required by the task.",
        is_default: false,
    },
    // context_fire
    ScenarioTemplate {
        id: "context_fire_v1",
        family: "context_fire",
        version: "1",
        title: "Context Fire",
        body: "Context and token budget are constrained. Prefer terse diffs, reuse, and deletion over new abstractions or generated scaffolding.",
        is_default: true,
    },
    ScenarioTemplate {
        id: "context_fire_v2",
        family: "context_fire",
        version: "2",
        title: "Context Fire (sparse)",
        body: "Token budget is low. Touch the fewest symbols. Reuse names and modules already in scope. Avoid scaffolding, generators, and multi-file moves.",
        is_default: false,
    },
    // breach_containment
    ScenarioTemplate {
        id: "breach_containment_v1",
        family: "breach_containment",
        version: "1",
        title: "Breach Containment",
        body: "Treat this as breach containment. Preserve validation, authz, and security boundaries. Minimalism must not remove required checks.",
        is_default: true,
    },
    ScenarioTemplate {
        id: "breach_containment_v2",
        family: "breach_containment",
        version: "2",
        title: "Breach Containment (hardening)",
        body: "Security-sensitive change. Never strip validation, authz, crypto, or audit logging to shrink the diff. Prefer fail-closed defaults.",
        is_default: false,
    },
];

/// Selects curated pressure scenarios; never free-form emergencies on the default path.
#[derive(Debug, Default)]
pub struct PressureEngine {
    include_baseline: bool,
    allowed_families: Vec<String>,
    /// Optional preferred template ids (from adaptation promotion).
    preferred_template_ids: Vec<String>,
}

impl PressureEngine {
    pub fn new(include_baseline: bool, allowed_families: Vec<String>) -> Self {
        Self {
            include_baseline,
            allowed_families,
            preferred_template_ids: Vec::new(),
        }
    }

    pub fn from_config(include_baseline: bool, allowed_families: &[String]) -> Self {
        Self {
            include_baseline,
            allowed_families: allowed_families.to_vec(),
            preferred_template_ids: Vec::new(),
        }
    }

    /// Prefer promoted / recommended template ids when selecting within a family.
    pub fn with_preferred_templates(mut self, ids: Vec<String>) -> Self {
        self.preferred_template_ids = ids;
        self
    }

    pub fn list_templates(&self) -> Vec<&'static ScenarioTemplate> {
        TEMPLATES
            .iter()
            .filter(|t| self.family_allowed(t.family))
            .collect()
    }

    /// All curated templates (including non-default variants).
    pub fn all_templates() -> &'static [ScenarioTemplate] {
        TEMPLATES
    }

    /// Templates for a family (both variants).
    pub fn templates_for_family(family: &str) -> Vec<&'static ScenarioTemplate> {
        TEMPLATES.iter().filter(|t| t.family == family).collect()
    }

    fn family_allowed(&self, family: &str) -> bool {
        self.allowed_families.is_empty() || self.allowed_families.iter().any(|f| f == family)
    }

    /// Select a scenario for the task category and fire level.
    ///
    /// Never returns a family excluded by `allowed_families`. When the preferred
    /// family is disallowed, falls back to `context_fire` if allowed, otherwise
    /// the first allowed curated template. Errors if the allow-list admits none.
    ///
    /// Within a family, prefers promoted template ids, else the default variant.
    pub fn select(
        &self,
        category: TaskCategory,
        fire_level: FireLevel,
    ) -> crate::error::Result<PressureScenario> {
        let family = category.pressure_family();
        let family = if self.family_allowed(family) {
            family
        } else if self.family_allowed("context_fire") {
            "context_fire"
        } else {
            TEMPLATES
                .iter()
                .find(|t| self.family_allowed(t.family))
                .map(|t| t.family)
                .ok_or_else(|| {
                    crate::error::TifError::Config(format!(
                        "no pressure scenario template allowed for families {:?}; task prefers `{family}`",
                        self.allowed_families
                    ))
                })?
        };

        let template = self.pick_variant(family).ok_or_else(|| {
            crate::error::TifError::Config(format!(
                "no pressure scenario template allowed for families {:?}; task prefers `{family}`",
                self.allowed_families
            ))
        })?;

        let intensity = fire_level_guidance(fire_level);
        let mut body = String::new();
        if self.include_baseline {
            body.push_str(BASELINE_CONTAINMENT_PROMPT);
            body.push_str("\n\n");
        }
        body.push_str(&format!(
            "SCENARIO: {} ({})\nFire Level: {}\n\n{}\n\n{}",
            template.title, template.family, fire_level, template.body, intensity
        ));

        Ok(PressureScenario {
            family: template.family.to_string(),
            template_id: template.id.to_string(),
            template_version: template.version.to_string(),
            fire_level,
            body,
            compact_status: format!("🔥 Containment active · Fire Level {}", fire_level.as_u8()),
        })
    }

    fn pick_variant(&self, family: &str) -> Option<&'static ScenarioTemplate> {
        let family_templates: Vec<_> = TEMPLATES
            .iter()
            .filter(|t| t.family == family && self.family_allowed(t.family))
            .collect();
        if family_templates.is_empty() {
            return None;
        }
        // Preferred / promoted ids first.
        for id in &self.preferred_template_ids {
            if let Some(t) = family_templates.iter().find(|t| t.id == id.as_str()) {
                return Some(*t);
            }
        }
        family_templates
            .iter()
            .find(|t| t.is_default)
            .copied()
            .or_else(|| family_templates.first().copied())
    }
}

fn fire_level_guidance(level: FireLevel) -> String {
    match level {
        FireLevel::Ember => {
            "Intensity: Ember — light brevity and reuse guidance; minimal intervention.".into()
        }
        FireLevel::Smolder => {
            "Intensity: Smolder — stronger YAGNI pressure; warn on avoidable additions.".into()
        }
        FireLevel::Containment => {
            "Intensity: Containment — strict minimal-diff expectations; automatic review if over budget.".into()
        }
        FireLevel::Critical => {
            "Intensity: Critical — aggressive reduction of files, abstractions, dependencies, and prose.".into()
        }
        FireLevel::FiveAlarm => {
            "Intensity: Five-Alarm — maximum restraint; intensified Firebreak and staged recovery.".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_included_by_default() {
        let engine = PressureEngine::new(true, vec![]);
        let s = engine
            .select(TaskCategory::BugFix, FireLevel::Containment)
            .unwrap();
        assert!(s.body.contains("CONTAINMENT MODE"));
        assert_eq!(s.family, "production_incident");
        assert!(s.compact_status.contains("Fire Level 3"));
        assert_eq!(s.template_id, "production_incident_v1");
    }

    #[test]
    fn respects_allowed_families() {
        let engine = PressureEngine::new(false, vec!["context_fire".into()]);
        let s = engine
            .select(TaskCategory::BugFix, FireLevel::Smolder)
            .unwrap();
        // Bug fix wants production_incident but only context_fire allowed → fallback
        assert_eq!(s.family, "context_fire");
    }

    #[test]
    fn does_not_inject_disallowed_context_fire_fallback() {
        // Only release_freeze allowed; task prefers production_incident.
        let engine = PressureEngine::new(false, vec!["release_freeze".into()]);
        let s = engine
            .select(TaskCategory::BugFix, FireLevel::Containment)
            .unwrap();
        assert_eq!(s.family, "release_freeze");
    }

    #[test]
    fn errors_when_no_allowed_family_matches() {
        let engine = PressureEngine::new(false, vec!["nonexistent_family".into()]);
        let err = engine
            .select(TaskCategory::BugFix, FireLevel::Ember)
            .unwrap_err();
        assert!(err.to_string().contains("no pressure scenario template"));
    }

    #[test]
    fn each_family_has_at_least_two_variants() {
        let mut by_family: std::collections::BTreeMap<&str, usize> =
            std::collections::BTreeMap::new();
        for t in TEMPLATES {
            *by_family.entry(t.family).or_default() += 1;
        }
        for (fam, n) in by_family {
            assert!(n >= 2, "family {fam} needs ≥2 variants, has {n}");
        }
    }

    #[test]
    fn preferred_template_selected() {
        let engine = PressureEngine::new(false, vec![])
            .with_preferred_templates(vec!["production_incident_v2".into()]);
        let s = engine
            .select(TaskCategory::BugFix, FireLevel::Containment)
            .unwrap();
        assert_eq!(s.template_id, "production_incident_v2");
        assert_eq!(s.template_version, "2");
    }
}

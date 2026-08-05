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

/// Curated scenario template.
#[derive(Debug, Clone)]
pub struct ScenarioTemplate {
    pub id: &'static str,
    pub family: &'static str,
    pub version: &'static str,
    pub title: &'static str,
    pub body: &'static str,
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

const TEMPLATES: &[ScenarioTemplate] = &[
    ScenarioTemplate {
        id: "production_incident_v1",
        family: "production_incident",
        version: "1",
        title: "Production Incident",
        body: "A production incident is open. Stabilize with the smallest verified fix. Avoid drive-by refactors and dependency additions.",
    },
    ScenarioTemplate {
        id: "release_freeze_v1",
        family: "release_freeze",
        version: "1",
        title: "Release Freeze",
        body: "A release freeze is in effect. Ship only the minimum required change. Prefer existing patterns; block nonessential public API growth.",
    },
    ScenarioTemplate {
        id: "limited_maintenance_window_v1",
        family: "limited_maintenance_window",
        version: "1",
        title: "Limited Maintenance Window",
        body: "The maintenance window is short. Refactor only what the task requires. Do not expand module boundaries or introduce new layers.",
    },
    ScenarioTemplate {
        id: "context_fire_v1",
        family: "context_fire",
        version: "1",
        title: "Context Fire",
        body: "Context and token budget are constrained. Prefer terse diffs, reuse, and deletion over new abstractions or generated scaffolding.",
    },
    ScenarioTemplate {
        id: "breach_containment_v1",
        family: "breach_containment",
        version: "1",
        title: "Breach Containment",
        body: "Treat this as breach containment. Preserve validation, authz, and security boundaries. Minimalism must not remove required checks.",
    },
];

/// Selects curated pressure scenarios; never free-form emergencies on the default path.
#[derive(Debug, Default)]
pub struct PressureEngine {
    include_baseline: bool,
    allowed_families: Vec<String>,
}

impl PressureEngine {
    pub fn new(include_baseline: bool, allowed_families: Vec<String>) -> Self {
        Self {
            include_baseline,
            allowed_families,
        }
    }

    pub fn from_config(include_baseline: bool, allowed_families: &[String]) -> Self {
        Self {
            include_baseline,
            allowed_families: allowed_families.to_vec(),
        }
    }

    pub fn list_templates(&self) -> Vec<&'static ScenarioTemplate> {
        TEMPLATES
            .iter()
            .filter(|t| self.family_allowed(t.family))
            .collect()
    }

    fn family_allowed(&self, family: &str) -> bool {
        self.allowed_families.is_empty() || self.allowed_families.iter().any(|f| f == family)
    }

    /// Select a scenario for the task category and fire level.
    pub fn select(&self, category: TaskCategory, fire_level: FireLevel) -> PressureScenario {
        let family = category.pressure_family();
        let template = TEMPLATES
            .iter()
            .find(|t| t.family == family && self.family_allowed(t.family))
            .or_else(|| {
                TEMPLATES
                    .iter()
                    .find(|t| t.family == "context_fire" && self.family_allowed(t.family))
            })
            .unwrap_or(&TEMPLATES[3]); // context_fire fallback

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

        PressureScenario {
            family: template.family.to_string(),
            template_id: template.id.to_string(),
            template_version: template.version.to_string(),
            fire_level,
            body,
            compact_status: format!("🔥 Containment active · Fire Level {}", fire_level.as_u8()),
        }
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
        let s = engine.select(TaskCategory::BugFix, FireLevel::Containment);
        assert!(s.body.contains("CONTAINMENT MODE"));
        assert_eq!(s.family, "production_incident");
        assert!(s.compact_status.contains("Fire Level 3"));
    }

    #[test]
    fn respects_allowed_families() {
        let engine = PressureEngine::new(false, vec!["context_fire".into()]);
        let s = engine.select(TaskCategory::BugFix, FireLevel::Smolder);
        // Bug fix wants production_incident but only context_fire allowed → fallback
        assert_eq!(s.family, "context_fire");
    }
}

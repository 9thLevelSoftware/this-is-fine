//! Task classification.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::{Result, TifError};
use crate::fire_level::FireLevel;

/// High-level task categories that influence Fire Level, pressure, and limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskCategory {
    BugFix,
    FeatureAddition,
    Refactor,
    ArchitectureWork,
    SecurityRemediation,
    PerformanceOptimization,
    ConfigurationChange,
    DependencyChange,
    Documentation,
    TestOnly,
    Prototype,
    Unknown,
}

impl TaskCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskCategory::BugFix => "bug_fix",
            TaskCategory::FeatureAddition => "feature_addition",
            TaskCategory::Refactor => "refactor",
            TaskCategory::ArchitectureWork => "architecture_work",
            TaskCategory::SecurityRemediation => "security_remediation",
            TaskCategory::PerformanceOptimization => "performance_optimization",
            TaskCategory::ConfigurationChange => "configuration_change",
            TaskCategory::DependencyChange => "dependency_change",
            TaskCategory::Documentation => "documentation",
            TaskCategory::TestOnly => "test_only",
            TaskCategory::Prototype => "prototype",
            TaskCategory::Unknown => "unknown",
        }
    }

    /// Suggested initial Fire Level for the category (never Five-Alarm).
    pub fn suggested_fire_level(self) -> FireLevel {
        match self {
            TaskCategory::BugFix => FireLevel::Containment,
            TaskCategory::FeatureAddition => FireLevel::Containment,
            TaskCategory::Refactor => FireLevel::Smolder,
            TaskCategory::ArchitectureWork => FireLevel::Ember,
            TaskCategory::SecurityRemediation => FireLevel::Critical,
            TaskCategory::PerformanceOptimization => FireLevel::Containment,
            TaskCategory::ConfigurationChange => FireLevel::Smolder,
            TaskCategory::DependencyChange => FireLevel::Critical,
            TaskCategory::Documentation => FireLevel::Ember,
            TaskCategory::TestOnly => FireLevel::Smolder,
            TaskCategory::Prototype => FireLevel::Ember,
            TaskCategory::Unknown => FireLevel::Containment,
        }
    }

    /// Pressure scenario family for this category.
    pub fn pressure_family(self) -> &'static str {
        match self {
            TaskCategory::BugFix => "production_incident",
            TaskCategory::FeatureAddition => "release_freeze",
            TaskCategory::Refactor | TaskCategory::ArchitectureWork => "limited_maintenance_window",
            TaskCategory::SecurityRemediation => "breach_containment",
            TaskCategory::PerformanceOptimization => "production_incident",
            TaskCategory::ConfigurationChange | TaskCategory::DependencyChange => "release_freeze",
            TaskCategory::Documentation | TaskCategory::TestOnly => "context_fire",
            TaskCategory::Prototype => "context_fire",
            TaskCategory::Unknown => "context_fire",
        }
    }
}

impl fmt::Display for TaskCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for TaskCategory {
    type Err = TifError;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "bug_fix" | "bug" | "fix" => Ok(TaskCategory::BugFix),
            "feature_addition" | "feature" => Ok(TaskCategory::FeatureAddition),
            "refactor" => Ok(TaskCategory::Refactor),
            "architecture_work" | "architecture" => Ok(TaskCategory::ArchitectureWork),
            "security_remediation" | "security" => Ok(TaskCategory::SecurityRemediation),
            "performance_optimization" | "performance" | "perf" => {
                Ok(TaskCategory::PerformanceOptimization)
            }
            "configuration_change" | "config" => Ok(TaskCategory::ConfigurationChange),
            "dependency_change" | "dependency" | "deps" => Ok(TaskCategory::DependencyChange),
            "documentation" | "docs" => Ok(TaskCategory::Documentation),
            "test_only" | "test" | "tests" => Ok(TaskCategory::TestOnly),
            "prototype" | "exploration" => Ok(TaskCategory::Prototype),
            "unknown" | "" => Ok(TaskCategory::Unknown),
            other => Err(TifError::Other(format!("unknown task category: {other}"))),
        }
    }
}

/// Heuristic task classifier (MVP: keyword-based; no network).
#[derive(Debug, Default)]
pub struct TaskClassifier;

impl TaskClassifier {
    pub fn new() -> Self {
        Self
    }

    /// Classify from free-form task text.
    pub fn classify(&self, task_text: &str) -> TaskCategory {
        let t = task_text.to_ascii_lowercase();

        let rules: &[(&[&str], TaskCategory)] = &[
            (
                &[
                    "cve",
                    "xss",
                    "injection",
                    "auth bypass",
                    "security",
                    "vulnerability",
                ],
                TaskCategory::SecurityRemediation,
            ),
            (
                &[
                    "fix bug",
                    "bugfix",
                    "regression",
                    "hotfix",
                    "crash",
                    "null pointer",
                ],
                TaskCategory::BugFix,
            ),
            (
                &[
                    "add feature",
                    "implement",
                    "new endpoint",
                    "new command",
                    "support for",
                ],
                TaskCategory::FeatureAddition,
            ),
            (
                &["refactor", "clean up", "restructure", "simplify code"],
                TaskCategory::Refactor,
            ),
            (
                &["architecture", "redesign system", "module boundaries"],
                TaskCategory::ArchitectureWork,
            ),
            (
                &["performance", "latency", "optimize", "slow", "throughput"],
                TaskCategory::PerformanceOptimization,
            ),
            (
                &["config", "configuration", "settings.toml", "env var"],
                TaskCategory::ConfigurationChange,
            ),
            (
                &[
                    "dependency",
                    "upgrade crate",
                    "bump version",
                    "cargo update",
                    "npm install",
                ],
                TaskCategory::DependencyChange,
            ),
            (
                &["documentation", "readme", "docs only", "doc comment"],
                TaskCategory::Documentation,
            ),
            (
                &["unit test", "add tests", "test coverage", "only tests"],
                TaskCategory::TestOnly,
            ),
            (
                &["prototype", "spike", "explore", "experiment"],
                TaskCategory::Prototype,
            ),
        ];

        for (keywords, category) in rules {
            if keywords.iter().any(|k| t.contains(k)) {
                return *category;
            }
        }
        TaskCategory::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_security() {
        let c = TaskClassifier::new();
        assert_eq!(
            c.classify("Fix XSS vulnerability in login form"),
            TaskCategory::SecurityRemediation
        );
    }

    #[test]
    fn classifies_bug() {
        let c = TaskClassifier::new();
        assert_eq!(
            c.classify("hotfix crash on empty input"),
            TaskCategory::BugFix
        );
    }

    #[test]
    fn never_suggests_five_alarm() {
        for cat in [
            TaskCategory::BugFix,
            TaskCategory::SecurityRemediation,
            TaskCategory::DependencyChange,
        ] {
            assert!(cat.suggested_fire_level().is_initial_selectable());
        }
    }
}

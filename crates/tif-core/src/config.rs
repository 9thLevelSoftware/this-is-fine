//! Configuration loading, validation, and merge precedence.
//!
//! Merge order (later overrides earlier):
//! 1. Global defaults
//! 2. Shared repository configuration (`.this-is-fine.toml`)
//! 3. Local repository overrides (`.this-is-fine.local.toml`)
//! 4. Task-derived policy (applied at compile time, not here)
//! 5. Current Fire Level (applied at compile time)
//! 6. Adaptive model-specific adjustments
//! 7. Explicit command-line overrides

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, TifError};
use crate::fire_level::FireLevel;

/// Current supported schema version for `.this-is-fine.toml`.
pub const SCHEMA_VERSION: u32 = 1;

pub const SHARED_CONFIG_NAME: &str = ".this-is-fine.toml";
pub const LOCAL_CONFIG_NAME: &str = ".this-is-fine.local.toml";
pub const STATE_DIR_NAME: &str = ".this-is-fine";

/// Fully resolved configuration after merge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub version: u32,
    pub enabled: bool,
    pub default_fire_level: u8,
    pub verification: VerificationConfig,
    pub simplicity: SimplicityConfig,
    pub approval: ApprovalConfig,
    pub audit: AuditConfig,
    pub rollback: RollbackConfig,
    pub reviewers: Vec<ReviewerConfig>,
    pub ci: CiConfig,
    pub pressure: PressureConfig,
    pub exclusions: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            enabled: true,
            default_fire_level: FireLevel::Containment.as_u8(),
            verification: VerificationConfig::default(),
            simplicity: SimplicityConfig::default(),
            approval: ApprovalConfig::default(),
            audit: AuditConfig::default(),
            rollback: RollbackConfig::default(),
            reviewers: Vec::new(),
            ci: CiConfig::default(),
            pressure: PressureConfig::default(),
            exclusions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct VerificationConfig {
    #[serde(default)]
    pub commands: Vec<String>,
    /// When true, attempt safe discovery if `commands` is empty.
    #[serde(default = "default_true")]
    pub discover: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SimplicityConfig {
    #[serde(default)]
    pub weights: SimplicityWeights,
    #[serde(default)]
    pub limits: SimplicityLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SimplicityWeights {
    pub runtime_dependency: f64,
    pub new_file: f64,
    pub public_interface: f64,
    pub abstraction: f64,
    pub added_line: f64,
    pub unrelated_change: f64,
    #[serde(default = "default_config_surface_weight")]
    pub configuration_surface: f64,
    #[serde(default = "default_generated_code_weight")]
    pub generated_code: f64,
    #[serde(default = "default_duplication_weight")]
    pub duplication: f64,
}

fn default_config_surface_weight() -> f64 {
    10.0
}
fn default_generated_code_weight() -> f64 {
    5.0
}
fn default_duplication_weight() -> f64 {
    12.0
}

impl Default for SimplicityWeights {
    fn default() -> Self {
        Self {
            runtime_dependency: 100.0,
            new_file: 25.0,
            public_interface: 20.0,
            abstraction: 15.0,
            added_line: 1.0,
            unrelated_change: 50.0,
            configuration_surface: 10.0,
            generated_code: 5.0,
            duplication: 12.0,
        }
    }
}

/// Hard limits; `None` means no hard cap for that metric.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SimplicityLimits {
    pub new_runtime_dependencies: Option<u32>,
    pub new_files: Option<u32>,
    pub public_interfaces: Option<u32>,
    pub abstractions: Option<u32>,
    pub added_lines: Option<u32>,
    pub score: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApprovalConfig {
    #[serde(default)]
    pub sensitive_paths: Vec<String>,
    #[serde(default)]
    pub sensitive_task_classes: Vec<String>,
    /// When true, Firebreak application always requires explicit approval.
    #[serde(default)]
    pub require_firebreak_approval: bool,
    /// When true (default), non-sensitive Firebreak candidates that pass
    /// re-verify + ranking may be applied without an interactive approval step.
    #[serde(default = "default_true")]
    pub auto_apply_firebreak: bool,
    /// Optional hours until a pending approval expires (`None` = no expiry).
    #[serde(default)]
    pub approval_ttl_hours: Option<u32>,
}

impl Default for ApprovalConfig {
    fn default() -> Self {
        Self {
            sensitive_paths: Vec::new(),
            sensitive_task_classes: Vec::new(),
            require_firebreak_approval: false,
            auto_apply_firebreak: true,
            approval_ttl_hours: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditConfig {
    /// `metadata`, `redacted`, or `full`.
    #[serde(default = "default_audit_tier")]
    pub tier: String,
    #[serde(default = "default_max_age_days")]
    pub max_age_days: u32,
    #[serde(default = "default_max_size_mb")]
    pub max_size_mb: u32,
}

fn default_audit_tier() -> String {
    "redacted".into()
}
fn default_max_age_days() -> u32 {
    90
}
fn default_max_size_mb() -> u32 {
    1024
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            tier: default_audit_tier(),
            max_age_days: default_max_age_days(),
            max_size_mb: default_max_size_mb(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RollbackConfig {
    #[serde(default = "default_rollback_days")]
    pub max_days: u32,
    #[serde(default = "default_successful_commits")]
    pub successful_commits: u32,
}

fn default_rollback_days() -> u32 {
    7
}
fn default_successful_commits() -> u32 {
    3
}

impl Default for RollbackConfig {
    fn default() -> Self {
        Self {
            max_days: default_rollback_days(),
            successful_commits: default_successful_commits(),
        }
    }
}

/// User-authorized reviewer model (local config typically).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewerConfig {
    pub id: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Credential reference: `ENV_NAME`, `env:ENV_NAME`, or `file:/path/to/secret`.
    #[serde(default)]
    pub credential_ref: Option<String>,
    /// Hosted source egress requires explicit opt-in (local-first default: false).
    #[serde(default)]
    pub allow_source_egress: bool,
    #[serde(default)]
    pub eligible_task_types: Vec<String>,
    #[serde(default = "default_max_firebreak_attempts")]
    pub max_firebreak_attempts: u32,
    #[serde(default)]
    pub priority: i32,
    /// Request timeout for HTTP/process backends (seconds).
    #[serde(default = "default_reviewer_timeout_secs")]
    pub timeout_secs: u64,
    /// Soft max input tokens (advisory for context packaging).
    #[serde(default)]
    pub max_input_tokens: Option<u64>,
    /// Soft max output tokens for provider requests.
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    /// Max bytes of context body (prompts + optional source) before truncation.
    #[serde(default = "default_max_context_bytes")]
    pub max_context_bytes: u64,
    /// For `provider = "process"`: argv to invoke (placeholders: `{isolation}`, `{request_json}`).
    #[serde(default)]
    pub process_argv: Option<Vec<String>>,
}

fn default_max_firebreak_attempts() -> u32 {
    2
}

fn default_reviewer_timeout_secs() -> u64 {
    120
}

fn default_max_context_bytes() -> u64 {
    256_000
}

impl ReviewerConfig {
    /// Construct a minimal authorized reviewer (tests / defaults).
    pub fn mock(id: &str, priority: i32) -> Self {
        Self {
            id: id.into(),
            provider: "mock".into(),
            model: id.into(),
            endpoint: None,
            credential_ref: None,
            allow_source_egress: false,
            eligible_task_types: vec![],
            max_firebreak_attempts: default_max_firebreak_attempts(),
            priority,
            timeout_secs: default_reviewer_timeout_secs(),
            max_input_tokens: None,
            max_output_tokens: None,
            max_context_bytes: default_max_context_bytes(),
            process_argv: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CiConfig {
    /// `warn` or `fail` on policy violations.
    #[serde(default = "default_ci_mode")]
    pub on_violation: String,
    #[serde(default)]
    pub allow_write: bool,
    #[serde(default)]
    pub allow_pr: bool,
}

fn default_ci_mode() -> String {
    "fail".into()
}

impl Default for CiConfig {
    fn default() -> Self {
        Self {
            on_violation: default_ci_mode(),
            allow_write: false,
            allow_pr: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PressureConfig {
    /// Restrict allowed scenario families; empty means all curated families.
    #[serde(default)]
    pub allowed_families: Vec<String>,
    #[serde(default = "default_true")]
    pub include_baseline: bool,
}

impl Default for PressureConfig {
    fn default() -> Self {
        Self {
            allowed_families: Vec::new(),
            include_baseline: true,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Partial TOML file representation (shared or local).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigFile {
    pub version: Option<u32>,
    pub enabled: Option<bool>,
    pub default_fire_level: Option<u8>,
    pub verification: Option<VerificationConfig>,
    pub simplicity: Option<SimplicityPartial>,
    pub approval: Option<ApprovalConfig>,
    pub audit: Option<AuditPartial>,
    pub rollback: Option<RollbackPartial>,
    #[serde(default)]
    pub reviewers: Vec<ReviewerConfig>,
    pub ci: Option<CiPartial>,
    pub pressure: Option<PressurePartial>,
    #[serde(default)]
    pub exclusions: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SimplicityPartial {
    pub weights: Option<SimplicityWeightsPartial>,
    pub limits: Option<SimplicityLimits>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SimplicityWeightsPartial {
    pub runtime_dependency: Option<f64>,
    pub new_file: Option<f64>,
    pub public_interface: Option<f64>,
    pub abstraction: Option<f64>,
    pub added_line: Option<f64>,
    pub unrelated_change: Option<f64>,
    pub configuration_surface: Option<f64>,
    pub generated_code: Option<f64>,
    pub duplication: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditPartial {
    pub tier: Option<String>,
    pub max_age_days: Option<u32>,
    pub max_size_mb: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RollbackPartial {
    pub max_days: Option<u32>,
    pub successful_commits: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CiPartial {
    pub on_violation: Option<String>,
    pub allow_write: Option<bool>,
    pub allow_pr: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PressurePartial {
    pub allowed_families: Option<Vec<String>>,
    pub include_baseline: Option<bool>,
}

/// Paths related to a repository's This Is Fine state.
#[derive(Debug, Clone)]
pub struct RepoPaths {
    pub root: PathBuf,
    pub shared_config: PathBuf,
    pub local_config: PathBuf,
    pub state_dir: PathBuf,
}

impl RepoPaths {
    pub fn for_root(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            shared_config: root.join(SHARED_CONFIG_NAME),
            local_config: root.join(LOCAL_CONFIG_NAME),
            state_dir: root.join(STATE_DIR_NAME),
            root,
        }
    }

    pub fn db_path(&self) -> PathBuf {
        self.state_dir.join("audit.db")
    }

    pub fn artifacts_dir(&self) -> PathBuf {
        self.state_dir.join("artifacts")
    }

    pub fn snapshots_dir(&self) -> PathBuf {
        self.state_dir.join("snapshots")
    }
}

/// Load and merge configuration for a repository root.
pub fn load_config(root: &Path) -> Result<Config> {
    let paths = RepoPaths::for_root(root);
    let mut cfg = Config::default();

    if paths.shared_config.exists() {
        let shared = parse_config_file(&paths.shared_config)?;
        merge_file(&mut cfg, &shared)?;
    }

    if paths.local_config.exists() {
        let local = parse_config_file(&paths.local_config)?;
        merge_file(&mut cfg, &local)?;
    }

    validate_config(&cfg)?;
    Ok(cfg)
}

/// Parse a single config file from disk.
pub fn parse_config_file(path: &Path) -> Result<ConfigFile> {
    let text = fs::read_to_string(path)
        .map_err(|e| TifError::Config(format!("failed to read {}: {e}", path.display())))?;
    parse_config_str(&text)
}

/// Parse TOML configuration text.
pub fn parse_config_str(text: &str) -> Result<ConfigFile> {
    let file: ConfigFile = toml::from_str(text)?;
    if let Some(v) = file.version {
        if v != SCHEMA_VERSION {
            return Err(TifError::UnsupportedSchema {
                found: v,
                expected: SCHEMA_VERSION,
            });
        }
    }
    Ok(file)
}

/// Merge a partial file into an accumulated config.
pub fn merge_file(base: &mut Config, file: &ConfigFile) -> Result<()> {
    if let Some(v) = file.version {
        base.version = v;
    }
    if let Some(e) = file.enabled {
        base.enabled = e;
    }
    if let Some(fl) = file.default_fire_level {
        // Five-Alarm cannot be a default initial level.
        FireLevel::parse_initial(fl)?;
        base.default_fire_level = fl;
    }
    if let Some(ref v) = file.verification {
        base.verification = v.clone();
    }
    if let Some(ref s) = file.simplicity {
        merge_simplicity(&mut base.simplicity, s);
    }
    if let Some(ref a) = file.approval {
        base.approval = a.clone();
    }
    if let Some(ref a) = file.audit {
        if let Some(ref t) = a.tier {
            base.audit.tier = t.clone();
        }
        if let Some(d) = a.max_age_days {
            base.audit.max_age_days = d;
        }
        if let Some(s) = a.max_size_mb {
            base.audit.max_size_mb = s;
        }
    }
    if let Some(ref r) = file.rollback {
        if let Some(d) = r.max_days {
            base.rollback.max_days = d;
        }
        if let Some(c) = r.successful_commits {
            base.rollback.successful_commits = c;
        }
    }
    if !file.reviewers.is_empty() {
        // Local reviewers replace/extend: append unique by id, later wins.
        for rev in &file.reviewers {
            if let Some(existing) = base.reviewers.iter_mut().find(|r| r.id == rev.id) {
                *existing = rev.clone();
            } else {
                base.reviewers.push(rev.clone());
            }
        }
    }
    if let Some(ref c) = file.ci {
        if let Some(ref m) = c.on_violation {
            base.ci.on_violation = m.clone();
        }
        if let Some(w) = c.allow_write {
            base.ci.allow_write = w;
        }
        if let Some(p) = c.allow_pr {
            base.ci.allow_pr = p;
        }
    }
    if let Some(ref p) = file.pressure {
        if let Some(ref f) = p.allowed_families {
            base.pressure.allowed_families = f.clone();
        }
        if let Some(b) = p.include_baseline {
            base.pressure.include_baseline = b;
        }
    }
    if !file.exclusions.is_empty() {
        base.exclusions = file.exclusions.clone();
    }
    Ok(())
}

fn merge_simplicity(base: &mut SimplicityConfig, partial: &SimplicityPartial) {
    if let Some(ref w) = partial.weights {
        if let Some(v) = w.runtime_dependency {
            base.weights.runtime_dependency = v;
        }
        if let Some(v) = w.new_file {
            base.weights.new_file = v;
        }
        if let Some(v) = w.public_interface {
            base.weights.public_interface = v;
        }
        if let Some(v) = w.abstraction {
            base.weights.abstraction = v;
        }
        if let Some(v) = w.added_line {
            base.weights.added_line = v;
        }
        if let Some(v) = w.unrelated_change {
            base.weights.unrelated_change = v;
        }
        if let Some(v) = w.configuration_surface {
            base.weights.configuration_surface = v;
        }
        if let Some(v) = w.generated_code {
            base.weights.generated_code = v;
        }
        if let Some(v) = w.duplication {
            base.weights.duplication = v;
        }
    }
    if let Some(ref l) = partial.limits {
        merge_limits_fieldwise(&mut base.limits, l);
    }
}

/// Merge hard limits field-by-field so a partial local table does not wipe shared caps.
fn merge_limits_fieldwise(base: &mut SimplicityLimits, patch: &SimplicityLimits) {
    if patch.new_runtime_dependencies.is_some() {
        base.new_runtime_dependencies = patch.new_runtime_dependencies;
    }
    if patch.new_files.is_some() {
        base.new_files = patch.new_files;
    }
    if patch.public_interfaces.is_some() {
        base.public_interfaces = patch.public_interfaces;
    }
    if patch.abstractions.is_some() {
        base.abstractions = patch.abstractions;
    }
    if patch.added_lines.is_some() {
        base.added_lines = patch.added_lines;
    }
    if patch.score.is_some() {
        base.score = patch.score;
    }
}

/// Validate a fully merged config.
pub fn validate_config(cfg: &Config) -> Result<()> {
    if cfg.version != SCHEMA_VERSION {
        return Err(TifError::UnsupportedSchema {
            found: cfg.version,
            expected: SCHEMA_VERSION,
        });
    }
    FireLevel::parse_initial(cfg.default_fire_level)?;

    let tier = cfg.audit.tier.as_str();
    if !matches!(tier, "metadata" | "redacted" | "full") {
        return Err(TifError::Config(format!(
            "invalid audit.tier '{tier}' (expected metadata|redacted|full)"
        )));
    }

    if !matches!(cfg.ci.on_violation.as_str(), "warn" | "fail") {
        return Err(TifError::Config(format!(
            "invalid ci.on_violation '{}' (expected warn|fail)",
            cfg.ci.on_violation
        )));
    }

    for rev in &cfg.reviewers {
        if rev.id.is_empty() || rev.provider.is_empty() || rev.model.is_empty() {
            return Err(TifError::Config(
                "reviewer entries require non-empty id, provider, and model".into(),
            ));
        }
    }

    Ok(())
}

/// Apply explicit CLI overrides after load.
pub fn apply_cli_overrides(
    cfg: &mut Config,
    enabled: Option<bool>,
    fire_level: Option<u8>,
) -> Result<()> {
    if let Some(e) = enabled {
        cfg.enabled = e;
    }
    if let Some(fl) = fire_level {
        FireLevel::parse_initial(fl)?;
        cfg.default_fire_level = fl;
    }
    validate_config(cfg)?;
    Ok(())
}

/// Default shared configuration content written by `tif init`.
pub fn default_shared_toml() -> String {
    r#"# This Is Fine — shared repository configuration (commit this file)
version = 1
enabled = true
default_fire_level = 3

[verification]
# Prefer explicit commands. Leave empty and set discover = true to attempt safe discovery.
commands = []
discover = true

[simplicity.weights]
runtime_dependency = 100
new_file = 25
public_interface = 20
abstraction = 15
added_line = 1
unrelated_change = 50

[simplicity.limits]
new_runtime_dependencies = 0

[approval]
sensitive_paths = []
require_firebreak_approval = false
auto_apply_firebreak = true
# approval_ttl_hours = 72

[audit]
tier = "redacted"
max_age_days = 90
max_size_mb = 1024

[rollback]
max_days = 7
successful_commits = 3

[ci]
on_violation = "fail"
allow_write = false
allow_pr = false

[pressure]
include_baseline = true
"#
    .to_string()
}

/// Default local configuration content written by `tif init`.
pub fn default_local_toml() -> String {
    r#"# This Is Fine — machine-local configuration (do not commit)
version = 1

# Authorized Firebreak reviewers (user must explicitly authorize each model).
# Offline mock (safe for tif reviewer test / CI):
# [[reviewers]]
# id = "local-mock"
# provider = "mock"
# model = "fixture"
# allow_source_egress = false
# priority = 100

# OpenAI-compatible hosted example (egress stays false by default):
# [[reviewers]]
# id = "hosted"
# provider = "openai_compatible"
# model = "gpt-4.1-mini"
# endpoint = "https://api.openai.com/v1"
# credential_ref = "env:TIF_REVIEWER_API_KEY"
# allow_source_egress = false
# timeout_secs = 120
# max_output_tokens = 4096
# priority = 10

# Local process backend:
# [[reviewers]]
# id = "local-cli"
# provider = "process"
# model = "custom"
# process_argv = ["my-reviewer", "--isolation", "{isolation}", "--request", "{request_json}"]
# allow_source_egress = false
"#
    .to_string()
}

/// Ensure state directory structure exists.
pub fn ensure_state_dirs(paths: &RepoPaths) -> Result<()> {
    fs::create_dir_all(&paths.state_dir)?;
    fs::create_dir_all(paths.artifacts_dir())?;
    fs::create_dir_all(paths.snapshots_dir())?;
    Ok(())
}

/// Initialize repository config files if missing.
pub fn init_repository(root: &Path, force: bool) -> Result<RepoPaths> {
    let paths = RepoPaths::for_root(root);
    ensure_state_dirs(&paths)?;

    if !paths.shared_config.exists() || force {
        fs::write(&paths.shared_config, default_shared_toml())?;
    }
    if !paths.local_config.exists() || force {
        fs::write(&paths.local_config, default_local_toml())?;
    }

    // Ensure local config is listed in .gitignore if present.
    let gitignore = paths.root.join(".gitignore");
    if gitignore.exists() {
        let content = fs::read_to_string(&gitignore)?;
        let needle = LOCAL_CONFIG_NAME;
        if !content.lines().any(|l| l.trim() == needle) {
            let mut updated = content;
            if !updated.ends_with('\n') && !updated.is_empty() {
                updated.push('\n');
            }
            updated.push_str(&format!(
                "\n# This Is Fine local overrides\n{needle}\n{STATE_DIR_NAME}/\n"
            ));
            fs::write(&gitignore, updated)?;
        }
    }

    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn defaults_validate() {
        let cfg = Config::default();
        validate_config(&cfg).unwrap();
    }

    #[test]
    fn shared_then_local_precedence() {
        let mut cfg = Config::default();
        let shared = ConfigFile {
            default_fire_level: Some(2),
            enabled: Some(true),
            ..Default::default()
        };
        merge_file(&mut cfg, &shared).unwrap();
        assert_eq!(cfg.default_fire_level, 2);

        let local = ConfigFile {
            default_fire_level: Some(4),
            enabled: Some(false),
            ..Default::default()
        };
        merge_file(&mut cfg, &local).unwrap();
        assert_eq!(cfg.default_fire_level, 4);
        assert!(!cfg.enabled);
    }

    #[test]
    fn rejects_five_alarm_as_default() {
        let mut cfg = Config::default();
        let file = ConfigFile {
            default_fire_level: Some(5),
            ..Default::default()
        };
        assert!(merge_file(&mut cfg, &file).is_err());
    }

    #[test]
    fn unsupported_schema() {
        let text = "version = 99\nenabled = true\n";
        assert!(matches!(
            parse_config_str(text),
            Err(TifError::UnsupportedSchema { found: 99, .. })
        ));
    }

    #[test]
    fn weight_override_partial() {
        let mut cfg = Config::default();
        let file = ConfigFile {
            simplicity: Some(SimplicityPartial {
                weights: Some(SimplicityWeightsPartial {
                    new_file: Some(99.0),
                    ..Default::default()
                }),
                limits: Some(SimplicityLimits {
                    new_runtime_dependencies: Some(0),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };
        merge_file(&mut cfg, &file).unwrap();
        assert_eq!(cfg.simplicity.weights.new_file, 99.0);
        assert_eq!(cfg.simplicity.weights.added_line, 1.0); // untouched default
        assert_eq!(cfg.simplicity.limits.new_runtime_dependencies, Some(0));
    }

    #[test]
    fn limits_merge_fieldwise_does_not_wipe_prior_caps() {
        let mut cfg = Config::default();
        // Shared sets a hard dep cap.
        let shared = ConfigFile {
            simplicity: Some(SimplicityPartial {
                limits: Some(SimplicityLimits {
                    new_runtime_dependencies: Some(0),
                    new_files: Some(3),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        merge_file(&mut cfg, &shared).unwrap();
        assert_eq!(cfg.simplicity.limits.new_runtime_dependencies, Some(0));
        assert_eq!(cfg.simplicity.limits.new_files, Some(3));

        // Local only overrides added_lines — must not clear dep/file caps.
        let local = ConfigFile {
            simplicity: Some(SimplicityPartial {
                limits: Some(SimplicityLimits {
                    added_lines: Some(50),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        merge_file(&mut cfg, &local).unwrap();
        assert_eq!(cfg.simplicity.limits.new_runtime_dependencies, Some(0));
        assert_eq!(cfg.simplicity.limits.new_files, Some(3));
        assert_eq!(cfg.simplicity.limits.added_lines, Some(50));
    }

    #[test]
    fn allow_source_egress_defaults_false() {
        let text = r#"
[[reviewers]]
id = "r1"
provider = "mock"
model = "m"
"#;
        let file = parse_config_str(text).unwrap();
        assert_eq!(file.reviewers.len(), 1);
        assert!(!file.reviewers[0].allow_source_egress);
    }

    #[test]
    fn init_and_load_roundtrip() {
        let dir = tempdir().unwrap();
        init_repository(dir.path(), false).unwrap();
        let cfg = load_config(dir.path()).unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.default_fire_level, 3);
        assert_eq!(cfg.version, SCHEMA_VERSION);
    }
}

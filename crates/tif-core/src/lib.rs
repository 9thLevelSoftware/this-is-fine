//! This Is Fine — core library.
//!
//! Local-first adaptive restraint and simplification for coding agents.
//!
//! Product promise: **Contain the fire. Do not remodel the building.**

pub mod adaptation;
pub mod assess;
pub mod audit;
pub mod config;
pub mod error;
pub mod fire_level;
pub mod firebreak;
pub mod inspector;
pub mod isolation;
pub mod orchestrator;
pub mod policy;
pub mod pressure;
pub mod protocol;
pub mod reviewer;
pub mod scoring;
pub mod task;
pub mod verify;

pub use adaptation::AdaptationEngine;
pub use assess::{AssessmentStatus, DamageAssessment, DamageAssessor};
pub use audit::{compact_status, redact_secrets, AuditStore, AuditTier};
pub use config::{
    apply_cli_overrides, default_local_toml, default_shared_toml, ensure_state_dirs,
    init_repository, load_config, validate_config, Config, RepoPaths, SCHEMA_VERSION,
};
pub use error::{Result, TifError};
pub use fire_level::FireLevel;
pub use firebreak::{fail_safe_guard, FirebreakEngine, FirebreakOutcome, FiveAlarmPlan};
pub use inspector::{find_repo_root, RepositoryInspector};
pub use isolation::{select_isolator, IsolationKind, IsolationSession, Isolator};
pub use orchestrator::{BeginRunRequest, RunId, RunOrchestrator, RunRecord, RunState};
pub use policy::{ContainmentPolicy, PolicyCompileRequest, PolicyCompiler};
pub use pressure::{PressureEngine, PressureScenario, BASELINE_CONTAINMENT_PROMPT};
pub use protocol::{JsonResponse, PROTOCOL_VERSION};
pub use reviewer::ReviewerSelector;
pub use scoring::{
    select_smaller_verified, CorrectnessFloor, DiffMetrics, ScoreResult, SimplicityScorer,
};
pub use task::{TaskCategory, TaskClassifier};
pub use verify::{plan_and_run, VerificationPlanner, VerificationReport, VerificationRunner};

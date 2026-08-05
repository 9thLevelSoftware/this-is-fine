//! This Is Fine — core library.
//!
//! Local-first adaptive restraint and simplification for coding agents.
//!
//! Product promise: **Contain the fire. Do not remodel the building.**

pub mod adaptation;
pub mod assess;
pub mod audit;
pub mod config;
pub mod credentials;
pub mod diff;
pub mod error;
pub mod fire_level;
pub mod firebreak;
pub mod inspector;
pub mod isolation;
pub mod orchestrator;
pub mod policy;
pub mod pressure;
pub mod protocol;
pub mod providers;
pub mod reviewer;
pub mod scoring;
pub mod task;
pub mod verify;

pub use adaptation::{
    assert_adaptation_safety, AdaptationEngine, AdaptationRecommendation, AdaptationStats,
    AppliedKnobs, PromotionGate, SelfApplyKnob, SelfApplyValue, VariantAction, VariantDecision,
    VariantStatus,
};
pub use assess::{
    analyze_test_change_diff, AssessmentStatus, DamageAssessment, DamageAssessor, TestChange,
};
pub use audit::{
    compact_status, looks_like_binary_artifact, redact_secrets, AuditStore, AuditTier, GcReport,
    IsolationGcOpts,
};
pub use config::{
    apply_cli_overrides, default_local_toml, default_shared_toml, ensure_state_dirs,
    init_repository, load_config, validate_config, Config, RepoPaths, SCHEMA_VERSION,
};
pub use credentials::{resolve_credential, resolve_credential_opt, secrets_dir};
pub use diff::{
    apply_generated_code_heuristics, dependency_delta_between_trees, dependency_delta_from_texts,
    dependency_delta_vs_git_head, metrics_from_git, metrics_from_tree_absolute,
    metrics_from_tree_diff, metrics_from_unified_diff, metrics_from_unified_diff_checked,
    path_looks_generated, path_looks_like_test, DependencyDelta, MAX_UNIFIED_DIFF_BYTES,
};
pub use error::{Result, TifError};
pub use fire_level::FireLevel;
pub use firebreak::{
    fail_safe_guard, BackendGenerateRequest, BackendGenerateResult, FirebreakEngine,
    FirebreakOutcome, FirebreakRequest, FiveAlarmCandidate, FiveAlarmCandidateKind, FiveAlarmPlan,
    FiveAlarmRunOptions, FiveAlarmRunResult, FiveAlarmStage, FiveAlarmTimelineEntry,
    IsolatedApplyRequest,
};
pub use inspector::{find_repo_root, RepositoryInspector};
pub use isolation::{
    apply_verified_candidate, count_commits_since, gc_expired_isolation,
    gc_with_rollback_retention, isolator_for_session, open_isolation, rollback_applied_candidate,
    select_isolator, session_candidate_root, ApplyLock, GitWorktreeIsolator, IsolationKind,
    IsolationSession, Isolator, RollbackRetention, SnapshotIsolator,
};
pub use orchestrator::{
    BeginRunRequest, FirebreakAutoOptions, IsolatedFirebreakParams, RunId, RunOrchestrator,
    RunRecord, RunState,
};
pub use policy::{ContainmentPolicy, PolicyCompileRequest, PolicyCompiler};
pub use pressure::{
    PressureEngine, PressureScenario, ScenarioTemplate, BASELINE_CONTAINMENT_PROMPT,
};
pub use protocol::{JsonResponse, PROTOCOL_VERSION};
pub use providers::{
    assert_clean_room_package, backend_for_provider, build_reviewer_context, BackendRegistry,
    ContextBuildRequest, ProbeResult, ReviewerBackend, ReviewerInvocationMode, ReviewerPatch,
    ReviewerTask,
};
pub use reviewer::{ReviewerSelector, ReviewerStats, SelectedReviewer};
pub use scoring::{
    select_smaller_verified, CorrectnessFloor, DiffMetrics, ScoreResult, SimplicityScorer,
    SCORING_VERSION,
};
pub use task::{TaskCategory, TaskClassifier};
pub use verify::{plan_and_run, VerificationPlanner, VerificationReport, VerificationRunner};

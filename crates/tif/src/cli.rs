//! CLI definition for `tif`.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// This Is Fine — containment and simplification governor for coding agents.
#[derive(Debug, Parser)]
#[command(name = "tif")]
#[command(about = "Contain the fire. Do not remodel the building.", long_about = None)]
#[command(version)]
pub struct Cli {
    /// Emit versioned JSON for agent adapters.
    #[arg(long, global = true)]
    pub json: bool,

    /// Repository root (defaults to cwd / discovered root).
    #[arg(long, global = true)]
    pub repo: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Initialize shared and local configuration.
    Init {
        /// Overwrite existing config files.
        #[arg(long)]
        force: bool,
    },
    /// Enable containment for the repository.
    On,
    /// Suspend containment for the repository.
    Off,
    /// Run lifecycle operations.
    #[command(subcommand)]
    Run(RunCmd),
    /// Produce a Damage Assessment for given metrics (or CI).
    Assess {
        #[arg(long, default_value_t = 0)]
        files_added: u32,
        #[arg(long, default_value_t = 0)]
        files_changed: u32,
        #[arg(long, default_value_t = 0)]
        lines_added: u32,
        #[arg(long, default_value_t = 0)]
        lines_removed: u32,
        #[arg(long, default_value_t = 0)]
        deps_added: u32,
        #[arg(long)]
        fire_level: Option<u8>,
        /// Derive metrics from `git status` / `git diff` in the repository.
        #[arg(long)]
        from_git: bool,
        /// Read a unified diff from this file (or `-` for stdin).
        #[arg(long)]
        from_diff: Option<PathBuf>,
    },
    /// Run Firebreak simplification (isolated; fail-safe).
    Firebreak {
        #[arg(long)]
        run_id: Option<String>,
        #[arg(long, default_value_t = 0)]
        files_added: u32,
        #[arg(long, default_value_t = 0)]
        files_changed: u32,
        #[arg(long, default_value_t = 0)]
        lines_added: u32,
        #[arg(long, default_value_t = 0)]
        deps_added: u32,
        /// Derive original metrics from git when not using a prior run.
        #[arg(long)]
        from_git: bool,
        /// Stage this directory as the isolated candidate tree and re-verify before apply.
        #[arg(long)]
        candidate: Option<PathBuf>,
        /// Authorize filesystem apply of a re-verified smaller candidate.
        #[arg(long)]
        apply: bool,
        /// Invoke authorized reviewer backend in isolation (no apply; Phase 1).
        #[arg(long)]
        invoke_backend: bool,
        /// Automatic closed loop: backend → re-verify → rank → apply if allowed.
        /// Same path as `tif run complete --auto-firebreak` when reviewers are configured.
        #[arg(long)]
        auto: bool,
        /// Optional task text for backend context.
        #[arg(long)]
        task: Option<String>,
    },
    /// Approve a pending Firebreak candidate (`AwaitingApproval`) and apply.
    Approve { run_id: String },
    /// Reject a pending Firebreak candidate; original workspace is retained.
    Reject { run_id: String },
    /// Get or set Fire Level (1–4 initial; 5 is escalation-only).
    #[command(name = "fire-level")]
    FireLevel {
        /// Fire level 1–4 to set; omit to get current level.
        level: Option<u8>,
    },
    /// Roll back to the original candidate for a run.
    Rollback { run_id: String },
    /// Containment policy operations.
    #[command(subcommand)]
    Policy(PolicyCmd),
    /// Plan and run verification.
    Verify {
        #[arg(long)]
        dry_run: bool,
    },
    /// Audit log inspection and retention.
    Audit {
        /// Show recent runs (default).
        #[arg(long, default_value_t = true)]
        show: bool,
        #[arg(long)]
        limit: Option<usize>,
        /// Run garbage collection (audit age+size and isolation retention).
        #[arg(long)]
        gc: bool,
        /// Purge all local audit data.
        #[arg(long)]
        purge: bool,
    },
    /// Inspect repository signals (languages, verification discovery).
    Inspect,
    /// Compact containment status.
    Status,
    /// Five-Alarm staged recovery (post-failure only; design §8.2).
    #[command(name = "five-alarm")]
    FiveAlarm {
        /// Print staged recovery plan outline.
        #[arg(long)]
        plan: bool,
        /// Run Five-Alarm staged recovery for an existing out-of-control run.
        #[arg(long)]
        run: Option<String>,
        /// Authorize apply of the smallest verified candidate (default: config auto_apply_firebreak).
        #[arg(long)]
        apply: bool,
        /// Note historical risk (still insufficient without current containment failure).
        #[arg(long)]
        historical_risk: bool,
    },
    /// Interactive TUI (keyboard-driven; every action has a CLI equivalent).
    Tui,
    /// Local adaptation stats, recommendations, and self-apply (allowlisted knobs only).
    #[command(subcommand)]
    Adaptation(AdaptationCmd),
    /// Authorized reviewer pool operations (local config only).
    #[command(subcommand)]
    Reviewer(ReviewerCmd),
}

/// Local adaptation: status, recommend, reset (no telemetry).
#[derive(Debug, Subcommand)]
pub enum AdaptationCmd {
    /// Show local adaptation stats and applied knobs (default).
    Status,
    /// Show recommendations for a task category.
    Recommend {
        /// Task category string (e.g. bug_fix, feature_addition). Default: unknown.
        #[arg(long)]
        category: Option<String>,
        /// Self-apply allowlisted knobs from the recommendation (fire level ≤4, thresholds, template).
        #[arg(long)]
        apply: bool,
    },
    /// Clear local adaptation stats and applied knobs.
    Reset,
}

/// Reviewer pool: list, probe connectivity, offline test with mock.
#[derive(Debug, Subcommand)]
pub enum ReviewerCmd {
    /// List authorized reviewers from config and compiled backends.
    List,
    /// Probe connectivity / readiness for one reviewer (or all).
    Probe {
        /// Reviewer id; omit to probe all.
        #[arg(long)]
        id: Option<String>,
    },
    /// Offline dry-run: invoke mock/process backend in a temp isolation tree (no apply).
    Test {
        /// Reviewer id (default: first mock-capable entry, else first pool entry).
        #[arg(long)]
        id: Option<String>,
        /// Optional task text for context packaging.
        #[arg(long)]
        task: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum PolicyCmd {
    /// Resolve/compile the effective containment policy.
    Resolve {
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        fire_level: Option<u8>,
    },
}

#[derive(Debug, Subcommand)]
pub enum RunCmd {
    /// Begin a containment run (policy + pressure).
    Begin {
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        fire_level: Option<u8>,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// Run even if containment is disabled.
        #[arg(long)]
        force: bool,
    },
    /// Signal implementation complete and score/verify.
    Complete {
        run_id: String,
        #[arg(long, default_value_t = 0)]
        files_added: u32,
        #[arg(long, default_value_t = 0)]
        files_changed: u32,
        #[arg(long, default_value_t = 0)]
        lines_added: u32,
        #[arg(long, default_value_t = 0)]
        lines_removed: u32,
        #[arg(long, default_value_t = 0)]
        deps_added: u32,
        #[arg(long)]
        auto_firebreak: bool,
        /// Derive metrics from git working tree instead of explicit flags.
        #[arg(long)]
        from_git: bool,
        /// Trusted-adapter signal: verification already ran outside tif.
        /// Not re-executed; omit in CI and use `tif verify` instead.
        #[arg(long)]
        verification_passed: Option<bool>,
    },
    /// Show a run by id.
    Show { run_id: String },
    /// Status of containment (alias of `tif status`).
    Status,
}

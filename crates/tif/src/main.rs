//! This Is Fine CLI (`tif`).

mod cli;
mod tui_app;

use anyhow::Context;
use clap::Parser;
use cli::{AdaptationCmd, Cli, Commands, PolicyCmd, ReviewerCmd, RunCmd};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tif_core::assess::{AssessmentStatus, DamageAssessor};
use tif_core::audit::{compact_status, AuditStore};
use tif_core::config::{
    apply_cli_overrides, ensure_state_dirs, init_repository, load_config, RepoPaths,
    SHARED_CONFIG_NAME,
};
use tif_core::credentials::resolve_credential_opt;
use tif_core::diff::{
    metrics_from_git, metrics_from_tree_absolute, metrics_from_unified_diff_checked,
};
use tif_core::fire_level::FireLevel;
use tif_core::firebreak::{
    candidate_floor_from_verification, BackendGenerateRequest, FirebreakEngine,
};
use tif_core::inspector::{find_repo_root, RepositoryInspector};
use tif_core::isolation::isolator_for_session;
use tif_core::orchestrator::{BeginRunRequest, RunOrchestrator, RunState};
use tif_core::policy::{PolicyCompileRequest, PolicyCompiler};
use tif_core::protocol::{
    AssessResult, FireLevelResult, JsonResponse, PolicyResolveResult, RollbackResult,
    RunBeginResult, VerifyResult,
};
use tif_core::providers::{backend_for_provider, BackendRegistry};
use tif_core::reviewer::ReviewerSelector;
use tif_core::scoring::{CorrectnessFloor, DiffMetrics, SimplicityScorer};
use tif_core::verify::{plan_and_run, VerificationPlanner};
use tif_core::{AdaptationEngine, FiveAlarmPlan, FiveAlarmRunOptions};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(1)
        }
    }
}

fn run() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();
    let json = cli.json;
    let root = resolve_root(cli.repo.clone())?;

    match cli.command {
        Commands::Init { force } => cmd_init(&root, force, json),
        Commands::On => cmd_set_enabled(&root, true, json),
        Commands::Off => cmd_set_enabled(&root, false, json),
        Commands::Run(sub) => match sub {
            RunCmd::Begin {
                task,
                fire_level,
                agent,
                model,
                force,
            } => cmd_run_begin(&root, task, fire_level, agent, model, force, json),
            RunCmd::Complete {
                run_id,
                files_added,
                files_changed,
                lines_added,
                lines_removed,
                deps_added,
                auto_firebreak,
                from_git,
                verification_passed,
            } => {
                let metrics = resolve_metrics(
                    &root,
                    from_git,
                    None,
                    DiffMetrics {
                        runtime_dependencies_added: deps_added,
                        files_added,
                        files_changed,
                        files_deleted: 0,
                        lines_added,
                        lines_removed,
                        ..Default::default()
                    },
                )?;
                cmd_run_complete(
                    &root,
                    &run_id,
                    metrics,
                    auto_firebreak,
                    verification_passed,
                    json,
                )
            }
            RunCmd::Show { run_id } => cmd_run_show(&root, &run_id, json),
            RunCmd::Status => cmd_run_status(&root, json),
        },
        Commands::Assess {
            files_added,
            files_changed,
            lines_added,
            lines_removed,
            deps_added,
            fire_level,
            from_git,
            from_diff,
        } => {
            let metrics = resolve_metrics(
                &root,
                from_git,
                from_diff.as_ref(),
                DiffMetrics {
                    runtime_dependencies_added: deps_added,
                    files_added,
                    files_changed,
                    lines_added,
                    lines_removed,
                    ..Default::default()
                },
            )?;
            cmd_assess(&root, metrics, fire_level, json)
        }
        Commands::Firebreak {
            run_id,
            files_added,
            files_changed,
            lines_added,
            deps_added,
            from_git,
            candidate,
            apply,
            invoke_backend,
            auto,
            task,
        } => {
            let metrics = resolve_metrics(
                &root,
                from_git,
                None,
                DiffMetrics {
                    runtime_dependencies_added: deps_added,
                    files_added,
                    files_changed,
                    lines_added,
                    ..Default::default()
                },
            )?;
            cmd_firebreak(
                &root,
                run_id,
                metrics,
                candidate,
                apply,
                invoke_backend,
                auto,
                task,
                json,
            )
        }
        Commands::Approve { run_id } => cmd_approve(&root, &run_id, json),
        Commands::Reject { run_id } => cmd_reject(&root, &run_id, json),
        Commands::FireLevel { level } => match level {
            None => cmd_fire_level_get(&root, json),
            Some(level) => cmd_fire_level_set(&root, level, json),
        },
        Commands::Rollback { run_id } => cmd_rollback(&root, &run_id, json),
        Commands::Policy(PolicyCmd::Resolve { task, fire_level }) => {
            cmd_policy_resolve(&root, task, fire_level, json)
        }
        Commands::Verify { dry_run } => cmd_verify(&root, dry_run, json),
        Commands::Audit {
            show,
            limit,
            gc,
            purge,
        } => {
            if gc {
                cmd_audit_gc(&root, json)
            } else if purge {
                cmd_audit_purge(&root, json)
            } else {
                cmd_audit_show(&root, limit.unwrap_or(20), show, json)
            }
        }
        Commands::Inspect => cmd_inspect(&root, json),
        Commands::Status => cmd_status(&root, json),
        Commands::FiveAlarm {
            plan,
            run: run_id,
            apply,
            historical_risk,
        } => {
            if plan {
                let outline = FiveAlarmPlan::staged_recovery();
                emit(&outline, json, |p| {
                    println!("Five-Alarm staged recovery:");
                    for (i, s) in p.steps.iter().enumerate() {
                        println!("  {}. {s}", i + 1);
                    }
                    println!();
                    println!("Escalate only after current containment failure:");
                    println!("  tif five-alarm --run <run_id> [--apply]");
                });
                Ok(ExitCode::SUCCESS)
            } else if let Some(id) = run_id {
                cmd_five_alarm_run(&root, &id, apply, historical_risk, json)
            } else {
                anyhow::bail!(
                    "Five-Alarm cannot be selected for initial implementation; use after containment failure (`tif five-alarm --plan` or `tif five-alarm --run <id>`)"
                );
            }
        }
        Commands::Tui => match tui_app::run_tui(&root) {
            Ok(()) => Ok(ExitCode::SUCCESS),
            Err(e) => {
                eprintln!("error: {e:#}");
                eprintln!("Hint: every TUI action has a CLI equivalent (tif status, tif assess, tif audit show).");
                Ok(ExitCode::from(2))
            }
        },
        Commands::Adaptation(sub) => match sub {
            AdaptationCmd::Status => cmd_adaptation_status(&root, json),
            AdaptationCmd::Recommend { category, apply } => {
                cmd_adaptation_recommend(&root, category.as_deref(), apply, json)
            }
            AdaptationCmd::Reset => cmd_adaptation_reset(&root, json),
        },
        Commands::Reviewer(sub) => match sub {
            ReviewerCmd::List => cmd_reviewer_list(&root, json),
            ReviewerCmd::Probe { id } => cmd_reviewer_probe(&root, id, json),
            ReviewerCmd::Test { id, task } => cmd_reviewer_test(&root, id, task, json),
        },
    }
}

fn resolve_root(repo: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let start = repo.unwrap_or(std::env::current_dir()?);
    Ok(find_repo_root(&start))
}

/// Resolve metrics from git, a unified diff file, or explicit CLI flags.
fn resolve_metrics(
    root: &Path,
    from_git: bool,
    from_diff: Option<&PathBuf>,
    explicit: DiffMetrics,
) -> anyhow::Result<DiffMetrics> {
    if let Some(path) = from_diff {
        let text = if path.as_os_str() == "-" {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        } else {
            std::fs::read_to_string(path)
                .with_context(|| format!("read diff file {}", path.display()))?
        };
        return metrics_from_unified_diff_checked(&text)
            .context("parsing unified diff (size-capped at 64 MiB)");
    }
    if from_git {
        return metrics_from_git(root).context("collecting git metrics");
    }
    Ok(explicit)
}

fn emit<T: serde::Serialize, F: FnOnce(&T)>(value: &T, json: bool, human: F) {
    if json {
        match serde_json::to_string_pretty(value) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("json error: {e}"),
        }
    } else {
        human(value);
    }
}

fn emit_ok<T: serde::Serialize, F: FnOnce(&T)>(data: T, json: bool, human: F) -> ExitCode {
    if json {
        let resp = JsonResponse::ok(data);
        println!(
            "{}",
            serde_json::to_string_pretty(&resp).unwrap_or_default()
        );
    } else {
        human(&data);
    }
    ExitCode::SUCCESS
}

fn emit_err(msg: impl Into<String>, json: bool) -> ExitCode {
    let msg = msg.into();
    if json {
        let resp = JsonResponse::<()>::err(&msg);
        println!(
            "{}",
            serde_json::to_string_pretty(&resp).unwrap_or_default()
        );
    } else {
        eprintln!("error: {msg}");
    }
    ExitCode::from(1)
}

fn cmd_init(root: &Path, force: bool, json: bool) -> anyhow::Result<ExitCode> {
    let paths = init_repository(root, force)?;
    if json {
        let data = serde_json::json!({
            "root": paths.root,
            "shared_config": paths.shared_config,
            "local_config": paths.local_config,
            "state_dir": paths.state_dir,
        });
        println!("{}", serde_json::to_string_pretty(&JsonResponse::ok(data))?);
    } else {
        println!("Initialized This Is Fine in {}", paths.root.display());
        println!("  shared: {}", paths.shared_config.display());
        println!("  local:  {} (do not commit)", paths.local_config.display());
        println!("  state:  {}", paths.state_dir.display());
        println!();
        println!("Contain the fire. Do not remodel the building.");
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_set_enabled(root: &Path, enabled: bool, json: bool) -> anyhow::Result<ExitCode> {
    let path = root.join(SHARED_CONFIG_NAME);
    if !path.exists() {
        anyhow::bail!("missing {SHARED_CONFIG_NAME}; run `tif init` first");
    }
    let text = std::fs::read_to_string(&path)?;
    let mut updated = false;
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("enabled") {
                updated = true;
                format!("enabled = {enabled}")
            } else {
                line.to_string()
            }
        })
        .collect();
    if !updated {
        lines.insert(0, format!("enabled = {enabled}"));
    }
    std::fs::write(&path, lines.join("\n") + "\n")?;

    let msg = if enabled {
        "Containment enabled (tif on)"
    } else {
        "Containment suspended (tif off)"
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&JsonResponse::ok(serde_json::json!({
                "enabled": enabled,
                "message": msg,
            })))?
        );
    } else {
        println!("{msg}");
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_run_begin(
    root: &Path,
    task: Option<String>,
    fire_level: Option<u8>,
    agent: Option<String>,
    model: Option<String>,
    force: bool,
    json: bool,
) -> anyhow::Result<ExitCode> {
    let mut cfg = load_config(root)?;
    apply_cli_overrides(&mut cfg, None, fire_level)?;
    let orch = RunOrchestrator::new();
    let fl = fire_level.map(FireLevel::parse_initial).transpose()?;
    let run = orch.begin(
        &cfg,
        &root.to_string_lossy(),
        BeginRunRequest {
            task_text: task,
            fire_level: fl,
            agent_id: agent,
            model_id: model,
            force,
            ..Default::default()
        },
    )?;

    let paths = RepoPaths::for_root(root);
    if let Ok(store) = AuditStore::open(&paths, &cfg.audit) {
        let _ = store.record_run(&run);
    }

    let fl_num = run
        .policy
        .as_ref()
        .map(|p| p.fire_level.as_u8())
        .unwrap_or(cfg.default_fire_level);
    let result = RunBeginResult {
        run_id: run.id.as_str().to_string(),
        state: run.state,
        compact_status: compact_status(fl_num, cfg.enabled),
        policy: run.policy.clone(),
    };

    if run.state == RunState::Failed || run.error.is_some() {
        let msg = run
            .error
            .clone()
            .unwrap_or_else(|| "run begin failed".into());
        return Ok(emit_err(msg, json));
    }

    Ok(emit_ok(result, json, |r| {
        println!("{}", r.compact_status);
        println!("run_id: {}", r.run_id);
        println!("state:  {:?}", r.state);
        if let Some(ref p) = r.policy {
            println!("task:   {}", p.task_category);
            println!("fire:   {}", p.fire_level);
            println!(
                "scenario: {} ({})",
                p.pressure.family, p.pressure.template_id
            );
        }
    }))
}

fn cmd_run_complete(
    root: &Path,
    run_id: &str,
    metrics: DiffMetrics,
    auto_firebreak: bool,
    verification_passed: Option<bool>,
    json: bool,
) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let paths = RepoPaths::for_root(root);
    let store = AuditStore::open(&paths, &cfg.audit)?;
    let mut run = store
        .get_run(run_id)?
        .with_context(|| format!("run not found: {run_id}"))?;

    let inspection = RepositoryInspector::new()
        .inspect(root)
        .context("repository inspection failed; cannot plan verification")?;
    // `--verification-passed` is a trusted-adapter signal (agent already ran checks).
    // It is not re-executed here; CI should omit this flag and use `tif verify` instead.
    let report = if let Some(passed) = verification_passed {
        let mut r = plan_and_run(root, &cfg.verification, Some(&inspection), true)?;
        r.all_required_passed = passed;
        r.incomplete_plan = !passed && r.checks.is_empty();
        if passed {
            // Adapter asserts a complete pass; clear unresolved flags for floor.
            r.has_unresolved = false;
            r.incomplete_plan = false;
            // Ensure at least one required synthetic check if plan was empty.
            if r.checks.is_empty() || !r.checks.iter().any(|c| c.check.required) {
                r.checks.push(tif_core::verify::CheckResult {
                    check: tif_core::verify::VerificationCheck {
                        id: "adapter-asserted".into(),
                        category: tif_core::verify::VerificationCategory::Custom,
                        command: "adapter-asserted".into(),
                        source: tif_core::verify::CheckSource::ExplicitConfig,
                        required: true,
                        evidence: Some(
                            "trusted adapter --verification-passed=true (not re-executed)".into(),
                        ),
                    },
                    status: tif_core::verify::CheckStatus::Passed,
                    exit_code: Some(0),
                    duration_ms: 0,
                    stdout_tail: String::new(),
                    stderr_tail: "verification_source=adapter".into(),
                });
            }
        } else {
            for c in &mut r.checks {
                if c.check.required {
                    c.status = tif_core::verify::CheckStatus::Failed;
                }
            }
            r.has_unresolved = false;
        }
        r
    } else {
        plan_and_run(root, &cfg.verification, Some(&inspection), false)?
    };

    let floor = CorrectnessFloor {
        verification_passed: report.satisfies_correctness_verification(),
        ..CorrectnessFloor::all_pass()
    };

    let orch = RunOrchestrator::new();
    orch.complete(&cfg, &mut run, metrics, report, floor, auto_firebreak)?;
    // Keep OutOfControl / AwaitingApproval open for manual Firebreak or approve/reject.
    // Contained / Restored / Rejected / Applied runs may close.
    if !matches!(
        run.state,
        RunState::OutOfControl | RunState::AwaitingApproval
    ) {
        let _ = orch.close(&mut run);
    }
    store.record_run(&run)?;
    let _ = store.record_adaptation_from_run(&run);

    Ok(emit_ok(
        serde_json::json!({
            "run_id": run.id.as_str(),
            "state": run.state,
            "assessment": run.assessment,
            "firebreak": run.firebreak,
            "approval_expires_at": run.approval_expires_at,
        }),
        json,
        |_| {
            println!("run_id: {}", run.id);
            println!("state:  {}", run.state.as_str());
            if let Some(ref a) = run.assessment {
                println!("{}", a.summary);
            }
            if let Some(ref fb) = run.firebreak {
                println!("firebreak: {}", fb.message);
            }
            if run.state == RunState::AwaitingApproval {
                println!("approve: tif approve {}", run.id);
                println!("reject:  tif reject {}", run.id);
            }
        },
    ))
}

fn cmd_run_show(root: &Path, run_id: &str, json: bool) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let store = AuditStore::open(&RepoPaths::for_root(root), &cfg.audit)?;
    match store.get_run(run_id)? {
        Some(run) => Ok(emit_ok(run, json, |r| {
            println!("run_id: {}", r.id);
            println!("state:  {}", r.state.as_str());
            println!("repo:   {}", r.repo_root);
            if let Some(ref a) = r.assessment {
                println!("assess: {}", a.summary);
            }
            for e in &r.events {
                println!("  · {e}");
            }
        })),
        None => Ok(emit_err(format!("run not found: {run_id}"), json)),
    }
}

fn cmd_run_status(root: &Path, json: bool) -> anyhow::Result<ExitCode> {
    cmd_status(root, json)
}

fn cmd_assess(
    root: &Path,
    metrics: DiffMetrics,
    fire_level: Option<u8>,
    json: bool,
) -> anyhow::Result<ExitCode> {
    let mut cfg = load_config(root)?;
    apply_cli_overrides(&mut cfg, None, fire_level)?;
    let fl = FireLevel::parse_initial(fire_level.unwrap_or(cfg.default_fire_level))?;
    let floor = CorrectnessFloor::all_pass();
    let scorer = SimplicityScorer::new(
        cfg.simplicity.weights.clone(),
        cfg.simplicity.limits.clone(),
    );
    let score = scorer.score(&metrics, &floor);
    let assessment =
        DamageAssessor::build("adhoc", "working-tree", fl, metrics, score, floor, None);
    let result = AssessResult { assessment };

    let code = if result.assessment.score.within_containment || cfg.ci.on_violation == "warn" {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    };

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&JsonResponse::ok(&result))?
        );
    } else {
        let a = &result.assessment;
        println!("{}", a.summary);
        println!("status: {:?}", a.status);
        println!("score:  {:.1}", a.score.score);
        if !a.score.hard_limit_violations.is_empty() {
            println!("hard limit violations:");
            for v in &a.score.hard_limit_violations {
                println!("  - {v}");
            }
        }
        println!(
            "metrics: +{} lines, {} files added, {} deps added",
            a.metrics.lines_added, a.metrics.files_added, a.metrics.runtime_dependencies_added
        );
    }
    Ok(code)
}

#[allow(clippy::too_many_arguments)]
fn cmd_firebreak(
    root: &Path,
    run_id: Option<String>,
    metrics: DiffMetrics,
    candidate: Option<PathBuf>,
    authorize_apply: bool,
    invoke_backend: bool,
    auto: bool,
    task: Option<String>,
    json: bool,
) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let paths = RepoPaths::for_root(root);
    ensure_state_dirs(&paths)?;
    let store = AuditStore::open(&paths, &cfg.audit)?;
    let orch = RunOrchestrator::new();

    if invoke_backend {
        return cmd_firebreak_invoke_backend(root, &cfg, &paths, metrics, task, json);
    }

    let mut run = if let Some(id) = run_id {
        store
            .get_run(&id)?
            .with_context(|| format!("run not found: {id}"))?
    } else {
        // Ad-hoc firebreak run with explicit verification plan (not vacuous pass).
        let mut r = orch.begin(
            &cfg,
            &root.to_string_lossy(),
            BeginRunRequest {
                task_text: Some(task.clone().unwrap_or_else(|| "manual firebreak".into())),
                force: true,
                ..Default::default()
            },
        )?;
        let report = tif_core::verify::VerificationReport {
            checks: vec![tif_core::verify::CheckResult {
                check: tif_core::verify::VerificationCheck {
                    id: "manual".into(),
                    category: tif_core::verify::VerificationCategory::Custom,
                    command: "manual".into(),
                    source: tif_core::verify::CheckSource::ExplicitConfig,
                    required: true,
                    evidence: Some("manual firebreak invocation".into()),
                },
                status: tif_core::verify::CheckStatus::Passed,
                exit_code: Some(0),
                duration_ms: 0,
                stdout_tail: String::new(),
                stderr_tail: String::new(),
            }],
            all_required_passed: true,
            has_unresolved: false,
            has_unresolved_required: false,
            incomplete_plan: false,
        };
        orch.complete(
            &cfg,
            &mut r,
            metrics,
            report,
            CorrectnessFloor::all_pass(),
            false,
        )?;
        r
    };

    // Legal states for Firebreak: Contained, OutOfControl, or already FirebreakRunning.
    // Closed runs with an OutOfControl assessment are reopened for manual Firebreak.
    if run.state == RunState::Closed {
        let was_out = run
            .assessment
            .as_ref()
            .is_some_and(|a| a.status == AssessmentStatus::OutOfControl);
        if was_out {
            run.state = RunState::OutOfControl;
            run.events
                .push("closed -> out_of_control (reopened for firebreak)".into());
        }
    }
    if !matches!(
        run.state,
        RunState::Contained | RunState::OutOfControl | RunState::FirebreakRunning
    ) {
        return Ok(emit_err(
            format!(
                "cannot start firebreak from state {} (expected contained or out_of_control)",
                run.state.as_str()
            ),
            json,
        ));
    }

    let floor = run
        .assessment
        .as_ref()
        .map(|a| a.correctness.clone())
        .unwrap_or_else(CorrectnessFloor::all_pass);

    if let Some(candidate_path) = candidate {
        // Isolated path: stage candidate, re-verify in isolation workspace, optionally apply.
        let (isolator, mut session) =
            FirebreakEngine::open_workspace(root, &paths.state_dir, run.id.as_str())?;
        FirebreakEngine::stage_candidate_tree(&session, &candidate_path)?;
        run.events.push(format!(
            "staged candidate tree from {}",
            candidate_path.display()
        ));

        // Persist isolation session BEFORE apply so crash/dual-failure still has
        // a recoverable session on disk (attach → record → apply → record).
        run.isolation_session = Some(session.clone());
        store.record_run(&run)?;

        // Re-verify against the isolated workspace path only — never fall back to
        // the original repo (that would re-verify the wrong tree).
        let inspection = RepositoryInspector::new()
            .inspect(&session.path)
            .with_context(|| {
                format!(
                    "failed to inspect isolation workspace {}",
                    session.path.display()
                )
            })?;
        // Propagate plan/run errors (include them in events). Incomplete plans are
        // already returned as Ok(report) with incomplete_plan=true — do not swallow
        // real errors into a silent synthetic Failed report.
        let report = match plan_and_run(&session.path, &cfg.verification, Some(&inspection), false)
        {
            Ok(r) => {
                if r.incomplete_plan {
                    run.events.push(
                        "isolated verification plan incomplete (no required checks ran)".into(),
                    );
                }
                r
            }
            Err(e) => {
                let err_msg = format!("isolated verification plan_and_run failed: {e}");
                run.events.push(err_msg.clone());
                // Persist session + events before failing so operators can inspect.
                run.isolation_session = Some(session.clone());
                let _ = store.record_run(&run);
                return Err(anyhow::anyhow!(err_msg));
            }
        };

        let candidate_floor = candidate_floor_from_verification(&report);
        // Ranking metrics must be the same *kind* on both sides.
        // Prefer git metrics for the candidate when available (worktree).
        // Otherwise use absolute tree weight for BOTH original and candidate —
        // never source↔candidate deltas (identical trees score ~0 and fail-open).
        let (candidate_metrics, ranking_original_metrics, ranking_original_score) =
            match metrics_from_git(&session.path) {
                Ok(m) => (m, None, None),
                Err(_) => {
                    let orig_abs = metrics_from_tree_absolute(root).with_context(|| {
                        format!(
                            "cannot collect absolute tree metrics for original {}",
                            root.display()
                        )
                    })?;
                    let cand_abs =
                        metrics_from_tree_absolute(&session.path).with_context(|| {
                            format!(
                                "cannot collect absolute tree metrics for candidate {}",
                                session.path.display()
                            )
                        })?;
                    let policy = run
                        .policy
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("run missing policy"))?;
                    let scorer =
                        SimplicityScorer::new(policy.weights.clone(), policy.limits.clone());
                    let orig_score = scorer.score(&orig_abs, &floor);
                    run.events.push(
                        "ranking uses absolute tree metrics for original and candidate (no git)"
                            .into(),
                    );
                    (cand_abs, Some(orig_abs), Some(orig_score))
                }
            };

        let apply = authorize_apply || (auto && cfg.approval.auto_apply_firebreak);
        let fb_result = orch.run_firebreak_isolated(
            &cfg,
            &mut run,
            tif_core::orchestrator::IsolatedFirebreakParams {
                original_floor: &floor,
                isolator: isolator.as_ref(),
                session: &mut session,
                candidate_metrics,
                candidate_floor,
                authorize_apply: apply,
                ranking_original_metrics,
                ranking_original_score,
                user_approved: false,
            },
        );
        // Re-persist after apply (applied / restore_pending / baseline_path).
        store.record_run(&run)?;
        let _ = store.record_adaptation_from_run(&run);
        fb_result?;
    } else if auto || !cfg.reviewers.is_empty() {
        // Phase 2 automatic closed loop when --auto or reviewers are configured.
        // Persist before apply is handled inside the closed loop (session attach).
        store.record_run(&run)?;
        let authorize = authorize_apply || auto || cfg.approval.auto_apply_firebreak;
        orch.run_firebreak_auto(
            &cfg,
            &mut run,
            &floor,
            tif_core::FirebreakAutoOptions {
                authorize_apply: authorize,
                user_approved: false,
            },
        )?;
        store.record_run(&run)?;
        let _ = store.record_adaptation_from_run(&run);
    } else {
        // No reviewers: fail-closed.
        orch.run_firebreak(&cfg, &mut run, &floor)?;
        store.record_run(&run)?;
        let _ = store.record_adaptation_from_run(&run);
    }

    Ok(emit_ok(
        serde_json::json!({
            "run_id": run.id.as_str(),
            "state": run.state,
            "firebreak": run.firebreak,
            "isolation": run.isolation_session,
            "approval_expires_at": run.approval_expires_at,
        }),
        json,
        |_| {
            if let Some(ref fb) = run.firebreak {
                println!("{}", fb.message);
                println!("applied: {}", fb.applied);
                println!("original_preserved: {}", fb.original_preserved);
                if let Some(ref s) = run.isolation_session {
                    println!("isolation: {} ({:?})", s.id, s.kind);
                }
                if run.state == RunState::AwaitingApproval {
                    println!("approve: tif approve {}", run.id);
                    println!("reject:  tif reject {}", run.id);
                }
            } else {
                println!("no firebreak outcome");
            }
        },
    ))
}

fn cmd_approve(root: &Path, run_id: &str, json: bool) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let paths = RepoPaths::for_root(root);
    let store = AuditStore::open(&paths, &cfg.audit)?;
    let mut run = store
        .get_run(run_id)?
        .with_context(|| format!("run not found: {run_id}"))?;
    let orch = RunOrchestrator::new();
    match orch.approve_firebreak(&cfg, &mut run) {
        Ok(()) => {
            store.record_run(&run)?;
            let _ = store.record_adaptation_from_run(&run);
            Ok(emit_ok(
                serde_json::json!({
                    "run_id": run.id.as_str(),
                    "state": run.state,
                    "firebreak": run.firebreak,
                    "isolation": run.isolation_session,
                }),
                json,
                |_| {
                    println!("approved run {}", run.id);
                    println!("state: {}", run.state.as_str());
                    if let Some(ref fb) = run.firebreak {
                        println!("{}", fb.message);
                        println!("applied: {}", fb.applied);
                    }
                },
            ))
        }
        Err(e) => {
            let _ = store.record_run(&run);
            Ok(emit_err(e.to_string(), json))
        }
    }
}

fn cmd_five_alarm_run(
    root: &Path,
    run_id: &str,
    apply: bool,
    historical_risk: bool,
    json: bool,
) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let paths = RepoPaths::for_root(root);
    ensure_state_dirs(&paths)?;
    let store = AuditStore::open(&paths, &cfg.audit)?;
    let mut run = store
        .get_run(run_id)?
        .with_context(|| format!("run not found: {run_id}"))?;

    // Legal entry: OOC, Restored (after failed firebreak), Contained (manual), or closed OOC.
    if run.state == RunState::Closed {
        let was_out = run
            .assessment
            .as_ref()
            .is_some_and(|a| a.status == AssessmentStatus::OutOfControl);
        if was_out {
            run.state = RunState::OutOfControl;
            run.events
                .push("closed -> out_of_control (reopened for five-alarm)".into());
        }
    }
    if !matches!(
        run.state,
        RunState::OutOfControl
            | RunState::Restored
            | RunState::Contained
            | RunState::FirebreakRunning
            | RunState::AwaitingApproval
    ) {
        return Ok(emit_err(
            format!(
                "cannot start Five-Alarm from state {} (expected out_of_control, restored, or contained after failure)",
                run.state.as_str()
            ),
            json,
        ));
    }

    let floor = run
        .assessment
        .as_ref()
        .map(|a| a.correctness.clone())
        .unwrap_or_else(CorrectnessFloor::all_pass);
    if !floor.passes() {
        return Ok(emit_err(
            "correctness floor does not pass; Five-Alarm requires a verified (correct) but out-of-containment implementation",
            json,
        ));
    }

    let orch = RunOrchestrator::new();
    store.record_run(&run)?;
    let authorize = apply || cfg.approval.auto_apply_firebreak;
    match orch.run_five_alarm(
        &cfg,
        &mut run,
        &floor,
        FiveAlarmRunOptions {
            authorize_apply: authorize,
            user_approved: apply,
            historical_risk_noted: historical_risk,
        },
    ) {
        Ok(result) => {
            store.record_run(&run)?;
            let _ = store.record_adaptation_from_run(&run);
            Ok(emit_ok(
                serde_json::json!({
                    "run_id": run.id.as_str(),
                    "state": run.state,
                    "five_alarm": result.plan,
                    "firebreak": result.firebreak,
                    "isolation": run.isolation_session,
                }),
                json,
                |_| {
                    println!("Five-Alarm run {}", run.id);
                    println!("stage: {}", result.plan.stage.as_str());
                    println!("{}", result.plan.message);
                    println!("applied: {}", result.plan.applied);
                    println!("original_preserved: {}", result.plan.original_preserved);
                    if !result.plan.timeline.is_empty() {
                        println!("timeline:");
                        for e in &result.plan.timeline {
                            println!("  [{}] {}", e.stage.as_str(), e.message);
                        }
                    }
                    if let Some(ref w) = result.plan.winner_id {
                        println!("winner: {w}");
                    }
                    if run.state == RunState::AwaitingApproval {
                        println!("approve: tif approve {}", run.id);
                        println!("reject:  tif reject {}", run.id);
                    }
                },
            ))
        }
        Err(e) => {
            let _ = store.record_run(&run);
            Ok(emit_err(e.to_string(), json))
        }
    }
}

fn cmd_reject(root: &Path, run_id: &str, json: bool) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let paths = RepoPaths::for_root(root);
    let store = AuditStore::open(&paths, &cfg.audit)?;
    let mut run = store
        .get_run(run_id)?
        .with_context(|| format!("run not found: {run_id}"))?;
    let orch = RunOrchestrator::new();
    match orch.reject_firebreak(&mut run, None) {
        Ok(()) => {
            store.record_run(&run)?;
            let _ = store.record_adaptation_from_run(&run);
            Ok(emit_ok(
                serde_json::json!({
                    "run_id": run.id.as_str(),
                    "state": run.state,
                    "firebreak": run.firebreak,
                }),
                json,
                |_| {
                    println!("rejected firebreak for run {}", run.id);
                    println!("state: {}", run.state.as_str());
                    println!("original preserved");
                },
            ))
        }
        Err(e) => Ok(emit_err(e.to_string(), json)),
    }
}

fn cmd_fire_level_get(root: &Path, json: bool) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let fl = FireLevel::parse_initial(cfg.default_fire_level)?;
    let result = FireLevelResult {
        fire_level: fl,
        name: fl.name().into(),
        initial_selectable: fl.is_initial_selectable(),
    };
    Ok(emit_ok(result, json, |r| {
        println!("Fire Level {} ({})", r.fire_level.as_u8(), r.name);
    }))
}

fn cmd_fire_level_set(root: &Path, level: u8, json: bool) -> anyhow::Result<ExitCode> {
    let fl = match FireLevel::parse_initial(level) {
        Ok(f) => f,
        Err(e) => return Ok(emit_err(e.to_string(), json)),
    };
    let path = root.join(SHARED_CONFIG_NAME);
    if !path.exists() {
        anyhow::bail!("missing {SHARED_CONFIG_NAME}; run `tif init` first");
    }
    let text = std::fs::read_to_string(&path)?;
    let mut updated = false;
    let lines: Vec<String> = text
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("default_fire_level") {
                updated = true;
                format!("default_fire_level = {level}")
            } else {
                line.to_string()
            }
        })
        .collect();
    let mut lines = lines;
    if !updated {
        lines.push(format!("default_fire_level = {level}"));
    }
    std::fs::write(&path, lines.join("\n") + "\n")?;

    let result = FireLevelResult {
        fire_level: fl,
        name: fl.name().into(),
        initial_selectable: true,
    };
    Ok(emit_ok(result, json, |r| {
        println!("Fire Level set to {} ({})", r.fire_level.as_u8(), r.name);
    }))
}

fn cmd_rollback(root: &Path, run_id: &str, json: bool) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let paths = RepoPaths::for_root(root);
    let store = AuditStore::open(&paths, &cfg.audit)?;
    let mut run = store
        .get_run(run_id)?
        .with_context(|| format!("run not found: {run_id}"))?;

    let orch = RunOrchestrator::new();
    let (restored_fs, reason, message) = if let Some(ref session) = run.isolation_session {
        if session.applied || session.restore_pending {
            let isolator = isolator_for_session(session, &paths.snapshots_dir());
            match orch.rollback_isolation(&mut run, isolator.as_ref()) {
                Ok(()) => (
                    true,
                    Some("restored".to_string()),
                    "Filesystem rollback restored original from baseline; audit updated"
                        .to_string(),
                ),
                Err(e) => {
                    run.events.push(format!("rollback failed: {e}"));
                    store.record_run(&run)?;
                    return Ok(emit_err(format!("filesystem rollback failed: {e}"), json));
                }
            }
        } else {
            run.events.push(
                "rollback requested; isolation session was never applied (original intact)".into(),
            );
            run.state = RunState::Restored;
            run.rollback_candidate_id = Some("original".into());
            (
                false,
                Some("never_applied".to_string()),
                "Original workspace was never modified; noop (audit marked restored)".to_string(),
            )
        }
    } else {
        run.events.push(
            "rollback requested; no isolation session (original workspace never modified by apply)"
                .into(),
        );
        run.state = RunState::Restored;
        run.rollback_candidate_id = Some("original".into());
        (
            false,
            Some("noop".to_string()),
            "No Firebreak apply on record; original preserved (audit marked restored)".to_string(),
        )
    };

    store.record_run(&run)?;
    let _ = store.record_adaptation_from_run(&run);

    let result = RollbackResult {
        run_id: run_id.into(),
        restored: restored_fs,
        reason,
        message,
    };
    Ok(emit_ok(result, json, |r| {
        println!("{}", r.message);
        if let Some(ref reason) = r.reason {
            println!("reason: {reason}");
        }
    }))
}

fn cmd_policy_resolve(
    root: &Path,
    task: Option<String>,
    fire_level: Option<u8>,
    json: bool,
) -> anyhow::Result<ExitCode> {
    let mut cfg = load_config(root)?;
    apply_cli_overrides(&mut cfg, None, fire_level)?;
    let fl = fire_level.map(FireLevel::parse_initial).transpose()?;
    let policy = PolicyCompiler::new().compile(
        &cfg,
        &PolicyCompileRequest {
            task_text: task,
            fire_level: fl,
            ..Default::default()
        },
    )?;
    let result = PolicyResolveResult {
        compact_status: policy.pressure.compact_status.clone(),
        policy,
    };
    Ok(emit_ok(result, json, |r| {
        println!("{}", r.compact_status);
        println!("policy_id: {}", r.policy.policy_id);
        println!("task:      {}", r.policy.task_category);
        println!("fire:      {}", r.policy.fire_level);
        println!("scenario:  {}", r.policy.pressure.family);
        println!("limits:    {:?}", r.policy.limits);
        println!();
        println!("--- pressure body ---");
        println!("{}", r.policy.pressure.body);
    }))
}

fn cmd_verify(root: &Path, dry_run: bool, json: bool) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root).context(
        "failed to load .this-is-fine.toml; fix config errors before running verification",
    )?;
    let inspection = RepositoryInspector::new()
        .inspect(root)
        .context("repository inspection failed; cannot plan verification")?;
    let planner = VerificationPlanner::new();
    let checks = planner.plan(&cfg.verification, Some(&inspection));
    let report = if dry_run {
        let mut runner = tif_core::VerificationRunner::new();
        runner.dry_run = true;
        runner.run(root, &checks)?
    } else {
        plan_and_run(root, &cfg.verification, Some(&inspection), false)?
    };
    let result = VerifyResult {
        report: report.clone(),
    };
    let code = if report.all_required_passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&JsonResponse::ok(result))?
        );
    } else {
        println!(
            "verification: {} required passed",
            if report.all_required_passed {
                "all"
            } else {
                "not all"
            }
        );
        for c in &report.checks {
            println!("  [{:?}] {} — {}", c.status, c.check.id, c.check.command);
        }
    }
    Ok(code)
}

fn cmd_audit_show(root: &Path, limit: usize, _show: bool, json: bool) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let store = AuditStore::open(&RepoPaths::for_root(root), &cfg.audit)?;
    let runs = store.list_runs(limit)?;
    Ok(emit_ok(runs, json, |runs| {
        if runs.is_empty() {
            println!("(no audit records)");
            return;
        }
        for r in runs {
            println!(
                "{}  {:>12}  fire={:?}  score={:?}  {}",
                r.id, r.state, r.fire_level, r.simplicity_score, r.summary
            );
        }
    }))
}

fn cmd_audit_gc(root: &Path, json: bool) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let paths = RepoPaths::for_root(root);
    ensure_state_dirs(&paths)?;
    let store = AuditStore::open(&paths, &cfg.audit)?;
    let snapshots = paths.snapshots_dir();
    let worktrees = paths.state_dir.join("worktrees");
    let isolation = tif_core::IsolationGcOpts {
        snapshots_dir: &snapshots,
        worktrees_dir: worktrees.exists().then_some(worktrees.as_path()),
        max_age_days: cfg.rollback.max_days,
    };
    let retention = tif_core::RollbackRetention::from_config(
        cfg.rollback.max_days,
        cfg.rollback.successful_commits,
    );
    let report = store.gc_with_options(Some(isolation), Some(retention), Some(root))?;
    Ok(emit_ok(report, json, |r| {
        println!(
            "gc: deleted_runs={} deleted_events={} reclaimed_bytes={} isolation_removed={}",
            r.deleted_runs, r.deleted_events, r.reclaimed_bytes, r.isolation_removed
        );
    }))
}

fn cmd_audit_purge(root: &Path, json: bool) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let store = AuditStore::open(&RepoPaths::for_root(root), &cfg.audit)?;
    store.purge_all()?;
    Ok(emit_ok(serde_json::json!({"purged": true}), json, |_| {
        println!("All local audit data purged.")
    }))
}

fn cmd_inspect(root: &Path, json: bool) -> anyhow::Result<ExitCode> {
    let insp = RepositoryInspector::new().inspect(root)?;
    Ok(emit_ok(insp, json, |i| {
        println!("root:     {}", i.root.display());
        println!("git:      {}", i.is_git);
        println!("langs:    {:?}", i.languages);
        println!("pkg mgrs: {:?}", i.package_managers);
        println!("verify:");
        for c in &i.verification_commands {
            println!(
                "  {} ({}) confident={} — {}",
                c.command,
                format!("{:?}", c.category).to_lowercase(),
                c.confident,
                c.evidence
            );
        }
        if !i.unresolved_notes.is_empty() {
            println!("unresolved:");
            for n in &i.unresolved_notes {
                println!("  - {n}");
            }
        }
    }))
}

fn cmd_status(root: &Path, json: bool) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root).unwrap_or_default();
    let status = compact_status(cfg.default_fire_level, cfg.enabled);
    let data = serde_json::json!({
        "root": root,
        "enabled": cfg.enabled,
        "default_fire_level": cfg.default_fire_level,
        "compact_status": status,
        "audit_tier": cfg.audit.tier,
        "reviewers": cfg.reviewers.len(),
    });
    Ok(emit_ok(data, json, |d| {
        println!("{}", d["compact_status"].as_str().unwrap_or(""));
        println!("repo:     {}", root.display());
        println!("enabled:  {}", cfg.enabled);
        println!("fire:     {}", cfg.default_fire_level);
        println!("audit:    {}", cfg.audit.tier);
        println!("reviewers: {}", cfg.reviewers.len());
    }))
}

fn cmd_firebreak_invoke_backend(
    root: &Path,
    cfg: &tif_core::Config,
    paths: &RepoPaths,
    metrics: DiffMetrics,
    task: Option<String>,
    json: bool,
) -> anyhow::Result<ExitCode> {
    if cfg.reviewers.is_empty() {
        return Ok(emit_err(
            "no reviewers authorized in local config; add [[reviewers]] to .this-is-fine.local.toml",
            json,
        ));
    }
    let policy = PolicyCompiler::new().compile(
        cfg,
        &PolicyCompileRequest {
            task_text: task.clone(),
            ..Default::default()
        },
    )?;
    let scorer = SimplicityScorer::new(policy.weights.clone(), policy.limits.clone());
    let floor = CorrectnessFloor::all_pass();
    let original_score = scorer.score(&metrics, &floor);
    let engine = FirebreakEngine::new(ReviewerSelector::new(cfg.reviewers.clone()));
    let mut gen_req = BackendGenerateRequest::standard(
        format!("cli-{}", chrono_like_id()),
        policy,
        root.to_path_buf(),
        paths.state_dir.clone(),
        metrics,
        original_score,
        floor,
    );
    gen_req.task_text = task;
    gen_req.verification_plan_summary =
        Some("use repository verification plan after generation".into());
    let result = engine.generate_with_backend(gen_req)?;

    let data = serde_json::json!({
        "outcome": result.outcome,
        "isolation_session_id": result.session.id,
        "isolation_path": result.session.path,
        "candidate_path": result.patch.as_ref().map(|p| p.candidate_root.clone()),
        "notes": result.patch.as_ref().map(|p| p.notes.clone()),
        "applied": false,
        "original_preserved": true,
    });
    let code = if result.outcome.success {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&JsonResponse::ok(data))?);
    } else {
        println!("{}", result.outcome.message);
        println!("isolation: {}", result.session.path.display());
        if let Some(p) = result.patch.as_ref() {
            println!("candidate: {}", p.candidate_root.display());
        }
        println!("(not applied — re-verify then tif firebreak --candidate … --apply)");
    }
    Ok(code)
}

fn chrono_like_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{n:x}")
}

fn cmd_reviewer_list(root: &Path, json: bool) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let reg = BackendRegistry::new();
    let compiled = reg.list_kinds();
    let rows: Vec<serde_json::Value> = cfg
        .reviewers
        .iter()
        .map(|r| {
            let backend_ok = backend_for_provider(&reg, &r.provider).is_ok();
            serde_json::json!({
                "id": r.id,
                "provider": r.provider,
                "model": r.model,
                "endpoint": r.endpoint,
                "allow_source_egress": r.allow_source_egress,
                "priority": r.priority,
                "max_firebreak_attempts": r.max_firebreak_attempts,
                "timeout_secs": r.timeout_secs,
                "backend_compiled": backend_ok,
                "credential_ref_set": r.credential_ref.is_some(),
            })
        })
        .collect();
    let data = serde_json::json!({
        "compiled_backends": compiled,
        "reviewers": rows,
    });
    Ok(emit_ok(data, json, |d| {
        println!("compiled backends: {:?}", d["compiled_backends"]);
        println!("authorized reviewers: {}", cfg.reviewers.len());
        for r in &cfg.reviewers {
            let ok = backend_for_provider(&reg, &r.provider).is_ok();
            println!(
                "  - {}  provider={} model={} egress={} backend={}",
                r.id,
                r.provider,
                r.model,
                r.allow_source_egress,
                if ok { "yes" } else { "NO" }
            );
        }
        if cfg.reviewers.is_empty() {
            println!("(none — add [[reviewers]] to .this-is-fine.local.toml)");
        }
    }))
}

fn cmd_reviewer_probe(root: &Path, id: Option<String>, json: bool) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let reg = BackendRegistry::new();
    let targets: Vec<_> = match id {
        Some(id) => cfg
            .reviewers
            .iter()
            .filter(|r| r.id == id)
            .cloned()
            .collect(),
        None => cfg.reviewers.clone(),
    };
    if targets.is_empty() {
        return Ok(emit_err("no matching reviewers to probe", json));
    }
    let mut results = Vec::new();
    let mut all_ok = true;
    for r in &targets {
        let backend = match backend_for_provider(&reg, &r.provider) {
            Ok(b) => b,
            Err(e) => {
                all_ok = false;
                results.push(serde_json::json!({
                    "reviewer_id": r.id,
                    "ok": false,
                    "message": e.to_string(),
                }));
                continue;
            }
        };
        let cred = resolve_credential_opt(r.credential_ref.as_deref())
            .ok()
            .flatten();
        match backend.probe(r, cred.as_deref()) {
            Ok(p) => {
                if !p.ok {
                    all_ok = false;
                }
                results.push(serde_json::to_value(p)?);
            }
            Err(e) => {
                all_ok = false;
                results.push(serde_json::json!({
                    "reviewer_id": r.id,
                    "ok": false,
                    "message": e.to_string(),
                }));
            }
        }
    }
    let data = serde_json::json!({ "results": results });
    if json {
        println!("{}", serde_json::to_string_pretty(&JsonResponse::ok(data))?);
    } else {
        for r in &results {
            println!(
                "{}: {} — {}",
                r.get("reviewer_id").and_then(|v| v.as_str()).unwrap_or("?"),
                if r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                    "ok"
                } else {
                    "FAIL"
                },
                r.get("message").and_then(|v| v.as_str()).unwrap_or("")
            );
        }
    }
    Ok(if all_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn cmd_reviewer_test(
    root: &Path,
    id: Option<String>,
    task: Option<String>,
    json: bool,
) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let paths = RepoPaths::for_root(root);
    ensure_state_dirs(&paths)?;
    // Prefer mock reviewer for offline test when id not specified.
    let chosen = if let Some(id) = id {
        cfg.reviewers
            .iter()
            .find(|r| r.id == id)
            .cloned()
            .with_context(|| format!("reviewer not found: {id}"))?
    } else {
        cfg.reviewers
            .iter()
            .find(|r| r.provider == "mock")
            .cloned()
            .or_else(|| cfg.reviewers.first().cloned())
            .context("no reviewers configured; add a mock [[reviewers]] entry for offline test")?
    };

    // Build a one-entry pool so selection is deterministic.
    let mut one = cfg.clone();
    one.reviewers = vec![chosen.clone()];
    let engine = FirebreakEngine::new(ReviewerSelector::new(one.reviewers.clone()));
    let policy = PolicyCompiler::new().compile(
        &one,
        &PolicyCompileRequest {
            task_text: task.clone(),
            ..Default::default()
        },
    )?;
    let metrics = DiffMetrics {
        lines_added: 10,
        files_added: 1,
        ..Default::default()
    };
    let scorer = SimplicityScorer::new(policy.weights.clone(), policy.limits.clone());
    let floor = CorrectnessFloor::all_pass();
    let score = scorer.score(&metrics, &floor);
    let mut gen_req = BackendGenerateRequest::standard(
        "reviewer-test",
        policy,
        root.to_path_buf(),
        paths.state_dir.clone(),
        metrics,
        score,
        floor,
    );
    gen_req.task_text = task.or_else(|| Some("offline reviewer test".into()));
    gen_req.preferred_reviewer_id = Some(chosen.id.clone());
    gen_req.max_output_bytes = 2_000_000;
    let result = engine.generate_with_backend(gen_req)?;

    let data = serde_json::json!({
        "reviewer_id": chosen.id,
        "provider": chosen.provider,
        "success": result.outcome.success,
        "message": result.outcome.message,
        "candidate_path": result.patch.as_ref().map(|p| p.candidate_root.clone()),
        "isolation_path": result.session.path,
        "original_preserved": true,
        "applied": false,
    });
    let code = if result.outcome.success {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&JsonResponse::ok(data))?);
    } else {
        println!("reviewer test: {}", chosen.id);
        println!("{}", result.outcome.message);
        if let Some(p) = result.patch.as_ref() {
            println!("candidate: {}", p.candidate_root.display());
        }
    }
    Ok(code)
}

fn cmd_adaptation_status(root: &Path, json: bool) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let paths = RepoPaths::for_root(root);
    let store = AuditStore::open(&paths, &cfg.audit)?;
    let stats = store.load_adaptation_stats()?;
    let eng = AdaptationEngine::load(stats);
    Ok(emit_ok(
        serde_json::json!({
            "stats": eng.stats(),
        }),
        json,
        |_| {
            println!("Local adaptation (no telemetry)");
            println!("runs: {}", eng.stats().total_runs);
            println!(
                "contained={} ooc={} firebreak_ok={} firebreak_fail={} rollbacks={}",
                eng.stats().contained,
                eng.stats().out_of_control,
                eng.stats().firebreak_success,
                eng.stats().firebreak_fail,
                eng.stats().rollbacks
            );
            println!(
                "pressure variants: {}",
                eng.stats().pressure_variant_scores.len()
            );
            for v in &eng.stats().pressure_variant_scores {
                println!(
                    "  {} trials={} score={:.1} verify={:.2} status={:?}",
                    v.template_id,
                    v.trials,
                    v.avg_simplicity_score,
                    v.verification_pass_rate,
                    v.status
                );
            }
            if !eng.stats().promoted_templates.is_empty() {
                println!("promoted: {:?}", eng.stats().promoted_templates);
            }
            println!("applied knobs: {:?}", eng.stats().applied_knobs);
        },
    ))
}

fn cmd_adaptation_recommend(
    root: &Path,
    category: Option<&str>,
    apply: bool,
    json: bool,
) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let paths = RepoPaths::for_root(root);
    let store = AuditStore::open(&paths, &cfg.audit)?;
    let stats = store.load_adaptation_stats()?;
    let mut eng = AdaptationEngine::load(stats);
    let cat = category
        .and_then(|s| s.parse::<tif_core::TaskCategory>().ok())
        .unwrap_or(tif_core::TaskCategory::Unknown);
    let rec = eng.recommend(cat);
    let mut applied = Vec::new();
    if apply {
        if let Some(fl) = rec.suggested_fire_level {
            // Allowlisted: fire level bias ≤4 only.
            eng.self_apply(
                tif_core::SelfApplyKnob::FireLevelBias,
                tif_core::SelfApplyValue::FireLevel(fl.as_u8()),
            )?;
            applied.push(format!("fire_level_bias={}", fl.as_u8()));
        }
        if let Some(scale) = rec.limit_scale {
            eng.self_apply(
                tif_core::SelfApplyKnob::Thresholds,
                tif_core::SelfApplyValue::LimitScale(scale),
            )?;
            applied.push(format!("limit_scale={scale}"));
        }
        if let Some(ref tid) = rec.preferred_pressure_template {
            eng.self_apply(
                tif_core::SelfApplyKnob::PressureTemplate,
                tif_core::SelfApplyValue::TemplateId(tid.clone()),
            )?;
            applied.push(format!("pressure_template={tid}"));
        }
        // Explicitly never apply floor / sensitive / verify knobs.
        store.save_adaptation_stats(eng.stats())?;
    }
    Ok(emit_ok(
        serde_json::json!({
            "category": cat.as_str(),
            "recommendation": rec,
            "applied": applied,
        }),
        json,
        |_| {
            println!("Adaptation recommend for {}", cat.as_str());
            if let Some(fl) = rec.suggested_fire_level {
                println!("  fire_level: {}", fl.as_u8());
            }
            if let Some(s) = rec.limit_scale {
                println!("  limit_scale: {s}");
            }
            if let Some(ref t) = rec.preferred_pressure_template {
                println!("  pressure_template: {t}");
            }
            if let Some(ref r) = rec.preferred_reviewer_id {
                println!("  reviewer: {r}");
            }
            for n in &rec.notes {
                println!("  note: {n}");
            }
            if apply {
                println!("self-applied (allowlisted): {applied:?}");
            }
        },
    ))
}

fn cmd_adaptation_reset(root: &Path, json: bool) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let paths = RepoPaths::for_root(root);
    let store = AuditStore::open(&paths, &cfg.audit)?;
    let mut eng = AdaptationEngine::load(store.load_adaptation_stats()?);
    eng.reset();
    store.save_adaptation_stats(eng.stats())?;
    Ok(emit_ok(serde_json::json!({"reset": true}), json, |_| {
        println!("Local adaptation stats reset.")
    }))
}

//! This Is Fine CLI (`tif`).

mod cli;

use anyhow::Context;
use clap::Parser;
use cli::{Cli, Commands, PolicyCmd, RunCmd};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tif_core::assess::{AssessmentStatus, DamageAssessor};
use tif_core::audit::{compact_status, AuditStore};
use tif_core::config::{
    apply_cli_overrides, init_repository, load_config, RepoPaths, SHARED_CONFIG_NAME,
};
use tif_core::fire_level::FireLevel;
use tif_core::inspector::{find_repo_root, RepositoryInspector};
use tif_core::orchestrator::{BeginRunRequest, RunOrchestrator, RunState};
use tif_core::policy::{PolicyCompileRequest, PolicyCompiler};
use tif_core::protocol::{
    AssessResult, FireLevelResult, JsonResponse, PolicyResolveResult, RollbackResult,
    RunBeginResult, VerifyResult,
};
use tif_core::scoring::{CorrectnessFloor, DiffMetrics, SimplicityScorer};
use tif_core::verify::{plan_and_run, VerificationPlanner};
use tif_core::{AdaptationEngine, FiveAlarmPlan};

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
                verification_passed,
            } => cmd_run_complete(
                &root,
                &run_id,
                DiffMetrics {
                    runtime_dependencies_added: deps_added,
                    files_added,
                    files_changed,
                    files_deleted: 0,
                    lines_added,
                    lines_removed,
                    ..Default::default()
                },
                auto_firebreak,
                verification_passed,
                json,
            ),
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
        } => cmd_assess(
            &root,
            DiffMetrics {
                runtime_dependencies_added: deps_added,
                files_added,
                files_changed,
                lines_added,
                lines_removed,
                ..Default::default()
            },
            fire_level,
            json,
        ),
        Commands::Firebreak {
            run_id,
            files_added,
            files_changed,
            lines_added,
            deps_added,
        } => cmd_firebreak(
            &root,
            run_id,
            DiffMetrics {
                runtime_dependencies_added: deps_added,
                files_added,
                files_changed,
                lines_added,
                ..Default::default()
            },
            json,
        ),
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
        Commands::FiveAlarm { plan } => {
            if plan {
                let plan = FiveAlarmPlan::staged_recovery();
                emit(&plan, json, |p| {
                    println!("Five-Alarm staged recovery:");
                    for (i, s) in p.steps.iter().enumerate() {
                        println!("  {}. {s}", i + 1);
                    }
                });
            } else {
                anyhow::bail!(
                    "Five-Alarm cannot be selected for initial implementation; use after containment failure (see `tif five-alarm --plan`)"
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        Commands::Tui => {
            eprintln!(
                "TUI is scaffolded for a later pass. Use CLI commands (every TUI action has a CLI equivalent)."
            );
            eprintln!("Try: tif status | tif assess | tif audit show | tif policy resolve");
            Ok(ExitCode::from(2))
        }
        Commands::Adaptation { show } => cmd_adaptation(&root, show, json),
    }
}

fn resolve_root(repo: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let start = repo.unwrap_or(std::env::current_dir()?);
    Ok(find_repo_root(&start))
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
    // Keep OutOfControl open so manual `tif firebreak --run-id` can still run.
    // Contained / Restored / Rejected / Applied runs may close.
    if run.state != RunState::OutOfControl {
        let _ = orch.close(&mut run);
    }
    store.record_run(&run)?;

    Ok(emit_ok(
        serde_json::json!({
            "run_id": run.id.as_str(),
            "state": run.state,
            "assessment": run.assessment,
            "firebreak": run.firebreak,
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

fn cmd_firebreak(
    root: &Path,
    run_id: Option<String>,
    metrics: DiffMetrics,
    json: bool,
) -> anyhow::Result<ExitCode> {
    let cfg = load_config(root)?;
    let paths = RepoPaths::for_root(root);
    let store = AuditStore::open(&paths, &cfg.audit)?;
    let orch = RunOrchestrator::new();

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
                task_text: Some("manual firebreak".into()),
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
    orch.run_firebreak(&cfg, &mut run, &floor)?;
    store.record_run(&run)?;

    Ok(emit_ok(
        serde_json::json!({
            "run_id": run.id.as_str(),
            "state": run.state,
            "firebreak": run.firebreak,
        }),
        json,
        |_| {
            if let Some(ref fb) = run.firebreak {
                println!("{}", fb.message);
                println!("applied: {}", fb.applied);
                println!("original_preserved: {}", fb.original_preserved);
            } else {
                println!("no firebreak outcome");
            }
        },
    ))
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
    let store = AuditStore::open(&RepoPaths::for_root(root), &cfg.audit)?;
    let mut run = store
        .get_run(run_id)?
        .with_context(|| format!("run not found: {run_id}"))?;

    // Scaffold: marks rollback intent; full patch restore uses isolation backends.
    run.events
        .push("rollback requested (logical mark; workspace patch restore not yet wired)".into());
    run.state = RunState::Restored;
    run.rollback_candidate_id = Some("original".into());
    store.record_run(&run)?;

    let result = RollbackResult {
        run_id: run_id.into(),
        restored: true,
        message:
            "Rollback marked in audit (logical); filesystem restore requires isolation backend"
                .into(),
    };
    Ok(emit_ok(result, json, |r| {
        println!("{}", r.message);
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
    let store = AuditStore::open(&RepoPaths::for_root(root), &cfg.audit)?;
    let report = store.gc()?;
    Ok(emit_ok(report, json, |r| {
        println!(
            "gc: deleted_runs={} deleted_events={} reclaimed_bytes={}",
            r.deleted_runs, r.deleted_events, r.reclaimed_bytes
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

fn cmd_adaptation(_root: &Path, _show: bool, json: bool) -> anyhow::Result<ExitCode> {
    // MVP: empty local engine; stats will load from SQLite in a later pass.
    let eng = AdaptationEngine::new();
    let rec = eng.recommend(tif_core::TaskCategory::Unknown);
    Ok(emit_ok(
        serde_json::json!({
            "stats": eng.stats(),
            "recommendation": rec,
        }),
        json,
        |_| {
            println!("Local adaptation (no telemetry)");
            println!("runs: {}", eng.stats().total_runs);
            println!("recommendations: {:?}", rec.notes);
        },
    ))
}

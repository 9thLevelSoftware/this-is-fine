//! Interactive production TUI for This Is Fine.
//!
//! Layout: header · nav · center · right metrics · bottom event stream.
//! Keyboard navigation is mandatory. Every view maps to a CLI equivalent.
//! Non-interactive environments should use the CLI/JSON path instead.

use anyhow::Context;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Terminal;
use std::io::{self, stdout};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tif_core::audit::AuditStore;
use tif_core::config::{load_config, Config, RepoPaths};
use tif_core::credentials::resolve_credential_opt;
use tif_core::diff::metrics_from_git;
use tif_core::fire_level::FireLevel;
use tif_core::isolation::isolator_for_session;
use tif_core::orchestrator::{RunOrchestrator, RunState};
use tif_core::providers::{backend_for_provider, BackendRegistry};
use tif_core::scoring::{CorrectnessFloor, SimplicityScorer};
use tif_core::{compact_status, DamageAssessor};

/// Minimum terminal width before the right metrics panel is collapsed.
const NARROW_WIDTH: u16 = 100;

/// Primary navigation screens (every screen has a CLI equivalent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    Dashboard,
    CurrentRun,
    DamageAssessment,
    Firebreak,
    Reviewers,
    Adaptation,
    Audit,
    Settings,
}

impl Screen {
    pub const ALL: [Screen; 8] = [
        Screen::Dashboard,
        Screen::CurrentRun,
        Screen::DamageAssessment,
        Screen::Firebreak,
        Screen::Reviewers,
        Screen::Adaptation,
        Screen::Audit,
        Screen::Settings,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Screen::Dashboard => "Dashboard",
            Screen::CurrentRun => "Current Run",
            Screen::DamageAssessment => "Damage Assessment",
            Screen::Firebreak => "Firebreak",
            Screen::Reviewers => "Reviewers",
            Screen::Adaptation => "Adaptation",
            Screen::Audit => "Audit",
            Screen::Settings => "Settings",
        }
    }

    pub fn cli_hint(self) -> &'static str {
        match self {
            Screen::Dashboard => "tif status",
            Screen::CurrentRun => "tif run show <id> · tif audit show",
            Screen::DamageAssessment => "tif assess [--from-git]",
            Screen::Firebreak => "tif approve|reject <id> · tif firebreak · tif rollback",
            Screen::Reviewers => "tif reviewer list|probe|test",
            Screen::Adaptation => "tif adaptation status|recommend",
            Screen::Audit => "tif audit show [--limit N] · tif audit --gc|--purge",
            Screen::Settings => "tif fire-level · tif policy resolve · edit .this-is-fine.toml",
        }
    }

    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn from_index(idx: usize) -> Option<Self> {
        Self::ALL.get(idx).copied()
    }
}

/// Pending confirmation for destructive operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingConfirm {
    Rollback { run_id: String },
    PurgeAudit,
}

/// In-memory TUI state (testable without a terminal).
#[derive(Debug)]
pub struct AppState {
    pub root: PathBuf,
    pub screen: Screen,
    pub nav_index: usize,
    pub enabled: bool,
    pub fire_level: u8,
    pub compact: String,
    pub recent_runs: Vec<String>,
    pub run_ids: Vec<String>,
    pub selected_run: usize,
    pub live_events: Vec<String>,
    pub assessment_summary: String,
    pub metrics_line: String,
    pub reviewers: usize,
    pub reviewer_lines: Vec<String>,
    pub probe_lines: Vec<String>,
    pub audit_tier: String,
    pub audit_filter: String,
    pub audit_lines: Vec<String>,
    pub settings_lines: Vec<String>,
    pub adaptation_lines: Vec<String>,
    pub awaiting_runs: Vec<String>,
    pub message: String,
    pub event_stream: Vec<String>,
    pub pending_confirm: Option<PendingConfirm>,
    pub filter_input_mode: bool,
    pub terminal_width: u16,
}

impl AppState {
    pub fn load(root: &Path) -> anyhow::Result<Self> {
        let cfg = load_config(root).unwrap_or_default();
        let compact = compact_status(cfg.default_fire_level, cfg.enabled);
        let paths = RepoPaths::for_root(root);

        let mut recent_runs = Vec::new();
        let mut run_ids = Vec::new();
        let mut live_events = Vec::new();
        let mut audit_lines = Vec::new();
        let mut awaiting_runs = Vec::new();
        let mut event_stream = Vec::new();

        if let Ok(store) = AuditStore::open(&paths, &cfg.audit) {
            if let Ok(runs) = store.list_runs(24) {
                for r in runs {
                    let line = format!("{}  {}  score={:?}", r.id, r.state, r.simplicity_score);
                    recent_runs.push(line.clone());
                    audit_lines.push(line);
                    run_ids.push(r.id.clone());
                    if r.state == "awaiting_approval" {
                        awaiting_runs.push(r.id.clone());
                    }
                    if let Ok(Some(full)) = store.get_run(&r.id) {
                        for e in full.events.iter().rev().take(8) {
                            live_events.push(format!("{}: {e}", r.id));
                        }
                        for e in full.events.iter().rev().take(3) {
                            event_stream.push(format!("[{}] {e}", &r.id[..8.min(r.id.len())]));
                        }
                    }
                }
            }
        }
        event_stream.truncate(40);
        live_events.truncate(48);
        if event_stream.is_empty() {
            event_stream.push("No run events yet — tif run begin to start".into());
        }

        let (assessment_summary, metrics_line) = match metrics_from_git(root) {
            Ok(m) => {
                let fl = FireLevel::parse_initial(cfg.default_fire_level)
                    .unwrap_or(FireLevel::Containment);
                let floor = CorrectnessFloor {
                    verification_passed: false,
                    notes: vec!["TUI metrics-only preview; correctness floor not evaluated".into()],
                    ..CorrectnessFloor::all_pass()
                };
                let score_floor = CorrectnessFloor::all_pass();
                let scorer = SimplicityScorer::new(
                    cfg.simplicity.weights.clone(),
                    cfg.simplicity.limits.clone(),
                );
                let score = scorer.score(&m, &score_floor);
                let da =
                    DamageAssessor::build("tui", "working-tree", fl, m.clone(), score, floor, None);
                (
                    format!(
                        "[metrics-only / unverified] {} · run tif verify for floor",
                        da.summary
                    ),
                    format!(
                        "+{} lines · {} added · {} changed · {} deps (unverified)",
                        m.lines_added, m.files_added, m.files_changed, m.runtime_dependencies_added
                    ),
                )
            }
            Err(_) => (
                "No git metrics (use tif assess --from-git in a git repo)".into(),
                "metrics: n/a".into(),
            ),
        };

        let reviewer_lines: Vec<String> = cfg
            .reviewers
            .iter()
            .map(|r| {
                format!(
                    "{}  provider={}  model={}  egress={}",
                    r.id, r.provider, r.model, r.allow_source_egress
                )
            })
            .collect();

        let settings_lines = settings_from_config(&cfg, root);
        let adaptation_lines = vec![
            "Local adaptation ranks pressure variants and reviewers".into(),
            "without network telemetry.".into(),
            String::new(),
            "CLI: tif adaptation status".into(),
            "CLI: tif adaptation recommend --category bug_fix".into(),
            "Self-apply never weakens floor / sensitive paths / verify.".into(),
        ];

        Ok(Self {
            root: root.to_path_buf(),
            screen: Screen::Dashboard,
            nav_index: 0,
            enabled: cfg.enabled,
            fire_level: cfg.default_fire_level,
            compact,
            recent_runs,
            run_ids,
            selected_run: 0,
            live_events,
            assessment_summary,
            metrics_line,
            reviewers: cfg.reviewers.len(),
            reviewer_lines,
            probe_lines: Vec::new(),
            audit_tier: cfg.audit.tier.clone(),
            audit_filter: String::new(),
            audit_lines,
            settings_lines,
            adaptation_lines,
            awaiting_runs,
            message: help_for(Screen::Dashboard),
            event_stream,
            pending_confirm: None,
            filter_input_mode: false,
            terminal_width: 120,
        })
    }

    pub fn refresh(&mut self) {
        if let Ok(next) = Self::load(&self.root) {
            let screen = self.screen;
            let nav_index = self.nav_index;
            let selected_run = self.selected_run;
            let audit_filter = self.audit_filter.clone();
            let width = self.terminal_width;
            *self = next;
            self.screen = screen;
            self.nav_index = nav_index;
            self.selected_run = selected_run.min(self.run_ids.len().saturating_sub(1));
            self.audit_filter = audit_filter;
            self.terminal_width = width;
            self.push_event("Refreshed from audit store + config");
            self.message = "Refreshed".into();
        } else {
            self.message = "Refresh failed".into();
        }
    }

    pub fn push_event(&mut self, msg: impl Into<String>) {
        self.event_stream.insert(0, msg.into());
        self.event_stream.truncate(48);
    }

    pub fn selected_run_id(&self) -> Option<&str> {
        self.run_ids.get(self.selected_run).map(|s| s.as_str())
    }

    pub fn filtered_audit_lines(&self) -> Vec<String> {
        if self.audit_filter.is_empty() {
            return self.audit_lines.clone();
        }
        let f = self.audit_filter.to_ascii_lowercase();
        self.audit_lines
            .iter()
            .filter(|l| l.to_ascii_lowercase().contains(&f))
            .cloned()
            .collect()
    }

    pub fn is_narrow(&self) -> bool {
        self.terminal_width < NARROW_WIDTH
    }

    /// Pure navigation transition (unit-tested).
    pub fn select_screen_index(&mut self, idx: usize) {
        if let Some(s) = Screen::from_index(idx) {
            self.nav_index = idx;
            self.screen = s;
            self.filter_input_mode = false;
            self.message = help_for(s);
        }
    }

    pub fn cycle_next(&mut self) {
        self.screen = self.screen.next();
        self.nav_index = Screen::ALL
            .iter()
            .position(|s| *s == self.screen)
            .unwrap_or(0);
        self.filter_input_mode = false;
        self.message = help_for(self.screen);
    }

    pub fn cycle_prev(&mut self) {
        self.screen = self.screen.prev();
        self.nav_index = Screen::ALL
            .iter()
            .position(|s| *s == self.screen)
            .unwrap_or(0);
        self.filter_input_mode = false;
        self.message = help_for(self.screen);
    }
}

fn help_for(screen: Screen) -> String {
    match screen {
        Screen::Firebreak => {
            "a approve · x reject · R rollback (confirm) · ↑↓ select run · r refresh · q quit"
                .into()
        }
        Screen::Reviewers => "p probe reviewers · r refresh · q quit".into(),
        Screen::Audit => {
            "/ filter · Esc clear filter · G gc · P purge (confirm) · r refresh · q quit".into()
        }
        Screen::CurrentRun => "↑↓ select run · r refresh · q quit".into(),
        _ => "↑↓ navigate · Tab next · 1-8 jump · r refresh · q quit".into(),
    }
}

fn settings_from_config(cfg: &Config, root: &Path) -> Vec<String> {
    vec![
        format!("root: {}", root.display()),
        format!("enabled = {}", cfg.enabled),
        format!("default_fire_level = {}", cfg.default_fire_level),
        format!("audit.tier = {}", cfg.audit.tier),
        format!("audit.max_age_days = {}", cfg.audit.max_age_days),
        format!("audit.max_size_mb = {}", cfg.audit.max_size_mb),
        format!("ci.on_violation = {}", cfg.ci.on_violation),
        format!("ci.allow_write = {}", cfg.ci.allow_write),
        format!("ci.allow_pr = {}", cfg.ci.allow_pr),
        format!(
            "approval.auto_apply_firebreak = {}",
            cfg.approval.auto_apply_firebreak
        ),
        format!(
            "approval.require_firebreak_approval = {}",
            cfg.approval.require_firebreak_approval
        ),
        format!(
            "verification.commands = {} entries",
            cfg.verification.commands.len()
        ),
        format!("reviewers (local) = {}", cfg.reviewers.len()),
        String::new(),
        "Shared: .this-is-fine.toml".into(),
        "Local:  .this-is-fine.local.toml (gitignored)".into(),
        String::new(),
        format!("CLI: {}", Screen::Settings.cli_hint()),
    ]
}

/// Run the interactive TUI. Returns an exit code-ish status via Result.
pub fn run_tui(root: &Path) -> anyhow::Result<()> {
    if !is_interactive() {
        anyhow::bail!(
            "TUI requires an interactive terminal. Use CLI commands instead (tif status, tif assess, tif audit show)."
        );
    }

    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen).context("enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("create terminal")?;

    let mut app = AppState::load(root)?;
    let result = event_loop(&mut terminal, &mut app);

    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    result
}

fn is_interactive() -> bool {
    use std::io::IsTerminal;
    if std::env::var_os("CI").is_some() {
        return false;
    }
    if std::env::var_os("TIF_FORCE_CLI").is_some() {
        return false;
    }
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut AppState,
) -> anyhow::Result<()> {
    loop {
        let size = terminal.size()?;
        app.terminal_width = size.width;
        terminal.draw(|f| draw(f, app))?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if handle_key(app, key.code, key.modifiers)? {
                    break;
                }
            }
        }
    }
    Ok(())
}

/// Handle one key. Returns true when the app should quit.
fn handle_key(app: &mut AppState, code: KeyCode, _mods: KeyModifiers) -> anyhow::Result<bool> {
    // Confirmation modal takes exclusive focus.
    if let Some(ref pending) = app.pending_confirm.clone() {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let p = pending.clone();
                app.pending_confirm = None;
                match p {
                    PendingConfirm::Rollback { run_id } => do_rollback(app, &run_id)?,
                    PendingConfirm::PurgeAudit => do_purge(app)?,
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.pending_confirm = None;
                app.message = "Cancelled".into();
                app.push_event("Confirmation cancelled");
            }
            _ => {}
        }
        return Ok(false);
    }

    // Audit filter input mode.
    if app.filter_input_mode {
        match code {
            KeyCode::Esc => {
                app.filter_input_mode = false;
                app.audit_filter.clear();
                app.message = help_for(Screen::Audit);
            }
            KeyCode::Enter => {
                app.filter_input_mode = false;
                app.message = format!("Filter: {:?}", app.audit_filter);
                app.push_event(format!("Audit filter set to '{}'", app.audit_filter));
            }
            KeyCode::Backspace => {
                app.audit_filter.pop();
            }
            KeyCode::Char(c) => {
                app.audit_filter.push(c);
            }
            _ => {}
        }
        return Ok(false);
    }

    match code {
        KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
        KeyCode::Char('r') => app.refresh(),
        KeyCode::Down | KeyCode::Char('j') => {
            if matches!(
                app.screen,
                Screen::CurrentRun | Screen::Firebreak | Screen::Audit
            ) && !app.run_ids.is_empty()
            {
                app.selected_run = (app.selected_run + 1) % app.run_ids.len();
            } else {
                app.nav_index = (app.nav_index + 1) % Screen::ALL.len();
                app.screen = Screen::ALL[app.nav_index];
                app.message = help_for(app.screen);
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if matches!(
                app.screen,
                Screen::CurrentRun | Screen::Firebreak | Screen::Audit
            ) && !app.run_ids.is_empty()
            {
                app.selected_run = (app.selected_run + app.run_ids.len() - 1) % app.run_ids.len();
            } else {
                app.nav_index = (app.nav_index + Screen::ALL.len() - 1) % Screen::ALL.len();
                app.screen = Screen::ALL[app.nav_index];
                app.message = help_for(app.screen);
            }
        }
        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => app.cycle_next(),
        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => app.cycle_prev(),
        KeyCode::Enter => {
            app.screen = Screen::ALL[app.nav_index];
            app.message = format!("{} · CLI: {}", app.screen.title(), app.screen.cli_hint());
        }
        KeyCode::Char('1') => app.select_screen_index(0),
        KeyCode::Char('2') => app.select_screen_index(1),
        KeyCode::Char('3') => app.select_screen_index(2),
        KeyCode::Char('4') => app.select_screen_index(3),
        KeyCode::Char('5') => app.select_screen_index(4),
        KeyCode::Char('6') => app.select_screen_index(5),
        KeyCode::Char('7') => app.select_screen_index(6),
        KeyCode::Char('8') => app.select_screen_index(7),
        KeyCode::Char('a') if app.screen == Screen::Firebreak => do_approve(app)?,
        KeyCode::Char('x') if app.screen == Screen::Firebreak => do_reject(app)?,
        KeyCode::Char('R') if app.screen == Screen::Firebreak => {
            if let Some(id) = app.selected_run_id().map(|s| s.to_string()) {
                app.pending_confirm = Some(PendingConfirm::Rollback { run_id: id.clone() });
                app.message = format!("Confirm rollback of {id}? [y/n]");
            } else {
                app.message = "No run selected for rollback".into();
            }
        }
        KeyCode::Char('p') if app.screen == Screen::Reviewers => do_probe(app)?,
        KeyCode::Char('/') if app.screen == Screen::Audit => {
            app.filter_input_mode = true;
            app.message = format!("Filter: {}_", app.audit_filter);
        }
        KeyCode::Char('G') if app.screen == Screen::Audit => do_gc(app)?,
        KeyCode::Char('P') if app.screen == Screen::Audit => {
            app.pending_confirm = Some(PendingConfirm::PurgeAudit);
            app.message = "Confirm PURGE all local audit data? [y/n]".into();
        }
        _ => {}
    }
    Ok(false)
}

fn do_approve(app: &mut AppState) -> anyhow::Result<()> {
    let Some(id) = app.selected_run_id().map(|s| s.to_string()) else {
        app.message = "No run selected".into();
        return Ok(());
    };
    let cfg = load_config(&app.root)?;
    let paths = RepoPaths::for_root(&app.root);
    let store = AuditStore::open(&paths, &cfg.audit)?;
    let mut run = match store.get_run(&id)? {
        Some(r) => r,
        None => {
            app.message = format!("run not found: {id}");
            return Ok(());
        }
    };
    if run.state != RunState::AwaitingApproval {
        app.message = format!(
            "run {} is {} (need awaiting_approval)",
            id,
            run.state.as_str()
        );
        app.push_event(app.message.clone());
        return Ok(());
    }
    let orch = RunOrchestrator::new();
    match orch.approve_firebreak(&cfg, &mut run) {
        Ok(()) => {
            store.record_run(&run)?;
            let _ = store.record_adaptation_from_run(&run);
            app.message = format!("Approved {id} → {}", run.state.as_str());
            app.push_event(format!("approve {id} ok"));
            app.refresh();
        }
        Err(e) => {
            let _ = store.record_run(&run);
            app.message = format!("Approve failed: {e}");
            app.push_event(app.message.clone());
        }
    }
    Ok(())
}

fn do_reject(app: &mut AppState) -> anyhow::Result<()> {
    let Some(id) = app.selected_run_id().map(|s| s.to_string()) else {
        app.message = "No run selected".into();
        return Ok(());
    };
    let cfg = load_config(&app.root)?;
    let paths = RepoPaths::for_root(&app.root);
    let store = AuditStore::open(&paths, &cfg.audit)?;
    let mut run = match store.get_run(&id)? {
        Some(r) => r,
        None => {
            app.message = format!("run not found: {id}");
            return Ok(());
        }
    };
    let orch = RunOrchestrator::new();
    match orch.reject_firebreak(&mut run, Some("rejected from TUI".into())) {
        Ok(()) => {
            store.record_run(&run)?;
            app.message = format!("Rejected {id}");
            app.push_event(format!("reject {id} ok"));
            app.refresh();
        }
        Err(e) => {
            app.message = format!("Reject failed: {e}");
            app.push_event(app.message.clone());
        }
    }
    Ok(())
}

fn do_rollback(app: &mut AppState, run_id: &str) -> anyhow::Result<()> {
    let cfg = load_config(&app.root)?;
    let paths = RepoPaths::for_root(&app.root);
    let store = AuditStore::open(&paths, &cfg.audit)?;
    let mut run = match store.get_run(run_id)? {
        Some(r) => r,
        None => {
            app.message = format!("run not found: {run_id}");
            return Ok(());
        }
    };
    let orch = RunOrchestrator::new();
    if let Some(ref session) = run.isolation_session {
        if session.applied || session.restore_pending {
            let isolator = isolator_for_session(session, &paths.snapshots_dir());
            match orch.rollback_isolation(&mut run, isolator.as_ref()) {
                Ok(()) => {
                    store.record_run(&run)?;
                    app.message = format!("Rolled back {run_id}");
                    app.push_event(format!("rollback {run_id} restored"));
                    app.refresh();
                }
                Err(e) => {
                    run.events.push(format!("rollback failed: {e}"));
                    store.record_run(&run)?;
                    app.message = format!("Rollback failed: {e}");
                    app.push_event(app.message.clone());
                }
            }
            return Ok(());
        }
    }
    run.events
        .push("rollback from TUI; original intact (noop)".into());
    run.state = RunState::Restored;
    store.record_run(&run)?;
    app.message = format!("Rollback noop for {run_id} (never applied)");
    app.push_event(app.message.clone());
    app.refresh();
    Ok(())
}

fn do_probe(app: &mut AppState) -> anyhow::Result<()> {
    let cfg = load_config(&app.root)?;
    if cfg.reviewers.is_empty() {
        app.message = "No authorized reviewers in local config".into();
        app.probe_lines = vec![app.message.clone()];
        app.push_event(app.message.clone());
        return Ok(());
    }
    let reg = BackendRegistry::new();
    let mut lines = Vec::new();
    for r in &cfg.reviewers {
        let backend = match backend_for_provider(&reg, &r.provider) {
            Ok(b) => b,
            Err(e) => {
                lines.push(format!("{} FAIL: {e}", r.id));
                continue;
            }
        };
        let cred = resolve_credential_opt(r.credential_ref.as_deref())
            .ok()
            .flatten();
        match backend.probe(r, cred.as_deref()) {
            Ok(p) => {
                lines.push(format!(
                    "{} {}: {}",
                    r.id,
                    if p.ok { "ok" } else { "FAIL" },
                    p.message
                ));
            }
            Err(e) => lines.push(format!("{} FAIL: {e}", r.id)),
        }
    }
    app.probe_lines = lines.clone();
    app.message = format!("Probed {} reviewer(s)", cfg.reviewers.len());
    app.push_event(app.message.clone());
    for l in lines {
        app.push_event(l);
    }
    Ok(())
}

fn do_gc(app: &mut AppState) -> anyhow::Result<()> {
    let cfg = load_config(&app.root)?;
    let paths = RepoPaths::for_root(&app.root);
    let store = AuditStore::open(&paths, &cfg.audit)?;
    let report = store.gc()?;
    app.message = format!(
        "GC: runs={} events={} reclaimed={}B isolation_removed={}",
        report.deleted_runs,
        report.deleted_events,
        report.reclaimed_bytes,
        report.isolation_removed
    );
    app.push_event(app.message.clone());
    app.refresh();
    Ok(())
}

fn do_purge(app: &mut AppState) -> anyhow::Result<()> {
    let cfg = load_config(&app.root)?;
    let paths = RepoPaths::for_root(&app.root);
    let store = AuditStore::open(&paths, &cfg.audit)?;
    store.purge_all()?;
    app.message = "Purged all local audit data".into();
    app.push_event(app.message.clone());
    app.refresh();
    Ok(())
}

fn draw(f: &mut ratatui::Frame, app: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(5),    // body
            Constraint::Length(6), // event stream
            Constraint::Length(3), // help
        ])
        .split(f.area());

    // Header
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            " This Is Fine ",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" · {} ", app.compact)),
        Span::raw(format!("· {}", app.root.display())),
    ]))
    .block(Block::default().borders(Borders::ALL).title("status"));
    f.render_widget(header, chunks[0]);

    // Body: nav + main + optional metrics
    if app.is_narrow() {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(20), Constraint::Min(20)])
            .split(chunks[1]);
        draw_nav(f, body[0], app);
        draw_main(f, body[1], app);
    } else {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(22),
                Constraint::Min(30),
                Constraint::Length(28),
            ])
            .split(chunks[1]);
        draw_nav(f, body[0], app);
        draw_main(f, body[1], app);
        draw_metrics(f, body[2], app);
    }

    // Bottom event stream (live run events)
    draw_event_stream(f, chunks[2], app);

    // Footer help / message
    let footer = Paragraph::new(app.message.as_str())
        .block(Block::default().borders(Borders::ALL).title("help"));
    f.render_widget(footer, chunks[3]);

    if app.pending_confirm.is_some() {
        draw_confirm_modal(f, app);
    }
}

fn draw_nav(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
    let items: Vec<ListItem> = Screen::ALL
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let label = format!("{}. {}", i + 1, s.title());
            if i == app.nav_index {
                ListItem::new(Span::styled(
                    label,
                    Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
                ))
            } else {
                ListItem::new(label)
            }
        })
        .collect();
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title("navigate"));
    f.render_widget(list, area);
}

fn draw_main(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
    let title = app.screen.title();
    let text = match app.screen {
        Screen::Dashboard => format!(
            "Repository: {}\n\
             Containment: {}\n\
             Fire Level: {}\n\
             Reviewers authorized: {}\n\
             Audit tier: {}\n\
             Awaiting approval: {}\n\
             \n\
             {}\n\
             {}\n\
             \n\
             Contain the fire. Do not remodel the building.\n\
             \n\
             CLI: {}",
            app.root.display(),
            if app.enabled { "enabled" } else { "suspended" },
            app.fire_level,
            app.reviewers,
            app.audit_tier,
            app.awaiting_runs.len(),
            app.assessment_summary,
            app.metrics_line,
            app.screen.cli_hint()
        ),
        Screen::CurrentRun => {
            let mut body = String::from("Recent runs (select with ↑↓):\n\n");
            if app.recent_runs.is_empty() {
                body.push_str("(no runs recorded yet)\n\nUse: tif run begin --task \"…\"");
            } else {
                for (i, r) in app.recent_runs.iter().enumerate() {
                    let mark = if i == app.selected_run { ">" } else { " " };
                    body.push_str(&format!("{mark} {r}\n"));
                }
            }
            body.push_str("\n--- live events ---\n");
            if app.live_events.is_empty() {
                body.push_str("(none)\n");
            } else {
                for e in app.live_events.iter().take(12) {
                    body.push_str(e);
                    body.push('\n');
                }
            }
            body.push_str(&format!("\nCLI: {}", app.screen.cli_hint()));
            body
        }
        Screen::DamageAssessment => format!(
            "{}\n\
             {}\n\
             \n\
             Score uses repository weights and hard limits.\n\
             Correctness floor is a gate, not a weight.\n\
             \n\
             CLI: {}",
            app.assessment_summary,
            app.metrics_line,
            app.screen.cli_hint()
        ),
        Screen::Firebreak => {
            let mut body = String::from(
                "Firebreak: isolated reviewer → re-verify → rank → apply/approval.\n\
                 Failed or unverified candidates never modify the original workspace.\n\n\
                 Awaiting approval:\n",
            );
            if app.awaiting_runs.is_empty() {
                body.push_str("  (none)\n");
            } else {
                for id in &app.awaiting_runs {
                    body.push_str(&format!("  • {id}\n"));
                }
            }
            body.push_str("\nRuns (↑↓ select · a approve · x reject · R rollback):\n");
            if app.run_ids.is_empty() {
                body.push_str("  (no runs)\n");
            } else {
                for (i, id) in app.run_ids.iter().enumerate() {
                    let mark = if i == app.selected_run { ">" } else { " " };
                    let line = app.recent_runs.get(i).map(|s| s.as_str()).unwrap_or(id);
                    body.push_str(&format!("{mark} {line}\n"));
                }
            }
            body.push_str(&format!("\nCLI: {}", app.screen.cli_hint()));
            body
        }
        Screen::Reviewers => {
            let mut body = format!(
                "Authorized reviewers: {}\n\
                 Configure in .this-is-fine.local.toml (gitignored).\n\
                 Hosted source egress requires explicit allow_source_egress.\n\n",
                app.reviewers
            );
            if app.reviewer_lines.is_empty() {
                body.push_str("(none configured)\n");
            } else {
                for l in &app.reviewer_lines {
                    body.push_str(l);
                    body.push('\n');
                }
            }
            if !app.probe_lines.is_empty() {
                body.push_str("\n--- probe results ---\n");
                for l in &app.probe_lines {
                    body.push_str(l);
                    body.push('\n');
                }
            } else {
                body.push_str("\nPress p to probe connectivity.\n");
            }
            body.push_str(&format!("\nCLI: {}", app.screen.cli_hint()));
            body
        }
        Screen::Adaptation => app.adaptation_lines.join("\n"),
        Screen::Audit => {
            let mut body = format!(
                "Audit log (local SQLite + CAS)  filter: {:?}\n\n",
                app.audit_filter
            );
            let lines = app.filtered_audit_lines();
            if lines.is_empty() {
                body.push_str("(empty or no match)\n");
            } else {
                for (i, r) in lines.iter().enumerate() {
                    let mark = if i == app.selected_run { ">" } else { " " };
                    body.push_str(&format!("{mark} {r}\n"));
                }
            }
            body.push_str("\n/ filter · G gc · P purge\n");
            body.push_str(&format!("CLI: {}", app.screen.cli_hint()));
            body
        }
        Screen::Settings => app.settings_lines.join("\n"),
    };

    let para = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(para, area);
}

fn draw_metrics(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
    let await_n = app.awaiting_runs.len();
    let text = format!(
        "Fire Level {}\n\
         {}\n\
         \n\
         {}\n\
         \n\
         Reviewers: {}\n\
         Awaiting: {}\n\
         Runs: {}\n\
         \n\
         Press 1-8 or ↑↓\n\
         r refresh · q quit",
        app.fire_level,
        app.compact,
        app.metrics_line,
        app.reviewers,
        await_n,
        app.run_ids.len()
    );
    let para = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title("metrics"));
    f.render_widget(para, area);
}

fn draw_event_stream(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
    let body = app
        .event_stream
        .iter()
        .take(area.height.saturating_sub(2) as usize)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let para = Paragraph::new(body)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title("event stream"));
    f.render_widget(para, area);
}

fn draw_confirm_modal(f: &mut ratatui::Frame, app: &AppState) {
    let area = centered_rect(60, 5, f.area());
    let msg = match app.pending_confirm {
        Some(PendingConfirm::Rollback { ref run_id }) => {
            format!("Rollback run {run_id}?\nThis restores the baseline. [y]es / [n]o")
        }
        Some(PendingConfirm::PurgeAudit) => {
            "PURGE all local audit data?\nThis cannot be undone. [y]es / [n]o".into()
        }
        None => return,
    };
    f.render_widget(Clear, area);
    let para = Paragraph::new(msg)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title("confirm"));
    f.render_widget(para, area);
}

fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height.min(90)) / 2),
            Constraint::Length(height),
            Constraint::Percentage((100 - height.min(90)) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_cycle_wraps() {
        let mut s = Screen::Dashboard;
        for _ in 0..Screen::ALL.len() {
            s = s.next();
        }
        assert_eq!(s, Screen::Dashboard);
        let mut s = Screen::Dashboard;
        s = s.prev();
        assert_eq!(s, Screen::Settings);
    }

    #[test]
    fn screen_from_index_and_titles() {
        assert_eq!(Screen::from_index(0), Some(Screen::Dashboard));
        assert_eq!(Screen::from_index(7), Some(Screen::Settings));
        assert_eq!(Screen::from_index(99), None);
        for s in Screen::ALL {
            assert!(!s.title().is_empty());
            assert!(s.cli_hint().contains("tif"));
        }
    }

    #[test]
    fn app_state_navigation() {
        let dir = tempfile::tempdir().unwrap();
        // Minimal repo files so load succeeds
        std::fs::write(
            dir.path().join(".this-is-fine.toml"),
            "version = 1\nenabled = true\ndefault_fire_level = 3\n",
        )
        .unwrap();
        let mut app = AppState::load(dir.path()).unwrap();
        assert_eq!(app.screen, Screen::Dashboard);
        app.cycle_next();
        assert_eq!(app.screen, Screen::CurrentRun);
        app.select_screen_index(3);
        assert_eq!(app.screen, Screen::Firebreak);
        app.select_screen_index(7);
        assert_eq!(app.screen, Screen::Settings);
        assert!(!app.settings_lines.is_empty());
    }

    #[test]
    fn narrow_collapses_right_panel() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".this-is-fine.toml"),
            "version = 1\nenabled = true\ndefault_fire_level = 3\n",
        )
        .unwrap();
        let mut app = AppState::load(dir.path()).unwrap();
        app.terminal_width = 80;
        assert!(app.is_narrow());
        app.terminal_width = 140;
        assert!(!app.is_narrow());
    }

    #[test]
    fn audit_filter_matches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".this-is-fine.toml"),
            "version = 1\nenabled = true\ndefault_fire_level = 3\n",
        )
        .unwrap();
        let mut app = AppState::load(dir.path()).unwrap();
        app.audit_lines = vec![
            "abc  contained  score=Some(1.0)".into(),
            "xyz  failed  score=None".into(),
        ];
        app.audit_filter = "failed".into();
        let f = app.filtered_audit_lines();
        assert_eq!(f.len(), 1);
        assert!(f[0].contains("xyz"));
    }

    #[test]
    fn pending_confirm_variants() {
        let p = PendingConfirm::PurgeAudit;
        assert_eq!(p, PendingConfirm::PurgeAudit);
        let r = PendingConfirm::Rollback { run_id: "x".into() };
        assert!(matches!(r, PendingConfirm::Rollback { .. }));
    }

    #[test]
    fn handle_key_quit_and_nav() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".this-is-fine.toml"),
            "version = 1\nenabled = true\ndefault_fire_level = 3\n",
        )
        .unwrap();
        let mut app = AppState::load(dir.path()).unwrap();
        assert!(!handle_key(&mut app, KeyCode::Char('2'), KeyModifiers::NONE).unwrap());
        assert_eq!(app.screen, Screen::CurrentRun);
        assert!(handle_key(&mut app, KeyCode::Char('q'), KeyModifiers::NONE).unwrap());
    }

    #[test]
    fn confirm_cancel() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".this-is-fine.toml"),
            "version = 1\nenabled = true\ndefault_fire_level = 3\n",
        )
        .unwrap();
        let mut app = AppState::load(dir.path()).unwrap();
        app.pending_confirm = Some(PendingConfirm::PurgeAudit);
        handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE).unwrap();
        assert!(app.pending_confirm.is_none());
    }
}

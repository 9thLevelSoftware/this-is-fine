//! Interactive TUI for This Is Fine.
//!
//! Keyboard navigation is mandatory. Every view maps to a CLI equivalent.
//! Non-interactive environments should use the CLI/JSON path instead.

use anyhow::Context;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Terminal;
use std::io::{self, stdout};
use std::path::Path;
use std::time::Duration;
use tif_core::audit::AuditStore;
use tif_core::config::{load_config, RepoPaths};
use tif_core::diff::metrics_from_git;
use tif_core::fire_level::FireLevel;
use tif_core::scoring::{CorrectnessFloor, SimplicityScorer};
use tif_core::{compact_status, DamageAssessor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
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
    const ALL: [Screen; 8] = [
        Screen::Dashboard,
        Screen::CurrentRun,
        Screen::DamageAssessment,
        Screen::Firebreak,
        Screen::Reviewers,
        Screen::Adaptation,
        Screen::Audit,
        Screen::Settings,
    ];

    fn title(self) -> &'static str {
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

    fn cli_hint(self) -> &'static str {
        match self {
            Screen::Dashboard => "tif status",
            Screen::CurrentRun => "tif run show <id> · tif audit show",
            Screen::DamageAssessment => "tif assess [--from-git]",
            Screen::Firebreak => "tif firebreak [--candidate PATH --apply]",
            Screen::Reviewers => "tif status · edit .this-is-fine.local.toml",
            Screen::Adaptation => "tif adaptation",
            Screen::Audit => "tif audit show",
            Screen::Settings => "tif fire-level · tif policy resolve",
        }
    }

    fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

struct AppState {
    root: std::path::PathBuf,
    screen: Screen,
    nav_index: usize,
    enabled: bool,
    fire_level: u8,
    compact: String,
    recent_runs: Vec<String>,
    assessment_summary: String,
    metrics_line: String,
    reviewers: usize,
    audit_tier: String,
    message: String,
}

impl AppState {
    fn load(root: &Path) -> anyhow::Result<Self> {
        let cfg = load_config(root).unwrap_or_default();
        let compact = compact_status(cfg.default_fire_level, cfg.enabled);
        let mut recent_runs = Vec::new();
        let paths = RepoPaths::for_root(root);
        if let Ok(store) = AuditStore::open(&paths, &cfg.audit) {
            if let Ok(runs) = store.list_runs(8) {
                for r in runs {
                    recent_runs.push(format!(
                        "{}  {}  score={:?}",
                        r.id, r.state, r.simplicity_score
                    ));
                }
            }
        }

        let (assessment_summary, metrics_line) = match metrics_from_git(root) {
            Ok(m) => {
                let fl = FireLevel::parse_initial(cfg.default_fire_level)
                    .unwrap_or(FireLevel::Containment);
                // Metrics-only preview: correctness floor is NOT evaluated (unverified).
                let floor = CorrectnessFloor {
                    verification_passed: false,
                    notes: vec!["TUI metrics-only preview; correctness floor not evaluated".into()],
                    ..CorrectnessFloor::all_pass()
                };
                // For display score only, use all-pass so numbers remain comparable;
                // summary is explicitly labeled unverified.
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

        Ok(Self {
            root: root.to_path_buf(),
            screen: Screen::Dashboard,
            nav_index: 0,
            enabled: cfg.enabled,
            fire_level: cfg.default_fire_level,
            compact,
            recent_runs,
            assessment_summary,
            metrics_line,
            reviewers: cfg.reviewers.len(),
            audit_tier: cfg.audit.tier,
            message: "↑↓ navigate · Tab next · Enter select · r refresh · q quit".into(),
        })
    }

    fn refresh(&mut self) {
        if let Ok(next) = Self::load(&self.root) {
            let screen = self.screen;
            let nav_index = self.nav_index;
            *self = next;
            self.screen = screen;
            self.nav_index = nav_index;
            self.message = "Refreshed".into();
        } else {
            self.message = "Refresh failed".into();
        }
    }
}

/// Run the interactive TUI. Returns an exit code-ish status via Result.
pub fn run_tui(root: &Path) -> anyhow::Result<()> {
    // Non-interactive environments should not enter raw mode.
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
    // Require a real terminal so piped/non-TTY runs fail closed to CLI.
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut AppState,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('r') => app.refresh(),
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.nav_index = (app.nav_index + 1) % Screen::ALL.len();
                        app.screen = Screen::ALL[app.nav_index];
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.nav_index = (app.nav_index + Screen::ALL.len() - 1) % Screen::ALL.len();
                        app.screen = Screen::ALL[app.nav_index];
                    }
                    KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                        app.screen = app.screen.next();
                        app.nav_index = Screen::ALL
                            .iter()
                            .position(|s| *s == app.screen)
                            .unwrap_or(0);
                    }
                    KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                        app.screen = app.screen.prev();
                        app.nav_index = Screen::ALL
                            .iter()
                            .position(|s| *s == app.screen)
                            .unwrap_or(0);
                    }
                    KeyCode::Enter => {
                        app.screen = Screen::ALL[app.nav_index];
                        app.message =
                            format!("{} · CLI: {}", app.screen.title(), app.screen.cli_hint());
                    }
                    KeyCode::Char('1') => select_screen(app, 0),
                    KeyCode::Char('2') => select_screen(app, 1),
                    KeyCode::Char('3') => select_screen(app, 2),
                    KeyCode::Char('4') => select_screen(app, 3),
                    KeyCode::Char('5') => select_screen(app, 4),
                    KeyCode::Char('6') => select_screen(app, 5),
                    KeyCode::Char('7') => select_screen(app, 6),
                    KeyCode::Char('8') => select_screen(app, 7),
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn select_screen(app: &mut AppState, idx: usize) {
    if idx < Screen::ALL.len() {
        app.nav_index = idx;
        app.screen = Screen::ALL[idx];
    }
}

fn draw(f: &mut ratatui::Frame, app: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
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

    // Body: nav + main + metrics
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

    // Footer
    let footer = Paragraph::new(app.message.as_str()).block(
        Block::default()
            .borders(Borders::ALL)
            .title("help / events"),
    );
    f.render_widget(footer, chunks[2]);
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
            app.assessment_summary,
            app.metrics_line,
            app.screen.cli_hint()
        ),
        Screen::CurrentRun => {
            let mut body = String::from("Recent runs (tif audit show):\n\n");
            if app.recent_runs.is_empty() {
                body.push_str("(no runs recorded yet)\n\nUse: tif run begin --task \"…\"");
            } else {
                for r in &app.recent_runs {
                    body.push_str(r);
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
        Screen::Firebreak => format!(
            "Firebreak runs an independent reviewer in isolation.\n\
             Failed or unverified candidates never modify the original workspace.\n\
             Successful smaller candidates can be applied with rollback retention.\n\
             \n\
             Open isolation: automatic on apply path\n\
             Rollback: tif rollback <run_id>\n\
             \n\
             CLI: {}",
            app.screen.cli_hint()
        ),
        Screen::Reviewers => format!(
            "Authorized reviewers: {}\n\
             Configure in .this-is-fine.local.toml (gitignored).\n\
             Hosted source egress requires explicit allow_source_egress.\n\
             \n\
             CLI: {}",
            app.reviewers,
            app.screen.cli_hint()
        ),
        Screen::Adaptation => format!(
            "Local adaptation ranks pressure variants and reviewers\n\
             without network telemetry.\n\
             \n\
             CLI: {}",
            app.screen.cli_hint()
        ),
        Screen::Audit => {
            let mut body = String::from("Audit log (local SQLite + CAS):\n\n");
            if app.recent_runs.is_empty() {
                body.push_str("(empty)\n");
            } else {
                for r in &app.recent_runs {
                    body.push_str(r);
                    body.push('\n');
                }
            }
            body.push_str(&format!("\nCLI: {}", app.screen.cli_hint()));
            body
        }
        Screen::Settings => format!(
            "enabled = {}\n\
             default_fire_level = {}\n\
             audit.tier = {}\n\
             \n\
             Shared: .this-is-fine.toml\n\
             Local:  .this-is-fine.local.toml\n\
             \n\
             CLI: {}",
            app.enabled,
            app.fire_level,
            app.audit_tier,
            app.screen.cli_hint()
        ),
    };

    let para = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(para, area);
}

fn draw_metrics(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
    let text = format!(
        "Fire Level {}\n\
         {}\n\
         \n\
         {}\n\
         \n\
         Press 1-8 or ↑↓\n\
         r refresh · q quit",
        app.fire_level, app.compact, app.metrics_line
    );
    let para = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title("metrics"));
    f.render_widget(para, area);
}

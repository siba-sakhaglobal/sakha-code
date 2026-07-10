//! sakha-tui: terminal UI for the Sakha Coding Agent.
//!
//! Public API skeleton — see spec `modules/13-ui-cli-tui-web-desktop.md`
//! "TUI Screens" and `crates/crate-work-breakdown.md`.

mod app;
mod data;
mod event;
mod views;

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{self as ct_event, Event as CtEvent, KeyCode as CtKeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Tabs};
use ratatui::{Frame, Terminal};

use app::TuiApp;
use data::{DataSource, MemoryDataSource};
use event::{AppEvent, KeyCode, KeyEvent};
use sakha_core::SessionId;
use sakha_memory::{Database, SessionStore, SqliteSessionStore};
use views::View;

const TICK_RATE: Duration = Duration::from_millis(250);

fn main() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Resolve the session to observe. In real usage this comes from
    // `sakha chat`/`sakha run` handing off a session id (e.g. via env var
    // or CLI flag wired at a higher layer); default to a fresh, empty
    // session id so the TUI always has something well-defined to refresh
    // against even with no prior session on disk.
    let session_id = std::env::var("SAKHA_SESSION_ID")
        .ok()
        .and_then(|s| s.parse::<SessionId>().ok())
        .unwrap_or_else(SessionId::new);

    let db_path = std::env::var("SAKHA_MEMORY_DB").unwrap_or_else(|_| "sakha-memory.sqlite3".to_string());
    let session_store: Arc<dyn SessionStore> = match Database::open(&db_path) {
        Ok(db) => Arc::new(SqliteSessionStore::new(Arc::new(db))),
        Err(err) => {
            tracing::warn!(error = %err, "falling back to in-memory session store");
            Arc::new(sakha_memory::InMemorySessionStore::new())
        }
    };
    let data_source = MemoryDataSource::new(session_store);

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;

    // A real TTY is required for the interactive render loop. When one is
    // not available (headless CI, piped output) fall back to a single
    // non-interactive refresh-and-exit so the binary still does useful work
    // and stays testable without a pty.
    if !is_probably_a_tty() {
        let mut app = TuiApp::new();
        let snapshot = runtime.block_on(data_source.refresh(session_id))?;
        app.on_event(AppEvent::Refreshed(snapshot));
        println!(
            "sakha-tui: non-interactive mode (no TTY). session={} turns={} tools={} loops={}",
            session_id,
            app.session_view.turns.len(),
            app.tools_view.calls.len(),
            app.loops_view.loops.len()
        );
        return Ok(());
    }

    run_terminal_loop(session_id, &data_source, &runtime)
}

fn is_probably_a_tty() -> bool {
    use std::io::IsTerminal;
    io::stdout().is_terminal()
}

fn run_terminal_loop(session_id: SessionId, data_source: &MemoryDataSource, runtime: &tokio::runtime::Runtime) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = TuiApp::new();
    let initial = runtime.block_on(data_source.refresh(session_id))?;
    app.on_event(AppEvent::Refreshed(initial));

    let result = event_loop(&mut terminal, &mut app, session_id, data_source, runtime);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut TuiApp,
    session_id: SessionId,
    data_source: &MemoryDataSource,
    runtime: &tokio::runtime::Runtime,
) -> anyhow::Result<()> {
    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|frame| render(frame, app))?;

        let timeout = TICK_RATE.saturating_sub(last_tick.elapsed());
        if ct_event::poll(timeout)? {
            match ct_event::read()? {
                CtEvent::Key(key) if key.kind == KeyEventKind::Press => {
                    if let Some(mapped) = map_key(key) {
                        app.on_event(AppEvent::Key(mapped));
                    }
                }
                CtEvent::Resize(width, height) => app.on_event(AppEvent::Resize { width, height }),
                _ => {}
            }
        }

        if last_tick.elapsed() >= TICK_RATE {
            app.on_event(AppEvent::Tick);
            last_tick = Instant::now();
        }

        if app.refresh_requested {
            let snapshot = runtime.block_on(data_source.refresh(session_id))?;
            app.on_event(AppEvent::Refreshed(snapshot));
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

fn map_key(key: crossterm::event::KeyEvent) -> Option<KeyEvent> {
    let ctrl = key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL);
    let code = match key.code {
        CtKeyCode::Char(c) => KeyCode::Char(c),
        CtKeyCode::Enter => KeyCode::Enter,
        CtKeyCode::Esc => KeyCode::Esc,
        CtKeyCode::Up => KeyCode::Up,
        CtKeyCode::Down => KeyCode::Down,
        CtKeyCode::Left => KeyCode::Left,
        CtKeyCode::Right => KeyCode::Right,
        CtKeyCode::Tab => KeyCode::Tab,
        CtKeyCode::Backspace => KeyCode::Backspace,
        _ => return None,
    };
    Some(KeyEvent { code, ctrl })
}

fn render(frame: &mut Frame, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    render_tabs(frame, app, chunks[0]);

    match app.active_view {
        View::Session => app.session_view.render(frame, chunks[1]),
        View::Tools => app.tools_view.render(frame, chunks[1]),
        View::Loops => app.loops_view.render(frame, chunks[1]),
        View::Diff | View::Provider | View::Memory | View::Research | View::Compression => {
            render_placeholder(frame, app.active_view, chunks[1])
        }
    }

    render_status_bar(frame, chunks[2]);
}

/// Renders a minimal, always-correct placeholder body for screens whose
/// data-backed view has not landed yet (see spec `modules/13-ui-cli-tui-web-desktop.md`
/// "TUI Screens"). Keeps the view reachable/testable via the tab bar and
/// keymap now, rather than the screen simply not existing.
fn render_placeholder(frame: &mut Frame, view: View, area: Rect) {
    let block = Block::default().title(view.label()).borders(Borders::ALL);
    let body = Paragraph::new(format!("{} view: no data source wired yet.", view.label())).block(block);
    frame.render_widget(body, area);
}

fn render_tabs(frame: &mut Frame, app: &TuiApp, area: Rect) {
    let titles: Vec<Line> = [
        "1:Session",
        "2:Tools",
        "3:Loops",
        "4:Diff",
        "5:Provider",
        "6:Memory",
        "7:Research",
        "8:Compression",
    ]
    .iter()
    .map(|t| Line::from(*t))
    .collect();
    let selected = match app.active_view {
        View::Session => 0,
        View::Tools => 1,
        View::Loops => 2,
        View::Diff => 3,
        View::Provider => 4,
        View::Memory => 5,
        View::Research => 6,
        View::Compression => 7,
    };
    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("sakha-tui"))
        .select(selected)
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    frame.render_widget(tabs, area);
}

fn render_status_bar(frame: &mut Frame, area: Rect) {
    let bar = Paragraph::new(Line::from(vec![Span::raw(
        "q: quit  tab/←→: switch view  1/2/3: jump  j/k or ↑/↓: navigate  r: refresh",
    )]));
    frame.render_widget(bar, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use event::{AppEvent, KeyCode as EvKeyCode, KeyEvent as EvKeyEvent};

    #[test]
    fn quit_key_sets_should_quit() {
        let mut app = TuiApp::new();
        app.on_event(AppEvent::Key(EvKeyEvent { code: EvKeyCode::Char('q'), ctrl: false }));
        assert!(app.should_quit);
    }

    #[test]
    fn tick_event_does_not_quit() {
        let mut app = TuiApp::new();
        app.on_event(AppEvent::Tick);
        assert!(!app.should_quit);
    }

    #[test]
    fn map_key_translates_plain_char() {
        let key = crossterm::event::KeyEvent::new(CtKeyCode::Char('q'), crossterm::event::KeyModifiers::NONE);
        let mapped = map_key(key).unwrap();
        assert_eq!(mapped.code, KeyCode::Char('q'));
        assert!(!mapped.ctrl);
    }

    #[test]
    fn map_key_translates_ctrl_modifier() {
        let key = crossterm::event::KeyEvent::new(CtKeyCode::Char('c'), crossterm::event::KeyModifiers::CONTROL);
        let mapped = map_key(key).unwrap();
        assert!(mapped.ctrl);
    }

    #[test]
    fn map_key_returns_none_for_unmapped_keys() {
        let key = crossterm::event::KeyEvent::new(CtKeyCode::F(1), crossterm::event::KeyModifiers::NONE);
        assert!(map_key(key).is_none());
    }
}

use std::{
    io,
    path::Path,
    sync::mpsc::{self, Receiver},
    time::Duration,
};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::{
    app::{AppAction, DashboardApp},
    runtime::load_dashboard_model,
    RenderError,
};

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen, cursor::Show);
    }
}

pub fn run_dashboard(journal_path: &Path) -> Result<(), RenderError> {
    let _guard =
        TerminalGuard::enter().map_err(|error| RenderError::Terminal(error.to_string()))?;
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal =
        Terminal::new(backend).map_err(|error| RenderError::Terminal(error.to_string()))?;
    let mut app = DashboardApp::new(load_dashboard_model(journal_path)?);
    let (_watcher, rx) = start_journal_watcher(journal_path)
        .map_err(|error| RenderError::Terminal(error.to_string()))?;

    loop {
        draw(&mut terminal, &app)?;

        while let Ok(()) = rx.try_recv() {
            app.replace_model(load_dashboard_model(journal_path)?);
            app.apply(AppAction::Refresh);
        }

        if event::poll(Duration::from_millis(250))
            .map_err(|error| RenderError::Terminal(error.to_string()))?
        {
            if let Some(action) = read_action()? {
                app.apply(action);
                if matches!(action, AppAction::Refresh) {
                    app.replace_model(load_dashboard_model(journal_path)?);
                }
            }
        }

        if app.state.should_quit {
            break;
        }
    }

    Ok(())
}

fn draw(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &DashboardApp,
) -> Result<(), RenderError> {
    terminal
        .draw(|frame| {
            let area = frame.area();
            crate::render_dashboard_buffer(app, None, area, frame.buffer_mut());
        })
        .map_err(|error| RenderError::Terminal(error.to_string()))?;
    Ok(())
}

fn read_action() -> Result<Option<AppAction>, RenderError> {
    let event = event::read().map_err(|error| RenderError::Terminal(error.to_string()))?;
    let Event::Key(key) = event else {
        return Ok(None);
    };
    if key.kind != KeyEventKind::Press {
        return Ok(None);
    }
    let action = match key.code {
        KeyCode::Up => Some(AppAction::Up),
        KeyCode::Down => Some(AppAction::Down),
        KeyCode::Left => Some(AppAction::Left),
        KeyCode::Right => Some(AppAction::Right),
        KeyCode::Char('q') => Some(AppAction::Quit),
        _ => None,
    };
    Ok(action)
}

fn start_journal_watcher(
    journal_path: &Path,
) -> notify::Result<(RecommendedWatcher, Receiver<()>)> {
    let watch_root = journal_path.parent().unwrap_or_else(|| Path::new("."));
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        if let Ok(event) = result {
            if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                let _ = tx.send(());
            }
        }
    })?;
    watcher.watch(watch_root, RecursiveMode::Recursive)?;
    Ok((watcher, rx))
}

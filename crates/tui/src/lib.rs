//! Ratatui-based visual dashboard for commit graph, status, and diff viewer.

use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::stdout;
use std::path::Path;
use std::time::Duration;
use thiserror::Error;

/// Errors arising from TUI rendering and terminal interaction.
#[derive(Debug, Error)]
pub enum TuiError {
    /// Terminal initialization error.
    #[error("terminal error: {0}")]
    Terminal(String),

    /// Standard I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Underlying core error.
    #[error("core error: {0}")]
    Core(#[from] oxidize_core::CoreError),
}

pub mod app;
pub mod ui;

pub use app::{App, CommitItem, TabMode};

/// Runs the interactive terminal UI dashboard.
pub fn run_tui(git_dir: &Path) -> Result<(), TuiError> {
    let mut app = App::new();
    app.load_repository(git_dir)?;

    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let res = run_loop(&mut terminal, &mut app);

    // Always restore terminal state
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    res
}

fn run_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<(), TuiError> {
    loop {
        terminal.draw(|f| ui::render(f, app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        app.should_quit = true;
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        app.next();
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        app.previous();
                    }
                    KeyCode::Tab => {
                        app.toggle_tab();
                    }
                    _ => {}
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

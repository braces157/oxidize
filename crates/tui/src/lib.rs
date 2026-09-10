//! Ratatui-based visual dashboard for commit graph, status, and diff viewer (LazyOx).

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
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
pub mod model;
pub mod ops;
pub mod ui;

pub use app::App;
pub use model::{
    ActiveModal, BranchItem, CommitItem, DiffLine, DiffLineKind, DiffView, FileItem,
    FileStatusKind, Panel, StashItem, TabMode,
};

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
                // Ignore key release events if terminal sends them
                if key.kind == crossterm::event::KeyEventKind::Release {
                    continue;
                }

                // If modal is open, forward keyboard events to modal
                if app.active_modal != ActiveModal::None {
                    match key.code {
                        KeyCode::Esc => {
                            app.close_modal();
                        }
                        KeyCode::Enter => {
                            let _ = app.submit_modal();
                        }
                        KeyCode::Backspace => {
                            app.handle_modal_backspace();
                        }
                        KeyCode::Left => {
                            app.handle_modal_left();
                        }
                        KeyCode::Right => {
                            app.handle_modal_right();
                        }
                        KeyCode::Char(c) => {
                            app.handle_modal_char(c);
                        }
                        _ => {}
                    }
                    continue;
                }

                match key.code {
                    // Quit
                    KeyCode::Char('q') | KeyCode::Esc => {
                        app.should_quit = true;
                    }

                    // Direct panel selection 1-4
                    KeyCode::Char('1') => {
                        app.select_panel(Panel::Files);
                    }
                    KeyCode::Char('2') => {
                        app.select_panel(Panel::Branches);
                    }
                    KeyCode::Char('3') => {
                        app.select_panel(Panel::Commits);
                    }
                    KeyCode::Char('4') => {
                        app.select_panel(Panel::Stash);
                    }

                    // Panel cycling
                    KeyCode::Tab | KeyCode::Char(']') => {
                        app.next_panel();
                    }
                    KeyCode::BackTab | KeyCode::Char('[') => {
                        app.prev_panel();
                    }

                    // Item navigation within focused panel
                    KeyCode::Char('j') | KeyCode::Down => {
                        app.next_item();
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        app.prev_item();
                    }

                    // Inspector scrolling
                    KeyCode::PageDown | KeyCode::Char('J') => {
                        app.scroll_inspector_down(5);
                    }
                    KeyCode::PageUp | KeyCode::Char('K') => {
                        app.scroll_inspector_up(5);
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.scroll_inspector_down(10);
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.scroll_inspector_up(10);
                    }

                    // Selection actions
                    KeyCode::Char(' ') => match app.active_panel {
                        Panel::Files => {
                            let _ = app.toggle_stage_selected();
                        }
                        Panel::Branches => {
                            let _ = app.checkout_selected_branch();
                        }
                        Panel::Stash => {
                            let _ = app.pop_selected_stash();
                        }
                        _ => {}
                    },
                    KeyCode::Enter => match app.active_panel {
                        Panel::Branches => {
                            let _ = app.checkout_selected_branch();
                        }
                        Panel::Stash => {
                            let _ = app.pop_selected_stash();
                        }
                        _ => {}
                    },

                    // Commit staged changes
                    KeyCode::Char('c') => {
                        if app.active_panel == Panel::Files {
                            app.open_commit_modal();
                        }
                    }

                    // Stage / Unstage all
                    KeyCode::Char('a') => {
                        if app.active_panel == Panel::Files {
                            let _ = app.stage_all();
                        }
                    }

                    // Create new branch
                    KeyCode::Char('n') => {
                        if app.active_panel == Panel::Branches {
                            app.open_create_branch_modal();
                        }
                    }

                    // Discard / Delete / Drop
                    KeyCode::Char('d') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        match app.active_panel {
                            Panel::Files => {
                                let _ = app.discard_selected_file();
                            }
                            Panel::Branches => {
                                let _ = app.delete_selected_branch();
                            }
                            Panel::Stash => {
                                let _ = app.drop_selected_stash();
                            }
                            _ => {}
                        }
                    }

                    // Keybindings Help Cheatsheet
                    KeyCode::Char('?') => {
                        app.active_modal = ActiveModal::Help;
                    }

                    // Refresh
                    KeyCode::Char('r') => {
                        let _ = app.refresh();
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

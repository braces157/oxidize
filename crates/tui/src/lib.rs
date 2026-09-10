//! Ratatui-based visual dashboard for commit graph, status, and diff viewer (LazyOx).

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseButton,
    MouseEventKind,
};
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

    /// Ref error.
    #[error("ref error: {0}")]
    Ref(#[from] oxidize_refs::RefError),

    /// Index error.
    #[error("index error: {0}")]
    Index(#[from] oxidize_index::IndexError),

    /// Pack error.
    #[error("pack error: {0}")]
    Pack(#[from] oxidize_pack::PackError),

    /// Transport error.
    #[error("transport error: {0}")]
    Transport(#[from] oxidize_transport::TransportError),
}

/// RAII guard managing terminal configuration (raw mode, alternate screen, mouse capture, cursor).
/// Ensures that every cleanup action is attempted unconditionally upon drop, panic, error, or partial setup.
#[derive(Debug, Default)]
pub struct TerminalSessionGuard {
    pub raw_mode_enabled: bool,
    pub alt_screen_active: bool,
    pub mouse_capture_active: bool,
    pub cursor_hidden: bool,
}

impl TerminalSessionGuard {
    /// Creates a new guard with no active states.
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquires terminal features step-by-step.
    /// If an intermediate step fails, previously applied states are rolled back via Drop.
    pub fn acquire() -> Result<Self, TuiError> {
        let mut guard = Self::new();

        enable_raw_mode()
            .map_err(|e| TuiError::Terminal(format!("failed to enable raw mode: {}", e)))?;
        guard.raw_mode_enabled = true;

        let mut out = stdout();
        execute!(out, EnterAlternateScreen)
            .map_err(|e| TuiError::Terminal(format!("failed to enter alternate screen: {}", e)))?;
        guard.alt_screen_active = true;

        execute!(out, EnableMouseCapture)
            .map_err(|e| TuiError::Terminal(format!("failed to enable mouse capture: {}", e)))?;
        guard.mouse_capture_active = true;

        execute!(out, crossterm::cursor::Hide)
            .map_err(|e| TuiError::Terminal(format!("failed to hide cursor: {}", e)))?;
        guard.cursor_hidden = true;

        Ok(guard)
    }

    /// Unconditionally attempts all cleanup steps independently.
    /// Failure in one step does not prevent remaining cleanup steps from executing.
    pub fn restore(&mut self) {
        let mut out = stdout();

        if self.cursor_hidden {
            let _ = execute!(out, crossterm::cursor::Show);
            self.cursor_hidden = false;
        }

        if self.mouse_capture_active {
            let _ = execute!(out, DisableMouseCapture);
            self.mouse_capture_active = false;
        }

        if self.alt_screen_active {
            let _ = execute!(out, LeaveAlternateScreen);
            self.alt_screen_active = false;
        }

        if self.raw_mode_enabled {
            let _ = disable_raw_mode();
            self.raw_mode_enabled = false;
        }
    }
}

impl Drop for TerminalSessionGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

pub mod app;
pub mod model;
pub mod ops;
pub mod ui;

pub use app::{App, BackgroundJob, BackgroundJobResult};
pub use model::{
    ActiveModal, BranchItem, BranchesTab, CommitItem, CommitsTab, DiffLine, DiffLineKind, DiffView,
    FileItem, FileStatusKind, FocusedWindow, Panel, ReflogItem, RemoteItem, StashItem, TabMode,
    TagItem,
};

/// Runs the interactive terminal UI dashboard.
pub fn run_tui(git_dir: &Path) -> Result<(), TuiError> {
    let mut app = App::new();
    app.load_repository(git_dir)?;

    let mut guard = TerminalSessionGuard::acquire()?;

    let out = stdout();
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    // Run event loop with unwind safety so any panic triggers terminal cleanup before propagating
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_loop(&mut terminal, &mut app)
    }));

    // Restore terminal before checking result or re-raising panic
    guard.restore();

    match result {
        Ok(res) => res,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn run_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<(), TuiError> {
    loop {
        if app.should_quit {
            break;
        }

        app.tick()?;

        terminal.draw(|f| ui::render(f, app))?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
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
                                if app.active_modal == ActiveModal::Help && (c == 'q' || c == '?') {
                                    app.close_modal();
                                } else {
                                    app.handle_modal_char(c);
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // If inspector window is focused, route navigation keys directly to inspector scrolling
                    if app.focused_window == FocusedWindow::Inspector {
                        match key.code {
                            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                                app.focus_sidebar();
                            }
                            KeyCode::Char('q') => {
                                app.should_quit = true;
                            }
                            KeyCode::Char('j') | KeyCode::Down => {
                                app.scroll_inspector_down(1);
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                app.scroll_inspector_up(1);
                            }
                            KeyCode::PageDown | KeyCode::Char(' ') | KeyCode::Char('J') => {
                                app.scroll_inspector_down(15);
                            }
                            KeyCode::PageUp | KeyCode::Char('K') => {
                                app.scroll_inspector_up(15);
                            }
                            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.scroll_inspector_down(15);
                            }
                            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.scroll_inspector_up(15);
                            }
                            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.scroll_inspector_down(25);
                            }
                            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.scroll_inspector_up(25);
                            }
                            KeyCode::Char('g') | KeyCode::Home => {
                                app.scroll_inspector_top();
                            }
                            KeyCode::Char('G') | KeyCode::End => {
                                app.scroll_inspector_bottom();
                            }
                            KeyCode::Char('?') => {
                                app.active_modal = ActiveModal::Help;
                            }
                            KeyCode::Tab => {
                                app.focus_sidebar();
                                app.next_panel();
                            }
                            KeyCode::BackTab => {
                                app.focus_sidebar();
                                app.prev_panel();
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // FocusedWindow is Sidebar:
                    match key.code {
                        // Quit
                        KeyCode::Esc | KeyCode::Char('q') => {
                            app.should_quit = true;
                        }

                        // Direct panel selection 1-5 (LazyGit layout: 1: Status, 2: Files, 3: Branches, 4: Commits, 5: Stash)
                        KeyCode::Char('1') => {
                            app.focus_sidebar();
                            app.select_panel(Panel::Status);
                        }
                        KeyCode::Char('2') => {
                            app.focus_sidebar();
                            app.select_panel(Panel::Files);
                        }
                        KeyCode::Char('3') => {
                            app.focus_sidebar();
                            app.select_panel(Panel::Branches);
                        }
                        KeyCode::Char('4') => {
                            app.focus_sidebar();
                            app.select_panel(Panel::Commits);
                        }
                        KeyCode::Char('5') => {
                            app.focus_sidebar();
                            app.select_panel(Panel::Stash);
                        }

                        // Window navigation: Vim h/l and Left/Right
                        KeyCode::Char('h') | KeyCode::Left => {
                            app.focus_sidebar();
                        }
                        KeyCode::Char('l') | KeyCode::Right => {
                            app.focus_inspector();
                        }

                        // Panel cycling: Tab / BackTab
                        KeyCode::Tab => {
                            app.next_panel();
                        }
                        KeyCode::BackTab => {
                            app.prev_panel();
                        }

                        // Sub-tab switching: [ and ]
                        KeyCode::Char(']') => {
                            app.next_tab();
                        }
                        KeyCode::Char('[') => {
                            app.prev_tab();
                        }

                        // Direct inspector scrolling with Ctrl/Alt modifier from sidebar
                        KeyCode::Down
                            if key.modifiers.contains(KeyModifiers::CONTROL)
                                || key.modifiers.contains(KeyModifiers::ALT) =>
                        {
                            app.scroll_inspector_down(3);
                        }
                        KeyCode::Up
                            if key.modifiers.contains(KeyModifiers::CONTROL)
                                || key.modifiers.contains(KeyModifiers::ALT) =>
                        {
                            app.scroll_inspector_up(3);
                        }

                        // Item navigation within focused sidebar panel
                        KeyCode::Char('j') | KeyCode::Down => {
                            app.next_item();
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            app.prev_item();
                        }

                        // Inspector scrolling directly from sidebar
                        KeyCode::PageDown => {
                            app.scroll_inspector_down(15);
                        }
                        KeyCode::PageUp => {
                            app.scroll_inspector_up(15);
                        }
                        KeyCode::Char('J') => {
                            app.scroll_inspector_down(5);
                        }
                        KeyCode::Char('K') => {
                            app.scroll_inspector_up(5);
                        }
                        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.scroll_inspector_down(15);
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.scroll_inspector_up(15);
                        }
                        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.scroll_inspector_down(25);
                        }
                        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.scroll_inspector_up(25);
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
                            Panel::Files => {
                                app.focus_inspector();
                            }
                            Panel::Branches => {
                                if app.branches_tab == BranchesTab::Local {
                                    let _ = app.checkout_selected_branch();
                                } else {
                                    app.focus_inspector();
                                }
                            }
                            Panel::Commits => {
                                app.focus_inspector();
                            }
                            Panel::Stash => {
                                let _ = app.pop_selected_stash();
                            }
                            Panel::Status => {
                                app.focus_inspector();
                            }
                        },

                        // Commit staged changes
                        KeyCode::Char('c') if app.active_panel == Panel::Files => {
                            app.open_commit_modal();
                        }

                        // Amend commit
                        KeyCode::Char('A')
                            if app.active_panel == Panel::Files
                                || app.active_panel == Panel::Commits =>
                        {
                            app.open_amend_modal();
                        }

                        // Stash save modal
                        KeyCode::Char('s')
                            if app.active_panel == Panel::Files
                                || app.active_panel == Panel::Stash =>
                        {
                            app.open_stash_save_modal();
                        }

                        // Stage / Unstage all (in Files) OR Stash apply (in Stash)
                        KeyCode::Char('a') => match app.active_panel {
                            Panel::Files => {
                                let _ = app.stage_all();
                            }
                            Panel::Stash => {
                                let _ = app.apply_selected_stash();
                            }
                            _ => {}
                        },

                        // Create new branch
                        KeyCode::Char('n') if app.active_panel == Panel::Branches => {
                            app.open_create_branch_modal();
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

                        // Push commits to remote (LazyGit convention: 'P')
                        KeyCode::Char('P') => {
                            let _ = app.push();
                        }

                        // Pull commits from remote (LazyGit convention: 'p')
                        KeyCode::Char('p') => {
                            let _ = app.pull();
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
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        app.scroll_inspector_down(3);
                    }
                    MouseEventKind::ScrollUp => {
                        app.scroll_inspector_up(3);
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        let size = terminal.size().unwrap_or_default();
                        let sidebar_width = size.width.saturating_mul(40) / 100;
                        if mouse.column >= sidebar_width {
                            app.focus_inspector();
                        } else {
                            app.focus_sidebar();
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

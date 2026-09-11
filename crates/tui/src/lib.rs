//! Ratatui-based visual dashboard for commit graph, status, and diff viewer (LazyOx).

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers,
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
pub mod mouse;
pub mod ops;
pub mod sequencer;
pub mod syntax;
pub mod ui;

pub use app::{App, BackgroundJob, BackgroundJobResult};
pub use model::{
    ActiveModal, BranchItem, BranchesTab, CommitDecoration, CommitItem, CommitsTab, ConfirmAction,
    DiffLine, DiffLineKind, DiffView, FileItem, FileStatusKind, FocusedWindow, Panel, ReflogItem,
    RemoteItem, ResetMode, StashItem, TabMode, TagItem,
};
pub use mouse::MouseState;
pub use sequencer::{RebaseAction, RebaseTodoItem, SequencerState, SequencerStatus};
pub use syntax::{Language, SyntaxHighlighter};

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
    let mut mouse_state = MouseState::new();

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
                        match &app.active_modal {
                            ActiveModal::Confirm { .. } => match key.code {
                                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                                    if let Err(e) = app.submit_modal() {
                                        app.set_error(e);
                                    }
                                }
                                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                    app.close_modal();
                                }
                                _ => {}
                            },
                            ActiveModal::Help => match key.code {
                                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                                    app.close_modal();
                                }
                                _ => {}
                            },
                            ActiveModal::RebaseTodo { .. } => match key.code {
                                KeyCode::Esc => {
                                    app.close_modal();
                                }
                                KeyCode::Enter => {
                                    if let Err(e) = app.submit_modal() {
                                        app.set_error(e);
                                    }
                                }
                                _ => {
                                    app.handle_rebase_todo_key(key.code, key.modifiers);
                                }
                            },
                            ActiveModal::CustomPatchMenu { .. } => match key.code {
                                KeyCode::Esc | KeyCode::Char('q') => {
                                    app.close_modal();
                                }
                                KeyCode::Enter => {
                                    if let Err(e) = app.submit_modal() {
                                        app.set_error(e);
                                    }
                                }
                                KeyCode::Char(c) if ('1'..='6').contains(&c) => {
                                    if let ActiveModal::CustomPatchMenu { ref mut selected } =
                                        app.active_modal
                                    {
                                        *selected = (c as usize) - ('1' as usize);
                                    }
                                    if let Err(e) = app.submit_modal() {
                                        app.set_error(e);
                                    }
                                }
                                _ => {
                                    app.handle_custom_patch_menu_key(key.code);
                                }
                            },
                            ActiveModal::WorktreeList { .. } => match key.code {
                                KeyCode::Esc | KeyCode::Char('q') => {
                                    app.close_modal();
                                }
                                KeyCode::Enter => {
                                    if let Err(e) = app.submit_modal() {
                                        app.set_error(e);
                                    }
                                }
                                _ => {
                                    app.handle_worktree_list_key(key.code);
                                }
                            },
                            ActiveModal::WorktreeAdd {
                                ref focused_field, ..
                            } => match key.code {
                                KeyCode::Esc => {
                                    app.close_modal();
                                }
                                KeyCode::Enter => {
                                    if let Err(e) = app.submit_modal() {
                                        app.set_error(e);
                                    }
                                }
                                KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                                    app.handle_worktree_add_key(key.code);
                                }
                                KeyCode::Char(' ') if *focused_field == 2 => {
                                    app.handle_worktree_add_key(key.code);
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
                            },
                            ActiveModal::StashSave {
                                ref focused_field, ..
                            } => match key.code {
                                KeyCode::Esc => {
                                    app.close_modal();
                                }
                                KeyCode::Enter => {
                                    if let Err(e) = app.submit_modal() {
                                        app.set_error(e);
                                    }
                                }
                                KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                                    app.handle_stash_save_key(key.code);
                                }
                                KeyCode::Char(' ') if *focused_field != 0 => {
                                    app.handle_stash_save_key(key.code);
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
                            },
                            ActiveModal::RemoteAdd { .. } => match key.code {
                                KeyCode::Esc => {
                                    app.close_modal();
                                }
                                KeyCode::Enter => {
                                    if let Err(e) = app.submit_modal() {
                                        app.set_error(e);
                                    }
                                }
                                KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                                    app.handle_remote_add_key(key.code);
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
                            },
                            ActiveModal::SubmoduleList { .. } => match key.code {
                                KeyCode::Esc | KeyCode::Char('q') => {
                                    app.close_modal();
                                }
                                KeyCode::Enter => {
                                    if let Err(e) = app.submit_modal() {
                                        app.set_error(e);
                                    }
                                }
                                _ => {
                                    app.handle_submodule_list_key(key.code);
                                }
                            },
                            ActiveModal::BisectMenu { .. } => match key.code {
                                KeyCode::Esc | KeyCode::Char('q') => {
                                    app.close_modal();
                                }
                                KeyCode::Enter => {
                                    if let Err(e) = app.submit_modal() {
                                        app.set_error(e);
                                    }
                                }
                                KeyCode::Char(c @ '1'..='4') => {
                                    if let ActiveModal::BisectMenu {
                                        ref mut selected, ..
                                    } = app.active_modal
                                    {
                                        *selected = (c as usize) - ('1' as usize);
                                    }
                                    if let Err(e) = app.submit_modal() {
                                        app.set_error(e);
                                    }
                                }
                                _ => {
                                    app.handle_bisect_menu_key(key.code);
                                }
                            },
                            ActiveModal::CommandPalette { .. } => match key.code {
                                KeyCode::Esc => {
                                    app.close_modal();
                                }
                                KeyCode::Enter => {
                                    if let Err(e) = app.submit_modal() {
                                        app.set_error(e);
                                    }
                                }
                                KeyCode::Up | KeyCode::Down => {
                                    app.handle_command_palette_key(key.code);
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
                            },
                            ActiveModal::ProviderLinks { .. } => match key.code {
                                KeyCode::Esc | KeyCode::Char('q') => {
                                    app.close_modal();
                                }
                                KeyCode::Enter => {
                                    if let Err(e) = app.submit_modal() {
                                        app.set_error(e);
                                    }
                                }
                                KeyCode::Char(c @ '1'..='4') => {
                                    if let ActiveModal::ProviderLinks {
                                        ref mut selected, ..
                                    } = app.active_modal
                                    {
                                        *selected = (c as usize) - ('1' as usize);
                                    }
                                    if let Err(e) = app.submit_modal() {
                                        app.set_error(e);
                                    }
                                }
                                _ => {
                                    app.handle_provider_links_key(key.code);
                                }
                            },
                            _ => match key.code {
                                KeyCode::Esc => {
                                    app.close_modal();
                                }
                                KeyCode::Enter => {
                                    if let Err(e) = app.submit_modal() {
                                        app.set_error(e);
                                    }
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
                            },
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
                            KeyCode::Char('[') => {
                                if app.has_hunks() {
                                    app.prev_hunk();
                                } else {
                                    app.scroll_inspector_up(15);
                                }
                            }
                            KeyCode::Char(']') => {
                                if app.has_hunks() {
                                    app.next_hunk();
                                } else {
                                    app.scroll_inspector_down(15);
                                }
                            }
                            KeyCode::Char(' ') => {
                                if app.has_hunks() {
                                    if let Err(e) = app.toggle_stage_selected_hunk() {
                                        app.set_error(e);
                                    }
                                } else {
                                    app.scroll_inspector_down(15);
                                }
                            }
                            KeyCode::Char('d')
                                if !key.modifiers.contains(KeyModifiers::CONTROL)
                                    && app.has_hunks() =>
                            {
                                app.prompt_discard_selected_hunk();
                            }
                            KeyCode::Char('a') if app.has_hunks() => {
                                app.toggle_current_hunk_in_patch_basket();
                            }
                            KeyCode::Char('P') | KeyCode::Char('p') => {
                                app.open_custom_patch_menu();
                            }
                            KeyCode::Char('j') | KeyCode::Down => {
                                app.scroll_inspector_down(1);
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                app.scroll_inspector_up(1);
                            }
                            KeyCode::PageDown | KeyCode::Char('J') => {
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
                        // Quit or clear filter
                        KeyCode::Esc => {
                            if app.commit_search_filter.is_some() {
                                app.clear_search_filter();
                            } else {
                                app.should_quit = true;
                            }
                        }
                        KeyCode::Char('q') => {
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
                                if let Err(e) = app.toggle_stage_selected() {
                                    app.set_error(e);
                                }
                            }
                            Panel::Branches if app.branches_tab == BranchesTab::Local => {
                                if let Err(e) = app.checkout_selected_branch() {
                                    app.set_error(e);
                                }
                            }
                            Panel::Commits => {
                                if let Err(e) = app.checkout_selected_commit() {
                                    app.set_error(e);
                                }
                            }
                            Panel::Stash => {
                                if let Err(e) = app.pop_selected_stash() {
                                    app.set_error(e);
                                }
                            }
                            _ => {}
                        },
                        KeyCode::Enter => match app.active_panel {
                            Panel::Files => {
                                app.focus_inspector();
                            }
                            Panel::Branches => {
                                if app.branches_tab == BranchesTab::Local {
                                    if let Err(e) = app.checkout_selected_branch() {
                                        app.set_error(e);
                                    }
                                } else {
                                    app.focus_inspector();
                                }
                            }
                            Panel::Commits => {
                                app.focus_inspector();
                            }
                            Panel::Stash => {
                                if let Err(e) = app.pop_selected_stash() {
                                    app.set_error(e);
                                }
                            }
                            Panel::Status => {
                                app.focus_inspector();
                            }
                        },

                        // Commit staged changes OR Cherry-pick commit
                        KeyCode::Char('c') => match app.active_panel {
                            Panel::Files => app.open_commit_modal(),
                            Panel::Commits => app.prompt_cherry_pick_selected_commit(),
                            _ => {}
                        },

                        // Reset to commit (Mixed / Hard)
                        KeyCode::Char('g') if app.active_panel == Panel::Commits => {
                            app.prompt_reset_selected_commit(crate::model::ResetMode::Mixed);
                        }
                        KeyCode::Char('G') if app.active_panel == Panel::Commits => {
                            app.prompt_reset_selected_commit(crate::model::ResetMode::Hard);
                        }

                        // Rename branch
                        KeyCode::Char('R')
                            if app.active_panel == Panel::Branches
                                && app.branches_tab == BranchesTab::Local =>
                        {
                            app.prompt_rename_selected_branch();
                        }

                        // Fast-forward merge branch OR Fetch all remotes
                        KeyCode::Char('f') => match app.active_panel {
                            Panel::Branches if app.branches_tab == BranchesTab::Remotes => {
                                app.fetch_all();
                            }
                            Panel::Branches if app.branches_tab == BranchesTab::Local => {
                                if let Err(e) = app.fast_forward_selected_branch() {
                                    app.set_error(e);
                                }
                            }
                            Panel::Status => {
                                app.fetch_all();
                            }
                            _ => {}
                        },
                        KeyCode::Char('M') if app.active_panel == Panel::Branches => {
                            if let Err(e) = app.fast_forward_selected_branch() {
                                app.set_error(e);
                            }
                        }

                        // Commit search filter
                        KeyCode::Char('/') if app.active_panel == Panel::Commits => {
                            app.open_search_filter_modal();
                        }

                        // Amend commit
                        KeyCode::Char('A')
                            if app.active_panel == Panel::Files
                                || app.active_panel == Panel::Commits =>
                        {
                            app.open_amend_modal();
                        }

                        // Interactive Rebase and Commit Revert in Commits panel
                        KeyCode::Char('i') if app.active_panel == Panel::Commits => {
                            app.open_rebase_todo_modal();
                        }
                        KeyCode::Char('t') if app.active_panel == Panel::Commits => {
                            app.prompt_revert_selected_commit();
                        }

                        // Conflict resolution shortcuts in Files panel
                        KeyCode::Char('o') if app.active_panel == Panel::Files => {
                            if let Some(f) = app.files.get(app.files_selected) {
                                if f.kind == crate::model::FileStatusKind::Conflicted {
                                    if let Err(e) = app.resolve_selected_file_conflict(
                                        crate::model::ConflictChoice::Ours,
                                    ) {
                                        app.set_error(e);
                                    }
                                }
                            }
                        }
                        KeyCode::Char('t') if app.active_panel == Panel::Files => {
                            if let Some(f) = app.files.get(app.files_selected) {
                                if f.kind == crate::model::FileStatusKind::Conflicted {
                                    if let Err(e) = app.resolve_selected_file_conflict(
                                        crate::model::ConflictChoice::Theirs,
                                    ) {
                                        app.set_error(e);
                                    }
                                }
                            }
                        }
                        KeyCode::Char('b')
                            if app.active_panel == Panel::Files
                                && !key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            if let Some(f) = app.files.get(app.files_selected) {
                                if f.kind == crate::model::FileStatusKind::Conflicted {
                                    if let Err(e) = app.resolve_selected_file_conflict(
                                        crate::model::ConflictChoice::Both,
                                    ) {
                                        app.set_error(e);
                                    }
                                }
                            }
                        }

                        // Rebase continue shortcut
                        KeyCode::Char('m') if app.sequencer_state.is_some() => {
                            if let Err(e) = app.rebase_continue() {
                                app.set_error(e);
                            }
                        }

                        // Stash save modal or Rebase skip
                        KeyCode::Char('s')
                            if app.active_panel == Panel::Files
                                || app.active_panel == Panel::Stash =>
                        {
                            if app.sequencer_state.is_some() && app.active_panel == Panel::Files {
                                if let Err(e) = app.rebase_skip() {
                                    app.set_error(e);
                                }
                            } else {
                                app.open_stash_save_modal();
                            }
                        }

                        // Stage / Unstage all (in Files) OR Rebase Abort OR Stash apply (in Stash) OR Add Remote
                        KeyCode::Char('a') => match app.active_panel {
                            Panel::Files => {
                                if app.sequencer_state.is_some() {
                                    app.prompt_rebase_abort();
                                } else if let Err(e) = app.stage_all() {
                                    app.set_error(e);
                                }
                            }
                            Panel::Stash => {
                                if let Err(e) = app.apply_selected_stash() {
                                    app.set_error(e);
                                }
                            }
                            Panel::Branches if app.branches_tab == BranchesTab::Remotes => {
                                app.open_remote_add_modal();
                            }
                            _ => {}
                        },

                        // Create new branch or tag or add remote
                        KeyCode::Char('n') => match app.active_panel {
                            Panel::Branches if app.branches_tab == BranchesTab::Local => {
                                app.open_create_branch_modal();
                            }
                            Panel::Branches if app.branches_tab == BranchesTab::Tags => {
                                app.prompt_create_tag();
                            }
                            Panel::Branches if app.branches_tab == BranchesTab::Remotes => {
                                app.open_remote_add_modal();
                            }
                            Panel::Commits => {
                                app.prompt_create_tag();
                            }
                            _ => {}
                        },

                        // Discard / Delete / Drop
                        KeyCode::Char('d') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            match app.active_panel {
                                Panel::Files => {
                                    app.prompt_discard_selected_file();
                                }
                                Panel::Branches if app.branches_tab == BranchesTab::Local => {
                                    app.prompt_delete_selected_branch();
                                }
                                Panel::Branches if app.branches_tab == BranchesTab::Tags => {
                                    app.prompt_delete_selected_tag();
                                }
                                Panel::Branches if app.branches_tab == BranchesTab::Remotes => {
                                    if let Some(r) = app.selected_remote() {
                                        if r.name.contains('/') {
                                            app.prompt_delete_selected_remote_branch();
                                        } else {
                                            app.prompt_remove_remote(r.name.clone());
                                        }
                                    }
                                }
                                Panel::Stash => {
                                    app.prompt_drop_selected_stash();
                                }
                                _ => {}
                            }
                        }

                        // Push commits to remote or push selected tag (LazyGit convention: 'P')
                        KeyCode::Char('P') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            match app.active_panel {
                                Panel::Branches if app.branches_tab == BranchesTab::Tags => {
                                    app.push_selected_tag();
                                }
                                _ => {
                                    if let Err(e) = app.push() {
                                        app.set_error(e);
                                    }
                                }
                            }
                        }

                        // Pull commits from remote (LazyGit convention: 'p')
                        KeyCode::Char('p') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if let Err(e) = app.pull() {
                                app.set_error(e);
                            }
                        }

                        // Branch from stash
                        KeyCode::Char('b') if app.active_panel == Panel::Stash => {
                            app.open_stash_branch_modal();
                        }

                        // Worktree list modal
                        KeyCode::Char('w') | KeyCode::Char('W') => match app.active_panel {
                            Panel::Branches | Panel::Status => {
                                app.open_worktree_list_modal();
                            }
                            _ => {}
                        },

                        // Custom patch menu
                        KeyCode::Char('v') => {
                            app.open_custom_patch_menu();
                        }
                        KeyCode::Char('p') | KeyCode::Char('P')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            app.open_command_palette();
                        }

                        // Command Palette modal (':' or 'Ctrl+p')
                        KeyCode::Char(':') => {
                            app.open_command_palette();
                        }

                        // Git Bisect control menu ('B')
                        KeyCode::Char('B') => {
                            app.open_bisect_menu_modal();
                        }

                        // Submodules list modal ('S')
                        KeyCode::Char('S') => {
                            app.open_submodule_list_modal();
                        }

                        // Remote Web Provider links ('O')
                        KeyCode::Char('O') => {
                            app.open_provider_links_modal();
                        }

                        // Force-push with lease ('F')
                        KeyCode::Char('F') => {
                            app.push_force_lease();
                        }

                        // Yank selected item ('y')
                        KeyCode::Char('y') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.yank_selected();
                        }

                        // Return to parent repository if nested ('u')
                        KeyCode::Char('u')
                            if !key.modifiers.contains(KeyModifiers::CONTROL)
                                && !app.repo_history.is_empty() =>
                        {
                            if let Err(e) = app.return_to_parent_repo() {
                                app.set_error(e);
                            }
                        }

                        // Keybindings Help Cheatsheet
                        KeyCode::Char('?') => {
                            app.active_modal = ActiveModal::Help;
                        }

                        // Refresh
                        KeyCode::Char('r') => {
                            if let Err(e) = app.refresh() {
                                app.set_error(e);
                            }
                        }

                        _ => {}
                    }
                }
                Event::Mouse(mouse) => {
                    let size = terminal.size().unwrap_or_default();
                    let screen_area = ratatui::layout::Rect {
                        x: 0,
                        y: 0,
                        width: size.width,
                        height: size.height,
                    };
                    let _ = mouse::handle_mouse_event(app, &mut mouse_state, mouse, screen_area);
                }
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

//! Mouse interaction and event dispatching for the LazyOx TUI dashboard.

use crate::app::App;
use crate::model::{ActiveModal, BranchesTab, CommitsTab, Panel};
use crate::ui::{
    branch_tab_ranges, build_footer, commit_tab_ranges, compute_layout, compute_modal_layout,
    window_range, AppLayout, FooterAction,
};
use crate::TuiError;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::time::{Duration, Instant};

/// Tracks mouse click state to detect single vs double clicks and active drag gestures.
#[derive(Debug, Clone, Copy, Default)]
pub struct MouseState {
    /// Timestamp of the preceding click event.
    pub last_click_time: Option<Instant>,
    /// Column and row coordinates of the preceding click event.
    pub last_click_pos: (u16, u16),
    /// Mouse button pressed in the preceding click event.
    pub last_click_button: Option<MouseButton>,
}

impl MouseState {
    /// Creates a new default `MouseState`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a mouse button click and returns `true` if this click qualifies as a double-click.
    pub fn record_click(&mut self, button: MouseButton, col: u16, row: u16) -> bool {
        let now = Instant::now();
        let is_double = if let (Some(last_time), Some(last_btn)) =
            (self.last_click_time, self.last_click_button)
        {
            last_btn == button
                && self.last_click_pos.1 == row
                && (self.last_click_pos.0 as i32 - col as i32).abs() <= 2
                && now.duration_since(last_time) < Duration::from_millis(400)
        } else {
            false
        };

        if is_double {
            self.last_click_time = None;
            self.last_click_button = None;
        } else {
            self.last_click_time = Some(now);
            self.last_click_pos = (col, row);
            self.last_click_button = Some(button);
        }

        is_double
    }
}

/// Dispatches a crossterm mouse event across active modals, docking panels, header tabs, and footer buttons.
pub fn handle_mouse_event(
    app: &mut App,
    state: &mut MouseState,
    event: MouseEvent,
    screen: Rect,
) -> Result<(), TuiError> {
    let col = event.column;
    let row = event.row;

    // 1. If modal is active, modal takes precedence
    if app.active_modal != ActiveModal::None {
        return handle_modal_mouse(app, state, event, screen);
    }

    let layout = match compute_layout(screen) {
        Some(l) => l,
        None => return Ok(()),
    };

    match event.kind {
        MouseEventKind::ScrollDown => {
            handle_scroll_down(app, &layout, col, row);
        }
        MouseEventKind::ScrollUp => {
            handle_scroll_up(app, &layout, col, row);
        }
        MouseEventKind::Down(MouseButton::Left) => {
            let is_double = state.record_click(MouseButton::Left, col, row);
            handle_left_click(app, &layout, col, row, is_double)?;
        }
        MouseEventKind::Down(MouseButton::Right) => {
            handle_right_click(app, &layout, col, row)?;
        }
        _ => {}
    }

    Ok(())
}

fn handle_modal_mouse(
    app: &mut App,
    state: &mut MouseState,
    event: MouseEvent,
    screen: Rect,
) -> Result<(), TuiError> {
    let modal_layout = match compute_modal_layout(&app.active_modal, screen) {
        Some(ml) => ml,
        None => return Ok(()),
    };

    let col = event.column;
    let row = event.row;

    match event.kind {
        MouseEventKind::Down(MouseButton::Right) => {
            // Right-click dismisses modal
            app.close_modal();
        }
        MouseEventKind::Down(MouseButton::Left) => {
            let _ = state.record_click(MouseButton::Left, col, row);

            // Click outside modal area dismisses modal
            if !rect_contains(modal_layout.area, col, row) {
                app.close_modal();
                return Ok(());
            }

            // Inside Help modal: clicking anywhere closes it
            if app.active_modal == ActiveModal::Help {
                app.close_modal();
                return Ok(());
            }

            // Inside action buttons area (e.g. bottom prompt hints)
            if let Some(act_rect) = modal_layout.action_rect {
                if rect_contains(act_rect, col, row) {
                    let mid_x = act_rect.x + act_rect.width / 2;
                    if col < mid_x {
                        if let Err(e) = app.submit_modal() {
                            app.set_error(e);
                        }
                    } else {
                        app.close_modal();
                    }
                    return Ok(());
                }
            }

            // Inside text input area: position text cursor
            if let Some(input_rect) = modal_layout.input_rect {
                if row == input_rect.y && col >= input_rect.x {
                    let char_pos = (col.saturating_sub(input_rect.x)) as usize;
                    app.set_modal_cursor(char_pos);
                }
            }
        }
        _ => {}
    }

    Ok(())
}

fn handle_left_click(
    app: &mut App,
    layout: &AppLayout,
    col: u16,
    row: u16,
    is_double: bool,
) -> Result<(), TuiError> {
    // 1. Check footer row
    if row == layout.footer.y {
        return handle_footer_click(app, layout.footer, col);
    }

    // 2. Check Inspector
    if rect_contains(layout.inspector, col, row) {
        app.focus_inspector();
        if is_inside_body(layout.inspector, col, row) {
            let row_offset = (row - (layout.inspector.y + 1)) as usize;
            let line_idx = app.inspector_scroll + row_offset;
            if let Some(hunk_idx) = app
                .cached_diff
                .as_ref()
                .and_then(|d| d.hunk_at_line(line_idx))
            {
                if is_double && app.selected_hunk() == Some(hunk_idx) {
                    if let Err(e) = app.toggle_stage_selected_hunk() {
                        app.set_error(e);
                    }
                } else {
                    app.select_hunk(hunk_idx);
                }
            }
        }
        return Ok(());
    }

    // 3. Status Panel
    if rect_contains(layout.status_panel, col, row) {
        app.focus_sidebar();
        app.select_panel(Panel::Status);
        return Ok(());
    }

    // 4. Files Panel
    if rect_contains(layout.files_panel, col, row) {
        app.focus_sidebar();
        app.select_panel(Panel::Files);
        if is_inside_body(layout.files_panel, col, row) {
            let row_offset = (row - (layout.files_panel.y + 1)) as usize;
            let avail_height = (layout.files_panel.height as usize).saturating_sub(2);
            let (start_idx, _) = window_range(app.files.len(), app.files_selected, avail_height);
            let target_idx = start_idx + row_offset;
            if target_idx < app.files.len() {
                if is_double && target_idx == app.files_selected {
                    if let Err(e) = app.toggle_stage_selected() {
                        app.set_error(e);
                    }
                } else {
                    app.select_file_index(target_idx);
                }
            }
        }
        return Ok(());
    }

    // 5. Branches Panel
    if rect_contains(layout.branches_panel, col, row) {
        app.focus_sidebar();
        app.select_panel(Panel::Branches);

        // Check if header tab was clicked
        if row == layout.branches_panel.y {
            for (tab, start_x, end_x) in branch_tab_ranges(app, layout.branches_panel) {
                if col >= start_x && col < end_x {
                    app.branches_tab = tab;
                    app.inspector_scroll = 0;
                    app.update_inspector();
                    return Ok(());
                }
            }
            return Ok(());
        }

        if is_inside_body(layout.branches_panel, col, row) {
            let row_offset = (row - (layout.branches_panel.y + 1)) as usize;
            let avail_height = (layout.branches_panel.height as usize).saturating_sub(2);

            match app.branches_tab {
                BranchesTab::Local => {
                    let local_count = app.branches.iter().filter(|b| !b.is_remote).count();
                    let (start_idx, _) =
                        window_range(local_count, app.branches_selected, avail_height);
                    let target_idx = start_idx + row_offset;
                    if target_idx < local_count {
                        if is_double && target_idx == app.branches_selected {
                            if let Err(e) = app.checkout_selected_branch() {
                                app.set_error(e);
                            }
                        } else {
                            app.select_branch_index(target_idx);
                        }
                    }
                }
                BranchesTab::Remotes => {
                    let (start_idx, _) =
                        window_range(app.remotes.len(), app.remotes_selected, avail_height);
                    let target_idx = start_idx + row_offset;
                    if target_idx < app.remotes.len() {
                        if is_double {
                            app.focus_inspector();
                        } else {
                            app.select_branch_index(target_idx);
                        }
                    }
                }
                BranchesTab::Tags => {
                    let (start_idx, _) =
                        window_range(app.tags.len(), app.tags_selected, avail_height);
                    let target_idx = start_idx + row_offset;
                    if target_idx < app.tags.len() {
                        if is_double {
                            app.focus_inspector();
                        } else {
                            app.select_branch_index(target_idx);
                        }
                    }
                }
            }
        }
        return Ok(());
    }

    // 6. Commits Panel
    if rect_contains(layout.commits_panel, col, row) {
        app.focus_sidebar();
        app.select_panel(Panel::Commits);

        // Check if header tab was clicked
        if row == layout.commits_panel.y {
            for (tab, start_x, end_x) in commit_tab_ranges(app, layout.commits_panel) {
                if col >= start_x && col < end_x {
                    app.commits_tab = tab;
                    app.inspector_scroll = 0;
                    app.update_inspector();
                    return Ok(());
                }
            }
            return Ok(());
        }

        if is_inside_body(layout.commits_panel, col, row) {
            let row_offset = (row - (layout.commits_panel.y + 1)) as usize;
            let avail_height = (layout.commits_panel.height as usize).saturating_sub(2);

            match app.commits_tab {
                CommitsTab::Commits => {
                    let indices = app.filtered_commit_indices();
                    let current_pos = indices
                        .iter()
                        .position(|&i| i == app.commits_selected)
                        .unwrap_or(0);
                    let (start_idx, _) = window_range(indices.len(), current_pos, avail_height);
                    let target_pos = start_idx + row_offset;
                    if target_pos < indices.len() {
                        let actual_idx = indices[target_pos];
                        if is_double {
                            app.focus_inspector();
                        } else {
                            app.select_commit_index(actual_idx);
                        }
                    }
                }
                CommitsTab::Reflog => {
                    let (start_idx, _) =
                        window_range(app.reflog.len(), app.reflog_selected, avail_height);
                    let target_idx = start_idx + row_offset;
                    if target_idx < app.reflog.len() {
                        if is_double {
                            app.focus_inspector();
                        } else {
                            app.select_commit_index(target_idx);
                        }
                    }
                }
            }
        }
        return Ok(());
    }

    // 7. Stash Panel
    if rect_contains(layout.stash_panel, col, row) {
        app.focus_sidebar();
        app.select_panel(Panel::Stash);

        if is_inside_body(layout.stash_panel, col, row) {
            let row_offset = (row - (layout.stash_panel.y + 1)) as usize;
            let avail_height = (layout.stash_panel.height as usize).saturating_sub(2);
            let (start_idx, _) =
                window_range(app.stashes.len(), app.stashes_selected, avail_height);
            let target_idx = start_idx + row_offset;
            if target_idx < app.stashes.len() {
                if is_double && target_idx == app.stashes_selected {
                    if let Err(e) = app.pop_selected_stash() {
                        app.set_error(e);
                    }
                } else {
                    app.select_stash_index(target_idx);
                }
            }
        }
        return Ok(());
    }

    Ok(())
}

fn handle_right_click(
    app: &mut App,
    layout: &AppLayout,
    col: u16,
    row: u16,
) -> Result<(), TuiError> {
    if rect_contains(layout.files_panel, col, row) && is_inside_body(layout.files_panel, col, row) {
        let row_offset = (row - (layout.files_panel.y + 1)) as usize;
        let avail_height = (layout.files_panel.height as usize).saturating_sub(2);
        let (start_idx, _) = window_range(app.files.len(), app.files_selected, avail_height);
        let target_idx = start_idx + row_offset;
        if target_idx < app.files.len() {
            app.focus_sidebar();
            app.select_panel(Panel::Files);
            app.select_file_index(target_idx);
            if let Err(e) = app.toggle_stage_selected() {
                app.set_error(e);
            }
        }
    }
    Ok(())
}

fn handle_footer_click(app: &mut App, footer_rect: Rect, col: u16) -> Result<(), TuiError> {
    let (_, buttons) = build_footer(app, footer_rect.width);
    for (start_x, end_x, action) in buttons {
        if col >= footer_rect.x + start_x && col < footer_rect.x + end_x {
            execute_footer_action(app, action)?;
            break;
        }
    }
    Ok(())
}

fn execute_footer_action(app: &mut App, action: FooterAction) -> Result<(), TuiError> {
    match action {
        FooterAction::Quit => app.should_quit = true,
        FooterAction::Help => app.active_modal = ActiveModal::Help,
        FooterAction::Push => {
            if let Err(e) = app.push() {
                app.set_error(e);
            }
        }
        FooterAction::Pull => {
            if let Err(e) = app.pull() {
                app.set_error(e);
            }
        }
        FooterAction::Refresh => {
            if let Err(e) = app.refresh() {
                app.set_error(e);
            }
        }
        FooterAction::FocusToggle => app.toggle_focus(),
        FooterAction::CyclePanel => app.next_panel(),
        FooterAction::StageToggle => {
            if let Err(e) = app.toggle_stage_selected() {
                app.set_error(e);
            }
        }
        FooterAction::StageAll => {
            if let Err(e) = app.stage_all() {
                app.set_error(e);
            }
        }
        FooterAction::Commit => app.open_commit_modal(),
        FooterAction::Amend => app.open_amend_modal(),
        FooterAction::StashSave => app.open_stash_save_modal(),
        FooterAction::Discard => {
            app.prompt_discard_selected_file();
        }
        FooterAction::ScrollDiff => app.focus_inspector(),
        FooterAction::CheckoutBranch => {
            if app.branches_tab == BranchesTab::Local {
                if let Err(e) = app.checkout_selected_branch() {
                    app.set_error(e);
                }
            }
        }
        FooterAction::NewBranch => app.open_create_branch_modal(),
        FooterAction::DeleteBranch => {
            if app.branches_tab == BranchesTab::Local {
                app.prompt_delete_selected_branch();
            }
        }
        FooterAction::SwitchTab => app.next_tab(),
        FooterAction::PopStash => {
            if let Err(e) = app.pop_selected_stash() {
                app.set_error(e);
            }
        }
        FooterAction::ApplyStash => {
            if let Err(e) = app.apply_selected_stash() {
                app.set_error(e);
            }
        }
        FooterAction::DropStash => {
            app.prompt_drop_selected_stash();
        }
        FooterAction::JumpStatus => {
            app.focus_sidebar();
            app.select_panel(Panel::Status);
        }
        FooterAction::JumpFiles => {
            app.focus_sidebar();
            app.select_panel(Panel::Files);
        }
        FooterAction::JumpBranches => {
            app.focus_sidebar();
            app.select_panel(Panel::Branches);
        }
        FooterAction::JumpCommits => {
            app.focus_sidebar();
            app.select_panel(Panel::Commits);
        }
        FooterAction::JumpStash => {
            app.focus_sidebar();
            app.select_panel(Panel::Stash);
        }
        FooterAction::ReturnToSidebar => app.focus_sidebar(),
        FooterAction::ScrollInspectorTop => app.scroll_inspector_top(),
        FooterAction::ScrollInspectorBottom => app.scroll_inspector_bottom(),
        FooterAction::PageInspectorDown => app.scroll_inspector_down(15),
        FooterAction::PrevHunk => app.prev_hunk(),
        FooterAction::NextHunk => app.next_hunk(),
        FooterAction::StageHunk => {
            if let Err(e) = app.toggle_stage_selected_hunk() {
                app.set_error(e);
            }
        }
        FooterAction::DiscardHunk => app.prompt_discard_selected_hunk(),
        FooterAction::CheckoutCommit => {
            if let Err(e) = app.checkout_selected_commit() {
                app.set_error(e);
            }
        }
        FooterAction::CherryPick => app.prompt_cherry_pick_selected_commit(),
        FooterAction::ResetCommit => {
            app.prompt_reset_selected_commit(crate::model::ResetMode::Mixed);
        }
        FooterAction::RenameBranch => app.prompt_rename_selected_branch(),
        FooterAction::FastForwardMerge => {
            if let Err(e) = app.fast_forward_selected_branch() {
                app.set_error(e);
            }
        }
        FooterAction::CreateTag => app.prompt_create_tag(),
        FooterAction::DeleteTag => app.prompt_delete_selected_tag(),
        FooterAction::SearchFilter => app.open_search_filter_modal(),
        FooterAction::InteractiveRebase => app.open_rebase_todo_modal(),
        FooterAction::RebaseContinue => {
            if let Err(e) = app.rebase_continue() {
                app.set_error(e);
            }
        }
        FooterAction::RebaseSkip => {
            if let Err(e) = app.rebase_skip() {
                app.set_error(e);
            }
        }
        FooterAction::RebaseAbort => app.prompt_rebase_abort(),
        FooterAction::RevertCommit => app.prompt_revert_selected_commit(),
        FooterAction::ResolveOurs => {
            if let Err(e) = app.resolve_selected_file_conflict(crate::model::ConflictChoice::Ours) {
                app.set_error(e);
            }
        }
        FooterAction::ResolveTheirs => {
            if let Err(e) = app.resolve_selected_file_conflict(crate::model::ConflictChoice::Theirs)
            {
                app.set_error(e);
            }
        }
        FooterAction::ResolveBoth => {
            if let Err(e) = app.resolve_selected_file_conflict(crate::model::ConflictChoice::Both) {
                app.set_error(e);
            }
        }
        FooterAction::AddToPatchBasket => app.toggle_current_hunk_in_patch_basket(),
        FooterAction::CustomPatchMenu => app.open_custom_patch_menu(),
        FooterAction::StashBranch => app.open_stash_branch_modal(),
        FooterAction::WorktreeList => app.open_worktree_list_modal(),
    }
    Ok(())
}

fn handle_scroll_down(app: &mut App, layout: &AppLayout, col: u16, row: u16) {
    if rect_contains(layout.inspector, col, row) {
        app.scroll_inspector_down(3);
    } else if rect_contains(layout.files_panel, col, row) {
        app.focus_sidebar();
        app.select_panel(Panel::Files);
        app.next_item();
    } else if rect_contains(layout.branches_panel, col, row) {
        app.focus_sidebar();
        app.select_panel(Panel::Branches);
        app.next_item();
    } else if rect_contains(layout.commits_panel, col, row) {
        app.focus_sidebar();
        app.select_panel(Panel::Commits);
        app.next_item();
    } else if rect_contains(layout.stash_panel, col, row) {
        app.focus_sidebar();
        app.select_panel(Panel::Stash);
        app.next_item();
    } else {
        app.scroll_inspector_down(3);
    }
}

fn handle_scroll_up(app: &mut App, layout: &AppLayout, col: u16, row: u16) {
    if rect_contains(layout.inspector, col, row) {
        app.scroll_inspector_up(3);
    } else if rect_contains(layout.files_panel, col, row) {
        app.focus_sidebar();
        app.select_panel(Panel::Files);
        app.prev_item();
    } else if rect_contains(layout.branches_panel, col, row) {
        app.focus_sidebar();
        app.select_panel(Panel::Branches);
        app.prev_item();
    } else if rect_contains(layout.commits_panel, col, row) {
        app.focus_sidebar();
        app.select_panel(Panel::Commits);
        app.prev_item();
    } else if rect_contains(layout.stash_panel, col, row) {
        app.focus_sidebar();
        app.select_panel(Panel::Stash);
        app.prev_item();
    } else {
        app.scroll_inspector_up(3);
    }
}

fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}

fn is_inside_body(rect: Rect, col: u16, row: u16) -> bool {
    col > rect.x && col + 1 < rect.x + rect.width && row > rect.y && row + 1 < rect.y + rect.height
}

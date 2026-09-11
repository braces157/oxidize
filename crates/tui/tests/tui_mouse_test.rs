//! Comprehensive real-time tests for mouse interactivity in LazyOx TUI.

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use oxidize_tui::model::{ActiveModal, BranchesTab, CommitsTab, FocusedWindow, Panel};
use oxidize_tui::mouse::{self, MouseState};
use oxidize_tui::ui::{self, branch_tab_ranges, commit_tab_ranges, compute_layout};
use oxidize_tui::App;
use ratatui::layout::Rect;
use std::fs;
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;
use tempfile::TempDir;

fn setup_test_repo() -> (TempDir, App) {
    let tmp = TempDir::new().unwrap();
    let repo_dir = tmp.path();

    let init_res = Command::new("git")
        .args(["init", "-b", "master"])
        .current_dir(repo_dir)
        .output()
        .expect("git init failed");
    assert!(init_res.status.success());

    Command::new("git")
        .args(["config", "user.name", "Mouse Tester"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "mouse@test.com"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    fs::write(repo_dir.join("file1.txt"), "hello world\nline 2\n").unwrap();
    fs::write(repo_dir.join("file2.txt"), "first version\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Create second branch
    Command::new("git")
        .args(["branch", "feature-x"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Create second commit
    fs::write(repo_dir.join("file1.txt"), "hello world\nline 2 modified\n").unwrap();
    Command::new("git")
        .args(["commit", "-am", "Second commit"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Make unstaged changes and untracked file
    fs::write(
        repo_dir.join("file1.txt"),
        "hello world\nline 2 modified again\n",
    )
    .unwrap();
    fs::write(repo_dir.join("untracked.txt"), "untracked file contents\n").unwrap();

    let mut app = App::new();
    app.load_repository(&repo_dir.join(".git")).unwrap();
    (tmp, app)
}

fn mouse_event(kind: MouseEventKind, col: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column: col,
        row,
        modifiers: KeyModifiers::empty(),
    }
}

#[test]
fn test_mouse_panel_switching_and_focus() {
    let (_tmp, mut app) = setup_test_repo();
    let mut state = MouseState::new();
    let screen = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 40,
    };
    let layout = compute_layout(screen).expect("valid layout");

    // 1. Click Status panel
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.status_panel.x + 2,
        layout.status_panel.y + 2,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    assert_eq!(app.active_panel, Panel::Status);
    assert_eq!(app.focused_window, FocusedWindow::Sidebar);

    // 2. Click Files panel
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.files_panel.x + 2,
        layout.files_panel.y + 2,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    assert_eq!(app.active_panel, Panel::Files);
    assert_eq!(app.focused_window, FocusedWindow::Sidebar);

    // 3. Click Branches panel
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.branches_panel.x + 2,
        layout.branches_panel.y + 2,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    assert_eq!(app.active_panel, Panel::Branches);
    assert_eq!(app.focused_window, FocusedWindow::Sidebar);

    // 4. Click Commits panel
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.commits_panel.x + 2,
        layout.commits_panel.y + 2,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    assert_eq!(app.active_panel, Panel::Commits);
    assert_eq!(app.focused_window, FocusedWindow::Sidebar);

    // 5. Click Stash panel
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.stash_panel.x + 2,
        layout.stash_panel.y + 2,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    assert_eq!(app.active_panel, Panel::Stash);
    assert_eq!(app.focused_window, FocusedWindow::Sidebar);

    // 6. Click Inspector pane
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.inspector.x + 5,
        layout.inspector.y + 5,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    assert_eq!(app.focused_window, FocusedWindow::Inspector);
}

#[test]
fn test_mouse_item_clicking_in_lists() {
    let (_tmp, mut app) = setup_test_repo();
    let mut state = MouseState::new();
    let screen = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 40,
    };
    let layout = compute_layout(screen).expect("valid layout");

    assert!(app.files.len() >= 2);

    // Click first item (row = files_panel.y + 1)
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.files_panel.x + 4,
        layout.files_panel.y + 1,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    assert_eq!(app.files_selected, 0);

    // Sleep 450ms so next click is single-click, not double-click
    sleep(Duration::from_millis(450));

    // Click second item (row = files_panel.y + 2)
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.files_panel.x + 4,
        layout.files_panel.y + 2,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    assert_eq!(app.files_selected, 1);
    assert!(app.cached_diff.is_some());

    // Switch to Commits and click second commit
    assert!(app.commits.len() >= 2);
    sleep(Duration::from_millis(450));
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.commits_panel.x + 4,
        layout.commits_panel.y + 2,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    assert_eq!(app.active_panel, Panel::Commits);
    assert_eq!(app.commits_selected, 1);
}

#[test]
fn test_mouse_subtab_header_clicking() {
    let (_tmp, mut app) = setup_test_repo();
    let mut state = MouseState::new();
    let screen = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 40,
    };
    let layout = compute_layout(screen).expect("valid layout");

    // Branches subtabs
    assert_eq!(app.branches_tab, BranchesTab::Local);
    let ranges = branch_tab_ranges(&app, layout.branches_panel);
    let remotes_range = ranges
        .iter()
        .find(|(t, _, _)| *t == BranchesTab::Remotes)
        .unwrap();

    // Click Remotes tab in Branches header
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        remotes_range.1 + 1,
        layout.branches_panel.y,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    assert_eq!(app.branches_tab, BranchesTab::Remotes);

    // Click Tags tab in Branches header
    let ranges_after = branch_tab_ranges(&app, layout.branches_panel);
    let tags_range = ranges_after
        .iter()
        .find(|(t, _, _)| *t == BranchesTab::Tags)
        .unwrap();
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        tags_range.1 + 1,
        layout.branches_panel.y,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    assert_eq!(app.branches_tab, BranchesTab::Tags);

    // Click Local tab back
    let ranges_after_tags = branch_tab_ranges(&app, layout.branches_panel);
    let local_range = ranges_after_tags
        .iter()
        .find(|(t, _, _)| *t == BranchesTab::Local)
        .unwrap();
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        local_range.1 + 1,
        layout.branches_panel.y,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    assert_eq!(app.branches_tab, BranchesTab::Local);

    // Commits subtabs
    assert_eq!(app.commits_tab, CommitsTab::Commits);
    let c_ranges = commit_tab_ranges(&app, layout.commits_panel);
    let reflog_range = c_ranges
        .iter()
        .find(|(t, _, _)| *t == CommitsTab::Reflog)
        .unwrap();
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        reflog_range.1 + 1,
        layout.commits_panel.y,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    assert_eq!(app.commits_tab, CommitsTab::Reflog);
}

#[test]
fn test_mouse_double_click_actions() {
    let (_tmp, mut app) = setup_test_repo();
    let mut state = MouseState::new();
    let screen = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 40,
    };
    let layout = compute_layout(screen).expect("valid layout");

    // 1. Double click on Files panel item -> toggles staging
    assert!(!app.files[0].kind.is_staged());
    let click1 = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.files_panel.x + 3,
        layout.files_panel.y + 1,
    );
    let click2 = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.files_panel.x + 3,
        layout.files_panel.y + 1,
    );

    mouse::handle_mouse_event(&mut app, &mut state, click1, screen).unwrap();
    mouse::handle_mouse_event(&mut app, &mut state, click2, screen).unwrap();
    assert!(
        app.files[0].kind.is_staged(),
        "Double click should stage the file"
    );

    // Double click again -> unstages
    let click3 = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.files_panel.x + 3,
        layout.files_panel.y + 1,
    );
    let click4 = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.files_panel.x + 3,
        layout.files_panel.y + 1,
    );
    mouse::handle_mouse_event(&mut app, &mut state, click3, screen).unwrap();
    mouse::handle_mouse_event(&mut app, &mut state, click4, screen).unwrap();
    assert!(
        !app.files[0].kind.is_staged(),
        "Double click again should unstage the file"
    );

    // 2. Double click on a commit item -> focuses inspector
    sleep(Duration::from_millis(450));
    let c_click1 = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.commits_panel.x + 3,
        layout.commits_panel.y + 1,
    );
    let c_click2 = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.commits_panel.x + 3,
        layout.commits_panel.y + 1,
    );
    mouse::handle_mouse_event(&mut app, &mut state, c_click1, screen).unwrap();
    mouse::handle_mouse_event(&mut app, &mut state, c_click2, screen).unwrap();
    assert_eq!(app.focused_window, FocusedWindow::Inspector);
}

#[test]
fn test_mouse_right_click_toggles_staging() {
    let (_tmp, mut app) = setup_test_repo();
    let mut state = MouseState::new();
    let screen = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 40,
    };
    let layout = compute_layout(screen).expect("valid layout");

    assert!(!app.files[0].kind.is_staged());
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Right),
        layout.files_panel.x + 5,
        layout.files_panel.y + 1,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    assert!(
        app.files[0].kind.is_staged(),
        "Right click should stage the file"
    );

    // Right click again -> unstages
    let ev2 = mouse_event(
        MouseEventKind::Down(MouseButton::Right),
        layout.files_panel.x + 5,
        layout.files_panel.y + 1,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev2, screen).unwrap();
    assert!(
        !app.files[0].kind.is_staged(),
        "Right click again should unstage the file"
    );
}

#[test]
fn test_mouse_wheel_scrolling_contextual() {
    let (_tmp, mut app) = setup_test_repo();
    let mut state = MouseState::new();
    let screen = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 40,
    };
    let layout = compute_layout(screen).expect("valid layout");

    // 1. Wheel over Files panel
    assert_eq!(app.files_selected, 0);
    let scroll_down = mouse_event(
        MouseEventKind::ScrollDown,
        layout.files_panel.x + 5,
        layout.files_panel.y + 3,
    );
    mouse::handle_mouse_event(&mut app, &mut state, scroll_down, screen).unwrap();
    assert_eq!(app.files_selected, 1);

    let scroll_up = mouse_event(
        MouseEventKind::ScrollUp,
        layout.files_panel.x + 5,
        layout.files_panel.y + 3,
    );
    mouse::handle_mouse_event(&mut app, &mut state, scroll_up, screen).unwrap();
    assert_eq!(app.files_selected, 0);

    // 2. Wheel over Commits panel
    assert_eq!(app.commits_selected, 0);
    let scroll_down_c = mouse_event(
        MouseEventKind::ScrollDown,
        layout.commits_panel.x + 5,
        layout.commits_panel.y + 3,
    );
    mouse::handle_mouse_event(&mut app, &mut state, scroll_down_c, screen).unwrap();
    assert_eq!(app.commits_selected, 1);

    let scroll_up_c = mouse_event(
        MouseEventKind::ScrollUp,
        layout.commits_panel.x + 5,
        layout.commits_panel.y + 3,
    );
    mouse::handle_mouse_event(&mut app, &mut state, scroll_up_c, screen).unwrap();
    assert_eq!(app.commits_selected, 0);

    // 3. Wheel over Inspector pane
    assert_eq!(app.inspector_scroll, 0);
    let scroll_down_i = mouse_event(
        MouseEventKind::ScrollDown,
        layout.inspector.x + 10,
        layout.inspector.y + 10,
    );
    mouse::handle_mouse_event(&mut app, &mut state, scroll_down_i, screen).unwrap();
    assert_eq!(app.inspector_scroll, 3);

    let scroll_up_i = mouse_event(
        MouseEventKind::ScrollUp,
        layout.inspector.x + 10,
        layout.inspector.y + 10,
    );
    mouse::handle_mouse_event(&mut app, &mut state, scroll_up_i, screen).unwrap();
    assert_eq!(app.inspector_scroll, 0);
}

#[test]
fn test_mouse_footer_button_clicking() {
    let (_tmp, mut app) = setup_test_repo();
    let mut state = MouseState::new();
    let screen = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 40,
    };
    let layout = compute_layout(screen).expect("valid layout");

    let (_, buttons) = ui::build_footer(&app, layout.footer.width);

    // 1. Click Help button '?'
    let help_btn = buttons
        .iter()
        .find(|(_, _, a)| *a == ui::FooterAction::Help)
        .expect("Help button in footer");
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.footer.x + help_btn.0 + 1,
        layout.footer.y,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    assert_eq!(app.active_modal, ActiveModal::Help);

    // Close help modal
    app.close_modal();

    // 2. Click Stage All button 'a'
    let all_btn = buttons
        .iter()
        .find(|(_, _, a)| *a == ui::FooterAction::StageAll)
        .expect("StageAll button in footer");
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.footer.x + all_btn.0 + 1,
        layout.footer.y,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    assert!(app.files.iter().all(|f| f.kind.is_staged()));

    // 3. Click Quit button 'q' (re-fetch buttons as status message update shifted bounds)
    let (_, buttons_updated) = ui::build_footer(&app, layout.footer.width);
    let quit_btn = buttons_updated
        .iter()
        .find(|(_, _, a)| *a == ui::FooterAction::Quit)
        .expect("Quit button in footer");
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        layout.footer.x + quit_btn.0 + 1,
        layout.footer.y,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    assert!(app.should_quit);
}

#[test]
fn test_mouse_modal_interactions() {
    let (_tmp, mut app) = setup_test_repo();
    let mut state = MouseState::new();
    let screen = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 40,
    };

    // Open CommitPrompt modal
    app.active_modal = ActiveModal::CommitPrompt {
        message: "feat: add feature".to_string(),
        cursor: 0,
    };

    let modal_layout = ui::compute_modal_layout(&app.active_modal, screen).unwrap();

    // 1. Click text input box at column offset 7
    let input_rect = modal_layout.input_rect.unwrap();
    let ev = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        input_rect.x + 7,
        input_rect.y,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev, screen).unwrap();
    if let ActiveModal::CommitPrompt { cursor, .. } = app.active_modal {
        assert_eq!(cursor, 7);
    } else {
        panic!("Modal closed unexpectedly");
    }

    // 2. Click outside modal rect (backdrop click dismisses modal)
    let ev_outside = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        modal_layout.area.x.saturating_sub(5),
        modal_layout.area.y.saturating_sub(5),
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev_outside, screen).unwrap();
    assert_eq!(app.active_modal, ActiveModal::None);

    // 3. Help modal closes on click inside
    app.active_modal = ActiveModal::Help;
    let help_layout = ui::compute_modal_layout(&app.active_modal, screen).unwrap();
    let ev_help = mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        help_layout.area.x + 5,
        help_layout.area.y + 5,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev_help, screen).unwrap();
    assert_eq!(app.active_modal, ActiveModal::None);

    // 4. Right click dismisses modal
    app.active_modal = ActiveModal::BranchCreate {
        name: "new-branch".to_string(),
        cursor: 0,
    };
    let ev_rc = mouse_event(
        MouseEventKind::Down(MouseButton::Right),
        help_layout.area.x + 5,
        help_layout.area.y + 5,
    );
    mouse::handle_mouse_event(&mut app, &mut state, ev_rc, screen).unwrap();
    assert_eq!(app.active_modal, ActiveModal::None);
}

#[test]
fn test_mouse_small_viewport_and_boundary_safety() {
    let (_tmp, mut app) = setup_test_repo();
    let mut state = MouseState::new();

    // Small terminal should not panic
    let small_screen = Rect {
        x: 0,
        y: 0,
        width: 15,
        height: 4,
    };
    let ev = mouse_event(MouseEventKind::Down(MouseButton::Left), 5, 2);
    let res = mouse::handle_mouse_event(&mut app, &mut state, ev, small_screen);
    assert!(res.is_ok());

    let ev_scroll = mouse_event(MouseEventKind::ScrollDown, 5, 2);
    let res_scroll = mouse::handle_mouse_event(&mut app, &mut state, ev_scroll, small_screen);
    assert!(res_scroll.is_ok());
}

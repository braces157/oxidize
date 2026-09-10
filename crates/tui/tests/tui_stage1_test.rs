//! Comprehensive real-time tests for Stage 1: Multi-Panel Navigation & Live Diff Inspector.

use oxidize_tui::model::{DiffLineKind, FileStatusKind, Panel};
use oxidize_tui::ui;
use oxidize_tui::App;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn test_panel_enum_cycling_and_indexing() {
    assert_eq!(Panel::Status.index(), 1);
    assert_eq!(Panel::Files.index(), 2);
    assert_eq!(Panel::Branches.index(), 3);
    assert_eq!(Panel::Commits.index(), 4);
    assert_eq!(Panel::Stash.index(), 5);

    // Forward cycle
    assert_eq!(Panel::Status.next(), Panel::Files);
    assert_eq!(Panel::Files.next(), Panel::Branches);
    assert_eq!(Panel::Branches.next(), Panel::Commits);
    assert_eq!(Panel::Commits.next(), Panel::Stash);
    assert_eq!(Panel::Stash.next(), Panel::Status);

    // Backward cycle
    assert_eq!(Panel::Status.prev(), Panel::Stash);
    assert_eq!(Panel::Files.prev(), Panel::Status);
    assert_eq!(Panel::Branches.prev(), Panel::Files);
    assert_eq!(Panel::Commits.prev(), Panel::Branches);
    assert_eq!(Panel::Stash.prev(), Panel::Commits);
}

#[test]
fn test_app_navigation_and_clamping() {
    let mut app = App::new();

    // Default state
    assert_eq!(app.active_panel, Panel::Files);
    assert_eq!(app.files_selected, 0);

    // Empty list navigation shouldn't panic or overflow
    app.next_item();
    assert_eq!(app.files_selected, 0);
    app.prev_item();
    assert_eq!(app.files_selected, 0);

    // Panel switching
    app.select_panel(Panel::Commits);
    assert_eq!(app.active_panel, Panel::Commits);
    assert_eq!(app.inspector_scroll, 0);

    app.next_panel();
    assert_eq!(app.active_panel, Panel::Stash);

    app.prev_panel();
    assert_eq!(app.active_panel, Panel::Commits);
}

#[test]
fn test_inspector_scrolling() {
    let mut app = App::new();
    let diff_text =
        "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10";
    app.cached_diff = Some(oxidize_tui::model::DiffView::from_unified_text(
        "Test", diff_text,
    ));

    assert_eq!(app.inspector_scroll, 0);
    app.scroll_inspector_down(3);
    assert_eq!(app.inspector_scroll, 3);
    app.scroll_inspector_down(5);
    assert_eq!(app.inspector_scroll, 8);
    // Clamping to max lines - 1
    app.scroll_inspector_down(10);
    assert_eq!(app.inspector_scroll, 9);

    // Scroll up
    app.scroll_inspector_up(4);
    assert_eq!(app.inspector_scroll, 5);
    app.scroll_inspector_up(10);
    assert_eq!(app.inspector_scroll, 0);
}

#[test]
fn test_repository_loading_and_multi_panel_state() {
    let tmp = TempDir::new().unwrap();
    let repo_dir = tmp.path();

    // 1. Initialize git repo
    let init_res = Command::new("git")
        .args(["init", "-b", "master"])
        .current_dir(repo_dir)
        .output()
        .expect("git init failed");
    assert!(init_res.status.success());

    Command::new("git")
        .args(["config", "user.name", "Tester"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "tester@test.com"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // 2. Initial commit
    fs::write(repo_dir.join("file1.txt"), "hello world\nline 2\n").unwrap();
    Command::new("git")
        .args(["add", "file1.txt"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // 3. Second commit
    fs::write(repo_dir.join("file2.txt"), "foo\nbar\n").unwrap();
    Command::new("git")
        .args(["add", "file2.txt"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Second commit"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // 4. Create an extra branch
    Command::new("git")
        .args(["branch", "feature/my-branch"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // 5. Create varied file statuses:
    // Staged new file
    fs::write(repo_dir.join("staged_new.txt"), "staged new content\n").unwrap();
    Command::new("git")
        .args(["add", "staged_new.txt"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Unstaged modified file
    fs::write(repo_dir.join("file1.txt"), "hello world modified\nline 2\n").unwrap();

    // Untracked file
    fs::write(repo_dir.join("untracked.rs"), "fn main() {}\n").unwrap();

    // 6. Test App loading
    let git_dir = repo_dir.join(".git");
    let mut app = App::new();
    app.load_repository(&git_dir)
        .expect("failed to load repository");

    // Verify branch info
    assert_eq!(app.branch_name, "master");

    // Verify branches list
    assert_eq!(app.branches.len(), 2);
    let master_branch = app.branches.iter().find(|b| b.name == "master").unwrap();
    assert!(master_branch.is_head);
    let feat_branch = app
        .branches
        .iter()
        .find(|b| b.name == "feature/my-branch")
        .unwrap();
    assert!(!feat_branch.is_head);

    // Verify commits list
    assert_eq!(app.commits.len(), 2);
    assert_eq!(app.commits[0].summary, "Second commit");
    assert_eq!(app.commits[1].summary, "Initial commit");

    // Verify files list
    assert_eq!(app.files.len(), 3);
    let staged_item = app
        .files
        .iter()
        .find(|f| f.path == "staged_new.txt")
        .unwrap();
    assert_eq!(staged_item.kind, FileStatusKind::StagedNew);

    let unstaged_item = app.files.iter().find(|f| f.path == "file1.txt").unwrap();
    assert_eq!(unstaged_item.kind, FileStatusKind::UnstagedModified);

    let untracked_item = app.files.iter().find(|f| f.path == "untracked.rs").unwrap();
    assert_eq!(untracked_item.kind, FileStatusKind::Untracked);

    // Verify live diff inspector on selected file
    assert!(app.cached_diff.is_some());
    let diff = app.cached_diff.as_ref().unwrap();
    assert!(!diff.lines.is_empty());

    // Switch to unstaged item and verify diff updates
    app.files_selected = app
        .files
        .iter()
        .position(|f| f.path == "file1.txt")
        .unwrap();
    app.update_inspector();
    let diff2 = app.cached_diff.as_ref().unwrap();
    assert!(diff2.title.contains("file1.txt"));
    assert!(diff2.lines.iter().any(|l| l.kind == DiffLineKind::Addition));
    assert!(diff2.lines.iter().any(|l| l.kind == DiffLineKind::Deletion));

    // Switch to Commits panel and verify commit diff
    app.select_panel(Panel::Commits);
    let commit_diff = app.cached_diff.as_ref().unwrap();
    assert!(commit_diff.title.contains("Second commit"));
    assert!(commit_diff
        .lines
        .iter()
        .any(|l| l.content.contains("Second commit")));

    // Switch to Branches panel and verify branch details
    app.select_panel(Panel::Branches);
    let branch_diff = app.cached_diff.as_ref().unwrap();
    assert!(branch_diff.title.contains("master"));
    assert!(branch_diff
        .lines
        .iter()
        .any(|l| l.content.contains("Active HEAD: YES")));
}

#[test]
fn test_ui_rendering_headless_backend() {
    let tmp = TempDir::new().unwrap();
    let repo_dir = tmp.path();

    Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Tester"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "tester@test.com"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    fs::write(repo_dir.join("readme.md"), "# Hello\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "chore: init"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    app.load_repository(&repo_dir.join(".git")).unwrap();

    // 1. Standard 80x24 terminal
    let backend_standard = TestBackend::new(80, 24);
    let mut terminal_standard = Terminal::new(backend_standard).unwrap();
    terminal_standard.draw(|f| ui::render(f, &app)).unwrap();

    // 2. Large 160x50 terminal
    let backend_large = TestBackend::new(160, 50);
    let mut terminal_large = Terminal::new(backend_large).unwrap();
    terminal_large.draw(|f| ui::render(f, &app)).unwrap();

    // 3. Compact 40x12 terminal
    let backend_compact = TestBackend::new(40, 12);
    let mut terminal_compact = Terminal::new(backend_compact).unwrap();
    terminal_compact.draw(|f| ui::render(f, &app)).unwrap();

    // 4. Verify buffer text contents
    let buffer = terminal_standard.backend().buffer();
    let text = format!("{:?}", buffer);
    assert!(text.contains("1 Status"));
    assert!(text.contains("2 Files"));
    assert!(text.contains("3 Branches"));
    assert!(text.contains("4 Commits"));
    assert!(text.contains("5 Stash"));
}

#[test]
fn test_empty_repository_handling() {
    let tmp = TempDir::new().unwrap();
    let repo_dir = tmp.path();

    Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    let res = app.load_repository(&repo_dir.join(".git"));
    assert!(res.is_ok());
    assert_eq!(app.branch_name, "main");
    assert!(app.commits.is_empty());
    assert!(app.files.is_empty());

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| ui::render(f, &app)).unwrap();
}

#[test]
fn test_stash_loading_and_inspection() {
    let tmp = TempDir::new().unwrap();
    let repo_dir = tmp.path();

    Command::new("git")
        .args(["init", "-b", "master"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Tester"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "tester@test.com"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    fs::write(repo_dir.join("stash_target.txt"), "committed line\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "base commit"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Modify and stash
    fs::write(repo_dir.join("stash_target.txt"), "modified for stash\n").unwrap();
    let stash_res = Command::new("git")
        .args(["stash", "push", "-m", "my cool stash"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    assert!(stash_res.status.success());

    let mut app = App::new();
    app.load_repository(&repo_dir.join(".git")).unwrap();

    assert_eq!(app.stashes.len(), 1);
    assert!(app.stashes[0].message.contains("my cool stash"));

    // Select Stash panel and inspect
    app.select_panel(Panel::Stash);
    let diff = app.cached_diff.as_ref().unwrap();
    assert!(diff.title.contains("my cool stash"));
    assert!(diff.lines.iter().any(|l| l.content.contains("stash@{0}")));
}

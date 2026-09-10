//! Comprehensive real-time tests for Stage 4 & 5: Branch & Stash Management and Help Modal.

use oxidize_tui::model::{ActiveModal, Panel};
use oxidize_tui::ui;
use oxidize_tui::App;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn create_test_repo() -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let repo_dir = tmp.path().to_path_buf();

    Command::new("git")
        .args(["init", "-b", "master"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Oxidize Developer"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "dev@oxidize.rs"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();

    fs::write(repo_dir.join("root.txt"), "first line\n").unwrap();
    Command::new("git").args(["add", "."]).current_dir(&repo_dir).output().unwrap();
    Command::new("git").args(["commit", "-m", "root commit"]).current_dir(&repo_dir).output().unwrap();

    let git_dir = repo_dir.join(".git");
    (tmp, git_dir)
}

#[test]
fn test_branch_checkout_and_working_tree_update() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    // 1. Create a branch and add a distinctive file
    Command::new("git")
        .args(["checkout", "-b", "feature-x"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    fs::write(repo_dir.join("feature_x.txt"), "feature X content\n").unwrap();
    Command::new("git").args(["add", "."]).current_dir(repo_dir).output().unwrap();
    Command::new("git").args(["commit", "-m", "add feature X"]).current_dir(repo_dir).output().unwrap();

    // Switch back to master
    Command::new("git")
        .args(["checkout", "master"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    assert!(!repo_dir.join("feature_x.txt").exists());

    // 2. Open in TUI and switch to feature-x branch via TUI
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    assert_eq!(app.branch_name, "master");

    app.select_panel(Panel::Branches);
    let feat_idx = app.branches.iter().position(|b| b.name == "feature-x").unwrap();
    app.branches_selected = feat_idx;

    // Checkout selected branch
    app.checkout_selected_branch().unwrap();

    // Verify branch switched
    assert_eq!(app.branch_name, "feature-x");
    assert!(repo_dir.join("feature_x.txt").exists());
    assert!(app.status_message.as_ref().unwrap().contains("Switched to branch 'feature-x'"));

    // Check that HEAD in branches panel is now marked correctly
    let current_branch_item = app.branches.iter().find(|b| b.name == "feature-x").unwrap();
    assert!(current_branch_item.is_head);
}

#[test]
fn test_branch_creation_via_modal() {
    let (_tmp, git_dir) = create_test_repo();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    assert_eq!(app.branch_name, "master");

    app.select_panel(Panel::Branches);
    app.open_create_branch_modal();
    assert!(matches!(app.active_modal, ActiveModal::BranchCreate { .. }));

    // Type branch name
    for c in "release/v1.0".chars() {
        app.handle_modal_char(c);
    }

    // Submit modal
    app.submit_modal().unwrap();

    // Verify new branch is active
    assert_eq!(app.active_modal, ActiveModal::None);
    assert_eq!(app.branch_name, "release/v1.0");
    assert!(app.branches.iter().any(|b| b.name == "release/v1.0" && b.is_head));
}

#[test]
fn test_branch_deletion() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    // Create an extra branch
    Command::new("git")
        .args(["branch", "to-delete"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    app.select_panel(Panel::Branches);

    // 1. Trying to delete active branch should fail
    let master_idx = app.branches.iter().position(|b| b.name == "master").unwrap();
    app.branches_selected = master_idx;
    app.delete_selected_branch().unwrap();
    assert!(app.status_message.as_ref().unwrap().contains("Cannot delete checked-out branch"));
    assert!(app.branches.iter().any(|b| b.name == "master"));

    // 2. Deleting other branch succeeds
    let del_idx = app.branches.iter().position(|b| b.name == "to-delete").unwrap();
    app.branches_selected = del_idx;
    app.delete_selected_branch().unwrap();

    assert!(app.status_message.as_ref().unwrap().contains("Deleted branch 'to-delete'"));
    assert!(!app.branches.iter().any(|b| b.name == "to-delete"));
    assert!(!git_dir.join("refs/heads/to-delete").exists());
}

#[test]
fn test_stash_pop_and_drop() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    // Modify a file and stash
    fs::write(repo_dir.join("root.txt"), "modified before stash\n").unwrap();
    Command::new("git")
        .args(["stash", "push", "-m", "stash testing"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Verify clean after stash
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    assert_eq!(app.stashes.len(), 1);

    // Pop stash
    app.select_panel(Panel::Stash);
    app.pop_selected_stash().unwrap();

    // Verify working tree received changes back
    assert!(app.status_message.as_ref().unwrap().contains("Popped stash"));
    assert_eq!(
        fs::read_to_string(repo_dir.join("root.txt")).unwrap(),
        "modified before stash\n"
    );
    assert_eq!(app.stashes.len(), 0);

    // Stash again and test drop
    Command::new("git")
        .args(["stash", "push", "-m", "second stash to drop"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    app.refresh().unwrap();
    assert_eq!(app.stashes.len(), 1);

    app.drop_selected_stash().unwrap();
    assert!(app.status_message.as_ref().unwrap().contains("Dropped stash"));
    assert_eq!(app.stashes.len(), 0);
}

#[test]
fn test_help_modal_and_branch_modal_rendering() {
    let (_tmp, git_dir) = create_test_repo();
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    let backend = TestBackend::new(90, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    // 1. Render Branch creation modal
    app.active_modal = ActiveModal::BranchCreate {
        name: "feature/new-branch".to_string(),
        cursor: 18,
    };
    terminal.draw(|f| ui::render(f, &app)).unwrap();
    let buf = terminal.backend().buffer();
    let text = format!("{:?}", buf);
    assert!(text.contains("Create & Checkout New Branch"));
    assert!(text.contains("feature/new-branch"));

    // 2. Render Help modal
    app.active_modal = ActiveModal::Help;
    terminal.draw(|f| ui::render(f, &app)).unwrap();
    let buf2 = terminal.backend().buffer();
    let text2 = format!("{:?}", buf2);
    assert!(text2.contains("Keyboard Shortcuts Cheatsheet"));
    assert!(text2.contains("Global Navigation"));
}

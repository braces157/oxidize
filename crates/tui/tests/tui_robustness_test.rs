//! Comprehensive headless regression tests for F17 (TUI Robustness & Safety Boundaries).

use oxidize_core::Object;
use oxidize_pack::RepoObjectStore;
use oxidize_refs::RefStore;
use oxidize_tui::model::ActiveModal;
use oxidize_tui::ops;
use oxidize_tui::ui;
use oxidize_tui::{App, BackgroundJob, BackgroundJobResult, TerminalSessionGuard};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::fs;
use std::panic;
use std::process::Command;
use std::sync::mpsc;
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
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "root commit"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();

    let git_dir = repo_dir.join(".git");
    (tmp, git_dir)
}

#[test]
fn test_terminal_session_guard_state_and_drop() {
    // 1. New guard has all flags cleared
    let mut guard = TerminalSessionGuard::new();
    assert!(!guard.raw_mode_enabled);
    assert!(!guard.alt_screen_active);
    assert!(!guard.mouse_capture_active);
    assert!(!guard.cursor_hidden);

    // 2. Simulated active state restore resets state
    guard.raw_mode_enabled = true;
    guard.alt_screen_active = true;
    guard.mouse_capture_active = true;
    guard.cursor_hidden = true;
    guard.restore();

    assert!(!guard.raw_mode_enabled);
    assert!(!guard.alt_screen_active);
    assert!(!guard.mouse_capture_active);
    assert!(!guard.cursor_hidden);

    // 3. Verify drop execution during panic unwind
    let result = panic::catch_unwind(|| {
        let mut g = TerminalSessionGuard::new();
        g.raw_mode_enabled = false;
        panic!("simulated TUI runtime panic");
    });
    assert!(result.is_err());
}

#[test]
fn test_unicode_modal_text_editing_and_cursor_safety() {
    let mut app = App::new();

    // Open commit modal manually
    app.active_modal = ActiveModal::CommitPrompt {
        message: String::new(),
        cursor: 0,
    };

    // Insert ASCII and multi-byte Unicode (Emoji, Japanese, Accents)
    // "Ox 🦀 漢字 é"
    let test_chars = ['O', 'x', ' ', '🦀', ' ', '漢', '字', ' ', 'é'];
    for c in test_chars {
        app.handle_modal_char(c);
    }

    if let ActiveModal::CommitPrompt {
        ref message,
        cursor,
    } = app.active_modal
    {
        assert_eq!(message, "Ox 🦀 漢字 é");
        assert_eq!(cursor, 9); // 9 characters
    } else {
        panic!("expected ActiveModal::CommitPrompt");
    }

    // Move cursor left 2 characters (before 'é')
    app.handle_modal_left();
    app.handle_modal_left();
    if let ActiveModal::CommitPrompt { cursor, .. } = app.active_modal {
        assert_eq!(cursor, 7);
    }

    // Move cursor left 1 more (before space)
    app.handle_modal_left();
    if let ActiveModal::CommitPrompt { cursor, .. } = app.active_modal {
        assert_eq!(cursor, 6);
    }

    // Delete character at cursor (backspace deletes before cursor: '漢')
    app.handle_modal_backspace();
    if let ActiveModal::CommitPrompt {
        ref message,
        cursor,
    } = app.active_modal
    {
        assert_eq!(message, "Ox 🦀 字 é");
        assert_eq!(cursor, 5);
    }

    // Move cursor all the way left to position 0
    for _ in 0..10 {
        app.handle_modal_left();
    }
    if let ActiveModal::CommitPrompt { cursor, .. } = app.active_modal {
        assert_eq!(cursor, 0);
    }

    // Insert at beginning
    app.handle_modal_char('★');
    if let ActiveModal::CommitPrompt {
        ref message,
        cursor,
    } = app.active_modal
    {
        assert_eq!(message, "★Ox 🦀 字 é");
        assert_eq!(cursor, 1);
    }

    // Move cursor right to end
    for _ in 0..20 {
        app.handle_modal_right();
    }
    app.handle_modal_backspace();
    if let ActiveModal::CommitPrompt { ref message, .. } = app.active_modal {
        assert_eq!(message, "★Ox 🦀 字 ");
    }
}

#[test]
fn test_selection_stability_across_refresh() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    // Create a branch and a couple of files
    Command::new("git")
        .args(["branch", "stable-branch"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    fs::write(repo_dir.join("file_a.txt"), "hello\n").unwrap();
    fs::write(repo_dir.join("file_b.txt"), "world\n").unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    assert!(app.files.len() >= 2);

    // Select the second file
    app.files_selected = 1;
    let selected_file_path = app.files[1].path.clone();

    // Select stable-branch
    let branch_pos = app
        .branches
        .iter()
        .position(|b| b.name == "stable-branch")
        .unwrap();
    app.branches_selected = branch_pos;

    // Refresh and check that selections remain identical
    app.refresh().unwrap();
    assert_eq!(
        app.selected_file().unwrap().path,
        selected_file_path,
        "File selection should be preserved across refresh"
    );
    assert_eq!(
        app.selected_branch().unwrap().name,
        "stable-branch",
        "Branch selection should be preserved across refresh"
    );
}

#[test]
fn test_tui_checkout_rejects_dirty_and_untracked_conflicts() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    // Commit file2 on feature branch
    Command::new("git")
        .args(["checkout", "-b", "feature"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    fs::write(repo_dir.join("tracked.txt"), "feature v1\n").unwrap();
    Command::new("git")
        .args(["add", "tracked.txt"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "feature commit"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Switch back to master
    Command::new("git")
        .args(["checkout", "master"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // 1. Dirty file test: modify root.txt and commit a different root.txt on feature branch
    Command::new("git")
        .args(["checkout", "feature"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    fs::write(repo_dir.join("root.txt"), "feature root content\n").unwrap();
    Command::new("git")
        .args(["commit", "-am", "diverged root"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["checkout", "master"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Dirty modification on master
    fs::write(repo_dir.join("root.txt"), "local uncommitted changes\n").unwrap();

    let store = RepoObjectStore::open(&git_dir).unwrap();
    let ref_store = RefStore::new(&git_dir);
    let feature_oid = ref_store.read_ref("refs/heads/feature").unwrap();
    let feature_commit = match store.read_object(&feature_oid).unwrap() {
        Object::Commit(c) => c,
        _ => panic!("expected commit"),
    };

    // Safe checkout should be rejected without force
    let checkout_res =
        ops::checkout_tree_and_update_index(repo_dir, &git_dir, &feature_commit.tree, false);
    assert!(
        checkout_res.is_err(),
        "Safe checkout must reject overwriting dirty changes"
    );

    // 2. Untracked file conflict test: restore root.txt, create untracked tracked.txt
    Command::new("git")
        .args(["checkout", "--", "root.txt"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    fs::write(
        repo_dir.join("tracked.txt"),
        "untracked colliding content\n",
    )
    .unwrap();

    let untracked_conflict_res =
        ops::checkout_tree_and_update_index(repo_dir, &git_dir, &feature_commit.tree, false);
    assert!(
        untracked_conflict_res.is_err(),
        "Safe checkout must reject overwriting untracked files"
    );

    // Force checkout should succeed
    let force_checkout_res =
        ops::checkout_tree_and_update_index(repo_dir, &git_dir, &feature_commit.tree, true);
    assert!(
        force_checkout_res.is_ok(),
        "Forced checkout should overwrite collisions"
    );
}

#[test]
fn test_tui_branch_deletion_unmerged_safety() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    // Create branch with unmerged commit
    Command::new("git")
        .args(["checkout", "-b", "unmerged-work"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    fs::write(repo_dir.join("work.txt"), "unmerged work\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "unmerged commit"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Switch back to master
    Command::new("git")
        .args(["checkout", "master"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Attempt deletion without force
    let del_res = ops::delete_branch(&git_dir, "unmerged-work", false);
    assert!(
        del_res.is_err(),
        "Deleting unmerged branch without force must fail"
    );
    let err_msg = del_res.unwrap_err().to_string();
    assert!(
        err_msg.contains("not fully merged"),
        "Error message should explain branch is not fully merged"
    );

    // Forced deletion succeeds
    let force_del_res = ops::delete_branch(&git_dir, "unmerged-work", true);
    assert!(
        force_del_res.is_ok(),
        "Forced deletion of unmerged branch must succeed"
    );

    let ref_store = RefStore::new(&git_dir);
    assert!(ref_store.read_ref("refs/heads/unmerged-work").is_err());
}

#[test]
fn test_background_job_non_blocking_and_concurrency_guard() {
    let (_tmp, git_dir) = create_test_repo();
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // Inject a background job
    let (tx, rx) = mpsc::channel();
    app.active_job = Some(BackgroundJob {
        description: "Simulated job".to_string(),
        receiver: rx,
    });

    // Concurrent operations should be blocked
    app.push().unwrap();
    assert_eq!(
        app.status_message.as_deref(),
        Some("⚠ A repository operation is already in progress")
    );

    app.pull().unwrap();
    assert_eq!(
        app.status_message.as_deref(),
        Some("⚠ A repository operation is already in progress")
    );

    // Complete the background job via channel
    tx.send(BackgroundJobResult::Success("Finished sync".to_string()))
        .unwrap();
    app.tick().unwrap();

    assert!(
        app.active_job.is_none(),
        "Active job should be cleared on completion"
    );
    assert_eq!(
        app.status_message.as_deref(),
        Some("✓ Finished sync"),
        "Status message should report background job success"
    );
}

#[test]
fn test_small_viewport_graceful_render() {
    let (_tmp, git_dir) = create_test_repo();
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // Very small terminal dimensions: 15 columns, 4 rows (below minimum 20x6)
    let backend = TestBackend::new(15, 4);
    let mut terminal = Terminal::new(backend).unwrap();

    let render_res = terminal.draw(|f| ui::render(f, &app));
    assert!(
        render_res.is_ok(),
        "Small viewport should render graceful fallback without panicking"
    );
}

//! Comprehensive real-time tests for Stage 3: Interactive Commit Modal & Text Input Handling.

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

    fs::write(repo_dir.join("root.txt"), "first line in root\n").unwrap();
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
fn test_modal_text_editing_and_cursor_navigation() {
    let mut app = App::new();
    app.active_modal = ActiveModal::CommitPrompt {
        message: String::new(),
        cursor: 0,
    };

    // Type "hello"
    for c in "hello".chars() {
        app.handle_modal_char(c);
    }
    match app.active_modal {
        ActiveModal::CommitPrompt {
            ref message,
            cursor,
        } => {
            assert_eq!(message, "hello");
            assert_eq!(cursor, 5);
        }
        _ => panic!("unexpected modal state"),
    }

    // Move cursor left 2 spots (between 'l' and 'o')
    app.handle_modal_left();
    app.handle_modal_left();
    match app.active_modal {
        ActiveModal::CommitPrompt { cursor, .. } => assert_eq!(cursor, 3),
        _ => panic!(),
    }

    // Insert 'p' -> "helplo"
    app.handle_modal_char('p');
    match app.active_modal {
        ActiveModal::CommitPrompt {
            ref message,
            cursor,
        } => {
            assert_eq!(message, "helplo");
            assert_eq!(cursor, 4);
        }
        _ => panic!(),
    }

    // Backspace removes 'p' -> "hello"
    app.handle_modal_backspace();
    match app.active_modal {
        ActiveModal::CommitPrompt {
            ref message,
            cursor,
        } => {
            assert_eq!(message, "hello");
            assert_eq!(cursor, 3);
        }
        _ => panic!(),
    }

    // Close modal
    app.close_modal();
    assert_eq!(app.active_modal, ActiveModal::None);
}

#[test]
fn test_open_commit_modal_requires_staged_changes() {
    let (_tmp, git_dir) = create_test_repo();
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // Working tree is clean -> open_commit_modal should reject
    app.open_commit_modal();
    assert_eq!(app.active_modal, ActiveModal::None);
    assert!(app
        .status_message
        .as_ref()
        .unwrap()
        .contains("No files are staged"));
}

#[test]
fn test_commit_aborted_on_empty_message() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    fs::write(repo_dir.join("change.txt"), "staged content\n").unwrap();
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // Stage the file
    app.toggle_stage_selected().unwrap();
    assert!(app.files.iter().any(|f| f.kind.is_staged()));

    // Open modal and submit with whitespace only
    app.open_commit_modal();
    app.handle_modal_char(' ');
    app.handle_modal_char(' ');
    app.submit_modal().unwrap();

    assert_eq!(app.active_modal, ActiveModal::None);
    assert!(app
        .status_message
        .as_ref()
        .unwrap()
        .contains("Commit aborted"));
    // File remains staged
    assert!(app.files.iter().any(|f| f.kind.is_staged()));
}

#[test]
fn test_successful_interactive_commit_workflow() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    // Add a new file and modify root.txt
    fs::write(
        repo_dir.join("feature.rs"),
        "pub fn lazy_git() -> bool { true }\n",
    )
    .unwrap();
    fs::write(repo_dir.join("root.txt"), "updated line in root\n").unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    let initial_commits_count = app.commits.len();
    assert_eq!(initial_commits_count, 1);

    // Stage all changes
    app.stage_all().unwrap();
    assert!(app.files.iter().all(|f| f.kind.is_staged()));

    // Open commit modal
    app.open_commit_modal();
    assert!(matches!(app.active_modal, ActiveModal::CommitPrompt { .. }));

    // Type commit message
    let msg = "feat: implement interactive commit dialog";
    for c in msg.chars() {
        app.handle_modal_char(c);
    }

    // Submit commit
    app.submit_modal().unwrap();

    // 1. Verify modal closed
    assert_eq!(app.active_modal, ActiveModal::None);

    // 2. Verify success status message
    let status_msg = app.status_message.as_ref().unwrap();
    assert!(status_msg.contains("master"));
    assert!(status_msg.contains(msg));

    // 3. Verify commit history advanced
    assert_eq!(app.commits.len(), initial_commits_count + 1);
    assert_eq!(app.commits[0].summary, msg);

    // 4. Verify working tree is now clean
    assert!(app.files.is_empty());

    // 5. Inspect the new commit in Commits panel
    app.select_panel(Panel::Commits);
    let diff = app.cached_diff.as_ref().unwrap();
    assert!(diff.title.contains(msg));
    assert!(diff
        .lines
        .iter()
        .any(|l| l.content.contains("pub fn lazy_git")));
}

#[test]
fn test_modal_rendering_with_test_backend() {
    let (_tmp, git_dir) = create_test_repo();
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    let backend = TestBackend::new(90, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    // 1. Render commit prompt modal
    app.active_modal = ActiveModal::CommitPrompt {
        message: "fix: resolve edge case".to_string(),
        cursor: 22,
    };
    terminal.draw(|f| ui::render(f, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    let buf_text = format!("{:?}", buffer);
    assert!(buf_text.contains("Commit Staged Changes"));
    assert!(buf_text.contains("fix: resolve edge case"));

    // 2. Render Help cheatsheet modal
    app.active_modal = ActiveModal::Help;
    terminal.draw(|f| ui::render(f, &app)).unwrap();
    let buffer2 = terminal.backend().buffer();
    let buf_text2 = format!("{:?}", buffer2);
    assert!(buf_text2.contains("Keyboard Shortcuts Cheatsheet"));
    assert!(buf_text2.contains("Space"));
    assert!(buf_text2.contains("Toggle stage"));
}

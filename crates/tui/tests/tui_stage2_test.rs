//! Comprehensive real-time tests for Stage 2: Interactive Staging & Working Tree Operations.

use oxidize_index::Index;
use oxidize_tui::model::FileStatusKind;
use oxidize_tui::App;
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
        .args(["config", "user.name", "Tester"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "tester@test.com"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();

    // Initial commit with two files
    fs::write(repo_dir.join("tracked1.txt"), "hello from tracked 1\n").unwrap();
    fs::write(repo_dir.join("tracked2.txt"), "hello from tracked 2\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "initial commit"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();

    let git_dir = repo_dir.join(".git");
    (tmp, git_dir)
}

#[test]
fn test_stage_and_unstage_untracked_file() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    // Create a new untracked file
    fs::write(repo_dir.join("new_file.txt"), "fresh content\n").unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // Verify it starts as Untracked
    let file_idx = app
        .files
        .iter()
        .position(|f| f.path == "new_file.txt")
        .unwrap();
    app.files_selected = file_idx;
    assert_eq!(app.files[file_idx].kind, FileStatusKind::Untracked);

    // 1. Stage the file
    app.toggle_stage_selected().unwrap();

    // Verify status updated to StagedNew
    assert!(app.status_message.as_ref().unwrap().contains("Staged"));
    let staged_idx = app
        .files
        .iter()
        .position(|f| f.path == "new_file.txt")
        .unwrap();
    assert_eq!(app.files[staged_idx].kind, FileStatusKind::StagedNew);

    // Verify written to index
    let index = Index::load_from(git_dir.join("index")).unwrap();
    assert!(index.find_entry("new_file.txt").is_some());

    // 2. Unstage the file
    app.files_selected = staged_idx;
    app.toggle_stage_selected().unwrap();

    // Verify status updated back to Untracked
    assert!(app.status_message.as_ref().unwrap().contains("Unstaged"));
    let untracked_idx = app
        .files
        .iter()
        .position(|f| f.path == "new_file.txt")
        .unwrap();
    assert_eq!(app.files[untracked_idx].kind, FileStatusKind::Untracked);

    // Verify removed from index
    let index2 = Index::load_from(git_dir.join("index")).unwrap();
    assert!(index2.find_entry("new_file.txt").is_none());
}

#[test]
fn test_stage_and_unstage_modified_file() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    // Modify tracked1.txt
    fs::write(repo_dir.join("tracked1.txt"), "modified line\n").unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    let file_idx = app
        .files
        .iter()
        .position(|f| f.path == "tracked1.txt")
        .unwrap();
    app.files_selected = file_idx;
    assert_eq!(app.files[file_idx].kind, FileStatusKind::UnstagedModified);

    // 1. Stage modification
    app.toggle_stage_selected().unwrap();

    let staged_idx = app
        .files
        .iter()
        .position(|f| f.path == "tracked1.txt")
        .unwrap();
    assert_eq!(app.files[staged_idx].kind, FileStatusKind::StagedModified);

    // 2. Unstage modification
    app.files_selected = staged_idx;
    app.toggle_stage_selected().unwrap();

    let unstaged_idx = app
        .files
        .iter()
        .position(|f| f.path == "tracked1.txt")
        .unwrap();
    assert_eq!(
        app.files[unstaged_idx].kind,
        FileStatusKind::UnstagedModified
    );
}

#[test]
fn test_stage_all_and_unstage_all() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    // Modify tracked1, delete tracked2, add untracked3
    fs::write(repo_dir.join("tracked1.txt"), "modified tracked1\n").unwrap();
    fs::remove_file(repo_dir.join("tracked2.txt")).unwrap();
    fs::write(repo_dir.join("untracked3.txt"), "new file 3\n").unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    assert_eq!(app.files.len(), 3);
    assert!(app.files.iter().all(|f| !f.kind.is_staged()));

    // 1. Stage All
    app.stage_all().unwrap();
    assert!(app.status_message.as_ref().unwrap().contains("Staged all"));
    assert_eq!(app.files.len(), 3);
    assert!(app.files.iter().all(|f| f.kind.is_staged()));

    // 2. Unstage All
    app.stage_all().unwrap();
    assert!(app
        .status_message
        .as_ref()
        .unwrap()
        .contains("Unstaged all"));
    assert_eq!(app.files.len(), 3);
    assert!(app.files.iter().all(|f| !f.kind.is_staged()));
}

#[test]
fn test_discard_modified_file() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    let original_content = "hello from tracked 1\n";
    assert_eq!(
        fs::read_to_string(repo_dir.join("tracked1.txt")).unwrap(),
        original_content
    );

    // Make an unwanted edit
    fs::write(repo_dir.join("tracked1.txt"), "unwanted corrupted change\n").unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    let file_idx = app
        .files
        .iter()
        .position(|f| f.path == "tracked1.txt")
        .unwrap();
    app.files_selected = file_idx;
    assert_eq!(app.files[file_idx].kind, FileStatusKind::UnstagedModified);

    // Discard changes
    app.discard_selected_file().unwrap();

    // Verify disk content reverted
    let reverted_content = fs::read_to_string(repo_dir.join("tracked1.txt")).unwrap();
    assert_eq!(reverted_content, original_content);

    // Verify working tree is clean
    assert!(app.files.is_empty());
}

#[test]
fn test_discard_untracked_file() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    let junk_path = repo_dir.join("temporary_junk.txt");
    fs::write(&junk_path, "throwaway content\n").unwrap();
    assert!(junk_path.exists());

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    let file_idx = app
        .files
        .iter()
        .position(|f| f.path == "temporary_junk.txt")
        .unwrap();
    app.files_selected = file_idx;

    // Discard untracked file
    app.discard_selected_file().unwrap();

    // Verify deleted from filesystem
    assert!(!junk_path.exists());
    assert!(app.files.is_empty());
}

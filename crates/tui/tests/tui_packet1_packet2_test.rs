//! Packet 1 & Packet 2 Integration Tests:
//! - Packet 1: Linked worktree discovery, common_dir resolution, packed-only branch deletion, upstream resolution.
//! - Packet 2: Recursive directory staging, partial hunk stage, partial hunk unstage, partial hunk discard, CRLF line-ending fidelity.

use oxidize_index::Index;
use oxidize_pack::store::RepoObjectStore;
use oxidize_refs::RefStore;
use oxidize_tui::model::{ActiveModal, ConfirmAction, FileStatusKind, Panel};
use oxidize_tui::ops;
use oxidize_tui::App;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn create_base_repo() -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let repo_dir = tmp.path().to_path_buf();

    Command::new("git")
        .args(["init", "-b", "main"])
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
    Command::new("git")
        .args(["config", "core.autocrlf", "false"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();

    fs::write(
        repo_dir.join("initial.txt"),
        "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\n",
    )
    .unwrap();

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
fn test_packet1_linked_worktree_discovery_and_shared_objects() {
    let (tmp, main_git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // Create a linked worktree using git worktree add
    let wt_dir = repo_dir.join("linked_wt");
    let output = Command::new("git")
        .args([
            "worktree",
            "add",
            wt_dir.to_str().unwrap(),
            "-b",
            "feature-wt",
        ])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "Failed to add worktree");

    let wt_git_file = wt_dir.join(".git");
    assert!(
        wt_git_file.is_file(),
        ".git in linked worktree must be a gitdir pointer file"
    );

    let mut app = App::new();
    app.load_repository(&wt_git_file)
        .expect("Must load linked worktree without error");

    // Assert correct path and repository truth resolution
    assert_eq!(
        app.repo_root.canonicalize().unwrap(),
        wt_dir.canonicalize().unwrap()
    );
    assert_eq!(
        app.common_dir.canonicalize().unwrap(),
        main_git_dir.canonicalize().unwrap()
    );
    assert_eq!(app.branch_name, "feature-wt");
    assert!(
        !app.commits.is_empty(),
        "Commits from common object store must be loaded"
    );

    // Modify a file in the linked worktree and verify dirty detection
    fs::write(wt_dir.join("new_wt_file.txt"), "worktree dirty content\n").unwrap();
    app.refresh().unwrap();
    assert!(app
        .files
        .iter()
        .any(|f| f.path == "new_wt_file.txt" && f.kind == FileStatusKind::Untracked));
}

#[test]
fn test_packet1_packed_only_branch_deletion_atomicity() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // Create a branch and pack all refs into packed-refs
    Command::new("git")
        .args(["branch", "packed-target"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["pack-refs", "--all"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Ensure loose ref is gone and packed-refs contains the ref
    assert!(!git_dir.join("refs/heads/packed-target").exists());
    let packed_content = fs::read_to_string(git_dir.join("packed-refs")).unwrap();
    assert!(packed_content.contains("refs/heads/packed-target"));

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    assert!(app.branches.iter().any(|b| b.name == "packed-target"));

    // Delete the packed branch
    app.delete_branch_by_name("packed-target")
        .expect("Must delete packed branch");
    assert!(!app.branches.iter().any(|b| b.name == "packed-target"));

    // Verify packed-refs file was updated atomically and does NOT contain the branch
    let updated_packed = fs::read_to_string(git_dir.join("packed-refs")).unwrap();
    assert!(!updated_packed.contains("refs/heads/packed-target"));

    // Reload app to verify branch does not resurrect
    let mut reloaded = App::new();
    reloaded.load_repository(&git_dir).unwrap();
    assert!(!reloaded.branches.iter().any(|b| b.name == "packed-target"));
}

#[test]
fn test_packet1_configured_upstream_resolution() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // Configure upstream tracking pointing to a different branch name
    Command::new("git")
        .args(["config", "branch.main.remote", "origin"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "branch.main.merge", "refs/heads/custom-upstream"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Get HEAD commit OID
    let ref_store = RefStore::new(&git_dir);
    let (_, head_oid) = ref_store.resolve_head().unwrap();
    let head_oid = head_oid.unwrap();

    // Write a fake remote ref origin/custom-upstream pointing to HEAD
    let remotes_dir = git_dir.join("refs/remotes/origin");
    fs::create_dir_all(&remotes_dir).unwrap();
    fs::write(
        remotes_dir.join("custom-upstream"),
        format!("{}\n", head_oid),
    )
    .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // Upstream matches HEAD -> ahead = 0, behind = 0
    assert_eq!(app.ahead_behind, (0, 0));

    // Now make a new commit on main so main is 1 ahead of custom-upstream
    fs::write(repo_dir.join("another.txt"), "hello\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "second commit"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    app.refresh().unwrap();
    assert_eq!(
        app.ahead_behind,
        (1, 0),
        "Must detect 1 commit ahead of custom-upstream"
    );
}

#[test]
fn test_packet2_untracked_directory_batch_staging() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // Create nested untracked directory with multiple files
    let sub_dir = repo_dir.join("src").join("nested");
    fs::create_dir_all(&sub_dir).unwrap();
    fs::write(sub_dir.join("alpha.rs"), "pub fn alpha() {}\n").unwrap();
    fs::write(sub_dir.join("beta.rs"), "pub fn beta() {}\n").unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // Stage directory "src" via ops::stage_path (which delegates to batch staging)
    ops::stage_path(repo_dir, &git_dir, "src").expect("Must stage directory recursively");

    // Verify index contains both nested files
    let index = Index::load_from(git_dir.join("index")).unwrap();
    assert!(index.find_entry("src/nested/alpha.rs").is_some());
    assert!(index.find_entry("src/nested/beta.rs").is_some());

    // Reload app and verify files show as staged
    app.refresh().unwrap();
    let alpha = app
        .files
        .iter()
        .find(|f| f.path == "src/nested/alpha.rs")
        .unwrap();
    let beta = app
        .files
        .iter()
        .find(|f| f.path == "src/nested/beta.rs")
        .unwrap();
    assert!(alpha.kind.is_staged());
    assert!(beta.kind.is_staged());
}

#[test]
fn test_packet2_partial_hunk_staging() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // Modify line 1 and line 10 to create 2 distinct hunks
    let modified = "line 1 MODIFIED\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10 MODIFIED\n";
    fs::write(repo_dir.join("initial.txt"), modified).unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    app.select_panel(Panel::Files);
    assert_eq!(app.selected_file().unwrap().path, "initial.txt");

    // Verify diff has 2 hunks
    let diff = app.cached_diff.as_ref().unwrap();
    assert_eq!(diff.hunks.len(), 2, "Expected 2 distinct hunks");
    assert_eq!(app.selected_hunk(), Some(0));

    // Stage hunk 0 (the modification to line 1)
    app.toggle_stage_selected_hunk().expect("Must stage hunk 0");

    // Check index content
    let store = RepoObjectStore::open(&git_dir).unwrap();
    let index = Index::load_from(git_dir.join("index")).unwrap();
    let entry = index.find_entry("initial.txt").unwrap();
    let staged_text = ops::read_blob_text(&store, &entry.oid);

    // Staged text should contain line 1 MODIFIED, but original line 10!
    assert!(staged_text.starts_with("line 1 MODIFIED\n"));
    assert!(staged_text.ends_with("line 9\nline 10\n"));

    // Working tree file still has both modifications
    let wt_text = fs::read_to_string(repo_dir.join("initial.txt")).unwrap();
    assert!(wt_text.starts_with("line 1 MODIFIED\n"));
    assert!(wt_text.ends_with("line 9\nline 10 MODIFIED\n"));
}

#[test]
fn test_packet2_partial_hunk_unstaging() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // Modify line 1 and line 10
    let modified = "line 1 MODIFIED\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10 MODIFIED\n";
    fs::write(repo_dir.join("initial.txt"), modified).unwrap();

    // Stage the entire file first
    Command::new("git")
        .args(["add", "initial.txt"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    app.select_panel(Panel::Files);

    // Selected file is StagedModified
    let file = app.selected_file().unwrap();
    assert!(file.kind.is_staged());

    let diff = app.cached_diff.as_ref().unwrap();
    assert_eq!(diff.hunks.len(), 2, "Should have 2 staged hunks");
    assert!(diff.is_staged);

    // Advance to hunk 1 (modifications at line 10)
    app.next_hunk();
    assert_eq!(app.selected_hunk(), Some(1));

    // Unstage hunk 1
    app.toggle_stage_selected_hunk()
        .expect("Must unstage hunk 1");

    // Verify index now retains hunk 0 (line 1 MODIFIED) but line 10 is reverted to original!
    let store = RepoObjectStore::open(&git_dir).unwrap();
    let index = Index::load_from(git_dir.join("index")).unwrap();
    let entry = index.find_entry("initial.txt").unwrap();
    let staged_text = ops::read_blob_text(&store, &entry.oid);

    assert!(staged_text.starts_with("line 1 MODIFIED\n"));
    assert!(staged_text.ends_with("line 9\nline 10\n"));
}

#[test]
fn test_packet2_partial_hunk_discard() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // Modify line 1 and line 10
    let modified = "line 1 MODIFIED\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10 MODIFIED\n";
    fs::write(repo_dir.join("initial.txt"), modified).unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    app.select_panel(Panel::Files);

    // Advance to hunk 1
    app.next_hunk();
    assert_eq!(app.selected_hunk(), Some(1));

    // Trigger discard confirmation modal
    app.prompt_discard_selected_hunk();
    match &app.active_modal {
        ActiveModal::Confirm { action, .. } => {
            assert_eq!(
                action,
                &ConfirmAction::DiscardHunk {
                    path: "initial.txt".to_string(),
                    hunk_idx: 1
                }
            );
        }
        _ => panic!("Expected Confirm modal for discarding hunk"),
    }

    // Submit the discard action
    app.submit_modal().expect("Must discard hunk 1");

    // Working tree file now has line 1 MODIFIED, but line 10 reverted to original
    let wt_text = fs::read_to_string(repo_dir.join("initial.txt")).unwrap();
    assert!(wt_text.starts_with("line 1 MODIFIED\n"));
    assert!(wt_text.ends_with("line 9\nline 10\n"));
}

#[test]
fn test_packet2_crlf_line_ending_fidelity() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // Write a CRLF file
    let crlf_content = "line 1\r\nline 2\r\nline 3\r\nline 4\r\nline 5\r\nline 6\r\nline 7\r\nline 8\r\nline 9\r\nline 10\r\n";
    fs::write(repo_dir.join("crlf.txt"), crlf_content).unwrap();
    Command::new("git")
        .args(["add", "crlf.txt"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "commit crlf file"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Modify line 1 and line 10 preserving CRLF
    let modified_crlf = "line 1 CHANGED\r\nline 2\r\nline 3\r\nline 4\r\nline 5\r\nline 6\r\nline 7\r\nline 8\r\nline 9\r\nline 10 CHANGED\r\n";
    fs::write(repo_dir.join("crlf.txt"), modified_crlf).unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    app.select_panel(Panel::Files);

    let crlf_file_idx = app.files.iter().position(|f| f.path == "crlf.txt").unwrap();
    app.select_file_index(crlf_file_idx);

    // Stage hunk 0
    assert_eq!(app.selected_hunk(), Some(0));
    app.toggle_stage_selected_hunk()
        .expect("Must stage CRLF hunk 0");

    // Verify index blob preserves exact CRLF line endings
    let store = RepoObjectStore::open(&git_dir).unwrap();
    let index = Index::load_from(git_dir.join("index")).unwrap();
    let entry = index.find_entry("crlf.txt").unwrap();

    let blob = match store.read_object(&entry.oid).unwrap() {
        oxidize_core::Object::Blob(b) => b,
        _ => panic!("Expected blob object in store"),
    };

    // Assert that \r\n is preserved on EVERY line in the staged blob
    let blob_str = String::from_utf8(blob.data.clone()).unwrap();
    assert!(blob_str.contains("line 1 CHANGED\r\n"));
    assert!(blob_str.contains("line 2\r\n"));
    assert!(blob_str.contains("line 10\r\n"));
    assert!(!blob_str.replace("\r\n", "").contains('\r')); // All \r are followed by \n
}

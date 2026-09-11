//! Packet 4 Integration Tests:
//! - Native Replay Sequencer state machine and .git/rebase-merge/ persistence
//! - Interactive Rebase Todo Editor Modal (navigation, action cycling, reordering)
//! - Interactive Rebase Workflows: Pick, Drop, Reorder, Reword, Squash, Fixup
//! - Conflict Detection & Index Stages (Stage 1 base, Stage 2 ours, Stage 3 theirs)
//! - Native Conflict Resolution: Ours, Theirs, Both
//! - Sequencer Control: Continue, Skip, Abort
//! - Commit Revert Workflow: Inverse 3-way merge commit creation
//! - UI Rendering of Rebase Modal, Conflicted File Badges, and Rebase Status Lines

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

use oxidize_core::id::ObjectId;
use oxidize_core::object::Object;
use oxidize_pack::store::RepoObjectStore;
use oxidize_refs::RefStore;
use oxidize_tui::model::{ActiveModal, ConfirmAction, ConflictChoice, FileStatusKind, Panel};
use oxidize_tui::sequencer::{RebaseAction, RebaseTodoItem, SequencerState, SequencerStatus};
use oxidize_tui::App;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn create_base_repo() -> (TempDir, PathBuf) {
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

    fs::write(repo_dir.join("base.txt"), "base content\n").unwrap();
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

fn get_head_oid(git_dir: &Path) -> ObjectId {
    RefStore::new(git_dir).resolve_head().unwrap().1.unwrap()
}

#[test]
fn test_packet4_rebase_todo_modal_navigation_and_actions() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // Add 3 commits on main
    for i in 1..=3 {
        let filename = format!("file{}.txt", i);
        fs::write(repo_dir.join(&filename), format!("content {}\n", i)).unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(repo_dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", &format!("commit {}", i)])
            .current_dir(repo_dir)
            .output()
            .unwrap();
    }

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // Focus commits panel
    app.select_panel(Panel::Commits);
    assert!(!app.commits.is_empty());

    // Select commit 1 (index 2: commit 1)
    app.commits_selected = 2;
    app.open_rebase_todo_modal();

    // Verify modal is open as RebaseTodo
    match &app.active_modal {
        ActiveModal::RebaseTodo {
            items, selected, ..
        } => {
            assert_eq!(*selected, 0);
            assert_eq!(items.len(), 3);
            assert_eq!(items[0].action, RebaseAction::Pick);
        }
        _ => panic!("Expected ActiveModal::RebaseTodo"),
    }

    // Cycle actions using space
    app.handle_rebase_todo_key(
        crossterm::event::KeyCode::Char(' '),
        crossterm::event::KeyModifiers::NONE,
    );
    if let ActiveModal::RebaseTodo { items, .. } = &app.active_modal {
        assert_eq!(items[0].action, RebaseAction::Reword);
    }

    // Change action directly using hotkeys 'd' for drop
    app.handle_rebase_todo_key(
        crossterm::event::KeyCode::Char('d'),
        crossterm::event::KeyModifiers::NONE,
    );
    if let ActiveModal::RebaseTodo { items, .. } = &app.active_modal {
        assert_eq!(items[0].action, RebaseAction::Drop);
    }

    // Change to edit ('e')
    app.handle_rebase_todo_key(
        crossterm::event::KeyCode::Char('e'),
        crossterm::event::KeyModifiers::NONE,
    );
    if let ActiveModal::RebaseTodo { items, .. } = &app.active_modal {
        assert_eq!(items[0].action, RebaseAction::Edit);
    }

    // Move down with 'j'
    app.handle_rebase_todo_key(
        crossterm::event::KeyCode::Char('j'),
        crossterm::event::KeyModifiers::NONE,
    );
    if let ActiveModal::RebaseTodo { selected, .. } = &app.active_modal {
        assert_eq!(*selected, 1);
    }

    // Reorder items: shift current item up with 'K'
    app.handle_rebase_todo_key(
        crossterm::event::KeyCode::Char('K'),
        crossterm::event::KeyModifiers::NONE,
    );
    if let ActiveModal::RebaseTodo {
        items, selected, ..
    } = &app.active_modal
    {
        assert_eq!(*selected, 0);
        assert_eq!(items[0].action, RebaseAction::Pick);
        assert_eq!(items[1].action, RebaseAction::Edit);
    }

    // Close modal
    app.close_modal();
    assert_eq!(app.active_modal, ActiveModal::None);
}

#[test]
fn test_packet4_interactive_rebase_pick_reorder_and_drop() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // Commit 1: add fileA.txt
    fs::write(repo_dir.join("fileA.txt"), "A\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "commit A"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Commit 2: add fileB.txt
    fs::write(repo_dir.join("fileB.txt"), "B\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "commit B"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Commit 3: add fileC.txt
    fs::write(repo_dir.join("fileC.txt"), "C\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "commit C"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // Onto commit is initial commit: select commit A (index 2 in [C, B, A, initial])
    app.select_panel(Panel::Commits);
    app.commits_selected = 2;
    let onto_oid = app.commits[3].oid;
    app.open_rebase_todo_modal();

    // We have 3 items: [commit A, commit B, commit C]
    if let ActiveModal::RebaseTodo { items, .. } = &mut app.active_modal {
        assert_eq!(items.len(), 3);
        // Set item 1 (commit B) to Drop
        items[1].action = RebaseAction::Drop;
        // Swap item 0 (commit A) and item 2 (commit C) -> new order: C, B(drop), A
        items.swap(0, 2);
    }

    // Submit modal -> executes rebase
    app.submit_modal().unwrap();

    // Verify rebase completed
    assert!(!SequencerState::is_active(&git_dir));

    // Reload repository
    app.load_repository(&git_dir).unwrap();

    // Verify fileB does not exist (dropped)
    assert!(!repo_dir.join("fileB.txt").exists());
    // Verify fileA and fileC exist
    assert!(repo_dir.join("fileA.txt").exists());
    assert!(repo_dir.join("fileC.txt").exists());

    // Verify commit order: HEAD should be commit A, parent should be commit C, parent parent should be initial
    let head_commit = &app.commits[0];
    assert_eq!(head_commit.summary, "commit A");
    let c_commit = &app.commits[1];
    assert_eq!(c_commit.summary, "commit C");
    let init_commit = &app.commits[2];
    assert_eq!(init_commit.oid, onto_oid);
}

#[test]
fn test_packet4_interactive_rebase_squash_and_fixup() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // Commit 1: add part1.txt
    fs::write(repo_dir.join("part1.txt"), "part 1\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "feature base"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Commit 2: add part2.txt (will be squashed)
    fs::write(repo_dir.join("part2.txt"), "part 2\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "squash details"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Commit 3: add part3.txt (will be fixup)
    fs::write(repo_dir.join("part3.txt"), "part 3\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "fixup typo"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // Rebase onto initial commit: select feature base (index 2 in [fixup, squash, base, initial])
    app.select_panel(Panel::Commits);
    app.commits_selected = 2;
    app.open_rebase_todo_modal();

    if let ActiveModal::RebaseTodo { items, .. } = &mut app.active_modal {
        assert_eq!(items.len(), 3);
        // item 0: Pick (feature base)
        items[0].action = RebaseAction::Pick;
        // item 1: Squash (squash details)
        items[1].action = RebaseAction::Squash;
        // item 2: Fixup (fixup typo)
        items[2].action = RebaseAction::Fixup;
    }

    app.submit_modal().unwrap();
    assert!(!SequencerState::is_active(&git_dir));

    // Reload repository
    app.load_repository(&git_dir).unwrap();

    // Files from all three commits should exist in worktree
    assert!(repo_dir.join("part1.txt").exists());
    assert!(repo_dir.join("part2.txt").exists());
    assert!(repo_dir.join("part3.txt").exists());

    // There should now be only 2 commits total: initial commit and the squashed commit
    assert_eq!(app.commits.len(), 2);
    let squashed_commit = &app.commits[0];
    // Squashed commit message should include feature base and squash details, but not fixup typo
    assert!(squashed_commit.full_message.contains("feature base"));
    assert!(squashed_commit.full_message.contains("squash details"));
    assert!(!squashed_commit.full_message.contains("fixup typo"));
}

#[test]
fn test_packet4_interactive_rebase_reword() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    fs::write(repo_dir.join("doc.txt"), "doc content\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "typo msg"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    app.select_panel(Panel::Commits);
    // Select the commit with typo (index 0)
    app.commits_selected = 0;
    app.open_rebase_todo_modal();

    if let ActiveModal::RebaseTodo { items, .. } = &mut app.active_modal {
        assert_eq!(items.len(), 1);
        items[0].action = RebaseAction::Reword;
        items[0].message = Some("corrected commit message\n".to_string());
    }

    app.submit_modal().unwrap();
    assert!(!SequencerState::is_active(&git_dir));

    app.load_repository(&git_dir).unwrap();
    assert_eq!(app.commits[0].summary, "corrected commit message");
}

#[test]
fn test_packet4_rebase_conflict_stop_continue_and_resolution() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // On main, write conflict.txt line: "main line"
    fs::write(repo_dir.join("conflict.txt"), "main line\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "main changes"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Checkout initial commit
    Command::new("git")
        .args(["checkout", "HEAD~1"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["checkout", "-b", "feature"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // On feature branch, write conflict.txt line: "feature line"
    fs::write(repo_dir.join("conflict.txt"), "feature line\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "feature changes"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // Find the main commit OID
    let main_oid = {
        let store = RefStore::new(&git_dir);
        store.read_ref("refs/heads/main").unwrap()
    };

    // Prepare sequencer directly to rebase feature onto main
    let feat_oid = get_head_oid(&git_dir);
    let todo_items = vec![RebaseTodoItem::new(
        RebaseAction::Pick,
        feat_oid,
        "feature changes".to_string(),
    )];

    let outcome = oxidize_tui::ops::start_interactive_rebase(
        repo_dir, &git_dir, &git_dir, &main_oid, todo_items,
    )
    .unwrap();

    // Verify it stopped with conflict
    match outcome {
        oxidize_tui::ops::ReplayStepOutcome::Conflict { conflicted_paths } => {
            assert!(conflicted_paths.contains(&"conflict.txt".to_string()));
        }
        _ => panic!("Expected conflict outcome"),
    }

    // Verify sequencer state exists on disk
    assert!(SequencerState::is_active(&git_dir));
    let seq = SequencerState::load(&git_dir).unwrap().unwrap();
    assert_eq!(seq.status, SequencerStatus::Conflicted);

    // Refresh app
    app.load_repository(&git_dir).unwrap();
    assert!(app.sequencer_state.is_some());

    // Check files panel: conflict.txt should have FileStatusKind::Conflicted
    let file = app
        .files
        .iter()
        .find(|f| f.path == "conflict.txt")
        .expect("conflict.txt not found");
    assert_eq!(file.kind, FileStatusKind::Conflicted);

    // Conflict markers should exist in worktree file
    let content = fs::read_to_string(repo_dir.join("conflict.txt")).unwrap();
    assert!(content.contains("<<<<<<<"));
    assert!(content.contains(">>>>>>>"));

    // Attempt rebase_continue while conflicts remain -> should report error
    app.rebase_continue().unwrap();
    assert!(
        app.status_message
            .as_ref()
            .unwrap()
            .contains("Cannot continue")
            || app.status_message.as_ref().unwrap().contains("conflict")
    );

    // Resolve conflict using "Ours" choice via app method
    let conflict_file_idx = app
        .files
        .iter()
        .position(|f| f.path == "conflict.txt")
        .unwrap();
    app.files_selected = conflict_file_idx;
    app.resolve_selected_file_conflict(ConflictChoice::Ours)
        .unwrap();

    // Verify resolved content in working tree equals "main line\n" (ours during rebase onto main)
    let resolved_content = fs::read_to_string(repo_dir.join("conflict.txt")).unwrap();
    assert_eq!(resolved_content, "main line\n");

    // Now continue rebase
    app.rebase_continue().unwrap();

    // Verify rebase completed
    assert!(!SequencerState::is_active(&git_dir));
    assert!(app.sequencer_state.is_none());
}

#[test]
fn test_packet4_rebase_abort_workflow() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    fs::write(repo_dir.join("file.txt"), "main 1\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "main 1"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Branch feature from initial commit
    Command::new("git")
        .args(["checkout", "HEAD~1"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["checkout", "-b", "feature"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    fs::write(repo_dir.join("file.txt"), "feature 1\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "feature 1"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let orig_feature_head = get_head_oid(&git_dir);

    let main_oid = {
        let store = RefStore::new(&git_dir);
        store.read_ref("refs/heads/main").unwrap()
    };

    // Start rebase producing conflict
    let todo_items = vec![RebaseTodoItem::new(
        RebaseAction::Pick,
        orig_feature_head,
        "feature 1".to_string(),
    )];
    let _ = oxidize_tui::ops::start_interactive_rebase(
        repo_dir, &git_dir, &git_dir, &main_oid, todo_items,
    )
    .unwrap();

    assert!(SequencerState::is_active(&git_dir));

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    assert!(app.sequencer_state.is_some());

    // Prompt rebase abort
    app.prompt_rebase_abort();
    match &app.active_modal {
        ActiveModal::Confirm { action, .. } => {
            assert_eq!(*action, ConfirmAction::RebaseAbort);
        }
        _ => panic!("Expected Confirm modal with RebaseAbort"),
    }

    // Submit confirm
    app.submit_modal().unwrap();

    // Verify rebase was aborted
    assert!(!SequencerState::is_active(&git_dir));
    assert!(app.sequencer_state.is_none());

    // Verify HEAD is back to original feature branch
    let cur_head = get_head_oid(&git_dir);
    assert_eq!(cur_head, orig_feature_head);

    // Working tree file should match feature 1
    assert_eq!(
        fs::read_to_string(repo_dir.join("file.txt")).unwrap(),
        "feature 1\n"
    );
}

#[test]
fn test_packet4_conflict_resolution_theirs_and_both() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // Commit base
    fs::write(repo_dir.join("doc.txt"), "line 1\nline 2\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "base doc"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let base_commit = get_head_oid(&git_dir);

    // Ours: change line 1
    fs::write(repo_dir.join("doc.txt"), "line 1 ours\nline 2\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "ours"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    let our_commit = get_head_oid(&git_dir);

    // Create branch theirs from base
    Command::new("git")
        .args(["checkout", "-b", "theirs-branch", &base_commit.to_string()])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    fs::write(repo_dir.join("doc.txt"), "line 1 theirs\nline 2\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "theirs"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    let their_commit = get_head_oid(&git_dir);

    // Checkout back to our branch
    Command::new("git")
        .args(["checkout", "main"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Merge their commit into index with conflicts
    let mut index = oxidize_index::Index::load_from(git_dir.join("index")).unwrap();
    let store = RepoObjectStore::open_with_common_dir(&git_dir, &git_dir).unwrap();
    let base_tree = match store.read_object(&base_commit).unwrap() {
        Object::Commit(c) => c.tree,
        _ => panic!(),
    };
    let our_tree = match store.read_object(&our_commit).unwrap() {
        Object::Commit(c) => c.tree,
        _ => panic!(),
    };
    let their_tree = match store.read_object(&their_commit).unwrap() {
        Object::Commit(c) => c.tree,
        _ => panic!(),
    };

    let conflicted = oxidize_tui::ops::merge_trees_into_index_and_worktree(
        repo_dir,
        &git_dir,
        &git_dir,
        &mut index,
        &base_tree,
        &our_tree,
        &their_tree,
        "HEAD",
        "theirs",
    )
    .unwrap();
    assert_eq!(conflicted, vec!["doc.txt"]);
    index.write_to(git_dir.join("index")).unwrap();

    // Verify index has conflict stages 1, 2, 3
    let entries = index.entries();
    let doc_entries: Vec<_> = entries.iter().filter(|e| e.path == "doc.txt").collect();
    assert_eq!(doc_entries.len(), 3);

    // Test resolving with "Theirs"
    oxidize_tui::ops::resolve_conflict_choice(
        repo_dir,
        &git_dir,
        &git_dir,
        "doc.txt",
        ConflictChoice::Theirs,
    )
    .unwrap();

    // File should contain theirs
    assert_eq!(
        fs::read_to_string(repo_dir.join("doc.txt")).unwrap(),
        "line 1 theirs\nline 2\n"
    );

    // Index should now only have stage 0 for doc.txt
    let idx_after = oxidize_index::Index::load_from(git_dir.join("index")).unwrap();
    let doc_after: Vec<_> = idx_after
        .entries()
        .iter()
        .filter(|e| e.path == "doc.txt")
        .collect();
    assert_eq!(doc_after.len(), 1);
    assert_eq!(doc_after[0].stage, 0);

    // Re-introduce conflict to test "Both"
    let _ = oxidize_tui::ops::merge_trees_into_index_and_worktree(
        repo_dir,
        &git_dir,
        &git_dir,
        &mut index,
        &base_tree,
        &our_tree,
        &their_tree,
        "HEAD",
        "theirs",
    )
    .unwrap();
    index.write_to(git_dir.join("index")).unwrap();

    oxidize_tui::ops::resolve_conflict_choice(
        repo_dir,
        &git_dir,
        &git_dir,
        "doc.txt",
        ConflictChoice::Both,
    )
    .unwrap();

    let both_content = fs::read_to_string(repo_dir.join("doc.txt")).unwrap();
    assert!(both_content.contains("line 1 ours"));
    assert!(both_content.contains("line 1 theirs"));
}

#[test]
fn test_packet4_commit_revert_workflow() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    fs::write(repo_dir.join("revert_me.txt"), "feature data\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "add revert_me file"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let commit_to_revert = get_head_oid(&git_dir);

    // Add another commit after it
    fs::write(repo_dir.join("keep_me.txt"), "keep data\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "keep this commit"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // Select the commit to revert (index 1: "add revert_me file")
    app.select_panel(Panel::Commits);
    app.commits_selected = 1;
    assert_eq!(app.commits[1].oid, commit_to_revert);

    // Prompt revert
    app.prompt_revert_selected_commit();
    match &app.active_modal {
        ActiveModal::Confirm { action, prompt, .. } => {
            assert_eq!(*action, ConfirmAction::Revert(commit_to_revert));
            assert!(prompt.contains("add revert_me file"));
        }
        _ => panic!("Expected Confirm modal with Revert"),
    }

    // Submit revert
    app.submit_modal().unwrap();

    // Verify revert commit was created on HEAD
    app.load_repository(&git_dir).unwrap();
    assert_eq!(app.commits[0].summary, "Revert \"add revert_me file\"");

    // The reverted file should no longer exist in worktree
    assert!(!repo_dir.join("revert_me.txt").exists());
    // The keep_me file should still exist
    assert!(repo_dir.join("keep_me.txt").exists());
}

#[test]
fn test_packet4_ui_rendering_with_test_backend() {
    let (tmp, git_dir) = create_base_repo();
    let _repo_dir = tmp.path();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // Set up mock sequencer state
    app.sequencer_state = Some(SequencerState {
        head_name: "refs/heads/main".to_string(),
        orig_head: ObjectId::ZERO,
        onto: ObjectId::ZERO,
        current_step: 2,
        total_steps: 5,
        todo: vec![RebaseTodoItem::new(
            RebaseAction::Pick,
            ObjectId::ZERO,
            "third step".to_string(),
        )],
        done: vec![RebaseTodoItem::new(
            RebaseAction::Pick,
            ObjectId::ZERO,
            "first step".to_string(),
        )],
        stopped_sha: None,
        status: SequencerStatus::Conflicted,
    });

    // Test backend 160x30
    let backend = TestBackend::new(160, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| {
            oxidize_tui::ui::render(f, &app);
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let mut rendered_text = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            rendered_text.push_str(buffer[(x, y)].symbol());
        }
        rendered_text.push('\n');
    }
    // Verify rebase status displayed in Status panel
    assert!(rendered_text.contains("step 2/5 (conflicted)"));
    // Verify footer actions include Continue Rebase and Abort
    assert!(rendered_text.contains("Continue Rebase"));
    assert!(rendered_text.contains("Abort"));

    // Now open RebaseTodo modal and verify modal rendering
    app.active_modal = ActiveModal::RebaseTodo {
        items: vec![
            RebaseTodoItem::new(RebaseAction::Pick, ObjectId::ZERO, "Step Alpha".to_string()),
            RebaseTodoItem::new(
                RebaseAction::Squash,
                ObjectId::ZERO,
                "Step Beta".to_string(),
            ),
        ],
        selected: 0,
        onto_oid: ObjectId::ZERO,
    };

    terminal
        .draw(|f| {
            oxidize_tui::ui::render(f, &app);
        })
        .unwrap();

    let modal_buffer = terminal.backend().buffer();
    let mut modal_text = String::new();
    for y in 0..modal_buffer.area.height {
        for x in 0..modal_buffer.area.width {
            modal_text.push_str(modal_buffer[(x, y)].symbol());
        }
        modal_text.push('\n');
    }

    assert!(modal_text.contains("Interactive Rebase"));
    assert!(modal_text.contains("Step Alpha"));
    assert!(modal_text.contains("Step Beta"));
    assert!(modal_text.contains("pick"));
    assert!(modal_text.contains("squash"));
}

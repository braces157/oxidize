//! Packet 5 Comprehensive Integration Tests:
//! - J03, J04, J07: Custom Patches, Persistent Patch Basket, Worktree/Index Patch Apply
//! - K01-K08: Stash Variants (include untracked, staged-only, keep-index), Stash Branching
//! - M01-M04: Linked Worktree Management (pure Rust creation, listing, switching, lock-protected removal)
//! - Headless Ratatui UI rendering & key navigation across all Packet 5 modals

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

use oxidize_core::object::Object;
use oxidize_diff::{compute_structured_diff, CustomPatchBasket, StructuredHunk};
use oxidize_index::Index;
use oxidize_pack::store::RepoObjectStore;
use oxidize_refs::RefStore;
use oxidize_tui::model::{ActiveModal, Panel};
use oxidize_tui::ops::{self, StashSaveOptions, WorktreeItem};
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

    fs::write(repo_dir.join("base.txt"), "line 1\nline 2\nline 3\n").unwrap();
    fs::write(repo_dir.join("other.txt"), "other alpha\nother beta\n").unwrap();
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

    (tmp, repo_dir)
}

#[test]
fn test_packet5_untracked_stash_save_and_apply() {
    let (_tmp, repo_root) = create_base_repo();
    let git_dir = repo_root.join(".git");

    // Modify tracked file
    fs::write(
        repo_root.join("base.txt"),
        "line 1\nmodified line 2\nline 3\n",
    )
    .unwrap();

    // Create untracked file
    fs::write(repo_root.join("untracked.txt"), "untracked file content\n").unwrap();
    assert!(repo_root.join("untracked.txt").exists());

    // Save stash with include_untracked = true
    let opts = StashSaveOptions {
        message: "stash with untracked".to_string(),
        include_untracked: true,
        staged_only: false,
        keep_index: false,
    };
    let stash_oid = ops::stash_save_with_options(&repo_root, &git_dir, &opts).unwrap();

    // 1. Untracked file should be deleted from working tree
    assert!(
        !repo_root.join("untracked.txt").exists(),
        "untracked.txt should be cleaned from WT"
    );

    // 2. Tracked file should be reverted to HEAD
    let base_content = fs::read_to_string(repo_root.join("base.txt")).unwrap();
    assert_eq!(base_content, "line 1\nline 2\nline 3\n");

    // 3. Stash commit should have 3 parents (HEAD, index, untracked)
    let store = RepoObjectStore::open(&git_dir).unwrap();
    if let Ok(Object::Commit(c)) = store.read_object(&stash_oid) {
        assert_eq!(
            c.parents.len(),
            3,
            "Stash commit with untracked files must have 3 parents"
        );
    } else {
        panic!("Stash OID is not a valid commit");
    }

    // 4. Apply stash -> should restore untracked file and modified base.txt
    let clean = ops::apply_stash(&repo_root, &git_dir, &stash_oid).unwrap();
    assert!(clean, "Stash application should be clean");

    assert!(
        repo_root.join("untracked.txt").exists(),
        "untracked.txt should be restored"
    );
    let restored_untracked = fs::read_to_string(repo_root.join("untracked.txt")).unwrap();
    assert_eq!(restored_untracked, "untracked file content\n");

    let restored_base = fs::read_to_string(repo_root.join("base.txt")).unwrap();
    assert_eq!(restored_base, "line 1\nmodified line 2\nline 3\n");
}

#[test]
fn test_packet5_staged_only_stash_save() {
    let (_tmp, repo_root) = create_base_repo();
    let git_dir = repo_root.join(".git");

    // Stage a change to base.txt
    fs::write(
        repo_root.join("base.txt"),
        "line 1 - staged change\nline 2\nline 3\n",
    )
    .unwrap();
    ops::stage_path(&repo_root, &git_dir, "base.txt").unwrap();

    // Make an unstaged change to other.txt
    fs::write(
        repo_root.join("other.txt"),
        "other alpha - unstaged WT change\nother beta\n",
    )
    .unwrap();

    // Save stash with staged_only = true
    let opts = StashSaveOptions {
        message: "staged only stash".to_string(),
        include_untracked: false,
        staged_only: true,
        keep_index: false,
    };
    let stash_oid = ops::stash_save_with_options(&repo_root, &git_dir, &opts).unwrap();

    // 1. Unstaged changes in other.txt must be preserved
    let other_content = fs::read_to_string(repo_root.join("other.txt")).unwrap();
    assert_eq!(
        other_content,
        "other alpha - unstaged WT change\nother beta\n"
    );

    // 2. base.txt in WT should be reverted to HEAD
    let base_content = fs::read_to_string(repo_root.join("base.txt")).unwrap();
    assert_eq!(base_content, "line 1\nline 2\nline 3\n");

    // 3. Index should be clean relative to HEAD
    let index = Index::load_from(git_dir.join("index")).unwrap();
    let base_entry = index.get_entry("base.txt").unwrap();
    let store = RepoObjectStore::open(&git_dir).unwrap();
    let ref_store = RefStore::new(&git_dir);
    let (_, head_oid) = ref_store.resolve_head().unwrap();
    if let Ok(Object::Commit(hc)) = store.read_object(&head_oid.unwrap()) {
        if let Ok(Object::Tree(ht)) = store.read_object(&hc.tree) {
            let tree_entry = ht.entries.iter().find(|e| e.name == "base.txt").unwrap();
            assert_eq!(
                base_entry.oid, tree_entry.id,
                "Index must match HEAD tree for staged_only stash"
            );
        }
    }

    // 4. Applying the stash restores the staged change
    let clean = ops::apply_stash(&repo_root, &git_dir, &stash_oid).unwrap();
    assert!(clean);
    let re_base = fs::read_to_string(repo_root.join("base.txt")).unwrap();
    assert_eq!(re_base, "line 1 - staged change\nline 2\nline 3\n");
}

#[test]
fn test_packet5_keep_index_stash_save() {
    let (_tmp, repo_root) = create_base_repo();
    let git_dir = repo_root.join(".git");

    // Stage a change to base.txt
    fs::write(
        repo_root.join("base.txt"),
        "line 1\nline 2 - staged change\nline 3\n",
    )
    .unwrap();
    ops::stage_path(&repo_root, &git_dir, "base.txt").unwrap();

    let index_before = Index::load_from(git_dir.join("index")).unwrap();
    let staged_oid_before = index_before.get_entry("base.txt").unwrap().oid;

    // Unstaged change in other.txt
    fs::write(
        repo_root.join("other.txt"),
        "other alpha modified in WT\nother beta\n",
    )
    .unwrap();

    // Save stash with keep_index = true
    let opts = StashSaveOptions {
        message: "keep index stash".to_string(),
        include_untracked: false,
        staged_only: false,
        keep_index: true,
    };
    let _stash_oid = ops::stash_save_with_options(&repo_root, &git_dir, &opts).unwrap();

    // 1. Working tree has the staged change still applied
    let base_content = fs::read_to_string(repo_root.join("base.txt")).unwrap();
    assert_eq!(base_content, "line 1\nline 2 - staged change\nline 3\n");

    // 2. Index still contains the staged entry!
    let index_after = Index::load_from(git_dir.join("index")).unwrap();
    let staged_oid_after = index_after.get_entry("base.txt").unwrap().oid;
    assert_eq!(
        staged_oid_before, staged_oid_after,
        "Index entry must be retained with keep_index: true"
    );

    // 3. Unstaged change in other.txt was reverted to HEAD
    let other_content = fs::read_to_string(repo_root.join("other.txt")).unwrap();
    assert_eq!(other_content, "other alpha\nother beta\n");
}

#[test]
fn test_packet5_stash_branch_workflow() {
    let (_tmp, repo_root) = create_base_repo();
    let git_dir = repo_root.join(".git");

    // Make change and stash it
    fs::write(
        repo_root.join("base.txt"),
        "line 1\nline 2 - stashed for branch\nline 3\n",
    )
    .unwrap();
    let opts = StashSaveOptions {
        message: "wip for new branch".to_string(),
        include_untracked: false,
        staged_only: false,
        keep_index: false,
    };
    ops::stash_save_with_options(&repo_root, &git_dir, &opts).unwrap();

    let stashes = ops::list_stashes(&git_dir).unwrap();
    assert_eq!(stashes.len(), 1);

    // Call stash_branch
    ops::stash_branch(&repo_root, &git_dir, 0, "feature-from-stash").unwrap();

    // 1. Ref store HEAD should now be feature-from-stash
    let ref_store = RefStore::new(&git_dir);
    let (head_ref, _) = ref_store.resolve_head().unwrap();
    assert_eq!(head_ref, "feature-from-stash");

    // 2. Working tree should contain the stashed change
    let content = fs::read_to_string(repo_root.join("base.txt")).unwrap();
    assert_eq!(content, "line 1\nline 2 - stashed for branch\nline 3\n");

    // 3. Stash was automatically dropped on clean application
    let remaining = ops::list_stashes(&git_dir).unwrap();
    assert!(
        remaining.is_empty(),
        "Stash must be dropped after branching"
    );
}

#[test]
fn test_packet5_custom_patch_basket_operations() {
    let (_tmp, repo_root) = create_base_repo();
    let git_dir = repo_root.join(".git");

    let old_text = "line 1\nline 2\nline 3\n";
    let new_text = "line 1\nline 2 modified\nline 3\n";
    let hunks = compute_structured_diff(old_text, new_text, 1);
    assert_eq!(hunks.len(), 1);

    let mut basket = CustomPatchBasket::new();
    assert!(basket.is_empty());
    assert!(basket.add_hunk("base.txt", hunks[0].clone()));
    assert_eq!(basket.len(), 1);

    // Duplicate hunk is prevented
    assert!(!basket.add_hunk("base.txt", hunks[0].clone()));
    assert_eq!(basket.len(), 1);

    // Check membership
    assert!(basket.contains_hunk("base.txt", &hunks[0]));

    // Format patch string
    let patch = basket.format_patch();
    assert!(patch.contains("diff --git a/base.txt b/base.txt"));
    assert!(patch.contains("+line 2 modified"));

    // Forward and reverse text application
    let applied = basket.apply_to_text("base.txt", old_text, false).unwrap();
    assert_eq!(applied, new_text);
    let reverted = basket.apply_to_text("base.txt", &applied, true).unwrap();
    assert_eq!(reverted, old_text);

    // Apply custom patch to worktree
    ops::apply_custom_patch_to_worktree(&repo_root, &basket, false).unwrap();
    let wt_content = fs::read_to_string(repo_root.join("base.txt")).unwrap();
    assert_eq!(wt_content, new_text);

    // Revert custom patch from worktree
    ops::apply_custom_patch_to_worktree(&repo_root, &basket, true).unwrap();
    let wt_reverted = fs::read_to_string(repo_root.join("base.txt")).unwrap();
    assert_eq!(wt_reverted, old_text);

    // Apply custom patch to index
    ops::apply_custom_patch_to_index(&repo_root, &git_dir, &basket, false).unwrap();
    let index = Index::load_from(git_dir.join("index")).unwrap();
    let store = RepoObjectStore::open(&git_dir).unwrap();
    let entry = index.get_entry("base.txt").unwrap();
    if let Ok(Object::Blob(b)) = store.read_object(&entry.oid) {
        assert_eq!(String::from_utf8_lossy(&b.data), new_text);
    }

    // Create commit directly from custom patch
    let commit_oid = ops::create_commit_from_custom_patch(
        &repo_root,
        &git_dir,
        &basket,
        "commit from custom patch",
    )
    .unwrap();

    let ref_store = RefStore::new(&git_dir);
    let (_, head_oid) = ref_store.resolve_head().unwrap();
    assert_eq!(head_oid, Some(commit_oid));
}

#[test]
fn test_packet5_linked_worktree_management() {
    let (_tmp, repo_root) = create_base_repo();
    let git_dir = repo_root.join(".git");
    let common_dir = git_dir.clone();

    // 1. Initial worktree listing has main worktree
    let worktrees = ops::list_worktrees(&git_dir, &common_dir, Some(&repo_root)).unwrap();
    assert_eq!(worktrees.len(), 1);
    assert!(worktrees[0].is_main);

    // 2. Create linked worktree
    let wt_path = repo_root.join("linked-wt-dir");
    let item = ops::create_worktree(
        &repo_root,
        &git_dir,
        &common_dir,
        &wt_path,
        "feature-linked-wt",
        true,
    )
    .unwrap();

    assert_eq!(item.name, "linked-wt-dir");
    assert_eq!(item.head_ref, "feature-linked-wt");
    assert!(!item.is_main);

    // Check .git file in linked worktree
    let dot_git = wt_path.join(".git");
    assert!(dot_git.is_file());
    let git_link = fs::read_to_string(&dot_git).unwrap();
    assert!(git_link.starts_with("gitdir:"));

    // Check checked out files in linked worktree
    assert!(wt_path.join("base.txt").exists());
    assert!(wt_path.join("other.txt").exists());

    // 3. Listing now returns 2 worktrees
    let worktrees_after = ops::list_worktrees(&git_dir, &common_dir, Some(&repo_root)).unwrap();
    assert_eq!(worktrees_after.len(), 2);
    let linked = worktrees_after.iter().find(|w| !w.is_main).unwrap();
    assert_eq!(linked.name, "linked-wt-dir");
    assert_eq!(linked.head_ref, "feature-linked-wt");

    // 4. Lock protection test: lock worktree
    let wt_admin_dir = common_dir.join("worktrees").join("linked-wt-dir");
    fs::write(wt_admin_dir.join("locked"), "investigating bug\n").unwrap();

    let locked_list = ops::list_worktrees(&git_dir, &common_dir, Some(&repo_root)).unwrap();
    let locked_wt = locked_list.iter().find(|w| !w.is_main).unwrap();
    assert!(locked_wt.is_locked);
    assert_eq!(locked_wt.lock_reason.as_deref(), Some("investigating bug"));

    // Attempt remove without force -> must fail
    let err = ops::remove_worktree(&common_dir, "linked-wt-dir", false);
    assert!(
        err.is_err(),
        "Removing locked worktree without force must fail"
    );

    // Remove with force -> must succeed
    ops::remove_worktree(&common_dir, "linked-wt-dir", true).unwrap();
    assert!(!wt_path.exists(), "Worktree directory must be removed");
    assert!(!wt_admin_dir.exists(), "Worktree admin dir must be removed");

    let final_list = ops::list_worktrees(&git_dir, &common_dir, Some(&repo_root)).unwrap();
    assert_eq!(final_list.len(), 1);
}

#[test]
fn test_packet5_app_worktree_and_patch_modals() {
    let (_tmp, repo_root) = create_base_repo();
    let git_dir = repo_root.join(".git");

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // 1. Worktree List modal and Add modal
    app.open_worktree_list_modal();
    match &app.active_modal {
        ActiveModal::WorktreeList { items, selected } => {
            assert_eq!(items.len(), 1);
            assert_eq!(*selected, 0);
        }
        _ => panic!("Expected WorktreeList modal"),
    }

    // Open Worktree Add modal
    app.open_worktree_add_modal();
    match &app.active_modal {
        ActiveModal::WorktreeAdd {
            create_branch,
            focused_field,
            ..
        } => {
            assert!(*create_branch);
            assert_eq!(*focused_field, 0);
        }
        _ => panic!("Expected WorktreeAdd modal"),
    }

    // Field cycling in WorktreeAdd
    app.handle_worktree_add_key(crossterm::event::KeyCode::Tab);
    if let ActiveModal::WorktreeAdd { focused_field, .. } = app.active_modal {
        assert_eq!(focused_field, 1);
    }
    app.handle_worktree_add_key(crossterm::event::KeyCode::Tab);
    if let ActiveModal::WorktreeAdd { focused_field, .. } = app.active_modal {
        assert_eq!(focused_field, 2);
    }
    // Space toggles create_branch
    app.handle_worktree_add_key(crossterm::event::KeyCode::Char(' '));
    if let ActiveModal::WorktreeAdd { create_branch, .. } = app.active_modal {
        assert!(!create_branch);
    }

    // 2. Custom Patch Menu modal
    app.open_custom_patch_menu();
    match &app.active_modal {
        ActiveModal::CustomPatchMenu { selected } => {
            assert_eq!(*selected, 0);
        }
        _ => panic!("Expected CustomPatchMenu modal"),
    }
    app.handle_custom_patch_menu_key(crossterm::event::KeyCode::Down);
    if let ActiveModal::CustomPatchMenu { selected } = app.active_modal {
        assert_eq!(selected, 1);
    }

    // 3. Stash Save options modal
    app.open_stash_save_modal();
    match &app.active_modal {
        ActiveModal::StashSave {
            include_untracked,
            staged_only,
            keep_index,
            focused_field,
            ..
        } => {
            assert!(!include_untracked);
            assert!(!staged_only);
            assert!(!keep_index);
            assert_eq!(*focused_field, 0);
        }
        _ => panic!("Expected StashSave modal"),
    }
    // Tab to include_untracked checkbox
    app.handle_stash_save_key(crossterm::event::KeyCode::Tab);
    if let ActiveModal::StashSave { focused_field, .. } = app.active_modal {
        assert_eq!(focused_field, 1);
    }
    // Space toggles include_untracked
    app.handle_stash_save_key(crossterm::event::KeyCode::Char(' '));
    if let ActiveModal::StashSave {
        include_untracked, ..
    } = app.active_modal
    {
        assert!(include_untracked);
    }

    // 4. Stash Branch modal
    // First make a change and save stash
    fs::write(repo_root.join("base.txt"), "stashed content for modal\n").unwrap();
    app.active_modal = ActiveModal::None;
    let _ = ops::stash_save(&repo_root, &git_dir, "branch stash test");
    app.refresh().unwrap();
    app.active_panel = Panel::Stash;
    app.stashes_selected = 0;

    app.open_stash_branch_modal();
    match &app.active_modal {
        ActiveModal::StashBranch {
            stash_idx,
            branch_name,
            ..
        } => {
            assert_eq!(*stash_idx, 0);
            assert_eq!(branch_name, "stash-0");
        }
        _ => panic!("Expected StashBranch modal"),
    }

    // 5. Switch worktree in App
    let wt_path = repo_root.join("app-switch-wt");
    ops::create_worktree(
        &repo_root,
        &git_dir,
        &git_dir,
        &wt_path,
        "app-switch-branch",
        true,
    )
    .unwrap();

    app.switch_worktree(&wt_path).unwrap();
    assert_eq!(app.branch_name, "app-switch-branch");
    assert_eq!(app.repo_root, wt_path);
}

#[test]
fn test_packet5_ui_rendering_headless() {
    let (_tmp, repo_root) = create_base_repo();
    let git_dir = repo_root.join(".git");

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();

    // 1. Render CustomPatchMenu
    app.open_custom_patch_menu();
    terminal
        .draw(|f| {
            oxidize_tui::ui::render(f, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(
        content.contains("Custom Patch Menu"),
        "CustomPatchMenu title must be rendered"
    );
    assert!(content.contains("Apply custom patch to working tree"));

    // 2. Render WorktreeList
    let wt_item = WorktreeItem {
        name: "main".to_string(),
        path: repo_root.clone(),
        head_ref: "main".to_string(),
        head_oid: None,
        is_main: true,
        is_locked: false,
        lock_reason: None,
        is_prunable: false,
    };
    app.active_modal = ActiveModal::WorktreeList {
        items: vec![wt_item],
        selected: 0,
    };
    terminal
        .draw(|f| {
            oxidize_tui::ui::render(f, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(content.contains("Linked Worktrees"));
    assert!(content.contains("[MAIN]"));

    // 3. Render WorktreeAdd
    app.open_worktree_add_modal();
    terminal
        .draw(|f| {
            oxidize_tui::ui::render(f, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(content.contains("Add Linked Worktree"));
    assert!(content.contains("Create new branch"));

    // 4. Render StashSave with options
    app.active_modal = ActiveModal::StashSave {
        message: "my test stash".to_string(),
        cursor: 13,
        include_untracked: true,
        staged_only: false,
        keep_index: true,
        focused_field: 1,
    };
    terminal
        .draw(|f| {
            oxidize_tui::ui::render(f, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(content.contains("Stash Working Directory Changes"));
    assert!(content.contains("[x] Include Untracked"));
    assert!(content.contains("[x] Keep Index"));

    // 5. Render StashBranch
    app.active_modal = ActiveModal::StashBranch {
        stash_idx: 2,
        branch_name: "feature-stash-branch".to_string(),
        cursor: 20,
    };
    terminal
        .draw(|f| {
            oxidize_tui::ui::render(f, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(content.contains("Branch from stash@{2}"));
    assert!(content.contains("feature-stash-branch"));

    // 6. Render Inspector with basket count
    app.active_modal = ActiveModal::None;
    app.custom_patch_basket.add_hunk(
        "file1.txt",
        StructuredHunk {
            old_start: 1,
            old_count: 1,
            new_start: 1,
            new_count: 1,
            lines: Vec::new(),
        },
    );
    terminal
        .draw(|f| {
            oxidize_tui::ui::render(f, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(content.contains("Basket: 1"));
}

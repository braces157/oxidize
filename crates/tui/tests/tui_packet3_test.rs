//! Packet 3 Integration Tests:
//! - Commit Graph DAG & Topological Heap Ordering
//! - Commit Branch/Tag/Remote Decorations
//! - Fast Commit Diffing with Tree Comparison
//! - Inspector Virtualization with Massive Diffs (>65,535 lines)
//! - Branch Rename Workflow (with .git/config updates)
//! - Native Fast-Forward Merge
//! - Three-Way Merge Cherry-Pick
//! - Native Git Reset (Soft, Mixed, Hard)

use oxidize_core::object::Object;
use oxidize_index::Index;
use oxidize_pack::store::RepoObjectStore;
use oxidize_refs::RefStore;
use oxidize_tui::model::{
    ActiveModal, BranchesTab, CommitDecoration, ConfirmAction, DiffLine, DiffLineKind, DiffView,
    Panel, ResetMode,
};
use oxidize_tui::App;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
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

    fs::write(repo_dir.join("initial.txt"), "initial line\n").unwrap();

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
fn test_packet3_commit_dag_lanes_and_topological_order() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // Create branch feature-1 with a commit
    Command::new("git")
        .args(["checkout", "-b", "feature-1"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    fs::write(repo_dir.join("feat1.txt"), "feature 1 line\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "feature 1 commit"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Switch back to main and make a commit
    Command::new("git")
        .args(["checkout", "main"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    fs::write(repo_dir.join("main2.txt"), "main commit 2\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "main 2 commit"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Merge feature-1 into main
    Command::new("git")
        .args([
            "merge",
            "feature-1",
            "--no-ff",
            "-m",
            "merge feature-1 into main",
        ])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).expect("Must load repo");

    assert!(app.commits.len() >= 4, "Must contain all 4 commits");
    // Verify top commit is the merge commit
    assert_eq!(app.commits[0].summary, "merge feature-1 into main");

    // Verify merge commit has graph prefix or node
    assert!(
        !app.commits[0].graph_prefix.is_empty(),
        "Graph prefix must be populated"
    );

    // Verify all commits have valid short_oid and dates
    for c in &app.commits {
        assert_eq!(c.short_oid.len(), 7);
        assert!(!c.author.is_empty());
    }
}

#[test]
fn test_packet3_commit_decorations() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // Create tag on initial commit
    Command::new("git")
        .args(["tag", "v1.0.0"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Create and checkout branch dev
    Command::new("git")
        .args(["checkout", "-b", "dev"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    fs::write(repo_dir.join("dev.txt"), "dev work\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "dev commit"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Create tag on dev commit
    Command::new("git")
        .args(["tag", "v2.0.0"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Create fake remote tracking branch refs/remotes/origin/main pointing to initial commit
    let ref_store = RefStore::new(&git_dir);
    let main_oid = ref_store.read_ref("refs/heads/main").unwrap();
    ref_store
        .update_ref("refs/remotes/origin/main", &main_oid, None, "test remote")
        .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).expect("Must load repo");

    // HEAD is dev
    let top_commit = &app.commits[0];
    assert_eq!(top_commit.summary, "dev commit");
    assert!(
        top_commit
            .decorations
            .iter()
            .any(|d| matches!(d, CommitDecoration::Head(name) if name == "dev")),
        "Top commit must have HEAD -> dev decoration"
    );
    assert!(
        top_commit
            .decorations
            .iter()
            .any(|d| matches!(d, CommitDecoration::Tag(name) if name == "v2.0.0")),
        "Top commit must have tag: v2.0.0 decoration"
    );

    // Initial commit
    let init_commit = &app.commits[1];
    assert_eq!(init_commit.summary, "initial commit");
    assert!(
        init_commit
            .decorations
            .iter()
            .any(|d| matches!(d, CommitDecoration::Branch(name) if name == "main")),
        "Initial commit must have main branch decoration"
    );
    assert!(
        init_commit
            .decorations
            .iter()
            .any(|d| matches!(d, CommitDecoration::Remote(name) if name == "origin/main")),
        "Initial commit must have origin/main remote decoration"
    );
    assert!(
        init_commit
            .decorations
            .iter()
            .any(|d| matches!(d, CommitDecoration::Tag(name) if name == "v1.0.0")),
        "Initial commit must have v1.0.0 tag decoration"
    );
}

#[test]
fn test_packet3_commit_diff_tree_filtering() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // Add multiple files in commit 1
    fs::write(repo_dir.join("unchanged.txt"), "same content\n").unwrap();
    fs::write(repo_dir.join("modified.txt"), "v1\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "commit 1 with multiple files"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // In commit 2, modify ONLY modified.txt
    fs::write(repo_dir.join("modified.txt"), "v2 modified content\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "commit 2 modifying only one file"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).expect("Must load repo");

    let store = RepoObjectStore::open(&git_dir).unwrap();
    let top_commit = app.commits[0].clone();

    // Compute diff for commit 2
    let diff = app.compute_commit_diff(&store, &top_commit);
    let diff_text: String = diff.lines.iter().map(|l| l.content.as_str()).collect();

    // Verify unchanged.txt is NOT in the diff (tree skipped)
    assert!(
        !diff_text.contains("unchanged.txt"),
        "Unchanged file must be skipped by tree comparison"
    );
    assert!(
        diff_text.contains("[M] modified.txt"),
        "Modified file must be identified"
    );

    // Verify diff is cached
    assert!(app.commit_diff_cache.contains_key(&top_commit.oid));
}

#[test]
fn test_packet3_inspector_virtualization_massive_diff() {
    let mut app = App::new();

    // Create a DiffView with 70,000 lines (exceeds u16::MAX = 65,535)
    let total_lines = 70_000;
    let mut diff_lines = Vec::with_capacity(total_lines);
    for i in 0..total_lines {
        diff_lines.push(DiffLine {
            kind: DiffLineKind::Context,
            content: format!("Line number {}", i),
        });
    }

    let mut dv = DiffView::new("Huge Virtualized Diff");
    dv.lines = diff_lines;
    app.cached_diff = Some(dv);
    app.focus_inspector();

    // Scroll beyond u16::MAX
    app.inspector_scroll = 68_000;

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    // Drawing must succeed without panic and correctly virtualize lines
    terminal
        .draw(|f| {
            oxidize_tui::ui::render(f, &app);
        })
        .expect("Virtualized render must succeed for >65535 lines");
}

#[test]
fn test_packet3_branch_rename_workflow() {
    let (tmp, git_dir) = create_base_repo();
    let _repo_dir = tmp.path();

    // Setup .git/config with branch section
    let config_content = "[core]\n\trepositoryformatversion = 0\n[branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n";
    fs::write(git_dir.join("config"), config_content).unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).expect("Must load repo");
    app.select_panel(Panel::Branches);
    app.branches_tab = BranchesTab::Local;

    // Trigger rename modal
    app.prompt_rename_selected_branch();
    match &app.active_modal {
        ActiveModal::BranchRename {
            old_name, new_name, ..
        } => {
            assert_eq!(old_name, "main");
            assert_eq!(new_name, "main");
        }
        _ => panic!("Expected BranchRename modal"),
    }

    // Enter new branch name "trunk"
    app.active_modal = ActiveModal::BranchRename {
        old_name: "main".to_string(),
        new_name: "trunk".to_string(),
        cursor: 5,
    };

    app.submit_modal().expect("Submit branch rename");

    // Verify active branch is now trunk
    assert_eq!(app.branch_name, "trunk");
    let ref_store = RefStore::new(&git_dir);
    assert!(ref_store.read_ref("refs/heads/trunk").is_ok());
    assert!(ref_store.read_ref("refs/heads/main").is_err());

    // Verify .git/config updated
    let new_config = fs::read_to_string(git_dir.join("config")).unwrap();
    assert!(new_config.contains("[branch \"trunk\"]"));
    assert!(!new_config.contains("[branch \"main\"]"));
}

#[test]
fn test_packet3_fast_forward_merge() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // Create fast-feature branch and add a commit
    Command::new("git")
        .args(["checkout", "-b", "fast-feature"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    fs::write(repo_dir.join("feature.txt"), "feature data\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "feature commit"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Switch back to main
    Command::new("git")
        .args(["checkout", "main"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).expect("Must load repo");
    app.select_panel(Panel::Branches);
    app.branches_tab = BranchesTab::Local;

    // Highlight fast-feature branch
    let ff_idx = app
        .branches
        .iter()
        .position(|b| b.name == "fast-feature")
        .expect("fast-feature must be listed");
    app.branches_selected = ff_idx;

    // Fast-forward merge
    app.fast_forward_selected_branch()
        .expect("Fast forward merge must succeed");

    // Verify HEAD is at feature commit and working tree contains feature.txt
    assert!(repo_dir.join("feature.txt").is_file());
    assert_eq!(
        fs::read_to_string(repo_dir.join("feature.txt")).unwrap(),
        "feature data\n"
    );

    let ref_store = RefStore::new(&git_dir);
    let main_oid = ref_store.read_ref("refs/heads/main").unwrap();
    let feat_oid = ref_store.read_ref("refs/heads/fast-feature").unwrap();
    assert_eq!(main_oid, feat_oid);
}

#[test]
fn test_packet3_cherry_pick_commit() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    // Create topic branch with a commit
    Command::new("git")
        .args(["checkout", "-b", "topic"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    fs::write(repo_dir.join("topic_file.txt"), "cherry picked content\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "topic commit to cherry pick"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Switch back to main and add a commit
    Command::new("git")
        .args(["checkout", "main"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    fs::write(repo_dir.join("main_work.txt"), "main work\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "main ongoing work"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).expect("Must load repo");
    app.select_panel(Panel::Commits);

    // Find the topic commit
    let topic_commit_idx = app
        .commits
        .iter()
        .position(|c| c.summary == "topic commit to cherry pick")
        .expect("Topic commit must be present in DAG");
    app.commits_selected = topic_commit_idx;

    // Prompt cherry-pick
    app.prompt_cherry_pick_selected_commit();
    match &app.active_modal {
        ActiveModal::Confirm { action, .. } => {
            assert!(matches!(action, ConfirmAction::CherryPick(_)));
        }
        _ => panic!("Expected confirmation modal for cherry-pick"),
    }

    // Submit cherry pick
    app.submit_modal()
        .expect("Cherry pick submission must succeed");

    // Verify cherry-picked file exists in working tree and index
    assert!(repo_dir.join("topic_file.txt").is_file());
    assert_eq!(
        fs::read_to_string(repo_dir.join("topic_file.txt")).unwrap(),
        "cherry picked content\n"
    );

    // Verify new commit created at HEAD
    let ref_store = RefStore::new(&git_dir);
    let (head_ref, head_oid) = ref_store.resolve_head().unwrap();
    assert_eq!(head_ref, "main");
    let store = RepoObjectStore::open(&git_dir).unwrap();
    if let Ok(Object::Commit(c)) = store.read_object(&head_oid.unwrap()) {
        assert_eq!(c.message, "topic commit to cherry pick\n");
    } else {
        panic!("HEAD must be a commit");
    }
}

#[test]
fn test_packet3_reset_soft() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    let ref_store = RefStore::new(&git_dir);
    let _c1_oid = ref_store.read_ref("refs/heads/main").unwrap();

    // Commit 2 (C2)
    fs::write(repo_dir.join("file2.txt"), "c2 content\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "commit 2"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    let c2_oid = ref_store.read_ref("refs/heads/main").unwrap();

    // Commit 3 (C3)
    fs::write(repo_dir.join("file3.txt"), "c3 content\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "commit 3"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).expect("Must load repo");
    app.select_panel(Panel::Commits);

    // Soft reset to C2: HEAD moves to C2, working tree and index preserve C3 changes
    app.active_modal = ActiveModal::Confirm {
        title: "Reset Soft".to_string(),
        prompt: "Reset".to_string(),
        action: ConfirmAction::ResetToCommit {
            target_oid: c2_oid,
            short_oid: c2_oid.to_string()[..7].to_string(),
            mode: ResetMode::Soft,
        },
    };
    app.submit_modal().expect("Soft reset must succeed");

    let current_head = ref_store.read_ref("refs/heads/main").unwrap();
    assert_eq!(current_head, c2_oid, "HEAD must move to C2");
    assert!(
        repo_dir.join("file3.txt").is_file(),
        "file3.txt must remain in WT"
    );

    // Index still has file3.txt
    let idx = Index::load_from(git_dir.join("index")).unwrap();
    assert!(
        idx.get_entry("file3.txt").is_some(),
        "file3.txt must remain staged in index"
    );
}

#[test]
fn test_packet3_reset_mixed() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    let ref_store = RefStore::new(&git_dir);
    let c1_oid = ref_store.read_ref("refs/heads/main").unwrap();

    // Commit 2 (C2)
    fs::write(repo_dir.join("file2.txt"), "c2 content\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "commit 2"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Commit 3 (C3)
    fs::write(repo_dir.join("file3.txt"), "c3 content\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "commit 3"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).expect("Must load repo");
    app.select_panel(Panel::Commits);

    // Mixed reset to C1: HEAD moves to C1, index reset to C1, files remain in WT
    app.active_modal = ActiveModal::Confirm {
        title: "Reset Mixed".to_string(),
        prompt: "Reset".to_string(),
        action: ConfirmAction::ResetToCommit {
            target_oid: c1_oid,
            short_oid: c1_oid.to_string()[..7].to_string(),
            mode: ResetMode::Mixed,
        },
    };
    app.submit_modal().expect("Mixed reset must succeed");

    let current_head = ref_store.read_ref("refs/heads/main").unwrap();
    assert_eq!(current_head, c1_oid, "HEAD must move to C1");
    assert!(
        repo_dir.join("file2.txt").is_file(),
        "file2.txt must remain in WT"
    );
    assert!(
        repo_dir.join("file3.txt").is_file(),
        "file3.txt must remain in WT"
    );

    // Index reset to C1: file2.txt and file3.txt are NOT in index
    let idx = Index::load_from(git_dir.join("index")).unwrap();
    assert!(
        idx.get_entry("file2.txt").is_none(),
        "file2.txt must not be in index"
    );
    assert!(
        idx.get_entry("file3.txt").is_none(),
        "file3.txt must not be in index"
    );
    assert!(
        idx.get_entry("initial.txt").is_some(),
        "initial.txt must be in index"
    );
}

#[test]
fn test_packet3_reset_hard() {
    let (tmp, git_dir) = create_base_repo();
    let repo_dir = tmp.path();

    let ref_store = RefStore::new(&git_dir);
    let c1_oid = ref_store.read_ref("refs/heads/main").unwrap();

    // Commit 2 (C2)
    fs::write(repo_dir.join("file2.txt"), "c2 content\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "commit 2"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Commit 3 (C3)
    fs::write(repo_dir.join("file3.txt"), "c3 content\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "commit 3"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).expect("Must load repo");
    app.select_panel(Panel::Commits);

    // Hard reset to C1: HEAD at C1, index reset to C1, and working tree cleaned to C1
    app.active_modal = ActiveModal::Confirm {
        title: "Reset Hard".to_string(),
        prompt: "Reset".to_string(),
        action: ConfirmAction::ResetToCommit {
            target_oid: c1_oid,
            short_oid: c1_oid.to_string()[..7].to_string(),
            mode: ResetMode::Hard,
        },
    };
    app.submit_modal().expect("Hard reset must succeed");

    let current_head = ref_store.read_ref("refs/heads/main").unwrap();
    assert_eq!(current_head, c1_oid, "HEAD must move to C1");

    assert!(
        !repo_dir.join("file2.txt").is_file(),
        "file2.txt must be deleted in Hard reset"
    );
    assert!(
        !repo_dir.join("file3.txt").is_file(),
        "file3.txt must be deleted in Hard reset"
    );
    assert!(
        repo_dir.join("initial.txt").is_file(),
        "initial.txt must remain in WT"
    );

    let idx = Index::load_from(git_dir.join("index")).unwrap();
    assert!(idx.get_entry("file2.txt").is_none());
    assert!(idx.get_entry("file3.txt").is_none());
    assert!(idx.get_entry("initial.txt").is_some());
}

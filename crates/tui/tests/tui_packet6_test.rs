//! Packet 6 Comprehensive Integration Tests:
//! - L01-L08: Remote management, push with force-with-lease, tag push, remote branch delete, fetch all
//! - M05-M08: Submodule discovery, init, update, nested enter and return navigation
//! - N01-N04: Git Bisect setup, good/bad marking, candidate navigation, culprit discovery, reset
//! - O01-O08: Command palette filtering & execution, remote provider links, yank attributes
//! - P01-P08: Headless Ratatui UI rendering & key navigation across all Packet 6 modals & bisect decorations

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

use oxidize_config::GitConfig;
use oxidize_core::ObjectId;
use oxidize_refs::RefStore;
use oxidize_tui::model::{ActiveModal, ConfirmAction, Panel};
use oxidize_tui::ops::{self, BisectState, ProviderUrls, SubmoduleItem};
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

fn create_bare_remote() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let remote_dir = tmp.path().to_path_buf();

    Command::new("git")
        .args(["init", "--bare", "-b", "main"])
        .current_dir(&remote_dir)
        .output()
        .unwrap();

    (tmp, remote_dir)
}

fn get_head_oid(git_dir: &Path) -> ObjectId {
    RefStore::new(git_dir).resolve_head().unwrap().1.unwrap()
}

fn make_commit(repo_dir: &Path, file: &str, content: &str, msg: &str) -> ObjectId {
    fs::write(repo_dir.join(file), content).unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", msg])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    get_head_oid(&repo_dir.join(".git"))
}

#[test]
fn test_packet6_remote_management_native() {
    let (_tmp, repo_root) = create_base_repo();
    let git_dir = repo_root.join(".git");

    // 1. Add remote natively
    ops::add_remote(&git_dir, "upstream", "https://github.com/test/upstream.git").unwrap();
    let config = GitConfig::load_from_file(git_dir.join("config")).unwrap();
    assert_eq!(
        config.get("remote", Some("upstream"), "url"),
        Some("https://github.com/test/upstream.git")
    );

    // 2. Rename remote natively
    ops::rename_remote(&git_dir, "upstream", "origin").unwrap();
    let config = GitConfig::load_from_file(git_dir.join("config")).unwrap();
    assert_eq!(config.get("remote", Some("upstream"), "url"), None);
    assert_eq!(
        config.get("remote", Some("origin"), "url"),
        Some("https://github.com/test/upstream.git")
    );

    // 3. Remove remote natively
    ops::remove_remote(&git_dir, "origin").unwrap();
    let config = GitConfig::load_from_file(git_dir.join("config")).unwrap();
    assert_eq!(config.get("remote", Some("origin"), "url"), None);

    // 4. Test adding remote via App modal submit
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    app.open_remote_add_modal();
    if let ActiveModal::RemoteAdd {
        ref mut name,
        ref mut url,
        ..
    } = app.active_modal
    {
        *name = "gitlab".to_string();
        *url = "git@gitlab.com:owner/project.git".to_string();
    }
    app.submit_modal().unwrap();
    assert!(app
        .status_message
        .as_ref()
        .unwrap()
        .contains("Added remote 'gitlab'"));
    let config = GitConfig::load_from_file(git_dir.join("config")).unwrap();
    assert_eq!(
        config.get("remote", Some("gitlab"), "url"),
        Some("git@gitlab.com:owner/project.git")
    );

    // 5. Test removing remote via Confirm dialog in App
    app.prompt_remove_remote("gitlab".to_string());
    if let ActiveModal::Confirm {
        action: ConfirmAction::DeleteRemote(ref name),
        ..
    } = app.active_modal
    {
        assert_eq!(name, "gitlab");
    } else {
        panic!("Expected Confirm modal for DeleteRemote");
    }
    app.submit_modal().unwrap();
    let config = GitConfig::load_from_file(git_dir.join("config")).unwrap();
    assert_eq!(config.get("remote", Some("gitlab"), "url"), None);
}

#[test]
fn test_packet6_push_force_with_lease_and_tag_and_delete_branch() {
    let (_tmp_repo, repo_root) = create_base_repo();
    let (_tmp_remote, remote_dir) = create_bare_remote();
    let git_dir = repo_root.join(".git");

    let remote_url = remote_dir.to_str().unwrap();
    ops::add_remote(&git_dir, "origin", remote_url).unwrap();

    // 1. Initial push to bare repo
    let ref_store = RefStore::new(&git_dir);
    let c1 = get_head_oid(&git_dir);

    let push_res = ops::push_to_remote_ext(
        &repo_root,
        &git_dir,
        Some("origin"),
        Some("main"),
        false,
        None,
    );
    assert!(
        push_res.is_ok(),
        "Initial push should succeed: {:?}",
        push_res
    );

    // 2. Make a second commit locally
    let c2 = make_commit(
        &repo_root,
        "base.txt",
        "line 1 updated\nline 2\nline 3\n",
        "second commit",
    );

    // 3. Test force-with-lease: mismatched lease should FAIL
    let fake_oid: ObjectId = "0123456789abcdef0123456789abcdef01234567".parse().unwrap();
    let stale_lease_res = ops::push_to_remote_ext(
        &repo_root,
        &git_dir,
        Some("origin"),
        Some("main"),
        false,
        Some(fake_oid),
    );
    assert!(
        stale_lease_res.is_err(),
        "Force-with-lease must reject when remote OID doesn't match expected lease"
    );

    // 4. Force-with-lease with CORRECT lease (c1) should succeed
    let correct_lease_res = ops::push_to_remote_ext(
        &repo_root,
        &git_dir,
        Some("origin"),
        Some("main"),
        false,
        Some(c1),
    );
    assert!(
        correct_lease_res.is_ok(),
        "Force-with-lease must succeed when lease matches: {:?}",
        correct_lease_res
    );

    // Verify remote branch now points to c2
    let remote_ref_store = RefStore::new(&remote_dir);
    let remote_main = remote_ref_store.read_ref("refs/heads/main").unwrap();
    assert_eq!(remote_main, c2);

    // 5. Test push tag to remote
    ref_store
        .update_ref("refs/tags/v1.0.0", &c2, None, "create tag")
        .unwrap();
    let tag_push_res = ops::push_tag_to_remote(&git_dir, Some("origin"), "v1.0.0");
    assert!(
        tag_push_res.is_ok(),
        "Tag push should succeed: {:?}",
        tag_push_res
    );
    let remote_tag = remote_ref_store.read_ref("refs/tags/v1.0.0").unwrap();
    assert_eq!(remote_tag, c2);

    // 6. Test delete remote branch
    // First push a branch to delete
    ref_store
        .update_ref("refs/heads/feature-temp", &c2, None, "create feature")
        .unwrap();
    ops::push_to_remote_ext(
        &repo_root,
        &git_dir,
        Some("origin"),
        Some("feature-temp"),
        false,
        None,
    )
    .unwrap();
    assert!(remote_ref_store.read_ref("refs/heads/feature-temp").is_ok());

    let del_res = ops::delete_remote_branch(&git_dir, Some("origin"), "feature-temp");
    assert!(
        del_res.is_ok(),
        "Delete remote branch should succeed: {:?}",
        del_res
    );
    assert!(
        remote_ref_store
            .read_ref("refs/heads/feature-temp")
            .is_err(),
        "Remote branch must be deleted"
    );

    // 7. Test fetch all remotes
    let fetch_res = ops::fetch_all_remotes(&git_dir, true);
    assert!(
        fetch_res.is_ok(),
        "Fetch all remotes should succeed: {:?}",
        fetch_res
    );
}

#[test]
fn test_packet6_submodule_discovery_init_update_and_navigation() {
    let (_tmp_main, repo_root) = create_base_repo();
    let (_tmp_sub, sub_repo_root) = create_base_repo();
    let git_dir = repo_root.join(".git");

    // Configure a submodule in .gitmodules
    let sub_url = sub_repo_root.to_str().unwrap().replace('\\', "/");
    let gitmodules_content = format!(
        "[submodule \"mysub\"]\n\tpath = mysub\n\turl = {}\n\tbranch = main\n",
        sub_url
    );
    fs::write(repo_root.join(".gitmodules"), &gitmodules_content).unwrap();

    // 1. Discover submodules
    let submodules = ops::list_submodules(&repo_root, &git_dir).unwrap();
    assert_eq!(submodules.len(), 1);
    assert_eq!(submodules[0].name, "mysub");
    assert_eq!(submodules[0].path, "mysub");
    assert_eq!(submodules[0].url, sub_url);
    assert!(!submodules[0].is_initialized);

    // 2. Submodule init
    ops::submodule_init(&repo_root, &git_dir, "mysub").unwrap();
    let config = GitConfig::load_from_file(git_dir.join("config")).unwrap();
    assert_eq!(
        config.get("submodule", Some("mysub"), "url"),
        Some(sub_url.as_str())
    );

    // 3. Submodule update (clones & checks out)
    ops::submodule_update(&repo_root, &git_dir, "mysub").unwrap();
    assert!(repo_root.join("mysub").join(".git").exists());
    assert!(repo_root.join("mysub").join("base.txt").exists());

    let updated_subs = ops::list_submodules(&repo_root, &git_dir).unwrap();
    assert_eq!(updated_subs.len(), 1);
    assert!(updated_subs[0].is_initialized);
    assert!(updated_subs[0].head_oid.is_some());

    // 4. Test App submodule navigation (enter and return)
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    assert_eq!(app.repo_root, repo_root);
    assert!(app.repo_history.is_empty());

    // Enter submodule
    app.enter_submodule("mysub").unwrap();
    assert_eq!(app.repo_root, repo_root.join("mysub"));
    assert_eq!(app.repo_history.len(), 1);
    assert_eq!(app.repo_history[0], repo_root);

    // Return to parent repository
    app.return_to_parent_repo().unwrap();
    assert_eq!(app.repo_root, repo_root);
    assert!(app.repo_history.is_empty());
}

#[test]
fn test_packet6_git_bisect_workflow() {
    let (_tmp, repo_root) = create_base_repo();
    let git_dir = repo_root.join(".git");

    // Create 4 commits:
    // C0: initial commit (good)
    // C1: add feature A
    // C2: bug introduced!
    // C3: add feature B (bad)
    let ref_store = RefStore::new(&git_dir);
    let c0 = get_head_oid(&git_dir);

    let c1 = make_commit(&repo_root, "featureA.txt", "feature A\n", "feature A");
    let c2 = make_commit(
        &repo_root,
        "bug.txt",
        "introduced bug here\n",
        "culprit commit",
    );
    let c3 = make_commit(&repo_root, "featureB.txt", "feature B\n", "feature B");

    // 1. Start bisect with C3 as bad and C0 as good
    let state = ops::bisect_start(&repo_root, &git_dir, Some(c3), Some(c0)).unwrap();
    assert!(state.is_active);
    assert_eq!(state.bad_oid, Some(c3));
    assert_eq!(state.good_oids, vec![c0]);
    assert!(state.remaining_steps > 0);

    // Head is now moved to midpoint candidate
    let (_, curr_head) = ref_store.resolve_head().unwrap();
    let mid1 = curr_head.unwrap();
    assert!(mid1 == c1 || mid1 == c2);

    // 2. If mid1 is c1, c1 is good; if mid1 is c2, c2 is bad.
    let state2 = if mid1 == c1 {
        // C1 is good -> mark good
        ops::bisect_mark(&repo_root, &git_dir, false).unwrap()
    } else {
        // C2 is bad -> mark bad
        ops::bisect_mark(&repo_root, &git_dir, true).unwrap()
    };

    // Head is now moved to the next candidate
    let (_, curr_head2) = ref_store.resolve_head().unwrap();
    let mid2 = curr_head2.unwrap();

    let _final_state = if mid2 == c1 {
        ops::bisect_mark(&repo_root, &git_dir, false).unwrap()
    } else if mid2 == c2 {
        ops::bisect_mark(&repo_root, &git_dir, true).unwrap()
    } else {
        state2
    };

    // The culprit must be c2
    let active_state = ops::get_bisect_state(&git_dir).unwrap();
    assert!(active_state.is_active);
    assert_eq!(
        active_state.culprit_oid,
        Some(c2),
        "Bisect must correctly identify culprit commit c2"
    );

    // 3. Reset bisect
    ops::bisect_reset(&repo_root, &git_dir).unwrap();
    let reset_state = ops::get_bisect_state(&git_dir).unwrap();
    assert!(!reset_state.is_active);
    assert!(!git_dir.join("BISECT_START").exists());

    // Head restored to main branch pointing to c3
    let (branch_name, final_head) = ref_store.resolve_head().unwrap();
    assert_eq!(branch_name, "main");
    assert_eq!(final_head, Some(c3));
}

#[test]
fn test_packet6_command_palette_and_filtering() {
    let (_tmp, repo_root) = create_base_repo();
    let git_dir = repo_root.join(".git");
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // 1. Open command palette
    app.open_command_palette();
    if let ActiveModal::CommandPalette { ref commands, .. } = app.active_modal {
        assert!(!commands.is_empty());
        assert!(commands.iter().any(|c| c.id == "push"));
        assert!(commands.iter().any(|c| c.id == "commit"));
        assert!(commands.iter().any(|c| c.id == "bisect"));
        assert!(commands.iter().any(|c| c.id == "submodules"));
    } else {
        panic!("Expected CommandPalette modal");
    }

    // 2. Type query "sub"
    app.handle_modal_char('s');
    app.handle_modal_char('u');
    app.handle_modal_char('b');
    if let ActiveModal::CommandPalette {
        ref commands,
        ref query,
        ..
    } = app.active_modal
    {
        assert_eq!(query, "sub");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].id, "submodules");
    } else {
        panic!("Expected CommandPalette modal");
    }

    // 3. Submit -> executes command to open submodules modal
    app.submit_modal().unwrap();
    if let ActiveModal::SubmoduleList { .. } = app.active_modal {
        // Success: executed the command from palette
    } else {
        panic!("Command palette failed to execute 'submodules'");
    }
}

#[test]
fn test_packet6_yank_and_provider_urls() {
    let (_tmp, repo_root) = create_base_repo();
    let git_dir = repo_root.join(".git");

    // 1. Test provider URLs parsing
    ops::add_remote(&git_dir, "origin", "git@github.com:oxidize-rs/oxidize.git").unwrap();
    let commit_oid: ObjectId = "1111222233334444555566667777888899990000".parse().unwrap();

    let urls = ops::get_provider_urls(&git_dir, Some(commit_oid), Some("main"));
    assert_eq!(
        urls.repo_url.as_deref(),
        Some("https://github.com/oxidize-rs/oxidize")
    );
    assert_eq!(
        urls.commit_url.as_deref(),
        Some(
            "https://github.com/oxidize-rs/oxidize/commit/1111222233334444555566667777888899990000"
        )
    );
    assert_eq!(
        urls.branch_url.as_deref(),
        Some("https://github.com/oxidize-rs/oxidize/tree/main")
    );
    assert_eq!(
        urls.pr_url.as_deref(),
        Some("https://github.com/oxidize-rs/oxidize/compare/main?expand=1")
    );

    // 2. Test yanking selected commit
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    app.active_panel = Panel::Commits;
    app.commits_selected = 0;
    app.yank_selected();
    assert!(app.status_message.as_ref().unwrap().contains("Yanked"));
}

#[test]
fn test_packet6_ui_headless_rendering() {
    let (_tmp, repo_root) = create_base_repo();
    let git_dir = repo_root.join(".git");
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();

    // 1. Render RemoteAdd modal
    app.open_remote_add_modal();
    terminal
        .draw(|f| {
            oxidize_tui::ui::render(f, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(
        content.contains("Add Remote"),
        "RemoteAdd modal title must be rendered"
    );
    assert!(content.contains("Remote Name:"));
    assert!(content.contains("Remote URL:"));

    // 2. Render SubmoduleList modal
    let dummy_oid: ObjectId = "abcdef0123456789abcdef0123456789abcdef01".parse().unwrap();
    let sub = SubmoduleItem {
        name: "sub_engine".to_string(),
        path: "engine".to_string(),
        url: "https://github.com/org/engine.git".to_string(),
        gitlink_oid: Some(dummy_oid),
        head_oid: Some(dummy_oid),
        is_initialized: true,
        is_dirty: false,
    };
    app.active_modal = ActiveModal::SubmoduleList {
        items: vec![sub],
        selected: 0,
    };
    terminal
        .draw(|f| {
            oxidize_tui::ui::render(f, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(content.contains("Submodules"));
    assert!(content.contains("sub_engine"));
    assert!(content.contains("Enter repo"));

    // 3. Render BisectMenu modal
    let bad_oid: ObjectId = "1111111111111111111111111111111111111111".parse().unwrap();
    let good_oid: ObjectId = "2222222222222222222222222222222222222222".parse().unwrap();
    app.bisect_state = BisectState {
        is_active: true,
        bad_oid: Some(bad_oid),
        good_oids: vec![good_oid],
        current_oid: None,
        orig_branch: Some("main".to_string()),
        remaining_steps: 3,
        total_revisions: 8,
        culprit_oid: None,
    };
    app.active_modal = ActiveModal::BisectMenu {
        state: app.bisect_state.clone(),
        selected: 0,
    };
    terminal
        .draw(|f| {
            oxidize_tui::ui::render(f, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(content.contains("Git Bisect Controls"));
    assert!(content.contains("Mark Current HEAD as Bad"));
    assert!(content.contains("Mark Current HEAD as Good"));
    assert!(content.contains("Reset / Abort Bisect"));

    // 4. Render CommandPalette modal
    app.open_command_palette();
    terminal
        .draw(|f| {
            oxidize_tui::ui::render(f, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(content.contains("Command Palette"));
    assert!(content.contains("commit"));
    assert!(content.contains("push"));

    // 5. Render ProviderLinks modal
    let urls = ProviderUrls {
        repo_url: Some("https://github.com/oxidize-rs/oxidize".to_string()),
        commit_url: Some("https://github.com/oxidize-rs/oxidize/commit/abc".to_string()),
        branch_url: Some("https://github.com/oxidize-rs/oxidize/tree/main".to_string()),
        pr_url: Some("https://github.com/oxidize-rs/oxidize/pull/new/main".to_string()),
    };
    app.active_modal = ActiveModal::ProviderLinks { urls, selected: 0 };
    terminal
        .draw(|f| {
            oxidize_tui::ui::render(f, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(content.contains("Web Provider Links"));
    assert!(content.contains("Repository:"));
    assert!(content.contains("Pull Request:"));

    // 6. Render Bisect status banner in Status panel and commit decorations
    app.active_modal = ActiveModal::None;
    app.active_panel = Panel::Status;
    terminal
        .draw(|f| {
            oxidize_tui::ui::render(f, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(
        content.contains("BISECTING (~3 steps left)"),
        "Status panel must render BISECTING banner"
    );
}

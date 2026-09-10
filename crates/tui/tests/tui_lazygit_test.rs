//! Comprehensive real-time tests for Authentic LazyGit Replica (5 panels, sub-tabs, Vim focus, amend, stash save/apply).

use oxidize_tui::model::{
    ActiveModal, BranchesTab, CommitsTab, FocusedWindow, Panel,
};
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
        .args(["config", "user.name", "LazyOx Developer"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "lazyox@oxidize.rs"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();

    fs::write(repo_dir.join("initial.txt"), "hello world\n").unwrap();
    Command::new("git").args(["add", "."]).current_dir(&repo_dir).output().unwrap();
    Command::new("git").args(["commit", "-m", "Initial commit"]).current_dir(&repo_dir).output().unwrap();

    let git_dir = repo_dir.join(".git");
    (tmp, git_dir)
}

#[test]
fn test_subtabs_switching() {
    let mut app = App::new();

    // In Branches panel
    app.select_panel(Panel::Branches);
    assert_eq!(app.branches_tab, BranchesTab::Local);

    app.next_tab();
    assert_eq!(app.branches_tab, BranchesTab::Remotes);

    app.next_tab();
    assert_eq!(app.branches_tab, BranchesTab::Tags);

    app.next_tab();
    assert_eq!(app.branches_tab, BranchesTab::Local);

    app.prev_tab();
    assert_eq!(app.branches_tab, BranchesTab::Tags);

    app.prev_tab();
    assert_eq!(app.branches_tab, BranchesTab::Remotes);

    // In Commits panel
    app.select_panel(Panel::Commits);
    assert_eq!(app.commits_tab, CommitsTab::Commits);

    app.next_tab();
    assert_eq!(app.commits_tab, CommitsTab::Reflog);

    app.next_tab();
    assert_eq!(app.commits_tab, CommitsTab::Commits);

    app.prev_tab();
    assert_eq!(app.commits_tab, CommitsTab::Reflog);
}

#[test]
fn test_vim_window_focus_navigation() {
    let mut app = App::new();
    assert_eq!(app.focused_window, FocusedWindow::Sidebar);

    app.focus_inspector();
    assert_eq!(app.focused_window, FocusedWindow::Inspector);

    app.focus_sidebar();
    assert_eq!(app.focused_window, FocusedWindow::Sidebar);

    app.toggle_focus();
    assert_eq!(app.focused_window, FocusedWindow::Inspector);

    app.toggle_focus();
    assert_eq!(app.focused_window, FocusedWindow::Sidebar);
}

#[test]
fn test_remotes_tags_and_reflog_loading() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    // Add a remote
    Command::new("git")
        .args(["remote", "add", "origin", "https://github.com/oxidize/ox.git"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Add a tag
    Command::new("git")
        .args(["tag", "v0.1.0"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Make second commit to generate reflog entries
    fs::write(repo_dir.join("second.txt"), "second file\n").unwrap();
    Command::new("git").args(["add", "."]).current_dir(repo_dir).output().unwrap();
    Command::new("git").args(["commit", "-m", "Second commit"]).current_dir(repo_dir).output().unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // 1. Verify remotes
    assert_eq!(app.remotes.len(), 1);
    assert_eq!(app.remotes[0].name, "origin");
    assert!(app.remotes[0].url.contains("https://github.com/oxidize/ox.git"));

    // 2. Verify tags
    assert_eq!(app.tags.len(), 1);
    assert_eq!(app.tags[0].name, "v0.1.0");

    // 3. Verify reflog
    assert!(!app.reflog.is_empty());
    assert!(app.reflog.iter().any(|r| r.action.contains("commit") || r.message.contains("Second commit")));

    // 4. Test Inspector for Remotes tab
    app.select_panel(Panel::Branches);
    app.branches_tab = BranchesTab::Remotes;
    app.update_inspector();
    let remote_diff = app.cached_diff.as_ref().unwrap();
    assert!(remote_diff.title.contains("Remote: origin"));
    assert!(remote_diff.lines.iter().any(|l| l.content.contains("https://github.com/oxidize/ox.git")));

    // 5. Test Inspector for Tags tab
    app.branches_tab = BranchesTab::Tags;
    app.update_inspector();
    let tag_diff = app.cached_diff.as_ref().unwrap();
    assert!(tag_diff.title.contains("Tag: v0.1.0"));

    // 6. Test Inspector for Reflog tab
    app.select_panel(Panel::Commits);
    app.commits_tab = CommitsTab::Reflog;
    app.update_inspector();
    let reflog_diff = app.cached_diff.as_ref().unwrap();
    assert!(reflog_diff.title.contains("Reflog:"));
}

#[test]
fn test_amend_commit_modal() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // Stage a new change
    fs::write(repo_dir.join("initial.txt"), "amended text content\n").unwrap();
    Command::new("git").args(["add", "."]).current_dir(repo_dir).output().unwrap();
    app.refresh().unwrap();

    // Open amend modal
    app.open_amend_modal();
    match app.active_modal {
        ActiveModal::CommitAmend { ref message, cursor } => {
            assert_eq!(message, "Initial commit");
            assert_eq!(cursor, message.len());
        }
        _ => panic!("Expected CommitAmend modal"),
    }

    // Modify the commit message
    for c in " - amended".chars() {
        app.handle_modal_char(c);
    }

    // Submit amend
    app.submit_modal().unwrap();
    assert_eq!(app.active_modal, ActiveModal::None);
    assert!(app.status_message.as_ref().unwrap().contains("Amended commit"));

    // Check commits list
    assert_eq!(app.commits.len(), 1);
    assert_eq!(app.commits[0].summary, "Initial commit - amended");
}

#[test]
fn test_stash_save_and_apply() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // Create a modified file
    fs::write(repo_dir.join("initial.txt"), "wip uncommitted changes\n").unwrap();
    app.refresh().unwrap();

    // Open Stash Save modal
    app.open_stash_save_modal();
    match app.active_modal {
        ActiveModal::StashSave { ref message, cursor } => {
            assert!(message.is_empty());
            assert_eq!(cursor, 0);
        }
        _ => panic!("Expected StashSave modal"),
    }

    for c in "feature WIP stash".chars() {
        app.handle_modal_char(c);
    }

    // Submit save modal
    app.submit_modal().unwrap();
    assert_eq!(app.active_modal, ActiveModal::None);
    assert!(app.status_message.as_ref().unwrap().contains("Saved stash"));
    assert_eq!(app.stashes.len(), 1);
    assert!(app.stashes[0].message.contains("feature WIP stash"));

    // Test apply stash (stash remains in stack, unlike pop)
    app.select_panel(Panel::Stash);
    app.apply_selected_stash().unwrap();
    assert!(app.status_message.as_ref().unwrap().contains("Applied stash@{0}"));
    assert_eq!(app.stashes.len(), 1);
    assert_eq!(
        fs::read_to_string(repo_dir.join("initial.txt")).unwrap(),
        "wip uncommitted changes\n"
    );
}

#[test]
fn test_status_panel_inspector() {
    let (_tmp, git_dir) = create_test_repo();
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    app.select_panel(Panel::Status);
    let diff = app.cached_diff.as_ref().unwrap();
    assert!(diff.title.contains("Status Overview"));
    assert!(diff.lines.iter().any(|l| l.content.contains("Branch:") && l.content.contains("master")));
    assert!(diff.lines.iter().any(|l| l.content.contains("Repository:") || l.content.contains("Path:")));
}

#[test]
fn test_contextual_help_modal_for_all_panels() {
    let (_tmp, git_dir) = create_test_repo();
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    // 1. Files panel help
    app.select_panel(Panel::Files);
    app.active_modal = ActiveModal::Help;
    terminal.draw(|f| ui::render(f, &app)).unwrap();
    let buf = terminal.backend().buffer();
    let text = format!("{:?}", buf);
    assert!(text.contains("Files") && text.contains("Cheatsheet"));
    assert!(text.contains("Amend"));

    // 2. Branches panel help
    app.select_panel(Panel::Branches);
    terminal.draw(|f| ui::render(f, &app)).unwrap();
    let buf2 = terminal.backend().buffer();
    let text2 = format!("{:?}", buf2);
    assert!(text2.contains("Branches") && text2.contains("Cheatsheet"));
    assert!(text2.contains("Switch sub-tab") || text2.contains("sub-tabs"));

    // 3. Commits panel help
    app.select_panel(Panel::Commits);
    terminal.draw(|f| ui::render(f, &app)).unwrap();
    let buf3 = terminal.backend().buffer();
    let text3 = format!("{:?}", buf3);
    assert!(text3.contains("Commits") && text3.contains("Cheatsheet"));

    // 4. Stash panel help
    app.select_panel(Panel::Stash);
    terminal.draw(|f| ui::render(f, &app)).unwrap();
    let buf4 = terminal.backend().buffer();
    let text4 = format!("{:?}", buf4);
    assert!(text4.contains("Stash") && text4.contains("Cheatsheet"));
    assert!(text4.contains("Apply selected stash") || text4.contains("Apply stash"));
}

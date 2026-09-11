//! Packet 0 Integration Tests: Safety, Highlighter Termination, Subtab Targeting,
//! Confirmation Dialogs, Footer Geometry, Concurrent Mutation Locks, and Draft Preservation.

use oxidize_tui::app::BackgroundJob;
use oxidize_tui::model::{
    ActiveModal, BranchesTab, ConfirmAction, DiffLine, DiffLineKind, FocusedWindow, Panel,
};
use oxidize_tui::syntax::{Language, SyntaxHighlighter};
use oxidize_tui::ui::{self, compute_modal_layout};
use oxidize_tui::{App, TuiError};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
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

    fs::write(
        repo_dir.join("main.rs"),
        "fn main() {\n    let $value = 42;\n}\n",
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
fn test_packet0_highlighter_dollar_tokens_and_exact_roundtrip() {
    let test_cases = [
        (Language::Shell, "+echo \"Value: $HOME\""),
        (Language::Php, "+$variable = 123;"),
        (Language::JavaScript, "+const $value = ref;"),
        (Language::Graphql, "+query MyQuery($id: ID!) {"),
        (Language::Generic, "+$ alone dollar and $$$ triple"),
    ];

    for (lang, text) in test_cases {
        let mut highlighter = SyntaxHighlighter::new(lang);
        let dl = DiffLine {
            kind: DiffLineKind::Addition,
            content: text.to_string(),
        };

        let start = std::time::Instant::now();
        let line = highlighter.highlight_line(&dl);
        assert!(
            start.elapsed() < std::time::Duration::from_millis(50),
            "Tokenization hung or looped on: {}",
            text
        );

        // Verify exact roundtrip: text content of all spans concatenated reproduces original line
        let reconstructed: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(reconstructed, text);
    }
}

#[test]
fn test_packet0_footer_unicode_width_alignment() {
    let (_tmp, git_dir) = create_test_repo();
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // With ASCII status message
    app.status_message = Some("Staged 1 file(s)".to_string());
    let (_, buttons_ascii) = ui::build_footer(&app, 120);
    let first_btn_start_ascii = buttons_ascii[0].0;

    // " Staged 1 file(s) " is 18 chars, separator is 3 cols -> first button should start at 21
    assert_eq!(first_btn_start_ascii, 18 + 3);

    // With Unicode status message containing checkmark (✓ is 3 bytes in UTF-8, but 1 display cell!)
    app.status_message = Some("✓ Staged 1 file(s)".to_string());
    let (_, buttons_unicode) = ui::build_footer(&app, 120);
    let first_btn_start_unicode = buttons_unicode[0].0;

    // " ✓ Staged 1 file(s) " is 20 display cells (1 + 1 + 1 + 16 + 1), separator is 3 cols -> start at 23
    // Byte length would have been 22 + 3 = 25, which would diverge from rendered cell columns!
    assert_eq!(first_btn_start_unicode, 20 + 3);
}

#[test]
fn test_packet0_subtab_safety_remotes_and_tags() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    // Create a local branch
    Command::new("git")
        .args(["branch", "feature-safe"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Create a tag
    Command::new("git")
        .args(["tag", "v1.0.0"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    app.select_panel(Panel::Branches);
    assert_eq!(app.branch_name, "master");

    // 1. Switch to Tags subtab
    app.branches_tab = BranchesTab::Tags;

    // Calling checkout on Tags must NOT check out local branch selection!
    let res = app.checkout_selected_branch();
    assert!(res.is_ok());
    assert_eq!(app.branch_name, "master");

    // Calling delete on Tags must NOT delete local branch selection!
    let res = app.delete_selected_branch();
    assert!(res.is_ok());
    assert!(app.branches.iter().any(|b| b.name == "feature-safe"));

    // 2. Switch to Remotes subtab
    app.branches_tab = BranchesTab::Remotes;

    let res = app.checkout_selected_branch();
    assert!(res.is_ok());
    assert_eq!(app.branch_name, "master");

    let res = app.delete_selected_branch();
    assert!(res.is_ok());
    assert!(app.branches.iter().any(|b| b.name == "feature-safe"));
}

#[test]
fn test_packet0_confirmation_modal_workflow() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    // Modify a tracked file
    fs::write(repo_dir.join("main.rs"), "fn main() { /* dirty */ }\n").unwrap();

    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    app.select_panel(Panel::Files);
    assert!(!app.files.is_empty());

    // Prompt discard
    app.prompt_discard_selected_file();
    match &app.active_modal {
        ActiveModal::Confirm {
            title,
            prompt,
            action,
        } => {
            assert!(title.contains("Confirm Discard"));
            assert!(prompt.contains("main.rs"));
            assert_eq!(*action, ConfirmAction::DiscardFile("main.rs".to_string()));
        }
        _ => panic!("Expected ActiveModal::Confirm"),
    }

    // Cancel modal - file should remain dirty
    app.close_modal();
    assert_eq!(app.active_modal, ActiveModal::None);
    let content = fs::read_to_string(repo_dir.join("main.rs")).unwrap();
    assert!(content.contains("dirty"));

    // Prompt again and submit
    app.prompt_discard_selected_file();
    let res = app.submit_modal();
    assert!(res.is_ok());
    assert_eq!(app.active_modal, ActiveModal::None);

    // Verify file changes were discarded
    let content = fs::read_to_string(repo_dir.join("main.rs")).unwrap();
    assert!(!content.contains("dirty"));
}

#[test]
fn test_packet0_concurrent_mutation_lock() {
    let (_tmp, git_dir) = create_test_repo();
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    // Simulate an active background job (e.g. pull)
    let (_tx, rx) = std::sync::mpsc::channel();
    app.active_job = Some(BackgroundJob {
        description: "Pulling from origin".to_string(),
        receiver: rx,
    });

    // All mutating operations must be rejected while job is active
    let res = app.toggle_stage_selected();
    assert!(matches!(res, Err(TuiError::Terminal(_))));

    let res = app.stage_all();
    assert!(matches!(res, Err(TuiError::Terminal(_))));

    let res = app.discard_selected_file();
    assert!(matches!(res, Err(TuiError::Terminal(_))));

    let res = app.checkout_selected_branch();
    assert!(matches!(res, Err(TuiError::Terminal(_))));

    let res = app.delete_selected_branch();
    assert!(matches!(res, Err(TuiError::Terminal(_))));

    let res = app.apply_selected_stash();
    assert!(matches!(res, Err(TuiError::Terminal(_))));

    let res = app.pop_selected_stash();
    assert!(matches!(res, Err(TuiError::Terminal(_))));

    let res = app.drop_selected_stash();
    assert!(matches!(res, Err(TuiError::Terminal(_))));

    // Read and navigation operations remain functional and responsive
    app.next_panel();
    assert_eq!(app.active_panel, Panel::Branches);
    app.focus_inspector();
    assert_eq!(app.focused_window, FocusedWindow::Inspector);
    app.scroll_inspector_down(5);
    assert_eq!(app.inspector_scroll, 5);
}

#[test]
fn test_packet0_recoverable_draft_preservation() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    // Create a new unstaged file and stage it
    fs::write(repo_dir.join("new.txt"), "hello").unwrap();
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    app.select_panel(Panel::Files);
    app.toggle_stage_selected().unwrap();

    // Open commit modal and type a draft
    app.open_commit_modal();
    assert!(matches!(app.active_modal, ActiveModal::CommitPrompt { .. }));

    for c in "feat: my detailed commit message draft".chars() {
        app.handle_modal_char(c);
    }

    if let ActiveModal::CommitPrompt { ref message, .. } = app.active_modal {
        assert_eq!(message, "feat: my detailed commit message draft");
    } else {
        panic!("Expected CommitPrompt");
    }

    // Simulate an external lock failure preventing commit creation
    let lock_path = git_dir.join("refs").join("heads").join("master.lock");
    fs::write(&lock_path, "locked").unwrap();

    // Submitting modal fails due to lock, but preserves user's draft!
    let _ = app.submit_modal();

    assert!(app
        .status_message
        .as_ref()
        .unwrap()
        .contains("Commit failed"));
    if let ActiveModal::CommitPrompt { ref message, .. } = app.active_modal {
        assert_eq!(
            message, "feat: my detailed commit message draft",
            "Draft message must be preserved on commit failure"
        );
    } else {
        panic!("Modal should remain open with draft intact on commit failure");
    }

    // Clean up lock and resubmit
    fs::remove_file(&lock_path).unwrap();
    let res = app.submit_modal();
    assert!(res.is_ok());
    assert_eq!(app.active_modal, ActiveModal::None);
    assert!(app.status_message.as_ref().unwrap().starts_with("✓ ["));

    // Modal layout computation for Confirm modal
    let confirm = ActiveModal::Confirm {
        title: "Test".to_string(),
        prompt: "Prompt".to_string(),
        action: ConfirmAction::DropStash(0),
    };
    let layout = compute_modal_layout(&confirm, Rect::new(0, 0, 100, 40));
    assert!(layout.is_some());
    let ml = layout.unwrap();
    assert!(ml.action_rect.is_some());
}

#[test]
fn test_packet0_amend_modal_distinct_head_title() {
    let (_tmp, git_dir) = create_test_repo();
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();

    app.open_amend_modal();
    if let ActiveModal::CommitAmend { ref message, .. } = app.active_modal {
        assert_eq!(message, "initial commit");
    } else {
        panic!("Expected CommitAmend");
    }

    // Render to TestBackend and verify "Amend HEAD Commit" appears in buffer
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| ui::render(f, &app)).unwrap();

    let buffer = terminal.backend().buffer();
    let mut buffer_str = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            buffer_str.push_str(buffer[(x, y)].symbol());
        }
        buffer_str.push('\n');
    }

    assert!(buffer_str.contains("Amend HEAD Commit"));
}

#[test]
fn test_packet0_stash_conflict_result() {
    let (tmp, git_dir) = create_test_repo();
    let repo_dir = tmp.path();

    // 1. Make a change and stash it
    fs::write(
        repo_dir.join("main.rs"),
        "fn main() { /* stash version */ }\n",
    )
    .unwrap();
    Command::new("git")
        .args(["stash", "push", "-m", "stash change"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // 2. Make conflicting commit on master
    fs::write(
        repo_dir.join("main.rs"),
        "fn main() { /* conflicting commit */ }\n",
    )
    .unwrap();
    Command::new("git")
        .args(["commit", "-am", "conflicting commit"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // 3. Open TUI, select stash panel, apply stash
    let mut app = App::new();
    app.load_repository(&git_dir).unwrap();
    app.select_panel(Panel::Stash);
    assert_eq!(app.stashes.len(), 1);

    let res = app.apply_selected_stash();
    assert!(res.is_ok());

    // 4. Verify explicit conflict status reporting
    let status = app.status_message.expect("status message set");
    assert!(
        status.contains("with conflicts"),
        "Status must explicitly declare conflict outcome: {}",
        status
    );
}

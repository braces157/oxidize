//! Review regressions: assertions describe REQUIRED behavior and fail on the reviewed checkout.
//! Copy to crates/tui/tests/parity_review_probes.rs and run that test target. Git is fixture/oracle only.
use oxidize_core::ObjectId;
use oxidize_diff::{apply_hunk_forward, apply_hunk_reverse, compute_structured_diff};
use oxidize_tui::{
    model::{ActiveModal, BranchesTab, ConfirmAction, Panel},
    ops,
    sequencer::{RebaseAction, RebaseTodoItem},
    App,
};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "NUL")
        .env("GIT_AUTHOR_NAME", "Review")
        .env("GIT_AUTHOR_EMAIL", "review@example.invalid")
        .env("GIT_COMMITTER_NAME", "Review")
        .env("GIT_COMMITTER_EMAIL", "review@example.invalid")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}
fn repo() -> TempDir {
    let t = TempDir::new().unwrap();
    git(t.path(), &["init", "-b", "main"]);
    git(t.path(), &["config", "core.autocrlf", "false"]);
    git(t.path(), &["config", "user.name", "Review"]);
    git(
        t.path(),
        &["config", "user.email", "review@example.invalid"],
    );
    fs::write(t.path().join("f.txt"), "one\ntwo\nthree\n").unwrap();
    git(t.path(), &["add", "."]);
    git(t.path(), &["commit", "-m", "base"]);
    t
}
#[test]
fn rebase_must_reject_dirty_worktree_before_mutation() {
    let t = repo();
    let r = t.path();
    let gd = r.join(".git");
    let base: ObjectId = git(r, &["rev-parse", "HEAD"]).parse().unwrap();
    fs::write(r.join("f.txt"), "committed change\n").unwrap();
    git(r, &["commit", "-am", "next"]);
    let head: ObjectId = git(r, &["rev-parse", "HEAD"]).parse().unwrap();
    fs::write(r.join("f.txt"), "UNCOMMITTED USER WORK\n").unwrap();
    let result = ops::start_interactive_rebase(
        r,
        &gd,
        &gd,
        &base,
        vec![RebaseTodoItem::new(RebaseAction::Pick, head, "next".into())],
    );
    assert_eq!(
        fs::read_to_string(r.join("f.txt")).unwrap(),
        "UNCOMMITTED USER WORK\n",
        "rebase result: {result:?}"
    );
    assert!(result.is_err());
}
#[test]
fn worktree_removal_must_reject_dirty_unlocked_tree() {
    let t = repo();
    let r = t.path();
    let gd = r.join(".git");
    let holder = TempDir::new().unwrap();
    let wt = holder.path().join("linked");
    git(r, &["worktree", "add", "-b", "topic", wt.to_str().unwrap()]);
    fs::write(wt.join("f.txt"), "UNCOMMITTED\n").unwrap();
    fs::write(wt.join("untracked.txt"), "ONLY COPY\n").unwrap();
    // Both resolved deletion targets are inside these disposable TempDir fixtures.
    assert!(wt
        .canonicalize()
        .unwrap()
        .starts_with(holder.path().canonicalize().unwrap()));
    let result = ops::remove_worktree(&gd, "linked", false);
    assert!(
        wt.join("untracked.txt").exists(),
        "dirty tree was removed: {result:?}"
    );
    assert!(result.is_err());
}
#[test]
fn stale_short_patch_must_return_error_without_panicking() {
    let h = compute_structured_diff("one\ntwo\nthree\n", "one\nchanged\nthree\n", 3).remove(0);
    let fwd = std::panic::catch_unwind(|| apply_hunk_forward("", &h));
    let rev = std::panic::catch_unwind(|| apply_hunk_reverse("", &h));
    assert!(
        matches!(fwd, Ok(Err(_))) && matches!(rev, Ok(Err(_))),
        "stale short target panicked"
    );
}
#[test]
fn staging_deleted_file_hunk_must_remove_index_entry() {
    let t = repo();
    let r = t.path();
    let gd = r.join(".git");
    let h = compute_structured_diff("one\ntwo\nthree\n", "", 3).remove(0);
    fs::remove_file(r.join("f.txt")).unwrap();
    ops::stage_hunk(r, &gd, "f.txt", &h).unwrap();
    assert_eq!(
        git(r, &["ls-files", "--stage", "--", "f.txt"]),
        "",
        "deletion was staged as an empty blob"
    );
}
#[test]
fn stage_hunk_must_reject_changed_worktree_snapshot() {
    let t = repo();
    let r = t.path();
    let gd = r.join(".git");
    let h = compute_structured_diff("one\ntwo\nthree\n", "one\nselected\nthree\n", 3).remove(0);
    fs::write(r.join("f.txt"), "one\nEXTERNAL EDIT\nthree\n").unwrap();
    let result = ops::stage_hunk(r, &gd, "f.txt", &h);
    assert!(
        result.is_err(),
        "staged cached bytes despite changed worktree: {}",
        git(r, &["show", ":f.txt"])
    );
}
#[test]
fn corrupted_index_must_not_load_as_repository_truth() {
    let t = repo();
    let r = t.path();
    let gd = r.join(".git");
    fs::write(gd.join("index"), b"corrupt index").unwrap();
    let mut app = App::new();
    assert!(
        app.load_repository(&gd).is_err(),
        "corrupted index accepted as empty"
    );
}
#[test]
fn tag_delete_must_target_selected_tag() {
    let t = repo();
    let r = t.path();
    git(r, &["tag", "release-v1"]);
    let mut app = App::new();
    app.load_repository(&r.join(".git")).unwrap();
    app.active_panel = Panel::Branches;
    app.branches_tab = BranchesTab::Tags;
    app.prompt_delete_selected_tag();
    match &app.active_modal {
        ActiveModal::Confirm {
            action: ConfirmAction::DeleteTag(name),
            ..
        } => assert_eq!(name, "release-v1"),
        other => panic!("expected selected tag confirmation: {other:?}"),
    }
}
#[test]
fn app_force_lease_must_use_recorded_remote_oid() {
    let t = repo();
    let r = t.path();
    let gd = r.join(".git");
    let remote = TempDir::new().unwrap();
    git(remote.path(), &["init", "--bare"]);
    git(
        r,
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    git(r, &["push", "-u", "origin", "main"]);
    fs::write(r.join("f.txt"), "new commit\n").unwrap();
    git(r, &["commit", "-am", "next"]);
    let expected = git(r, &["rev-parse", "HEAD"]);
    let mut app = App::new();
    app.load_repository(&gd).unwrap();
    app.commits_selected = app
        .commits
        .iter()
        .position(|c| c.oid.to_string() == expected)
        .unwrap();
    app.push_force_lease();
    assert_eq!(
        git(remote.path(), &["rev-parse", "refs/heads/main"]),
        expected,
        "{:?}",
        app.status_message
    );
}

#[test]
fn stash_untracked_restore_must_preserve_existing_file() {
    let t = repo();
    let r = t.path();
    let gd = r.join(".git");
    fs::write(r.join("local.txt"), "stashed content\n").unwrap();
    git(
        r,
        &["stash", "push", "--include-untracked", "-m", "fixture"],
    );
    let stash: ObjectId = git(r, &["rev-parse", "refs/stash"]).parse().unwrap();
    fs::write(r.join("local.txt"), "NEW USER CONTENT\n").unwrap();
    let result = ops::apply_stash(r, &gd, &stash);
    assert_eq!(
        fs::read_to_string(r.join("local.txt")).unwrap(),
        "NEW USER CONTENT\n",
        "stash result: {result:?}"
    );
}

#[test]
fn replay_must_not_merge_distinct_invalid_utf8_as_identical_text() {
    let t = repo();
    let r = t.path();
    let gd = r.join(".git");
    fs::write(r.join("raw.bin"), [0xff, b'\n']).unwrap();
    git(r, &["add", "."]);
    git(r, &["commit", "-m", "raw base"]);
    let base = git(r, &["rev-parse", "HEAD"]);
    let base_tree: ObjectId = git(r, &["rev-parse", "HEAD^{tree}"]).parse().unwrap();
    fs::write(r.join("raw.bin"), [0xfe, b'\n']).unwrap();
    git(r, &["commit", "-am", "ours"]);
    let ours: ObjectId = git(r, &["rev-parse", "HEAD^{tree}"]).parse().unwrap();
    git(r, &["checkout", "-b", "side", &base]);
    fs::write(r.join("raw.bin"), [0xfd, b'\n']).unwrap();
    git(r, &["commit", "-am", "theirs"]);
    let theirs: ObjectId = git(r, &["rev-parse", "HEAD^{tree}"]).parse().unwrap();
    git(r, &["checkout", "main"]);
    let mut index = oxidize_index::Index::load_from(&gd.join("index")).unwrap();
    let conflicts = ops::merge_trees_into_index_and_worktree(
        r, &gd, &gd, &mut index, &base_tree, &ours, &theirs, "ours", "theirs",
    )
    .unwrap();
    assert!(
        conflicts.contains(&"raw.bin".to_string()),
        "distinct binary edits merged cleanly into {:?}",
        fs::read(r.join("raw.bin")).unwrap()
    );
}

#[test]
fn conflicted_fixup_continue_must_preserve_fixup_semantics() {
    let t = repo();
    let r = t.path();
    let gd = r.join(".git");
    let base = git(r, &["rev-parse", "HEAD"]);
    fs::write(r.join("f.txt"), "ours\n").unwrap();
    git(r, &["commit", "-am", "keep message"]);
    let first: ObjectId = git(r, &["rev-parse", "HEAD"]).parse().unwrap();
    git(r, &["checkout", "-b", "side", &base]);
    fs::write(r.join("f.txt"), "theirs\n").unwrap();
    git(r, &["commit", "-am", "discard fixup message"]);
    let second: ObjectId = git(r, &["rev-parse", "HEAD"]).parse().unwrap();
    git(r, &["checkout", "main"]);
    let result = ops::start_interactive_rebase(
        r,
        &gd,
        &gd,
        &base.parse().unwrap(),
        vec![
            RebaseTodoItem::new(RebaseAction::Pick, first, "first".into()),
            RebaseTodoItem::new(RebaseAction::Fixup, second, "second".into()),
        ],
    )
    .unwrap();
    assert!(matches!(result, ops::ReplayStepOutcome::Conflict { .. }));
    ops::resolve_conflict_choice(
        r,
        &gd,
        &gd,
        "f.txt",
        oxidize_tui::model::ConflictChoice::Theirs,
    )
    .unwrap();
    ops::rebase_continue(r, &gd, &gd).unwrap();
    assert_eq!(
        git(r, &["rev-list", "--count", "HEAD"]),
        "2",
        "fixup created an additional commit"
    );
    assert_eq!(git(r, &["log", "-1", "--format=%s"]), "keep message");
}

#[test]
fn rebase_todo_must_exclude_other_branch_commits() {
    let t = repo();
    let r = t.path();
    fs::write(r.join("f.txt"), "main change\n").unwrap();
    git(r, &["commit", "-am", "main tip"]);
    let main: ObjectId = git(r, &["rev-parse", "HEAD"]).parse().unwrap();
    git(r, &["checkout", "-b", "side"]);
    fs::write(r.join("side.txt"), "side-only work\n").unwrap();
    git(r, &["add", "."]);
    git(r, &["commit", "-m", "side only"]);
    let side: ObjectId = git(r, &["rev-parse", "HEAD"]).parse().unwrap();
    git(r, &["checkout", "main"]);
    let mut app = App::new();
    app.load_repository(&r.join(".git")).unwrap();
    app.commits_selected = app.commits.iter().position(|c| c.oid == main).unwrap();
    app.open_rebase_todo_modal();
    match &app.active_modal {
        ActiveModal::RebaseTodo { items, .. } => assert!(
            !items.iter().any(|i| i.commit_oid == side),
            "other branch's commit entered main rebase plan"
        ),
        other => panic!("expected todo: {other:?}"),
    }
}

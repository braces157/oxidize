use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

fn run_cmd(cmd: &str, args: &[&str], cwd: &Path) -> String {
    let output = Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|e| panic!("failed to execute {}: {}", cmd, e));

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!(
            "command '{} {:?}' failed with exit code {:?}\nstdout: {}\nstderr: {}",
            cmd,
            args,
            output.status.code(),
            stdout,
            stderr
        );
    }

    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn ox_bin() -> String {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_ox") {
        return path;
    }
    let mut path = std::env::current_exe().expect("current exe path");
    path.pop();
    if path.file_name().and_then(|n| n.to_str()) == Some("deps") {
        path.pop();
    }
    path.push(if cfg!(windows) { "ox.exe" } else { "ox" });
    path.to_string_lossy().to_string()
}

#[test]
fn test_gitignore_filtering_with_status_and_add() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let ox = ox_bin();

    run_cmd(&ox, &["init"], root);
    run_cmd("git", &["config", "user.name", "Tester"], root);
    run_cmd("git", &["config", "user.email", "tester@example.com"], root);

    // Create .gitignore
    fs::write(root.join(".gitignore"), "*.log\nbuild/\n!keep.log\n").unwrap();

    fs::write(root.join("debug.log"), "drop me\n").unwrap();
    fs::write(root.join("keep.log"), "save me\n").unwrap();
    fs::create_dir_all(root.join("build")).unwrap();
    fs::write(root.join("build").join("output.bin"), "binary data\n").unwrap();
    fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();

    // ox status should ignore debug.log and build/, but include keep.log, main.rs, .gitignore
    let status_out = run_cmd(&ox, &["status"], root);
    assert!(
        status_out.contains("keep.log"),
        "keep.log should be untracked"
    );
    assert!(
        status_out.contains("main.rs"),
        "main.rs should be untracked"
    );
    assert!(
        !status_out.contains("debug.log"),
        "debug.log should be ignored"
    );
    assert!(
        !status_out.contains("output.bin"),
        "output.bin should be ignored"
    );

    // ox add .
    run_cmd(&ox, &["add", "."], root);

    // Verify staged files
    let ls_files = run_cmd(&ox, &["ls-files"], root);
    assert!(ls_files.contains(".gitignore"));
    assert!(ls_files.contains("keep.log"));
    assert!(ls_files.contains("main.rs"));
    assert!(!ls_files.contains("debug.log"));
    assert!(!ls_files.contains("output.bin"));

    // Verify official git sees exact same index
    let git_status = run_cmd("git", &["status", "--porcelain"], root);
    assert!(git_status.contains("A  .gitignore"));
    assert!(git_status.contains("A  keep.log"));
    assert!(git_status.contains("A  main.rs"));
    assert!(!git_status.contains("debug.log"));
}

#[test]
fn test_ox_rm_mv_restore() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let ox = ox_bin();

    run_cmd(&ox, &["init"], root);
    run_cmd("git", &["config", "user.name", "Tester"], root);
    run_cmd("git", &["config", "user.email", "tester@example.com"], root);

    fs::write(root.join("file1.txt"), "hello file 1\n").unwrap();
    fs::write(root.join("file2.txt"), "hello file 2\n").unwrap();
    fs::write(root.join("file3.txt"), "hello file 3\n").unwrap();
    run_cmd(&ox, &["add", "."], root);
    run_cmd(&ox, &["commit", "-m", "Initial commit"], root);

    // 1. Test ox rm (removes from index and disk)
    run_cmd(&ox, &["rm", "file1.txt"], root);
    assert!(
        !root.join("file1.txt").exists(),
        "file1.txt should be deleted from disk"
    );
    let ls_files = run_cmd(&ox, &["ls-files"], root);
    assert!(
        !ls_files.contains("file1.txt"),
        "file1.txt should be removed from index"
    );

    // 2. Test ox rm --cached (removes from index, keeps on disk)
    run_cmd(&ox, &["rm", "--cached", "file2.txt"], root);
    assert!(
        root.join("file2.txt").exists(),
        "file2.txt should remain on disk"
    );
    let ls_files2 = run_cmd(&ox, &["ls-files"], root);
    assert!(
        !ls_files2.contains("file2.txt"),
        "file2.txt should be removed from index"
    );

    // 3. Test ox mv
    run_cmd(&ox, &["mv", "file3.txt", "renamed3.txt"], root);
    assert!(!root.join("file3.txt").exists());
    assert!(root.join("renamed3.txt").exists());
    let ls_files3 = run_cmd(&ox, &["ls-files"], root);
    assert!(ls_files3.contains("renamed3.txt"));
    assert!(!ls_files3.contains("file3.txt"));

    // 4. Test ox restore (restore working tree from index)
    fs::write(root.join("renamed3.txt"), "modified corrupted content\n").unwrap();
    run_cmd(&ox, &["restore", "renamed3.txt"], root);
    let restored = fs::read_to_string(root.join("renamed3.txt")).unwrap();
    assert_eq!(restored.replace("\r\n", "\n"), "hello file 3\n");
}

#[test]
fn test_ox_stash_push_list_pop() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let ox = ox_bin();

    run_cmd(&ox, &["init"], root);
    run_cmd("git", &["config", "user.name", "Tester"], root);
    run_cmd("git", &["config", "user.email", "tester@example.com"], root);

    fs::write(root.join("app.rs"), "fn original() {}\n").unwrap();
    run_cmd(&ox, &["add", "."], root);
    run_cmd(&ox, &["commit", "-m", "Commit 1"], root);

    // Modify file
    fs::write(
        root.join("app.rs"),
        "fn original() {}\nfn experimental() {}\n",
    )
    .unwrap();

    // Stash changes
    let stash_out = run_cmd(&ox, &["stash", "push", "-m", "experimenting"], root);
    assert!(stash_out.contains("Saved working directory"));

    // Working directory should be clean (reverted to Commit 1)
    let app_clean = fs::read_to_string(root.join("app.rs")).unwrap();
    assert_eq!(app_clean.replace("\r\n", "\n"), "fn original() {}\n");

    // Stash list
    let list_out = run_cmd(&ox, &["stash", "list"], root);
    assert!(list_out.contains("stash@{0}"));

    // Stash pop
    let pop_out = run_cmd(&ox, &["stash", "pop"], root);
    assert!(pop_out.contains("Dropped refs/stash@{0}"));

    // Verify modified content restored
    let app_restored = fs::read_to_string(root.join("app.rs")).unwrap();
    assert!(app_restored.contains("experimental"));
}

#[test]
fn test_ox_rebase_and_cherry_pick() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let ox = ox_bin();

    run_cmd(&ox, &["init"], root);
    run_cmd("git", &["config", "user.name", "Tester"], root);
    run_cmd("git", &["config", "user.email", "tester@example.com"], root);

    // Commit on master
    fs::write(root.join("base.txt"), "base content\n").unwrap();
    run_cmd(&ox, &["add", "."], root);
    run_cmd(&ox, &["commit", "-m", "Base commit"], root);

    // Create feature branch
    run_cmd(&ox, &["branch", "feature"], root);
    run_cmd(&ox, &["checkout", "feature"], root);

    fs::write(root.join("feature.txt"), "feature code\n").unwrap();
    run_cmd(&ox, &["add", "."], root);
    run_cmd(&ox, &["commit", "-m", "Feature commit"], root);

    // Switch back to master and make a commit
    run_cmd(&ox, &["checkout", "master"], root);
    fs::write(root.join("master.txt"), "master update\n").unwrap();
    run_cmd(&ox, &["add", "."], root);
    run_cmd(&ox, &["commit", "-m", "Master commit"], root);

    // Switch to feature and rebase onto master
    run_cmd(&ox, &["checkout", "feature"], root);
    let rebase_out = run_cmd(&ox, &["rebase", "master"], root);
    assert!(rebase_out.contains("Successfully rebased"));

    // Verify feature has all three files
    assert!(root.join("base.txt").exists());
    assert!(root.join("master.txt").exists());
    assert!(root.join("feature.txt").exists());

    // Verify official git passes fsck on this rebased history
    run_cmd("git", &["fsck", "--full"], root);

    // Test cherry-pick: create a new branch and cherry-pick the feature commit
    run_cmd(&ox, &["checkout", "master"], root);
    run_cmd(&ox, &["branch", "cherry-target"], root);
    run_cmd(&ox, &["checkout", "cherry-target"], root);
    assert!(!root.join("feature.txt").exists());

    run_cmd(&ox, &["cherry-pick", "feature"], root);
    assert!(root.join("feature.txt").exists());
}

#[test]
fn test_ox_tag_and_blame_and_reflog() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let ox = ox_bin();

    run_cmd(&ox, &["init"], root);
    run_cmd("git", &["config", "user.name", "Annotator"], root);
    run_cmd(
        "git",
        &["config", "user.email", "annotator@example.com"],
        root,
    );

    fs::write(
        root.join("poem.txt"),
        "Roses are red\nViolets are blue\nOx is in Rust\n",
    )
    .unwrap();
    run_cmd(&ox, &["add", "."], root);
    run_cmd(&ox, &["commit", "-m", "Poem verse 1"], root);

    // Annotated tag
    run_cmd(&ox, &["tag", "-a", "-m", "Release v1.0", "v1.0"], root);

    // Lightweight tag
    run_cmd(&ox, &["tag", "v1.0-lw"], root);

    // List tags
    let tag_list = run_cmd(&ox, &["tag"], root);
    assert!(tag_list.contains("v1.0"));
    assert!(tag_list.contains("v1.0-lw"));

    // Verify git tag verifies our tags
    let git_tag_list = run_cmd("git", &["tag"], root);
    assert!(git_tag_list.contains("v1.0"));
    assert!(git_tag_list.contains("v1.0-lw"));

    // Blame test
    let blame_out = run_cmd(&ox, &["blame", "poem.txt"], root);
    assert!(blame_out.contains("Roses are red"));
    assert!(blame_out.contains("Annotator"));

    // Reflog test
    let reflog_out = run_cmd(&ox, &["reflog"], root);
    assert!(reflog_out.contains("HEAD@{0}"));
}

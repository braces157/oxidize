//! Comprehensive differential test for Phase 4 porcelain: init, commit, log, diff, rev-parse.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn ox_bin() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_ox") {
        return PathBuf::from(path);
    }
    let mut path = std::env::current_exe().expect("failed to get current_exe");
    path.pop();
    if path.file_name().and_then(|n| n.to_str()) == Some("deps") {
        path.pop();
    }
    path.push(if cfg!(windows) { "ox.exe" } else { "ox" });
    path
}

#[test]
fn test_commit_and_log_interoperable_with_git() {
    let tmp = TempDir::new().unwrap();

    // 1. Initialize repository with ox
    let init = Command::new(ox_bin())
        .arg("init")
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(init.status.success());

    // Configure user in git so real git doesn't complain when committing
    Command::new("git")
        .args(["config", "user.name", "Oxidize Tester"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "tester@oxidize.dev"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    // 2. Create and add file
    let file = tmp.path().join("greeting.txt");
    fs::write(&file, b"line 1\nline 2\nline 3\n").unwrap();

    let add = Command::new(ox_bin())
        .args(["add", "greeting.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(add.status.success());

    // 3. Commit with ox
    let commit1 = Command::new(ox_bin())
        .args(["commit", "-m", "Initial commit from ox"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(commit1.status.success());

    // 4. Verify real git can read the commit via git log
    let git_log = Command::new("git")
        .args(["log", "-1"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(git_log.status.success());
    let log_str = String::from_utf8_lossy(&git_log.stdout);
    assert!(log_str.contains("Initial commit from ox"));

    // 5. Check git rev-parse HEAD matches ox rev-parse HEAD
    let git_head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    let ox_head = Command::new(ox_bin())
        .args(["rev-parse", "HEAD"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&ox_head.stdout).trim(),
        String::from_utf8_lossy(&git_head.stdout).trim()
    );

    // 6. Modify file and check ox diff
    fs::write(&file, b"line 1\nline 2 modified\nline 3\n").unwrap();
    let ox_diff = Command::new(ox_bin())
        .arg("diff")
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(ox_diff.status.success());
    let diff_str = String::from_utf8_lossy(&ox_diff.stdout);
    assert!(diff_str.contains("-line 2"));
    assert!(diff_str.contains("+line 2 modified"));

    // 7. Commit with real git to test reverse compatibility
    let git_add = Command::new("git")
        .args(["add", "greeting.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(git_add.status.success());

    let git_commit = Command::new("git")
        .args(["commit", "-m", "Second commit from real git"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(git_commit.status.success());

    // 8. Verify ox log sees both commits
    let ox_log = Command::new(ox_bin())
        .args(["log", "--oneline"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(ox_log.status.success());
    let ox_log_str = String::from_utf8_lossy(&ox_log.stdout);
    assert!(ox_log_str.contains("Second commit from real git"));
    assert!(ox_log_str.contains("Initial commit from ox"));

    // 9. Verify ox rev-parse HEAD~1 resolves to first commit
    let ox_rev = Command::new(ox_bin())
        .args(["rev-parse", "HEAD~1"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(ox_rev.status.success());
    let first_sha = String::from_utf8_lossy(&ox_head.stdout).trim().to_string();
    assert_eq!(String::from_utf8_lossy(&ox_rev.stdout).trim(), first_sha);
}

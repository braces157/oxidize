//! Differential tests for Phase 5: branching, checkout, fast-forward and 3-way merge with conflicts.

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
fn test_branch_and_checkout_interoperable_with_git() {
    let tmp = TempDir::new().unwrap();

    let init = Command::new(ox_bin())
        .arg("init")
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(init.status.success());

    // 1. Initial commit on master
    let file = tmp.path().join("file.txt");
    fs::write(&file, b"base content\n").unwrap();

    Command::new(ox_bin())
        .args(["add", "file.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new(ox_bin())
        .args(["commit", "-m", "base commit"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    // 2. Create feature branch and switch to it
    let b_out = Command::new(ox_bin())
        .args(["checkout", "-b", "feature"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(b_out.status.success());

    // Verify git branch sees feature branch as active
    let git_b = Command::new("git")
        .arg("branch")
        .current_dir(tmp.path())
        .output()
        .unwrap();
    let b_str = String::from_utf8_lossy(&git_b.stdout);
    assert!(b_str.contains("* feature"));

    // 3. Commit new file on feature
    let f_file = tmp.path().join("feature.txt");
    fs::write(&f_file, b"feature content\n").unwrap();
    Command::new(ox_bin())
        .args(["add", "feature.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new(ox_bin())
        .args(["commit", "-m", "feature commit"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    // 4. Switch back to master
    let switch_master = Command::new(ox_bin())
        .args(["checkout", "master"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(switch_master.status.success());

    // feature.txt should no longer exist in master working directory
    assert!(!f_file.exists());

    // 5. Fast-forward merge feature into master
    let merge = Command::new(ox_bin())
        .args(["merge", "feature"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(merge.status.success());

    // Now feature.txt should exist on master
    assert!(f_file.exists());

    // Verify git log sees both commits on master
    let git_log = Command::new("git")
        .args(["log", "--oneline"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    let log_str = String::from_utf8_lossy(&git_log.stdout);
    assert!(log_str.contains("feature commit"));
    assert!(log_str.contains("base commit"));
}

#[test]
fn test_three_way_merge_with_conflicts() {
    let tmp = TempDir::new().unwrap();

    Command::new(ox_bin())
        .arg("init")
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let doc = tmp.path().join("doc.txt");
    fs::write(&doc, b"common line 1\ncommon line 2\n").unwrap();

    Command::new(ox_bin())
        .args(["add", "doc.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new(ox_bin())
        .args(["commit", "-m", "base commit"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    // Branch dev
    Command::new(ox_bin())
        .args(["checkout", "-b", "dev"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    fs::write(&doc, b"common line 1\nline 2 changed in dev\n").unwrap();
    Command::new(ox_bin())
        .args(["add", "doc.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new(ox_bin())
        .args(["commit", "-m", "dev commit"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    // Switch to master and make conflicting change
    Command::new(ox_bin())
        .args(["checkout", "master"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    fs::write(&doc, b"common line 1\nline 2 changed in master\n").unwrap();
    Command::new(ox_bin())
        .args(["add", "doc.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new(ox_bin())
        .args(["commit", "-m", "master commit"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    // Attempt merge dev into master (exits with non-zero on conflict)
    let merge_out = Command::new(ox_bin())
        .args(["merge", "dev"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        !merge_out.status.success(),
        "merge with conflict should exit with non-zero status"
    );
    let merge_stdout = String::from_utf8_lossy(&merge_out.stdout);
    assert!(merge_stdout.contains("CONFLICT"));

    // Check conflict markers in doc.txt
    let content = fs::read_to_string(&doc).unwrap();
    assert!(content.contains("<<<<<<< HEAD"));
    assert!(content.contains("line 2 changed in master"));
    assert!(content.contains("======="));
    assert!(content.contains("line 2 changed in dev"));
    assert!(content.contains(">>>>>>> dev"));
}

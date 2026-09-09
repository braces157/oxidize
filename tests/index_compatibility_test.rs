//! Differential tests verifying index binary compatibility between `ox` and official `git`.

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
fn test_ox_add_and_git_status_porcelain() {
    let tmp = TempDir::new().unwrap();

    // 1. Init repo with real git
    let git_init = Command::new("git")
        .arg("init")
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(git_init.status.success());

    // 2. Create test files
    let file1 = tmp.path().join("hello.txt");
    fs::write(&file1, b"hello world\n").unwrap();

    let sub_dir = tmp.path().join("subdir");
    fs::create_dir_all(&sub_dir).unwrap();
    let file2 = sub_dir.join("nested.txt");
    fs::write(&file2, b"nested content\n").unwrap();

    // 3. Stage files using `ox add`
    let ox_add = Command::new(ox_bin())
        .args(["add", "hello.txt", "subdir/nested.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        ox_add.status.success(),
        "ox add failed: {}",
        String::from_utf8_lossy(&ox_add.stderr)
    );

    // 4. Verify real git status reports both files staged (A)
    let git_status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(git_status.status.success());

    let status_str = String::from_utf8_lossy(&git_status.stdout);
    assert!(
        status_str.contains("A  hello.txt"),
        "expected A hello.txt in git status, got:\n{}",
        status_str
    );
    assert!(
        status_str.contains("A  subdir/nested.txt"),
        "expected A subdir/nested.txt in git status, got:\n{}",
        status_str
    );
}

#[test]
fn test_git_add_and_ox_ls_files() {
    let tmp = TempDir::new().unwrap();

    let git_init = Command::new("git")
        .arg("init")
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(git_init.status.success());

    let file1 = tmp.path().join("alpha.txt");
    fs::write(&file1, b"alpha").unwrap();
    let file2 = tmp.path().join("beta.txt");
    fs::write(&file2, b"beta").unwrap();

    // Stage with git
    let git_add = Command::new("git")
        .args(["add", "."])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(git_add.status.success());

    // Read with ox ls-files -s
    let ox_ls = Command::new(ox_bin())
        .args(["ls-files", "-s"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(ox_ls.status.success());

    // Read with git ls-files -s
    let git_ls = Command::new("git")
        .args(["ls-files", "-s"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(git_ls.status.success());

    assert_eq!(
        String::from_utf8_lossy(&ox_ls.stdout).trim(),
        String::from_utf8_lossy(&git_ls.stdout).trim()
    );
}

#[test]
fn test_ox_write_tree_matches_git_write_tree() {
    let tmp = TempDir::new().unwrap();

    let init = Command::new(ox_bin())
        .arg("init")
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(init.status.success());

    // Create complex nested structure
    fs::write(tmp.path().join("root.txt"), b"root file\n").unwrap();
    let src_dir = tmp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("main.rs"), b"fn main() {}\n").unwrap();

    let util_dir = src_dir.join("util");
    fs::create_dir_all(&util_dir).unwrap();
    fs::write(util_dir.join("helper.rs"), b"pub fn help() {}\n").unwrap();

    // Stage all with ox
    let ox_add = Command::new(ox_bin())
        .args(["add", "."])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(ox_add.status.success());

    // Run ox write-tree
    let ox_wt = Command::new(ox_bin())
        .arg("write-tree")
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(ox_wt.status.success());
    let ox_tree_sha = String::from_utf8_lossy(&ox_wt.stdout).trim().to_string();

    // Run git write-tree on the exact same index
    let git_wt = Command::new("git")
        .arg("write-tree")
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(git_wt.status.success());
    let git_tree_sha = String::from_utf8_lossy(&git_wt.stdout).trim().to_string();

    assert_eq!(
        ox_tree_sha, git_tree_sha,
        "ox write-tree must produce identical tree SHA-1 to git write-tree"
    );
}

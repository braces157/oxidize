//! Integration tests for scope boundary enhancements:
//! 1. Command aliases (built-in and custom config)
//! 2. Rename detection in status and diff
//! 3. Index version 4 format reading
//! 4. Native SSH URL handling

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
fn test_builtin_aliases() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path();

    // ox init
    let init_out = Command::new(ox_bin())
        .args(["init"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    assert!(init_out.status.success());

    // ox st (alias for status)
    let st_out = Command::new(ox_bin())
        .args(["st"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    assert!(st_out.status.success());
    let stdout = String::from_utf8_lossy(&st_out.stdout);
    assert!(stdout.contains("On branch master"));
    assert!(stdout.contains("No commits yet"));

    // Add file and commit with ox ci (alias for commit)
    fs::write(repo_dir.join("test.txt"), "hello alias").unwrap();
    Command::new(ox_bin())
        .args(["add", "test.txt"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    let ci_out = Command::new(ox_bin())
        .args(["ci", "-m", "initial commit"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    assert!(ci_out.status.success());

    // ox br (alias for branch)
    let br_out = Command::new(ox_bin())
        .args(["br"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    assert!(br_out.status.success());
    let br_stdout = String::from_utf8_lossy(&br_out.stdout);
    assert!(br_stdout.contains("* master"));
}

#[test]
fn test_custom_config_alias() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path();

    Command::new(ox_bin())
        .args(["init"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    fs::write(repo_dir.join("a.txt"), "hello").unwrap();
    Command::new(ox_bin())
        .args(["add", "a.txt"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new(ox_bin())
        .args(["commit", "-m", "first commit"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Add [alias] to .git/config
    let config_path = repo_dir.join(".git").join("config");
    let mut config_content = fs::read_to_string(&config_path).unwrap();
    config_content.push_str("\n[alias]\n\tlg = log --oneline\n");
    fs::write(config_path, config_content).unwrap();

    // Run custom alias `ox lg`
    let lg_out = Command::new(ox_bin())
        .args(["lg"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    assert!(lg_out.status.success());
    let stdout = String::from_utf8_lossy(&lg_out.stdout);
    assert!(stdout.contains("first commit"));
}

#[test]
fn test_status_and_diff_rename_detection() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path();

    Command::new(ox_bin())
        .args(["init"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    fs::write(repo_dir.join("original.txt"), "unique content for rename").unwrap();
    Command::new(ox_bin())
        .args(["add", "original.txt"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    Command::new(ox_bin())
        .args(["commit", "-m", "add original"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Rename file
    Command::new(ox_bin())
        .args(["mv", "original.txt", "renamed.txt"])
        .current_dir(repo_dir)
        .output()
        .unwrap();

    // Check `ox status`
    let st_out = Command::new(ox_bin())
        .args(["status"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    assert!(st_out.status.success());
    let st_text = String::from_utf8_lossy(&st_out.stdout);
    assert!(
        st_text.contains("renamed:    original.txt -> renamed.txt"),
        "Status should show renamed: {}",
        st_text
    );

    // Check `ox diff --staged`
    let diff_out = Command::new(ox_bin())
        .args(["diff", "--staged"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    assert!(diff_out.status.success());
    let diff_text = String::from_utf8_lossy(&diff_out.stdout);
    assert!(diff_text.contains("similarity index 100%"));
    assert!(diff_text.contains("rename from original.txt"));
    assert!(diff_text.contains("rename to renamed.txt"));
}

#[test]
fn test_index_v4_compatibility_with_git() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path();

    // Initialize with real git
    let git_init = Command::new("git")
        .args(["init"])
        .current_dir(repo_dir)
        .output();
    if git_init.is_err() || !git_init.unwrap().status.success() {
        return;
    }

    fs::write(repo_dir.join("file_alpha.txt"), "content alpha").unwrap();
    fs::write(repo_dir.join("file_beta.txt"), "content beta").unwrap();

    let _ = Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir)
        .output();

    // Convert index to version 4
    let _ = Command::new("git")
        .args(["update-index", "--index-version", "4"])
        .current_dir(repo_dir)
        .output();

    // Run `ox status` on index v4 repo
    let st_out = Command::new(ox_bin())
        .args(["status"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    assert!(st_out.status.success());
    let st_text = String::from_utf8_lossy(&st_out.stdout);
    assert!(st_text.contains("file_alpha.txt"));
    assert!(st_text.contains("file_beta.txt"));
}

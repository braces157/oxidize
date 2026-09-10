use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

fn ox_bin() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_ox") {
        return PathBuf::from(path);
    }
    let mut path = std::env::current_exe().expect("current exe path");
    path.pop();
    if path.file_name().and_then(|n| n.to_str()) == Some("deps") {
        path.pop();
    }
    path.push(if cfg!(windows) { "ox.exe" } else { "ox" });
    path
}

fn run_ox(args: &[&str], cwd: &Path) -> String {
    let output = Command::new(ox_bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|e| panic!("failed to run ox {:?}: {}", args, e));

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!(
            "ox {:?} failed with exit code {:?}\nstdout: {}\nstderr: {}",
            args,
            output.status.code(),
            stdout,
            stderr
        );
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn run_git(args: &[&str], cwd: &Path) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!(
            "git {:?} failed with exit code {:?}\nstdout: {}\nstderr: {}",
            args,
            output.status.code(),
            stdout,
            stderr
        );
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn test_ox_config_and_git_parity() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    run_ox(&["init"], root);

    // 1. Set configuration via ox config
    run_ox(&["config", "user.name", "Alice Engineer"], root);
    run_ox(&["config", "user.email", "alice@example.com"], root);
    run_ox(&["config", "core.bare", "false"], root);

    // 2. Query via ox config
    assert_eq!(run_ox(&["config", "user.name"], root), "Alice Engineer");
    assert_eq!(
        run_ox(&["config", "--get", "user.email"], root),
        "alice@example.com"
    );

    // 3. Official git config should read the exact same values
    assert_eq!(run_git(&["config", "user.name"], root), "Alice Engineer");
    assert_eq!(
        run_git(&["config", "user.email"], root),
        "alice@example.com"
    );

    // 4. Test listing configuration
    let list_out = run_ox(&["config", "--list"], root);
    assert!(list_out.contains("user.name=Alice Engineer"));
    assert!(list_out.contains("user.email=alice@example.com"));

    // 5. Test unset
    run_ox(&["config", "--unset", "user.email"], root);
    let list_after_unset = run_ox(&["config", "--list"], root);
    assert!(!list_after_unset.contains("user.email="));
}

#[test]
fn test_ox_clean() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    run_ox(&["init"], root);
    run_ox(&["config", "user.name", "Tester"], root);
    run_ox(&["config", "user.email", "tester@example.com"], root);

    // Track a committed file
    fs::write(root.join("tracked.txt"), "tracked file\n").unwrap();
    run_ox(&["add", "tracked.txt"], root);
    run_ox(&["commit", "-m", "Initial commit"], root);

    // Create untracked file, untracked directory, and ignored file
    fs::write(root.join(".gitignore"), "*.log\n").unwrap();
    run_ox(&["add", ".gitignore"], root);
    run_ox(&["commit", "-m", "Add gitignore"], root);

    fs::write(root.join("untracked.txt"), "junk file\n").unwrap();
    fs::write(root.join("ignored.log"), "ignore me\n").unwrap();
    let untracked_dir = root.join("junk_dir");
    fs::create_dir_all(&untracked_dir).unwrap();
    fs::write(untracked_dir.join("inner.tmp"), "temp data\n").unwrap();

    // Clean without -f should fail (refuse to clean)
    let refuse_output = Command::new(ox_bin())
        .arg("clean")
        .current_dir(root)
        .output()
        .unwrap();
    assert!(!refuse_output.status.success());

    // Dry run
    let dry_run = run_ox(&["clean", "-n", "-d"], root);
    assert!(dry_run.contains("Would remove untracked.txt"));
    assert!(root.join("untracked.txt").exists());

    // Force clean
    let clean_out = run_ox(&["clean", "-f", "-d"], root);
    assert!(clean_out.contains("Removing untracked.txt"));
    assert!(!root.join("untracked.txt").exists());
    assert!(!untracked_dir.exists());

    // Tracked and ignored files should still exist
    assert!(root.join("tracked.txt").exists());
    assert!(root.join("ignored.log").exists());
}

#[test]
fn test_ox_show_and_rev_list() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    run_ox(&["init"], root);
    run_ox(&["config", "user.name", "Alice"], root);
    run_ox(&["config", "user.email", "alice@example.com"], root);

    fs::write(root.join("hello.txt"), "Hello, World!\n").unwrap();
    run_ox(&["add", "hello.txt"], root);
    run_ox(&["commit", "-m", "Initial commit"], root);

    fs::write(root.join("hello.txt"), "Hello, World!\nSecond line.\n").unwrap();
    run_ox(&["add", "hello.txt"], root);
    run_ox(&["commit", "-m", "Add second line"], root);

    // Test ox show
    let show_out = run_ox(&["show"], root);
    assert!(show_out.contains("commit "));
    assert!(show_out.contains("Author: Alice <alice@example.com>"));
    assert!(show_out.contains("Add second line"));
    assert!(show_out.contains("+Second line."));

    // Test ox rev-list vs git rev-list
    let ox_revs = run_ox(&["rev-list", "HEAD"], root);
    let git_revs = run_git(&["rev-list", "HEAD"], root);
    assert_eq!(ox_revs, git_revs);
}

#[test]
fn test_ox_symbolic_ref_and_update_ref_and_show_ref() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    run_ox(&["init"], root);
    run_ox(&["config", "user.name", "Tester"], root);
    run_ox(&["config", "user.email", "tester@example.com"], root);

    fs::write(root.join("file.txt"), "data\n").unwrap();
    run_ox(&["add", "file.txt"], root);
    run_ox(&["commit", "-m", "Initial commit"], root);

    // 1. Test symbolic-ref read
    let sym_head = run_ox(&["symbolic-ref", "HEAD"], root);
    assert_eq!(sym_head, "refs/heads/master");

    // 2. Test symbolic-ref update
    run_ox(&["symbolic-ref", "HEAD", "refs/heads/development"], root);
    assert_eq!(
        run_ox(&["symbolic-ref", "HEAD"], root),
        "refs/heads/development"
    );

    // Restore to master
    run_ox(&["symbolic-ref", "HEAD", "refs/heads/master"], root);

    // 3. Test update-ref
    let head_oid = run_ox(&["rev-parse", "HEAD"], root);
    run_ox(&["update-ref", "refs/heads/custom-pin", &head_oid], root);
    let custom_oid = run_ox(&["rev-parse", "refs/heads/custom-pin"], root);
    assert_eq!(head_oid, custom_oid);

    // 4. Test show-ref
    let show_ref_out = run_ox(&["show-ref"], root);
    assert!(show_ref_out.contains("refs/heads/master"));
    assert!(show_ref_out.contains("refs/heads/custom-pin"));
}

#[test]
fn test_ox_merge_base_and_read_tree() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    run_ox(&["init"], root);
    run_ox(&["config", "user.name", "Tester"], root);
    run_ox(&["config", "user.email", "tester@example.com"], root);

    // Commit 1 (base)
    fs::write(root.join("file1.txt"), "base\n").unwrap();
    run_ox(&["add", "file1.txt"], root);
    run_ox(&["commit", "-m", "Base commit"], root);
    let base_oid = run_ox(&["rev-parse", "HEAD"], root);

    // Branch A
    run_ox(&["branch", "branch-a"], root);
    run_ox(&["checkout", "branch-a"], root);
    fs::write(root.join("file_a.txt"), "branch a content\n").unwrap();
    run_ox(&["add", "file_a.txt"], root);
    run_ox(&["commit", "-m", "Branch A commit"], root);

    // Branch B
    run_ox(&["checkout", "master"], root);
    run_ox(&["branch", "branch-b"], root);
    run_ox(&["checkout", "branch-b"], root);
    fs::write(root.join("file_b.txt"), "branch b content\n").unwrap();
    run_ox(&["add", "file_b.txt"], root);
    run_ox(&["commit", "-m", "Branch B commit"], root);

    // Test ox merge-base matches git merge-base
    let ox_base = run_ox(&["merge-base", "branch-a", "branch-b"], root);
    let git_base = run_git(&["merge-base", "branch-a", "branch-b"], root);
    assert_eq!(ox_base, base_oid);
    assert_eq!(ox_base, git_base);

    // Test read-tree: switch to a new branch, run read-tree branch-a
    run_ox(&["checkout", "-b", "clean-slate"], root);
    run_ox(&["read-tree", "branch-a"], root);

    let ls_files = run_ox(&["ls-files"], root);
    assert!(ls_files.contains("file1.txt"));
    assert!(ls_files.contains("file_a.txt"));
}

#[test]
fn test_ox_packed_repository_resilience() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    run_ox(&["init"], root);
    run_ox(&["config", "user.name", "Tester"], root);
    run_ox(&["config", "user.email", "tester@example.com"], root);

    // Create 3 commits
    fs::write(root.join("f1.txt"), "first\n").unwrap();
    run_ox(&["add", "f1.txt"], root);
    run_ox(&["commit", "-m", "Commit 1"], root);

    fs::write(root.join("f2.txt"), "second\n").unwrap();
    run_ox(&["add", "f2.txt"], root);
    run_ox(&["commit", "-m", "Commit 2"], root);

    fs::write(root.join("f3.txt"), "third\n").unwrap();
    run_ox(&["add", "f3.txt"], root);
    run_ox(&["commit", "-m", "Commit 3"], root);

    // Pack all loose objects into a packfile and prune loose objects
    run_ox(&["gc"], root);

    // Verify commands work seamlessly on the packed repo
    let rev_head_parent = run_ox(&["rev-parse", "HEAD~1"], root);
    assert_eq!(rev_head_parent.len(), 40);

    let rev_list = run_ox(&["rev-list", "HEAD"], root);
    let lines: Vec<&str> = rev_list.lines().collect();
    assert_eq!(lines.len(), 3);

    let show_out = run_ox(&["show", "HEAD~1"], root);
    assert!(show_out.contains("Commit 2"));

    // Real git also verifies the packed repository integrity
    run_git(&["fsck", "--full"], root);
}

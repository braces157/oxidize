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
fn test_ox_clone_and_remote_interoperability_with_git() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let ox = ox_bin();

    // 1. Set up a source git repository
    let src_dir = root.join("src_repo");
    fs::create_dir_all(&src_dir).unwrap();
    run_cmd("git", &["init", "-b", "main"], &src_dir);
    run_cmd("git", &["config", "user.name", "Original Author"], &src_dir);
    run_cmd(
        "git",
        &["config", "user.email", "author@example.com"],
        &src_dir,
    );

    fs::write(src_dir.join("README.md"), "# Hello from Source Repo\n").unwrap();
    fs::write(
        src_dir.join("hello.rs"),
        "fn main() { println!(\"Hi\"); }\n",
    )
    .unwrap();
    run_cmd("git", &["add", "."], &src_dir);
    run_cmd(
        "git",
        &["commit", "-m", "Initial commit from git"],
        &src_dir,
    );

    // 2. Create bare repository to serve as remote
    let bare_dir = root.join("remote.git");
    run_cmd(
        "git",
        &[
            "clone",
            "--bare",
            src_dir.to_str().unwrap(),
            bare_dir.to_str().unwrap(),
        ],
        root,
    );

    // 3. Clone with ox
    let clone_dir = root.join("ox_clone");
    let clone_out = run_cmd(
        &ox,
        &[
            "clone",
            bare_dir.to_str().unwrap(),
            clone_dir.to_str().unwrap(),
        ],
        root,
    );
    assert!(clone_out.contains("Cloning into"));

    // Verify checked-out files
    let readme = fs::read_to_string(clone_dir.join("README.md")).unwrap();
    assert_eq!(readme, "# Hello from Source Repo\n");
    let code = fs::read_to_string(clone_dir.join("hello.rs")).unwrap();
    assert!(code.contains("println!"));

    // Verify git status inside ox_clone is clean
    let git_status = run_cmd("git", &["status", "--porcelain"], &clone_dir);
    assert!(
        git_status.is_empty(),
        "working tree should be clean after ox clone"
    );

    // Verify git log matches source
    let src_log = run_cmd("git", &["log", "--oneline"], &src_dir);
    let ox_log = run_cmd("git", &["log", "--oneline"], &clone_dir);
    assert_eq!(src_log, ox_log);
}

#[test]
fn test_ox_fetch_pull_and_push() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let ox = ox_bin();

    // 1. Set up bare repository
    let src_dir = root.join("source");
    fs::create_dir_all(&src_dir).unwrap();
    run_cmd("git", &["init", "-b", "main"], &src_dir);
    run_cmd("git", &["config", "user.name", "Dev One"], &src_dir);
    run_cmd(
        "git",
        &["config", "user.email", "dev1@example.com"],
        &src_dir,
    );

    fs::write(src_dir.join("file1.txt"), "Version 1\n").unwrap();
    run_cmd("git", &["add", "."], &src_dir);
    run_cmd("git", &["commit", "-m", "v1 commit"], &src_dir);

    let bare_dir = root.join("upstream.git");
    run_cmd(
        "git",
        &[
            "clone",
            "--bare",
            src_dir.to_str().unwrap(),
            bare_dir.to_str().unwrap(),
        ],
        root,
    );

    // 2. Clone using ox
    let ox_repo = root.join("ox_work");
    run_cmd(
        &ox,
        &[
            "clone",
            bare_dir.to_str().unwrap(),
            ox_repo.to_str().unwrap(),
        ],
        root,
    );

    // 3. Push a new commit from ox
    run_cmd("git", &["config", "user.name", "Ox User"], &ox_repo);
    run_cmd("git", &["config", "user.email", "ox@example.com"], &ox_repo);

    fs::write(ox_repo.join("ox_file.txt"), "Created by ox\n").unwrap();
    run_cmd(&ox, &["add", "ox_file.txt"], &ox_repo);
    run_cmd(&ox, &["commit", "-m", "commit by ox"], &ox_repo);

    let push_out = run_cmd(&ox, &["push", "origin", "main"], &ox_repo);
    assert!(push_out.contains("main -> main"));

    // 4. In source repo, git pull to verify ox's push was successful
    run_cmd(
        "git",
        &["remote", "add", "origin", bare_dir.to_str().unwrap()],
        &src_dir,
    );
    run_cmd("git", &["pull", "origin", "main"], &src_dir);

    let pulled_content = fs::read_to_string(src_dir.join("ox_file.txt")).unwrap();
    assert_eq!(pulled_content.replace("\r\n", "\n"), "Created by ox\n");

    // 5. Test ox remote management
    let _remote_add_out = run_cmd(
        &ox,
        &["remote", "add", "backup", "https://example.com/backup.git"],
        &ox_repo,
    );
    let remotes_list = run_cmd(&ox, &["remote"], &ox_repo);
    assert!(remotes_list.contains("origin"));
    assert!(remotes_list.contains("backup"));

    run_cmd(&ox, &["remote", "remove", "backup"], &ox_repo);
    let remotes_after = run_cmd(&ox, &["remote"], &ox_repo);
    assert!(!remotes_after.contains("backup"));
}

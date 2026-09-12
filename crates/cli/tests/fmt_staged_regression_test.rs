use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn ox_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ox"))
}

fn git(repo: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git must be installed for compatibility tests")
}

#[test]
fn staged_fmt_checks_index_blob_and_preserves_worktree() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    assert!(git(repo, &["init"]).status.success());

    let path = repo.join("demo.rs");
    fs::write(&path, "fn main(){println!(\"x\");}\n").unwrap();
    assert!(git(repo, &["add", "demo.rs"]).status.success());

    let formatted_worktree = "fn main() {\n    println!(\"x\");\n}\n";
    fs::write(&path, formatted_worktree).unwrap();

    let check = Command::new(ox_bin())
        .args(["fmt", "--check", "--staged"])
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        !check.status.success(),
        "check must inspect the unformatted staged blob, not the formatted working tree"
    );

    let format = Command::new(ox_bin())
        .args(["fmt", "--staged"])
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        format.status.success(),
        "staged formatter failed: {}",
        String::from_utf8_lossy(&format.stderr)
    );

    assert_eq!(fs::read_to_string(&path).unwrap(), formatted_worktree);
    let staged = git(repo, &["show", ":demo.rs"]);
    assert!(staged.status.success());
    assert_eq!(
        String::from_utf8(staged.stdout).unwrap(),
        formatted_worktree
    );
}

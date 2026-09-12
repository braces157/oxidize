use oxidize_config::GitIgnore;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn git_ignores(repo: &std::path::Path, path: &str) -> bool {
    Command::new("git")
        .args(["check-ignore", "-q", "--", path])
        .current_dir(repo)
        .status()
        .expect("git must be installed for compatibility tests")
        .success()
}

#[test]
fn escaped_patterns_match_real_git() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    assert!(Command::new("git")
        .arg("init")
        .current_dir(repo)
        .status()
        .unwrap()
        .success());

    let patterns = "\\#literal\n\\!bang\nfile\\*.txt\ntrailing\\ \ncache/\n";
    fs::write(repo.join(".gitignore"), patterns).unwrap();
    fs::write(repo.join("#literal"), "x").unwrap();
    fs::write(repo.join("!bang"), "x").unwrap();
    #[cfg(not(windows))]
    fs::write(repo.join("file*.txt"), "x").unwrap();
    fs::write(repo.join("file123.txt"), "x").unwrap();
    #[cfg(not(windows))]
    fs::write(repo.join("trailing "), "x").unwrap();
    fs::create_dir(repo.join("cache")).unwrap();
    fs::write(repo.join("cache/item.txt"), "x").unwrap();

    let ours = GitIgnore::load_from_dir(repo).unwrap();
    for (path, is_dir) in [
        ("#literal", false),
        ("!bang", false),
        ("file123.txt", false),
        ("cache", true),
        ("cache/item.txt", false),
    ] {
        assert_eq!(
            ours.is_ignored(path, is_dir),
            git_ignores(repo, path),
            "Oxidize disagrees with Git for {path:?}"
        );
    }

    #[cfg(not(windows))]
    assert_eq!(
        ours.is_ignored("file*.txt", false),
        git_ignores(repo, "file*.txt"),
        "Oxidize disagrees with Git for an escaped glob metacharacter"
    );

    #[cfg(not(windows))]
    assert_eq!(
        ours.is_ignored("trailing ", false),
        git_ignores(repo, "trailing "),
        "Oxidize disagrees with Git for an escaped trailing space"
    );
}

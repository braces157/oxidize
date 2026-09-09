//! Differential tests comparing `ox` object plumbing against real `git`.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
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
fn test_hash_object_file_matches_real_git() {
    let tmp = TempDir::new().unwrap();
    let file1 = tmp.path().join("empty.txt");
    fs::write(&file1, b"").unwrap();

    let file2 = tmp.path().join("hello.txt");
    fs::write(&file2, b"hello world\n").unwrap();

    let file3 = tmp.path().join("binary.bin");
    fs::write(&file3, [0, 1, 2, 3, 255, 254, 0, 128]).unwrap();

    for file in [&file1, &file2, &file3] {
        let git_out = Command::new("git")
            .args(["hash-object", file.to_str().unwrap()])
            .output()
            .expect("failed to run git");
        assert!(git_out.status.success());
        let git_hash = String::from_utf8_lossy(&git_out.stdout).trim().to_string();

        let ox_out = Command::new(ox_bin())
            .args(["hash-object", file.to_str().unwrap()])
            .output()
            .expect("failed to run ox");
        assert!(
            ox_out.status.success(),
            "ox stderr: {}",
            String::from_utf8_lossy(&ox_out.stderr)
        );
        let ox_hash = String::from_utf8_lossy(&ox_out.stdout).trim().to_string();

        assert_eq!(ox_hash, git_hash, "Hash mismatch for {:?}", file);
    }
}

#[test]
fn test_hash_object_stdin_matches_real_git() {
    let test_payload = b"Git and Oxidize agree on this string\n";

    let mut git_child = Command::new("git")
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn git");
    git_child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(test_payload)
        .unwrap();
    let git_out = git_child.wait_with_output().unwrap();
    let git_hash = String::from_utf8_lossy(&git_out.stdout).trim().to_string();

    let mut ox_child = Command::new(ox_bin())
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn ox");
    ox_child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(test_payload)
        .unwrap();
    let ox_out = ox_child.wait_with_output().unwrap();
    let ox_hash = String::from_utf8_lossy(&ox_out.stdout).trim().to_string();

    assert_eq!(ox_hash, git_hash);
}

#[test]
fn test_cat_file_and_store_interoperability() {
    let tmp = TempDir::new().unwrap();

    // 1. Initialize repo with ox
    let init_out = Command::new(ox_bin())
        .arg("init")
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(init_out.status.success());

    // 2. Write an object with ox hash-object -w
    let sample_file = tmp.path().join("sample.txt");
    fs::write(&sample_file, b"Rust + Git = Oxidize\n").unwrap();

    let ox_hash_out = Command::new(ox_bin())
        .args(["hash-object", "-w", sample_file.to_str().unwrap()])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(ox_hash_out.status.success());
    let oid = String::from_utf8_lossy(&ox_hash_out.stdout)
        .trim()
        .to_string();

    // 3. Verify real git can read the object written by ox
    let git_cat = Command::new("git")
        .args(["cat-file", "-p", &oid])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(git_cat.status.success());
    assert_eq!(git_cat.stdout, b"Rust + Git = Oxidize\n");

    // 4. Verify ox cat-file -p reads it identically
    let ox_cat = Command::new(ox_bin())
        .args(["cat-file", "-p", &oid])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(ox_cat.status.success());
    assert_eq!(ox_cat.stdout, b"Rust + Git = Oxidize\n");

    // 5. Test -t and -s
    let ox_type = Command::new(ox_bin())
        .args(["cat-file", "-t", &oid])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&ox_type.stdout).trim(), "blob");

    let ox_size = Command::new(ox_bin())
        .args(["cat-file", "-s", &oid])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&ox_size.stdout).trim(), "21");
}

#[test]
fn test_mktree_and_ls_tree_matches_git() {
    let tmp = TempDir::new().unwrap();

    // Initialize with git
    let git_init = Command::new("git")
        .arg("init")
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(git_init.status.success());

    // Write two blobs with git hash-object -w
    let file1 = tmp.path().join("a.txt");
    fs::write(&file1, b"aaa").unwrap();
    let h1_out = Command::new("git")
        .args(["hash-object", "-w", "a.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    let h1 = String::from_utf8_lossy(&h1_out.stdout).trim().to_string();

    let file2 = tmp.path().join("b.txt");
    fs::write(&file2, b"bbb").unwrap();
    let h2_out = Command::new("git")
        .args(["hash-object", "-w", "b.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    let h2 = String::from_utf8_lossy(&h2_out.stdout).trim().to_string();

    // Build mktree input
    let tree_input = format!("100644 blob {}\ta.txt\n100644 blob {}\tb.txt\n", h1, h2);

    // Run git mktree
    let mut git_mktree = Command::new("git")
        .arg("mktree")
        .current_dir(tmp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    git_mktree
        .stdin
        .as_mut()
        .unwrap()
        .write_all(tree_input.as_bytes())
        .unwrap();
    let git_tree_out = git_mktree.wait_with_output().unwrap();
    let git_tree_oid = String::from_utf8_lossy(&git_tree_out.stdout)
        .trim()
        .to_string();

    // Run ox mktree
    let mut ox_mktree = Command::new(ox_bin())
        .arg("mktree")
        .current_dir(tmp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    ox_mktree
        .stdin
        .as_mut()
        .unwrap()
        .write_all(tree_input.as_bytes())
        .unwrap();
    let ox_tree_out = ox_mktree.wait_with_output().unwrap();
    let ox_tree_oid = String::from_utf8_lossy(&ox_tree_out.stdout)
        .trim()
        .to_string();

    assert_eq!(
        ox_tree_oid, git_tree_oid,
        "Tree object ID must match real git mktree"
    );

    // Compare ls-tree output
    let git_ls = Command::new("git")
        .args(["ls-tree", &git_tree_oid])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    let ox_ls = Command::new(ox_bin())
        .args(["ls-tree", &ox_tree_oid])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    assert_eq!(
        String::from_utf8_lossy(&ox_ls.stdout),
        String::from_utf8_lossy(&git_ls.stdout)
    );
}

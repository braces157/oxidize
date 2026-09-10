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
fn test_ox_pack_objects_verified_by_real_git() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let ox = ox_bin();

    // 1. Initialize repo with ox
    run_cmd(&ox, &["init"], root);

    // Configure test identity
    run_cmd("git", &["config", "user.name", "Oxidize Test"], root);
    run_cmd("git", &["config", "user.email", "test@oxidize.rs"], root);

    // 2. Create several files and commits to have objects
    fs::write(root.join("hello.txt"), "Hello, Oxidize!").unwrap();
    run_cmd(&ox, &["add", "hello.txt"], root);
    run_cmd(&ox, &["commit", "-m", "commit 1"], root);

    fs::write(root.join("world.txt"), "A second file for packing.").unwrap();
    fs::write(root.join("hello.txt"), "Hello, Oxidize modified!").unwrap();
    run_cmd(&ox, &["add", "world.txt", "hello.txt"], root);
    run_cmd(&ox, &["commit", "-m", "commit 2"], root);

    // 3. Run ox pack-objects
    let pack_out = run_cmd(&ox, &["pack-objects", "mypack"], root);
    let pack_hash = pack_out.trim();
    assert_eq!(pack_hash.len(), 40);

    let pack_file = root.join(format!("mypack-{}.pack", pack_hash));
    let idx_file = root.join(format!("mypack-{}.idx", pack_hash));
    assert!(pack_file.exists(), "packfile must exist: {:?}", pack_file);
    assert!(idx_file.exists(), "idx file must exist: {:?}", idx_file);

    // 4. Verify with real git verify-pack -v
    let git_verify_out = run_cmd(
        "git",
        &["verify-pack", "-v", idx_file.to_str().unwrap()],
        root,
    );
    assert!(
        git_verify_out.contains(pack_hash)
            || git_verify_out.contains("commit")
            || git_verify_out.contains("blob")
    );

    // 5. Verify with ox verify-pack -v
    let ox_verify_out = run_cmd(
        &ox,
        &["verify-pack", "-v", idx_file.to_str().unwrap()],
        root,
    );
    assert!(ox_verify_out.contains("OK"));
}

#[test]
fn test_ox_verify_pack_on_git_generated_pack() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let ox = ox_bin();

    // 1. Initialize with git
    run_cmd("git", &["init"], root);
    run_cmd("git", &["config", "user.name", "Git User"], root);
    run_cmd("git", &["config", "user.email", "git@example.com"], root);

    // 2. Make commits with git
    fs::write(root.join("file1.txt"), "Sample content 1").unwrap();
    run_cmd("git", &["add", "."], root);
    run_cmd("git", &["commit", "-m", "Initial commit"], root);

    fs::write(root.join("file2.txt"), "Sample content 2").unwrap();
    run_cmd("git", &["add", "."], root);
    run_cmd("git", &["commit", "-m", "Second commit"], root);

    // 3. Repack with real git
    run_cmd("git", &["repack", "-ad"], root);

    // Find git generated idx file in .git/objects/pack/
    let pack_dir = root.join(".git").join("objects").join("pack");
    let mut idx_file = None;
    for entry in fs::read_dir(&pack_dir).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().and_then(|s| s.to_str()) == Some("idx") {
            idx_file = Some(entry.path());
            break;
        }
    }
    let idx_path = idx_file.expect("git must have generated an .idx file");

    // 4. Verify with ox verify-pack -v
    let ox_verify_out = run_cmd(
        &ox,
        &["verify-pack", "-v", idx_path.to_str().unwrap()],
        root,
    );
    assert!(ox_verify_out.contains("OK"));
    assert!(ox_verify_out.contains("commit"));
    assert!(ox_verify_out.contains("blob"));
}

#[test]
fn test_ox_gc_and_fsck_interoperability() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let ox = ox_bin();

    // 1. Initialize repo with ox
    run_cmd(&ox, &["init"], root);
    run_cmd("git", &["config", "user.name", "Ox GC"], root);
    run_cmd("git", &["config", "user.email", "gc@oxidize.rs"], root);

    // 2. Create commits
    fs::write(root.join("a.txt"), "File A").unwrap();
    run_cmd(&ox, &["add", "a.txt"], root);
    run_cmd(&ox, &["commit", "-m", "Commit A"], root);

    fs::write(root.join("b.txt"), "File B").unwrap();
    run_cmd(&ox, &["add", "b.txt"], root);
    run_cmd(&ox, &["commit", "-m", "Commit B"], root);

    // Check ox fsck before gc
    let _fsck_pre = run_cmd(&ox, &["fsck"], root);
    // Should have no missing objects

    // 3. Run ox gc
    let gc_out = run_cmd(&ox, &["gc"], root);
    assert!(gc_out.contains("Packed"));

    // Verify .git/objects/pack contains pack and idx
    let pack_dir = root.join(".git").join("objects").join("pack");
    let mut has_pack = false;
    let mut has_idx = false;
    for entry in fs::read_dir(&pack_dir).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().and_then(|s| s.to_str()) == Some("pack") {
            has_pack = true;
        }
        if p.extension().and_then(|s| s.to_str()) == Some("idx") {
            has_idx = true;
        }
    }
    assert!(has_pack, "packfile must be created in .git/objects/pack");
    assert!(has_idx, "idx file must be created in .git/objects/pack");

    // 4. Verify that real git fsck passes on the repo after ox gc!
    let git_fsck_out = run_cmd("git", &["fsck", "--full"], root);
    assert!(!git_fsck_out.contains("error"));
    assert!(!git_fsck_out.contains("missing"));

    // 5. Verify that ox fsck also passes
    let ox_fsck_out = run_cmd(&ox, &["fsck"], root);
    assert!(!ox_fsck_out.contains("missing"));

    // 6. Verify that ox log and git log can read the packed repository
    let ox_log = run_cmd(&ox, &["log", "--oneline"], root);
    let git_log = run_cmd("git", &["log", "--oneline"], root);
    assert_eq!(ox_log, git_log);
}

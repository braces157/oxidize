use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Instant;
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
fn test_bench_add_throughput() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let ox = ox_bin();

    run_cmd(&ox, &["init"], root);

    // Create 200 files across directories with realistic code content
    let total_files = 200;
    let mut total_bytes = 0usize;

    for i in 0..total_files {
        let sub_dir = root.join(format!("dir_{}", i % 10));
        fs::create_dir_all(&sub_dir).unwrap();

        let file_path = sub_dir.join(format!("source_{}.rs", i));
        let content = format!(
            "// Module {}\npub fn compute_{}() -> usize {{\n    let val = {};\n    val * 42\n}}\n",
            i,
            i,
            i * 100
        )
        .repeat(50); // ~2-3 KB per file

        total_bytes += content.len();
        fs::write(&file_path, content).unwrap();
    }

    let start = Instant::now();
    run_cmd(&ox, &["add", "."], root);
    let elapsed = start.elapsed();

    let total_mb = total_bytes as f64 / (1024.0 * 1024.0);
    let throughput_mb_s = total_mb / elapsed.as_secs_f64();

    println!(
        "\n[BENCHMARK] `ox add` processed {} files ({:.2} MB) in {:.3?} ({:.2} MB/s)",
        total_files, total_mb, elapsed, throughput_mb_s
    );

    // Verify all files were added properly
    let ls_files = run_cmd(&ox, &["ls-files"], root);
    assert_eq!(ls_files.lines().count(), total_files);

    // Verify git agrees that index matches working tree
    let git_status = run_cmd("git", &["status", "--porcelain"], root);
    // All files should be staged (marked with 'A ')
    assert_eq!(
        git_status.lines().filter(|l| l.starts_with("A ")).count(),
        total_files
    );
}

#[test]
fn test_bench_status_speed() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let ox = ox_bin();

    run_cmd(&ox, &["init"], root);

    // Create 100 files and commit them
    for i in 0..100 {
        let p = root.join(format!("file_{}.txt", i));
        fs::write(&p, format!("Initial content for file {}", i)).unwrap();
    }
    run_cmd(&ox, &["add", "."], root);

    run_cmd("git", &["config", "user.name", "Ox Benchmark"], root);
    run_cmd("git", &["config", "user.email", "bench@oxidize.rs"], root);
    run_cmd(&ox, &["commit", "-m", "initial commit"], root);

    // Modify 10 files, delete 5 files, add 10 untracked files
    for i in 0..10 {
        let p = root.join(format!("file_{}.txt", i));
        fs::write(&p, format!("Modified content for file {}", i)).unwrap();
    }
    for i in 10..15 {
        let p = root.join(format!("file_{}.txt", i));
        fs::remove_file(&p).unwrap();
    }
    for i in 0..10 {
        let p = root.join(format!("untracked_{}.txt", i));
        fs::write(&p, "New untracked content").unwrap();
    }

    // Benchmark ox status
    let start_ox = Instant::now();
    let ox_status = run_cmd(&ox, &["status"], root);
    let ox_elapsed = start_ox.elapsed();

    // Benchmark git status
    let start_git = Instant::now();
    let git_status = run_cmd("git", &["status", "--porcelain"], root);
    let git_elapsed = start_git.elapsed();

    println!(
        "\n[BENCHMARK] Status comparison:\n  `ox status`: {:.3?}\n  `git status`: {:.3?}",
        ox_elapsed, git_elapsed
    );

    assert!(ox_status.contains("Changes not staged for commit:"));
    assert!(ox_status.contains("Untracked files:"));
    assert_eq!(git_status.lines().count(), 25);
}

#[test]
fn test_bench_pack_objects_compression_and_speed() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let ox = ox_bin();

    run_cmd(&ox, &["init"], root);
    run_cmd("git", &["config", "user.name", "Ox Benchmark"], root);
    run_cmd("git", &["config", "user.email", "bench@oxidize.rs"], root);

    // Create revisions of large files with small incremental changes (ideal for deltas)
    for rev in 0..20 {
        for f in 0..5 {
            let p = root.join(format!("doc_{}.txt", f));
            let content = format!(
                "Document {} Revision {}\n{}\nTail marker.",
                f,
                rev,
                "The quick brown fox jumps over the lazy dog. ".repeat(100)
            );
            fs::write(&p, content).unwrap();
        }
        run_cmd(&ox, &["add", "."], root);
        run_cmd(&ox, &["commit", "-m", &format!("revision {}", rev)], root);
    }

    // Measure loose object directory size
    let objects_dir = root.join(".git").join("objects");
    let mut loose_size = 0u64;
    for entry in fs::read_dir(&objects_dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name != "pack" && name != "info" {
                for file_entry in fs::read_dir(entry.path()).unwrap() {
                    let file_entry = file_entry.unwrap();
                    loose_size += file_entry.metadata().unwrap().len();
                }
            }
        }
    }

    // Run ox gc with parallel pack generation
    let start_gc = Instant::now();
    run_cmd(&ox, &["gc"], root);
    let gc_elapsed = start_gc.elapsed();

    // Measure packfile size
    let pack_dir = objects_dir.join("pack");
    let mut pack_size = 0u64;
    for entry in fs::read_dir(&pack_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("pack") {
            pack_size += entry.metadata().unwrap().len();
        }
    }

    let compression_ratio = loose_size as f64 / pack_size.max(1) as f64;
    println!(
        "\n[BENCHMARK] `ox gc` (parallel pack generation):\n  Time: {:.3?}\n  Loose size: {} bytes\n  Pack size: {} bytes\n  Compression ratio: {:.2}x",
        gc_elapsed, loose_size, pack_size, compression_ratio
    );

    assert!(pack_size > 0);
    assert!(
        compression_ratio > 1.0,
        "Packfile should be smaller than loose objects"
    );

    // Real git must verify the packfile cleanly
    for entry in fs::read_dir(&pack_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("idx") {
            let verify_out = run_cmd("git", &["verify-pack", "-v", path.to_str().unwrap()], root);
            assert!(verify_out.contains("OK") || !verify_out.is_empty());
        }
    }
}

#[test]
fn test_bench_log_traversal() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let ox = ox_bin();

    run_cmd(&ox, &["init"], root);
    run_cmd("git", &["config", "user.name", "Ox Benchmark"], root);
    run_cmd("git", &["config", "user.email", "bench@oxidize.rs"], root);

    // Create 50 commits
    let num_commits = 50;
    for i in 0..num_commits {
        fs::write(root.join("log_test.txt"), format!("Commit number {}", i)).unwrap();
        run_cmd(&ox, &["add", "log_test.txt"], root);
        run_cmd(
            &ox,
            &["commit", "-m", &format!("commit message {}", i)],
            root,
        );
    }

    // Benchmark ox log
    let start_ox = Instant::now();
    let ox_log = run_cmd(&ox, &["log", "--oneline"], root);
    let ox_elapsed = start_ox.elapsed();

    // Benchmark git log
    let start_git = Instant::now();
    let git_log = run_cmd("git", &["log", "--oneline"], root);
    let git_elapsed = start_git.elapsed();

    println!(
        "\n[BENCHMARK] Log traversal ({} commits):\n  `ox log --oneline`: {:.3?}\n  `git log --oneline`: {:.3?}",
        num_commits, ox_elapsed, git_elapsed
    );

    assert_eq!(ox_log.lines().count(), num_commits);
    assert_eq!(git_log.lines().count(), num_commits);
}

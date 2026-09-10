//! Integration tests for safety boundaries:
//! F01: Path traversal protection in checkout and tree handling.
//! F09: Ref name validation and namespace containment.

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

fn git_env() -> Vec<(&'static str, &'static str)> {
    vec![
        ("GIT_CONFIG_NOSYSTEM", "1"),
        (
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        ),
        ("GIT_AUTHOR_NAME", "Safety Test"),
        ("GIT_AUTHOR_EMAIL", "safety@example.invalid"),
        ("GIT_COMMITTER_NAME", "Safety Test"),
        ("GIT_COMMITTER_EMAIL", "safety@example.invalid"),
    ]
}

fn run_git(dir: &std::path::Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(dir);
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    let output = cmd
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));
    if !output.status.success() {
        panic!(
            "git {:?} failed: {}\nstdout: {}",
            args,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn test_f01_tree_path_traversal_rejected() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    fs::create_dir(&repo_dir).unwrap();

    let mut git_cmd = Command::new("git");
    git_cmd.args(["init", "-b", "main"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        git_cmd.env(k, v);
    }
    assert!(git_cmd.output().unwrap().status.success());

    // Create a base commit
    fs::write(repo_dir.join("a.txt"), "base content\n").unwrap();
    let mut add_cmd = Command::new("git");
    add_cmd.args(["add", "a.txt"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        add_cmd.env(k, v);
    }
    assert!(add_cmd.output().unwrap().status.success());

    let mut ci_cmd = Command::new("git");
    ci_cmd.args(["commit", "-m", "base"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci_cmd.env(k, v);
    }
    assert!(ci_cmd.output().unwrap().status.success());

    // Get blob hash of a.txt
    let rev_out = Command::new("git")
        .args(["rev-parse", "HEAD:a.txt"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    let blob_hex = String::from_utf8_lossy(&rev_out.stdout).trim().to_string();
    let blob_bytes: Vec<u8> = (0..blob_hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&blob_hex[i..i + 2], 16).unwrap())
        .collect();

    // Create a synthetic tree with path traversal "../escaped-tree-sentinel"
    let mut raw_tree = Vec::new();
    raw_tree.extend_from_slice(b"100644 ../escaped-tree-sentinel\0");
    raw_tree.extend_from_slice(&blob_bytes);

    let mut hash_obj = Command::new("git");
    hash_obj
        .args(["hash-object", "--literally", "-w", "-t", "tree", "--stdin"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        hash_obj.env(k, v);
    }
    use std::io::Write;
    let mut child = hash_obj
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(&raw_tree).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let tree_oid = String::from_utf8_lossy(&out.stdout).trim().to_string();

    // Commit that tree to branch "malformed"
    let mut commit_tree = Command::new("git");
    commit_tree
        .args(["commit-tree", &tree_oid, "-m", "malformed tree probe"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        commit_tree.env(k, v);
    }
    let commit_out = commit_tree.output().unwrap();
    assert!(commit_out.status.success());
    let commit_oid = String::from_utf8_lossy(&commit_out.stdout)
        .trim()
        .to_string();

    let mut update_ref = Command::new("git");
    update_ref
        .args(["update-ref", "refs/heads/malformed", &commit_oid])
        .current_dir(&repo_dir);
    assert!(update_ref.output().unwrap().status.success());

    // Now attempt `ox checkout malformed`
    let ox_out = Command::new(ox_bin())
        .args(["checkout", "malformed"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();

    let sentinel_file = temp_dir.path().join("escaped-tree-sentinel");
    assert!(
        !sentinel_file.exists(),
        "CRITICAL SECURITY FAILURE: ox checkout created a file outside the repository!"
    );
    assert!(
        !ox_out.status.success(),
        "ox checkout malformed should have failed, but returned success"
    );
}

#[test]
fn test_f09_ref_traversal_rejected() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    fs::create_dir(&repo_dir).unwrap();

    let mut git_cmd = Command::new("git");
    git_cmd.args(["init", "-b", "main"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        git_cmd.env(k, v);
    }
    assert!(git_cmd.output().unwrap().status.success());

    fs::write(repo_dir.join("a.txt"), "hello\n").unwrap();
    let mut add_cmd = Command::new("git");
    add_cmd.args(["add", "a.txt"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        add_cmd.env(k, v);
    }
    assert!(add_cmd.output().unwrap().status.success());

    let mut ci_cmd = Command::new("git");
    ci_cmd.args(["commit", "-m", "init"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci_cmd.env(k, v);
    }
    assert!(ci_cmd.output().unwrap().status.success());

    // Try creating a branch with directory traversal
    let ox_out = Command::new(ox_bin())
        .args(["branch", "../../review-sentinel"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();

    let sentinel = repo_dir.join(".git").join("review-sentinel");
    assert!(
        !sentinel.exists(),
        "CRITICAL SECURITY FAILURE: ox branch created a file in .git root outside refs/heads!"
    );
    assert!(
        !ox_out.status.success(),
        "ox branch ../../review-sentinel should have failed, but returned success"
    );
}

#[test]
fn test_f04_existing_index_lock_blocks_mutation() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    fs::create_dir(&repo_dir).unwrap();

    let mut git_cmd = Command::new("git");
    git_cmd.args(["init", "-b", "main"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        git_cmd.env(k, v);
    }
    assert!(git_cmd.output().unwrap().status.success());

    let file_path = repo_dir.join("a.txt");
    fs::write(&file_path, "hello\n").unwrap();

    // Create an existing index.lock with sentinel content
    let lock_path = repo_dir.join(".git").join("index.lock");
    fs::write(&lock_path, "existing owner\n").unwrap();

    // Run `ox add a.txt`
    let ox_out = Command::new(ox_bin())
        .args(["add", "a.txt"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();

    assert!(
        !ox_out.status.success(),
        "ox add should have failed when index.lock already exists"
    );
    assert!(
        lock_path.exists(),
        "index.lock should still exist and not have been stolen/deleted"
    );
    let lock_content = fs::read_to_string(&lock_path).unwrap();
    assert_eq!(
        lock_content, "existing owner\n",
        "existing lock content was overwritten or truncated!"
    );
}

#[test]
fn test_f04_existing_ref_lock_blocks_mutation() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    fs::create_dir(&repo_dir).unwrap();

    let mut git_cmd = Command::new("git");
    git_cmd.args(["init", "-b", "main"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        git_cmd.env(k, v);
    }
    assert!(git_cmd.output().unwrap().status.success());

    let file_path = repo_dir.join("a.txt");
    fs::write(&file_path, "hello\n").unwrap();
    let mut add_cmd = Command::new("git");
    add_cmd.args(["add", "a.txt"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        add_cmd.env(k, v);
    }
    assert!(add_cmd.output().unwrap().status.success());

    let mut ci_cmd = Command::new("git");
    ci_cmd.args(["commit", "-m", "init"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci_cmd.env(k, v);
    }
    assert!(ci_cmd.output().unwrap().status.success());

    let rev_out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    let head_oid = String::from_utf8_lossy(&rev_out.stdout).trim().to_string();

    // Create an existing ref lock for refs/heads/main
    let lock_path = repo_dir
        .join(".git")
        .join("refs")
        .join("heads")
        .join("main.lock");
    fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    fs::write(&lock_path, "existing owner\n").unwrap();

    // Run `ox update-ref refs/heads/main <head_oid>`
    let ox_out = Command::new(ox_bin())
        .args(["update-ref", "refs/heads/main", &head_oid])
        .current_dir(&repo_dir)
        .output()
        .unwrap();

    assert!(
        !ox_out.status.success(),
        "ox update-ref should have failed when ref.lock already exists"
    );
    assert!(
        lock_path.exists(),
        "ref.lock should still exist and not have been stolen/deleted"
    );
    let lock_content = fs::read_to_string(&lock_path).unwrap();
    assert_eq!(
        lock_content, "existing owner\n",
        "existing lock content was overwritten or truncated!"
    );
}

#[test]
fn test_f02_dirty_checkout_rejected() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    fs::create_dir(&repo_dir).unwrap();

    let mut git_cmd = Command::new("git");
    git_cmd.args(["init", "-b", "main"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        git_cmd.env(k, v);
    }
    assert!(git_cmd.output().unwrap().status.success());

    fs::write(repo_dir.join("a.txt"), "base\n").unwrap();
    let mut add_cmd = Command::new("git");
    add_cmd.args(["add", "a.txt"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        add_cmd.env(k, v);
    }
    assert!(add_cmd.output().unwrap().status.success());

    let mut ci_cmd = Command::new("git");
    ci_cmd.args(["commit", "-m", "base"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci_cmd.env(k, v);
    }
    assert!(ci_cmd.output().unwrap().status.success());

    // Create branch "other" with different content in a.txt
    let mut branch_cmd = Command::new("git");
    branch_cmd
        .args(["checkout", "-b", "other"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        branch_cmd.env(k, v);
    }
    assert!(branch_cmd.output().unwrap().status.success());

    fs::write(repo_dir.join("a.txt"), "other\n").unwrap();
    let mut ci_other = Command::new("git");
    ci_other
        .args(["commit", "-am", "other"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci_other.env(k, v);
    }
    assert!(ci_other.output().unwrap().status.success());

    // Switch back to main
    let mut checkout_main = Command::new("git");
    checkout_main
        .args(["checkout", "main"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        checkout_main.env(k, v);
    }
    assert!(checkout_main.output().unwrap().status.success());

    // Make precious uncommitted edit in a.txt
    fs::write(repo_dir.join("a.txt"), "precious uncommitted edit\n").unwrap();

    // Now try `ox checkout other`
    let ox_out = Command::new(ox_bin())
        .args(["checkout", "other"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();

    assert!(
        !ox_out.status.success(),
        "ox checkout other should have been rejected due to local uncommitted edits"
    );
    let after_content = fs::read_to_string(repo_dir.join("a.txt")).unwrap();
    assert_eq!(
        after_content, "precious uncommitted edit\n",
        "CRITICAL DATA LOSS: ox checkout overwrote uncommitted modifications!"
    );
}

#[test]
fn test_f03_rm_without_force_rejected() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    fs::create_dir(&repo_dir).unwrap();

    let mut git_cmd = Command::new("git");
    git_cmd.args(["init", "-b", "main"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        git_cmd.env(k, v);
    }
    assert!(git_cmd.output().unwrap().status.success());

    let file_path = repo_dir.join("a.txt");
    fs::write(&file_path, "base\n").unwrap();
    let mut add_cmd = Command::new("git");
    add_cmd.args(["add", "a.txt"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        add_cmd.env(k, v);
    }
    assert!(add_cmd.output().unwrap().status.success());

    let mut ci_cmd = Command::new("git");
    ci_cmd.args(["commit", "-m", "base"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci_cmd.env(k, v);
    }
    assert!(ci_cmd.output().unwrap().status.success());

    // Modify a.txt in worktree without staging
    fs::write(&file_path, "precious uncommitted edit\n").unwrap();

    // ox rm a.txt without -f should fail and keep the file
    let ox_out = Command::new(ox_bin())
        .args(["rm", "a.txt"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();

    assert!(
        !ox_out.status.success(),
        "ox rm a.txt without -f should fail when file has local modifications"
    );
    assert!(
        file_path.exists(),
        "CRITICAL DATA LOSS: ox rm deleted locally modified file without --force!"
    );
    assert_eq!(
        fs::read_to_string(&file_path).unwrap(),
        "precious uncommitted edit\n"
    );
}

#[test]
fn test_f03_recursive_rm_preserves_untracked_collateral() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    fs::create_dir(&repo_dir).unwrap();

    let mut git_cmd = Command::new("git");
    git_cmd.args(["init", "-b", "main"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        git_cmd.env(k, v);
    }
    assert!(git_cmd.output().unwrap().status.success());

    let folder = repo_dir.join("folder");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("tracked.txt"), "tracked").unwrap();

    let mut add_cmd = Command::new("git");
    add_cmd.args(["add", "."]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        add_cmd.env(k, v);
    }
    assert!(add_cmd.output().unwrap().status.success());

    let mut ci_cmd = Command::new("git");
    ci_cmd
        .args(["commit", "-m", "folder"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci_cmd.env(k, v);
    }
    assert!(ci_cmd.output().unwrap().status.success());

    // Add untracked precious file in folder
    let precious = folder.join("precious.txt");
    fs::write(&precious, "untracked work").unwrap();

    // ox rm -r folder
    let ox_out = Command::new(ox_bin())
        .args(["rm", "-r", "folder"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();

    assert!(
        ox_out.status.success(),
        "ox rm -r folder should succeed removing tracked file: {}",
        String::from_utf8_lossy(&ox_out.stderr)
    );
    assert!(
        !folder.join("tracked.txt").exists(),
        "tracked.txt should have been removed"
    );
    assert!(
        precious.exists(),
        "CRITICAL DATA LOSS: ox rm -r deleted unrelated untracked files!"
    );
    assert_eq!(fs::read_to_string(&precious).unwrap(), "untracked work");
}

#[test]
fn test_f08_packed_checkout() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    fs::create_dir(&repo_dir).unwrap();

    let mut git_cmd = Command::new("git");
    git_cmd.args(["init", "-b", "main"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        git_cmd.env(k, v);
    }
    assert!(git_cmd.output().unwrap().status.success());

    fs::write(repo_dir.join("a.txt"), "base\n").unwrap();
    let mut add_cmd = Command::new("git");
    add_cmd.args(["add", "a.txt"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        add_cmd.env(k, v);
    }
    assert!(add_cmd.output().unwrap().status.success());

    let mut ci_cmd = Command::new("git");
    ci_cmd.args(["commit", "-m", "base"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci_cmd.env(k, v);
    }
    assert!(ci_cmd.output().unwrap().status.success());

    // Pack repository and prune loose objects completely
    let mut gc_cmd = Command::new("git");
    gc_cmd.args(["gc", "--prune=now"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        gc_cmd.env(k, v);
    }
    assert!(gc_cmd.output().unwrap().status.success());

    // ox checkout main must succeed!
    let ox_out = Command::new(ox_bin())
        .args(["checkout", "main"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();

    assert!(
        ox_out.status.success(),
        "ox checkout main failed on packed repo: {}",
        String::from_utf8_lossy(&ox_out.stderr)
    );
}

#[test]
fn test_f05_v4_index_mutation_survives_git() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    fs::create_dir(&repo_dir).unwrap();

    let mut git_cmd = Command::new("git");
    git_cmd.args(["init", "-b", "main"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        git_cmd.env(k, v);
    }
    assert!(git_cmd.output().unwrap().status.success());

    fs::write(repo_dir.join("a.txt"), "first\n").unwrap();
    let mut add_cmd = Command::new("git");
    add_cmd.args(["add", "a.txt"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        add_cmd.env(k, v);
    }
    assert!(add_cmd.output().unwrap().status.success());

    // Switch index to version 4 with git
    let mut update_idx = Command::new("git");
    update_idx
        .args(["update-index", "--index-version=4"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        update_idx.env(k, v);
    }
    assert!(update_idx.output().unwrap().status.success());

    // Add a second file using ox
    fs::write(repo_dir.join("b.txt"), "second\n").unwrap();
    let ox_out = Command::new(ox_bin())
        .args(["add", "b.txt"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(
        ox_out.status.success(),
        "ox add b.txt failed: {}",
        String::from_utf8_lossy(&ox_out.stderr)
    );

    // Git must be able to read the index without error!
    let mut ls_files = Command::new("git");
    ls_files
        .args(["ls-files", "--stage"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        ls_files.env(k, v);
    }
    let ls_out = ls_files.output().unwrap();
    assert!(
        ls_out.status.success(),
        "CRITICAL INDEX CORRUPTION: git ls-files failed on ox-mutated v4 index: {}",
        String::from_utf8_lossy(&ls_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&ls_out.stdout);
    assert!(stdout.contains("a.txt"), "missing a.txt in index");
    assert!(stdout.contains("b.txt"), "missing b.txt in index");
}

#[test]
fn test_f13_delta_bounds_and_parser_integrity() {
    use oxidize_pack::delta::apply_delta;
    use oxidize_transport::pkt_line::{read_pkt_lines, PktLine, SidebandDemuxer};

    // 1. Truncated delta copy operand must return Err without panicking
    // Base size = 1, target size = 1, opcode = 0x91 (copy with byte flags 0x01 and 0x10 set, but no operand bytes provided)
    let probe_delta = [1, 1, 0x91];
    let result = std::panic::catch_unwind(|| apply_delta(b"a", &probe_delta));
    assert!(
        result.is_ok(),
        "apply_delta panicked on truncated copy instruction"
    );
    assert!(
        result.unwrap().is_err(),
        "apply_delta should return Err for truncated copy instruction"
    );

    // Test truncated copy for all opcode copy flag bits
    for flag in [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40] {
        let opcode = 0x80 | flag;
        let delta = [1, 1, opcode];
        let res = std::panic::catch_unwind(|| apply_delta(b"a", &delta));
        assert!(res.is_ok(), "apply_delta panicked on opcode {:#x}", opcode);
        assert!(
            res.unwrap().is_err(),
            "apply_delta should fail on truncated opcode {:#x}",
            opcode
        );
    }

    // 2. Truncated pkt-line prefix (1-3 bytes) must be rejected with Err, not accepted
    let truncated_pkt = b"00";
    let pkt_res = read_pkt_lines(&truncated_pkt[..]);
    assert!(
        pkt_res.is_err(),
        "read_pkt_lines accepted truncated 2-byte prefix '00'"
    );

    let truncated_pkt_1 = b"0";
    assert!(
        read_pkt_lines(&truncated_pkt_1[..]).is_err(),
        "read_pkt_lines accepted truncated 1-byte prefix"
    );

    let truncated_pkt_3 = b"004";
    assert!(
        read_pkt_lines(&truncated_pkt_3[..]).is_err(),
        "read_pkt_lines accepted truncated 3-byte prefix"
    );

    // 3. Sideband demuxer must NOT contaminate pack data with NAK negotiation packets
    let frames = vec![
        PktLine::Data(b"NAK\n".to_vec()),
        PktLine::Data(b"\x01PACK".to_vec()),
    ];
    let demux = SidebandDemuxer::from_lines(&frames).expect("demuxer failed");
    assert_eq!(
        demux.pack_data, b"PACK",
        "demuxer contaminated pack_data with NAK packet: {:?}",
        demux.pack_data
    );
}

#[test]
fn test_f11_ssh_hardening() {
    use oxidize_transport::ssh::{
        parse_command_tokens, parse_ssh_url, sq_quote, validate_ssh_endpoint, SshClient,
        SshEndpoint, SshInvocation,
    };

    // 1. Absolute paths in ssh:// URLs must be preserved (not stripped of leading /)
    let ep = parse_ssh_url("ssh://git@example.invalid/absolute/repo.git").unwrap();
    assert_eq!(
        ep.path, "/absolute/repo.git",
        "ssh:// URL stripped leading slash from absolute path"
    );

    // 2. SCP-style paths preserve relative vs absolute paths
    let ep_scp_rel = parse_ssh_url("git@example.invalid:relative/repo.git").unwrap();
    assert_eq!(ep_scp_rel.path, "relative/repo.git");

    let ep_scp_abs = parse_ssh_url("git@example.invalid:/var/git/repo.git").unwrap();
    assert_eq!(ep_scp_abs.path, "/var/git/repo.git");

    // 3. Option-shaped hosts and usernames must be rejected before spawning
    let bad_hosts = [
        "ssh://-oProxyCommand=calc.exe/repo.git",
        "-oProxyCommand=calc.exe:repo.git",
        "ssh://-v@host/repo.git",
        "-v@host:repo.git",
        "ssh://--help/repo.git",
    ];
    for bad in bad_hosts {
        let res = parse_ssh_url(bad);
        assert!(
            res.is_err(),
            "option-shaped host/user '{}' should have been rejected",
            bad
        );
    }

    // Direct endpoint validation rejection
    let evil_endpoint = SshEndpoint {
        host: "-oProxyCommand=calc.exe".to_string(),
        user: None,
        port: None,
        path: "repo.git".to_string(),
    };
    assert!(validate_ssh_endpoint(&evil_endpoint).is_err());

    let evil_user_endpoint = SshEndpoint {
        host: "example.invalid".to_string(),
        user: Some("-oProxyCommand=calc.exe".to_string()),
        port: None,
        path: "repo.git".to_string(),
    };
    assert!(validate_ssh_endpoint(&evil_user_endpoint).is_err());

    // 4. Remote argument quoting must safely escape single quotes and metacharacters
    assert_eq!(sq_quote("simple"), "'simple'");
    assert_eq!(sq_quote("foo'bar"), "'foo'\\''bar'");
    assert_eq!(sq_quote("foo; rm -rf /; bar"), "'foo; rm -rf /; bar'");

    // 5. Command tokenization preserves executable paths with spaces and options
    let tokens =
        parse_command_tokens(r#""C:\Program Files\OpenSSH\ssh.exe" -v -o "BatchMode=yes""#);
    assert_eq!(
        tokens,
        vec![
            r"C:\Program Files\OpenSSH\ssh.exe",
            "-v",
            "-o",
            "BatchMode=yes"
        ]
    );

    // 6. SshClient argument building with recording fake SSH helper
    let temp = tempfile::TempDir::new().unwrap();
    let record_file = temp.path().join("recorded_args.txt");

    // Create a fake SSH script/batch on Windows or shell on Unix
    #[cfg(windows)]
    let fake_ssh = {
        let fake = temp.path().join("fake_ssh.bat");
        fs::write(
            &fake,
            format!("@echo off\r\necho %* > \"{}\"\r\n", record_file.display()),
        )
        .unwrap();
        fake
    };

    #[cfg(not(windows))]
    let fake_ssh = {
        let fake = temp.path().join("fake_ssh.sh");
        fs::write(
            &fake,
            format!("#!/bin/sh\necho \"$@\" > \"{}\"\n", record_file.display()),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        fake
    };

    let invocation = SshInvocation::new(
        fake_ssh.clone(),
        vec!["-i".to_string(), "key_with space".to_string()],
    );
    let client = SshClient::with_invocation(invocation);

    let test_ep = SshEndpoint {
        host: "git.example.invalid".to_string(),
        user: Some("git".to_string()),
        port: Some(2222),
        path: "/srv/git/my repo's.git".to_string(),
    };

    let (prog, args) = client
        .build_command_args(&test_ep, "git-upload-pack")
        .expect("failed to build args");
    assert_eq!(prog, fake_ssh);
    assert_eq!(
        args,
        vec![
            "-i",
            "key_with space",
            "-p",
            "2222",
            "git@git.example.invalid",
            "git-upload-pack '/srv/git/my repo'\\''s.git'"
        ]
    );

    // Ensure evil endpoint rejects before spawning
    assert!(client
        .build_command_args(&evil_endpoint, "git-upload-pack")
        .is_err());
}

#[test]
fn test_f06_unmerged_index_rejected_in_write_tree_and_commit() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    fs::create_dir(&repo_dir).unwrap();

    let mut git_cmd = Command::new("git");
    git_cmd.args(["init", "-b", "main"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        git_cmd.env(k, v);
    }
    assert!(git_cmd.output().unwrap().status.success());

    fs::write(repo_dir.join("f.txt"), "base\n").unwrap();
    let mut add_cmd = Command::new("git");
    add_cmd.args(["add", "f.txt"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        add_cmd.env(k, v);
    }
    assert!(add_cmd.output().unwrap().status.success());

    let mut ci_cmd = Command::new("git");
    ci_cmd.args(["commit", "-m", "base"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci_cmd.env(k, v);
    }
    assert!(ci_cmd.output().unwrap().status.success());

    // Create branch feature and commit change
    let mut br_cmd = Command::new("git");
    br_cmd
        .args(["checkout", "-b", "feature"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        br_cmd.env(k, v);
    }
    assert!(br_cmd.output().unwrap().status.success());

    fs::write(repo_dir.join("f.txt"), "feature content\n").unwrap();
    let mut ci_feat = Command::new("git");
    ci_feat
        .args(["commit", "-am", "feature commit"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci_feat.env(k, v);
    }
    assert!(ci_feat.output().unwrap().status.success());

    // Checkout main and commit conflicting change
    let mut co_main = Command::new("git");
    co_main.args(["checkout", "main"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        co_main.env(k, v);
    }
    assert!(co_main.output().unwrap().status.success());

    fs::write(repo_dir.join("f.txt"), "main content\n").unwrap();
    let mut ci_main = Command::new("git");
    ci_main
        .args(["commit", "-am", "main commit"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci_main.env(k, v);
    }
    assert!(ci_main.output().unwrap().status.success());

    // Cause a merge conflict using git merge
    let mut merge_cmd = Command::new("git");
    merge_cmd.args(["merge", "feature"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        merge_cmd.env(k, v);
    }
    let merge_out = merge_cmd.output().unwrap();
    assert!(!merge_out.status.success(), "expected git merge conflict");

    // ox write-tree MUST fail when index contains unmerged paths
    let wt_out = Command::new(ox_bin())
        .args(["write-tree"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(
        !wt_out.status.success(),
        "CRITICAL ERROR: ox write-tree succeeded on unmerged index!"
    );
    let wt_err = String::from_utf8_lossy(&wt_out.stderr);
    assert!(
        wt_err.contains("unmerged"),
        "error did not mention unmerged: {}",
        wt_err
    );

    // ox commit MUST fail when index contains unmerged paths
    let ci_fail = Command::new(ox_bin())
        .args(["commit", "-m", "unmerged commit"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(
        !ci_fail.status.success(),
        "CRITICAL ERROR: ox commit succeeded on unmerged index!"
    );
}

#[test]
fn test_f06_merge_conflict_records_stages_and_merge_head() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    fs::create_dir(&repo_dir).unwrap();

    let mut git_cmd = Command::new("git");
    git_cmd.args(["init", "-b", "main"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        git_cmd.env(k, v);
    }
    assert!(git_cmd.output().unwrap().status.success());

    fs::write(repo_dir.join("f.txt"), "common base line\n").unwrap();
    let mut add_cmd = Command::new("git");
    add_cmd.args(["add", "f.txt"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        add_cmd.env(k, v);
    }
    assert!(add_cmd.output().unwrap().status.success());

    let mut ci_cmd = Command::new("git");
    ci_cmd.args(["commit", "-m", "base"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci_cmd.env(k, v);
    }
    assert!(ci_cmd.output().unwrap().status.success());

    // Create feature branch
    let mut br_cmd = Command::new("git");
    br_cmd
        .args(["checkout", "-b", "feature"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        br_cmd.env(k, v);
    }
    assert!(br_cmd.output().unwrap().status.success());

    fs::write(repo_dir.join("f.txt"), "feature edit line\n").unwrap();
    let mut ci_feat = Command::new("git");
    ci_feat
        .args(["commit", "-am", "feat edit"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci_feat.env(k, v);
    }
    assert!(ci_feat.output().unwrap().status.success());

    let mut feat_head = Command::new("git");
    feat_head.args(["rev-parse", "HEAD"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        feat_head.env(k, v);
    }
    let feat_oid_str = String::from_utf8(feat_head.output().unwrap().stdout)
        .unwrap()
        .trim()
        .to_string();

    // Checkout main and commit conflicting line
    let mut co_main = Command::new("git");
    co_main.args(["checkout", "main"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        co_main.env(k, v);
    }
    assert!(co_main.output().unwrap().status.success());

    fs::write(repo_dir.join("f.txt"), "main edit line\n").unwrap();
    let mut ci_main = Command::new("git");
    ci_main
        .args(["commit", "-am", "main edit"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci_main.env(k, v);
    }
    assert!(ci_main.output().unwrap().status.success());

    // Run ox merge feature -> MUST fail with exit code != 0
    let merge_out = Command::new(ox_bin())
        .args(["merge", "feature"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(
        !merge_out.status.success(),
        "ox merge should return nonzero exit code on conflicts"
    );

    // Check that .git/MERGE_HEAD was written with feature commit OID
    let merge_head_file = repo_dir.join(".git").join("MERGE_HEAD");
    assert!(
        merge_head_file.exists(),
        ".git/MERGE_HEAD was not created during merge conflict"
    );
    let merge_head_content = fs::read_to_string(&merge_head_file).unwrap();
    assert_eq!(merge_head_content.trim(), feat_oid_str);

    // Check that Git sees unmerged stages 1, 2, and 3
    let mut ls_unmerged = Command::new("git");
    ls_unmerged
        .args(["ls-files", "--unmerged"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        ls_unmerged.env(k, v);
    }
    let ls_out = ls_unmerged.output().unwrap();
    assert!(ls_out.status.success());
    let unmerged_text = String::from_utf8_lossy(&ls_out.stdout);
    assert!(
        unmerged_text.contains(" 1\tf.txt"),
        "stage 1 missing in unmerged index: {}",
        unmerged_text
    );
    assert!(
        unmerged_text.contains(" 2\tf.txt"),
        "stage 2 missing in unmerged index: {}",
        unmerged_text
    );
    assert!(
        unmerged_text.contains(" 3\tf.txt"),
        "stage 3 missing in unmerged index: {}",
        unmerged_text
    );

    // Resolve the conflict
    fs::write(repo_dir.join("f.txt"), "resolved edit line\n").unwrap();
    let add_res = Command::new(ox_bin())
        .args(["add", "f.txt"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(add_res.status.success(), "ox add f.txt failed");

    // Commit the resolved merge
    let ci_res = Command::new(ox_bin())
        .args(["commit", "-m", "resolved merge"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(
        ci_res.status.success(),
        "ox commit failed on resolved merge: {}",
        String::from_utf8_lossy(&ci_res.stderr)
    );

    // MERGE_HEAD must be cleaned up
    assert!(
        !merge_head_file.exists(),
        ".git/MERGE_HEAD was not cleaned up after commit"
    );

    // Verify git cat-file -p HEAD shows TWO parents!
    let mut show_head = Command::new("git");
    show_head
        .args(["cat-file", "-p", "HEAD"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        show_head.env(k, v);
    }
    let cat_head = String::from_utf8_lossy(&show_head.output().unwrap().stdout).to_string();
    let parent_count = cat_head
        .lines()
        .filter(|l| l.starts_with("parent "))
        .count();
    assert_eq!(
        parent_count, 2,
        "merge commit should have 2 parents, got:\n{}",
        cat_head
    );
}

#[test]
fn test_f16_empty_index_commit_deleting_last_file() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    fs::create_dir(&repo_dir).unwrap();

    let mut git_cmd = Command::new("git");
    git_cmd.args(["init", "-b", "main"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        git_cmd.env(k, v);
    }
    assert!(git_cmd.output().unwrap().status.success());

    fs::write(repo_dir.join("only.txt"), "only file\n").unwrap();
    let mut add_cmd = Command::new("git");
    add_cmd.args(["add", "only.txt"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        add_cmd.env(k, v);
    }
    assert!(add_cmd.output().unwrap().status.success());

    let mut ci_cmd = Command::new("git");
    ci_cmd
        .args(["commit", "-m", "add only file"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci_cmd.env(k, v);
    }
    assert!(ci_cmd.output().unwrap().status.success());

    // Remove only.txt using ox rm
    let rm_res = Command::new(ox_bin())
        .args(["rm", "only.txt"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(
        rm_res.status.success(),
        "ox rm only.txt failed: {}",
        String::from_utf8_lossy(&rm_res.stderr)
    );

    // Commit the deletion of the last tracked file
    let ci_res = Command::new(ox_bin())
        .args(["commit", "-m", "delete only file"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(
        ci_res.status.success(),
        "CRITICAL ERROR: ox commit failed on empty index deleting last file: {}",
        String::from_utf8_lossy(&ci_res.stderr)
    );

    // Git log must show 2 commits
    let mut log_cmd = Command::new("git");
    log_cmd
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        log_cmd.env(k, v);
    }
    let count_str = String::from_utf8_lossy(&log_cmd.output().unwrap().stdout)
        .trim()
        .to_string();
    assert_eq!(count_str, "2", "expected 2 commits in git log");
}

#[test]
fn test_f07_stash_stack_middle_drop_and_conflict_preservation() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    fs::create_dir(&repo_dir).unwrap();

    let mut git_cmd = Command::new("git");
    git_cmd.args(["init", "-b", "main"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        git_cmd.env(k, v);
    }
    assert!(git_cmd.output().unwrap().status.success());

    fs::write(repo_dir.join("base.txt"), "base line\n").unwrap();
    fs::write(repo_dir.join("del.txt"), "will delete\n").unwrap();
    let mut add_cmd = Command::new("git");
    add_cmd.args(["add", "."]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        add_cmd.env(k, v);
    }
    assert!(add_cmd.output().unwrap().status.success());

    let mut ci_cmd = Command::new("git");
    ci_cmd
        .args(["commit", "-m", "init base"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci_cmd.env(k, v);
    }
    assert!(ci_cmd.output().unwrap().status.success());

    // 1. Create a 3-entry stash stack
    // Stash 0 (bottom)
    fs::write(repo_dir.join("base.txt"), "stash 0 changes\n").unwrap();
    let s0 = Command::new(ox_bin())
        .args(["stash", "push", "-m", "stash zero"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(s0.status.success());

    // Stash 1 (middle)
    fs::write(repo_dir.join("base.txt"), "stash 1 changes\n").unwrap();
    let s1 = Command::new(ox_bin())
        .args(["stash", "push", "-m", "stash one"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(s1.status.success());

    // Stash 2 (top)
    fs::write(repo_dir.join("base.txt"), "stash 2 changes\n").unwrap();
    let s2 = Command::new(ox_bin())
        .args(["stash", "push", "-m", "stash two"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(s2.status.success());

    // Verify git stash list sees all 3
    let mut list_git = Command::new("git");
    list_git.args(["stash", "list"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        list_git.env(k, v);
    }
    let git_list_out = String::from_utf8_lossy(&list_git.output().unwrap().stdout).to_string();
    assert!(git_list_out.contains("stash@{0}:"));
    assert!(git_list_out.contains("stash@{1}:"));
    assert!(git_list_out.contains("stash@{2}:"));
    assert!(git_list_out.contains("stash two"));
    assert!(git_list_out.contains("stash one"));
    assert!(git_list_out.contains("stash zero"));

    // 2. Middle drop: drop stash@{1} ("stash one")
    let drop_res = Command::new(ox_bin())
        .args(["stash", "drop", "1"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(
        drop_res.status.success(),
        "drop failed: {}",
        String::from_utf8_lossy(&drop_res.stderr)
    );

    // After dropping middle, git stash list must show stash two (idx 0) and stash zero (idx 1)
    let mut list_git2 = Command::new("git");
    list_git2.args(["stash", "list"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        list_git2.env(k, v);
    }
    let git_list_out2 = String::from_utf8_lossy(&list_git2.output().unwrap().stdout).to_string();
    assert!(git_list_out2.contains("stash@{0}:"));
    assert!(git_list_out2.contains("stash@{1}:"));
    assert!(!git_list_out2.contains("stash@{2}:"));
    assert!(git_list_out2.contains("stash two"));
    assert!(git_list_out2.contains("stash zero"));
    assert!(!git_list_out2.contains("stash one"));

    // 3. Staged + tracked deletions preservation in stash
    fs::remove_file(repo_dir.join("del.txt")).unwrap(); // Tracked deletion
    fs::write(repo_dir.join("staged.txt"), "staged content\n").unwrap();
    let mut add_staged = Command::new("git");
    add_staged
        .args(["add", "staged.txt"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        add_staged.env(k, v);
    }
    assert!(add_staged.output().unwrap().status.success());

    let s3 = Command::new(ox_bin())
        .args(["stash", "push", "-m", "staged and deleted"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(s3.status.success());

    // Verify worktree is restored cleanly
    assert!(
        repo_dir.join("del.txt").exists(),
        "del.txt must be restored"
    );
    assert!(
        !repo_dir.join("staged.txt").exists(),
        "staged.txt must be cleaned up from worktree"
    );

    // 4. Pop after unrelated edits:
    fs::write(repo_dir.join("unrelated.txt"), "unrelated work\n").unwrap();
    let pop_res = Command::new(ox_bin())
        .args(["stash", "pop"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(
        pop_res.status.success(),
        "pop failed: {}",
        String::from_utf8_lossy(&pop_res.stderr)
    );

    // Unrelated work is preserved
    assert_eq!(
        fs::read_to_string(repo_dir.join("unrelated.txt")).unwrap(),
        "unrelated work\n"
    );
    // Deleted file was deleted again by stash
    assert!(!repo_dir.join("del.txt").exists());
    // Staged file was recreated by stash
    assert_eq!(
        fs::read_to_string(repo_dir.join("staged.txt")).unwrap(),
        "staged content\n"
    );

    // 5. Conflict preservation:
    // Create local conflict in base.txt
    fs::write(repo_dir.join("base.txt"), "local conflicting line\n").unwrap();
    let pop_conflict = Command::new(ox_bin())
        .args(["stash", "pop"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    // Must fail on conflict!
    assert!(
        !pop_conflict.status.success(),
        "pop with conflict must exit non-zero"
    );

    // The stash must NOT be dropped on conflict!
    let mut list_git3 = Command::new("git");
    list_git3.args(["stash", "list"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        list_git3.env(k, v);
    }
    let git_list_out3 = String::from_utf8_lossy(&list_git3.output().unwrap().stdout).to_string();
    assert!(
        git_list_out3.contains("stash two"),
        "stash two must be preserved in stash stack on conflict"
    );
}

#[test]
fn test_f15_status_staging_and_mode_semantics() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    fs::create_dir(&repo_dir).unwrap();

    let mut git_cmd = Command::new("git");
    git_cmd.args(["init", "-b", "main"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        git_cmd.env(k, v);
    }
    assert!(git_cmd.output().unwrap().status.success());

    // 1. Staged executable bit change detection in ox status
    fs::write(repo_dir.join("a.txt"), "executable test\n").unwrap();
    let mut add1 = Command::new("git");
    add1.args(["add", "a.txt"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        add1.env(k, v);
    }
    assert!(add1.output().unwrap().status.success());

    let mut ci1 = Command::new("git");
    ci1.args(["commit", "-m", "base"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci1.env(k, v);
    }
    assert!(ci1.output().unwrap().status.success());

    // Change mode to +x in index
    let mut chmod_cmd = Command::new("git");
    chmod_cmd
        .args(["update-index", "--chmod=+x", "a.txt"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        chmod_cmd.env(k, v);
    }
    assert!(chmod_cmd.output().unwrap().status.success());

    // ox status must NOT report clean; must report staged modified for a.txt!
    let st1 = Command::new(ox_bin())
        .args(["status"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    let st1_out = String::from_utf8_lossy(&st1.stdout).to_string();
    let st1_err = String::from_utf8_lossy(&st1.stderr).to_string();
    assert!(
        st1_out.contains("Changes to be committed:") && st1_out.contains("modified:   a.txt"),
        "ox status must report mode change as staged modified, status: {}, got stdout: '{}', stderr: '{}'",
        st1.status,
        st1_out,
        st1_err
    );

    // 2. Subdirectory add . scope
    let sub_dir = repo_dir.join("sub");
    fs::create_dir(&sub_dir).unwrap();
    fs::write(sub_dir.join("sub_file.txt"), "sub content\n").unwrap();
    fs::write(repo_dir.join("root_untracked.txt"), "root untracked\n").unwrap();

    let add_sub = Command::new(ox_bin())
        .args(["add", "."])
        .current_dir(&sub_dir)
        .output()
        .unwrap();
    assert!(
        add_sub.status.success(),
        "ox add . in sub failed: {}",
        String::from_utf8_lossy(&add_sub.stderr)
    );

    let mut ls_files = Command::new("git");
    ls_files.args(["ls-files"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        ls_files.env(k, v);
    }
    let ls_out = String::from_utf8_lossy(&ls_files.output().unwrap().stdout).to_string();
    assert!(
        ls_out.contains("sub/sub_file.txt"),
        "sub_file.txt must be staged"
    );
    assert!(
        !ls_out.contains("root_untracked.txt"),
        "root_untracked.txt must NOT be staged when running add . in subdirectory"
    );

    // 3. Tracked deletion staging
    let mut ci2 = Command::new("git");
    ci2.args(["commit", "-m", "add sub_file"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci2.env(k, v);
    }
    assert!(ci2.output().unwrap().status.success());

    // Delete sub_file.txt from disk
    fs::remove_file(sub_dir.join("sub_file.txt")).unwrap();
    let add_del = Command::new(ox_bin())
        .args(["add", "."])
        .current_dir(&sub_dir)
        .output()
        .unwrap();
    assert!(
        add_del.status.success(),
        "ox add . staging deletion failed: {}",
        String::from_utf8_lossy(&add_del.stderr)
    );

    let st2 = Command::new(ox_bin())
        .args(["status"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    let st2_out = String::from_utf8_lossy(&st2.stdout).to_string();
    assert!(
        st2_out.contains("deleted:    sub/sub_file.txt"),
        "ox status must report deleted sub/sub_file.txt, got: {}",
        st2_out
    );

    // 4. Tracked file exception to .gitignore & Hierarchical nested .gitignore
    fs::write(repo_dir.join("app.log"), "log line 1\n").unwrap();
    let mut add_log = Command::new("git");
    add_log.args(["add", "app.log"]).current_dir(&repo_dir);
    for (k, v) in git_env() {
        add_log.env(k, v);
    }
    assert!(add_log.output().unwrap().status.success());

    let mut ci3 = Command::new("git");
    ci3.args(["commit", "-m", "track app.log"])
        .current_dir(&repo_dir);
    for (k, v) in git_env() {
        ci3.env(k, v);
    }
    assert!(ci3.output().unwrap().status.success());

    // Add *.log to root .gitignore
    fs::write(repo_dir.join(".gitignore"), "*.log\n").unwrap();

    // Now modify tracked app.log
    fs::write(repo_dir.join("app.log"), "log line 2\n").unwrap();

    // ox add . must STILL stage the tracked file app.log despite .gitignore!
    let add_tracked = Command::new(ox_bin())
        .args(["add", "."])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(add_tracked.status.success());

    let st3 = Command::new(ox_bin())
        .args(["status"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    let st3_out = String::from_utf8_lossy(&st3.stdout).to_string();
    assert!(
        st3_out.contains("modified:   app.log"),
        "tracked file app.log must be staged modified despite .gitignore, got: {}",
        st3_out
    );

    // 5. Nested .gitignore override: sub/.gitignore with !sub.log
    fs::write(sub_dir.join(".gitignore"), "!sub.log\n").unwrap();
    fs::write(sub_dir.join("sub.log"), "sub log content\n").unwrap();
    fs::write(repo_dir.join("other.log"), "ignored log\n").unwrap();

    // Create a directory containing ONLY ignored files
    let ignored_dir = repo_dir.join("ignored_dir");
    fs::create_dir(&ignored_dir).unwrap();
    fs::write(ignored_dir.join("test.log"), "ignored in dir\n").unwrap();

    let st4 = Command::new(ox_bin())
        .args(["status"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    let st4_out = String::from_utf8_lossy(&st4.stdout).to_string();
    assert!(
        st4_out.contains("sub/"),
        "sub/ has unignored content so it should be listed in untracked files, got: {}",
        st4_out
    );
    assert!(
        !st4_out.contains("ignored_dir"),
        "ignored_dir contains only ignored files so it must NOT be listed in untracked files, got: {}",
        st4_out
    );
    assert!(
        !st4_out.contains("other.log"),
        "other.log should be ignored by root .gitignore, got: {}",
        st4_out
    );

    // Now test that ox add sub/sub.log successfully stages it
    let add_sub_log = Command::new(ox_bin())
        .args(["add", "sub/sub.log"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(
        add_sub_log.status.success(),
        "ox add sub/sub.log failed: {}",
        String::from_utf8_lossy(&add_sub_log.stderr)
    );

    let st5 = Command::new(ox_bin())
        .args(["status"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    let st5_out = String::from_utf8_lossy(&st5.stdout).to_string();
    assert!(
        st5_out.contains("new file:   sub/sub.log"),
        "sub/sub.log should be staged as new file, got: {}",
        st5_out
    );
}

#[test]
fn test_f10_push_safety_and_reachability() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let ox = ox_bin();

    // 1. Create a source repository with an initial commit
    let src_dir = root.join("src_repo");
    fs::create_dir(&src_dir).unwrap();
    run_git(&src_dir, &["init", "-b", "main"]);
    run_git(&src_dir, &["config", "user.name", "Test User"]);
    run_git(&src_dir, &["config", "user.email", "test@example.com"]);
    fs::write(src_dir.join("initial.txt"), "initial commit\n").unwrap();
    run_git(&src_dir, &["add", "."]);
    run_git(&src_dir, &["commit", "-m", "initial"]);

    // 2. Clone destination repository from source repository (non-bare)
    let dest_dir = root.join("dest_repo");
    run_git(
        root,
        &[
            "clone",
            src_dir.to_str().unwrap(),
            dest_dir.to_str().unwrap(),
        ],
    );
    run_git(&dest_dir, &["config", "user.name", "Dest User"]);
    run_git(&dest_dir, &["config", "user.email", "dest@example.com"]);

    // Configure origin remote in src pointing to dest
    run_git(
        &src_dir,
        &["remote", "add", "origin", dest_dir.to_str().unwrap()],
    );

    // Create a new commit in src
    fs::write(src_dir.join("local.txt"), "local commit 1\n").unwrap();
    run_git(&src_dir, &["add", "."]);
    run_git(&src_dir, &["commit", "-m", "local commit 1"]);

    // Scenario A: Pushing to checked out branch of a non-bare repo must be rejected
    let push_checked_out = Command::new(&ox)
        .args(["push", "origin", "main"])
        .current_dir(&src_dir)
        .output()
        .unwrap();
    assert!(
        !push_checked_out.status.success(),
        "push to checked out branch should fail, but succeeded"
    );
    let err_msg = String::from_utf8_lossy(&push_checked_out.stderr).to_string();
    assert!(
        err_msg.contains("refusing to update checked out branch"),
        "error should mention refusing to update checked out branch, got: {}",
        err_msg
    );

    // Verify destination worktree and HEAD are not modified
    assert!(!dest_dir.join("local.txt").exists());

    // Scenario B: Destination switches to another branch, but creates divergent history on main
    run_git(&dest_dir, &["checkout", "-b", "other"]);
    run_git(&dest_dir, &["checkout", "main"]);
    fs::write(dest_dir.join("remote.txt"), "remote commit\n").unwrap();
    run_git(&dest_dir, &["add", "."]);
    run_git(&dest_dir, &["commit", "-m", "remote divergent commit"]);
    let remote_main_before = run_git(&dest_dir, &["rev-parse", "main"]);

    // Switch dest_repo to 'other' so 'main' is no longer checked out
    run_git(&dest_dir, &["checkout", "other"]);

    // Write an unreachable orphan blob in src_repo
    let orphan_output = Command::new("git")
        .args(["hash-object", "-w", "--stdin"])
        .current_dir(&src_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(b"secret orphan blob")
                .unwrap();
            child.wait_with_output()
        })
        .unwrap();
    let orphan_oid = String::from_utf8_lossy(&orphan_output.stdout)
        .trim()
        .to_string();
    assert!(!orphan_oid.is_empty(), "orphan oid should not be empty");

    // Push without --force must fail because destination main has divergent commit
    let push_divergent = Command::new(&ox)
        .args(["push", "origin", "main"])
        .current_dir(&src_dir)
        .output()
        .unwrap();
    assert!(
        !push_divergent.status.success(),
        "push without --force on divergent branch should fail, but succeeded"
    );
    let div_err = String::from_utf8_lossy(&push_divergent.stderr).to_string();
    assert!(
        div_err.contains("rejected") || div_err.contains("fast-forward"),
        "error should mention rejection or fast-forward, got: {}",
        div_err
    );
    let remote_main_after_div = run_git(&dest_dir, &["rev-parse", "main"]);
    assert_eq!(
        remote_main_before, remote_main_after_div,
        "remote main ref must not have changed"
    );

    // Scenario C: Push with --force succeeds on non-checked-out branch
    let push_force = Command::new(&ox)
        .args(["push", "origin", "main", "--force"])
        .current_dir(&src_dir)
        .output()
        .unwrap();
    assert!(
        push_force.status.success(),
        "push --force should succeed, but failed: {}",
        String::from_utf8_lossy(&push_force.stderr)
    );
    let remote_main_after_force = run_git(&dest_dir, &["rev-parse", "main"]);
    let local_main = run_git(&src_dir, &["rev-parse", "main"]);
    assert_eq!(remote_main_after_force, local_main);

    // Verify that the unreachable orphan blob was NOT transferred to dest_repo!
    let cat_orphan = Command::new("git")
        .args(["cat-file", "-e", &orphan_oid])
        .current_dir(&dest_dir)
        .output()
        .unwrap();
    assert!(
        !cat_orphan.status.success(),
        "unreachable orphan blob was transferred to destination repository!"
    );

    // Scenario D: Up-to-date push
    let push_uptodate = Command::new(&ox)
        .args(["push", "origin", "main"])
        .current_dir(&src_dir)
        .output()
        .unwrap();
    assert!(push_uptodate.status.success());
    let uptodate_out = String::from_utf8_lossy(&push_uptodate.stdout);
    assert!(
        uptodate_out.contains("Everything up-to-date"),
        "up-to-date push should report Everything up-to-date, got: {}",
        uptodate_out
    );
}

#[test]
fn test_f12_transport_protocol_state_machine() {
    use oxidize_transport::pkt_line::{
        encode_pkt_line, parse_pkt_line, read_next_pkt_line, PktLine, SidebandDemuxer,
    };
    use oxidize_transport::protocol::{
        negotiate_receive_pack_capabilities, negotiate_upload_pack_capabilities, parse_push_report,
    };

    // 1. Incremental reader handles clean EOF at packet boundary
    let mut empty_stream: &[u8] = b"";
    let next = read_next_pkt_line(&mut empty_stream).unwrap();
    assert!(next.is_none(), "expected clean EOF at boundary");

    // 2. Incremental reader rejects truncated 1, 2, and 3 byte prefixes
    for prefix in [b"0".as_slice(), b"00".as_slice(), b"004".as_slice()] {
        let mut reader = prefix;
        let err = read_next_pkt_line(&mut reader);
        assert!(
            err.is_err(),
            "expected error on truncated prefix {:?}",
            prefix
        );
    }

    // 3. Reject packet lengths exceeding 65524
    let oversized_pkt = b"ffff"; // 65535 > 65524
    let parse_oversized = parse_pkt_line(&oversized_pkt[..]);
    assert!(
        parse_oversized.is_err(),
        "parse_pkt_line should reject length > 65524"
    );

    // 4. SidebandDemuxer::read_stream cleanly demuxes without contaminating pack_data
    let mut stream = Vec::new();
    // Negotiation packets at stream head
    stream.extend_from_slice(&encode_pkt_line(b"NAK\n"));
    // Band 1: pack chunk
    let mut b1 = vec![1u8];
    b1.extend_from_slice(b"PACKDATA");
    stream.extend_from_slice(&encode_pkt_line(&b1));
    // Band 2: progress
    let mut b2 = vec![2u8];
    b2.extend_from_slice(b"remote: Counting objects: 100%\n");
    stream.extend_from_slice(&encode_pkt_line(&b2));
    // Flush packet
    stream.extend_from_slice(b"0000");

    let demux = SidebandDemuxer::read_stream(&stream[..]).unwrap();
    assert_eq!(demux.pack_data, b"PACKDATA");
    assert_eq!(demux.progress.len(), 1);
    assert!(demux.progress[0].contains("Counting objects"));

    // 5. Capability negotiation intersection
    let srv_caps = vec![
        "side-band-64k".to_string(),
        "multi_ack".to_string(),
        "ofs-delta".to_string(),
    ];
    let caps = negotiate_upload_pack_capabilities(&srv_caps);
    assert!(caps.contains(&"side-band-64k".to_string()));
    assert!(caps.contains(&"multi_ack".to_string()));
    assert!(!caps.contains(&"multi_ack_detailed".to_string()));

    let recv_caps = vec!["report-status".to_string()];
    let r_caps = negotiate_receive_pack_capabilities(&recv_caps);
    assert!(r_caps.contains(&"report-status".to_string()));
    assert!(!r_caps.contains(&"side-band-64k".to_string()));

    // 6. Push report parsing
    let report_ok = vec![
        PktLine::Data(b"unpack ok\n".to_vec()),
        PktLine::Data(b"ok refs/heads/main\n".to_vec()),
        PktLine::Flush,
    ];
    let rep = parse_push_report(&report_ok).unwrap();
    assert!(rep.is_success());

    let report_ng = vec![
        PktLine::Data(b"unpack ok\n".to_vec()),
        PktLine::Data(b"ng refs/heads/main [rejected - non-fast-forward]\n".to_vec()),
        PktLine::Flush,
    ];
    let rep_ng = parse_push_report(&report_ng).unwrap();
    assert!(!rep_ng.is_success());
    assert!(!rep_ng.ref_statuses[0].ok);
}

#[test]
fn test_f17_cli_and_tui_branch_deletion_safety() {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    fs::create_dir_all(&repo_dir).unwrap();

    let ox = ox_bin();
    let status = Command::new(&ox)
        .arg("init")
        .current_dir(&repo_dir)
        .status()
        .unwrap();
    assert!(status.success());

    fs::write(repo_dir.join("root.txt"), "initial\n").unwrap();
    let status = Command::new(&ox)
        .args(["add", "root.txt"])
        .current_dir(&repo_dir)
        .status()
        .unwrap();
    assert!(status.success());
    let status = Command::new(&ox)
        .args(["commit", "-m", "initial commit"])
        .current_dir(&repo_dir)
        .status()
        .unwrap();
    assert!(status.success());

    // Create branch unmerged-feat and add unique commit
    let status = Command::new(&ox)
        .args(["branch", "unmerged-feat"])
        .current_dir(&repo_dir)
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(&ox)
        .args(["checkout", "unmerged-feat"])
        .current_dir(&repo_dir)
        .status()
        .unwrap();
    assert!(status.success());

    fs::write(repo_dir.join("feat.txt"), "feat\n").unwrap();
    Command::new(&ox)
        .args(["add", "feat.txt"])
        .current_dir(&repo_dir)
        .status()
        .unwrap();
    Command::new(&ox)
        .args(["commit", "-m", "feat commit"])
        .current_dir(&repo_dir)
        .status()
        .unwrap();

    // Checkout master
    Command::new(&ox)
        .args(["checkout", "master"])
        .current_dir(&repo_dir)
        .status()
        .unwrap();

    // Attempt to delete unmerged branch with -d (should fail)
    let output = Command::new(&ox)
        .args(["branch", "-d", "unmerged-feat"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "ox branch -d must fail on unmerged branch"
    );
    let err_str = String::from_utf8_lossy(&output.stderr);
    assert!(
        err_str.contains("not fully merged"),
        "stderr should mention not fully merged: {}",
        err_str
    );

    // Forced delete with -D succeeds
    let output = Command::new(&ox)
        .args(["branch", "-D", "unmerged-feat"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "ox branch -D must succeed on unmerged branch"
    );
}

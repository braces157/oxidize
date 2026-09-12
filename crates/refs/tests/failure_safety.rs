use oxidize_core::ObjectId;
use oxidize_refs::RefStore;
use std::fs;
use std::str::FromStr;
use tempfile::TempDir;

fn oid(hex: &str) -> ObjectId {
    ObjectId::from_str(hex).unwrap()
}

fn repo() -> (TempDir, RefStore) {
    let temp = TempDir::new().unwrap();
    let git = temp.path().join(".git");
    fs::create_dir_all(git.join("refs/heads")).unwrap();
    fs::create_dir_all(git.join("logs/refs/heads")).unwrap();
    fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    let old = oid("1111111111111111111111111111111111111111");
    fs::write(git.join("refs/heads/main"), format!("{old}\n")).unwrap();
    (temp, RefStore::new(git))
}

#[test]
fn update_ref_rolls_back_when_reflog_creation_fails_after_ref_write() {
    let (temp, store) = repo();
    let git = temp.path().join(".git");
    fs::remove_dir_all(git.join("logs/refs/heads")).unwrap();
    fs::write(git.join("logs/refs/heads"), "blocks directory creation").unwrap();
    let old = store.read_ref("refs/heads/main").unwrap();
    let new = oid("2222222222222222222222222222222222222222");
    assert!(store
        .update_ref("refs/heads/main", &new, Some(&old), "test")
        .is_err());
    assert_eq!(store.read_ref("refs/heads/main").unwrap(), old);
}

#[test]
fn branch_rename_rolls_back_head_and_refs_when_reflog_move_fails() {
    let (temp, store) = repo();
    let git = temp.path().join(".git");
    let old = store.read_ref("refs/heads/main").unwrap();
    fs::write(git.join("logs/refs/heads/main"), "existing log\n").unwrap();
    fs::create_dir_all(git.join("logs/refs/heads/topic")).unwrap();
    assert!(store.rename_branch("main", "topic").is_err());
    assert_eq!(store.read_ref("refs/heads/main").unwrap(), old);
    assert!(store.read_ref("refs/heads/topic").is_err());
    assert_eq!(
        fs::read_to_string(git.join("HEAD")).unwrap(),
        "ref: refs/heads/main\n"
    );
}

#[test]
fn stash_drop_restores_reflog_if_ref_update_fails() {
    let temp = TempDir::new().unwrap();
    let git = temp.path().join(".git");
    fs::create_dir_all(git.join("refs")).unwrap();
    fs::create_dir_all(git.join("logs/refs")).unwrap();
    fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    let first = oid("1111111111111111111111111111111111111111");
    let second = oid("2222222222222222222222222222222222222222");
    let sig = "Test <test@example.com> 0 +0000";
    let log = format!(
        "{} {} {}\tfirst\n{} {} {}\tsecond\n",
        ObjectId::ZERO,
        first,
        sig,
        first,
        second,
        sig
    );
    fs::write(git.join("logs/refs/stash"), &log).unwrap();
    fs::write(git.join("refs/stash"), format!("{second}\n")).unwrap();
    fs::write(git.join("refs/stash.lock"), "busy").unwrap();
    let store = RefStore::new(&git);
    assert!(store.stash_drop(0).is_err());
    assert_eq!(
        fs::read_to_string(git.join("logs/refs/stash")).unwrap(),
        log
    );
    assert_eq!(
        fs::read_to_string(git.join("refs/stash")).unwrap(),
        format!("{second}\n")
    );
}

#[test]
fn symbolic_remote_head_is_resolved_when_listing_remotes() {
    let temp = TempDir::new().unwrap();
    let git = temp.path().join(".git");
    fs::create_dir_all(git.join("refs/remotes/origin")).unwrap();
    fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    let target = oid("3333333333333333333333333333333333333333");
    fs::write(git.join("refs/remotes/origin/main"), format!("{target}\n")).unwrap();
    fs::write(
        git.join("refs/remotes/origin/HEAD"),
        "ref: refs/remotes/origin/main\n",
    )
    .unwrap();

    let remotes = RefStore::new(&git).list_remotes().unwrap();
    assert_eq!(remotes.get("origin/main"), Some(&target));
    assert_eq!(remotes.get("origin/HEAD"), Some(&target));
}

#[test]
fn delete_ref_preserves_other_peeled_packed_ref_lines() {
    let temp = TempDir::new().unwrap();
    let git = temp.path().join(".git");
    fs::create_dir_all(&git).unwrap();
    let branch = oid("1111111111111111111111111111111111111111");
    let tag = oid("2222222222222222222222222222222222222222");
    let peeled = oid("3333333333333333333333333333333333333333");
    fs::write(
        git.join("packed-refs"),
        format!(
            "# pack-refs with: peeled fully-peeled\n{branch} refs/remotes/origin/main\n{tag} refs/tags/v1\n^{peeled}\n"
        ),
    )
    .unwrap();

    let store = RefStore::new(&git);
    store.delete_ref("refs/remotes/origin/main").unwrap();
    let packed = fs::read_to_string(git.join("packed-refs")).unwrap();
    assert!(!packed.contains("refs/remotes/origin/main"));
    assert!(packed.contains(&format!("{tag} refs/tags/v1")));
    assert!(packed.contains(&format!("^{peeled}")));
}

#[test]
fn delete_ref_rejects_malformed_packed_refs_before_removing_loose_ref() {
    let temp = TempDir::new().unwrap();
    let git = temp.path().join(".git");
    fs::create_dir_all(git.join("refs/remotes/origin")).unwrap();
    let target = oid("4444444444444444444444444444444444444444");
    let loose = git.join("refs/remotes/origin/main");
    fs::write(&loose, format!("{target}\n")).unwrap();
    fs::write(git.join("packed-refs"), "this-is-not-a-packed-ref\n").unwrap();

    let store = RefStore::new(&git);
    assert!(store.delete_ref("refs/remotes/origin/main").is_err());
    assert_eq!(fs::read_to_string(loose).unwrap(), format!("{target}\n"));
}

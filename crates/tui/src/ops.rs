use crate::model::{ConflictChoice, ReflogItem, RemoteItem, ResetMode, StashItem, TagItem};
use crate::sequencer::{RebaseAction, RebaseTodoItem, SequencerState, SequencerStatus};
use crate::TuiError;
use oxidize_config::GitConfig;
use oxidize_core::id::ObjectId;
use oxidize_core::object::{Blob, Commit, Object, Signature};
use oxidize_core::store::LooseObjectStore;
use oxidize_core::LockFile;
use oxidize_diff::{three_way_merge, CustomPatchBasket};
use oxidize_index::{flatten_tree, write_tree, Index, IndexEntry};
use oxidize_pack::{PackIndex, RepoObjectStore};
use oxidize_refs::{RefError, RefStore};
use oxidize_transport::{
    discover_local_refs, fetch_local_pack, is_ssh_url, resolve_local_path, SmartHttpClient,
    SshClient,
};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fs;
use std::path::Path;

fn resolve_common_dir(git_dir: &Path) -> std::path::PathBuf {
    let commondir_file = git_dir.join("commondir");
    if commondir_file.is_file() {
        if let Ok(rel) = fs::read_to_string(&commondir_file) {
            let rel = rel.trim();
            if !rel.is_empty() {
                let joined = git_dir.join(rel);
                if let Ok(canon) = joined.canonicalize() {
                    return canon;
                }
            }
        }
    }
    git_dir.to_path_buf()
}

fn load_repo_config(git_dir: &Path) -> Result<GitConfig, TuiError> {
    GitConfig::load_from_file(git_dir.join("config"))
        .map_err(|error| TuiError::Terminal(format!("cannot read repository config: {}", error)))
}

fn rollback_config(
    config_path: &Path,
    original: &[u8],
    operation_error: impl std::fmt::Display,
) -> TuiError {
    let restore = (|| -> Result<(), TuiError> {
        let mut lock =
            LockFile::acquire(config_path).map_err(|e| TuiError::Terminal(e.to_string()))?;
        use std::io::Write;
        lock.write_all(original)?;
        lock.commit().map_err(|e| TuiError::Terminal(e.to_string()))
    })();
    match restore {
        Ok(()) => TuiError::Terminal(operation_error.to_string()),
        Err(rollback_error) => TuiError::Terminal(format!(
            "{}; restoring repository config also failed: {}",
            operation_error, rollback_error
        )),
    }
}

fn remove_file_if_exists(path: &Path) -> Result<(), TuiError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn remove_dir_all_if_exists(path: &Path) -> Result<(), TuiError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn collect_untracked_files(
    repo_root: &Path,
    dir: &Path,
    gitignore: &oxidize_config::GitIgnore,
    out: &mut Vec<String>,
) -> Result<(), TuiError> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name == ".git" {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(repo_root) {
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let is_dir = path.is_dir();
            if gitignore.is_ignored(&rel_str, is_dir) {
                continue;
            }
            if is_dir {
                collect_untracked_files(repo_root, &path, gitignore, out)?;
            } else if path.is_file() {
                out.push(rel_str);
            }
        }
    }
    Ok(())
}

/// Reads blob text from store, returning an empty string if reading fails.
pub fn read_blob_text(store: &RepoObjectStore, oid: &ObjectId) -> String {
    if let Ok(Object::Blob(b)) = store.read_object(oid) {
        String::from_utf8_lossy(&b.data).to_string()
    } else {
        String::new()
    }
}

fn read_blob_text_required(store: &RepoObjectStore, oid: &ObjectId) -> Result<String, TuiError> {
    match store
        .read_object(oid)
        .map_err(|e| TuiError::Terminal(e.to_string()))?
    {
        Object::Blob(blob) => Ok(String::from_utf8_lossy(&blob.data).into_owned()),
        _ => Err(TuiError::Terminal(format!(
            "object '{}' referenced by the index is not a blob",
            oid
        ))),
    }
}

/// Stages multiple paths in a single atomic index transaction, supporting single files, deletions, and directories recursively.
pub fn stage_paths(
    repo_root: &Path,
    git_dir: &Path,
    paths: &[impl AsRef<str>],
) -> Result<(), TuiError> {
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;

    let common_dir = resolve_common_dir(git_dir);
    let store = LooseObjectStore::new(common_dir.join("objects"));

    let gitignore = oxidize_config::GitIgnore::load_from_dir(repo_root)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let mut files_to_stage = Vec::new();
    let mut deletions = Vec::new();

    for p in paths {
        let rel = p.as_ref();
        let full = match oxidize_core::safe_join(repo_root, rel) {
            Ok(f) => f,
            Err(e) => return Err(TuiError::Terminal(e.to_string())),
        };

        if full.is_file() {
            files_to_stage.push(rel.to_string());
        } else if full.is_dir() {
            collect_untracked_files(repo_root, &full, &gitignore, &mut files_to_stage)?;
        } else if !full.exists() {
            deletions.push(rel.to_string());
        }
    }

    // Apply deletions
    for del in deletions {
        index.remove_entry(&del);
    }

    // Apply files
    files_to_stage.sort();
    files_to_stage.dedup();

    for rel_path in files_to_stage {
        let full_path = oxidize_core::safe_join(repo_root, &rel_path)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        let data = fs::read(&full_path)?;
        let meta = fs::metadata(&full_path)?;
        let blob = Object::Blob(Blob::new(data));
        let oid = store.write_object(&blob)?;
        let entry = IndexEntry::from_fs_metadata(rel_path, oid, &meta, 0);
        index.add_entry(entry);
    }

    index
        .write_to(&index_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    Ok(())
}

/// Stages a single file or directory.
pub fn stage_path(repo_root: &Path, git_dir: &Path, rel_path: &str) -> Result<(), TuiError> {
    stage_paths(repo_root, git_dir, &[rel_path])
}

/// Unstages multiple files in a single atomic index transaction.
pub fn unstage_paths(
    repo_root: &Path,
    git_dir: &Path,
    paths: &[impl AsRef<str>],
) -> Result<(), TuiError> {
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;

    let ref_store = RefStore::new(git_dir);
    let head_oid_opt = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?
        .1;

    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;

    let head_map = match head_oid_opt {
        Some(oid) => match store
            .read_object(&oid)
            .map_err(|e| TuiError::Terminal(e.to_string()))?
        {
            Object::Commit(commit) => flatten_tree(&store, &commit.tree, "")
                .map_err(|e| TuiError::Terminal(e.to_string()))?,
            _ => return Err(TuiError::Terminal("HEAD is not a commit".to_string())),
        },
        None => BTreeMap::new(),
    };

    for p in paths {
        let rel_path = p.as_ref();
        if let Some((mode, head_oid)) = head_map.get(rel_path) {
            let full_path = match oxidize_core::safe_join(repo_root, rel_path) {
                Ok(f) => f,
                Err(e) => return Err(TuiError::Terminal(e.to_string())),
            };
            let meta = fs::metadata(&full_path).ok();
            let file_size = meta.as_ref().map(|m| m.len() as u32).unwrap_or(0);

            let entry = IndexEntry {
                ctime_sec: 0,
                ctime_nsec: 0,
                mtime_sec: 0,
                mtime_nsec: 0,
                dev: 0,
                ino: 0,
                mode: mode.0,
                uid: 0,
                gid: 0,
                file_size,
                oid: *head_oid,
                stage: 0,
                assume_valid: false,
                path: rel_path.to_string(),
            };
            index.add_entry(entry);
        } else {
            index.remove_entry(rel_path);
        }
    }

    index
        .write_to(&index_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    Ok(())
}

/// Unstages a single file (restores previous HEAD state in index, or removes from index if new).
pub fn unstage_path(repo_root: &Path, git_dir: &Path, rel_path: &str) -> Result<(), TuiError> {
    unstage_paths(repo_root, git_dir, &[rel_path])
}

/// Stages a specific hunk into the index.
pub fn stage_hunk(
    repo_root: &Path,
    git_dir: &Path,
    rel_path: &str,
    hunk: &oxidize_diff::StructuredHunk,
) -> Result<(), TuiError> {
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let common_dir = resolve_common_dir(git_dir);
    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;

    let current_staged_text = if let Some(entry) = index.find_entry(rel_path) {
        read_blob_text_required(&store, &entry.oid)?
    } else {
        String::new()
    };

    let full_path = oxidize_core::safe_join(repo_root, rel_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let worktree_text = if full_path.exists() {
        fs::read_to_string(&full_path).map_err(|e| TuiError::Terminal(e.to_string()))?
    } else {
        String::new()
    };

    let current_hunks =
        oxidize_diff::compute_structured_diff(&current_staged_text, &worktree_text, 3);
    if !current_hunks.contains(hunk) {
        return Err(TuiError::Terminal(
            "Staging rejected: worktree snapshot has changed since hunk was computed".to_string(),
        ));
    }

    let new_staged_text =
        oxidize_diff::apply_hunk_forward(&current_staged_text, hunk).map_err(TuiError::Terminal)?;

    if !full_path.exists() && new_staged_text.is_empty() {
        index.remove_entry(rel_path);
        index
            .write_to(&index_path)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        return Ok(());
    }

    let new_staged_bytes = new_staged_text.into_bytes();
    let file_size = new_staged_bytes.len() as u32;
    let loose = LooseObjectStore::new(common_dir.join("objects"));
    let blob = Object::Blob(Blob::new(new_staged_bytes));
    let new_oid = loose.write_object(&blob)?;

    let mode = index
        .find_entry(rel_path)
        .map(|e| e.mode)
        .unwrap_or(0o100644);

    let entry = IndexEntry {
        ctime_sec: 0,
        ctime_nsec: 0,
        mtime_sec: 0,
        mtime_nsec: 0,
        dev: 0,
        ino: 0,
        mode,
        uid: 0,
        gid: 0,
        file_size,
        oid: new_oid,
        stage: 0,
        assume_valid: false,
        path: rel_path.to_string(),
    };
    index.add_entry(entry);

    index
        .write_to(&index_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    Ok(())
}

/// Unstages a specific hunk from the index.
pub fn unstage_hunk(
    repo_root: &Path,
    git_dir: &Path,
    rel_path: &str,
    hunk: &oxidize_diff::StructuredHunk,
) -> Result<(), TuiError> {
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let common_dir = resolve_common_dir(git_dir);
    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;

    let entry = index
        .find_entry(rel_path)
        .ok_or_else(|| TuiError::Terminal(format!("File '{}' not found in index", rel_path)))?;

    let current_staged_text = read_blob_text_required(&store, &entry.oid)?;

    let new_staged_text =
        oxidize_diff::apply_hunk_reverse(&current_staged_text, hunk).map_err(TuiError::Terminal)?;

    let ref_store = RefStore::with_common_dir(git_dir, &common_dir);
    let head_oid_opt = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?
        .1;
    let head_map = match head_oid_opt {
        Some(oid) => match store
            .read_object(&oid)
            .map_err(|e| TuiError::Terminal(e.to_string()))?
        {
            Object::Commit(commit) => flatten_tree(&store, &commit.tree, "")
                .map_err(|e| TuiError::Terminal(e.to_string()))?,
            _ => return Err(TuiError::Terminal("HEAD is not a commit".to_string())),
        },
        None => BTreeMap::new(),
    };

    if let Some((_mode, head_oid)) = head_map.get(rel_path) {
        let head_text = read_blob_text_required(&store, head_oid)?;
        if head_text == new_staged_text {
            let full_path = oxidize_core::safe_join(repo_root, rel_path)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            let meta = fs::metadata(&full_path).ok();
            let file_size = meta.as_ref().map(|m| m.len() as u32).unwrap_or(0);
            let entry = IndexEntry {
                ctime_sec: 0,
                ctime_nsec: 0,
                mtime_sec: 0,
                mtime_nsec: 0,
                dev: 0,
                ino: 0,
                mode: entry.mode,
                uid: 0,
                gid: 0,
                file_size,
                oid: *head_oid,
                stage: 0,
                assume_valid: false,
                path: rel_path.to_string(),
            };
            index.add_entry(entry);
        } else {
            let new_staged_bytes = new_staged_text.into_bytes();
            let file_size = new_staged_bytes.len() as u32;
            let loose = LooseObjectStore::new(common_dir.join("objects"));
            let blob = Object::Blob(Blob::new(new_staged_bytes));
            let new_oid = loose.write_object(&blob)?;
            let mut updated_entry = entry.clone();
            updated_entry.oid = new_oid;
            updated_entry.file_size = file_size;
            index.add_entry(updated_entry);
        }
    } else if new_staged_text.is_empty() {
        index.remove_entry(rel_path);
    } else {
        let new_staged_bytes = new_staged_text.into_bytes();
        let file_size = new_staged_bytes.len() as u32;
        let loose = LooseObjectStore::new(common_dir.join("objects"));
        let blob = Object::Blob(Blob::new(new_staged_bytes));
        let new_oid = loose.write_object(&blob)?;
        let mut updated_entry = entry.clone();
        updated_entry.oid = new_oid;
        updated_entry.file_size = file_size;
        index.add_entry(updated_entry);
    }

    index
        .write_to(&index_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    Ok(())
}

/// Discards a specific hunk from the working tree.
pub fn discard_hunk(
    repo_root: &Path,
    rel_path: &str,
    hunk: &oxidize_diff::StructuredHunk,
) -> Result<(), TuiError> {
    let full_path = oxidize_core::safe_join(repo_root, rel_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    if !full_path.exists() {
        return Err(TuiError::Terminal(format!(
            "Working tree file '{}' not found",
            rel_path
        )));
    }

    let worktree_text = fs::read_to_string(&full_path)?;
    let reverted_text =
        oxidize_diff::apply_hunk_reverse(&worktree_text, hunk).map_err(TuiError::Terminal)?;

    fs::write(&full_path, reverted_text.as_bytes())?;
    Ok(())
}

/// Discards unstaged modifications to a file, or removes untracked file.
pub fn discard_path(repo_root: &Path, git_dir: &Path, rel_path: &str) -> Result<(), TuiError> {
    let full_path = oxidize_core::safe_join(repo_root, rel_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let index_path = git_dir.join("index");
    let index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;

    if let Some(entry) = index.find_entry(rel_path) {
        // Tracked file: restore content from index blob
        let store =
            RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
        let blob = match store
            .read_object(&entry.oid)
            .map_err(|e| TuiError::Terminal(e.to_string()))?
        {
            Object::Blob(blob) => blob,
            _ => return Err(TuiError::Terminal("index entry is not a blob".to_string())),
        };
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&full_path, &blob.data)?;
    } else {
        // Untracked file: delete from disk
        if full_path.is_file() {
            fs::remove_file(&full_path)?;
        } else if full_path.is_dir() {
            fs::remove_dir_all(&full_path)?;
        }
    }

    Ok(())
}

/// Retrieves default committer signature from repository or environment.
pub fn get_signature(git_dir: &Path) -> Signature {
    let mut name = "Oxidize User".to_string();
    let mut email = "user@oxidize.dev".to_string();

    let config_path = git_dir.join("config");
    if let Ok(cfg) = GitConfig::load_from_file(&config_path) {
        if let Some(n) = cfg.get("user", None, "name") {
            name = n.to_string();
        }
        if let Some(e) = cfg.get("user", None, "email") {
            email = e.to_string();
        }
    }

    if let Ok(author_name) = std::env::var("GIT_AUTHOR_NAME") {
        name = author_name;
    }
    if let Ok(author_email) = std::env::var("GIT_AUTHOR_EMAIL") {
        email = author_email;
    }

    let now = chrono::Local::now();
    Signature {
        name,
        email,
        time_seconds: now.timestamp(),
        tz_offset: now.format("%z").to_string(),
    }
}

/// Writes staged index to a Tree and creates a new Commit object, advancing current branch.
pub fn create_commit(
    _repo_root: &Path,
    git_dir: &Path,
    message: &str,
) -> Result<ObjectId, TuiError> {
    let common_dir = resolve_common_dir(git_dir);
    let store = LooseObjectStore::new(common_dir.join("objects"));
    let repo_store =
        RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let ref_store = RefStore::new(git_dir);
    let index_path = git_dir.join("index");
    let index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;

    // 1. Reject unmerged paths
    let mut unmerged = Vec::new();
    for entry in &index.entries {
        if entry.stage != 0 {
            unmerged.push(entry.path.clone());
        }
    }
    if !unmerged.is_empty() {
        unmerged.sort();
        unmerged.dedup();
        return Err(TuiError::Terminal(format!(
            "cannot commit: you have unmerged files ({})",
            unmerged.join(", ")
        )));
    }

    // 2. Build tree from index
    let tree_oid = write_tree(&index, &store).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let (branch_name, head_commit_oid) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    // 3. Check for MERGE_HEAD
    let merge_head_file = git_dir.join("MERGE_HEAD");
    let merge_head_oid = if merge_head_file.exists() {
        if let Ok(content) = fs::read_to_string(&merge_head_file) {
            let hex = content.trim();
            if hex.is_empty() {
                None
            } else {
                hex.parse::<ObjectId>().ok()
            }
        } else {
            None
        }
    } else {
        None
    };

    // 4. Check if working tree / index is clean relative to HEAD
    if let Some(ref head_oid) = head_commit_oid {
        if let Ok(Object::Commit(head_commit)) = repo_store.read_object(head_oid) {
            if head_commit.tree == tree_oid && merge_head_oid.is_none() {
                return Err(TuiError::Terminal(
                    "nothing to commit, working tree clean".to_string(),
                ));
            }
        }
    } else if index.entries.is_empty() {
        return Err(TuiError::Terminal(
            "nothing to commit (index is empty)".to_string(),
        ));
    }

    let mut parents = Vec::new();
    if let Some(p) = head_commit_oid {
        parents.push(p);
    }
    if let Some(mp) = merge_head_oid {
        if !parents.contains(&mp) {
            parents.push(mp);
        }
    }

    let sig = get_signature(git_dir);
    let commit = Commit {
        tree: tree_oid,
        parents,
        author: sig.clone(),
        committer: sig,
        gpg_sig: None,
        message: format!("{}\n", message.trim()),
    };

    let commit_oid = store
        .write_object(&Object::Commit(commit))
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let first_line = message.lines().next().unwrap_or("").trim();
    let ref_msg = format!("commit: {}", first_line);
    ref_store
        .update_ref(
            &branch_name,
            &commit_oid,
            head_commit_oid.as_ref(),
            &ref_msg,
        )
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    // Clean up merge state files
    remove_file_if_exists(&git_dir.join("MERGE_HEAD"))?;
    remove_file_if_exists(&git_dir.join("MERGE_MSG"))?;
    remove_file_if_exists(&git_dir.join("MERGE_MODE"))?;

    Ok(commit_oid)
}

/// Traverses commit history to check if `ancestor` is reachable from `descendant`.
pub fn is_ancestor(
    store: &impl oxidize_core::ObjectReader,
    ancestor: &ObjectId,
    descendant: &ObjectId,
) -> Result<bool, TuiError> {
    if ancestor == descendant {
        return Ok(true);
    }
    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();

    queue.push_back(*descendant);
    visited.insert(*descendant);

    while let Some(curr) = queue.pop_front() {
        if curr == *ancestor {
            return Ok(true);
        }
        if let Ok(Object::Commit(c)) = store.read_object(&curr) {
            for p in c.parents {
                if visited.insert(p) {
                    queue.push_back(p);
                }
            }
        }
    }

    Ok(false)
}

/// Checks out a commit tree to the working directory and synchronizes the index.
/// When `force` is false, verifies that local modifications and untracked files will not be clobbered.
pub fn checkout_tree_and_update_index(
    repo_root: &Path,
    git_dir: &Path,
    target_tree_oid: &ObjectId,
    force: bool,
) -> Result<(), TuiError> {
    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let ref_store = RefStore::new(git_dir);

    let target_map =
        flatten_tree(&store, target_tree_oid, "").map_err(|e| TuiError::Terminal(e.to_string()))?;

    // Preflight: validate every target path and index path before mutating anything
    for path in target_map.keys() {
        oxidize_core::safe_join(repo_root, path).map_err(|e| TuiError::Terminal(e.to_string()))?;
    }
    for entry in &index.entries {
        oxidize_core::safe_join(repo_root, &entry.path)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
    }

    let head_tree_oid = match ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?
        .1
    {
        Some(head_oid) => {
            if let Ok(Object::Commit(c)) = store.read_object(&head_oid) {
                Some(c.tree)
            } else {
                None
            }
        }
        None => None,
    };

    let head_map = match head_tree_oid {
        Some(ref oid) => {
            flatten_tree(&store, oid, "").map_err(|e| TuiError::Terminal(e.to_string()))?
        }
        None => BTreeMap::new(),
    };

    if !force {
        let mut dirty_paths = Vec::new();
        let mut untracked_collisions = Vec::new();

        // Collect all paths touched between HEAD and target
        let mut touched_paths = BTreeSet::new();
        for (p, target_val) in &target_map {
            if head_map.get(p) != Some(target_val) {
                touched_paths.insert(p.clone());
            }
        }
        for p in head_map.keys() {
            if !target_map.contains_key(p) {
                touched_paths.insert(p.clone());
            }
        }

        for path in &touched_paths {
            let full_path = match oxidize_core::safe_join(repo_root, path) {
                Ok(p) => p,
                Err(e) => return Err(TuiError::Terminal(e.to_string())),
            };
            if let Some(entry) = index.find_entry(path) {
                if full_path.is_file() {
                    let data = fs::read(&full_path)?;
                    let wt_oid = ObjectId::hash_blob(&data);
                    if wt_oid != entry.oid {
                        dirty_paths.push(path.clone());
                        continue;
                    }
                } else if !full_path.exists() && entry.stage == 0 {
                    dirty_paths.push(path.clone());
                    continue;
                }

                // Check staged changes vs HEAD
                let head_val = head_map.get(path);
                if head_val.map(|(_, oid)| *oid) != Some(entry.oid) {
                    dirty_paths.push(path.clone());
                }
            } else if full_path.exists() {
                // Untracked collision
                untracked_collisions.push(path.clone());
            }
        }

        if !dirty_paths.is_empty() {
            return Err(TuiError::Terminal(format!(
                "error: Your local changes to the following files would be overwritten by checkout:\n\t{}\nPlease commit your changes or stash them before you switch branches.\nAborting",
                dirty_paths.join("\n\t")
            )));
        }

        if !untracked_collisions.is_empty() {
            return Err(TuiError::Terminal(format!(
                "error: The following untracked working tree files would be overwritten by checkout:\n\t{}\nPlease move or remove them before you switch branches.\nAborting",
                untracked_collisions.join("\n\t")
            )));
        }

        // Apply changes: only update paths that changed between HEAD and target
        for path in &touched_paths {
            let full_path = match oxidize_core::safe_join(repo_root, path) {
                Ok(p) => p,
                Err(e) => return Err(TuiError::Terminal(e.to_string())),
            };
            if let Some((_mode, oid)) = target_map.get(path) {
                if let Some(parent) = full_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let blob = match store
                    .read_object(oid)
                    .map_err(|e| TuiError::Terminal(e.to_string()))?
                {
                    Object::Blob(blob) => blob,
                    _ => {
                        return Err(TuiError::Terminal(format!(
                            "checkout expected blob for '{}'",
                            path
                        )))
                    }
                };
                fs::write(&full_path, &blob.data)?;
                let meta = fs::metadata(&full_path)?;
                let entry = IndexEntry::from_fs_metadata(path.clone(), *oid, &meta, 0);
                index.add_entry(entry);
            } else {
                // Removed in target
                if full_path.exists() {
                    fs::remove_file(&full_path)?;
                }
                index.remove_entry(path);
                let mut parent = full_path.parent();
                while let Some(p) = parent {
                    if p == repo_root || !p.starts_with(repo_root) {
                        break;
                    }
                    if fs::remove_dir(p).is_err() {
                        break;
                    }
                    parent = p.parent();
                }
            }
        }
    } else {
        // Force checkout
        let mut paths_to_remove = BTreeSet::new();
        for entry in &index.entries {
            if !target_map.contains_key(&entry.path) {
                paths_to_remove.insert(entry.path.clone());
            }
        }
        for path in head_map.keys() {
            if !target_map.contains_key(path) {
                paths_to_remove.insert(path.clone());
            }
        }
        for path in paths_to_remove {
            let full_path = match oxidize_core::safe_join(repo_root, &path) {
                Ok(p) => p,
                Err(e) => return Err(TuiError::Terminal(e.to_string())),
            };
            if full_path.exists() {
                fs::remove_file(&full_path)?;
            }
            let mut parent = full_path.parent();
            while let Some(p) = parent {
                if p == repo_root || !p.starts_with(repo_root) {
                    break;
                }
                if fs::remove_dir(p).is_err() {
                    break;
                }
                parent = p.parent();
            }
        }

        index.entries.clear();
        for (path, (_mode, oid)) in target_map {
            let full_path = match oxidize_core::safe_join(repo_root, &path) {
                Ok(p) => p,
                Err(e) => return Err(TuiError::Terminal(e.to_string())),
            };
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent)?;
            }

            let blob = match store
                .read_object(&oid)
                .map_err(|e| TuiError::Terminal(e.to_string()))?
            {
                Object::Blob(blob) => blob,
                _ => {
                    return Err(TuiError::Terminal(format!(
                        "checkout expected blob for '{}'",
                        path
                    )))
                }
            };
            fs::write(&full_path, &blob.data)?;
            let meta = fs::metadata(&full_path)?;
            let entry = IndexEntry::from_fs_metadata(path, oid, &meta, 0);
            index.add_entry(entry);
        }
    }

    index
        .write_to(&index_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    Ok(())
}

/// Switches HEAD to an existing local branch, updating the working directory and index safely.
pub fn checkout_branch(
    repo_root: &Path,
    git_dir: &Path,
    branch_name: &str,
) -> Result<(), TuiError> {
    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let ref_store = RefStore::new(git_dir);
    let branch_ref = format!("refs/heads/{}", branch_name);
    let commit_oid = ref_store
        .read_ref(&branch_ref)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let commit = match store.read_object(&commit_oid) {
        Ok(Object::Commit(c)) => c,
        _ => {
            return Err(TuiError::Terminal(
                "branch does not point to a commit".to_string(),
            ))
        }
    };

    checkout_tree_and_update_index(repo_root, git_dir, &commit.tree, false)?;
    ref_store
        .set_head_symbolic(branch_name)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    Ok(())
}

/// Creates a new branch pointing to HEAD and immediately checks it out.
pub fn create_and_checkout_branch(
    _repo_root: &Path,
    git_dir: &Path,
    new_branch: &str,
) -> Result<(), TuiError> {
    let ref_store = RefStore::new(git_dir);
    let (_, head_oid_opt) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let head_oid = head_oid_opt.ok_or_else(|| {
        TuiError::Terminal("cannot create branch: HEAD has no commits".to_string())
    })?;

    ref_store
        .create_branch(new_branch, &head_oid)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    ref_store
        .set_head_symbolic(new_branch)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    Ok(())
}

/// Deletes an existing local branch.
/// If `force` is false, verifies that the branch is merged into HEAD before deleting.
pub fn delete_branch(git_dir: &Path, branch_name: &str, force: bool) -> Result<(), TuiError> {
    oxidize_core::validate_branch_name(branch_name)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let ref_store = RefStore::new(git_dir);
    let (active_branch, head_oid_opt) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    if active_branch == branch_name {
        return Err(TuiError::Terminal(format!(
            "cannot delete checked-out branch '{}'",
            branch_name
        )));
    }

    let branch_ref = format!("refs/heads/{}", branch_name);
    let branch_oid = ref_store
        .read_ref(&branch_ref)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    if !force {
        let store =
            RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
        let head_oid = head_oid_opt.ok_or_else(|| {
            TuiError::Terminal("cannot verify branch merge status: HEAD has no commits".to_string())
        })?;

        let merged = is_ancestor(&store, &branch_oid, &head_oid)?;
        if !merged {
            return Err(TuiError::Terminal(format!(
                "The branch '{}' is not fully merged.\nIf you are sure you want to delete it, run with force.",
                branch_name
            )));
        }
    }

    ref_store
        .delete_branch(branch_name)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    Ok(())
}

/// Drops a stash entry by index from the stash stack.
pub fn drop_stash(git_dir: &Path, index: usize) -> Result<ObjectId, TuiError> {
    let ref_store = RefStore::new(git_dir);
    ref_store
        .stash_drop(index)
        .map_err(|e| TuiError::Terminal(e.to_string()))
}

/// Pops the selected stash into the working tree and drops it from the stash stack on success.
/// If conflicts occur, the stash is preserved on the stack and Ok(false) is returned.
pub fn pop_stash(
    repo_root: &Path,
    git_dir: &Path,
    index: usize,
    stash_oid: &ObjectId,
) -> Result<bool, TuiError> {
    let clean = apply_stash(repo_root, git_dir, stash_oid)?;
    if clean {
        drop_stash(git_dir, index)?;
    }
    Ok(clean)
}

/// Reads configured remote repositories from `.git/config`.
pub fn read_remotes(git_dir: &Path) -> Vec<RemoteItem> {
    let mut remotes = Vec::new();
    let config_path = git_dir.join("config");
    if let Ok(content) = fs::read_to_string(&config_path) {
        let mut current_remote: Option<String> = None;
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("[remote \"") && line.ends_with("\"]") {
                let name = &line[9..line.len() - 2];
                current_remote = Some(name.to_string());
            } else if line.starts_with('[') {
                current_remote = None;
            } else if let Some(ref name) = current_remote {
                if let Some(eq) = line.find('=') {
                    let key = line[..eq].trim();
                    let val = line[eq + 1..].trim();
                    if key.eq_ignore_ascii_case("url") {
                        remotes.push(RemoteItem {
                            name: name.clone(),
                            url: val.to_string(),
                        });
                        current_remote = None;
                    }
                }
            }
        }
    }
    remotes
}

/// Reads local tags from `.git/refs/tags/` and `.git/packed-refs`.
pub fn read_tags(git_dir: &Path) -> Vec<TagItem> {
    let mut tags = Vec::new();
    let tags_dir = git_dir.join("refs").join("tags");
    if tags_dir.exists() && tags_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(tags_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if let Ok(content) = fs::read_to_string(&path) {
                        let trimmed = content.trim();
                        if let Ok(oid) = trimmed.parse::<ObjectId>() {
                            let short_oid = if trimmed.len() >= 7 {
                                trimmed[..7].to_string()
                            } else {
                                trimmed.to_string()
                            };
                            tags.push(TagItem {
                                name,
                                oid,
                                short_oid,
                                message: None,
                            });
                        }
                    }
                }
            }
        }
    }

    // Also check packed-refs
    let packed_path = git_dir.join("packed-refs");
    if packed_path.exists() {
        if let Ok(content) = fs::read_to_string(packed_path) {
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('#') || line.starts_with('^') || line.is_empty() {
                    continue;
                }
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 && parts[1].starts_with("refs/tags/") {
                    let tag_name = parts[1].trim_start_matches("refs/tags/").to_string();
                    if !tags.iter().any(|t| t.name == tag_name) {
                        if let Ok(oid) = parts[0].parse::<ObjectId>() {
                            let short_oid = parts[0][..7.min(parts[0].len())].to_string();
                            tags.push(TagItem {
                                name: tag_name,
                                oid,
                                short_oid,
                                message: None,
                            });
                        }
                    }
                }
            }
        }
    }

    tags.sort_by(|a, b| a.name.cmp(&b.name));
    tags
}

/// Reads reflog history from `.git/logs/HEAD`.
pub fn read_reflog(git_dir: &Path) -> Vec<ReflogItem> {
    let mut entries = Vec::new();
    let reflog_path = git_dir.join("logs").join("HEAD");
    if let Ok(content) = fs::read_to_string(reflog_path) {
        let lines: Vec<&str> = content.lines().collect();
        for (idx, line) in lines.iter().rev().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(tab_idx) = line.find('\t') {
                let meta_part = &line[..tab_idx];
                let msg_part = &line[tab_idx + 1..];
                let meta_tokens: Vec<&str> = meta_part.split_whitespace().collect();
                if meta_tokens.len() >= 2 {
                    let new_sha = meta_tokens[1];
                    if let Ok(oid) = new_sha.parse::<ObjectId>() {
                        let short_oid = if new_sha.len() >= 7 {
                            new_sha[..7].to_string()
                        } else {
                            new_sha.to_string()
                        };

                        let (action, msg) = if let Some(colon) = msg_part.find(':') {
                            (
                                msg_part[..colon].trim().to_string(),
                                msg_part[colon + 1..].trim().to_string(),
                            )
                        } else {
                            ("action".to_string(), msg_part.trim().to_string())
                        };

                        entries.push(ReflogItem {
                            index: idx,
                            selector: format!("HEAD@{{{}}}", idx),
                            oid,
                            short_oid,
                            action,
                            message: msg,
                        });
                    }
                }
            }
        }
    }
    entries
}

/// Amends current HEAD commit with new message and currently staged index.
pub fn amend_commit(
    _repo_root: &Path,
    git_dir: &Path,
    message: &str,
) -> Result<ObjectId, TuiError> {
    let common_dir = resolve_common_dir(git_dir);
    let store = LooseObjectStore::new(common_dir.join("objects"));
    let repo_store =
        RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let ref_store = RefStore::new(git_dir);
    let index_path = git_dir.join("index");
    let index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;

    let (branch_name, head_commit_oid) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let head_oid = head_commit_oid
        .ok_or_else(|| TuiError::Terminal("cannot amend: repository has no commits".to_string()))?;

    let head_commit = match repo_store.read_object(&head_oid) {
        Ok(Object::Commit(c)) => c,
        _ => return Err(TuiError::Terminal("HEAD is not a commit".to_string())),
    };

    let tree_oid = write_tree(&index, &store).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let sig = get_signature(git_dir);

    let commit = Commit {
        tree: tree_oid,
        parents: head_commit.parents,
        author: head_commit.author,
        committer: sig,
        gpg_sig: None,
        message: format!("{}\n", message.trim()),
    };

    let commit_oid = store
        .write_object(&Object::Commit(commit))
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let first_line = message.lines().next().unwrap_or("").trim();
    let ref_msg = format!("commit (amend): {}", first_line);
    ref_store
        .update_ref(&branch_name, &commit_oid, Some(&head_oid), &ref_msg)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    Ok(commit_oid)
}

/// Applies a stash commit without dropping it from the stash stack.
/// Performs a 3-way merge between base commit, current worktree/index, and stash tree.
/// Returns Ok(true) if cleanly applied, Ok(false) if merge conflicts occurred.
pub fn apply_stash(
    repo_root: &Path,
    git_dir: &Path,
    stash_oid: &ObjectId,
) -> Result<bool, TuiError> {
    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let loose_store = LooseObjectStore::new(git_dir.join("objects"));
    let stash_commit = match store.read_object(stash_oid) {
        Ok(Object::Commit(c)) => c,
        _ => return Err(TuiError::Terminal("stash is not a commit".to_string())),
    };

    let base_oid = stash_commit
        .parents
        .first()
        .ok_or_else(|| TuiError::Terminal("stash commit has no parents".to_string()))?;
    let base_commit = match store.read_object(base_oid) {
        Ok(Object::Commit(c)) => c,
        _ => return Err(TuiError::Terminal("stash base is not a commit".to_string())),
    };

    let base_files = flatten_tree(&store, &base_commit.tree, "")
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let stash_files = flatten_tree(&store, &stash_commit.tree, "")
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    // Preflight 3rd parent untracked files: ensure none collide with existing files having different contents
    if stash_commit.parents.len() >= 3 {
        let untracked_oid = &stash_commit.parents[2];
        let u_commit = match store
            .read_object(untracked_oid)
            .map_err(|e| TuiError::Terminal(e.to_string()))?
        {
            Object::Commit(commit) => commit,
            _ => {
                return Err(TuiError::Terminal(
                    "stash untracked parent is not a commit".to_string(),
                ))
            }
        };
        let untracked_files = flatten_tree(&store, &u_commit.tree, "")
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        for (path, (_, blob_oid)) in &untracked_files {
            let full = oxidize_core::safe_join(repo_root, path)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            if full.exists() {
                let existing_bytes = fs::read(&full)?;
                let (_, untracked_bytes) = store
                    .read_raw(blob_oid)
                    .map_err(|e| TuiError::Terminal(e.to_string()))?;
                if existing_bytes != untracked_bytes {
                    return Err(TuiError::Terminal(format!(
                        "Untracked working tree file '{}' would be overwritten by merge",
                        path
                    )));
                }
            }
        }
    }

    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;

    // Collect all paths where base and stash differ
    let mut all_paths = BTreeSet::new();
    for p in base_files.keys() {
        all_paths.insert(p.clone());
    }
    for p in stash_files.keys() {
        all_paths.insert(p.clone());
    }

    let mut has_conflicts = false;

    for path in all_paths {
        let base_entry = base_files.get(&path).copied();
        let stash_entry = stash_files.get(&path).copied();

        if base_entry == stash_entry {
            // Unchanged by stash
            continue;
        }

        let full_path = match oxidize_core::safe_join(repo_root, &path) {
            Ok(p) => p,
            Err(e) => return Err(TuiError::Terminal(e.to_string())),
        };

        match (base_entry, stash_entry) {
            (None, Some((stash_mode, stash_blob_oid))) => {
                // File added by stash
                let stash_bytes = store
                    .read_raw(&stash_blob_oid)
                    .map_err(|e| TuiError::Terminal(e.to_string()))?
                    .1;
                if full_path.exists() {
                    let current_bytes = fs::read(&full_path)?;
                    if current_bytes == stash_bytes {
                        let meta = fs::metadata(&full_path)?;
                        index.add_entry(IndexEntry::from_fs_metadata(
                            path.clone(),
                            stash_blob_oid,
                            &meta,
                            0,
                        ));
                    } else {
                        has_conflicts = true;
                        let our_blob = loose_store
                            .write_object(&Object::Blob(Blob::new(current_bytes.clone())))
                            .map_err(|e| TuiError::Terminal(e.to_string()))?;

                        let is_bin = oxidize_diff::is_binary_content(&current_bytes)
                            || oxidize_diff::is_binary_content(&stash_bytes);
                        let cur_str_res = std::str::from_utf8(&current_bytes);
                        let stash_str_res = std::str::from_utf8(&stash_bytes);

                        if !is_bin {
                            if let (Ok(cur_str), Ok(stash_str)) = (cur_str_res, stash_str_res) {
                                let merged = three_way_merge(
                                    "",
                                    cur_str,
                                    stash_str,
                                    "Updated upstream",
                                    "Stashed changes",
                                );
                                fs::write(&full_path, merged.content.as_bytes())?;
                            }
                        }

                        index.remove_entry(&path);
                        let mut e2 = IndexEntry::new(path.clone(), our_blob, stash_mode.0);
                        e2.stage = 2;
                        let mut e3 = IndexEntry::new(path.clone(), stash_blob_oid, stash_mode.0);
                        e3.stage = 3;
                        index.add_entry(e2);
                        index.add_entry(e3);
                    }
                } else {
                    if let Some(parent) = full_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(&full_path, &stash_bytes)?;
                    let meta = fs::metadata(&full_path)?;
                    index.add_entry(IndexEntry::from_fs_metadata(
                        path.clone(),
                        stash_blob_oid,
                        &meta,
                        0,
                    ));
                }
            }
            (Some((base_mode, base_blob_oid)), None) => {
                // File deleted by stash
                if full_path.exists() {
                    let current_bytes = fs::read(&full_path)?;
                    let base_bytes = store
                        .read_raw(&base_blob_oid)
                        .map_err(|e| TuiError::Terminal(e.to_string()))?
                        .1;
                    if current_bytes == base_bytes
                        || bytes_equal_ignoring_crlf(&current_bytes, &base_bytes)
                    {
                        fs::remove_file(&full_path)?;
                        index.remove_entry(&path);
                    } else {
                        has_conflicts = true;
                        let our_blob = loose_store
                            .write_object(&Object::Blob(Blob::new(current_bytes)))
                            .map_err(|e| TuiError::Terminal(e.to_string()))?;
                        index.remove_entry(&path);
                        let mut e1 = IndexEntry::new(path.clone(), base_blob_oid, base_mode.0);
                        e1.stage = 1;
                        let mut e2 = IndexEntry::new(path.clone(), our_blob, base_mode.0);
                        e2.stage = 2;
                        index.add_entry(e1);
                        index.add_entry(e2);
                    }
                } else {
                    index.remove_entry(&path);
                }
            }
            (Some((base_mode, base_blob_oid)), Some((stash_mode, stash_blob_oid))) => {
                // File modified by stash
                let base_bytes = store
                    .read_raw(&base_blob_oid)
                    .map_err(|e| TuiError::Terminal(e.to_string()))?
                    .1;
                let stash_bytes = store
                    .read_raw(&stash_blob_oid)
                    .map_err(|e| TuiError::Terminal(e.to_string()))?
                    .1;

                if full_path.exists() {
                    let current_bytes = fs::read(&full_path)?;
                    if current_bytes == base_bytes
                        || bytes_equal_ignoring_crlf(&current_bytes, &base_bytes)
                    {
                        fs::write(&full_path, &stash_bytes)?;
                        let meta = fs::metadata(&full_path)?;
                        index.add_entry(IndexEntry::from_fs_metadata(
                            path.clone(),
                            stash_blob_oid,
                            &meta,
                            0,
                        ));
                    } else if current_bytes == stash_bytes
                        || bytes_equal_ignoring_crlf(&current_bytes, &stash_bytes)
                    {
                        let meta = fs::metadata(&full_path)?;
                        index.add_entry(IndexEntry::from_fs_metadata(
                            path.clone(),
                            stash_blob_oid,
                            &meta,
                            0,
                        ));
                    } else {
                        let is_bin = oxidize_diff::is_binary_content(&base_bytes)
                            || oxidize_diff::is_binary_content(&current_bytes)
                            || oxidize_diff::is_binary_content(&stash_bytes);
                        let base_str_res = std::str::from_utf8(&base_bytes);
                        let cur_str_res = std::str::from_utf8(&current_bytes);
                        let stash_str_res = std::str::from_utf8(&stash_bytes);
                        let text_merge_opt = if !is_bin {
                            if let (Ok(base_str), Ok(cur_str), Ok(stash_str)) =
                                (base_str_res, cur_str_res, stash_str_res)
                            {
                                Some(three_way_merge(
                                    base_str,
                                    cur_str,
                                    stash_str,
                                    "Updated upstream",
                                    "Stashed changes",
                                ))
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        if let Some(merged) = text_merge_opt {
                            fs::write(&full_path, merged.content.as_bytes())?;
                            if merged.has_conflicts {
                                has_conflicts = true;
                                let our_blob = loose_store
                                    .write_object(&Object::Blob(Blob::new(current_bytes)))
                                    .map_err(|e| TuiError::Terminal(e.to_string()))?;
                                index.remove_entry(&path);
                                let mut e1 =
                                    IndexEntry::new(path.clone(), base_blob_oid, base_mode.0);
                                e1.stage = 1;
                                let mut e2 = IndexEntry::new(path.clone(), our_blob, base_mode.0);
                                e2.stage = 2;
                                let mut e3 =
                                    IndexEntry::new(path.clone(), stash_blob_oid, stash_mode.0);
                                e3.stage = 3;
                                index.add_entry(e1);
                                index.add_entry(e2);
                                index.add_entry(e3);
                            } else {
                                let merged_blob = loose_store
                                    .write_object(&Object::Blob(Blob::new(
                                        merged.content.into_bytes(),
                                    )))
                                    .map_err(|e| TuiError::Terminal(e.to_string()))?;
                                let meta = fs::metadata(&full_path)?;
                                index.add_entry(IndexEntry::from_fs_metadata(
                                    path.clone(),
                                    merged_blob,
                                    &meta,
                                    0,
                                ));
                            }
                        } else {
                            has_conflicts = true;
                            let our_blob = loose_store
                                .write_object(&Object::Blob(Blob::new(current_bytes)))
                                .map_err(|e| TuiError::Terminal(e.to_string()))?;
                            index.remove_entry(&path);
                            let mut e1 = IndexEntry::new(path.clone(), base_blob_oid, base_mode.0);
                            e1.stage = 1;
                            let mut e2 = IndexEntry::new(path.clone(), our_blob, base_mode.0);
                            e2.stage = 2;
                            let mut e3 =
                                IndexEntry::new(path.clone(), stash_blob_oid, stash_mode.0);
                            e3.stage = 3;
                            index.add_entry(e1);
                            index.add_entry(e2);
                            index.add_entry(e3);
                        }
                    }
                } else {
                    has_conflicts = true;
                    fs::write(&full_path, &stash_bytes)?;
                    index.remove_entry(&path);
                    let mut e1 = IndexEntry::new(path.clone(), base_blob_oid, base_mode.0);
                    e1.stage = 1;
                    let mut e3 = IndexEntry::new(path.clone(), stash_blob_oid, stash_mode.0);
                    e3.stage = 3;
                    index.add_entry(e1);
                    index.add_entry(e3);
                }
            }
            (None, None) => {}
        }
    }

    index
        .write_to(&index_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    // Restore untracked files if present in 3rd parent commit
    if stash_commit.parents.len() >= 3 {
        let untracked_oid = &stash_commit.parents[2];
        let u_commit = match store
            .read_object(untracked_oid)
            .map_err(|e| TuiError::Terminal(e.to_string()))?
        {
            Object::Commit(commit) => commit,
            _ => {
                return Err(TuiError::Terminal(
                    "stash untracked parent is not a commit".to_string(),
                ))
            }
        };
        let untracked_files = flatten_tree(&store, &u_commit.tree, "")
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        for (path, (_, blob_oid)) in untracked_files {
            let full = oxidize_core::safe_join(repo_root, &path)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            let (_, data) = store
                .read_raw(&blob_oid)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&full, &data)?;
        }
    }

    Ok(!has_conflicts)
}

/// Options controlling stash save behavior.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StashSaveOptions {
    /// Custom stash message.
    pub message: String,
    /// Whether to include untracked files (writes 3rd parent commit to stash).
    pub include_untracked: bool,
    /// Whether to stash staged changes only (keeps unstaged changes in worktree).
    pub staged_only: bool,
    /// Whether to keep index changes staged after stashing.
    pub keep_index: bool,
}

/// Creates a new stash commit saving working directory changes and index state.
pub fn stash_save(repo_root: &Path, git_dir: &Path, message: &str) -> Result<ObjectId, TuiError> {
    stash_save_with_options(
        repo_root,
        git_dir,
        &StashSaveOptions {
            message: message.to_string(),
            include_untracked: false,
            staged_only: false,
            keep_index: false,
        },
    )
}

/// Creates a new stash commit according to the provided `StashSaveOptions`.
pub fn stash_save_with_options(
    repo_root: &Path,
    git_dir: &Path,
    opts: &StashSaveOptions,
) -> Result<ObjectId, TuiError> {
    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let loose_store = LooseObjectStore::new(git_dir.join("objects"));
    let ref_store = RefStore::new(git_dir);
    let index_path = git_dir.join("index");
    let index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;

    let (branch_name, head_commit_oid) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let head_oid = head_commit_oid
        .ok_or_else(|| TuiError::Terminal("cannot stash: HEAD has no commits".to_string()))?;

    let head_commit = match store.read_object(&head_oid) {
        Ok(Object::Commit(c)) => c,
        _ => return Err(TuiError::Terminal("HEAD is not a commit".to_string())),
    };

    let status = oxidize_index::compute_status(repo_root, &index, Some(&head_commit.tree), &store)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let gitignore = oxidize_config::GitIgnore::load_from_dir(repo_root)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let mut untracked_files = Vec::new();
    if opts.include_untracked {
        let mut all_files = Vec::new();
        collect_untracked_files(repo_root, repo_root, &gitignore, &mut all_files)?;
        for p in all_files {
            if index.find_entry(&p).is_none() {
                untracked_files.push(p);
            }
        }
        untracked_files.sort();
        untracked_files.dedup();
    }

    if opts.staged_only {
        if status.staged.is_empty() {
            return Err(TuiError::Terminal("No staged changes to save".to_string()));
        }
    } else if opts.include_untracked {
        if status.staged.is_empty() && status.unstaged.is_empty() && untracked_files.is_empty() {
            return Err(TuiError::Terminal("No local changes to save".to_string()));
        }
    } else if status.staged.is_empty() && status.unstaged.is_empty() {
        return Err(TuiError::Terminal("No local changes to save".to_string()));
    }

    let sig = get_signature(git_dir);
    let head_short = &head_oid.to_string()[..7];
    let head_first_line = head_commit.message.lines().next().unwrap_or("").trim();

    // 1. Index commit
    let index_tree_oid =
        write_tree(&index, &loose_store).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let index_commit = Commit {
        tree: index_tree_oid,
        parents: vec![head_oid],
        author: sig.clone(),
        committer: sig.clone(),
        gpg_sig: None,
        message: format!(
            "index on {}: {} {}\n",
            branch_name, head_short, head_first_line
        ),
    };
    let index_commit_oid = loose_store
        .write_object(&Object::Commit(index_commit))
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    // 2. Worktree commit tree
    let work_tree_oid = if opts.staged_only {
        index_tree_oid
    } else {
        let mut work_index = index.clone();
        for change in &status.unstaged {
            match change {
                oxidize_index::UnstagedChange::Modified(p) => {
                    let full = repo_root.join(p);
                    let data = fs::read(&full)?;
                    let blob = Object::Blob(Blob::new(data));
                    let oid = loose_store
                        .write_object(&blob)
                        .map_err(|e| TuiError::Terminal(e.to_string()))?;
                    let meta = fs::metadata(&full)?;
                    work_index.add_entry(IndexEntry::from_fs_metadata(p.clone(), oid, &meta, 0));
                }
                oxidize_index::UnstagedChange::Deleted(p) => {
                    work_index.remove_entry(p);
                }
            }
        }
        write_tree(&work_index, &loose_store).map_err(|e| TuiError::Terminal(e.to_string()))?
    };

    // 3. Untracked commit (optional 3rd parent)
    let mut untracked_commit_oid_opt = None;
    if opts.include_untracked && !untracked_files.is_empty() {
        let mut untracked_index = Index::new();
        for p in &untracked_files {
            let full = repo_root.join(p);
            let data = fs::read(&full)?;
            let meta = fs::metadata(&full)?;
            let blob = Object::Blob(Blob::new(data));
            let oid = loose_store
                .write_object(&blob)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            untracked_index.add_entry(IndexEntry::from_fs_metadata(p.clone(), oid, &meta, 0));
        }
        let untracked_tree_oid = write_tree(&untracked_index, &loose_store)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        let untracked_commit = Commit {
            tree: untracked_tree_oid,
            parents: vec![],
            author: sig.clone(),
            committer: sig.clone(),
            gpg_sig: None,
            message: format!(
                "untracked files on {}: {} {}\n",
                branch_name, head_short, head_first_line
            ),
        };
        untracked_commit_oid_opt = Some(
            loose_store
                .write_object(&Object::Commit(untracked_commit))
                .map_err(|e| TuiError::Terminal(e.to_string()))?,
        );
    }

    let stash_msg = if opts.message.trim().is_empty() {
        format!("WIP on {}: {} {}", branch_name, head_short, head_first_line)
    } else {
        opts.message.trim().to_string()
    };

    let mut parents = vec![head_oid, index_commit_oid];
    if let Some(u_oid) = untracked_commit_oid_opt {
        parents.push(u_oid);
    }

    let stash_commit = Commit {
        tree: work_tree_oid,
        parents,
        author: sig.clone(),
        committer: sig,
        gpg_sig: None,
        message: format!("{}\n", stash_msg),
    };
    let stash_oid = loose_store
        .write_object(&Object::Commit(stash_commit))
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    ref_store
        .update_ref(
            "refs/stash",
            &stash_oid,
            None,
            &format!("WIP on {}: {}", branch_name, stash_msg),
        )
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    // Reset WT and index according to options
    if opts.staged_only {
        // Reset index to HEAD commit tree
        let head_files = flatten_tree(&store, &head_commit.tree, "")
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        let mut new_index = Index::new();
        for (p, (mode, oid)) in &head_files {
            let full = repo_root.join(p);
            if let Ok(meta) = fs::metadata(&full) {
                new_index.add_entry(IndexEntry::from_fs_metadata(p.clone(), *oid, &meta, 0));
            } else {
                new_index.add_entry(IndexEntry {
                    ctime_sec: 0,
                    ctime_nsec: 0,
                    mtime_sec: 0,
                    mtime_nsec: 0,
                    dev: 0,
                    ino: 0,
                    mode: mode.0,
                    uid: 0,
                    gid: 0,
                    file_size: 0,
                    oid: *oid,
                    stage: 0,
                    assume_valid: false,
                    path: p.clone(),
                });
            }
        }
        new_index
            .write_to(&index_path)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;

        // For files that were staged and NOT in unstaged changes: revert WT to HEAD
        let unstaged_paths: HashSet<String> = status
            .unstaged
            .iter()
            .map(|c| match c {
                oxidize_index::UnstagedChange::Modified(p) => p.clone(),
                oxidize_index::UnstagedChange::Deleted(p) => p.clone(),
            })
            .collect();
        for staged_change in &status.staged {
            let p = match staged_change {
                oxidize_index::StagedChange::New(p) => p,
                oxidize_index::StagedChange::Modified(p) => p,
                oxidize_index::StagedChange::Deleted(p) => p,
                oxidize_index::StagedChange::Renamed { to, .. } => to,
            };
            if !unstaged_paths.contains(p) {
                let full = repo_root.join(p);
                if let Some((_, head_blob_oid)) = head_files.get(p) {
                    let blob = match store
                        .read_object(head_blob_oid)
                        .map_err(|e| TuiError::Terminal(e.to_string()))?
                    {
                        Object::Blob(blob) => blob,
                        _ => {
                            return Err(TuiError::Terminal(format!(
                                "stash restore expected blob for '{}'",
                                p
                            )))
                        }
                    };
                    if let Some(parent) = full.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(&full, &blob.data)?;
                } else {
                    if full.exists() {
                        fs::remove_file(&full)?;
                    }
                }
            }
        }
    } else if opts.keep_index {
        // Reset WT to HEAD
        checkout_tree_and_update_index(repo_root, git_dir, &head_commit.tree, true)?;
        // Write original staged index back
        index
            .write_to(&index_path)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        // Checkout staged files to WT
        for entry in index.entries() {
            let full = repo_root.join(&entry.path);
            let (_, data) = store
                .read_raw(&entry.oid)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&full, &data)?;
        }
    } else {
        // Standard stash: reset WT and index to HEAD
        checkout_tree_and_update_index(repo_root, git_dir, &head_commit.tree, true)?;
        if opts.include_untracked {
            for p in &untracked_files {
                let full = repo_root.join(p);
                if full.exists() {
                    fs::remove_file(&full)?;
                }
            }
        }
    }

    Ok(stash_oid)
}

/// Pushes local branch commits to the specified remote repository using native Rust transport.
pub fn push_to_remote(
    repo_root: &Path,
    git_dir: &Path,
    remote_opt: Option<&str>,
    branch_opt: Option<&str>,
    force: bool,
) -> Result<String, TuiError> {
    push_to_remote_ext(repo_root, git_dir, remote_opt, branch_opt, force, None)
}

/// Pushes branch to remote repository with optional force and force-with-lease check.
pub fn push_to_remote_ext(
    _repo_root: &Path,
    git_dir: &Path,
    remote_opt: Option<&str>,
    branch_opt: Option<&str>,
    force: bool,
    force_with_lease: Option<ObjectId>,
) -> Result<String, TuiError> {
    let config = load_repo_config(git_dir)?;

    let remote_name = remote_opt.unwrap_or("origin");
    let url = config.get_remote_url(remote_name).ok_or_else(|| {
        TuiError::Terminal(format!(
            "fatal: No configured push destination for remote '{}'",
            remote_name
        ))
    })?;

    let ref_store = RefStore::new(git_dir);
    let branch = if let Some(b) = branch_opt {
        b.to_string()
    } else {
        let (curr, _) = ref_store
            .resolve_head()
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        curr
    };

    let local_ref_name = format!("refs/heads/{}", branch);
    let local_oid = ref_store
        .read_ref(&local_ref_name)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    // Discover remote refs and capabilities
    let (remote_refs, server_caps, is_local, is_ssh) =
        if let Some(local_path) = resolve_local_path(url) {
            let (refs, _) =
                discover_local_refs(&local_path).map_err(|e| TuiError::Terminal(e.to_string()))?;
            (refs, Vec::new(), Some(local_path), false)
        } else if is_ssh_url(url) {
            let client = SshClient::new();
            let (refs, caps) = client
                .discover_receive_pack(url)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            (refs, caps, None, true)
        } else {
            let client = SmartHttpClient::new();
            let (refs, caps) = client
                .discover_receive_pack(url)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            (refs, caps, None, false)
        };

    let remote_target_name = format!("refs/heads/{}", branch);
    let remote_old_oid = remote_refs
        .iter()
        .find(|r| r.name == remote_target_name)
        .map(|r| r.oid)
        .unwrap_or(ObjectId::ZERO);

    // Force-with-lease validation: remote ref must match expected OID
    if let Some(expected) = force_with_lease {
        if remote_old_oid != expected {
            return Err(TuiError::Terminal(
                "Updates were rejected because the remote reference has changed since you last checked."
                    .to_string(),
            ));
        }
    }

    // 1. Up-to-date check
    if !remote_old_oid.is_zero() && remote_old_oid == local_oid {
        return Ok("Everything up-to-date".to_string());
    }

    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;

    // 2. Fast-forward ancestry check
    let is_ff = if remote_old_oid.is_zero() {
        true
    } else {
        is_ancestor(&store, &remote_old_oid, &local_oid)?
    };

    if !is_ff && !force && force_with_lease.is_none() {
        return Err(TuiError::Terminal(
            "Updates were rejected because the remote contains work that you do not have locally.\nUse force to overwrite."
                .to_string(),
        ));
    }

    // 3. Checked-out branch protection on local non-bare destination
    if let Some(ref dest_path) = is_local {
        if dest_path.join(".git").is_dir() {
            let dest_git_dir = dest_path.join(".git");
            let dest_ref_store = RefStore::new(&dest_git_dir);
            if let Ok((dest_head, _)) = dest_ref_store.resolve_head() {
                let is_checked_out = dest_head == remote_target_name
                    || format!("refs/heads/{}", dest_head) == remote_target_name;
                if is_checked_out {
                    return Err(TuiError::Terminal(format!(
                        "fatal: refusing to update checked out branch: {}\nBy default, updating the current branch in a non-bare repository is denied.",
                        remote_target_name
                    )));
                }
            }
        }
    }

    // 4. Pack only reachable objects required from local_oid, excluding haves
    let haves = if remote_old_oid.is_zero() {
        Vec::new()
    } else {
        vec![remote_old_oid]
    };
    let objects = store
        .collect_reachable_objects(&[local_oid], &haves)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let (pack_bytes, _, _) =
        oxidize_pack::write_pack(&objects, true).map_err(|e| TuiError::Terminal(e.to_string()))?;

    if let Some(dest_path) = is_local {
        let dest_git_dir = if dest_path.join(".git").is_dir() {
            dest_path.join(".git")
        } else {
            dest_path.clone()
        };

        if !pack_bytes.is_empty() {
            let (indexed_objs, pack_checksum) = oxidize_pack::index_packfile(&pack_bytes)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            let pack_dir = dest_git_dir.join("objects").join("pack");
            fs::create_dir_all(&pack_dir)?;
            let pack_file = pack_dir.join(format!("pack-{}.pack", pack_checksum));
            let idx_file = pack_dir.join(format!("pack-{}.idx", pack_checksum));
            fs::write(&pack_file, &pack_bytes)?;
            PackIndex::write_to(indexed_objs, &pack_checksum, &idx_file)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
        }

        let dest_ref_store = RefStore::new(&dest_git_dir);
        let expected_old = if force || remote_old_oid.is_zero() || force_with_lease.is_some() {
            None
        } else {
            Some(&remote_old_oid)
        };
        dest_ref_store
            .update_ref(
                &remote_target_name,
                &local_oid,
                expected_old,
                "push: from local",
            )
            .map_err(|e| TuiError::Terminal(e.to_string()))?;

        // Update local tracking branch
        let tracking_ref = format!("refs/remotes/{}/{}", remote_name, branch);
        ref_store
            .update_ref(&tracking_ref, &local_oid, None, "update tracking ref")
            .map_err(|e| {
                TuiError::Terminal(format!(
                    "remote push succeeded, but local tracking ref update failed: {}",
                    e
                ))
            })?;

        Ok(format!("Pushed {} to {}", branch, remote_name))
    } else {
        // HTTP or SSH remote
        let updates = [(&remote_old_oid, &local_oid, remote_target_name.as_str())];
        let report = if is_ssh {
            let client = SshClient::new();
            client
                .push_pack_with_caps(url, &updates, &pack_bytes, &server_caps)
                .map_err(|e| TuiError::Terminal(e.to_string()))?
        } else {
            let client = SmartHttpClient::new();
            client
                .push_pack_with_caps(url, &updates, &pack_bytes, &server_caps)
                .map_err(|e| TuiError::Terminal(e.to_string()))?
        };

        if !report.is_success() {
            return Err(TuiError::Terminal(format!(
                "Push rejected: {}",
                report.display_summary()
            )));
        }

        let tracking_ref = format!("refs/remotes/{}/{}", remote_name, branch);
        ref_store
            .update_ref(&tracking_ref, &local_oid, None, "update tracking ref")
            .map_err(|e| {
                TuiError::Terminal(format!(
                    "remote push succeeded, but local tracking ref update failed: {}",
                    e
                ))
            })?;

        Ok(format!("Pushed {} to {}", branch, remote_name))
    }
}

/// Pushes a tag to the specified remote.
pub fn push_tag_to_remote(
    git_dir: &Path,
    remote_opt: Option<&str>,
    tag_name: &str,
) -> Result<String, TuiError> {
    let config = load_repo_config(git_dir)?;
    let remote_name = remote_opt.unwrap_or("origin");
    let url = config
        .get_remote_url(remote_name)
        .ok_or_else(|| TuiError::Terminal(format!("No configured remote '{}'", remote_name)))?;

    let ref_store = RefStore::new(git_dir);
    let tag_ref_name = format!("refs/tags/{}", tag_name);
    let tag_oid = ref_store
        .read_ref(&tag_ref_name)
        .map_err(|e| TuiError::Terminal(format!("Tag '{}' not found: {}", tag_name, e)))?;

    let (remote_refs, server_caps, is_local, is_ssh) =
        if let Some(local_path) = resolve_local_path(url) {
            let (refs, _) =
                discover_local_refs(&local_path).map_err(|e| TuiError::Terminal(e.to_string()))?;
            (refs, Vec::new(), Some(local_path), false)
        } else if is_ssh_url(url) {
            let client = SshClient::new();
            let (refs, caps) = client
                .discover_receive_pack(url)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            (refs, caps, None, true)
        } else {
            let client = SmartHttpClient::new();
            let (refs, caps) = client
                .discover_receive_pack(url)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            (refs, caps, None, false)
        };

    let remote_old_oid = remote_refs
        .iter()
        .find(|r| r.name == tag_ref_name)
        .map(|r| r.oid)
        .unwrap_or(ObjectId::ZERO);

    if remote_old_oid == tag_oid {
        return Ok(format!(
            "Tag '{}' is already up to date on {}",
            tag_name, remote_name
        ));
    }

    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let haves = if remote_old_oid.is_zero() {
        Vec::new()
    } else {
        vec![remote_old_oid]
    };
    let objects = store
        .collect_reachable_objects(&[tag_oid], &haves)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let (pack_bytes, _, _) =
        oxidize_pack::write_pack(&objects, true).map_err(|e| TuiError::Terminal(e.to_string()))?;

    if let Some(dest_path) = is_local {
        let dest_git_dir = if dest_path.join(".git").is_dir() {
            dest_path.join(".git")
        } else {
            dest_path.clone()
        };
        if !pack_bytes.is_empty() {
            let (indexed_objs, pack_checksum) = oxidize_pack::index_packfile(&pack_bytes)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            let pack_dir = dest_git_dir.join("objects").join("pack");
            fs::create_dir_all(&pack_dir)?;
            let pack_file = pack_dir.join(format!("pack-{}.pack", pack_checksum));
            let idx_file = pack_dir.join(format!("pack-{}.idx", pack_checksum));
            fs::write(&pack_file, &pack_bytes)?;
            PackIndex::write_to(indexed_objs, &pack_checksum, &idx_file)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
        }
        let dest_ref_store = RefStore::new(&dest_git_dir);
        dest_ref_store
            .update_ref(&tag_ref_name, &tag_oid, None, "push: tag")
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
    } else {
        let updates = [(&remote_old_oid, &tag_oid, tag_ref_name.as_str())];
        let report = if is_ssh {
            let client = SshClient::new();
            client
                .push_pack_with_caps(url, &updates, &pack_bytes, &server_caps)
                .map_err(|e| TuiError::Terminal(e.to_string()))?
        } else {
            let client = SmartHttpClient::new();
            client
                .push_pack_with_caps(url, &updates, &pack_bytes, &server_caps)
                .map_err(|e| TuiError::Terminal(e.to_string()))?
        };
        if !report.is_success() {
            return Err(TuiError::Terminal(format!(
                "Tag push rejected: {}",
                report.display_summary()
            )));
        }
    }

    Ok(format!("Pushed tag '{}' to {}", tag_name, remote_name))
}

/// Deletes a branch on the remote repository.
pub fn delete_remote_branch(
    git_dir: &Path,
    remote_opt: Option<&str>,
    branch_name: &str,
) -> Result<String, TuiError> {
    let config = load_repo_config(git_dir)?;
    let remote_name = remote_opt.unwrap_or("origin");
    let url = config
        .get_remote_url(remote_name)
        .ok_or_else(|| TuiError::Terminal(format!("No configured remote '{}'", remote_name)))?;

    let ref_store = RefStore::new(git_dir);
    let target_ref_name = format!("refs/heads/{}", branch_name);

    let (remote_refs, server_caps, is_local, is_ssh) =
        if let Some(local_path) = resolve_local_path(url) {
            let (refs, _) =
                discover_local_refs(&local_path).map_err(|e| TuiError::Terminal(e.to_string()))?;
            (refs, Vec::new(), Some(local_path), false)
        } else if is_ssh_url(url) {
            let client = SshClient::new();
            let (refs, caps) = client
                .discover_receive_pack(url)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            (refs, caps, None, true)
        } else {
            let client = SmartHttpClient::new();
            let (refs, caps) = client
                .discover_receive_pack(url)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            (refs, caps, None, false)
        };

    let remote_old_oid = remote_refs
        .iter()
        .find(|r| r.name == target_ref_name)
        .map(|r| r.oid)
        .ok_or_else(|| {
            TuiError::Terminal(format!(
                "remote branch '{}' not found on {}",
                branch_name, remote_name
            ))
        })?;

    if let Some(dest_path) = is_local {
        let dest_git_dir = if dest_path.join(".git").is_dir() {
            dest_path.join(".git")
        } else {
            dest_path.clone()
        };
        let dest_ref_store = RefStore::new(&dest_git_dir);
        dest_ref_store.delete_branch(branch_name).map_err(|e| {
            TuiError::Terminal(format!(
                "failed to delete branch '{}' on local remote '{}': {}",
                branch_name, remote_name, e
            ))
        })?;
    } else {
        let updates = [(&remote_old_oid, &ObjectId::ZERO, target_ref_name.as_str())];
        let report = if is_ssh {
            let client = SshClient::new();
            client
                .push_pack_with_caps(url, &updates, &[], &server_caps)
                .map_err(|e| TuiError::Terminal(e.to_string()))?
        } else {
            let client = SmartHttpClient::new();
            client
                .push_pack_with_caps(url, &updates, &[], &server_caps)
                .map_err(|e| TuiError::Terminal(e.to_string()))?
        };
        if !report.is_success() {
            return Err(TuiError::Terminal(format!(
                "Remote branch deletion rejected: {}",
                report.display_summary()
            )));
        }
    }

    // Clean up local tracking branch
    let tracking_ref = format!("refs/remotes/{}/{}", remote_name, branch_name);
    match ref_store.delete_ref(&tracking_ref) {
        Ok(()) | Err(RefError::NotFound(_)) => {}
        Err(e) => {
            return Err(TuiError::Terminal(format!(
                "remote branch deletion succeeded, but local tracking ref '{}' could not be removed: {}",
                tracking_ref, e
            )))
        }
    }

    Ok(format!(
        "Deleted remote branch '{}/{}'",
        remote_name, branch_name
    ))
}

/// Fetches from all configured remotes with optional prune of stale tracking branches.
pub fn fetch_all_remotes(git_dir: &Path, prune: bool) -> Result<String, TuiError> {
    let config = load_repo_config(git_dir)?;
    let remotes = config.list_remotes();
    if remotes.is_empty() {
        return Ok("No remotes configured".to_string());
    }

    let mut messages = Vec::new();
    let ref_store = RefStore::new(git_dir);

    for (name, _) in &remotes {
        let msg = fetch_from_remote(git_dir, Some(name))?;
        messages.push(format!("{}: {}", name, msg));

        if prune {
            if let Some(url) = config.get_remote_url(name) {
                let remote_refs_res = if let Some(local_path) = resolve_local_path(url) {
                    discover_local_refs(&local_path).map(|(r, _)| r)
                } else if is_ssh_url(url) {
                    SshClient::new()
                        .discover_upload_pack(url)
                        .map(|(r, _, _)| r)
                } else {
                    SmartHttpClient::new()
                        .discover_upload_pack(url)
                        .map(|(r, _, _)| r)
                };

                let remote_refs = remote_refs_res.map_err(|e| {
                    TuiError::Terminal(format!(
                        "fetch from '{}' succeeded, but prune discovery failed: {}",
                        name, e
                    ))
                })?;
                let remote_branches: HashSet<String> = remote_refs
                    .iter()
                    .filter_map(|r| r.name.strip_prefix("refs/heads/").map(|s| s.to_string()))
                    .collect();

                let prefix = format!("{}/", name);
                let tracking_refs = ref_store.list_remotes().map_err(|e| {
                    TuiError::Terminal(format!(
                        "fetch from '{}' succeeded, but local tracking refs could not be listed for pruning: {}",
                        name, e
                    ))
                })?;
                for (tracking_branch, _oid) in tracking_refs {
                    if let Some(remote_branch) = tracking_branch.strip_prefix(&prefix) {
                        if !remote_branches.contains(remote_branch) {
                            let full_ref = format!("refs/remotes/{}", tracking_branch);
                            ref_store.delete_ref(&full_ref).map_err(|e| {
                                TuiError::Terminal(format!(
                                    "fetch from '{}' succeeded, but stale tracking ref '{}' could not be pruned: {}",
                                    name, full_ref, e
                                ))
                            })?;
                            messages.push(format!("Pruned tracking branch {}", tracking_branch));
                        }
                    }
                }
            }
        }
    }

    Ok(messages.join("\n"))
}

/// Adds a remote to git configuration.
pub fn add_remote(git_dir: &Path, name: &str, url: &str) -> Result<(), TuiError> {
    let config_path = git_dir.join("config");
    let mut config = load_repo_config(git_dir)?;
    if config.get_remote_url(name).is_some() {
        return Err(TuiError::Terminal(format!(
            "Remote '{}' already exists",
            name
        )));
    }
    config.add_remote(name, url);
    config
        .save_to_file(&config_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))
}

/// Removes a remote from git configuration and cleans up its tracking branches.
pub fn remove_remote(git_dir: &Path, name: &str) -> Result<(), TuiError> {
    let config_path = git_dir.join("config");
    let mut config = load_repo_config(git_dir)?;
    let original = fs::read(&config_path)?;
    if !config.remove_remote(name) {
        return Err(TuiError::Terminal(format!("Remote '{}' not found", name)));
    }
    config
        .save_to_file(&config_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let ref_store = RefStore::new(git_dir);
    if let Err(error) = ref_store.remove_remote_refs(name) {
        return Err(rollback_config(
            &config_path,
            &original,
            format!(
                "failed to remove tracking refs for remote '{}': {}",
                name, error
            ),
        ));
    }
    Ok(())
}

/// Renames a remote in git configuration.
pub fn rename_remote(git_dir: &Path, old_name: &str, new_name: &str) -> Result<(), TuiError> {
    let config_path = git_dir.join("config");
    let mut config = load_repo_config(git_dir)?;
    let original = fs::read(&config_path)?;
    if config.get_remote_url(new_name).is_some() {
        return Err(TuiError::Terminal(format!(
            "Remote '{}' already exists",
            new_name
        )));
    }
    if !config
        .rename_subsection("remote", old_name, new_name)
        .map_err(|e| TuiError::Terminal(e.to_string()))?
    {
        return Err(TuiError::Terminal(format!(
            "Remote '{}' not found",
            old_name
        )));
    }
    config.replace_in_values(
        "remote",
        Some(new_name),
        "fetch",
        &format!("refs/remotes/{}/", old_name),
        &format!("refs/remotes/{}/", new_name),
    );
    for branch in config.subsections("branch") {
        if config.get("branch", Some(&branch), "remote") == Some(old_name) {
            config.set("branch", Some(&branch), "remote", new_name);
        }
    }
    config
        .save_to_file(&config_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let ref_store = RefStore::new(git_dir);
    if let Err(error) = ref_store.rename_remote_refs(old_name, new_name) {
        return Err(rollback_config(
            &config_path,
            &original,
            format!(
                "failed to rename tracking refs from '{}' to '{}': {}",
                old_name, new_name, error
            ),
        ));
    }
    Ok(())
}

/// Provider URLs for viewing repository, commits, branches, and PRs on web hosts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderUrls {
    pub repo_url: Option<String>,
    pub commit_url: Option<String>,
    pub branch_url: Option<String>,
    pub pr_url: Option<String>,
}

/// Parses remote URL and constructs web provider links for commit, branch, and pull request.
pub fn get_provider_urls(
    git_dir: &Path,
    commit_oid: Option<ObjectId>,
    branch_opt: Option<&str>,
) -> ProviderUrls {
    let config_path = git_dir.join("config");
    let config = GitConfig::load_from_file(&config_path).unwrap_or_default();
    let remote_url_str = config
        .get_remote_url("origin")
        .map(|s| s.to_string())
        .or_else(|| config.list_remotes().first().map(|(_, u)| u.clone()));

    let raw_url = match remote_url_str.as_deref() {
        Some(u) => u.trim(),
        None => return ProviderUrls::default(),
    };

    let (host, path) = if let Some(stripped) = raw_url.strip_prefix("git@") {
        if let Some((h, p)) = stripped.split_once(':') {
            (h, p)
        } else {
            return ProviderUrls::default();
        }
    } else if let Some(stripped) = raw_url.strip_prefix("ssh://git@") {
        if let Some((h, p)) = stripped.split_once('/') {
            (h, p)
        } else {
            return ProviderUrls::default();
        }
    } else if let Some(stripped) = raw_url.strip_prefix("https://") {
        if let Some((h, p)) = stripped.split_once('/') {
            (h, p)
        } else {
            return ProviderUrls::default();
        }
    } else if let Some(stripped) = raw_url.strip_prefix("http://") {
        if let Some((h, p)) = stripped.split_once('/') {
            (h, p)
        } else {
            return ProviderUrls::default();
        }
    } else {
        return ProviderUrls::default();
    };

    let clean_path = path.trim_end_matches(".git").trim_matches('/');
    let repo_url = format!("https://{}/{}", host, clean_path);

    let is_gitlab = host.contains("gitlab");
    let is_bitbucket = host.contains("bitbucket");

    let commit_url = commit_oid.map(|oid| {
        if is_gitlab {
            format!("{}/-/commit/{}", repo_url, oid)
        } else if is_bitbucket {
            format!("{}/commits/{}", repo_url, oid)
        } else {
            format!("{}/commit/{}", repo_url, oid)
        }
    });

    let branch_url = branch_opt.map(|b| {
        if is_gitlab {
            format!("{}/-/tree/{}", repo_url, b)
        } else if is_bitbucket {
            format!("{}/src/{}", repo_url, b)
        } else {
            format!("{}/tree/{}", repo_url, b)
        }
    });

    let pr_url = branch_opt.map(|b| {
        if is_gitlab {
            format!(
                "{}/-/merge_requests/new?merge_request%5Bsource_branch%5D={}",
                repo_url, b
            )
        } else if is_bitbucket {
            format!("{}/pull-requests/new?source={}", repo_url, b)
        } else {
            format!("{}/compare/{}?expand=1", repo_url, b)
        }
    });

    ProviderUrls {
        repo_url: Some(repo_url),
        commit_url,
        branch_url,
        pr_url,
    }
}

/// Fetches objects and updates tracking refs from remote repository.
pub fn fetch_from_remote(git_dir: &Path, remote_opt: Option<&str>) -> Result<String, TuiError> {
    let config = load_repo_config(git_dir)?;

    let remote_name = remote_opt.unwrap_or("origin");
    let url = config
        .get_remote_url(remote_name)
        .ok_or_else(|| TuiError::Terminal(format!("No configured remote '{}'", remote_name)))?;

    let ref_store = RefStore::new(git_dir);
    let (remote_refs, server_caps, is_local, is_ssh) =
        if let Some(local_path) = resolve_local_path(url) {
            let (refs, _) =
                discover_local_refs(&local_path).map_err(|e| TuiError::Terminal(e.to_string()))?;
            (refs, Vec::new(), Some(local_path), false)
        } else if is_ssh_url(url) {
            let client = SshClient::new();
            let (refs, caps, _) = client
                .discover_upload_pack(url)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            (refs, caps, None, true)
        } else {
            let client = SmartHttpClient::new();
            let (refs, caps, _) = client
                .discover_upload_pack(url)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            (refs, caps, None, false)
        };

    let mut wants = Vec::new();
    let mut updated_refs: Vec<(String, ObjectId)> = Vec::new();

    for r in &remote_refs {
        if let Some(branch) = r.name.strip_prefix("refs/heads/") {
            let tracking_ref = format!("refs/remotes/{}/{}", remote_name, branch);
            let local_tracking_oid = match ref_store.read_ref(&tracking_ref) {
                Ok(oid) => Some(oid),
                Err(RefError::NotFound(_)) => None,
                Err(e) => {
                    return Err(TuiError::Terminal(format!(
                        "cannot read local tracking ref '{}': {}",
                        tracking_ref, e
                    )))
                }
            };
            if local_tracking_oid != Some(r.oid) {
                wants.push(r.oid);
                updated_refs.push((tracking_ref, r.oid));
            }
        }
    }

    if wants.is_empty() {
        return Ok("Everything up-to-date".to_string());
    }

    let pack_bytes = if let Some(ref local_path) = is_local {
        fetch_local_pack(local_path, &wants).map_err(|e| TuiError::Terminal(e.to_string()))?
    } else if is_ssh {
        let client = SshClient::new();
        client
            .fetch_pack_with_caps(url, &wants, &[], &server_caps)
            .map_err(|e| TuiError::Terminal(e.to_string()))?
            .0
    } else {
        let client = SmartHttpClient::new();
        client
            .fetch_pack_with_caps(url, &wants, &[], &server_caps)
            .map_err(|e| TuiError::Terminal(e.to_string()))?
            .0
    };

    if !pack_bytes.is_empty() {
        let (indexed_objs, pack_checksum) = oxidize_pack::index_packfile(&pack_bytes)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        let pack_dir = git_dir.join("objects").join("pack");
        fs::create_dir_all(&pack_dir)?;
        let pack_file = pack_dir.join(format!("pack-{}.pack", pack_checksum));
        let idx_file = pack_dir.join(format!("pack-{}.idx", pack_checksum));
        fs::write(&pack_file, &pack_bytes)?;
        PackIndex::write_to(indexed_objs, &pack_checksum, &idx_file)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
    }

    for (target_ref, oid) in &updated_refs {
        ref_store
            .update_ref(target_ref, oid, None, "fetch: update tracking ref")
            .map_err(|e| {
                TuiError::Terminal(format!(
                    "fetch completed, but tracking ref '{}' could not be updated: {}",
                    target_ref, e
                ))
            })?;
    }

    Ok(format!(
        "Fetched {} objects from {}",
        wants.len(),
        remote_name
    ))
}

/// Pulls latest commits from remote and integrates into the active branch.
pub fn pull_from_remote(
    repo_root: &Path,
    git_dir: &Path,
    remote_opt: Option<&str>,
    branch_opt: Option<&str>,
) -> Result<String, TuiError> {
    let fetch_res = fetch_from_remote(git_dir, remote_opt)?;

    let ref_store = RefStore::new(git_dir);
    let (current_branch, head_oid_opt) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let remote_name = remote_opt.unwrap_or("origin");
    let branch_name = branch_opt.unwrap_or(&current_branch);

    let target_ref = format!("refs/remotes/{}/{}", remote_name, branch_name);
    let target_oid = match ref_store.read_ref(&target_ref) {
        Ok(oid) => oid,
        Err(_) => return Ok(fetch_res),
    };

    let head_oid = match head_oid_opt {
        Some(h) => h,
        None => {
            // Initial checkout
            checkout_branch(repo_root, git_dir, branch_name)?;
            return Ok(format!("Checked out {}", branch_name));
        }
    };

    if head_oid == target_oid {
        return Ok("Already up to date.".to_string());
    }

    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;

    // Fast-forward check
    if is_ancestor(&store, &head_oid, &target_oid)? {
        let target_commit = match store.read_object(&target_oid) {
            Ok(Object::Commit(c)) => c,
            _ => return Err(TuiError::Terminal("Target ref is not a commit".to_string())),
        };

        checkout_tree_and_update_index(repo_root, git_dir, &target_commit.tree, false)?;
        ref_store
            .update_ref(
                &format!("refs/heads/{}", current_branch),
                &target_oid,
                Some(&head_oid),
                &format!("pull: fast-forward to {}", target_ref),
            )
            .map_err(|e| TuiError::Terminal(e.to_string()))?;

        return Ok(format!("Fast-forward to {}", &target_oid.to_string()[..7]));
    }

    Err(TuiError::Terminal(
        "Cannot fast-forward; please resolve merge manually".to_string(),
    ))
}

fn bytes_equal_ignoring_crlf(a: &[u8], b: &[u8]) -> bool {
    if a == b {
        return true;
    }
    if oxidize_diff::is_binary_content(a) || oxidize_diff::is_binary_content(b) {
        return false;
    }
    fn normalize_crlf(bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\r' && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                i += 1;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
        }
        out
    }
    normalize_crlf(a) == normalize_crlf(b)
}

/// Renames a branch from `old_name` to `new_name`, updating refs, HEAD, reflogs, and `.git/config`.
pub fn rename_branch(
    _repo_root: &Path,
    git_dir: &Path,
    common_dir: &Path,
    old_name: &str,
    new_name: &str,
) -> Result<(), TuiError> {
    let config_path = common_dir.join("config");
    let mut config = GitConfig::load_from_file(&config_path)
        .map_err(|e| TuiError::Terminal(format!("cannot read repository config: {}", e)))?;
    let original_config = if config_path.exists() {
        Some(fs::read(&config_path)?)
    } else {
        None
    };
    let config_changed = config
        .rename_subsection("branch", old_name, new_name)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    if config_changed {
        config.save_to_file(&config_path).map_err(|e| {
            TuiError::Terminal(format!("cannot update branch configuration: {}", e))
        })?;
    }

    let ref_store = RefStore::with_common_dir(git_dir, common_dir);
    if let Err(error) = ref_store.rename_branch(old_name, new_name) {
        if config_changed {
            if let Some(original_config) = original_config.as_deref() {
                return Err(rollback_config(
                    &config_path,
                    original_config,
                    format!("failed to rename branch refs: {}", error),
                ));
            }
        }
        return Err(TuiError::Terminal(format!(
            "failed to rename branch refs: {}",
            error
        )));
    }

    Ok(())
}

/// Fast-forward merges `target_branch` into the current active branch.
pub fn fast_forward_merge(
    repo_root: &Path,
    git_dir: &Path,
    common_dir: &Path,
    target_branch: &str,
) -> Result<(), TuiError> {
    let ref_store = RefStore::with_common_dir(git_dir, common_dir);
    let store = RepoObjectStore::open_with_common_dir(git_dir, common_dir)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let (active_branch, head_oid_opt) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    if active_branch == target_branch {
        return Err(TuiError::Terminal(format!(
            "Cannot merge branch '{}' into itself",
            target_branch
        )));
    }

    let head_oid = head_oid_opt.ok_or_else(|| {
        TuiError::Terminal("Cannot merge into an empty repository with no commits".to_string())
    })?;

    let target_oid = ref_store
        .read_ref(&format!("refs/heads/{}", target_branch))
        .map_err(|_| TuiError::Terminal(format!("Target branch '{}' not found", target_branch)))?;

    if head_oid == target_oid {
        return Ok(()); // Already up to date
    }

    // Check if HEAD is an ancestor of target (required for fast-forward)
    if !is_ancestor(&store, &head_oid, &target_oid)? {
        return Err(TuiError::Terminal(format!(
            "Cannot fast-forward merge '{}': not a direct descendant of active branch '{}'",
            target_branch, active_branch
        )));
    }

    // Read target commit to get tree OID
    let target_commit = match store
        .read_object(&target_oid)
        .map_err(|e| TuiError::Terminal(e.to_string()))?
    {
        Object::Commit(c) => c,
        _ => return Err(TuiError::Terminal("Target ref is not a commit".to_string())),
    };

    // Update working tree and index safely (preflighting collisions)
    checkout_tree_and_update_index(repo_root, git_dir, &target_commit.tree, false)?;

    // Update branch ref
    let ref_name = format!("refs/heads/{}", active_branch);
    let message = format!("merge {}: Fast-forward", target_branch);
    ref_store
        .update_ref(&ref_name, &target_oid, Some(&head_oid), &message)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    Ok(())
}

/// Checks out a historical commit into detached HEAD state.
pub fn checkout_commit(
    repo_root: &Path,
    git_dir: &Path,
    common_dir: &Path,
    target_oid: &ObjectId,
) -> Result<(), TuiError> {
    let store = RepoObjectStore::open_with_common_dir(git_dir, common_dir)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let ref_store = RefStore::with_common_dir(git_dir, common_dir);

    let commit = match store
        .read_object(target_oid)
        .map_err(|e| TuiError::Terminal(e.to_string()))?
    {
        Object::Commit(c) => c,
        _ => return Err(TuiError::Terminal("Target is not a commit".to_string())),
    };

    // Preflight and checkout working tree & index
    checkout_tree_and_update_index(repo_root, git_dir, &commit.tree, false)?;

    // Detach HEAD
    ref_store
        .set_head_detached(target_oid)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let sig = get_signature(git_dir);
    let msg = format!("checkout: moving to {}", target_oid);
    ref_store
        .append_reflog("HEAD", target_oid, target_oid, &sig, &msg)
        .map_err(|e| {
            TuiError::Terminal(format!(
                "checkout completed, but HEAD reflog update failed: {}",
                e
            ))
        })?;

    Ok(())
}

/// Applies a historical commit onto current HEAD using native three-way merge.
pub fn cherry_pick_commit(
    repo_root: &Path,
    git_dir: &Path,
    common_dir: &Path,
    target_oid: &ObjectId,
) -> Result<ObjectId, TuiError> {
    let store = RepoObjectStore::open_with_common_dir(git_dir, common_dir)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let ref_store = RefStore::with_common_dir(git_dir, common_dir);
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;

    let (active_branch, head_oid_opt) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let head_oid = head_oid_opt.ok_or_else(|| {
        TuiError::Terminal("Cannot cherry-pick in an empty repository with no HEAD".to_string())
    })?;

    let target_commit = match store
        .read_object(target_oid)
        .map_err(|e| TuiError::Terminal(e.to_string()))?
    {
        Object::Commit(c) => c,
        _ => {
            return Err(TuiError::Terminal(
                "Target object is not a commit".to_string(),
            ))
        }
    };

    let head_commit = match store
        .read_object(&head_oid)
        .map_err(|e| TuiError::Terminal(e.to_string()))?
    {
        Object::Commit(c) => c,
        _ => {
            return Err(TuiError::Terminal(
                "HEAD object is not a commit".to_string(),
            ))
        }
    };

    let base_files = if let Some(parent_oid) = target_commit.parents.first() {
        let parent = match store
            .read_object(parent_oid)
            .map_err(|e| TuiError::Terminal(e.to_string()))?
        {
            Object::Commit(commit) => commit,
            _ => {
                return Err(TuiError::Terminal(
                    "cherry-pick parent is not a commit".to_string(),
                ))
            }
        };
        flatten_tree(&store, &parent.tree, "").map_err(|e| TuiError::Terminal(e.to_string()))?
    } else {
        BTreeMap::new()
    };

    let our_files = flatten_tree(&store, &head_commit.tree, "")
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let their_files = flatten_tree(&store, &target_commit.tree, "")
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let mut all_paths = BTreeSet::new();
    for p in base_files.keys() {
        all_paths.insert(p.clone());
    }
    for p in our_files.keys() {
        all_paths.insert(p.clone());
    }
    for p in their_files.keys() {
        all_paths.insert(p.clone());
    }

    let loose_store = LooseObjectStore::new(common_dir.join("objects"));
    let mut had_conflicts = false;

    for path in all_paths {
        let base_entry = base_files.get(&path);
        let our_entry = our_files.get(&path);
        let their_entry = their_files.get(&path);

        // If target commit did not touch this path, keep our entry
        if base_entry == their_entry {
            continue;
        }

        // If our tree matches base, take their entry cleanly
        if base_entry == our_entry {
            if let Some(&(_their_mode, their_blob_oid)) = their_entry {
                let full_path = repo_root.join(&path);
                if let Some(parent) = full_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let blob_data = store
                    .read_raw(&their_blob_oid)
                    .map_err(|e| TuiError::Terminal(e.to_string()))?
                    .1;
                fs::write(&full_path, &blob_data)?;
                let meta = fs::metadata(&full_path)?;
                index.add_entry(IndexEntry::from_fs_metadata(
                    path.clone(),
                    their_blob_oid,
                    &meta,
                    0,
                ));
            } else {
                // File deleted in their commit
                let full_path = repo_root.join(&path);
                if full_path.exists() {
                    fs::remove_file(&full_path)?;
                }
                index.remove_entry(&path);
            }
            continue;
        }

        // If both sides made the exact same change
        if our_entry == their_entry {
            continue;
        }

        // Three-way merge needed
        let read_text = |entry: Option<&(oxidize_core::object::FileMode, ObjectId)>| {
            entry
                .map(|(_, id)| {
                    store
                        .read_raw(id)
                        .map(|(_, data)| String::from_utf8_lossy(&data).into_owned())
                        .map_err(|e| TuiError::Terminal(e.to_string()))
                })
                .transpose()
                .map(|text| text.unwrap_or_default())
        };
        let base_text = read_text(base_entry)?;
        let our_text = read_text(our_entry)?;
        let their_text = read_text(their_entry)?;

        let merged = three_way_merge(
            &base_text,
            &our_text,
            &their_text,
            "HEAD",
            &target_oid.to_string()[..7],
        );
        let full_path = repo_root.join(&path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&full_path, merged.content.as_bytes())?;

        let blob_oid = loose_store
            .write_object(&Object::Blob(Blob::new(merged.content.into_bytes())))
            .map_err(|e| TuiError::Terminal(e.to_string()))?;

        let meta = fs::metadata(&full_path)?;
        let stage = if merged.has_conflicts { 1 } else { 0 };
        index.add_entry(IndexEntry::from_fs_metadata(
            path.clone(),
            blob_oid,
            &meta,
            stage,
        ));

        if merged.has_conflicts {
            had_conflicts = true;
        }
    }

    index
        .write_to(&index_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    if had_conflicts {
        return Err(TuiError::Terminal(format!(
            "Cherry-pick of {} resulted in merge conflicts",
            &target_oid.to_string()[..7]
        )));
    }

    let tree_oid =
        write_tree(&index, store.loose()).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let sig = get_signature(git_dir);
    let new_commit = Commit {
        tree: tree_oid,
        parents: vec![head_oid],
        author: target_commit.author,
        committer: sig,
        gpg_sig: None,
        message: target_commit.message.clone(),
    };
    let new_oid = loose_store
        .write_object(&Object::Commit(new_commit))
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let first_line = target_commit.message.lines().next().unwrap_or("");
    let ref_msg = format!("cherry-pick: {}", first_line);

    if active_branch == "HEAD" || active_branch.is_empty() {
        ref_store
            .set_head_detached(&new_oid)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
    } else {
        ref_store
            .update_ref(
                &format!("refs/heads/{}", active_branch),
                &new_oid,
                Some(&head_oid),
                &ref_msg,
            )
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
    }

    Ok(new_oid)
}

/// Resets the current branch / HEAD to `target_oid` using Soft, Mixed, or Hard mode.
pub fn reset_to_commit(
    repo_root: &Path,
    git_dir: &Path,
    common_dir: &Path,
    target_oid: &ObjectId,
    mode: ResetMode,
) -> Result<(), TuiError> {
    let store = RepoObjectStore::open_with_common_dir(git_dir, common_dir)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let ref_store = RefStore::with_common_dir(git_dir, common_dir);

    let target_commit = match store
        .read_object(target_oid)
        .map_err(|e| TuiError::Terminal(e.to_string()))?
    {
        Object::Commit(c) => c,
        _ => return Err(TuiError::Terminal("Target is not a commit".to_string())),
    };

    let (active_branch, head_oid_opt) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let old_head = head_oid_opt.unwrap_or(ObjectId::ZERO);

    match mode {
        ResetMode::Soft => {
            // Only move ref pointer
        }
        ResetMode::Mixed => {
            // Reset index to target tree, leave working tree untouched
            let index_path = git_dir.join("index");
            let mut index = Index::new();
            let target_map = flatten_tree(&store, &target_commit.tree, "")
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            for (path, (fmode, blob_oid)) in target_map {
                let full_path = repo_root.join(&path);
                let (mtime, file_size) = if let Ok(meta) = fs::metadata(&full_path) {
                    let mtime = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as u32)
                        .unwrap_or(0);
                    let sz = meta.len() as u32;
                    (mtime, sz)
                } else {
                    (0, 0)
                };

                let mut entry = IndexEntry::new(path, blob_oid, fmode.0);
                entry.file_size = file_size;
                entry.mtime_sec = mtime;
                index.add_entry(entry);
            }
            index
                .write_to(&index_path)
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
        }
        ResetMode::Hard => {
            // Reset index and working tree to target tree
            checkout_tree_and_update_index(repo_root, git_dir, &target_commit.tree, true)?;
        }
    }

    // Move branch pointer or detached HEAD
    let ref_msg = format!("reset: moving to {}", target_oid);
    if active_branch == "HEAD" || active_branch.is_empty() {
        ref_store
            .set_head_detached(target_oid)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
    } else {
        let branch_ref = format!("refs/heads/{}", active_branch);
        ref_store
            .update_ref(&branch_ref, target_oid, None, &ref_msg)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
    }

    let sig = get_signature(git_dir);
    ref_store
        .append_reflog("HEAD", &old_head, target_oid, &sig, &ref_msg)
        .map_err(|e| {
            TuiError::Terminal(format!(
                "reset completed, but HEAD reflog update failed: {e}"
            ))
        })?;

    Ok(())
}

/// Creates a lightweight tag pointing to `target_oid`.
pub fn create_tag(
    git_dir: &Path,
    common_dir: &Path,
    name: &str,
    target_oid: &ObjectId,
) -> Result<(), TuiError> {
    let ref_store = RefStore::with_common_dir(git_dir, common_dir);
    ref_store
        .create_tag(name, target_oid)
        .map_err(|e| TuiError::Terminal(e.to_string()))
}

/// Deletes a tag by name.
pub fn delete_tag(git_dir: &Path, common_dir: &Path, name: &str) -> Result<(), TuiError> {
    let ref_store = RefStore::with_common_dir(git_dir, common_dir);
    ref_store
        .delete_tag(name)
        .map_err(|e| TuiError::Terminal(e.to_string()))
}

/// Result of executing rebase steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayStepOutcome {
    Finished,
    StoppedForEditing { commit_oid: ObjectId },
    Conflict { conflicted_paths: Vec<String> },
}

/// Merges `their_tree_oid` onto `our_tree_oid` using `base_tree_oid` as common ancestor.
/// Updates the working tree on disk and writes stage 0 entries to `index` on clean merges,
/// or stages 1, 2, 3 and conflict markers on conflicts.
#[allow(clippy::too_many_arguments)]
pub fn merge_trees_into_index_and_worktree(
    repo_root: &Path,
    git_dir: &Path,
    common_dir: &Path,
    index: &mut Index,
    base_tree_oid: &ObjectId,
    our_tree_oid: &ObjectId,
    their_tree_oid: &ObjectId,
    our_label: &str,
    their_label: &str,
) -> Result<Vec<String>, TuiError> {
    let store = RepoObjectStore::open_with_common_dir(git_dir, common_dir)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let loose_store = LooseObjectStore::new(common_dir.join("objects"));

    let base_map = if *base_tree_oid != ObjectId::ZERO {
        flatten_tree(&store, base_tree_oid, "").map_err(|e| TuiError::Terminal(e.to_string()))?
    } else {
        BTreeMap::new()
    };
    let our_map = if *our_tree_oid != ObjectId::ZERO {
        flatten_tree(&store, our_tree_oid, "").map_err(|e| TuiError::Terminal(e.to_string()))?
    } else {
        BTreeMap::new()
    };
    let their_map = if *their_tree_oid != ObjectId::ZERO {
        flatten_tree(&store, their_tree_oid, "").map_err(|e| TuiError::Terminal(e.to_string()))?
    } else {
        BTreeMap::new()
    };

    let mut all_paths = BTreeSet::new();
    for p in base_map.keys() {
        all_paths.insert(p.clone());
    }
    for p in our_map.keys() {
        all_paths.insert(p.clone());
    }
    for p in their_map.keys() {
        all_paths.insert(p.clone());
    }

    let mut conflicted_paths = Vec::new();

    for path in all_paths {
        let base_entry = base_map.get(&path);
        let our_entry = our_map.get(&path);
        let their_entry = their_map.get(&path);

        // Case 1: Both sides identical -> keep our_entry
        if our_entry == their_entry {
            continue;
        }

        // Case 2: Their side didn't touch it -> keep our_entry
        if base_entry == their_entry {
            continue;
        }

        // Case 3: Our side didn't touch it, their side changed it
        if base_entry == our_entry {
            let full_path = match oxidize_core::safe_join(repo_root, &path) {
                Ok(p) => p,
                Err(e) => return Err(TuiError::Terminal(e.to_string())),
            };
            if let Some((their_mode, their_blob_oid)) = their_entry {
                if let Some(parent) = full_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let blob = match store
                    .read_object(their_blob_oid)
                    .map_err(|e| TuiError::Terminal(e.to_string()))?
                {
                    Object::Blob(blob) => blob,
                    _ => {
                        return Err(TuiError::Terminal(format!(
                            "merge expected blob for '{}'",
                            path
                        )))
                    }
                };
                fs::write(&full_path, &blob.data)?;
                let meta = fs::metadata(&full_path)?;
                let file_size = meta.len() as u32;
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as u32)
                    .unwrap_or(0);

                let mut entry = IndexEntry::new(path.clone(), *their_blob_oid, their_mode.0);
                entry.file_size = file_size;
                entry.mtime_sec = mtime;
                index.add_entry(entry);
            } else {
                // Deleted in their branch
                if full_path.exists() {
                    fs::remove_file(&full_path)?;
                }
                index.remove_entry(&path);
            }
            continue;
        }

        // Case 4: Both sides touched the file -> 3-way content merge
        let full_path = match oxidize_core::safe_join(repo_root, &path) {
            Ok(p) => p,
            Err(e) => return Err(TuiError::Terminal(e.to_string())),
        };

        let base_raw = match base_entry {
            Some((_, id)) => Some(
                store
                    .read_raw(id)
                    .map_err(|e| TuiError::Terminal(e.to_string()))?
                    .1,
            ),
            None => None,
        };
        let our_raw = match our_entry {
            Some((_, id)) => Some(
                store
                    .read_raw(id)
                    .map_err(|e| TuiError::Terminal(e.to_string()))?
                    .1,
            ),
            None => None,
        };
        let their_raw = match their_entry {
            Some((_, id)) => Some(
                store
                    .read_raw(id)
                    .map_err(|e| TuiError::Terminal(e.to_string()))?
                    .1,
            ),
            None => None,
        };

        let is_binary = base_raw
            .as_deref()
            .is_some_and(oxidize_diff::is_binary_content)
            || our_raw
                .as_deref()
                .is_some_and(oxidize_diff::is_binary_content)
            || their_raw
                .as_deref()
                .is_some_and(oxidize_diff::is_binary_content)
            || base_raw
                .as_deref()
                .is_some_and(|b| std::str::from_utf8(b).is_err())
            || our_raw
                .as_deref()
                .is_some_and(|b| std::str::from_utf8(b).is_err())
            || their_raw
                .as_deref()
                .is_some_and(|b| std::str::from_utf8(b).is_err());

        let is_modify_delete =
            base_entry.is_some() && (our_entry.is_none() || their_entry.is_none());
        let is_mode_conflict = match (our_entry, their_entry) {
            (Some((m1, _)), Some((m2, _))) => m1.0 != m2.0,
            _ => false,
        };

        if is_binary || is_modify_delete || is_mode_conflict {
            conflicted_paths.push(path.clone());
            index.remove_entry(&path);
            if let Some((base_mode, base_oid)) = base_entry {
                let mut e1 = IndexEntry::new(path.clone(), *base_oid, base_mode.0);
                e1.stage = 1;
                index.add_entry(e1);
            }
            if let Some((our_mode, our_oid)) = our_entry {
                let mut e2 = IndexEntry::new(path.clone(), *our_oid, our_mode.0);
                e2.stage = 2;
                index.add_entry(e2);
            }
            if let Some((their_mode, their_oid)) = their_entry {
                let mut e3 = IndexEntry::new(path.clone(), *their_oid, their_mode.0);
                e3.stage = 3;
                index.add_entry(e3);
            }
            // Preserve raw our bytes in the worktree if present
            if let Some(ref data) = our_raw {
                if let Some(parent) = full_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&full_path, data)?;
            } else if full_path.exists() {
                fs::remove_file(&full_path)?;
            }
            continue;
        }

        let base_text = std::str::from_utf8(base_raw.as_deref().unwrap_or(b"")).unwrap_or("");
        let our_text = std::str::from_utf8(our_raw.as_deref().unwrap_or(b"")).unwrap_or("");
        let their_text = std::str::from_utf8(their_raw.as_deref().unwrap_or(b"")).unwrap_or("");

        let merged = three_way_merge(base_text, our_text, their_text, our_label, their_label);

        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&full_path, merged.content.as_bytes())?;

        if merged.has_conflicts {
            conflicted_paths.push(path.clone());
            index.remove_entry(&path);
            if let Some((base_mode, base_oid)) = base_entry {
                let mut e1 = IndexEntry::new(path.clone(), *base_oid, base_mode.0);
                e1.stage = 1;
                index.add_entry(e1);
            }
            if let Some((our_mode, our_oid)) = our_entry {
                let mut e2 = IndexEntry::new(path.clone(), *our_oid, our_mode.0);
                e2.stage = 2;
                index.add_entry(e2);
            }
            if let Some((their_mode, their_oid)) = their_entry {
                let mut e3 = IndexEntry::new(path.clone(), *their_oid, their_mode.0);
                e3.stage = 3;
                index.add_entry(e3);
            }
        } else {
            let blob_oid = loose_store
                .write_object(&Object::Blob(Blob::new(merged.content.into_bytes())))
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            let meta = fs::metadata(&full_path)?;
            let file_size = meta.len() as u32;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as u32)
                .unwrap_or(0);

            let mode = our_entry
                .or(their_entry)
                .map(|(m, _)| m.0)
                .unwrap_or(0o100644);
            let mut entry = IndexEntry::new(path.clone(), blob_oid, mode);
            entry.file_size = file_size;
            entry.mtime_sec = mtime;
            index.add_entry(entry);
        }
    }

    Ok(conflicted_paths)
}

/// Starts an interactive rebase with the specified plan onto `onto`.
pub fn start_interactive_rebase(
    repo_root: &Path,
    git_dir: &Path,
    common_dir: &Path,
    onto: &ObjectId,
    todo_items: Vec<RebaseTodoItem>,
) -> Result<ReplayStepOutcome, TuiError> {
    if SequencerState::load(git_dir)?.is_some() {
        return Err(TuiError::Terminal(
            "A rebase or sequencer operation is already in progress".to_string(),
        ));
    }

    let ref_store = RefStore::with_common_dir(git_dir, common_dir);
    let store = RepoObjectStore::open_with_common_dir(git_dir, common_dir)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let (active_branch, head_oid_opt) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let orig_head =
        head_oid_opt.ok_or_else(|| TuiError::Terminal("HEAD has no commits".to_string()))?;

    let index_path = git_dir.join("index");
    let index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let orig_head_commit = match store
        .read_object(&orig_head)
        .map_err(|e| TuiError::Terminal(e.to_string()))?
    {
        Object::Commit(c) => c,
        _ => return Err(TuiError::Terminal("HEAD is not a commit".to_string())),
    };

    let status =
        oxidize_index::compute_status(repo_root, &index, Some(&orig_head_commit.tree), &store)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
    if !status.staged.is_empty() || !status.unstaged.is_empty() {
        return Err(TuiError::Terminal(
            "cannot rebase: you have uncommitted changes. Please commit or stash them.".to_string(),
        ));
    }

    let onto_commit = match store
        .read_object(onto)
        .map_err(|e| TuiError::Terminal(e.to_string()))?
    {
        Object::Commit(c) => c,
        _ => {
            return Err(TuiError::Terminal(
                "Onto target is not a commit".to_string(),
            ))
        }
    };

    let total_steps = todo_items.len();
    let state = SequencerState {
        head_name: active_branch,
        orig_head,
        onto: *onto,
        current_step: 1,
        total_steps,
        todo: todo_items,
        done: Vec::new(),
        stopped_sha: None,
        status: SequencerStatus::Running,
    };
    state.save(git_dir)?;

    // Reset working tree and index to onto commit tree with collision checks
    if let Err(e) = checkout_tree_and_update_index(repo_root, git_dir, &onto_commit.tree, false) {
        remove_dir_all_if_exists(&git_dir.join("rebase-merge"))?;
        return Err(e);
    }
    if let Err(e) = ref_store.set_head_detached(onto) {
        remove_dir_all_if_exists(&git_dir.join("rebase-merge"))?;
        return Err(TuiError::Terminal(e.to_string()));
    }

    run_rebase_loop(repo_root, git_dir, common_dir)
}

/// Loops through rebase todo items until completion, conflict, or edit stop.
pub fn run_rebase_loop(
    repo_root: &Path,
    git_dir: &Path,
    common_dir: &Path,
) -> Result<ReplayStepOutcome, TuiError> {
    let mut state = match SequencerState::load(git_dir)? {
        Some(s) => s,
        None => return Ok(ReplayStepOutcome::Finished),
    };

    let store = RepoObjectStore::open_with_common_dir(git_dir, common_dir)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let loose_store = LooseObjectStore::new(common_dir.join("objects"));
    let ref_store = RefStore::with_common_dir(git_dir, common_dir);
    let index_path = git_dir.join("index");

    while !state.todo.is_empty() {
        let item = state.todo.remove(0);
        let commit_oid = item.commit_oid;
        let commit = match store
            .read_object(&commit_oid)
            .map_err(|e| TuiError::Terminal(e.to_string()))?
        {
            Object::Commit(c) => c,
            _ => {
                return Err(TuiError::Terminal(format!(
                    "Object {} is not a commit",
                    commit_oid
                )))
            }
        };

        let current_head_oid = ref_store
            .resolve_head()
            .map_err(|e| TuiError::Terminal(e.to_string()))?
            .1
            .ok_or_else(|| TuiError::Terminal("HEAD missing during rebase".to_string()))?;

        let head_commit = match store
            .read_object(&current_head_oid)
            .map_err(|e| TuiError::Terminal(e.to_string()))?
        {
            Object::Commit(c) => c,
            _ => return Err(TuiError::Terminal("HEAD is not a commit".to_string())),
        };

        match item.action {
            RebaseAction::Drop => {
                state.done.push(item);
                state.current_step += 1;
                state.save(git_dir)?;
                continue;
            }
            RebaseAction::Pick | RebaseAction::Reword | RebaseAction::Edit => {
                let base_tree_oid = if !commit.parents.is_empty() {
                    let parent_commit = match store
                        .read_object(&commit.parents[0])
                        .map_err(|e| TuiError::Terminal(e.to_string()))?
                    {
                        Object::Commit(c) => c,
                        _ => return Err(TuiError::Terminal("Parent is not a commit".to_string())),
                    };
                    parent_commit.tree
                } else {
                    ObjectId::ZERO
                };

                let mut index =
                    Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;

                let conflicts = merge_trees_into_index_and_worktree(
                    repo_root,
                    git_dir,
                    common_dir,
                    &mut index,
                    &base_tree_oid,
                    &head_commit.tree,
                    &commit.tree,
                    "HEAD",
                    &item.short_oid,
                )?;

                index
                    .write_to(&index_path)
                    .map_err(|e| TuiError::Terminal(e.to_string()))?;

                if !conflicts.is_empty() {
                    state.status = SequencerStatus::Conflicted;
                    state.stopped_sha = Some(commit_oid);
                    state.todo.insert(0, item);
                    state.save(git_dir)?;
                    return Ok(ReplayStepOutcome::Conflict {
                        conflicted_paths: conflicts,
                    });
                }

                let new_tree_oid = write_tree(&index, store.loose())
                    .map_err(|e| TuiError::Terminal(e.to_string()))?;
                let sig = get_signature(git_dir);
                let msg = if item.action == RebaseAction::Reword {
                    item.message.clone().unwrap_or(commit.message.clone())
                } else {
                    commit.message.clone()
                };

                let new_commit = Commit {
                    tree: new_tree_oid,
                    parents: vec![current_head_oid],
                    author: commit.author,
                    committer: sig,
                    gpg_sig: None,
                    message: msg,
                };
                let new_commit_oid = loose_store
                    .write_object(&Object::Commit(new_commit))
                    .map_err(|e| TuiError::Terminal(e.to_string()))?;

                ref_store
                    .set_head_detached(&new_commit_oid)
                    .map_err(|e| TuiError::Terminal(e.to_string()))?;

                if item.action == RebaseAction::Edit {
                    state.status = SequencerStatus::StoppedForEditing;
                    state.stopped_sha = Some(new_commit_oid);
                    state.done.push(item);
                    state.current_step += 1;
                    state.save(git_dir)?;
                    return Ok(ReplayStepOutcome::StoppedForEditing {
                        commit_oid: new_commit_oid,
                    });
                }

                state.done.push(item);
                state.current_step += 1;
                state.save(git_dir)?;
            }
            RebaseAction::Squash | RebaseAction::Fixup => {
                let base_tree_oid = if !commit.parents.is_empty() {
                    let parent_commit = match store
                        .read_object(&commit.parents[0])
                        .map_err(|e| TuiError::Terminal(e.to_string()))?
                    {
                        Object::Commit(c) => c,
                        _ => return Err(TuiError::Terminal("Parent is not a commit".to_string())),
                    };
                    parent_commit.tree
                } else {
                    ObjectId::ZERO
                };

                let mut index =
                    Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;

                let conflicts = merge_trees_into_index_and_worktree(
                    repo_root,
                    git_dir,
                    common_dir,
                    &mut index,
                    &base_tree_oid,
                    &head_commit.tree,
                    &commit.tree,
                    "HEAD",
                    &item.short_oid,
                )?;

                index
                    .write_to(&index_path)
                    .map_err(|e| TuiError::Terminal(e.to_string()))?;

                if !conflicts.is_empty() {
                    state.status = SequencerStatus::Conflicted;
                    state.stopped_sha = Some(commit_oid);
                    state.todo.insert(0, item);
                    state.save(git_dir)?;
                    return Ok(ReplayStepOutcome::Conflict {
                        conflicted_paths: conflicts,
                    });
                }

                let new_tree_oid = write_tree(&index, store.loose())
                    .map_err(|e| TuiError::Terminal(e.to_string()))?;
                let sig = get_signature(git_dir);
                let msg = if item.action == RebaseAction::Squash {
                    format!(
                        "{}\n\n{}",
                        head_commit.message.trim(),
                        commit.message.trim()
                    )
                } else {
                    head_commit.message.clone()
                };

                let new_commit = Commit {
                    tree: new_tree_oid,
                    parents: head_commit.parents.clone(),
                    author: head_commit.author,
                    committer: sig,
                    gpg_sig: None,
                    message: msg,
                };
                let new_commit_oid = loose_store
                    .write_object(&Object::Commit(new_commit))
                    .map_err(|e| TuiError::Terminal(e.to_string()))?;

                ref_store
                    .set_head_detached(&new_commit_oid)
                    .map_err(|e| TuiError::Terminal(e.to_string()))?;

                state.done.push(item);
                state.current_step += 1;
                state.save(git_dir)?;
            }
        }
    }

    // All steps done! Finalize rebase:
    let final_head_oid = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?
        .1
        .ok_or_else(|| TuiError::Terminal("Final HEAD missing".to_string()))?;

    let head_name = state.head_name.clone();
    if head_name != "HEAD" && !head_name.is_empty() {
        let branch_name = head_name.strip_prefix("refs/heads/").unwrap_or(&head_name);
        let branch_ref = format!("refs/heads/{}", branch_name);
        ref_store
            .update_ref(&branch_ref, &final_head_oid, None, "rebase finished")
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        ref_store
            .set_head_symbolic(branch_name)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
    }

    SequencerState::clear(git_dir)?;
    Ok(ReplayStepOutcome::Finished)
}

/// Continues an active interactive rebase after resolving conflicts or editing.
pub fn rebase_continue(
    repo_root: &Path,
    git_dir: &Path,
    common_dir: &Path,
) -> Result<ReplayStepOutcome, TuiError> {
    let index_path = git_dir.join("index");
    let index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;
    if index.entries.iter().any(|e| e.stage > 0) {
        return Err(TuiError::Terminal(
            "Cannot continue rebase: you have unresolved merge conflicts. Resolve and stage all files first."
                .to_string(),
        ));
    }

    let mut state = match SequencerState::load(git_dir)? {
        Some(s) => s,
        None => return Ok(ReplayStepOutcome::Finished),
    };

    let store = RepoObjectStore::open_with_common_dir(git_dir, common_dir)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let loose_store = LooseObjectStore::new(common_dir.join("objects"));
    let ref_store = RefStore::with_common_dir(git_dir, common_dir);

    // If stopped on conflict, commit the resolved changes
    if state.status == SequencerStatus::Conflicted && !state.todo.is_empty() {
        let item = state.todo.remove(0);
        let commit_oid = item.commit_oid;
        let orig_commit = match store
            .read_object(&commit_oid)
            .map_err(|e| TuiError::Terminal(e.to_string()))?
        {
            Object::Commit(c) => c,
            _ => return Err(TuiError::Terminal("Invalid commit".to_string())),
        };

        let current_head_oid = ref_store
            .resolve_head()
            .map_err(|e| TuiError::Terminal(e.to_string()))?
            .1
            .ok_or_else(|| TuiError::Terminal("HEAD missing".to_string()))?;

        let head_commit = match store
            .read_object(&current_head_oid)
            .map_err(|e| TuiError::Terminal(e.to_string()))?
        {
            Object::Commit(c) => c,
            _ => return Err(TuiError::Terminal("HEAD is not a commit".to_string())),
        };

        let new_tree_oid =
            write_tree(&index, store.loose()).map_err(|e| TuiError::Terminal(e.to_string()))?;
        let sig = get_signature(git_dir);

        let (parents, author, msg) = match item.action {
            RebaseAction::Squash => {
                let m = format!(
                    "{}\n\n{}",
                    head_commit.message.trim(),
                    orig_commit.message.trim()
                );
                (head_commit.parents.clone(), head_commit.author, m)
            }
            RebaseAction::Fixup => (
                head_commit.parents.clone(),
                head_commit.author,
                head_commit.message.clone(),
            ),
            RebaseAction::Reword => {
                let m = item.message.clone().unwrap_or(orig_commit.message);
                (vec![current_head_oid], orig_commit.author, m)
            }
            _ => {
                let m = item.message.clone().unwrap_or(orig_commit.message);
                (vec![current_head_oid], orig_commit.author, m)
            }
        };

        let new_commit = Commit {
            tree: new_tree_oid,
            parents,
            author,
            committer: sig,
            gpg_sig: None,
            message: msg,
        };
        let new_commit_oid = loose_store
            .write_object(&Object::Commit(new_commit))
            .map_err(|e| TuiError::Terminal(e.to_string()))?;

        ref_store
            .set_head_detached(&new_commit_oid)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;

        state.done.push(item);
        state.current_step += 1;
        state.status = SequencerStatus::Running;
        state.stopped_sha = None;
        state.save(git_dir)?;
    } else if state.status == SequencerStatus::StoppedForEditing {
        state.status = SequencerStatus::Running;
        state.stopped_sha = None;
        state.save(git_dir)?;
    }

    run_rebase_loop(repo_root, git_dir, common_dir)
}

/// Skips the current conflicted/stopped step in rebase.
pub fn rebase_skip(
    repo_root: &Path,
    git_dir: &Path,
    common_dir: &Path,
) -> Result<ReplayStepOutcome, TuiError> {
    let mut state = match SequencerState::load(git_dir)? {
        Some(s) => s,
        None => return Ok(ReplayStepOutcome::Finished),
    };

    let ref_store = RefStore::with_common_dir(git_dir, common_dir);
    let store = RepoObjectStore::open_with_common_dir(git_dir, common_dir)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let current_head_oid = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?
        .1
        .ok_or_else(|| TuiError::Terminal("HEAD missing".to_string()))?;

    let head_commit = match store
        .read_object(&current_head_oid)
        .map_err(|e| TuiError::Terminal(e.to_string()))?
    {
        Object::Commit(c) => c,
        _ => return Err(TuiError::Terminal("HEAD is not a commit".to_string())),
    };

    // Reset WT and index to HEAD
    checkout_tree_and_update_index(repo_root, git_dir, &head_commit.tree, true)?;

    if !state.todo.is_empty() {
        let item = state.todo.remove(0);
        state.done.push(item);
        state.current_step += 1;
    }
    state.status = SequencerStatus::Running;
    state.stopped_sha = None;
    state.save(git_dir)?;

    run_rebase_loop(repo_root, git_dir, common_dir)
}

/// Aborts an active rebase, restoring original HEAD and branch pointer.
pub fn rebase_abort(repo_root: &Path, git_dir: &Path, common_dir: &Path) -> Result<(), TuiError> {
    let state = match SequencerState::load(git_dir)? {
        Some(s) => s,
        None => return Ok(()),
    };

    let store = RepoObjectStore::open_with_common_dir(git_dir, common_dir)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let ref_store = RefStore::with_common_dir(git_dir, common_dir);

    let orig_commit = match store
        .read_object(&state.orig_head)
        .map_err(|e| TuiError::Terminal(e.to_string()))?
    {
        Object::Commit(c) => c,
        _ => {
            return Err(TuiError::Terminal(
                "Original HEAD commit not found".to_string(),
            ))
        }
    };

    // Restore working tree and index to orig_head
    checkout_tree_and_update_index(repo_root, git_dir, &orig_commit.tree, true)?;

    if state.head_name != "HEAD" && !state.head_name.is_empty() {
        let branch_name = state
            .head_name
            .strip_prefix("refs/heads/")
            .unwrap_or(&state.head_name);
        let branch_ref = format!("refs/heads/{}", branch_name);
        ref_store
            .update_ref(
                &branch_ref,
                &state.orig_head,
                None,
                "rebase abort: returning to orig-head",
            )
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        ref_store
            .set_head_symbolic(branch_name)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
    } else {
        ref_store
            .set_head_detached(&state.orig_head)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
    }

    SequencerState::clear(git_dir)?;
    Ok(())
}

/// Resolves a conflicted file by choosing Ours, Theirs, or Both.
pub fn resolve_conflict_choice(
    repo_root: &Path,
    git_dir: &Path,
    common_dir: &Path,
    rel_path: &str,
    choice: ConflictChoice,
) -> Result<(), TuiError> {
    let store = RepoObjectStore::open_with_common_dir(git_dir, common_dir)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let loose_store = LooseObjectStore::new(common_dir.join("objects"));
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;

    let e2 = index
        .entries
        .iter()
        .find(|e| e.path == rel_path && e.stage == 2)
        .cloned();
    let e3 = index
        .entries
        .iter()
        .find(|e| e.path == rel_path && e.stage == 3)
        .cloned();

    let full_path = match oxidize_core::safe_join(repo_root, rel_path) {
        Ok(p) => p,
        Err(e) => return Err(TuiError::Terminal(e.to_string())),
    };

    match choice {
        ConflictChoice::Ours => {
            let e = e2.ok_or_else(|| {
                TuiError::Terminal(format!("No stage 2 (ours) entry for {}", rel_path))
            })?;
            let blob_data = match store
                .read_object(&e.oid)
                .map_err(|err| TuiError::Terminal(err.to_string()))?
            {
                Object::Blob(b) => b.data,
                _ => return Err(TuiError::Terminal("Not a blob".to_string())),
            };
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&full_path, &blob_data)?;
            let meta = fs::metadata(&full_path)?;
            let mut entry = IndexEntry::new(rel_path.to_string(), e.oid, e.mode);
            entry.file_size = meta.len() as u32;
            if let Ok(mtime) = meta.modified().and_then(|t| {
                t.duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| std::io::Error::other("err"))
            }) {
                entry.mtime_sec = mtime.as_secs() as u32;
            }
            entry.stage = 0;
            index.add_entry(entry);
        }
        ConflictChoice::Theirs => {
            let e = e3.ok_or_else(|| {
                TuiError::Terminal(format!("No stage 3 (theirs) entry for {}", rel_path))
            })?;
            let blob_data = match store
                .read_object(&e.oid)
                .map_err(|err| TuiError::Terminal(err.to_string()))?
            {
                Object::Blob(b) => b.data,
                _ => return Err(TuiError::Terminal("Not a blob".to_string())),
            };
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&full_path, &blob_data)?;
            let meta = fs::metadata(&full_path)?;
            let mut entry = IndexEntry::new(rel_path.to_string(), e.oid, e.mode);
            entry.file_size = meta.len() as u32;
            if let Ok(mtime) = meta.modified().and_then(|t| {
                t.duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| std::io::Error::other("err"))
            }) {
                entry.mtime_sec = mtime.as_secs() as u32;
            }
            entry.stage = 0;
            index.add_entry(entry);
        }
        ConflictChoice::Both => {
            let d2 = if let Some(e) = &e2 {
                match store
                    .read_object(&e.oid)
                    .map_err(|err| TuiError::Terminal(err.to_string()))?
                {
                    Object::Blob(b) => b.data,
                    _ => Vec::new(),
                }
            } else {
                Vec::new()
            };
            let d3 = if let Some(e) = &e3 {
                match store
                    .read_object(&e.oid)
                    .map_err(|err| TuiError::Terminal(err.to_string()))?
                {
                    Object::Blob(b) => b.data,
                    _ => Vec::new(),
                }
            } else {
                Vec::new()
            };
            let mut combined = d2;
            if !combined.is_empty() && !combined.ends_with(b"\n") {
                combined.push(b'\n');
            }
            combined.extend_from_slice(&d3);

            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&full_path, &combined)?;

            let new_blob_oid = loose_store
                .write_object(&Object::Blob(Blob::new(combined.clone())))
                .map_err(|e| TuiError::Terminal(e.to_string()))?;

            let mode = e2.or(e3).map(|e| e.mode).unwrap_or(0o100644);
            let mut entry = IndexEntry::new(rel_path.to_string(), new_blob_oid, mode);
            entry.file_size = combined.len() as u32;
            entry.stage = 0;
            index.add_entry(entry);
        }
    }

    index
        .write_to(&index_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    Ok(())
}

/// Creates a revert commit for `target_oid`.
pub fn revert_commit(
    repo_root: &Path,
    git_dir: &Path,
    common_dir: &Path,
    target_oid: &ObjectId,
) -> Result<ObjectId, TuiError> {
    let store = RepoObjectStore::open_with_common_dir(git_dir, common_dir)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let loose_store = LooseObjectStore::new(common_dir.join("objects"));
    let ref_store = RefStore::with_common_dir(git_dir, common_dir);
    let index_path = git_dir.join("index");

    let (active_branch, head_oid_opt) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let head_oid =
        head_oid_opt.ok_or_else(|| TuiError::Terminal("HEAD has no commits".to_string()))?;

    let target_commit = match store
        .read_object(target_oid)
        .map_err(|e| TuiError::Terminal(e.to_string()))?
    {
        Object::Commit(c) => c,
        _ => return Err(TuiError::Terminal("Target is not a commit".to_string())),
    };

    let head_commit = match store
        .read_object(&head_oid)
        .map_err(|e| TuiError::Terminal(e.to_string()))?
    {
        Object::Commit(c) => c,
        _ => return Err(TuiError::Terminal("HEAD is not a commit".to_string())),
    };

    let parent_tree = if !target_commit.parents.is_empty() {
        let p = match store
            .read_object(&target_commit.parents[0])
            .map_err(|e| TuiError::Terminal(e.to_string()))?
        {
            Object::Commit(c) => c,
            _ => return Err(TuiError::Terminal("Parent is not a commit".to_string())),
        };
        p.tree
    } else {
        ObjectId::ZERO
    };

    let mut index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;

    let conflicts = merge_trees_into_index_and_worktree(
        repo_root,
        git_dir,
        common_dir,
        &mut index,
        &target_commit.tree,
        &head_commit.tree,
        &parent_tree,
        "HEAD",
        &format!("parent of {}", &target_oid.to_string()[..7]),
    )?;

    index
        .write_to(&index_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    if !conflicts.is_empty() {
        return Err(TuiError::Terminal(format!(
            "Revert of {} resulted in conflicts in: {}",
            &target_oid.to_string()[..7],
            conflicts.join(", ")
        )));
    }

    let new_tree_oid =
        write_tree(&index, store.loose()).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let sig = get_signature(git_dir);
    let first_line = target_commit.message.lines().next().unwrap_or("");
    let revert_msg = format!(
        "Revert \"{}\"\n\nThis reverts commit {}.\n",
        first_line, target_oid
    );

    let new_commit = Commit {
        tree: new_tree_oid,
        parents: vec![head_oid],
        author: sig.clone(),
        committer: sig,
        gpg_sig: None,
        message: revert_msg,
    };
    let new_commit_oid = loose_store
        .write_object(&Object::Commit(new_commit))
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let ref_msg = format!("revert: {}", first_line);
    if active_branch == "HEAD" || active_branch.is_empty() {
        ref_store
            .set_head_detached(&new_commit_oid)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
    } else {
        let branch_ref = format!("refs/heads/{}", active_branch);
        ref_store
            .update_ref(&branch_ref, &new_commit_oid, Some(&head_oid), &ref_msg)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
    }

    Ok(new_commit_oid)
}

/// Lists all stashes for the repository by inspecting `.git/logs/refs/stash` and `.git/refs/stash`.
pub fn list_stashes(git_dir: &Path) -> Result<Vec<StashItem>, TuiError> {
    let common_dir = resolve_common_dir(git_dir);
    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let mut stashes = Vec::new();

    let stash_log = {
        let common_log = common_dir.join("logs").join("refs").join("stash");
        if common_log.exists() {
            common_log
        } else {
            git_dir.join("logs").join("refs").join("stash")
        }
    };

    if stash_log.exists() {
        if let Ok(content) = fs::read_to_string(&stash_log) {
            let lines: Vec<&str> = content.lines().collect();
            for (idx, line) in lines.iter().rev().enumerate() {
                let parts: Vec<&str> = line.split('\t').collect();
                let msg = parts.get(1).unwrap_or(&"WIP on stash").to_string();
                let mut oid = ObjectId::ZERO;
                if let Some(left) = parts.first() {
                    let tokens: Vec<&str> = left.split_whitespace().collect();
                    if let Some(target_sha) = tokens.get(1) {
                        if let Ok(parsed) = target_sha.parse::<ObjectId>() {
                            oid = parsed;
                        }
                    }
                }
                stashes.push(StashItem {
                    index: idx,
                    oid,
                    message: msg,
                    date: String::new(),
                });
            }
        }
    } else {
        let stash_ref = {
            let common_ref = common_dir.join("refs").join("stash");
            if common_ref.exists() {
                common_ref
            } else {
                git_dir.join("refs").join("stash")
            }
        };
        if stash_ref.exists() {
            if let Ok(content) = fs::read_to_string(&stash_ref) {
                if let Ok(oid) = content.trim().parse::<ObjectId>() {
                    let summary = if let Ok(Object::Commit(c)) = store.read_object(&oid) {
                        c.message.lines().next().unwrap_or("").to_string()
                    } else {
                        "WIP on branch".to_string()
                    };
                    stashes.push(StashItem {
                        index: 0,
                        oid,
                        message: summary,
                        date: String::new(),
                    });
                }
            }
        }
    }

    Ok(stashes)
}

/// Creates and checks out a new branch starting from the commit at which the stash was created,
/// applies the stash, and drops the stash if application succeeded cleanly.
pub fn stash_branch(
    repo_root: &Path,
    git_dir: &Path,
    stash_index: usize,
    branch_name: &str,
) -> Result<(), TuiError> {
    let ref_store = RefStore::new(git_dir);
    let stashes = list_stashes(git_dir)?;
    let stash_item = stashes
        .get(stash_index)
        .ok_or_else(|| TuiError::Terminal(format!("stash entry @{{{}}} not found", stash_index)))?;

    let stash_oid = stash_item.oid;
    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let stash_commit = match store.read_object(&stash_oid) {
        Ok(Object::Commit(c)) => c,
        _ => return Err(TuiError::Terminal("stash is not a commit".to_string())),
    };

    let base_oid = stash_commit
        .parents
        .first()
        .ok_or_else(|| TuiError::Terminal("stash commit has no parents".to_string()))?;

    // Create branch at base_oid
    ref_store
        .create_branch(branch_name, base_oid)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    // Checkout the new branch
    checkout_branch(repo_root, git_dir, branch_name)?;

    // Apply stash
    let clean = apply_stash(repo_root, git_dir, &stash_oid)?;
    if clean {
        drop_stash(git_dir, stash_index)?;
    }

    Ok(())
}

/// Applies all hunks in the custom patch basket to the working tree.
pub fn apply_custom_patch_to_worktree(
    repo_root: &Path,
    basket: &CustomPatchBasket,
    reverse: bool,
) -> Result<(), TuiError> {
    if basket.is_empty() {
        return Err(TuiError::Terminal(
            "Custom patch basket is empty".to_string(),
        ));
    }
    let mut planned_writes = Vec::new();
    for path in basket.paths() {
        let full = match oxidize_core::safe_join(repo_root, &path) {
            Ok(f) => f,
            Err(e) => return Err(TuiError::Terminal(e.to_string())),
        };
        let current_text = if full.is_file() {
            fs::read_to_string(&full)?
        } else {
            String::new()
        };
        let new_text = basket
            .apply_to_text(&path, &current_text, reverse)
            .map_err(TuiError::Terminal)?;
        planned_writes.push((full, new_text));
    }

    for (full, new_text) in planned_writes {
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&full, new_text.as_bytes())?;
    }
    Ok(())
}

/// Applies all hunks in the custom patch basket to the index.
pub fn apply_custom_patch_to_index(
    repo_root: &Path,
    git_dir: &Path,
    basket: &CustomPatchBasket,
    reverse: bool,
) -> Result<(), TuiError> {
    if basket.is_empty() {
        return Err(TuiError::Terminal(
            "Custom patch basket is empty".to_string(),
        ));
    }
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let common_dir = resolve_common_dir(git_dir);
    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let loose = LooseObjectStore::new(common_dir.join("objects"));

    // Phase 1: Preflight all paths and text applications before writing any blobs or modifying index
    let mut planned_updates = Vec::new();
    for path in basket.paths() {
        let full = match oxidize_core::safe_join(repo_root, &path) {
            Ok(f) => f,
            Err(e) => return Err(TuiError::Terminal(e.to_string())),
        };
        let current_text = if let Some(entry) = index.find_entry(&path) {
            read_blob_text(&store, &entry.oid)
        } else {
            String::new()
        };
        let new_text = basket
            .apply_to_text(&path, &current_text, reverse)
            .map_err(TuiError::Terminal)?;
        let mode = index.find_entry(&path).map(|e| e.mode).unwrap_or(0o100644);
        let meta = fs::metadata(&full).ok();
        planned_updates.push((path, new_text, mode, meta));
    }

    // Phase 2: Write blobs and update index entries
    for (path, new_text, mode, meta) in planned_updates {
        let new_bytes = new_text.into_bytes();
        let file_size = new_bytes.len() as u32;
        let blob = Object::Blob(Blob::new(new_bytes));
        let new_oid = loose.write_object(&blob)?;

        let entry = IndexEntry {
            ctime_sec: meta.as_ref().map(|m| m.len() as u32).unwrap_or(0),
            ctime_nsec: 0,
            mtime_sec: meta.as_ref().map(|m| m.len() as u32).unwrap_or(0),
            mtime_nsec: 0,
            dev: 0,
            ino: 0,
            mode,
            uid: 0,
            gid: 0,
            file_size,
            oid: new_oid,
            stage: 0,
            assume_valid: false,
            path,
        };
        index.add_entry(entry);
    }

    index
        .write_to(&index_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    Ok(())
}

/// Creates a new commit on top of HEAD containing the custom patch changes.
pub fn create_commit_from_custom_patch(
    repo_root: &Path,
    git_dir: &Path,
    basket: &CustomPatchBasket,
    message: &str,
) -> Result<ObjectId, TuiError> {
    apply_custom_patch_to_index(repo_root, git_dir, basket, false)?;
    apply_custom_patch_to_worktree(repo_root, basket, false)?;
    create_commit(repo_root, git_dir, message)
}

/// Applies a custom patch (forward or reverse) to a specific target commit.
pub fn apply_custom_patch_to_commit(
    repo_root: &Path,
    git_dir: &Path,
    basket: &CustomPatchBasket,
    target_oid: ObjectId,
    reverse: bool,
) -> Result<ObjectId, TuiError> {
    let ref_store = RefStore::new(git_dir);
    let (_, head_oid) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    if head_oid == Some(target_oid) {
        apply_custom_patch_to_index(repo_root, git_dir, basket, reverse)?;
        apply_custom_patch_to_worktree(repo_root, basket, reverse)?;
        let store =
            RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
        let commit = match store.read_object(&target_oid) {
            Ok(Object::Commit(c)) => c,
            _ => return Err(TuiError::Terminal("HEAD is not a commit".to_string())),
        };
        amend_commit(repo_root, git_dir, &commit.message)
    } else {
        Err(TuiError::Terminal(
            "Applying custom patch to historical non-HEAD commit is not yet supported; target commit must be HEAD".to_string(),
        ))
    }
}

/// Represents a linked or main worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeItem {
    /// Worktree identifier name.
    pub name: String,
    /// Absolute or relative path to worktree root.
    pub path: std::path::PathBuf,
    /// Branch or commit checked out in this worktree.
    pub head_ref: String,
    /// OID of currently checked out commit, if resolved.
    pub head_oid: Option<ObjectId>,
    /// Whether this is the main repository worktree.
    pub is_main: bool,
    /// Whether this worktree is locked against pruning/removal.
    pub is_locked: bool,
    /// Optional lock reason.
    pub lock_reason: Option<String>,
    /// Whether the worktree directory is missing from disk (prunable).
    pub is_prunable: bool,
}

/// Lists all worktrees (main and linked) for the repository.
pub fn list_worktrees(
    _git_dir: &Path,
    common_dir: &Path,
    main_worktree: Option<&Path>,
) -> Result<Vec<WorktreeItem>, TuiError> {
    let mut worktrees = Vec::new();

    // 1. Main worktree
    let main_path = main_worktree
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| common_dir.parent().unwrap_or(common_dir).to_path_buf());
    let main_ref_store = RefStore::new(common_dir);
    let (main_head_ref, main_head_oid) = main_ref_store
        .resolve_head()
        .unwrap_or_else(|_| ("HEAD".to_string(), None));

    worktrees.push(WorktreeItem {
        name: main_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "main".to_string()),
        path: main_path.clone(),
        head_ref: main_head_ref,
        head_oid: main_head_oid,
        is_main: true,
        is_locked: false,
        lock_reason: None,
        is_prunable: !main_path.exists(),
    });

    // 2. Linked worktrees under common_dir/worktrees/
    let wt_admin_root = common_dir.join("worktrees");
    if wt_admin_root.is_dir() {
        if let Ok(entries) = fs::read_dir(&wt_admin_root) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                if !entry_path.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();

                let gitdir_file = entry_path.join("gitdir");
                let wt_path = if let Ok(content) = fs::read_to_string(&gitdir_file) {
                    let p = std::path::PathBuf::from(content.trim());
                    if p.file_name().is_some_and(|f| f == ".git") {
                        p.parent().unwrap_or(&p).to_path_buf()
                    } else {
                        p
                    }
                } else {
                    continue;
                };

                let is_prunable = !wt_path.exists();

                let head_file = entry_path.join("HEAD");
                let mut head_ref = "HEAD".to_string();
                let mut head_oid = None;
                if let Ok(head_content) = fs::read_to_string(&head_file) {
                    let trimmed = head_content.trim();
                    if let Some(rest) = trimmed.strip_prefix("ref: ") {
                        let branch = rest.trim();
                        head_ref = branch
                            .strip_prefix("refs/heads/")
                            .unwrap_or(branch)
                            .to_string();
                        if let Ok(oid) = main_ref_store.read_ref(branch) {
                            head_oid = Some(oid);
                        }
                    } else if let Ok(oid) = trimmed.parse::<ObjectId>() {
                        head_ref = format!("{:.7}", oid);
                        head_oid = Some(oid);
                    }
                }

                let lock_file = entry_path.join("locked");
                let (is_locked, lock_reason) = if lock_file.is_file() {
                    let reason = fs::read_to_string(&lock_file)
                        .ok()
                        .map(|s| s.trim().to_string());
                    (true, reason)
                } else {
                    (false, None)
                };

                worktrees.push(WorktreeItem {
                    name,
                    path: wt_path,
                    head_ref,
                    head_oid,
                    is_main: false,
                    is_locked,
                    lock_reason,
                    is_prunable,
                });
            }
        }
    }

    Ok(worktrees)
}

/// Creates a new linked worktree at `new_wt_path` with the specified branch.
pub fn create_worktree(
    repo_root: &Path,
    git_dir: &Path,
    common_dir: &Path,
    new_wt_path: &Path,
    branch_name: &str,
    create_branch: bool,
) -> Result<WorktreeItem, TuiError> {
    if new_wt_path.is_file() {
        return Err(TuiError::Terminal(format!(
            "'{}' already exists and is a file",
            new_wt_path.display()
        )));
    }
    if new_wt_path.is_dir() {
        let is_empty = fs::read_dir(new_wt_path)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if !is_empty {
            return Err(TuiError::Terminal(format!(
                "'{}' already exists and is not empty",
                new_wt_path.display()
            )));
        }
    }

    let existing_wts = list_worktrees(git_dir, common_dir, Some(repo_root))?;
    let target_ref_name = branch_name
        .strip_prefix("refs/heads/")
        .unwrap_or(branch_name);

    for wt in &existing_wts {
        if wt.head_ref == target_ref_name {
            return Err(TuiError::Terminal(format!(
                "fatal: '{}' is already checked out at '{}'",
                target_ref_name,
                wt.path.display()
            )));
        }
    }

    let ref_store = RefStore::new(common_dir);
    let store = RepoObjectStore::open(common_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;

    let branch_full_ref = format!("refs/heads/{}", target_ref_name);
    let commit_oid = if create_branch {
        let (_, head_oid) = ref_store
            .resolve_head()
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        let head_oid = head_oid.ok_or_else(|| {
            TuiError::Terminal("cannot create worktree: HEAD has no commits".to_string())
        })?;
        ref_store
            .create_branch(target_ref_name, &head_oid)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        head_oid
    } else {
        ref_store.read_ref(&branch_full_ref).map_err(|e| {
            TuiError::Terminal(format!("branch '{}' not found: {}", target_ref_name, e))
        })?
    };

    let commit = match store.read_object(&commit_oid) {
        Ok(Object::Commit(c)) => c,
        _ => return Err(TuiError::Terminal("target ref is not a commit".to_string())),
    };

    let wt_name = new_wt_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "wt".to_string());

    let wt_admin_dir = common_dir.join("worktrees").join(&wt_name);
    fs::create_dir_all(&wt_admin_dir)?;
    fs::write(wt_admin_dir.join("commondir"), "../..\n")?;

    let abs_wt = if new_wt_path.is_relative() {
        std::env::current_dir()
            .map(|d| d.join(new_wt_path))
            .unwrap_or_else(|_| new_wt_path.to_path_buf())
    } else {
        new_wt_path.to_path_buf()
    };
    let wt_git_file_target = abs_wt.join(".git");
    fs::write(
        wt_admin_dir.join("gitdir"),
        format!("{}\n", wt_git_file_target.display()),
    )?;
    fs::write(
        wt_admin_dir.join("HEAD"),
        format!("ref: refs/heads/{}\n", target_ref_name),
    )?;

    // Create target working tree directory and .git gitfile
    fs::create_dir_all(&abs_wt)?;
    fs::write(
        &wt_git_file_target,
        format!("gitdir: {}\n", wt_admin_dir.display()),
    )?;

    // Populate worktree files and index from commit tree
    let tree_files =
        flatten_tree(&store, &commit.tree, "").map_err(|e| TuiError::Terminal(e.to_string()))?;
    let mut wt_index = Index::new();

    for (rel_path, (mode, blob_oid)) in tree_files {
        let full = abs_wt.join(&rel_path);
        let (_, data) = store
            .read_raw(&blob_oid)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&full, &data)?;
        let meta = fs::metadata(&full)?;
        let entry = IndexEntry {
            ctime_sec: 0,
            ctime_nsec: 0,
            mtime_sec: 0,
            mtime_nsec: 0,
            dev: 0,
            ino: 0,
            mode: mode.0,
            uid: 0,
            gid: 0,
            file_size: meta.len() as u32,
            oid: blob_oid,
            stage: 0,
            assume_valid: false,
            path: rel_path,
        };
        wt_index.add_entry(entry);
    }

    wt_index
        .write_to(wt_admin_dir.join("index"))
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    Ok(WorktreeItem {
        name: wt_name,
        path: abs_wt,
        head_ref: target_ref_name.to_string(),
        head_oid: Some(commit_oid),
        is_main: false,
        is_locked: false,
        lock_reason: None,
        is_prunable: false,
    })
}

/// Removes a linked worktree from disk and clears its `.git/worktrees/<name>` administrative metadata.
pub fn remove_worktree(common_dir: &Path, wt_name: &str, force: bool) -> Result<(), TuiError> {
    let wt_admin_dir = common_dir.join("worktrees").join(wt_name);
    if !wt_admin_dir.is_dir() {
        return Err(TuiError::Terminal(format!(
            "worktree metadata for '{}' not found",
            wt_name
        )));
    }

    let lock_file = wt_admin_dir.join("locked");
    if lock_file.exists() {
        if !lock_file.is_file() {
            return Err(TuiError::Terminal(format!(
                "worktree '{}' has malformed lock metadata",
                wt_name
            )));
        }
        let reason = fs::read_to_string(&lock_file)?;
        if !force {
            return Err(TuiError::Terminal(format!(
                "worktree '{}' is locked: {}",
                wt_name,
                reason.trim()
            )));
        }
    }

    let gitdir_path = wt_admin_dir.join("gitdir");
    if !gitdir_path.is_file() {
        return Err(TuiError::Terminal(format!(
            "worktree '{}' is missing required gitdir metadata",
            wt_name
        )));
    }
    let content = fs::read_to_string(&gitdir_path)
        .map_err(|e| TuiError::Terminal(format!("cannot read worktree gitdir: {}", e)))?;
    let raw_git_path = content.trim();
    if raw_git_path.is_empty() {
        return Err(TuiError::Terminal(format!(
            "worktree '{}' has empty gitdir metadata",
            wt_name
        )));
    }
    let git_link = std::path::PathBuf::from(raw_git_path);
    if git_link.file_name().is_none_or(|name| name != ".git") {
        return Err(TuiError::Terminal(format!(
            "worktree '{}' gitdir metadata does not point to a .git file",
            wt_name
        )));
    }
    let wt_path = git_link
        .parent()
        .ok_or_else(|| TuiError::Terminal("worktree gitdir has no parent".to_string()))?
        .to_path_buf();
    if !wt_path.is_dir() {
        return Err(TuiError::Terminal(format!(
            "worktree directory '{}' is missing or unreadable",
            wt_path.display()
        )));
    }

    let main_repo_root = if common_dir.file_name().is_some_and(|name| name == ".git") {
        common_dir
            .parent()
            .ok_or_else(|| TuiError::Terminal("common git directory has no parent".to_string()))?
    } else {
        common_dir
    };
    let can_wt = wt_path.canonicalize()?;
    let can_main = main_repo_root.canonicalize()?;
    if can_wt == can_main {
        return Err(TuiError::Terminal(
            "cannot remove main worktree".to_string(),
        ));
    }
    let can_cwd = std::env::current_dir()?.canonicalize()?;
    if can_wt == can_cwd {
        return Err(TuiError::Terminal(
            "cannot remove current working directory worktree".to_string(),
        ));
    }

    let wt_dotgit = wt_path.join(".git");
    if !wt_dotgit.is_file() {
        return Err(TuiError::Terminal(format!(
            "worktree '{}' is missing reciprocal .git link",
            wt_name
        )));
    }
    let dotgit_content = fs::read_to_string(&wt_dotgit)?;
    let target = dotgit_content.strip_prefix("gitdir:").ok_or_else(|| {
        TuiError::Terminal(format!(
            "worktree '{}' has malformed reciprocal .git link",
            wt_name
        ))
    })?;
    let target_path = std::path::PathBuf::from(target.trim());
    if target_path.as_os_str().is_empty() {
        return Err(TuiError::Terminal(format!(
            "worktree '{}' has empty reciprocal .git link",
            wt_name
        )));
    }
    let resolved_target = if target_path.is_relative() {
        wt_path.join(target_path)
    } else {
        target_path
    };
    if resolved_target.canonicalize()? != wt_admin_dir.canonicalize()? {
        return Err(TuiError::Terminal(format!(
            "worktree '{}' has mismatched reciprocal administrative path",
            wt_name
        )));
    }

    let wt_index_path = wt_admin_dir.join("index");
    if !wt_index_path.is_file() {
        return Err(TuiError::Terminal(format!(
            "worktree '{}' is missing required index",
            wt_name
        )));
    }
    let wt_index = Index::load_from(&wt_index_path)
        .map_err(|e| TuiError::Terminal(format!("cannot read worktree index: {}", e)))?;
    let head_path = wt_admin_dir.join("HEAD");
    if !head_path.is_file() {
        return Err(TuiError::Terminal(format!(
            "worktree '{}' is missing required HEAD metadata",
            wt_name
        )));
    }
    let wt_ref_store = RefStore::with_common_dir(&wt_admin_dir, common_dir);
    let (_, wt_head_oid_opt) = wt_ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(format!("cannot resolve worktree HEAD: {}", e)))?;
    let wt_store =
        RepoObjectStore::open(common_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let wt_head_tree = match wt_head_oid_opt {
        Some(oid) => match wt_store
            .read_object(&oid)
            .map_err(|e| TuiError::Terminal(format!("cannot read worktree HEAD commit: {}", e)))?
        {
            Object::Commit(commit) => Some(commit.tree),
            _ => {
                return Err(TuiError::Terminal(
                    "worktree HEAD does not point to a commit".to_string(),
                ))
            }
        },
        None => None,
    };

    if !force {
        let gitignore = oxidize_config::GitIgnore::load_from_dir(&wt_path)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        let status = oxidize_index::compute_status_with_ignore(
            &wt_path,
            &wt_index,
            wt_head_tree.as_ref(),
            &wt_store,
            Some(&|path, is_dir| gitignore.is_ignored(path, is_dir)),
        )
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
        if !status.staged.is_empty() || !status.unstaged.is_empty() || !status.untracked.is_empty()
        {
            return Err(TuiError::Terminal(format!(
                "cannot remove worktree '{}': worktree contains modified or untracked files (use force to delete)",
                wt_name
            )));
        }
    }

    fs::remove_dir_all(&wt_path).map_err(|e| {
        TuiError::Terminal(format!(
            "failed to remove worktree directory '{}': {}",
            wt_path.display(),
            e
        ))
    })?;
    fs::remove_dir_all(&wt_admin_dir).map_err(|e| {
        TuiError::Terminal(format!(
            "failed to remove worktree metadata '{}': {}",
            wt_admin_dir.display(),
            e
        ))
    })?;
    Ok(())
}

/// Represents a Git submodule defined in `.gitmodules`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmoduleItem {
    pub name: String,
    pub path: String,
    pub url: String,
    pub gitlink_oid: Option<ObjectId>,
    pub head_oid: Option<ObjectId>,
    pub is_initialized: bool,
    pub is_dirty: bool,
}

/// Lists all submodules defined in `.gitmodules` and checks their status.
pub fn list_submodules(repo_root: &Path, git_dir: &Path) -> Result<Vec<SubmoduleItem>, TuiError> {
    let gitmodules_path = repo_root.join(".gitmodules");
    if !gitmodules_path.is_file() {
        return Ok(Vec::new());
    }

    let config = GitConfig::load_from_file(&gitmodules_path).unwrap_or_default();
    let index_path = git_dir.join("index");
    let index = Index::load_from(&index_path).ok();

    let mut submodules = Vec::new();
    let names = config.subsections("submodule");

    for name in names {
        let path = config
            .get("submodule", Some(&name), "path")
            .unwrap_or(&name)
            .to_string();
        let url = config
            .get("submodule", Some(&name), "url")
            .unwrap_or("")
            .to_string();

        let norm_path = path.replace('\\', "/");
        let gitlink_oid = index.as_ref().and_then(|idx| {
            idx.entries
                .iter()
                .find(|e| e.path == norm_path && e.mode == 0o160000)
                .map(|e| e.oid)
        });

        let sub_root = repo_root.join(&norm_path);
        let sub_git = sub_root.join(".git");
        let repo_config = GitConfig::load_from_file(git_dir.join("config")).unwrap_or_default();
        let is_configured = repo_config.get("submodule", Some(&name), "url").is_some();
        let (is_initialized, head_oid, is_dirty) = if sub_git.exists() {
            let actual_git_dir = if sub_git.is_file() {
                if let Ok(content) = fs::read_to_string(&sub_git) {
                    let trimmed = content.trim();
                    if let Some(target) = trimmed.strip_prefix("gitdir:") {
                        let rel = target.trim();
                        let p = sub_root.join(rel);
                        p.canonicalize().unwrap_or(p)
                    } else {
                        sub_git.clone()
                    }
                } else {
                    sub_git.clone()
                }
            } else {
                sub_git.clone()
            };

            let sub_ref_store = RefStore::new(&actual_git_dir);
            let sub_head_oid = sub_ref_store.resolve_head().ok().and_then(|(_, oid)| oid);
            let dirty = if let (Some(g_oid), Some(h_oid)) = (gitlink_oid, sub_head_oid) {
                g_oid != h_oid
            } else {
                false
            };

            (true, sub_head_oid, dirty)
        } else {
            (is_configured, None, false)
        };

        submodules.push(SubmoduleItem {
            name,
            path,
            url,
            gitlink_oid,
            head_oid,
            is_initialized,
            is_dirty,
        });
    }

    Ok(submodules)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), TuiError> {
    fs::create_dir_all(dst).map_err(|e| TuiError::Terminal(e.to_string()))?;
    for entry in fs::read_dir(src).map_err(|e| TuiError::Terminal(e.to_string()))? {
        let entry = entry.map_err(|e| TuiError::Terminal(e.to_string()))?;
        let ft = entry
            .file_type()
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        let target = dst.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else if ft.is_file() {
            fs::copy(entry.path(), target).map_err(|e| TuiError::Terminal(e.to_string()))?;
        }
    }
    Ok(())
}

/// Initializes a submodule by writing its configuration into `.git/config`.
pub fn submodule_init(
    repo_root: &Path,
    git_dir: &Path,
    submodule_name: &str,
) -> Result<(), TuiError> {
    let gitmodules_path = repo_root.join(".gitmodules");
    if !gitmodules_path.is_file() {
        return Err(TuiError::Terminal("No .gitmodules file found".to_string()));
    }
    let modules_cfg = GitConfig::load_from_file(&gitmodules_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let url = modules_cfg
        .get("submodule", Some(submodule_name), "url")
        .ok_or_else(|| {
            TuiError::Terminal(format!(
                "Submodule '{}' not found in .gitmodules",
                submodule_name
            ))
        })?;

    let config_path = git_dir.join("config");
    let mut config =
        GitConfig::load_from_file(&config_path).map_err(|e| TuiError::Terminal(e.to_string()))?;
    config.set("submodule", Some(submodule_name), "url", url);
    config
        .save_to_file(&config_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))
}

/// Updates a submodule by checking out the commit specified in the parent gitlink.
pub fn submodule_update(
    repo_root: &Path,
    git_dir: &Path,
    submodule_name: &str,
) -> Result<(), TuiError> {
    let submodules = list_submodules(repo_root, git_dir)?;
    let sub = submodules
        .iter()
        .find(|s| s.name == submodule_name)
        .ok_or_else(|| TuiError::Terminal(format!("Submodule '{}' not found", submodule_name)))?;

    if !sub.is_initialized {
        return Err(TuiError::Terminal(format!(
            "Submodule '{}' is not initialized. Run init first.",
            submodule_name
        )));
    }

    let sub_root = repo_root.join(&sub.path);
    let sub_git = sub_root.join(".git");

    if !sub_git.exists() {
        let url_path = Path::new(&sub.url);
        if url_path.exists() {
            let src_git = if url_path.join(".git").exists() {
                url_path.join(".git")
            } else {
                url_path.to_path_buf()
            };
            copy_dir_recursive(&src_git, &sub_git)?;
        } else {
            fs::create_dir_all(&sub_git).map_err(|e| TuiError::Terminal(e.to_string()))?;
        }
    }

    let actual_git_dir = if sub_git.is_file() {
        if let Ok(content) = fs::read_to_string(&sub_git) {
            let trimmed = content.trim();
            if let Some(target) = trimmed.strip_prefix("gitdir:") {
                let rel = target.trim();
                let p = sub_root.join(rel);
                p.canonicalize().unwrap_or(p)
            } else {
                sub_git.clone()
            }
        } else {
            sub_git.clone()
        }
    } else {
        sub_git.clone()
    };

    let target_oid = match sub.gitlink_oid {
        Some(oid) => oid,
        None => {
            let sub_ref_store = RefStore::new(&actual_git_dir);
            let (_, head) = sub_ref_store
                .resolve_head()
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            head.ok_or_else(|| {
                TuiError::Terminal(format!("Submodule '{}' has empty HEAD", submodule_name))
            })?
        }
    };

    let sub_store =
        RepoObjectStore::open(&actual_git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let commit = match sub_store.read_object(&target_oid) {
        Ok(Object::Commit(c)) => c,
        _ => {
            return Err(TuiError::Terminal(format!(
                "Gitlink commit {} not found in submodule objects",
                target_oid
            )))
        }
    };

    // Preflight: protect dirty submodule worktree and uncommitted changes
    let sub_index_path = actual_git_dir.join("index");
    if sub_index_path.is_file() {
        if let Ok(sub_index) = Index::load_from(&sub_index_path) {
            let has_any_worktree_files = if let Ok(entries) = fs::read_dir(&sub_root) {
                entries
                    .filter_map(|e| e.ok())
                    .any(|e| e.file_name() != ".git")
            } else {
                false
            };

            if has_any_worktree_files {
                let sub_ref_store = RefStore::new(&actual_git_dir);
                if let Ok((_, Some(sub_head))) = sub_ref_store.resolve_head() {
                    if let Ok(Object::Commit(c)) = sub_store.read_object(&sub_head) {
                        if let Ok(sub_status) = oxidize_index::compute_status(
                            &sub_root,
                            &sub_index,
                            Some(&c.tree),
                            &sub_store,
                        ) {
                            if !sub_status.staged.is_empty() || !sub_status.unstaged.is_empty() {
                                return Err(TuiError::Terminal(format!(
                                    "Submodule '{}' contains uncommitted changes. Please commit or stash them before updating.",
                                    submodule_name
                                )));
                            }
                        }
                    }
                }
            }

            // Check untracked collisions against incoming commit tree
            let target_map = flatten_tree(&sub_store, &commit.tree, "")
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
            let mut collisions = Vec::new();
            for path in target_map.keys() {
                let full = match oxidize_core::safe_join(&sub_root, path) {
                    Ok(p) => p,
                    Err(e) => return Err(TuiError::Terminal(e.to_string())),
                };
                if full.is_file() && sub_index.find_entry(path).is_none() {
                    collisions.push(path.clone());
                }
            }
            if !collisions.is_empty() {
                return Err(TuiError::Terminal(format!(
                    "error: The following untracked working tree files in submodule '{}' would be overwritten by checkout:\n\t{}\nPlease move or remove them before updating.",
                    submodule_name,
                    collisions.join("\n\t")
                )));
            }
        }
    }

    checkout_tree_and_update_index(&sub_root, &actual_git_dir, &commit.tree, true)?;
    let sub_ref_store = RefStore::new(&actual_git_dir);
    sub_ref_store
        .update_ref(
            "HEAD",
            &target_oid,
            None,
            "submodule update: checkout gitlink",
        )
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    Ok(())
}

/// Current state of an active or inactive Git bisect session.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BisectState {
    pub is_active: bool,
    pub bad_oid: Option<ObjectId>,
    pub good_oids: Vec<ObjectId>,
    pub current_oid: Option<ObjectId>,
    pub orig_branch: Option<String>,
    pub remaining_steps: usize,
    pub total_revisions: usize,
    pub culprit_oid: Option<ObjectId>,
}

/// Reads the current bisect state from `.git/BISECT_*` files and computes progress.
pub fn get_bisect_state(git_dir: &Path) -> Result<BisectState, TuiError> {
    let start_file = git_dir.join("BISECT_START");
    if !start_file.is_file() {
        return Ok(BisectState::default());
    }

    let orig_branch = fs::read_to_string(&start_file)
        .ok()
        .map(|s| s.trim().to_string());
    let bad_oid: Option<ObjectId> = fs::read_to_string(git_dir.join("BISECT_BAD"))
        .ok()
        .and_then(|s| s.trim().parse().ok());

    let mut good_oids = Vec::new();
    if let Ok(content) = fs::read_to_string(git_dir.join("BISECT_GOOD")) {
        for line in content.lines() {
            if let Ok(oid) = line.trim().parse::<ObjectId>() {
                good_oids.push(oid);
            }
        }
    }

    let ref_store = RefStore::new(git_dir);
    let (_, current_oid) = ref_store
        .resolve_head()
        .unwrap_or_else(|_| ("HEAD".to_string(), None));

    if bad_oid.is_none() || good_oids.is_empty() {
        return Ok(BisectState {
            is_active: true,
            bad_oid,
            good_oids,
            current_oid,
            orig_branch,
            remaining_steps: 0,
            total_revisions: 0,
            culprit_oid: None,
        });
    }

    let store = match RepoObjectStore::open(git_dir) {
        Ok(s) => s,
        Err(_) => {
            return Ok(BisectState {
                is_active: true,
                bad_oid,
                good_oids,
                current_oid,
                orig_branch,
                remaining_steps: 0,
                total_revisions: 0,
                culprit_oid: None,
            })
        }
    };

    let bad = bad_oid.unwrap();
    let candidates = compute_bisect_candidates(&store, bad, &good_oids);
    let total_revisions = candidates.len();
    let remaining_steps = if total_revisions > 1 {
        (total_revisions as f64).log2().ceil() as usize
    } else {
        0
    };
    let culprit_oid = if total_revisions <= 1 {
        candidates.first().copied().or(bad_oid)
    } else {
        None
    };

    Ok(BisectState {
        is_active: true,
        bad_oid,
        good_oids,
        current_oid,
        orig_branch,
        remaining_steps,
        total_revisions,
        culprit_oid,
    })
}

fn compute_bisect_candidates(
    store: &RepoObjectStore,
    bad_oid: ObjectId,
    good_oids: &[ObjectId],
) -> Vec<ObjectId> {
    let mut good_ancestors = HashSet::new();
    let mut q = VecDeque::new();
    for &g in good_oids {
        good_ancestors.insert(g);
        q.push_back(g);
    }
    while let Some(c) = q.pop_front() {
        if let Ok(Object::Commit(commit)) = store.read_object(&c) {
            for p in commit.parents {
                if good_ancestors.insert(p) {
                    q.push_back(p);
                }
            }
        }
    }

    let mut candidates = Vec::new();
    let mut visited = HashSet::new();
    let mut q2 = VecDeque::new();
    visited.insert(bad_oid);
    q2.push_back(bad_oid);

    while let Some(c) = q2.pop_front() {
        if !good_ancestors.contains(&c) {
            candidates.push(c);
            if let Ok(Object::Commit(commit)) = store.read_object(&c) {
                for p in commit.parents {
                    if visited.insert(p) {
                        q2.push_back(p);
                    }
                }
            }
        }
    }

    candidates
}

/// Starts a new bisect session.
pub fn bisect_start(
    repo_root: &Path,
    git_dir: &Path,
    bad_opt: Option<ObjectId>,
    good_opt: Option<ObjectId>,
) -> Result<BisectState, TuiError> {
    let ref_store = RefStore::new(git_dir);
    let (head_branch, head_oid) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let orig_name = if !head_branch.is_empty() && head_branch != "HEAD" {
        head_branch
    } else if let Some(oid) = head_oid {
        oid.to_string()
    } else {
        return Err(TuiError::Terminal(
            "Cannot bisect an unborn repository".to_string(),
        ));
    };

    fs::write(git_dir.join("BISECT_START"), format!("{}\n", orig_name))?;

    let bad_oid = match bad_opt.or(head_oid) {
        Some(b) => b,
        None => return Err(TuiError::Terminal("No bad commit specified".to_string())),
    };
    fs::write(git_dir.join("BISECT_BAD"), format!("{}\n", bad_oid))?;

    if let Some(good) = good_opt {
        fs::write(git_dir.join("BISECT_GOOD"), format!("{}\n", good))?;
    }

    // If both bad and good are set, step bisect immediately
    if let Some(good) = good_opt {
        let store =
            RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
        let candidates = compute_bisect_candidates(&store, bad_oid, &[good]);
        if candidates.len() > 1 {
            let mid = candidates[candidates.len() / 2];
            let commit = match store.read_object(&mid) {
                Ok(Object::Commit(c)) => c,
                _ => return Err(TuiError::Terminal("Candidate is not a commit".to_string())),
            };
            checkout_tree_and_update_index(repo_root, git_dir, &commit.tree, false)?;
            ref_store
                .update_ref("HEAD", &mid, None, "bisect: checkout midpoint")
                .map_err(|e| TuiError::Terminal(e.to_string()))?;
        }
    }

    get_bisect_state(git_dir)
}

/// Marks the current commit as good or bad and advances to the next midpoint candidate.
pub fn bisect_mark(
    repo_root: &Path,
    git_dir: &Path,
    is_bad: bool,
) -> Result<BisectState, TuiError> {
    let start_file = git_dir.join("BISECT_START");
    if !start_file.is_file() {
        return Err(TuiError::Terminal(
            "We are not bisecting. Run bisect start first.".to_string(),
        ));
    }

    let ref_store = RefStore::new(git_dir);
    let (_, head_oid) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let current_oid = head_oid
        .ok_or_else(|| TuiError::Terminal("HEAD is not pointing to a valid commit".to_string()))?;

    if is_bad {
        fs::write(git_dir.join("BISECT_BAD"), format!("{}\n", current_oid))?;
    } else {
        let good_file = git_dir.join("BISECT_GOOD");
        let mut existing = match fs::read_to_string(&good_file) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error.into()),
        };
        existing.push_str(&format!("{}\n", current_oid));
        fs::write(&good_file, existing)?;
    }

    let bad_oid: ObjectId = fs::read_to_string(git_dir.join("BISECT_BAD"))
        .map_err(|_| TuiError::Terminal("Missing BISECT_BAD".to_string()))?
        .trim()
        .parse()
        .map_err(|_| TuiError::Terminal("Invalid BISECT_BAD OID".to_string()))?;

    let good_file = git_dir.join("BISECT_GOOD");
    let mut good_oids = Vec::new();
    if let Ok(content) = fs::read_to_string(&good_file) {
        for line in content.lines() {
            if let Ok(oid) = line.trim().parse::<ObjectId>() {
                good_oids.push(oid);
            }
        }
    }

    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let candidates = compute_bisect_candidates(&store, bad_oid, &good_oids);

    if candidates.len() > 1 {
        let mid = candidates[candidates.len() / 2];
        let commit = match store.read_object(&mid) {
            Ok(Object::Commit(c)) => c,
            _ => return Err(TuiError::Terminal("Candidate is not a commit".to_string())),
        };
        checkout_tree_and_update_index(repo_root, git_dir, &commit.tree, false)?;
        ref_store
            .update_ref("HEAD", &mid, None, "bisect: checkout midpoint")
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
    }

    get_bisect_state(git_dir)
}

/// Skips the current commit and checks out another candidate.
pub fn bisect_skip(repo_root: &Path, git_dir: &Path) -> Result<BisectState, TuiError> {
    let state = get_bisect_state(git_dir)?;
    if !state.is_active || state.bad_oid.is_none() {
        return Err(TuiError::Terminal("We are not bisecting".to_string()));
    }
    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let candidates = compute_bisect_candidates(&store, state.bad_oid.unwrap(), &state.good_oids);
    if candidates.len() > 2 {
        let alt_mid = candidates[candidates.len() / 3];
        let commit = match store.read_object(&alt_mid) {
            Ok(Object::Commit(c)) => c,
            _ => return Err(TuiError::Terminal("Candidate is not a commit".to_string())),
        };
        checkout_tree_and_update_index(repo_root, git_dir, &commit.tree, false)?;
        let ref_store = RefStore::new(git_dir);
        ref_store
            .update_ref("HEAD", &alt_mid, None, "bisect: skip commit")
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
    }
    get_bisect_state(git_dir)
}

/// Resets the bisect session and restores the original branch or commit.
pub fn bisect_reset(repo_root: &Path, git_dir: &Path) -> Result<(), TuiError> {
    let start_file = git_dir.join("BISECT_START");
    if !start_file.is_file() {
        return Ok(());
    }
    let orig = fs::read_to_string(&start_file)
        .map_err(|e| TuiError::Terminal(e.to_string()))?
        .trim()
        .to_string();

    if !orig.is_empty() {
        checkout_branch(repo_root, git_dir, &orig)?;
    }

    remove_file_if_exists(&start_file)?;
    remove_file_if_exists(&git_dir.join("BISECT_BAD"))?;
    remove_file_if_exists(&git_dir.join("BISECT_GOOD"))?;
    remove_file_if_exists(&git_dir.join("BISECT_LOG"))?;
    remove_file_if_exists(&git_dir.join("BISECT_ANCESTORS_OK"))?;

    Ok(())
}

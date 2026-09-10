use crate::model::{ReflogItem, RemoteItem, TagItem};
use crate::TuiError;
use oxidize_config::GitConfig;
use oxidize_core::id::ObjectId;
use oxidize_core::object::{Blob, Commit, Object, Signature};
use oxidize_core::store::LooseObjectStore;
use oxidize_diff::three_way_merge;
use oxidize_index::{flatten_tree, write_tree, Index, IndexEntry};
use oxidize_pack::{PackIndex, RepoObjectStore};
use oxidize_refs::RefStore;
use oxidize_transport::{
    discover_local_refs, fetch_local_pack, is_ssh_url, resolve_local_path, SmartHttpClient,
    SshClient,
};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fs;
use std::path::Path;

/// Stages a single file (adds to index and loose object store, or removes from index if deleted in working tree).
pub fn stage_path(repo_root: &Path, git_dir: &Path, rel_path: &str) -> Result<(), TuiError> {
    let full_path = oxidize_core::safe_join(repo_root, rel_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;

    if full_path.exists() && full_path.is_file() {
        let data = fs::read(&full_path)?;
        let meta = fs::metadata(&full_path)?;

        let store = LooseObjectStore::new(git_dir.join("objects"));
        let blob = Object::Blob(Blob::new(data));
        let oid = store.write_object(&blob)?;

        let entry = IndexEntry::from_fs_metadata(rel_path.to_string(), oid, &meta, 0);
        index.add_entry(entry);
    } else if !full_path.exists() {
        // File was deleted in working tree -> stage deletion in index
        index.remove_entry(rel_path);
    }

    index
        .write_to(&index_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    Ok(())
}

/// Unstages a single file (restores previous HEAD state in index, or removes from index if new).
pub fn unstage_path(repo_root: &Path, git_dir: &Path, rel_path: &str) -> Result<(), TuiError> {
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;

    let ref_store = RefStore::new(git_dir);
    let head_oid_opt = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?
        .1;

    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;

    let head_map = head_oid_opt
        .and_then(|oid| {
            if let Ok(Object::Commit(c)) = store.read_object(&oid) {
                flatten_tree(&store, &c.tree, "").ok()
            } else {
                None
            }
        })
        .unwrap_or_default();

    if let Some((mode, head_oid)) = head_map.get(rel_path) {
        // File existed in HEAD -> restore index entry to HEAD OID
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
        // File was newly added and not in HEAD -> remove entry from index
        index.remove_entry(rel_path);
    }

    index
        .write_to(&index_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
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
        if let Ok(Object::Blob(blob)) = store.read_object(&entry.oid) {
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&full_path, &blob.data)?;
        }
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
    let store = LooseObjectStore::new(git_dir.join("objects"));
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
        if let Ok(Object::Commit(head_commit)) = store.read_object(head_oid) {
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
    let _ = fs::remove_file(git_dir.join("MERGE_HEAD"));
    let _ = fs::remove_file(git_dir.join("MERGE_MSG"));
    let _ = fs::remove_file(git_dir.join("MERGE_MODE"));

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
        Some(ref oid) => flatten_tree(&store, oid, "").unwrap_or_default(),
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
                if let Ok(Object::Blob(blob)) = store.read_object(oid) {
                    fs::write(&full_path, &blob.data)?;
                }
                if let Ok(meta) = fs::metadata(&full_path) {
                    let entry = IndexEntry::from_fs_metadata(path.clone(), *oid, &meta, 0);
                    index.add_entry(entry);
                }
            } else {
                // Removed in target
                if full_path.exists() {
                    let _ = fs::remove_file(&full_path);
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
        for entry in &index.entries {
            if !target_map.contains_key(&entry.path) {
                let full_path = match oxidize_core::safe_join(repo_root, &entry.path) {
                    Ok(p) => p,
                    Err(e) => return Err(TuiError::Terminal(e.to_string())),
                };
                if full_path.exists() {
                    let _ = fs::remove_file(&full_path);
                }
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

            if let Ok(Object::Blob(blob)) = store.read_object(&oid) {
                fs::write(&full_path, &blob.data)?;
            }

            if let Ok(meta) = fs::metadata(&full_path) {
                let entry = IndexEntry::from_fs_metadata(path, oid, &meta, 0);
                index.add_entry(entry);
            }
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
    let store = LooseObjectStore::new(git_dir.join("objects"));
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
                        let _ = fs::remove_file(&full_path);
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
    Ok(!has_conflicts)
}

/// Creates a new stash commit saving working directory changes and index state.
pub fn stash_save(repo_root: &Path, git_dir: &Path, message: &str) -> Result<ObjectId, TuiError> {
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

    if status.staged.is_empty() && status.unstaged.is_empty() {
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

    // 2. Worktree tree: apply all unstaged changes (both modified AND deleted) to work_index
    let mut work_index = index.clone();
    for change in &status.unstaged {
        match change {
            oxidize_index::UnstagedChange::Modified(p) => {
                let full = repo_root.join(p);
                if let Ok(data) = fs::read(&full) {
                    let blob = Object::Blob(Blob::new(data));
                    if let Ok(oid) = loose_store.write_object(&blob) {
                        if let Ok(meta) = fs::metadata(&full) {
                            work_index.add_entry(IndexEntry::from_fs_metadata(
                                p.clone(),
                                oid,
                                &meta,
                                0,
                            ));
                        }
                    }
                }
            }
            oxidize_index::UnstagedChange::Deleted(p) => {
                work_index.remove_entry(p);
            }
        }
    }
    let work_tree_oid =
        write_tree(&work_index, &loose_store).map_err(|e| TuiError::Terminal(e.to_string()))?;

    let stash_msg = if message.trim().is_empty() {
        format!("WIP on {}: {} {}", branch_name, head_short, head_first_line)
    } else {
        message.trim().to_string()
    };

    // 3. Stash commit: 2 parents (HEAD and index_commit_oid)
    let stash_commit = Commit {
        tree: work_tree_oid,
        parents: vec![head_oid, index_commit_oid],
        author: sig.clone(),
        committer: sig,
        gpg_sig: None,
        message: format!("{}\n", stash_msg),
    };
    let stash_oid = loose_store
        .write_object(&Object::Commit(stash_commit))
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    // 4. Update refs/stash using ref_store (maintains reflog!)
    ref_store
        .update_ref(
            "refs/stash",
            &stash_oid,
            None,
            &format!("WIP on {}: {}", branch_name, stash_msg),
        )
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    // 5. Reset worktree and index to HEAD commit tree
    checkout_tree_and_update_index(repo_root, git_dir, &head_commit.tree, true)?;

    Ok(stash_oid)
}

/// Pushes local branch commits to the specified remote repository using native Rust transport.
pub fn push_to_remote(
    _repo_root: &Path,
    git_dir: &Path,
    remote_opt: Option<&str>,
    branch_opt: Option<&str>,
    force: bool,
) -> Result<String, TuiError> {
    let config_path = git_dir.join("config");
    let config = GitConfig::load_from_file(&config_path).unwrap_or_default();

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

    if !is_ff && !force {
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
        let expected_old = if force || remote_old_oid.is_zero() {
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
        let _ = ref_store.update_ref(&tracking_ref, &local_oid, None, "update tracking ref");

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
        let _ = ref_store.update_ref(&tracking_ref, &local_oid, None, "update tracking ref");

        Ok(format!("Pushed {} to {}", branch, remote_name))
    }
}

/// Fetches objects and updates tracking refs from remote repository.
pub fn fetch_from_remote(git_dir: &Path, remote_opt: Option<&str>) -> Result<String, TuiError> {
    let config_path = git_dir.join("config");
    let config = GitConfig::load_from_file(&config_path).unwrap_or_default();

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
            let local_tracking_oid = ref_store.read_ref(&tracking_ref).ok();
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
        let _ = ref_store.update_ref(target_ref, oid, None, "fetch: update tracking ref");
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
    let a_clean = a.iter().filter(|&&byte| byte != b'\r');
    let b_clean = b.iter().filter(|&&byte| byte != b'\r');
    a_clean.eq(b_clean)
}

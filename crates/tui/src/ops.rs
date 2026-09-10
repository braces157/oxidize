use crate::model::{ReflogItem, RemoteItem, TagItem};
use crate::TuiError;
use oxidize_config::GitConfig;
use oxidize_core::id::ObjectId;
use oxidize_core::object::{Blob, Commit, Object, Signature};
use oxidize_core::store::LooseObjectStore;
use oxidize_index::{flatten_tree, write_tree, Index, IndexEntry};
use oxidize_pack::RepoObjectStore;
use oxidize_refs::RefStore;
use std::fs;
use std::path::Path;

/// Stages a single file (adds to index and loose object store, or removes from index if deleted in working tree).
pub fn stage_path(repo_root: &Path, git_dir: &Path, rel_path: &str) -> Result<(), TuiError> {
    let full_path = repo_root.join(rel_path);
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

    let store =
        RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;

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
        let full_path = repo_root.join(rel_path);
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
    let index_path = git_dir.join("index");
    let index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let full_path = repo_root.join(rel_path);

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

    if index.entries.is_empty() {
        return Err(TuiError::Terminal(
            "nothing to commit (index is empty)".to_string(),
        ));
    }

    let tree_oid = write_tree(&index, &store).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let (branch_name, head_commit_oid) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let parents = if let Some(p) = head_commit_oid {
        vec![p]
    } else {
        Vec::new()
    };

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
        .update_ref(&branch_name, &commit_oid, head_commit_oid.as_ref(), &ref_msg)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    Ok(commit_oid)
}

/// Checks out a commit tree to the working directory and synchronizes the index.
pub fn checkout_tree_and_update_index(
    repo_root: &Path,
    git_dir: &Path,
    target_tree_oid: &ObjectId,
) -> Result<(), TuiError> {
    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;

    let target_map = flatten_tree(&store, target_tree_oid, "")
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    // 1. Remove files from working tree that are in old index but not in new target tree
    for entry in &index.entries {
        if !target_map.contains_key(&entry.path) {
            let full_path = repo_root.join(&entry.path);
            if full_path.exists() {
                let _ = fs::remove_file(&full_path);
            }
        }
    }

    // 2. Write new files to working tree and create new index entries
    index.entries.clear();
    for (path, (_mode, oid)) in target_map {
        let full_path = repo_root.join(&path);
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

    index
        .write_to(&index_path)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    Ok(())
}

/// Switches HEAD to an existing local branch, updating the working directory and index.
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
        _ => return Err(TuiError::Terminal("branch does not point to a commit".to_string())),
    };

    checkout_tree_and_update_index(repo_root, git_dir, &commit.tree)?;
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
pub fn delete_branch(git_dir: &Path, branch_name: &str) -> Result<(), TuiError> {
    let ref_store = RefStore::new(git_dir);
    let (active_branch, _) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    if active_branch == branch_name {
        return Err(TuiError::Terminal(format!(
            "cannot delete checked-out branch '{}'",
            branch_name
        )));
    }

    ref_store
        .delete_branch(branch_name)
        .map_err(|e| TuiError::Terminal(e.to_string()))?;
    Ok(())
}

/// Pops the selected stash into the working tree and drops it from the stash stack.
pub fn pop_stash(
    repo_root: &Path,
    git_dir: &Path,
    stash_oid: &ObjectId,
) -> Result<(), TuiError> {
    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let stash_commit = match store.read_object(stash_oid) {
        Ok(Object::Commit(c)) => c,
        _ => return Err(TuiError::Terminal("stash is not a commit".to_string())),
    };

    checkout_tree_and_update_index(repo_root, git_dir, &stash_commit.tree)?;

    let stash_ref = git_dir.join("refs").join("stash");
    if stash_ref.exists() {
        let _ = fs::remove_file(stash_ref);
    }
    let stash_log = git_dir.join("logs").join("refs").join("stash");
    if stash_log.exists() {
        let _ = fs::remove_file(stash_log);
    }

    Ok(())
}

/// Drops all stash entries or the stash ref.
pub fn drop_stash(git_dir: &Path) -> Result<(), TuiError> {
    let stash_ref = git_dir.join("refs").join("stash");
    if stash_ref.exists() {
        let _ = fs::remove_file(stash_ref);
    }
    let stash_log = git_dir.join("logs").join("refs").join("stash");
    if stash_log.exists() {
        let _ = fs::remove_file(stash_log);
    }
    Ok(())
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
    let repo_store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let ref_store = RefStore::new(git_dir);
    let index_path = git_dir.join("index");
    let index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;

    let (branch_name, head_commit_oid) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let head_oid = head_commit_oid.ok_or_else(|| {
        TuiError::Terminal("cannot amend: repository has no commits".to_string())
    })?;

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
pub fn apply_stash(
    repo_root: &Path,
    git_dir: &Path,
    stash_oid: &ObjectId,
) -> Result<(), TuiError> {
    let store = RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let stash_commit = match store.read_object(stash_oid) {
        Ok(Object::Commit(c)) => c,
        _ => return Err(TuiError::Terminal("stash is not a commit".to_string())),
    };

    checkout_tree_and_update_index(repo_root, git_dir, &stash_commit.tree)
}

/// Creates a new stash commit saving working directory changes and index state.
pub fn stash_save(
    repo_root: &Path,
    git_dir: &Path,
    message: &str,
) -> Result<ObjectId, TuiError> {
    let store = LooseObjectStore::new(git_dir.join("objects"));
    let ref_store = RefStore::new(git_dir);
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path).map_err(|e| TuiError::Terminal(e.to_string()))?;

    // Stage all modified files into temporary index for the stash
    for entry in &mut index.entries {
        let full_path = repo_root.join(&entry.path);
        if full_path.exists() && full_path.is_file() {
            if let Ok(data) = fs::read(&full_path) {
                if let Ok(meta) = fs::metadata(&full_path) {
                    let blob = Object::Blob(Blob::new(data));
                    if let Ok(oid) = store.write_object(&blob) {
                        *entry = IndexEntry::from_fs_metadata(entry.path.clone(), oid, &meta, 0);
                    }
                }
            }
        }
    }

    let tree_oid = write_tree(&index, &store).map_err(|e| TuiError::Terminal(e.to_string()))?;
    let (branch_name, head_commit_oid) = ref_store
        .resolve_head()
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    let parents = if let Some(p) = head_commit_oid {
        vec![p]
    } else {
        Vec::new()
    };

    let sig = get_signature(git_dir);
    let stash_msg = if message.trim().is_empty() {
        format!(
            "WIP on {}: {}",
            branch_name,
            head_commit_oid
                .map(|o| o.to_string()[..7].to_string())
                .unwrap_or_default()
        )
    } else {
        message.trim().to_string()
    };

    let commit = Commit {
        tree: tree_oid,
        parents,
        author: sig.clone(),
        committer: sig,
        gpg_sig: None,
        message: format!("{}\n", stash_msg),
    };

    let stash_oid = store
        .write_object(&Object::Commit(commit))
        .map_err(|e| TuiError::Terminal(e.to_string()))?;

    // Update .git/refs/stash
    let stash_ref = git_dir.join("refs").join("stash");
    if let Some(parent) = stash_ref.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&stash_ref, format!("{}\n", stash_oid))?;

    // Append to .git/logs/refs/stash
    let stash_log = git_dir.join("logs").join("refs").join("stash");
    if let Some(parent) = stash_log.parent() {
        fs::create_dir_all(parent)?;
    }
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stash_log)?;
    let old_sha = "0000000000000000000000000000000000000000";
    writeln!(
        file,
        "{} {} Oxidize <user@oxidize.dev> 0 +0000\t{}",
        old_sha, stash_oid, stash_msg
    )?;

    Ok(stash_oid)
}

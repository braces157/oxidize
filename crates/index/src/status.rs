//! Working tree, index, and HEAD status computation.

use crate::index::Index;
use crate::IndexError;
use oxidize_core::id::ObjectId;
use oxidize_core::object::{FileMode, Object};
use oxidize_core::store::ObjectReader;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// Status category of a file in the staging area (index vs HEAD).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagedChange {
    /// Newly added file staged.
    New(String),
    /// Modified file staged.
    Modified(String),
    /// Deleted file staged.
    Deleted(String),
}

/// Status category of a file in the working tree (working tree vs index).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnstagedChange {
    /// Modified in working tree compared to index.
    Modified(String),
    /// Deleted in working tree compared to index.
    Deleted(String),
}

/// Overall repository status summarizing staged, unstaged, and untracked files.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RepoStatus {
    /// Files staged for next commit.
    pub staged: Vec<StagedChange>,
    /// Tracked files modified/deleted in working tree.
    pub unstaged: Vec<UnstagedChange>,
    /// Untracked files in working tree.
    pub untracked: Vec<String>,
}

/// Recursively flattens a Tree object into a map of `relative_path -> (mode, oid)`.
pub fn flatten_tree(
    store: &impl ObjectReader,
    tree_oid: &ObjectId,
    prefix: &str,
) -> Result<BTreeMap<String, (FileMode, ObjectId)>, IndexError> {
    let mut map = BTreeMap::new();
    let obj = store.read_object(tree_oid)?;
    let tree = match obj {
        Object::Tree(t) => t,
        _ => return Ok(map),
    };

    for entry in tree.entries {
        let path = if prefix.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", prefix, entry.name)
        };

        if entry.mode.is_tree() {
            let submap = flatten_tree(store, &entry.id, &path)?;
            map.extend(submap);
        } else {
            map.insert(path, (entry.mode, entry.id));
        }
    }

    Ok(map)
}

/// Closure type for ignore filtering: `(rel_path, is_dir) -> is_ignored`.
pub type IgnoreFilter<'a> = &'a dyn Fn(&str, bool) -> bool;

/// Computes the complete repository status, ignoring untracked files if matched by `is_ignored`.
pub fn compute_status_with_ignore(
    repo_root: &Path,
    index: &Index,
    head_tree_oid: Option<&ObjectId>,
    store: &impl ObjectReader,
    is_ignored: Option<IgnoreFilter>,
) -> Result<RepoStatus, IndexError> {
    let mut status = RepoStatus::default();

    // 1. Flatten HEAD tree (if repository has a commit)
    let head_entries = if let Some(oid) = head_tree_oid {
        flatten_tree(store, oid, "")?
    } else {
        BTreeMap::new()
    };

    // Index entries mapped by path
    let index_map: BTreeMap<String, &crate::entry::IndexEntry> = index
        .entries()
        .iter()
        .map(|e| (e.path.clone(), e))
        .collect();

    // Staged changes: compare index with HEAD
    for (path, entry) in &index_map {
        match head_entries.get(path) {
            None => {
                status.staged.push(StagedChange::New(path.clone()));
            }
            Some((_mode, head_oid)) => {
                if &entry.oid != head_oid {
                    status.staged.push(StagedChange::Modified(path.clone()));
                }
            }
        }
    }

    // Staged deletions: in HEAD but missing in index
    for path in head_entries.keys() {
        if !index_map.contains_key(path) {
            status.staged.push(StagedChange::Deleted(path.clone()));
        }
    }

    // 2. Unstaged changes: compare working tree with index
    for (path, entry) in &index_map {
        let full_path = repo_root.join(path);
        if !full_path.exists() {
            status.unstaged.push(UnstagedChange::Deleted(path.clone()));
        } else if let Ok(meta) = fs::metadata(&full_path) {
            let size = meta.len() as u32;
            if let Ok(data) = fs::read(&full_path) {
                let blob = Object::Blob(oxidize_core::object::Blob::new(data));
                if blob.id() != entry.oid {
                    status.unstaged.push(UnstagedChange::Modified(path.clone()));
                }
            } else if size != entry.file_size {
                status.unstaged.push(UnstagedChange::Modified(path.clone()));
            }
        }
    }

    // 3. Untracked files: scan working directory
    let mut tracked_or_ignored = BTreeSet::new();
    for path in index_map.keys() {
        tracked_or_ignored.insert(path.clone());
    }

    let mut untracked_list = Vec::new();
    scan_untracked(
        repo_root,
        repo_root,
        &tracked_or_ignored,
        &mut untracked_list,
        is_ignored,
    )?;
    untracked_list.sort();
    status.untracked = untracked_list;

    Ok(status)
}

/// Computes the complete repository status with default filters.
pub fn compute_status(
    repo_root: &Path,
    index: &Index,
    head_tree_oid: Option<&ObjectId>,
    store: &impl ObjectReader,
) -> Result<RepoStatus, IndexError> {
    compute_status_with_ignore(repo_root, index, head_tree_oid, store, None)
}

fn scan_untracked(
    root: &Path,
    current: &Path,
    tracked: &BTreeSet<String>,
    untracked: &mut Vec<String>,
    is_ignored: Option<IgnoreFilter>,
) -> Result<(), IndexError> {
    if !current.exists() || !current.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str == ".git" {
            continue;
        }

        let rel_path = path
            .strip_prefix(root)
            .map_err(|e| IndexError::EntryParseError(e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");

        if let Some(check_ignore) = is_ignored {
            if check_ignore(&rel_path, path.is_dir()) {
                continue;
            }
        }

        if path.is_dir() {
            // Check if directory contains any tracked files
            let dir_prefix = format!("{}/", rel_path);
            let has_tracked = tracked.iter().any(|t| t.starts_with(&dir_prefix));
            if has_tracked {
                scan_untracked(root, &path, tracked, untracked, is_ignored)?;
            } else {
                // Whole directory is untracked
                untracked.push(format!("{}/", rel_path));
            }
        } else if !tracked.contains(&rel_path) {
            untracked.push(rel_path);
        }
    }

    Ok(())
}

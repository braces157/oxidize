//! Working tree, index, and HEAD status computation.

use crate::index::Index;
use crate::IndexError;
use oxidize_core::id::ObjectId;
use oxidize_core::object::{FileMode, Object};
use oxidize_core::store::LooseObjectStore;
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

/// Status category of an unstaged file (working tree vs index).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnstagedChange {
    /// File modified in working directory.
    Modified(String),
    /// File deleted from working directory.
    Deleted(String),
}

/// Consolidated repository status.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoStatus {
    /// Changes staged for commit (index vs HEAD).
    pub staged: Vec<StagedChange>,
    /// Changes not staged for commit (working tree vs index).
    pub unstaged: Vec<UnstagedChange>,
    /// Files present in working tree but not tracked in index.
    pub untracked: Vec<String>,
}

/// Recursively flattens a Tree object into a map of `relative_path -> (mode, oid)`.
pub fn flatten_tree(
    store: &LooseObjectStore,
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

/// Computes the complete repository status.
pub fn compute_status(
    repo_root: &Path,
    index: &Index,
    head_tree_oid: Option<&ObjectId>,
    store: &LooseObjectStore,
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
            if size != entry.file_size {
                status.unstaged.push(UnstagedChange::Modified(path.clone()));
            } else {
                // Size matches, check if content hash matches
                if let Ok(data) = fs::read(&full_path) {
                    let blob = Object::Blob(oxidize_core::object::Blob::new(data));
                    if blob.id() != entry.oid {
                        status.unstaged.push(UnstagedChange::Modified(path.clone()));
                    }
                }
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
    )?;
    untracked_list.sort();
    status.untracked = untracked_list;

    Ok(status)
}

fn scan_untracked(
    root: &Path,
    current: &Path,
    tracked: &BTreeSet<String>,
    untracked: &mut Vec<String>,
) -> Result<(), IndexError> {
    if !current.exists() || !current.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str == ".git" || name_str == "target" {
            continue;
        }

        let rel_path = path
            .strip_prefix(root)
            .map_err(|e| IndexError::EntryParseError(e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");

        if path.is_dir() {
            // Check if directory contains any tracked files
            let dir_prefix = format!("{}/", rel_path);
            let has_tracked = tracked.iter().any(|t| t.starts_with(&dir_prefix));
            if has_tracked {
                scan_untracked(root, &path, tracked, untracked)?;
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

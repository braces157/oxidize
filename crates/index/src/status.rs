//! Working tree, index, and HEAD status computation.

use crate::index::Index;
use crate::IndexError;
use oxidize_core::id::ObjectId;
use oxidize_core::object::{FileMode, Object};
use oxidize_core::store::ObjectReader;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

/// Status category of a file in the staging area (index vs HEAD).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagedChange {
    /// Newly added file staged.
    New(String),
    /// Modified file staged.
    Modified(String),
    /// Deleted file staged.
    Deleted(String),
    /// Renamed file staged.
    Renamed {
        /// Previous path in HEAD.
        from: String,
        /// New path in index.
        to: String,
    },
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
    /// Unmerged paths with conflict stages (1, 2, 3).
    pub unmerged: Vec<String>,
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
        oxidize_core::validate_repo_path(&path).map_err(IndexError::Core)?;

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

    // Separate unmerged conflict stages (stage > 0) from normal staged entries (stage == 0)
    let mut unmerged_paths = BTreeSet::new();
    let mut stage0_entries: BTreeMap<String, &crate::entry::IndexEntry> = BTreeMap::new();

    for e in index.entries() {
        if e.stage > 0 {
            unmerged_paths.insert(e.path.clone());
        } else {
            stage0_entries.insert(e.path.clone(), e);
        }
    }
    // Remove any path from stage0_entries that also has unmerged stages
    for p in &unmerged_paths {
        stage0_entries.remove(p);
    }

    // Collect candidates for staged changes
    let mut staged_new: Vec<(String, ObjectId)> = Vec::new();
    let mut staged_modified: Vec<String> = Vec::new();
    let mut staged_deleted: Vec<(String, ObjectId)> = Vec::new();

    for (path, entry) in &stage0_entries {
        match head_entries.get(path) {
            None => {
                staged_new.push((path.clone(), entry.oid));
            }
            Some((head_mode, head_oid)) => {
                if &entry.oid != head_oid || entry.mode != head_mode.0 {
                    staged_modified.push(path.clone());
                }
            }
        }
    }

    for (path, (_mode, head_oid)) in &head_entries {
        if !stage0_entries.contains_key(path) && !unmerged_paths.contains(path) {
            staged_deleted.push((path.clone(), *head_oid));
        }
    }

    // Exact rename detection: match identical ObjectIds between staged_deleted and staged_new
    // Using an OID-indexed map avoids quadratic O(N*M) scanning on bulk renames.
    let mut deleted_by_oid: std::collections::HashMap<ObjectId, Vec<(usize, String)>> =
        std::collections::HashMap::new();
    for (idx, (del_path, del_oid)) in staged_deleted.iter().enumerate().rev() {
        deleted_by_oid
            .entry(*del_oid)
            .or_default()
            .push((idx, del_path.clone()));
    }

    let mut matched_deleted = std::collections::HashSet::new();
    let mut matched_new = std::collections::HashSet::new();

    for (new_idx, (new_path, new_oid)) in staged_new.iter().enumerate() {
        if let Some(candidates) = deleted_by_oid.get_mut(new_oid) {
            if let Some((del_idx, del_path)) = candidates.pop() {
                matched_deleted.insert(del_idx);
                matched_new.insert(new_idx);
                status.staged.push(StagedChange::Renamed {
                    from: del_path,
                    to: new_path.clone(),
                });
            }
        }
    }

    for path in staged_modified {
        status.staged.push(StagedChange::Modified(path));
    }

    for (idx, (path, _)) in staged_new.into_iter().enumerate() {
        if !matched_new.contains(&idx) {
            status.staged.push(StagedChange::New(path));
        }
    }

    for (idx, (path, _)) in staged_deleted.into_iter().enumerate() {
        if !matched_deleted.contains(&idx) {
            status.staged.push(StagedChange::Deleted(path));
        }
    }

    status.staged.sort_by(|a, b| {
        let path_a = match a {
            StagedChange::New(p) | StagedChange::Modified(p) | StagedChange::Deleted(p) => p,
            StagedChange::Renamed { to, .. } => to,
        };
        let path_b = match b {
            StagedChange::New(p) | StagedChange::Modified(p) | StagedChange::Deleted(p) => p,
            StagedChange::Renamed { to, .. } => to,
        };
        path_a.cmp(path_b)
    });

    // 2. Unstaged changes: compare working tree with index in parallel with Rayon
    let stage0_list: Vec<&crate::entry::IndexEntry> = stage0_entries.values().copied().collect();
    let mut unstaged: Vec<UnstagedChange> = stage0_list
        .into_par_iter()
        .filter_map(|entry| {
            let full_path = repo_root.join(&entry.path);
            let meta = match fs::symlink_metadata(&full_path) {
                Ok(m) => m,
                Err(err) => {
                    if err.kind() == std::io::ErrorKind::NotFound {
                        return Some(UnstagedChange::Deleted(entry.path.clone()));
                    } else {
                        // Stat/permission error: surface as modified rather than silently clean
                        return Some(UnstagedChange::Modified(entry.path.clone()));
                    }
                }
            };

            let size = meta.len() as u32;
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let duration = mtime
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default();
            let mtime_sec = duration.as_secs() as u32;
            let mtime_nsec = duration.subsec_nanos();

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let is_exec = (meta.permissions().mode() & 0o111) != 0;
                let is_symlink = meta.file_type().is_symlink();
                let file_mode = if is_symlink {
                    0o120000
                } else if is_exec {
                    0o100755
                } else {
                    0o100644
                };
                if file_mode != entry.mode {
                    return Some(UnstagedChange::Modified(entry.path.clone()));
                }
            }

            // Stat cache check:
            if size != entry.file_size {
                Some(UnstagedChange::Modified(entry.path.clone()))
            } else if entry.mtime_sec != 0
                && entry.mtime_sec == mtime_sec
                && entry.mtime_nsec == mtime_nsec
            {
                None
            } else {
                // Size matches but mtime changed (or mtime is 0). Hash to confirm actual content changes.
                match fs::read(&full_path) {
                    Ok(data) => {
                        let blob = Object::Blob(oxidize_core::object::Blob::new(data));
                        if blob.id() != entry.oid {
                            Some(UnstagedChange::Modified(entry.path.clone()))
                        } else {
                            None
                        }
                    }
                    Err(_) => Some(UnstagedChange::Modified(entry.path.clone())),
                }
            }
        })
        .collect();
    unstaged.sort_by(|a, b| {
        let p_a = match a {
            UnstagedChange::Modified(p) | UnstagedChange::Deleted(p) => p,
        };
        let p_b = match b {
            UnstagedChange::Modified(p) | UnstagedChange::Deleted(p) => p,
        };
        p_a.cmp(p_b)
    });
    status.unstaged = unstaged;

    // 3. Unmerged paths
    status.unmerged = unmerged_paths.iter().cloned().collect();

    // 4. Untracked files: scan working directory
    let mut tracked_or_ignored = BTreeSet::new();
    for path in stage0_entries.keys() {
        tracked_or_ignored.insert(path.clone());
    }
    for path in &unmerged_paths {
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
            // Check if directory contains any tracked files using O(log N) range lookup
            let dir_prefix = format!("{}/", rel_path);
            let has_tracked = tracked
                .range(dir_prefix.clone()..)
                .next()
                .is_some_and(|s| s.starts_with(&dir_prefix));
            if has_tracked {
                scan_untracked(root, &path, tracked, untracked, is_ignored)?;
            } else if dir_has_any_unignored_files(root, &path, is_ignored) {
                // Whole directory is untracked
                untracked.push(format!("{}/", rel_path));
            }
        } else if !tracked.contains(&rel_path) {
            untracked.push(rel_path);
        }
    }

    Ok(())
}

fn dir_has_any_unignored_files(root: &Path, dir: &Path, is_ignored: Option<IgnoreFilter>) -> bool {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy() == ".git" {
            continue;
        }
        let rel_path = match path.strip_prefix(root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        let is_dir = path.is_dir();
        if let Some(check_ignore) = is_ignored {
            if check_ignore(&rel_path, is_dir) {
                continue;
            }
        }
        if is_dir {
            if dir_has_any_unignored_files(root, &path, is_ignored) {
                return true;
            }
        } else {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxidize_core::object::{Blob, Tree, TreeEntry};
    use oxidize_core::store::LooseObjectStore;

    #[test]
    fn test_status_rename_detection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo_root = temp_dir.path();
        let objects_dir = repo_root.join(".git").join("objects");
        let store = LooseObjectStore::new(&objects_dir);

        // Create blob for "content"
        let blob = Blob::new(b"hello world".to_vec());
        let blob_id = store.write_object(&Object::Blob(blob)).unwrap();

        // Create HEAD tree containing "old_name.txt"
        let tree = Tree::new(vec![TreeEntry {
            mode: FileMode::REGULAR,
            name: "old_name.txt".to_string(),
            id: blob_id,
        }]);
        let tree_id = store.write_object(&Object::Tree(tree)).unwrap();

        // Create index containing "new_name.txt" with the same blob_id
        let mut index = Index::new();
        index.add_entry(crate::entry::IndexEntry {
            ctime_sec: 100,
            ctime_nsec: 100,
            mtime_sec: 100,
            mtime_nsec: 100,
            dev: 0,
            ino: 0,
            mode: 0o100644,
            uid: 0,
            gid: 0,
            file_size: 11,
            oid: blob_id,
            stage: 0,
            assume_valid: false,
            path: "new_name.txt".to_string(),
        });

        // Write file on disk so it's not detected as unstaged deleted
        std::fs::write(repo_root.join("new_name.txt"), b"hello world").unwrap();

        let status = compute_status(repo_root, &index, Some(&tree_id), &store).unwrap();
        assert_eq!(
            status.staged,
            vec![StagedChange::Renamed {
                from: "old_name.txt".to_string(),
                to: "new_name.txt".to_string(),
            }]
        );
    }

    #[test]
    fn test_status_mode_change() {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo_root = temp_dir.path();
        let objects_dir = repo_root.join(".git").join("objects");
        let store = LooseObjectStore::new(&objects_dir);

        let blob = Blob::new(b"script".to_vec());
        let blob_id = store.write_object(&Object::Blob(blob)).unwrap();

        // HEAD tree has script.sh as 100644 regular file
        let tree = Tree::new(vec![TreeEntry {
            mode: FileMode::REGULAR,
            name: "script.sh".to_string(),
            id: blob_id,
        }]);
        let tree_id = store.write_object(&Object::Tree(tree)).unwrap();

        // Index has script.sh with identical blob_id, but mode 100755 (chmod +x)
        let mut index = Index::new();
        index.add_entry(crate::entry::IndexEntry {
            ctime_sec: 100,
            ctime_nsec: 100,
            mtime_sec: 100,
            mtime_nsec: 100,
            dev: 0,
            ino: 0,
            mode: 0o100755,
            uid: 0,
            gid: 0,
            file_size: 6,
            oid: blob_id,
            stage: 0,
            assume_valid: false,
            path: "script.sh".to_string(),
        });

        std::fs::write(repo_root.join("script.sh"), b"script").unwrap();

        let status = compute_status(repo_root, &index, Some(&tree_id), &store).unwrap();
        assert_eq!(
            status.staged,
            vec![StagedChange::Modified("script.sh".to_string())]
        );
    }

    #[test]
    fn test_status_unmerged_paths() {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo_root = temp_dir.path();
        let objects_dir = repo_root.join(".git").join("objects");
        let store = LooseObjectStore::new(&objects_dir);

        let mut index = Index::new();
        // Add conflict stages 1, 2, 3 for conflict.txt
        for stage in 1..=3 {
            index.add_entry(crate::entry::IndexEntry {
                ctime_sec: 100,
                ctime_nsec: 100,
                mtime_sec: 100,
                mtime_nsec: 100,
                dev: 0,
                ino: 0,
                mode: 0o100644,
                uid: 0,
                gid: 0,
                file_size: 10,
                oid: ObjectId::from_bytes([0x11; 20]),
                stage,
                assume_valid: false,
                path: "conflict.txt".to_string(),
            });
        }

        std::fs::write(repo_root.join("conflict.txt"), b"conflict").unwrap();

        let status = compute_status(repo_root, &index, None, &store).unwrap();
        assert_eq!(status.unmerged, vec!["conflict.txt".to_string()]);
        assert!(status.staged.is_empty());
        assert!(status.untracked.is_empty());
    }
}

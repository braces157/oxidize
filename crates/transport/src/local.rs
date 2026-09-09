//! Local repository transport for filesystem paths and file:// URLs.

use crate::protocol::RemoteRef;
use crate::TransportError;
use oxidize_core::id::ObjectId;
use oxidize_pack::{write_pack, RepoObjectStore};
use oxidize_refs::RefStore;
use std::path::{Path, PathBuf};

/// Resolves a local path from a URL (handling file:// prefixes and Windows drive letters).
pub fn resolve_local_path(url: &str) -> Option<PathBuf> {
    if let Some(rest) = url.strip_prefix("file://") {
        #[cfg(windows)]
        let rest = rest.strip_prefix('/').unwrap_or(rest);
        Some(PathBuf::from(rest))
    } else if Path::new(url).exists()
        || url.starts_with('.')
        || url.starts_with('/')
        || (url.len() >= 2 && url.as_bytes()[1] == b':')
    {
        Some(PathBuf::from(url))
    } else {
        None
    }
}

/// Discovers references in a local repository (bare or standard).
pub fn discover_local_refs(
    path: &Path,
) -> Result<(Vec<RemoteRef>, Option<String>), TransportError> {
    let git_dir = if path.join(".git").is_dir() {
        path.join(".git")
    } else if path.join("HEAD").exists() {
        path.to_path_buf()
    } else {
        return Err(TransportError::RefError(format!(
            "not a git repository: {}",
            path.display()
        )));
    };

    let ref_store = RefStore::new(&git_dir);
    let mut refs = Vec::new();
    let (head_target, head_oid_opt) = ref_store
        .resolve_head()
        .map_err(|e| TransportError::RefError(format!("failed to resolve HEAD: {}", e)))?;

    let default_branch = if head_target == "HEAD" {
        None
    } else if head_target.starts_with("refs/") {
        Some(head_target)
    } else {
        Some(format!("refs/heads/{}", head_target))
    };

    if let Some(head_oid) = head_oid_opt {
        refs.push(RemoteRef {
            oid: head_oid,
            name: "HEAD".to_string(),
        });
    }

    let branches = ref_store
        .list_branches()
        .map_err(|e| TransportError::RefError(format!("failed to list branches: {}", e)))?;

    for (branch, oid) in branches {
        refs.push(RemoteRef {
            oid,
            name: format!("refs/heads/{}", branch),
        });
    }

    // Also include tags if present
    let tags_dir = git_dir.join("refs").join("tags");
    if tags_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(tags_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() {
                    let tag = p.file_name().unwrap().to_string_lossy().to_string();
                    if let Ok(oid_str) = std::fs::read_to_string(&p) {
                        if let Ok(oid) = oid_str.trim().parse::<ObjectId>() {
                            refs.push(RemoteRef {
                                oid,
                                name: format!("refs/tags/{}", tag),
                            });
                        }
                    }
                }
            }
        }
    }

    Ok((refs, default_branch))
}

/// Packs all objects required for the given `wants` from a local repository.
pub fn fetch_local_pack(
    source_path: &Path,
    _wants: &[ObjectId],
) -> Result<Vec<u8>, TransportError> {
    let git_dir = if source_path.join(".git").is_dir() {
        source_path.join(".git")
    } else {
        source_path.to_path_buf()
    };

    let store = RepoObjectStore::open(&git_dir)?;
    let all_objects = store.collect_all_objects()?;
    let (pack_bytes, _, _) = write_pack(&all_objects, true)?;
    Ok(pack_bytes)
}

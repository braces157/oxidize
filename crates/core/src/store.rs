//! Loose object content-addressable storage.

use crate::error::CoreError;
use crate::id::ObjectId;
use crate::object::{Blob, Commit, FileMode, Object, ObjectType, Signature, Tag, Tree, TreeEntry};
use crate::path::strip_verbatim_prefix;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Storage and retrieval of Git loose objects from `.git/objects/`.
#[derive(Debug, Clone)]
pub struct LooseObjectStore {
    root: PathBuf,
}

/// Trait for read-only object retrieval from storage (loose or packed).
pub trait ObjectReader {
    /// Reads and deserializes a Git object by ID.
    fn read_object(&self, id: &ObjectId) -> Result<Object, CoreError>;

    /// Resolves an object by prefix.
    fn find_by_prefix(&self, prefix: &str) -> Result<ObjectId, CoreError> {
        if prefix.len() == 40 {
            if let Ok(oid) = prefix.parse::<ObjectId>() {
                if self.read_object(&oid).is_ok() {
                    return Ok(oid);
                }
            }
        }
        Err(CoreError::ObjectNotFound(prefix.to_string()))
    }
}

impl ObjectReader for LooseObjectStore {
    fn read_object(&self, id: &ObjectId) -> Result<Object, CoreError> {
        self.read_object(id)
    }

    fn find_by_prefix(&self, prefix: &str) -> Result<ObjectId, CoreError> {
        self.find_by_prefix(prefix)
    }
}

impl LooseObjectStore {
    /// Creates a store pointing to `.git/objects/`.
    pub fn new(objects_dir: impl Into<PathBuf>) -> Self {
        Self {
            root: objects_dir.into(),
        }
    }

    /// Returns the path to the directory containing objects.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Computes the filesystem path for a given loose object ID.
    pub fn object_path(&self, id: &ObjectId) -> PathBuf {
        self.root.join(id.loose_dir()).join(id.loose_file())
    }

    /// Checks if a loose object exists.
    pub fn exists(&self, id: &ObjectId) -> bool {
        self.object_path(id).exists()
    }

    /// Writes an object to the loose object store, returning its `ObjectId`.
    pub fn write_object(&self, object: &Object) -> Result<ObjectId, CoreError> {
        let id = object.id();
        let path = self.object_path(&id);

        if path.exists() {
            return Ok(id);
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let serialized = object.serialize_with_header();

        // Write to a uniquely named temporary file in the same directory then atomically rename
        let temp_filename = format!(
            "{}.{}.tmp",
            id.loose_file(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let temp_path = self.root.join(id.loose_dir()).join(temp_filename);
        let temp_file = File::create(&temp_path)?;
        let mut encoder = ZlibEncoder::new(temp_file, Compression::default());
        encoder.write_all(&serialized)?;
        encoder.finish()?;

        if let Err(e) = fs::rename(&temp_path, &path) {
            let _ = fs::remove_file(&temp_path);
            if !path.exists() {
                return Err(CoreError::Io(e));
            }
        }

        Ok(id)
    }

    /// Reads an object by its ID from the loose object store.
    pub fn read_object(&self, id: &ObjectId) -> Result<Object, CoreError> {
        let path = self.object_path(id);
        if !path.exists() {
            return Err(CoreError::ObjectNotFound(id.to_string()));
        }

        let file = File::open(path)?;
        let mut decoder = ZlibDecoder::new(file);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)?;

        parse_loose_object(&decompressed)
    }

    /// Reads raw object payload and its object type directly from loose storage.
    pub fn read_raw(&self, id: &ObjectId) -> Result<(ObjectType, Vec<u8>), CoreError> {
        let path = self.object_path(id);
        if !path.exists() {
            return Err(CoreError::ObjectNotFound(id.to_string()));
        }

        let file = File::open(path)?;
        let mut decoder = ZlibDecoder::new(file);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)?;

        let nul_pos = decompressed
            .iter()
            .position(|&b| b == 0)
            .ok_or(CoreError::CorruptedHeader)?;
        let header = std::str::from_utf8(&decompressed[..nul_pos])
            .map_err(|_| CoreError::CorruptedHeader)?;
        let mut parts = header.split(' ');
        let type_str = parts.next().ok_or(CoreError::CorruptedHeader)?;
        let obj_type: ObjectType = type_str.parse()?;
        let data = decompressed[nul_pos + 1..].to_vec();
        Ok((obj_type, data))
    }

    /// Resolves an object ID by full hex or prefix (at least 4 characters).
    pub fn find_by_prefix(&self, prefix: &str) -> Result<ObjectId, CoreError> {
        let prefix = prefix.trim();
        if prefix.len() == 40 {
            let oid: ObjectId = prefix.parse()?;
            if self.exists(&oid) {
                return Ok(oid);
            } else {
                return Err(CoreError::ObjectNotFound(prefix.to_string()));
            }
        }

        if prefix.len() < 4 {
            return Err(CoreError::InvalidObjectId(
                "object prefix must be at least 4 characters".to_string(),
            ));
        }

        let dir_prefix = &prefix[..2];
        let file_prefix = &prefix[2..];
        let dir_path = self.root.join(dir_prefix);

        if !dir_path.exists() {
            return Err(CoreError::ObjectNotFound(prefix.to_string()));
        }

        let mut matches = Vec::new();
        for entry in fs::read_dir(dir_path)? {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(file_prefix) && name_str.len() == 38 {
                let full_hex = format!("{}{}", dir_prefix, name_str);
                if let Ok(oid) = full_hex.parse::<ObjectId>() {
                    matches.push(oid);
                }
            }
        }

        match matches.len() {
            0 => Err(CoreError::ObjectNotFound(prefix.to_string())),
            1 => Ok(matches[0]),
            _ => Err(CoreError::AmbiguousPrefix(prefix.to_string())),
        }
    }

    /// Convenience helper to create and store a Blob from byte slice.
    pub fn write_blob(&self, data: &[u8]) -> Result<ObjectId, CoreError> {
        let blob = Object::Blob(Blob::new(data.to_vec()));
        self.write_object(&blob)
    }
}

/// Repository layout context distinguishing worktree, git dir, common dir, and bare mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoContext {
    /// Working tree root directory, if this repository has a worktree.
    pub worktree: Option<PathBuf>,
    /// Git directory for this worktree (contains index, HEAD, MERGE_HEAD, etc.).
    pub git_dir: PathBuf,
    /// Common git directory (contains objects, refs, config, hooks).
    /// Same as `git_dir` for standard repositories and bare repositories.
    /// Points to main repository for linked worktrees.
    pub common_dir: PathBuf,
    /// Whether this repository is bare.
    pub is_bare: bool,
}

impl RepoContext {
    /// Discovers repository context starting from `start` and walking ancestor directories.
    pub fn discover(start: &Path) -> Result<Self, CoreError> {
        let mut current = if start.is_relative() {
            std::env::current_dir()?.join(start)
        } else {
            start.to_path_buf()
        };

        if let Ok(canon) = current.canonicalize() {
            current = strip_verbatim_prefix(&canon);
        }

        loop {
            let candidate = current.join(".git");
            if candidate.is_dir() {
                // Check for linked worktree (commondir file inside git_dir)
                let commondir_file = candidate.join("commondir");
                let common_dir = if commondir_file.is_file() {
                    let rel = std::fs::read_to_string(&commondir_file)?.trim().to_string();
                    candidate.join(rel)
                } else {
                    candidate.clone()
                };

                let common_dir =
                    strip_verbatim_prefix(&common_dir.canonicalize().unwrap_or(common_dir));
                let candidate =
                    strip_verbatim_prefix(&candidate.canonicalize().unwrap_or(candidate));
                let worktree = Some(current.clone());

                return Ok(Self {
                    worktree,
                    git_dir: candidate,
                    common_dir,
                    is_bare: false,
                });
            } else if candidate.is_file() {
                // Gitfile: e.g. "gitdir: <path>"
                let content = std::fs::read_to_string(&candidate)?;
                let trimmed = content.trim();
                if let Some(rest) = trimmed.strip_prefix("gitdir:") {
                    let gitdir_path_str = rest.trim();
                    let target_gitdir = if Path::new(gitdir_path_str).is_relative() {
                        current.join(gitdir_path_str)
                    } else {
                        PathBuf::from(gitdir_path_str)
                    };
                    let target_gitdir = strip_verbatim_prefix(
                        &target_gitdir.canonicalize().unwrap_or(target_gitdir),
                    );
                    if target_gitdir.is_dir() {
                        let commondir_file = target_gitdir.join("commondir");
                        let common_dir = if commondir_file.is_file() {
                            let rel = std::fs::read_to_string(&commondir_file)?.trim().to_string();
                            target_gitdir.join(rel)
                        } else {
                            target_gitdir.clone()
                        };
                        let common_dir =
                            strip_verbatim_prefix(&common_dir.canonicalize().unwrap_or(common_dir));
                        return Ok(Self {
                            worktree: Some(current),
                            git_dir: target_gitdir,
                            common_dir,
                            is_bare: false,
                        });
                    }
                }
                return Err(CoreError::RepoNotFound);
            }

            // Check if current directory itself is a bare repository or a .git directory
            if current.join("HEAD").is_file()
                && current.join("objects").is_dir()
                && current.join("refs").is_dir()
            {
                // If this directory is named ".git", it is the git_dir of its parent worktree,
                // unless bare = true is configured in config.
                let is_named_git = current.file_name().map(|n| n == ".git").unwrap_or(false);
                let is_bare_config =
                    if let Ok(cfg) = std::fs::read_to_string(current.join("config")) {
                        cfg.lines().any(|l| {
                            let t = l.trim();
                            t == "bare = true" || t == "bare=true"
                        })
                    } else {
                        false
                    };

                if is_named_git && !is_bare_config {
                    if let Some(parent) = current.parent() {
                        let git_dir = strip_verbatim_prefix(
                            &current.canonicalize().unwrap_or(current.clone()),
                        );
                        let commondir_file = git_dir.join("commondir");
                        let common_dir = if commondir_file.is_file() {
                            let rel = std::fs::read_to_string(&commondir_file)?.trim().to_string();
                            git_dir.join(rel)
                        } else {
                            git_dir.clone()
                        };
                        let common_dir =
                            strip_verbatim_prefix(&common_dir.canonicalize().unwrap_or(common_dir));
                        let worktree = strip_verbatim_prefix(
                            &parent.canonicalize().unwrap_or(parent.to_path_buf()),
                        );
                        return Ok(Self {
                            worktree: Some(worktree),
                            git_dir,
                            common_dir,
                            is_bare: false,
                        });
                    }
                }

                let bare_dir = strip_verbatim_prefix(&current.canonicalize().unwrap_or(current));
                return Ok(Self {
                    worktree: None,
                    git_dir: bare_dir.clone(),
                    common_dir: bare_dir,
                    is_bare: true,
                });
            }

            if !current.pop() {
                return Err(CoreError::RepoNotFound);
            }
        }
    }
}

/// Locates the `.git` directory starting from `start` and traversing ancestors.
pub fn find_git_dir(start: &Path) -> Result<PathBuf, CoreError> {
    RepoContext::discover(start).map(|ctx| ctx.git_dir)
}

/// Parses raw decompressed bytes into an `Object`.
pub fn parse_loose_object(bytes: &[u8]) -> Result<Object, CoreError> {
    // Format: "<type> <size>\0<data>"
    let nul_pos = bytes
        .iter()
        .position(|&b| b == 0)
        .ok_or(CoreError::CorruptedHeader)?;
    let header = std::str::from_utf8(&bytes[..nul_pos]).map_err(|_| CoreError::CorruptedHeader)?;
    let mut parts = header.split(' ');
    let type_str = parts.next().ok_or(CoreError::CorruptedHeader)?;
    let size_str = parts.next().ok_or(CoreError::CorruptedHeader)?;

    let obj_type: ObjectType = type_str.parse()?;
    let size: usize = size_str.parse().map_err(|_| CoreError::CorruptedHeader)?;
    let data = &bytes[nul_pos + 1..];

    if data.len() != size {
        return Err(CoreError::SizeMismatch {
            expected: size,
            actual: data.len(),
        });
    }

    parse_object_from_content(obj_type, data)
}

/// Parses raw object payload (without `<type> <size>\0` header) given its `ObjectType`.
pub fn parse_object_from_content(obj_type: ObjectType, data: &[u8]) -> Result<Object, CoreError> {
    match obj_type {
        ObjectType::Blob => Ok(Object::Blob(Blob::new(data.to_vec()))),
        ObjectType::Tree => parse_tree_content(data),
        ObjectType::Commit => parse_commit_content(data),
        ObjectType::Tag => parse_tag_content(data),
    }
}

fn parse_tree_content(data: &[u8]) -> Result<Object, CoreError> {
    let mut entries = Vec::new();
    let mut cursor = 0;

    while cursor < data.len() {
        let space_pos = data[cursor..]
            .iter()
            .position(|&b| b == b' ')
            .ok_or_else(|| CoreError::ParseError {
                object_type: "tree",
                reason: "missing space after mode".to_string(),
            })?
            + cursor;
        let mode_str =
            std::str::from_utf8(&data[cursor..space_pos]).map_err(|_| CoreError::ParseError {
                object_type: "tree",
                reason: "invalid mode utf8".to_string(),
            })?;
        let mode_num = u32::from_str_radix(mode_str, 8).map_err(|_| CoreError::ParseError {
            object_type: "tree",
            reason: format!("invalid octal mode {}", mode_str),
        })?;

        let nul_pos = data[space_pos + 1..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| CoreError::ParseError {
                object_type: "tree",
                reason: "missing nul after name".to_string(),
            })?
            + (space_pos + 1);
        let name = std::str::from_utf8(&data[space_pos + 1..nul_pos]).map_err(|_| {
            CoreError::ParseError {
                object_type: "tree",
                reason: "invalid name utf8".to_string(),
            }
        })?;
        crate::path::validate_tree_component(name).map_err(|e| CoreError::ParseError {
            object_type: "tree",
            reason: format!("invalid tree entry name '{}': {}", name, e),
        })?;

        let oid_start = nul_pos + 1;
        let oid_end = oid_start + 20;
        if oid_end > data.len() {
            return Err(CoreError::ParseError {
                object_type: "tree",
                reason: "truncated object id in tree entry".to_string(),
            });
        }

        let mut oid_bytes = [0u8; 20];
        oid_bytes.copy_from_slice(&data[oid_start..oid_end]);
        let id = ObjectId::from_bytes(oid_bytes);

        entries.push(TreeEntry {
            mode: FileMode(mode_num),
            name: name.to_string(),
            id,
        });

        cursor = oid_end;
    }

    Ok(Object::Tree(Tree::new(entries)))
}

fn parse_commit_content(data: &[u8]) -> Result<Object, CoreError> {
    let text = std::str::from_utf8(data).map_err(|_| CoreError::ParseError {
        object_type: "commit",
        reason: "non-utf8 commit content".to_string(),
    })?;

    let mut tree_id = None;
    let mut parents = Vec::new();
    let mut author = None;
    let mut committer = None;
    let mut gpg_sig = None;

    let mut in_gpg = false;
    let mut gpg_buf = String::new();

    let mut message_offset = 0;
    let mut byte_idx = 0;

    for line in text.split('\n') {
        byte_idx += line.len() + 1;
        if line.is_empty() && !in_gpg {
            message_offset = byte_idx.min(data.len());
            break;
        }

        if in_gpg {
            if let Some(rest) = line.strip_prefix(' ') {
                gpg_buf.push_str(rest);
                gpg_buf.push('\n');
                continue;
            } else {
                in_gpg = false;
                gpg_sig = Some(gpg_buf.clone());
            }
        }

        if let Some(rest) = line.strip_prefix("tree ") {
            tree_id = Some(rest.trim().parse()?);
        } else if let Some(rest) = line.strip_prefix("parent ") {
            parents.push(rest.trim().parse()?);
        } else if let Some(rest) = line.strip_prefix("author ") {
            author = Some(parse_signature(rest)?);
        } else if let Some(rest) = line.strip_prefix("committer ") {
            committer = Some(parse_signature(rest)?);
        } else if let Some(rest) = line.strip_prefix("gpgsig ") {
            in_gpg = true;
            gpg_buf.clear();
            if !rest.is_empty() {
                gpg_buf.push_str(rest);
                gpg_buf.push('\n');
            }
        }
    }

    let message = if message_offset <= data.len() {
        std::str::from_utf8(&data[message_offset..])
            .unwrap_or("")
            .to_string()
    } else {
        String::new()
    };

    let tree = tree_id.ok_or_else(|| CoreError::ParseError {
        object_type: "commit",
        reason: "missing tree field".to_string(),
    })?;
    let author = author.ok_or_else(|| CoreError::ParseError {
        object_type: "commit",
        reason: "missing author field".to_string(),
    })?;
    let committer = committer.ok_or_else(|| CoreError::ParseError {
        object_type: "commit",
        reason: "missing committer field".to_string(),
    })?;

    Ok(Object::Commit(Commit {
        tree,
        parents,
        author,
        committer,
        gpg_sig,
        message,
    }))
}

fn parse_tag_content(data: &[u8]) -> Result<Object, CoreError> {
    let text = std::str::from_utf8(data).map_err(|_| CoreError::ParseError {
        object_type: "tag",
        reason: "non-utf8 tag content".to_string(),
    })?;

    let mut target = None;
    let mut target_type = None;
    let mut tag_name = None;
    let mut tagger = None;
    let mut message_offset = 0;
    let mut byte_idx = 0;

    for line in text.split('\n') {
        byte_idx += line.len() + 1;
        if line.is_empty() {
            message_offset = byte_idx.min(data.len());
            break;
        }

        if let Some(rest) = line.strip_prefix("object ") {
            target = Some(rest.trim().parse()?);
        } else if let Some(rest) = line.strip_prefix("type ") {
            target_type = Some(rest.trim().parse()?);
        } else if let Some(rest) = line.strip_prefix("tag ") {
            tag_name = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("tagger ") {
            tagger = Some(parse_signature(rest)?);
        }
    }

    let message = if message_offset <= data.len() {
        std::str::from_utf8(&data[message_offset..])
            .unwrap_or("")
            .to_string()
    } else {
        String::new()
    };

    let target = target.ok_or_else(|| CoreError::ParseError {
        object_type: "tag",
        reason: "missing object field".to_string(),
    })?;
    let target_type = target_type.ok_or_else(|| CoreError::ParseError {
        object_type: "tag",
        reason: "missing type field".to_string(),
    })?;
    let name = tag_name.ok_or_else(|| CoreError::ParseError {
        object_type: "tag",
        reason: "missing tag field".to_string(),
    })?;

    Ok(Object::Tag(Tag {
        target,
        target_type,
        name,
        tagger,
        message,
    }))
}

fn parse_signature(s: &str) -> Result<Signature, CoreError> {
    Signature::parse(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_repo_context_discover_standard() {
        let temp = TempDir::new().unwrap();
        let repo_root = temp.path().join("repo");
        fs::create_dir(&repo_root).unwrap();
        fs::create_dir(repo_root.join(".git")).unwrap();

        let sub_dir = repo_root.join("sub").join("nested");
        fs::create_dir_all(&sub_dir).unwrap();

        let ctx = RepoContext::discover(&sub_dir).unwrap();
        assert!(!ctx.is_bare);
        let repo_root = strip_verbatim_prefix(&repo_root.canonicalize().unwrap());
        assert_eq!(ctx.worktree.unwrap(), repo_root);
        assert_eq!(ctx.git_dir, repo_root.join(".git"));
        assert_eq!(ctx.common_dir, repo_root.join(".git"));
    }

    #[test]
    fn test_repo_context_discover_gitfile_and_linked_worktree() {
        let temp = TempDir::new().unwrap();
        let main_repo = temp.path().join("main_repo");
        let main_git = main_repo.join(".git");
        fs::create_dir_all(&main_git).unwrap();

        // Linked worktree repo
        let worktree_dir = temp.path().join("wt");
        fs::create_dir_all(&worktree_dir).unwrap();

        let wt_gitdir = main_git.join("worktrees").join("wt");
        fs::create_dir_all(&wt_gitdir).unwrap();
        fs::write(wt_gitdir.join("commondir"), "../..\n").unwrap();

        // Worktree has .git file pointing to wt_gitdir
        fs::write(
            worktree_dir.join(".git"),
            format!("gitdir: {}\n", wt_gitdir.display()),
        )
        .unwrap();

        let ctx = RepoContext::discover(&worktree_dir).unwrap();
        assert!(!ctx.is_bare);
        let worktree_dir = strip_verbatim_prefix(&worktree_dir.canonicalize().unwrap());
        let wt_gitdir = strip_verbatim_prefix(&wt_gitdir.canonicalize().unwrap());
        let main_git = strip_verbatim_prefix(&main_git.canonicalize().unwrap());
        assert_eq!(ctx.worktree.unwrap(), worktree_dir);
        assert_eq!(ctx.git_dir, wt_gitdir);
        assert_eq!(ctx.common_dir, main_git);
    }

    #[test]
    fn test_repo_context_discover_bare() {
        let temp = TempDir::new().unwrap();
        let bare_dir = temp.path().join("bare.git");
        fs::create_dir_all(bare_dir.join("objects")).unwrap();
        fs::create_dir_all(bare_dir.join("refs")).unwrap();
        fs::write(bare_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let ctx = RepoContext::discover(&bare_dir).unwrap();
        assert!(ctx.is_bare);
        assert!(ctx.worktree.is_none());
        let bare_dir = strip_verbatim_prefix(&bare_dir.canonicalize().unwrap());
        assert_eq!(ctx.git_dir, bare_dir);
        assert_eq!(ctx.common_dir, bare_dir);
    }
}

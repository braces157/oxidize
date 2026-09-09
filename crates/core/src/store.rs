//! Loose object content-addressable storage.

use crate::error::CoreError;
use crate::id::ObjectId;
use crate::object::{Blob, Commit, FileMode, Object, ObjectType, Signature, Tag, Tree, TreeEntry};
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

        // Write to a temporary file in the same directory then atomically rename
        let temp_path = path.with_extension("tmp");
        let temp_file = File::create(&temp_path)?;
        let mut encoder = ZlibEncoder::new(temp_file, Compression::default());
        encoder.write_all(&serialized)?;
        encoder.finish()?;

        fs::rename(temp_path, path)?;

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
    // Format: "Name <email> timestamp tz"
    let open_bracket = s.find('<').ok_or_else(|| CoreError::ParseError {
        object_type: "signature",
        reason: "missing '<'".to_string(),
    })?;
    let close_bracket = s.find('>').ok_or_else(|| CoreError::ParseError {
        object_type: "signature",
        reason: "missing '>'".to_string(),
    })?;

    let name = s[..open_bracket].trim().to_string();
    let email = s[open_bracket + 1..close_bracket].trim().to_string();
    let rest = s[close_bracket + 1..].trim();
    let mut rest_parts = rest.split_whitespace();
    let time_str = rest_parts.next().ok_or_else(|| CoreError::ParseError {
        object_type: "signature",
        reason: "missing timestamp".to_string(),
    })?;
    let tz_offset = rest_parts.next().unwrap_or("+0000").to_string();

    let time_seconds: i64 = time_str.parse().map_err(|_| CoreError::ParseError {
        object_type: "signature",
        reason: "invalid timestamp number".to_string(),
    })?;

    Ok(Signature {
        name,
        email,
        time_seconds,
        tz_offset,
    })
}

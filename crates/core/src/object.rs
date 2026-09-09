//! Git object types: Blob, Tree, Commit, Tag, and file modes.

use crate::error::CoreError;
use crate::id::ObjectId;
use sha1::{Digest, Sha1};
use std::fmt;

/// Object types recognized by Git.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectType {
    /// Arbitrary binary content.
    Blob,
    /// Directory listing of entries.
    Tree,
    /// Commit pointing to a tree, parents, and metadata.
    Commit,
    /// Annotated tag pointing to an object.
    Tag,
}

impl ObjectType {
    /// Returns the canonical Git type name string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Blob => "blob",
            Self::Tree => "tree",
            Self::Commit => "commit",
            Self::Tag => "tag",
        }
    }
}

impl fmt::Display for ObjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ObjectType {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "blob" => Ok(Self::Blob),
            "tree" => Ok(Self::Tree),
            "commit" => Ok(Self::Commit),
            "tag" => Ok(Self::Tag),
            _ => Err(CoreError::UnknownObjectType(s.to_string())),
        }
    }
}

/// Git file mode representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileMode(pub u32);

impl FileMode {
    /// Normal non-executable file (`100644`).
    pub const REGULAR: Self = Self(0o100644);
    /// Executable file (`100755`).
    pub const EXECUTABLE: Self = Self(0o100755);
    /// Directory / Subtree (`040000`).
    pub const TREE: Self = Self(0o040000);
    /// Symbolic link (`120000`).
    pub const SYMLINK: Self = Self(0o120000);
    /// Submodule / gitlink (`160000`).
    pub const GITLINK: Self = Self(0o160000);

    /// Checks if this mode represents a directory / tree.
    pub fn is_tree(&self) -> bool {
        self.0 == Self::TREE.0
    }

    /// Octal string representation used in Tree serialization (e.g. "100644" or "40000").
    pub fn as_octal_str(&self) -> String {
        format!("{:o}", self.0)
    }

    /// Returns the canonical object type associated with this mode.
    pub fn object_type(&self) -> ObjectType {
        if self.is_tree() {
            ObjectType::Tree
        } else if self.0 == Self::GITLINK.0 {
            ObjectType::Commit
        } else {
            ObjectType::Blob
        }
    }

    /// 6-character zero-padded octal string used in cat-file and ls-tree display.
    pub fn display_str(&self) -> String {
        format!("{:06o}", self.0)
    }
}

/// A Blob object containing uninterpreted data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    /// Raw payload bytes.
    pub data: Vec<u8>,
}

impl Blob {
    /// Creates a new Blob from raw data bytes.
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }
}

/// An entry within a Git Tree object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// File mode (e.g. 100644 for regular file, 40000 for subtree).
    pub mode: FileMode,
    /// Filename without slashes.
    pub name: String,
    /// Object ID (SHA-1) of the entry.
    pub id: ObjectId,
}

/// A Tree object representing a directory snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tree {
    /// Sorted list of tree entries.
    pub entries: Vec<TreeEntry>,
}

impl Tree {
    /// Creates a new Tree with entries sorted per Git canonical ordering:
    /// entries are compared byte-by-byte, and directories are compared as if ending with a trailing slash `/`.
    pub fn new(mut entries: Vec<TreeEntry>) -> Self {
        entries.sort_by(|a, b| {
            let a_name = if a.mode.is_tree() {
                format!("{}/", a.name)
            } else {
                a.name.clone()
            };
            let b_name = if b.mode.is_tree() {
                format!("{}/", b.name)
            } else {
                b.name.clone()
            };
            a_name.as_bytes().cmp(b_name.as_bytes())
        });
        Self { entries }
    }
}

/// Person identity signature with timestamp and timezone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// Full name.
    pub name: String,
    /// Email address.
    pub email: String,
    /// Unix timestamp in seconds.
    pub time_seconds: i64,
    /// Timezone offset string formatted as `+0700` or `-0500`.
    pub tz_offset: String,
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} <{}> {} {}",
            self.name, self.email, self.time_seconds, self.tz_offset
        )
    }
}

impl Signature {
    /// Parses a Git signature line e.g. "Name <email> 1234567890 +0000".
    pub fn parse(s: &str) -> Result<Self, crate::error::CoreError> {
        let open_bracket = s
            .find('<')
            .ok_or_else(|| crate::error::CoreError::ParseError {
                object_type: "signature",
                reason: "missing '<'".to_string(),
            })?;
        let close_bracket = s
            .find('>')
            .ok_or_else(|| crate::error::CoreError::ParseError {
                object_type: "signature",
                reason: "missing '>'".to_string(),
            })?;

        let name = s[..open_bracket].trim().to_string();
        let email = s[open_bracket + 1..close_bracket].trim().to_string();
        let rest = s[close_bracket + 1..].trim();
        let mut rest_parts = rest.split_whitespace();
        let time_str = rest_parts
            .next()
            .ok_or_else(|| crate::error::CoreError::ParseError {
                object_type: "signature",
                reason: "missing timestamp".to_string(),
            })?;
        let tz_offset = rest_parts.next().unwrap_or("+0000").to_string();

        let time_seconds: i64 =
            time_str
                .parse()
                .map_err(|_| crate::error::CoreError::ParseError {
                    object_type: "signature",
                    reason: "invalid timestamp number".to_string(),
                })?;

        Ok(Self {
            name,
            email,
            time_seconds,
            tz_offset,
        })
    }
}

/// A Commit object recording history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Root Tree object ID.
    pub tree: ObjectId,
    /// Parent commit object IDs.
    pub parents: Vec<ObjectId>,
    /// Author information.
    pub author: Signature,
    /// Committer information.
    pub committer: Signature,
    /// GPG signature (if any).
    pub gpg_sig: Option<String>,
    /// Commit message.
    pub message: String,
}

/// An annotated Tag object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    /// Target object ID being tagged.
    pub target: ObjectId,
    /// Target object type.
    pub target_type: ObjectType,
    /// Tag name.
    pub name: String,
    /// Tagger information.
    pub tagger: Option<Signature>,
    /// Tag annotation message.
    pub message: String,
}

/// Unified Git object representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Object {
    /// Blob variant.
    Blob(Blob),
    /// Tree variant.
    Tree(Tree),
    /// Commit variant.
    Commit(Commit),
    /// Tag variant.
    Tag(Tag),
}

impl Object {
    /// Returns the object type.
    pub fn object_type(&self) -> ObjectType {
        match self {
            Self::Blob(_) => ObjectType::Blob,
            Self::Tree(_) => ObjectType::Tree,
            Self::Commit(_) => ObjectType::Commit,
            Self::Tag(_) => ObjectType::Tag,
        }
    }

    /// Serializes object content into Git canonical bytes (without the header).
    pub fn serialize_content(&self) -> Vec<u8> {
        match self {
            Self::Blob(blob) => blob.data.clone(),
            Self::Tree(tree) => {
                let mut out = Vec::new();
                for entry in &tree.entries {
                    // Git tree format: "<mode_octal> <name>\0<20-byte-sha1>"
                    out.extend_from_slice(entry.mode.as_octal_str().as_bytes());
                    out.push(b' ');
                    out.extend_from_slice(entry.name.as_bytes());
                    out.push(0);
                    out.extend_from_slice(entry.id.as_bytes());
                }
                out
            }
            Self::Commit(commit) => {
                let mut out = Vec::new();
                out.extend_from_slice(format!("tree {}\n", commit.tree).as_bytes());
                for parent in &commit.parents {
                    out.extend_from_slice(format!("parent {}\n", parent).as_bytes());
                }
                out.extend_from_slice(format!("author {}\n", commit.author).as_bytes());
                out.extend_from_slice(format!("committer {}\n", commit.committer).as_bytes());
                if let Some(ref sig) = commit.gpg_sig {
                    out.extend_from_slice(b"gpgsig ");
                    for line in sig.lines() {
                        out.extend_from_slice(format!(" {}\n", line).as_bytes());
                    }
                }
                out.push(b'\n');
                out.extend_from_slice(commit.message.as_bytes());
                out
            }
            Self::Tag(tag) => {
                let mut out = Vec::new();
                out.extend_from_slice(format!("object {}\n", tag.target).as_bytes());
                out.extend_from_slice(format!("type {}\n", tag.target_type).as_bytes());
                out.extend_from_slice(format!("tag {}\n", tag.name).as_bytes());
                if let Some(ref tagger) = tag.tagger {
                    out.extend_from_slice(format!("tagger {}\n", tagger).as_bytes());
                }
                out.push(b'\n');
                out.extend_from_slice(tag.message.as_bytes());
                out
            }
        }
    }

    /// Formats the complete Git object with header: `"<type> <size>\0<content>"`.
    pub fn serialize_with_header(&self) -> Vec<u8> {
        let content = self.serialize_content();
        let header = format!("{} {}\0", self.object_type().as_str(), content.len());
        let mut out = Vec::with_capacity(header.len() + content.len());
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(&content);
        out
    }

    /// Computes the SHA-1 ObjectId of this object.
    pub fn id(&self) -> ObjectId {
        let serialized = self.serialize_with_header();
        let mut hasher = Sha1::new();
        hasher.update(&serialized);
        let result: [u8; 20] = hasher.finalize().into();
        ObjectId::from_bytes(result)
    }
}

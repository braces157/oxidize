//! Git Index (staging area) DIRC format parser, serializer, and working tree scanner.

use thiserror::Error;

/// Errors arising from index and staging operations.
#[derive(Debug, Error)]
pub enum IndexError {
    /// Invalid header signature (must be DIRC).
    #[error("invalid index signature: expected DIRC")]
    InvalidSignature,

    /// Unsupported index version.
    #[error("unsupported index version: {0}")]
    UnsupportedVersion(u32),

    /// Checksum mismatch at end of index file.
    #[error("index checksum mismatch")]
    ChecksumMismatch,

    /// Parse failure on index entry.
    #[error("failed to parse index entry: {0}")]
    EntryParseError(String),

    /// Standard I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Underlying core error.
    #[error("core error: {0}")]
    Core(#[from] oxidize_core::CoreError),
}

pub mod entry;
pub mod index;
pub mod status;
pub mod tree;

pub use entry::IndexEntry;
pub use index::Index;
pub use status::{
    compute_status, compute_status_with_ignore, flatten_tree, IgnoreFilter, RepoStatus,
    StagedChange, UnstagedChange,
};
pub use tree::write_tree;

//! Typed errors for core Git object operations.

use thiserror::Error;

/// Errors arising from core Git object manipulation and storage.
#[derive(Debug, Error)]
pub enum CoreError {
    /// Invalid object ID format.
    #[error("invalid object ID: {0}")]
    InvalidObjectId(String),

    /// Unknown or unsupported object type.
    #[error("unknown object type: {0}")]
    UnknownObjectType(String),

    /// Object header corrupted or missing NUL separator.
    #[error("corrupted object header")]
    CorruptedHeader,

    /// Object size mismatch between header and payload.
    #[error("object size mismatch: expected {expected}, got {actual}")]
    SizeMismatch {
        /// Expected size declared in header
        expected: usize,
        /// Actual size of decoded content
        actual: usize,
    },

    /// Object parsing error.
    #[error("failed to parse {object_type}: {reason}")]
    ParseError {
        /// Type of object being parsed
        object_type: &'static str,
        /// Detailed reason for failure
        reason: String,
    },

    /// Object was not found in object store.
    #[error("object not found: {0}")]
    ObjectNotFound(String),

    /// Ambiguous short SHA-1 prefix matched multiple objects.
    #[error("short SHA-1 {0} is ambiguous")]
    AmbiguousPrefix(String),

    /// Not a Git repository.
    #[error("not a git repository (or any of the parent directories): .git")]
    RepoNotFound,

    /// Standard I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

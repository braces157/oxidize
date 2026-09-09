//! Git Packfile format, delta compression/decompression, and .idx v2 indexing.

use thiserror::Error;

/// Errors arising from packfile and index operations.
#[derive(Debug, Error)]
pub enum PackError {
    /// Invalid packfile signature.
    #[error("invalid pack signature: expected PACK")]
    InvalidPackSignature,

    /// Invalid index signature.
    #[error("invalid index signature")]
    InvalidIndexSignature,

    /// Unsupported pack or index version.
    #[error("unsupported version: {0}")]
    UnsupportedVersion(u32),

    /// Checksum verification failure.
    #[error("checksum verification failed")]
    ChecksumMismatch,

    /// Delta resolution error.
    #[error("failed to apply delta: {0}")]
    DeltaError(String),

    /// Standard I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Underlying core error.
    #[error("core error: {0}")]
    Core(#[from] oxidize_core::CoreError),
}

pub mod delta;
pub mod index;
pub mod packfile;
pub mod store;

pub use delta::{apply_delta, create_delta};
pub use index::{IndexedObject, PackIndex};
pub use packfile::{
    index_packfile, read_pack_object_at, unpack_packfile, write_pack, RawPackObject,
};
pub use store::{PackHandle, PackStore, RepoObjectStore};

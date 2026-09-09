//! Git references: HEAD, branches, tags, packed-refs, reflogs, and rev-parse.

use thiserror::Error;

/// Errors arising from reference operations.
#[derive(Debug, Error)]
pub enum RefError {
    /// Reference not found.
    #[error("reference not found: {0}")]
    NotFound(String),

    /// Invalid reference name.
    #[error("invalid reference name: {0}")]
    InvalidName(String),

    /// Revision parse error.
    #[error("failed to resolve revision: {0}")]
    RevParseError(String),

    /// Standard I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Underlying core error.
    #[error("core error: {0}")]
    Core(#[from] oxidize_core::CoreError),
}

pub mod ref_store;
pub mod signature;

pub use ref_store::{RefStore, ReflogEntry};
pub use signature::get_default_signature;

//! Myers diff algorithm, unified diff formatter, and 3-way merge engine.

use thiserror::Error;

/// Errors arising from diff and merge computations.
#[derive(Debug, Error)]
pub enum DiffError {
    /// Diff parsing or computation error.
    #[error("diff error: {0}")]
    Computation(String),

    /// Merge conflict detected.
    #[error("merge conflict in {path}")]
    Conflict {
        /// File path with merge conflict
        path: String,
    },

    /// Standard I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Underlying core error.
    #[error("core error: {0}")]
    Core(#[from] oxidize_core::CoreError),
}

pub mod myers;
pub mod unified;

pub use myers::{myers_diff, DiffOp};
pub use unified::{format_unified_diff, Hunk};

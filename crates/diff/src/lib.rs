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

pub mod merge;
pub mod myers;
pub mod patch;
pub mod unified;

pub use merge::{is_binary_content, three_way_merge, MergeResult};
pub use myers::{myers_diff, DiffOp};
pub use patch::{
    apply_hunk_forward, apply_hunk_reverse, compute_structured_diff, reconstruct_exact_lines,
    split_exact_lines, CustomPatchBasket, CustomPatchHunk, ExactLine, PatchLine, PatchLineKind,
    StructuredHunk,
};
pub use unified::{format_unified_diff, Hunk};

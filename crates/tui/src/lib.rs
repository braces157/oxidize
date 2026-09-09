//! Ratatui-based visual dashboard for commit graph, status, and diff viewer.

use thiserror::Error;

/// Errors arising from TUI rendering and terminal interaction.
#[derive(Debug, Error)]
pub enum TuiError {
    /// Terminal initialization error.
    #[error("terminal error: {0}")]
    Terminal(String),

    /// Standard I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Underlying core error.
    #[error("core error: {0}")]
    Core(#[from] oxidize_core::CoreError),
}

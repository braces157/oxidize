//! Git configuration (.git/config, ~/.gitconfig) INI parser and .gitignore pattern matching.

use thiserror::Error;

/// Errors arising from configuration and pattern matching.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Configuration syntax error.
    #[error("config syntax error: {0}")]
    SyntaxError(String),

    /// Missing required configuration key.
    #[error("missing config key: {0}")]
    MissingKey(String),

    /// Standard I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Underlying core error.
    #[error("core error: {0}")]
    Core(#[from] oxidize_core::CoreError),
}

pub mod ignore;
pub mod ini;
pub use ignore::{GitIgnore, IgnorePattern};
pub use ini::{ConfigSectionKey, GitConfig};

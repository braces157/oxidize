//! Git network protocols: pkt-line framing, protocol v2 negotiation, and smart HTTP client.

use thiserror::Error;

/// Errors arising from network transport operations.
#[derive(Debug, Error)]
pub enum TransportError {
    /// Pkt-line framing error.
    #[error("pkt-line framing error: {0}")]
    PktLineError(String),

    /// HTTP network error.
    #[error("HTTP error: {0}")]
    Http(String),

    /// Protocol v2 negotiation error.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// Remote ref resolution error.
    #[error("remote ref error: {0}")]
    RefError(String),

    /// Standard I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Underlying packfile error.
    #[error("pack error: {0}")]
    Pack(#[from] oxidize_pack::PackError),

    /// Underlying core error.
    #[error("core error: {0}")]
    Core(#[from] oxidize_core::CoreError),
}

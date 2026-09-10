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

pub mod client;
pub mod local;
pub mod pkt_line;
pub mod protocol;
pub mod ssh;

pub use client::SmartHttpClient;
pub use local::{
    discover_local_refs, fetch_local_pack, fetch_local_pack_negotiated, resolve_local_path,
};
pub use pkt_line::{
    encode_flush, encode_pkt_line, encode_pkt_line_str, parse_pkt_line, read_next_pkt_line,
    read_pkt_lines, read_pkt_lines_until_flush, read_single_pkt_line, PktLine, SidebandDemuxer,
};
pub use protocol::{
    build_receive_pack_request, build_receive_pack_request_with_caps, build_upload_pack_request,
    build_upload_pack_request_with_caps, negotiate_receive_pack_capabilities,
    negotiate_upload_pack_capabilities, parse_push_report, parse_ref_advertisement, PushRefStatus,
    PushReport, RemoteRef,
};
pub use ssh::{
    find_ssh_binary, is_ssh_url, parse_command_tokens, parse_ssh_url, sq_quote,
    validate_ssh_endpoint, SshClient, SshEndpoint, SshInvocation,
};

//! Smart HTTP client implementation using `ureq`.

use crate::pkt_line::{read_pkt_lines, SidebandDemuxer};
use crate::protocol::{
    build_receive_pack_request_with_caps, build_upload_pack_request_with_caps,
    parse_ref_advertisement, RemoteRef, UploadPackDiscovery,
};
use crate::TransportError;
use oxidize_core::id::ObjectId;

/// Smart HTTP Git client.
pub struct SmartHttpClient {
    user_agent: String,
}

impl Default for SmartHttpClient {
    fn default() -> Self {
        Self {
            user_agent: format!("git/2.0 (oxidize/{})", env!("CARGO_PKG_VERSION")),
        }
    }
}

impl SmartHttpClient {
    /// Creates a new `SmartHttpClient`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Discovers remote references for `git-upload-pack`.
    /// Returns `(refs, capabilities, symref_head)`.
    pub fn discover_upload_pack(&self, url: &str) -> Result<UploadPackDiscovery, TransportError> {
        let endpoint = format!(
            "{}/info/refs?service=git-upload-pack",
            url.trim_end_matches('/')
        );
        let response = ureq::get(&endpoint)
            .set("User-Agent", &self.user_agent)
            .set("Git-Protocol", "version=1")
            .call()
            .map_err(|e| TransportError::Http(format!("GET {}: {}", endpoint, e)))?;

        let lines = read_pkt_lines(response.into_reader())?;
        parse_ref_advertisement(&lines)
    }

    /// Fetches a packfile for the given `wants` and `haves` from the remote server.
    /// Returns `(pack_bytes, progress_messages)`.
    pub fn fetch_pack(
        &self,
        url: &str,
        wants: &[ObjectId],
        haves: &[ObjectId],
    ) -> Result<(Vec<u8>, Vec<String>), TransportError> {
        self.fetch_pack_with_caps(url, wants, haves, &[])
    }

    /// Fetches a packfile using negotiated server capabilities.
    pub fn fetch_pack_with_caps(
        &self,
        url: &str,
        wants: &[ObjectId],
        haves: &[ObjectId],
        server_caps: &[String],
    ) -> Result<(Vec<u8>, Vec<String>), TransportError> {
        let endpoint = format!("{}/git-upload-pack", url.trim_end_matches('/'));
        let body = build_upload_pack_request_with_caps(wants, haves, server_caps);

        let response = ureq::post(&endpoint)
            .set("User-Agent", &self.user_agent)
            .set("Content-Type", "application/x-git-upload-pack-request")
            .set("Accept", "application/x-git-upload-pack-result")
            .send_bytes(&body)
            .map_err(|e| TransportError::Http(format!("POST {}: {}", endpoint, e)))?;

        let demux = SidebandDemuxer::read_stream(response.into_reader())?;

        if demux.pack_data.is_empty() {
            return Err(TransportError::Protocol(
                "no packfile data received from remote".to_string(),
            ));
        }

        Ok((demux.pack_data, demux.progress))
    }

    /// Discovers remote references for `git-receive-pack` before pushing.
    pub fn discover_receive_pack(
        &self,
        url: &str,
    ) -> Result<(Vec<RemoteRef>, Vec<String>), TransportError> {
        let endpoint = format!(
            "{}/info/refs?service=git-receive-pack",
            url.trim_end_matches('/')
        );
        let response = ureq::get(&endpoint)
            .set("User-Agent", &self.user_agent)
            .call()
            .map_err(|e| TransportError::Http(format!("GET {}: {}", endpoint, e)))?;

        let lines = read_pkt_lines(response.into_reader())?;
        let (refs, caps, _) = parse_ref_advertisement(&lines)?;
        Ok((refs, caps))
    }

    /// Pushes local commits to the remote server using `git-receive-pack`.
    pub fn push_pack(
        &self,
        url: &str,
        updates: &[(&ObjectId, &ObjectId, &str)],
        pack_data: &[u8],
    ) -> Result<crate::protocol::PushReport, TransportError> {
        self.push_pack_with_caps(url, updates, pack_data, &[])
    }

    /// Pushes local commits to the remote server using `git-receive-pack` and negotiated capabilities.
    pub fn push_pack_with_caps(
        &self,
        url: &str,
        updates: &[(&ObjectId, &ObjectId, &str)],
        pack_data: &[u8],
        server_caps: &[String],
    ) -> Result<crate::protocol::PushReport, TransportError> {
        let endpoint = format!("{}/git-receive-pack", url.trim_end_matches('/'));
        let body = build_receive_pack_request_with_caps(updates, pack_data, server_caps);

        let response = ureq::post(&endpoint)
            .set("User-Agent", &self.user_agent)
            .set("Content-Type", "application/x-git-receive-pack-request")
            .send_bytes(&body)
            .map_err(|e| TransportError::Http(format!("POST {}: {}", endpoint, e)))?;

        let lines = read_pkt_lines(response.into_reader())?;
        crate::protocol::parse_push_report(&lines)
    }
}

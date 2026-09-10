//! Git protocol negotiation, reference advertisement parser, and request builder.

use crate::pkt_line::{encode_flush, encode_pkt_line, encode_pkt_line_str, PktLine};
use crate::TransportError;
use oxidize_core::id::ObjectId;

/// A remote reference advertised by the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRef {
    /// SHA-1 object ID pointed to by this reference.
    pub oid: ObjectId,
    /// Reference name (e.g. `HEAD`, `refs/heads/main`, `refs/tags/v1.0`).
    pub name: String,
}

/// Discovered upload pack metadata: `(references, server_capabilities, default_symref_head)`.
pub type UploadPackDiscovery = (Vec<RemoteRef>, Vec<String>, Option<String>);

/// Parses the reference advertisement from a Git smart HTTP server.
/// Returns `(refs, capabilities, symref_head)`.
pub fn parse_ref_advertisement(lines: &[PktLine]) -> Result<UploadPackDiscovery, TransportError> {
    let mut refs = Vec::new();
    let mut capabilities = Vec::new();
    let mut symref_head = None;

    for line in lines {
        let text = match line.to_text() {
            Some(t) => t,
            None => continue,
        };

        // Skip service header comment e.g. "# service=git-upload-pack"
        if text.starts_with('#') {
            continue;
        }

        // Protocol v2 version indicator
        if text.starts_with("version 2") {
            capabilities.push("version 2".to_string());
            continue;
        }

        // Ref line format: "<oid> <name>\0<capabilities>" (first line) or "<oid> <name>"
        let (ref_part, caps_part) = if let Some(idx) = text.find('\0') {
            (&text[..idx], Some(&text[idx + 1..]))
        } else {
            (text, None)
        };

        if let Some(caps_str) = caps_part {
            for cap in caps_str.split_whitespace() {
                capabilities.push(cap.to_string());
                if let Some(target) = cap.strip_prefix("symref=HEAD:") {
                    symref_head = Some(target.to_string());
                }
            }
        }

        let mut parts = ref_part.split_whitespace();
        let oid_str = match parts.next() {
            Some(o) => o,
            None => continue,
        };
        let ref_name = match parts.next() {
            Some(n) => n,
            None => continue,
        };

        if let Ok(oid) = oid_str.parse::<ObjectId>() {
            refs.push(RemoteRef {
                oid,
                name: ref_name.to_string(),
            });
        }
    }

    if capabilities.iter().any(|c| c == "version 2") && refs.is_empty() {
        return Err(TransportError::Protocol(
            "unsupported Git protocol version 2; only Git protocol v1 is supported".to_string(),
        ));
    }

    Ok((refs, capabilities, symref_head))
}

/// Negotiates client capabilities for upload-pack based on server advertisement.
pub fn negotiate_upload_pack_capabilities(server_caps: &[String]) -> Vec<String> {
    let mut chosen = Vec::new();
    if server_caps.iter().any(|c| c == "side-band-64k") {
        chosen.push("side-band-64k".to_string());
    } else if server_caps.iter().any(|c| c == "side-band") {
        chosen.push("side-band".to_string());
    }

    if server_caps.iter().any(|c| c == "multi_ack_detailed") {
        chosen.push("multi_ack_detailed".to_string());
    } else if server_caps.iter().any(|c| c == "multi_ack") {
        chosen.push("multi_ack".to_string());
    }

    if server_caps.iter().any(|c| c == "ofs-delta") {
        chosen.push("ofs-delta".to_string());
    }

    chosen.push(format!("agent=ox/{}", env!("CARGO_PKG_VERSION")));
    chosen
}

/// Negotiates client capabilities for receive-pack based on server advertisement.
pub fn negotiate_receive_pack_capabilities(server_caps: &[String]) -> Vec<String> {
    let mut chosen = Vec::new();
    if server_caps.iter().any(|c| c == "report-status") {
        chosen.push("report-status".to_string());
    }
    if server_caps.iter().any(|c| c == "side-band-64k") {
        chosen.push("side-band-64k".to_string());
    } else if server_caps.iter().any(|c| c == "side-band") {
        chosen.push("side-band".to_string());
    }
    if server_caps.iter().any(|c| c == "ofs-delta") {
        chosen.push("ofs-delta".to_string());
    }
    chosen.push(format!("agent=ox/{}", env!("CARGO_PKG_VERSION")));
    chosen
}

/// Builds the `upload-pack` negotiation request body for fetching or cloning with negotiated capabilities.
pub fn build_upload_pack_request_with_caps(
    wants: &[ObjectId],
    haves: &[ObjectId],
    server_caps: &[String],
) -> Vec<u8> {
    let mut buf = Vec::new();
    if wants.is_empty() {
        return buf;
    }

    let caps = negotiate_upload_pack_capabilities(server_caps);
    let caps_str = caps.join(" ");

    let first_want = format!("want {} {}\n", wants[0], caps_str);
    buf.extend_from_slice(&encode_pkt_line(first_want.as_bytes()));

    for want in &wants[1..] {
        let line = format!("want {}\n", want);
        buf.extend_from_slice(&encode_pkt_line(line.as_bytes()));
    }

    // Flush after wants
    buf.extend_from_slice(&encode_flush());

    for have in haves {
        let line = format!("have {}\n", have);
        buf.extend_from_slice(&encode_pkt_line(line.as_bytes()));
    }

    buf.extend_from_slice(&encode_pkt_line_str("done\n"));
    buf
}

/// Builds the `upload-pack` negotiation request body for fetching or cloning.
pub fn build_upload_pack_request(wants: &[ObjectId], haves: &[ObjectId]) -> Vec<u8> {
    build_upload_pack_request_with_caps(
        wants,
        haves,
        &[
            "multi_ack_detailed".to_string(),
            "side-band-64k".to_string(),
            "ofs-delta".to_string(),
        ],
    )
}

/// Builds the `receive-pack` push request body with command lines and raw packfile payload with negotiated capabilities.
pub fn build_receive_pack_request_with_caps(
    updates: &[(&ObjectId, &ObjectId, &str)],
    pack_data: &[u8],
    server_caps: &[String],
) -> Vec<u8> {
    let mut buf = Vec::new();
    if updates.is_empty() {
        return buf;
    }

    let caps = negotiate_receive_pack_capabilities(server_caps);
    let caps_str = caps.join(" ");

    let (old_oid, new_oid, ref_name) = updates[0];
    let first_line = format!("{} {} {}\0{}\n", old_oid, new_oid, ref_name, caps_str);
    buf.extend_from_slice(&encode_pkt_line(first_line.as_bytes()));

    for (old_oid, new_oid, ref_name) in &updates[1..] {
        let line = format!("{} {} {}\n", old_oid, new_oid, ref_name);
        buf.extend_from_slice(&encode_pkt_line(line.as_bytes()));
    }

    buf.extend_from_slice(&encode_flush());
    buf.extend_from_slice(pack_data);
    buf
}

/// Builds the `receive-pack` push request body with command lines and raw packfile payload.
pub fn build_receive_pack_request(
    updates: &[(&ObjectId, &ObjectId, &str)],
    pack_data: &[u8],
) -> Vec<u8> {
    build_receive_pack_request_with_caps(
        updates,
        pack_data,
        &["report-status".to_string(), "ofs-delta".to_string()],
    )
}

/// Result for an individual ref update in a push report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushRefStatus {
    /// Remote reference name (e.g. `refs/heads/main`).
    pub ref_name: String,
    /// Whether the ref was successfully updated.
    pub ok: bool,
    /// Server error or rejection reason if not ok.
    pub message: Option<String>,
}

/// Status report returned by `receive-pack` after a push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushReport {
    /// Whether the remote unpacked the packfile successfully.
    pub unpack_ok: bool,
    /// Error message if unpacking failed.
    pub unpack_error: Option<String>,
    /// Status for each ref requested in the push.
    pub ref_statuses: Vec<PushRefStatus>,
}

impl PushReport {
    /// Returns true if unpacking succeeded and all ref updates succeeded.
    pub fn is_success(&self) -> bool {
        self.unpack_ok && self.ref_statuses.iter().all(|r| r.ok)
    }

    /// Formats a human-readable summary of the report.
    pub fn display_summary(&self) -> String {
        let mut out = Vec::new();
        if !self.unpack_ok {
            out.push(format!(
                "remote: unpack error: {}",
                self.unpack_error.as_deref().unwrap_or("unknown error")
            ));
        }
        for r in &self.ref_statuses {
            if r.ok {
                out.push(format!("remote: ok {}", r.ref_name));
            } else {
                out.push(format!(
                    "remote: rejected: {} ({})",
                    r.ref_name,
                    r.message.as_deref().unwrap_or("error")
                ));
            }
        }
        out.join("\n")
    }
}

/// Parses the pkt-line stream of a push report (`unpack ok|error`, `ok|ng <ref>`).
pub fn parse_push_report(lines: &[PktLine]) -> Result<PushReport, TransportError> {
    let mut unpack_ok = false;
    let mut unpack_error = None;
    let mut ref_statuses = Vec::new();
    let mut saw_report = false;

    for line in lines {
        let text = match line.to_text() {
            Some(t) => t.trim(),
            None => continue,
        };
        if text.is_empty() {
            continue;
        }

        if let Some(status) = text.strip_prefix("unpack ") {
            saw_report = true;
            if status == "ok" {
                unpack_ok = true;
            } else {
                unpack_ok = false;
                unpack_error = Some(status.to_string());
            }
        } else if let Some(ref_name) = text.strip_prefix("ok ") {
            saw_report = true;
            ref_statuses.push(PushRefStatus {
                ref_name: ref_name.trim().to_string(),
                ok: true,
                message: None,
            });
        } else if let Some(rest) = text.strip_prefix("ng ") {
            saw_report = true;
            let mut parts = rest.splitn(2, ' ');
            let ref_name = parts.next().unwrap_or("").trim().to_string();
            let msg = parts.next().map(|m| m.trim().to_string());
            ref_statuses.push(PushRefStatus {
                ref_name,
                ok: false,
                message: msg,
            });
        }
    }

    if !saw_report {
        return Ok(PushReport {
            unpack_ok: true,
            unpack_error: None,
            ref_statuses: Vec::new(),
        });
    }

    Ok(PushReport {
        unpack_ok,
        unpack_error,
        ref_statuses,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ref_advertisement() {
        let text1 = "1111111111111111111111111111111111111111 HEAD\0multi_ack side-band-64k symref=HEAD:refs/heads/main\n";
        let text2 = "1111111111111111111111111111111111111111 refs/heads/main\n";
        let text3 = "2222222222222222222222222222222222222222 refs/tags/v1.0\n";

        let lines = vec![
            PktLine::Data(b"# service=git-upload-pack\n".to_vec()),
            PktLine::Flush,
            PktLine::Data(text1.as_bytes().to_vec()),
            PktLine::Data(text2.as_bytes().to_vec()),
            PktLine::Data(text3.as_bytes().to_vec()),
            PktLine::Flush,
        ];

        let (refs, caps, symref) = parse_ref_advertisement(&lines).unwrap();
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].name, "HEAD");
        assert_eq!(refs[1].name, "refs/heads/main");
        assert_eq!(refs[2].name, "refs/tags/v1.0");

        assert!(caps.contains(&"side-band-64k".to_string()));
        assert_eq!(symref, Some("refs/heads/main".to_string()));
    }

    #[test]
    fn test_build_upload_pack_request() {
        let want: ObjectId = "1111111111111111111111111111111111111111".parse().unwrap();
        let have: ObjectId = "2222222222222222222222222222222222222222".parse().unwrap();

        let req = build_upload_pack_request(&[want], &[have]);
        let s = String::from_utf8_lossy(&req);
        assert!(s.contains("want 1111111111111111111111111111111111111111"));
        assert!(s.contains("side-band-64k"));
        assert!(s.contains("0000"));
        assert!(s.contains("have 2222222222222222222222222222222222222222"));
        assert!(s.contains("done"));
    }

    #[test]
    fn test_negotiate_capabilities() {
        let server_caps = vec![
            "multi_ack".to_string(),
            "side-band-64k".to_string(),
            "ofs-delta".to_string(),
        ];
        let negotiated = negotiate_upload_pack_capabilities(&server_caps);
        assert!(negotiated.contains(&"side-band-64k".to_string()));
        assert!(negotiated.contains(&"multi_ack".to_string()));
        assert!(!negotiated.contains(&"multi_ack_detailed".to_string()));
        assert!(negotiated.contains(&"ofs-delta".to_string()));
        assert!(negotiated.iter().any(|c| c.starts_with("agent=")));

        let recv_caps = vec!["report-status".to_string(), "side-band".to_string()];
        let negotiated_recv = negotiate_receive_pack_capabilities(&recv_caps);
        assert!(negotiated_recv.contains(&"report-status".to_string()));
        assert!(negotiated_recv.contains(&"side-band".to_string()));
        assert!(!negotiated_recv.contains(&"side-band-64k".to_string()));
    }

    #[test]
    fn test_parse_push_report_success_and_failures() {
        let success_lines = vec![
            PktLine::Data(b"unpack ok\n".to_vec()),
            PktLine::Data(b"ok refs/heads/main\n".to_vec()),
            PktLine::Flush,
        ];
        let report = parse_push_report(&success_lines).unwrap();
        assert!(report.is_success());
        assert_eq!(report.ref_statuses.len(), 1);
        assert_eq!(report.ref_statuses[0].ref_name, "refs/heads/main");
        assert!(report.ref_statuses[0].ok);

        let rejected_lines = vec![
            PktLine::Data(b"unpack ok\n".to_vec()),
            PktLine::Data(b"ng refs/heads/main [rejected - non-fast-forward]\n".to_vec()),
            PktLine::Flush,
        ];
        let rej_report = parse_push_report(&rejected_lines).unwrap();
        assert!(!rej_report.is_success());
        assert!(!rej_report.ref_statuses[0].ok);
        assert!(rej_report.ref_statuses[0]
            .message
            .as_deref()
            .unwrap()
            .contains("non-fast-forward"));

        let unpack_err_lines = vec![
            PktLine::Data(b"unpack error: index-pack failed\n".to_vec()),
            PktLine::Data(b"ng refs/heads/main unpack failed\n".to_vec()),
            PktLine::Flush,
        ];
        let err_report = parse_push_report(&unpack_err_lines).unwrap();
        assert!(!err_report.is_success());
        assert!(!err_report.unpack_ok);
        assert!(err_report
            .unpack_error
            .as_deref()
            .unwrap()
            .contains("index-pack failed"));
    }

    #[test]
    fn test_protocol_v2_rejection() {
        let v2_lines = vec![PktLine::Data(b"version 2\n".to_vec()), PktLine::Flush];
        let res = parse_ref_advertisement(&v2_lines);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("protocol version 2"));
    }
}

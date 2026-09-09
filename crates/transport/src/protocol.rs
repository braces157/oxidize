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

    Ok((refs, capabilities, symref_head))
}

/// Builds the `upload-pack` negotiation request body for fetching or cloning.
pub fn build_upload_pack_request(wants: &[ObjectId], haves: &[ObjectId]) -> Vec<u8> {
    let mut buf = Vec::new();
    if wants.is_empty() {
        return buf;
    }

    // First want carries client capabilities
    let first_want = format!(
        "want {} multi_ack_detailed side-band-64k ofs-delta agent=ox/0.1.0\n",
        wants[0]
    );
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

/// Builds the `receive-pack` push request body with command lines and raw packfile payload.
pub fn build_receive_pack_request(
    updates: &[(&ObjectId, &ObjectId, &str)],
    pack_data: &[u8],
) -> Vec<u8> {
    let mut buf = Vec::new();
    if updates.is_empty() {
        return buf;
    }

    let (old_oid, new_oid, ref_name) = updates[0];
    let first_line = format!(
        "{} {} {}\0report-status ofs-delta agent=ox/0.1.0\n",
        old_oid, new_oid, ref_name
    );
    buf.extend_from_slice(&encode_pkt_line(first_line.as_bytes()));

    for (old_oid, new_oid, ref_name) in &updates[1..] {
        let line = format!("{} {} {}\n", old_oid, new_oid, ref_name);
        buf.extend_from_slice(&encode_pkt_line(line.as_bytes()));
    }

    buf.extend_from_slice(&encode_flush());
    buf.extend_from_slice(pack_data);
    buf
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
}

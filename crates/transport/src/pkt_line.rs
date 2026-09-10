//! Git pkt-line framing and sideband demultiplexing.

use crate::TransportError;
use std::io::Read;

/// A parsed packet line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PktLine {
    /// Data packet with binary payload.
    Data(Vec<u8>),
    /// Flush packet (`0000`).
    Flush,
    /// Delimiter packet (`0001`).
    Delim,
    /// Response end packet (`0002`).
    ResponseEnd,
}

impl PktLine {
    /// Returns the payload as a string slice if valid UTF-8.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Data(bytes) => std::str::from_utf8(bytes).ok(),
            _ => None,
        }
    }

    /// Returns the trimmed text content of a data packet line.
    pub fn to_text(&self) -> Option<&str> {
        self.as_str().map(|s| s.trim_end_matches(['\r', '\n']))
    }

    /// Checks if this is a Flush packet (`0000`).
    pub fn is_flush(&self) -> bool {
        matches!(self, Self::Flush)
    }
}

/// Formats a byte slice as a Git pkt-line with a 4-byte hex prefix.
pub fn encode_pkt_line(data: &[u8]) -> Vec<u8> {
    let total_len = data.len() + 4;
    let mut buf = format!("{:04x}", total_len).into_bytes();
    buf.extend_from_slice(data);
    buf
}

/// Formats a string as a Git pkt-line, ensuring trailing newline.
pub fn encode_pkt_line_str(s: &str) -> Vec<u8> {
    let mut data = s.as_bytes().to_vec();
    if !data.ends_with(b"\n") {
        data.push(b'\n');
    }
    encode_pkt_line(&data)
}

/// Returns the 4-byte flush packet `0000`.
pub fn encode_flush() -> Vec<u8> {
    b"0000".to_vec()
}

/// Returns the 4-byte delimiter packet `0001`.
pub fn encode_delim() -> Vec<u8> {
    b"0001".to_vec()
}

/// Parses a single pkt-line from a byte slice.
/// Returns `Ok(Some((pkt_line, bytes_consumed)))` or `Ok(None)` if more bytes are needed.
pub fn parse_pkt_line(input: &[u8]) -> Result<Option<(PktLine, usize)>, TransportError> {
    if input.len() < 4 {
        return Ok(None);
    }

    let hex_str = std::str::from_utf8(&input[..4])
        .map_err(|_| TransportError::PktLineError("non-utf8 length prefix".to_string()))?;

    if hex_str == "0000" {
        return Ok(Some((PktLine::Flush, 4)));
    }
    if hex_str == "0001" {
        return Ok(Some((PktLine::Delim, 4)));
    }
    if hex_str == "0002" {
        return Ok(Some((PktLine::ResponseEnd, 4)));
    }

    let len = usize::from_str_radix(hex_str, 16).map_err(|e| {
        TransportError::PktLineError(format!("invalid hex length '{}': {}", hex_str, e))
    })?;

    if len < 4 {
        return Err(TransportError::PktLineError(format!(
            "invalid packet length {}",
            len
        )));
    }

    if input.len() < len {
        return Ok(None);
    }

    let payload = input[4..len].to_vec();
    Ok(Some((PktLine::Data(payload), len)))
}

/// Reads a single packet line directly from a reader.
pub fn read_single_pkt_line<R: Read>(reader: &mut R) -> Result<PktLine, TransportError> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let hex_str = std::str::from_utf8(&len_buf)
        .map_err(|e| TransportError::PktLineError(format!("invalid hex length utf8: {}", e)))?;

    if hex_str == "0000" {
        return Ok(PktLine::Flush);
    }
    if hex_str == "0001" {
        return Ok(PktLine::Delim);
    }
    if hex_str == "0002" {
        return Ok(PktLine::ResponseEnd);
    }

    let len = usize::from_str_radix(hex_str, 16).map_err(|e| {
        TransportError::PktLineError(format!("invalid hex length '{}': {}", hex_str, e))
    })?;

    if len < 4 {
        return Err(TransportError::PktLineError(format!(
            "invalid packet length {}",
            len
        )));
    }

    let mut payload = vec![0u8; len - 4];
    reader.read_exact(&mut payload)?;
    Ok(PktLine::Data(payload))
}

/// Reads all packet lines until EOF from a reader.
pub fn read_pkt_lines(mut reader: impl Read) -> Result<Vec<PktLine>, TransportError> {
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;

    let mut lines = Vec::new();
    let mut slice = &buf[..];

    while !slice.is_empty() {
        match parse_pkt_line(slice)? {
            Some((line, consumed)) => {
                lines.push(line);
                slice = &slice[consumed..];
            }
            None => {
                if slice.len() < 4 {
                    break;
                }
                return Err(TransportError::PktLineError(
                    "truncated pkt-line".to_string(),
                ));
            }
        }
    }

    Ok(lines)
}

/// Sideband channels demultiplexed from a side-band stream.
#[derive(Debug, Default, Clone)]
pub struct SidebandDemuxer {
    /// Channel 1: Binary packfile data.
    pub pack_data: Vec<u8>,
    /// Channel 2: Progress messages.
    pub progress: Vec<String>,
    /// Channel 3: Error messages.
    pub errors: Vec<String>,
}

impl SidebandDemuxer {
    /// Demultiplexes a list of pkt-lines according to side-band-64k rules.
    pub fn from_lines(lines: &[PktLine]) -> Result<Self, TransportError> {
        let mut demux = Self::default();

        for line in lines {
            if let PktLine::Data(ref bytes) = line {
                if bytes.is_empty() {
                    continue;
                }
                let band = bytes[0];
                let payload = &bytes[1..];
                match band {
                    1 => {
                        demux.pack_data.extend_from_slice(payload);
                    }
                    2 => {
                        let msg = String::from_utf8_lossy(payload).to_string();
                        demux.progress.push(msg);
                    }
                    3 => {
                        let err = String::from_utf8_lossy(payload).to_string();
                        demux.errors.push(err);
                    }
                    _ => {
                        // Fallback for non-sideband data: treat entire payload as pack data
                        demux.pack_data.extend_from_slice(bytes);
                    }
                }
            }
        }

        if !demux.errors.is_empty() {
            return Err(TransportError::Protocol(demux.errors.join("; ")));
        }

        Ok(demux)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pkt_line_encode_decode() {
        let text = "want 1111222233334444555566667777888899990000\n";
        let encoded = encode_pkt_line(text.as_bytes());
        assert_eq!(&encoded[..4], format!("{:04x}", text.len() + 4).as_bytes());

        let (parsed, consumed) = parse_pkt_line(&encoded).unwrap().unwrap();
        assert_eq!(consumed, encoded.len());
        match parsed {
            PktLine::Data(bytes) => assert_eq!(bytes, text.as_bytes()),
            _ => panic!("expected data pkt"),
        }
    }

    #[test]
    fn test_special_packets() {
        assert_eq!(parse_pkt_line(b"0000").unwrap(), Some((PktLine::Flush, 4)));
        assert_eq!(parse_pkt_line(b"0001").unwrap(), Some((PktLine::Delim, 4)));
        assert_eq!(
            parse_pkt_line(b"0002").unwrap(),
            Some((PktLine::ResponseEnd, 4))
        );
    }

    #[test]
    fn test_sideband_demux() {
        let mut lines = Vec::new();
        // Band 1: pack chunk
        let mut b1 = vec![1u8];
        b1.extend_from_slice(b"PACKDATA");
        lines.push(PktLine::Data(b1));

        // Band 2: progress
        let mut b2 = vec![2u8];
        b2.extend_from_slice(b"Counting objects: 100%\n");
        lines.push(PktLine::Data(b2));

        let demux = SidebandDemuxer::from_lines(&lines).unwrap();
        assert_eq!(demux.pack_data, b"PACKDATA");
        assert_eq!(demux.progress.len(), 1);
        assert!(demux.progress[0].contains("Counting objects"));
    }
}

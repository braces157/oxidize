//! Three-way line-based merge engine with canonical Git conflict markers and terminator preservation.

use crate::myers::{myers_diff, DiffOp};

/// Result of a 3-way file merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeResult {
    /// Merged file content (including conflict markers if conflicted).
    pub content: String,
    /// Indicates whether one or more conflicts were encountered.
    pub has_conflicts: bool,
}

/// Checks if byte data appears to be binary using Git's NUL-byte heuristic in the first 8000 bytes.
pub fn is_binary_content(data: &[u8]) -> bool {
    let limit = data.len().min(8000);
    data[..limit].contains(&0)
}

/// Splits a string into lines while preserving original line terminators (`\r\n` or `\n`).
pub fn split_lines_with_terminator(s: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\n' {
            lines.push(&s[start..=i]);
            start = i + 1;
        } else if bytes[i] == b'\r' && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
            lines.push(&s[start..=i + 1]);
            i += 1;
            start = i + 1;
        }
        i += 1;
    }
    if start < s.len() {
        lines.push(&s[start..]);
    }
    lines
}

/// Performs a 3-way merge of text content from common ancestor `base`, `ours`, and `theirs`.
/// Preserves CRLF and trailing newline semantics.
pub fn three_way_merge(
    base: &str,
    ours: &str,
    theirs: &str,
    our_label: &str,
    their_label: &str,
) -> MergeResult {
    // Fast path: identical content
    if ours == theirs {
        return MergeResult {
            content: ours.to_string(),
            has_conflicts: false,
        };
    }

    // Fast path: changed only in theirs
    if ours == base {
        return MergeResult {
            content: theirs.to_string(),
            has_conflicts: false,
        };
    }

    // Fast path: changed only in ours
    if theirs == base {
        return MergeResult {
            content: ours.to_string(),
            has_conflicts: false,
        };
    }

    // Line-level 3-way merge preserving original line terminators
    let base_lines = split_lines_with_terminator(base);
    let our_lines = split_lines_with_terminator(ours);
    let their_lines = split_lines_with_terminator(theirs);

    let diff_ours = myers_diff(&base_lines, &our_lines);
    let diff_theirs = myers_diff(&base_lines, &their_lines);

    // Group diff ops into base-indexed chunks
    let our_chunks = extract_base_chunks(&diff_ours);
    let their_chunks = extract_base_chunks(&diff_theirs);

    let nl = if ours.contains("\r\n") || theirs.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };

    let mut out = String::new();
    let mut has_conflicts = false;

    let mut base_idx = 0;
    while base_idx <= base_lines.len() {
        let our_c = our_chunks.get(&base_idx);
        let their_c = their_chunks.get(&base_idx);

        match (our_c, their_c) {
            (None, None) => {
                if base_idx < base_lines.len() {
                    out.push_str(base_lines[base_idx]);
                }
            }
            (Some(c_ours), None) => {
                apply_chunk(&mut out, c_ours);
            }
            (None, Some(c_theirs)) => {
                apply_chunk(&mut out, c_theirs);
            }
            (Some(c_ours), Some(c_theirs)) => {
                if c_ours == c_theirs {
                    apply_chunk(&mut out, c_ours);
                } else {
                    // Conflict detected
                    has_conflicts = true;
                    out.push_str(&format!("<<<<<<< {}{}", our_label, nl));
                    for line in &c_ours.inserted {
                        out.push_str(line);
                    }
                    if let Some(last) = c_ours.inserted.last() {
                        if !last.ends_with('\n') {
                            out.push_str(nl);
                        }
                    }
                    out.push_str(&format!("======={}", nl));
                    for line in &c_theirs.inserted {
                        out.push_str(line);
                    }
                    if let Some(last) = c_theirs.inserted.last() {
                        if !last.ends_with('\n') {
                            out.push_str(nl);
                        }
                    }
                    out.push_str(&format!(">>>>>>> {}{}", their_label, nl));
                }
            }
        }

        base_idx += 1;
    }

    MergeResult {
        content: out,
        has_conflicts,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct BaseChunk<'a> {
    deleted_base_line: bool,
    inserted: Vec<&'a str>,
}

fn apply_chunk(out: &mut String, chunk: &BaseChunk) {
    for line in &chunk.inserted {
        out.push_str(line);
    }
}

fn extract_base_chunks<'a>(ops: &[DiffOp<'a>]) -> std::collections::BTreeMap<usize, BaseChunk<'a>> {
    let mut chunks: std::collections::BTreeMap<usize, BaseChunk<'a>> =
        std::collections::BTreeMap::new();
    let mut base_idx = 0;

    for op in ops {
        match op {
            DiffOp::Keep(_) => {
                base_idx += 1;
            }
            DiffOp::Delete(_) => {
                let chunk = chunks.entry(base_idx).or_default();
                chunk.deleted_base_line = true;
                base_idx += 1;
            }
            DiffOp::Insert(line) => {
                let chunk = chunks.entry(base_idx).or_default();
                chunk.inserted.push(line);
            }
        }
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crlf_and_no_trailing_newline_preserved() {
        let base = "line1\r\nline2\r\nline3";
        let ours = "line1\r\nline2-modified\r\nline3";
        let theirs = "line1\r\nline2\r\nline3";

        let res = three_way_merge(base, ours, theirs, "HEAD", "theirs");
        assert!(!res.has_conflicts);
        assert_eq!(res.content, "line1\r\nline2-modified\r\nline3");
        assert!(!res.content.ends_with('\n'));
    }

    #[test]
    fn test_binary_detection() {
        assert!(is_binary_content(b"hello\x00world"));
        assert!(!is_binary_content(b"hello world\n"));
    }
}

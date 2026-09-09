//! Three-way line-based merge engine with canonical Git conflict markers.

use crate::myers::{myers_diff, DiffOp};

/// Result of a 3-way file merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeResult {
    /// Merged file content (including conflict markers if conflicted).
    pub content: String,
    /// Indicates whether one or more conflicts were encountered.
    pub has_conflicts: bool,
}

/// Performs a 3-way merge of text content from common ancestor `base`, `ours`, and `theirs`.
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

    // Line-level 3-way merge
    let base_lines: Vec<&str> = if base.is_empty() {
        Vec::new()
    } else {
        base.lines().collect()
    };
    let our_lines: Vec<&str> = if ours.is_empty() {
        Vec::new()
    } else {
        ours.lines().collect()
    };
    let their_lines: Vec<&str> = if theirs.is_empty() {
        Vec::new()
    } else {
        theirs.lines().collect()
    };

    let diff_ours = myers_diff(&base_lines, &our_lines);
    let diff_theirs = myers_diff(&base_lines, &their_lines);

    // Group diff ops into base-indexed chunks
    let our_chunks = extract_base_chunks(&diff_ours);
    let their_chunks = extract_base_chunks(&diff_theirs);

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
                    out.push('\n');
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
                    out.push_str(&format!("<<<<<<< {}\n", our_label));
                    for line in &c_ours.inserted {
                        out.push_str(line);
                        out.push('\n');
                    }
                    out.push_str("=======\n");
                    for line in &c_theirs.inserted {
                        out.push_str(line);
                        out.push('\n');
                    }
                    out.push_str(&format!(">>>>>>> {}\n", their_label));
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
        out.push('\n');
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

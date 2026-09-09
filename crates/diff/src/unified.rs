//! Unified diff formatter grouping diff operations into hunks.

use crate::myers::{myers_diff, DiffOp};

/// Represents a single hunk within a unified diff.
#[derive(Debug, Clone)]
pub struct Hunk<'a> {
    /// 1-based start line in original version.
    pub old_start: usize,
    /// Number of lines in original version.
    pub old_count: usize,
    /// 1-based start line in new version.
    pub new_start: usize,
    /// Number of lines in new version.
    pub new_count: usize,
    /// Individual line operations.
    pub lines: Vec<DiffOp<'a>>,
}

/// Formats a complete unified diff for a file.
pub fn format_unified_diff(
    old_path: &str,
    new_path: &str,
    old_content: &str,
    new_content: &str,
    context_size: usize,
) -> Option<String> {
    if old_content == new_content {
        return None;
    }

    let a_lines: Vec<&str> = if old_content.is_empty() {
        Vec::new()
    } else {
        old_content.lines().collect()
    };
    let b_lines: Vec<&str> = if new_content.is_empty() {
        Vec::new()
    } else {
        new_content.lines().collect()
    };

    let ops = myers_diff(&a_lines, &b_lines);
    let hunks = create_hunks(&ops, context_size);

    if hunks.is_empty() {
        return None;
    }

    let mut out = String::new();
    out.push_str(&format!("diff --git a/{} b/{}\n", old_path, new_path));
    out.push_str(&format!("--- a/{}\n", old_path));
    out.push_str(&format!("+++ b/{}\n", new_path));

    for hunk in hunks {
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
        ));
        for line_op in hunk.lines {
            match line_op {
                DiffOp::Keep(l) => {
                    out.push(' ');
                    out.push_str(l);
                    out.push('\n');
                }
                DiffOp::Insert(l) => {
                    out.push('+');
                    out.push_str(l);
                    out.push('\n');
                }
                DiffOp::Delete(l) => {
                    out.push('-');
                    out.push_str(l);
                    out.push('\n');
                }
            }
        }
    }

    Some(out)
}

fn create_hunks<'a>(ops: &[DiffOp<'a>], context_size: usize) -> Vec<Hunk<'a>> {
    let mut hunks = Vec::new();
    let mut i = 0;
    let mut old_line: usize = 1;
    let mut new_line: usize = 1;

    while i < ops.len() {
        // Find next change
        while i < ops.len() && matches!(ops[i], DiffOp::Keep(_)) {
            old_line += 1;
            new_line += 1;
            i += 1;
        }

        if i >= ops.len() {
            break;
        }

        // We found a change. Back up by context lines
        let context_start = i.saturating_sub(context_size);
        let back_count = i - context_start;
        let hunk_old_start = old_line.saturating_sub(back_count);
        let hunk_new_start = new_line.saturating_sub(back_count);

        let mut hunk_lines = Vec::new();
        hunk_lines.extend_from_slice(&ops[context_start..i]);

        let mut hunk_old_count = back_count;
        let mut hunk_new_count = back_count;

        while i < ops.len() {
            match &ops[i] {
                DiffOp::Keep(_) => {
                    // Check how many keeps follow
                    let mut keep_count = 0;
                    let mut j = i;
                    while j < ops.len() && matches!(ops[j], DiffOp::Keep(_)) {
                        keep_count += 1;
                        j += 1;
                    }

                    if keep_count > 2 * context_size && j < ops.len() {
                        // End of this hunk, take context_size keeps
                        hunk_lines.extend_from_slice(&ops[i..i + context_size]);
                        hunk_old_count += context_size;
                        hunk_new_count += context_size;
                        old_line += keep_count;
                        new_line += keep_count;
                        i = j;
                        break;
                    } else if j >= ops.len() {
                        // End of diff, take up to context_size keeps
                        let take = keep_count.min(context_size);
                        hunk_lines.extend_from_slice(&ops[i..i + take]);
                        hunk_old_count += take;
                        hunk_new_count += take;
                        i = j;
                        break;
                    } else {
                        // Keep within hunk
                        hunk_lines.push(ops[i].clone());
                        hunk_old_count += 1;
                        hunk_new_count += 1;
                        old_line += 1;
                        new_line += 1;
                        i += 1;
                    }
                }
                DiffOp::Delete(_) => {
                    hunk_lines.push(ops[i].clone());
                    hunk_old_count += 1;
                    old_line += 1;
                    i += 1;
                }
                DiffOp::Insert(_) => {
                    hunk_lines.push(ops[i].clone());
                    hunk_new_count += 1;
                    new_line += 1;
                    i += 1;
                }
            }
        }

        hunks.push(Hunk {
            old_start: hunk_old_start,
            old_count: hunk_old_count,
            new_start: hunk_new_start,
            new_count: hunk_new_count,
            lines: hunk_lines,
        });
    }

    hunks
}

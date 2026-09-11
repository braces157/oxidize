//! Byte-truthful structured patch engine preserving line endings and EOF newlines.

use crate::myers::{myers_diff, DiffOp};

/// Represents an exact line with line-ending and trailing-newline fidelity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactLine {
    /// Line content excluding trailing `\r` and `\n`.
    pub text: String,
    /// Whether this line ended with CRLF (`\r\n`).
    pub has_crlf: bool,
    /// Whether this line ended with a newline character.
    pub has_newline: bool,
}

/// Splits text into exact lines, preserving CRLF vs LF and EOF newline status.
pub fn split_exact_lines(text: &str) -> Vec<ExactLine> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\n' {
            let has_crlf = i > start && bytes[i - 1] == b'\r';
            let line_end = if has_crlf { i - 1 } else { i };
            let line_text = &text[start..line_end];
            lines.push(ExactLine {
                text: line_text.to_string(),
                has_crlf,
                has_newline: true,
            });
            start = i + 1;
        }
        i += 1;
    }

    if start < bytes.len() {
        let line_text = &text[start..];
        lines.push(ExactLine {
            text: line_text.to_string(),
            has_crlf: false,
            has_newline: false,
        });
    }

    lines
}

/// Reconstructs a string from exact lines.
pub fn reconstruct_exact_lines(lines: &[ExactLine]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(&line.text);
        if line.has_newline {
            if line.has_crlf {
                out.push_str("\r\n");
            } else {
                out.push('\n');
            }
        }
    }
    out
}

/// Operation kind for a diff line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchLineKind {
    /// Context line unchanged between versions.
    Context,
    /// Added line.
    Addition,
    /// Deleted line.
    Deletion,
}

/// An individual line in a structured diff hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchLine {
    /// Line modification kind.
    pub kind: PatchLineKind,
    /// Line content text (excluding line terminator).
    pub content: String,
    /// Whether the original line had CRLF (`\r\n`).
    pub has_crlf: bool,
    /// Whether the original line had a newline terminator.
    pub has_newline: bool,
}

/// A structured, byte-truthful diff hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredHunk {
    /// 1-based start line in original version (0 if empty file).
    pub old_start: usize,
    /// Number of lines in original version.
    pub old_count: usize,
    /// 1-based start line in new version (0 if empty file).
    pub new_start: usize,
    /// Number of lines in new version.
    pub new_count: usize,
    /// Lines comprising this hunk.
    pub lines: Vec<PatchLine>,
}

impl StructuredHunk {
    /// Formats the standard unified diff header for this hunk, e.g. `@@ -1,5 +1,6 @@\n`.
    pub fn header(&self) -> String {
        format!(
            "@@ -{},{} +{},{} @@\n",
            self.old_start, self.old_count, self.new_start, self.new_count
        )
    }
}

/// Computes structured hunks between old and new text with exact line-ending preservation.
pub fn compute_structured_diff(
    old_content: &str,
    new_content: &str,
    context_size: usize,
) -> Vec<StructuredHunk> {
    if old_content == new_content {
        return Vec::new();
    }

    let old_lines = split_exact_lines(old_content);
    let new_lines = split_exact_lines(new_content);

    let old_raw: Vec<String> = old_lines
        .iter()
        .map(|l| {
            format!(
                "{}{}",
                l.text,
                if l.has_newline {
                    if l.has_crlf {
                        "\r\n"
                    } else {
                        "\n"
                    }
                } else {
                    ""
                }
            )
        })
        .collect();

    let new_raw: Vec<String> = new_lines
        .iter()
        .map(|l| {
            format!(
                "{}{}",
                l.text,
                if l.has_newline {
                    if l.has_crlf {
                        "\r\n"
                    } else {
                        "\n"
                    }
                } else {
                    ""
                }
            )
        })
        .collect();

    let old_slices: Vec<&str> = old_raw.iter().map(|s| s.as_str()).collect();
    let new_slices: Vec<&str> = new_raw.iter().map(|s| s.as_str()).collect();

    let diff_ops = myers_diff(&old_slices, &new_slices);

    let mut patch_ops = Vec::with_capacity(diff_ops.len());
    let mut old_idx = 0;
    let mut new_idx = 0;

    for op in diff_ops {
        match op {
            DiffOp::Keep(_) => {
                let l = &old_lines[old_idx];
                patch_ops.push(PatchLine {
                    kind: PatchLineKind::Context,
                    content: l.text.clone(),
                    has_crlf: l.has_crlf,
                    has_newline: l.has_newline,
                });
                old_idx += 1;
                new_idx += 1;
            }
            DiffOp::Delete(_) => {
                let l = &old_lines[old_idx];
                patch_ops.push(PatchLine {
                    kind: PatchLineKind::Deletion,
                    content: l.text.clone(),
                    has_crlf: l.has_crlf,
                    has_newline: l.has_newline,
                });
                old_idx += 1;
            }
            DiffOp::Insert(_) => {
                let l = &new_lines[new_idx];
                patch_ops.push(PatchLine {
                    kind: PatchLineKind::Addition,
                    content: l.text.clone(),
                    has_crlf: l.has_crlf,
                    has_newline: l.has_newline,
                });
                new_idx += 1;
            }
        }
    }

    create_structured_hunks(&patch_ops, context_size)
}

fn create_structured_hunks(ops: &[PatchLine], context_size: usize) -> Vec<StructuredHunk> {
    let mut hunks = Vec::new();
    let mut i = 0;
    let mut old_line: usize = 1;
    let mut new_line: usize = 1;

    while i < ops.len() {
        // Skip leading keeps
        while i < ops.len() && ops[i].kind == PatchLineKind::Context {
            old_line += 1;
            new_line += 1;
            i += 1;
        }

        if i >= ops.len() {
            break;
        }

        // Back up by context_size
        let context_start = i.saturating_sub(context_size);
        let back_count = i - context_start;
        let hunk_old_start = old_line.saturating_sub(back_count).max(1);
        let hunk_new_start = new_line.saturating_sub(back_count).max(1);

        let mut hunk_lines = Vec::new();
        hunk_lines.extend_from_slice(&ops[context_start..i]);

        let mut hunk_old_count = back_count;
        let mut hunk_new_count = back_count;

        while i < ops.len() {
            match ops[i].kind {
                PatchLineKind::Context => {
                    let mut keep_count = 0;
                    let mut j = i;
                    while j < ops.len() && ops[j].kind == PatchLineKind::Context {
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
                        old_line += keep_count;
                        new_line += keep_count;
                        i = j;
                        break;
                    } else {
                        // Context line within hunk
                        hunk_lines.push(ops[i].clone());
                        hunk_old_count += 1;
                        hunk_new_count += 1;
                        old_line += 1;
                        new_line += 1;
                        i += 1;
                    }
                }
                PatchLineKind::Deletion => {
                    hunk_lines.push(ops[i].clone());
                    hunk_old_count += 1;
                    old_line += 1;
                    i += 1;
                }
                PatchLineKind::Addition => {
                    hunk_lines.push(ops[i].clone());
                    hunk_new_count += 1;
                    new_line += 1;
                    i += 1;
                }
            }
        }

        hunks.push(StructuredHunk {
            old_start: hunk_old_start,
            old_count: hunk_old_count,
            new_start: hunk_new_start,
            new_count: hunk_new_count,
            lines: hunk_lines,
        });
    }

    hunks
}

fn matches_lines(base: &[ExactLine], expected: &[&PatchLine]) -> bool {
    if base.len() != expected.len() {
        return false;
    }
    for (b, e) in base.iter().zip(expected.iter()) {
        if b.text != e.content {
            return false;
        }
    }
    true
}

/// Applies a structured hunk forward onto base content (staging into index).
pub fn apply_hunk_forward(base_text: &str, hunk: &StructuredHunk) -> Result<String, String> {
    let mut base_lines = split_exact_lines(base_text);

    let pre_image: Vec<&PatchLine> = hunk
        .lines
        .iter()
        .filter(|l| l.kind == PatchLineKind::Context || l.kind == PatchLineKind::Deletion)
        .collect();

    let post_image: Vec<&PatchLine> = hunk
        .lines
        .iter()
        .filter(|l| l.kind == PatchLineKind::Context || l.kind == PatchLineKind::Addition)
        .collect();

    let expected_start = if hunk.old_start == 0 {
        0
    } else {
        (hunk.old_start - 1).min(base_lines.len())
    };

    // 1. Try exact expected position
    let match_pos = if expected_start + pre_image.len() <= base_lines.len()
        && matches_lines(
            &base_lines[expected_start..expected_start + pre_image.len()],
            &pre_image,
        ) {
        Some(expected_start)
    } else {
        // 2. Scan for exact match anywhere in the file
        let mut found = None;
        if !pre_image.is_empty() {
            if base_lines.len() >= pre_image.len() {
                for idx in 0..=base_lines.len() - pre_image.len() {
                    if matches_lines(&base_lines[idx..idx + pre_image.len()], &pre_image) {
                        found = Some(idx);
                        break;
                    }
                }
            }
        } else {
            found = Some(expected_start);
        }
        found
    };

    let match_pos = if let Some(pos) = match_pos {
        Some(pos)
    } else {
        let has_changes = hunk
            .lines
            .iter()
            .any(|l| l.kind == PatchLineKind::Addition || l.kind == PatchLineKind::Deletion);
        if has_changes && !post_image.is_empty() {
            if expected_start + post_image.len() <= base_lines.len()
                && matches_lines(
                    &base_lines[expected_start..expected_start + post_image.len()],
                    &post_image,
                )
            {
                return Ok(reconstruct_exact_lines(&base_lines));
            }
            let mut already_applied = false;
            if base_lines.len() >= post_image.len() {
                for idx in 0..=base_lines.len() - post_image.len() {
                    if matches_lines(&base_lines[idx..idx + post_image.len()], &post_image) {
                        already_applied = true;
                        break;
                    }
                }
            }
            if already_applied {
                return Ok(reconstruct_exact_lines(&base_lines));
            }
        }
        None
    };

    let match_idx = match_pos.ok_or_else(|| {
        format!(
            "pre-image mismatch: hunk at line {} could not be applied cleanly",
            hunk.old_start
        )
    })?;

    // Replace pre-image with post-image
    let replacement: Vec<ExactLine> = post_image
        .iter()
        .map(|p| ExactLine {
            text: p.content.clone(),
            has_crlf: p.has_crlf,
            has_newline: p.has_newline,
        })
        .collect();

    base_lines.splice(match_idx..match_idx + pre_image.len(), replacement);

    Ok(reconstruct_exact_lines(&base_lines))
}

/// Applies a structured hunk in reverse onto target content (unstaging from index or discarding from worktree).
pub fn apply_hunk_reverse(target_text: &str, hunk: &StructuredHunk) -> Result<String, String> {
    let mut target_lines = split_exact_lines(target_text);

    // For reverse application:
    // Pre-image is Context + Addition (what is currently in target)
    let pre_image: Vec<&PatchLine> = hunk
        .lines
        .iter()
        .filter(|l| l.kind == PatchLineKind::Context || l.kind == PatchLineKind::Addition)
        .collect();

    // Replacement is Context + Deletion (what we want to revert back to)
    let replacement_image: Vec<&PatchLine> = hunk
        .lines
        .iter()
        .filter(|l| l.kind == PatchLineKind::Context || l.kind == PatchLineKind::Deletion)
        .collect();

    let expected_start = if hunk.new_start == 0 {
        0
    } else {
        (hunk.new_start - 1).min(target_lines.len())
    };

    // 1. Try exact expected position
    let match_pos = if expected_start + pre_image.len() <= target_lines.len()
        && matches_lines(
            &target_lines[expected_start..expected_start + pre_image.len()],
            &pre_image,
        ) {
        Some(expected_start)
    } else {
        // 2. Scan for exact match anywhere in the file
        let mut found = None;
        if !pre_image.is_empty() {
            if target_lines.len() >= pre_image.len() {
                for idx in 0..=target_lines.len() - pre_image.len() {
                    if matches_lines(&target_lines[idx..idx + pre_image.len()], &pre_image) {
                        found = Some(idx);
                        break;
                    }
                }
            }
        } else {
            found = Some(expected_start);
        }
        found
    };

    let match_pos = if let Some(pos) = match_pos {
        Some(pos)
    } else {
        let has_changes = hunk
            .lines
            .iter()
            .any(|l| l.kind == PatchLineKind::Addition || l.kind == PatchLineKind::Deletion);
        if has_changes && !replacement_image.is_empty() {
            if expected_start + replacement_image.len() <= target_lines.len()
                && matches_lines(
                    &target_lines[expected_start..expected_start + replacement_image.len()],
                    &replacement_image,
                )
            {
                return Ok(reconstruct_exact_lines(&target_lines));
            }
            let mut already_reverted = false;
            if target_lines.len() >= replacement_image.len() {
                for idx in 0..=target_lines.len() - replacement_image.len() {
                    if matches_lines(
                        &target_lines[idx..idx + replacement_image.len()],
                        &replacement_image,
                    ) {
                        already_reverted = true;
                        break;
                    }
                }
            }
            if already_reverted {
                return Ok(reconstruct_exact_lines(&target_lines));
            }
        }
        None
    };

    let match_idx = match_pos.ok_or_else(|| {
        format!(
            "pre-image mismatch: reverse hunk at line {} could not be applied cleanly",
            hunk.new_start
        )
    })?;

    let replacement: Vec<ExactLine> = replacement_image
        .iter()
        .map(|p| ExactLine {
            text: p.content.clone(),
            has_crlf: p.has_crlf,
            has_newline: p.has_newline,
        })
        .collect();

    target_lines.splice(match_idx..match_idx + pre_image.len(), replacement);

    Ok(reconstruct_exact_lines(&target_lines))
}

/// A hunk stored in a custom patch basket with its target file path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomPatchHunk {
    /// Relative path of the file to which this hunk applies.
    pub path: String,
    /// The structured diff hunk.
    pub hunk: StructuredHunk,
}

/// A persistent patch basket collecting structured hunks across files and commits.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CustomPatchBasket {
    pub hunks: Vec<CustomPatchHunk>,
}

impl CustomPatchBasket {
    /// Creates a new empty custom patch basket.
    pub fn new() -> Self {
        Self { hunks: Vec::new() }
    }

    /// Adds a hunk to the basket. Returns true if added, false if an identical hunk for this path was already present.
    pub fn add_hunk(&mut self, path: &str, hunk: StructuredHunk) -> bool {
        if self.hunks.iter().any(|h| h.path == path && h.hunk == hunk) {
            return false;
        }
        self.hunks.push(CustomPatchHunk {
            path: path.to_string(),
            hunk,
        });
        true
    }

    /// Checks if a hunk matching the path and hunk contents is already in the basket.
    pub fn contains_hunk(&self, path: &str, hunk: &StructuredHunk) -> bool {
        self.hunks.iter().any(|h| h.path == path && &h.hunk == hunk)
    }

    /// Removes a matching hunk from the basket if present. Returns true if removed.
    pub fn remove_matching_hunk(&mut self, path: &str, hunk: &StructuredHunk) -> bool {
        if let Some(pos) = self
            .hunks
            .iter()
            .position(|h| h.path == path && &h.hunk == hunk)
        {
            self.hunks.remove(pos);
            true
        } else {
            false
        }
    }

    /// Removes a hunk at the specified index.
    pub fn remove_hunk(&mut self, index: usize) -> Option<CustomPatchHunk> {
        if index < self.hunks.len() {
            Some(self.hunks.remove(index))
        } else {
            None
        }
    }

    /// Removes all hunks matching a specific path. Returns the number of removed hunks.
    pub fn remove_path(&mut self, path: &str) -> usize {
        let before = self.hunks.len();
        self.hunks.retain(|h| h.path != path);
        before - self.hunks.len()
    }

    /// Clears the basket.
    pub fn clear(&mut self) {
        self.hunks.clear();
    }

    /// Returns true if the basket contains no hunks.
    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }

    /// Returns the total number of hunks in the basket.
    pub fn len(&self) -> usize {
        self.hunks.len()
    }

    /// Returns unique paths in the basket, sorted.
    pub fn paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = self.hunks.iter().map(|h| h.path.clone()).collect();
        paths.sort();
        paths.dedup();
        paths
    }

    /// Returns references to all hunks for a given path.
    pub fn hunks_for_path(&self, path: &str) -> Vec<&StructuredHunk> {
        self.hunks
            .iter()
            .filter(|h| h.path == path)
            .map(|h| &h.hunk)
            .collect()
    }

    /// Applies all hunks in this basket for `path` onto `text`.
    pub fn apply_to_text(&self, path: &str, text: &str, reverse: bool) -> Result<String, String> {
        let mut current = text.to_string();
        for hunk in self.hunks_for_path(path) {
            current = if reverse {
                apply_hunk_reverse(&current, hunk)?
            } else {
                apply_hunk_forward(&current, hunk)?
            };
        }
        Ok(current)
    }

    /// Formats the custom patch basket as a standard Git unified diff patch.
    pub fn format_patch(&self) -> String {
        let mut out = String::new();
        for path in self.paths() {
            out.push_str(&format!(
                "diff --git a/{0} b/{0}\n--- a/{0}\n+++ b/{0}\n",
                path
            ));
            for hunk in self.hunks_for_path(&path) {
                out.push_str(&hunk.header());
                for line in &hunk.lines {
                    let prefix = match line.kind {
                        PatchLineKind::Context => " ",
                        PatchLineKind::Addition => "+",
                        PatchLineKind::Deletion => "-",
                    };
                    out.push_str(prefix);
                    out.push_str(&line.content);
                    if line.has_newline {
                        if line.has_crlf {
                            out.push_str("\r\n");
                        } else {
                            out.push('\n');
                        }
                    } else {
                        out.push_str("\n\\ No newline at end of file\n");
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_lines_crlf_roundtrip() {
        let text = "foo\r\nbar\r\nbaz\r\n";
        let lines = split_exact_lines(text);
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|l| l.has_crlf && l.has_newline));
        assert_eq!(reconstruct_exact_lines(&lines), text);
    }

    #[test]
    fn test_exact_lines_no_trailing_newline() {
        let text = "first line\nsecond line";
        let lines = split_exact_lines(text);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].has_newline);
        assert!(!lines[1].has_newline);
        assert_eq!(reconstruct_exact_lines(&lines), text);
    }

    #[test]
    fn test_structured_diff_hunks_and_roundtrip() {
        let old_text = "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\n";
        let new_text = "line1\nline2 - changed\nline3\nline4\nline5\nline6\nline7\nline8 - changed\nline9\nline10\n";

        let hunks = compute_structured_diff(old_text, new_text, 1);
        assert_eq!(hunks.len(), 2);

        // Apply hunk 0 forward
        let step1 = apply_hunk_forward(old_text, &hunks[0]).unwrap();
        assert!(step1.contains("line2 - changed"));
        assert!(!step1.contains("line8 - changed"));

        // Apply hunk 1 forward
        let step2 = apply_hunk_forward(&step1, &hunks[1]).unwrap();
        assert_eq!(step2, new_text);

        // Revert hunk 1
        let rev1 = apply_hunk_reverse(&step2, &hunks[1]).unwrap();
        assert_eq!(rev1, step1);

        // Revert hunk 0
        let rev0 = apply_hunk_reverse(&rev1, &hunks[0]).unwrap();
        assert_eq!(rev0, old_text);
    }

    #[test]
    fn test_crlf_hunk_application() {
        let old_text = "one\r\ntwo\r\nthree\r\n";
        let new_text = "one\r\ntwo modified\r\nthree\r\n";

        let hunks = compute_structured_diff(old_text, new_text, 1);
        assert_eq!(hunks.len(), 1);

        let applied = apply_hunk_forward(old_text, &hunks[0]).unwrap();
        assert_eq!(applied, new_text);

        let reverted = apply_hunk_reverse(&applied, &hunks[0]).unwrap();
        assert_eq!(reverted, old_text);
    }

    #[test]
    fn test_custom_patch_basket_operations() {
        let text1_old = "alpha\nbeta\ngamma\n";
        let text1_new = "alpha\nbeta modified\ngamma\n";
        let hunks1 = compute_structured_diff(text1_old, text1_new, 1);

        let mut basket = CustomPatchBasket::new();
        assert!(basket.is_empty());
        assert!(basket.add_hunk("file1.txt", hunks1[0].clone()));
        assert_eq!(basket.len(), 1);
        // Duplicate hunk is rejected
        assert!(!basket.add_hunk("file1.txt", hunks1[0].clone()));
        assert_eq!(basket.len(), 1);

        let patch_str = basket.format_patch();
        assert!(patch_str.contains("diff --git a/file1.txt b/file1.txt"));
        assert!(patch_str.contains("+beta modified"));

        let applied = basket.apply_to_text("file1.txt", text1_old, false).unwrap();
        assert_eq!(applied, text1_new);

        let reverted = basket.apply_to_text("file1.txt", &applied, true).unwrap();
        assert_eq!(reverted, text1_old);

        basket.clear();
        assert!(basket.is_empty());
    }
}
